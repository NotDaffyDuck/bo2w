use super::*;

/// The time projectiles are placed at this frame: the client clock once it
/// runs, else the snapshot's own time, moved onto the trajectory timeline.
pub(super) fn missile_time(
    presented: &net::PresentedSnapshot,
    clock: Option<&net::FrameClock>,
) -> i32 {
    let frame_time = match clock {
        Some(clock) if clock.started() => clock.time(),
        _ => presented.tick().map_or(0, sim::level_time_ms),
    };
    presented.trajectory_time_ms(frame_time)
}

pub(super) fn missile_origin(
    presented: &net::PresentedSnapshot,
    row: &PresentedProjectile,
    time: i32,
) -> [f32; 3] {
    match row {
        PresentedProjectile::Authoritative(projectile) => {
            presented.projectile_origin_at(projectile, time)
        }
        PresentedProjectile::Predicted { .. } => row.origin_at(time),
    }
}

pub(super) fn clip_phase(looping: bool, phase: f32) -> f32 {
    if looping {
        phase.rem_euclid(1.0)
    } else {
        phase.clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Skeletons and animation for scripted models
// ---------------------------------------------------------------------------

/// Builds the combined skeleton of a composition from its models' pose
/// sources, attaching each child at its tag.
pub(super) fn composition_dobj(
    state: &xmodel_runtime::DObjSemanticState,
    sources: &[&xmodel_runtime::ModelPoseSrc],
) -> Option<DObj> {
    let parts: Vec<(&xmodel_runtime::ModelPoseSrc, Option<Attach>)> = state
        .composition
        .models
        .iter()
        .zip(sources)
        .map(|(desc, &source)| {
            let attach = desc.parent_model.map(|parent| {
                let parent = usize::from(parent);
                // An empty tag joins at the child's root bone (a zombie's
                // head at the body's spine); same-named bones then follow
                // the parent's.
                let tag = match desc.attach_tag.as_deref() {
                    Some(tag) if !tag.is_empty() => tag.to_owned(),
                    _ => sources
                        .get(parent)
                        .map(|p| xmodel_runtime::empty_tag_attach(p, source))
                        .unwrap_or_default(),
                };
                Attach {
                    parent_model: parent,
                    tag,
                }
            });
            (source, attach)
        })
        .collect();
    DObj::build(&parts).ok()
}

pub(super) fn pose_sources<'a>(
    fpv: &'a asset_model::FpvMeshCatalog,
    state: &xmodel_runtime::DObjSemanticState,
) -> Option<Vec<&'a xmodel_runtime::ModelPoseSrc>> {
    state
        .composition
        .models
        .iter()
        .map(|desc| {
            fpv.get(AssetNamespace::T6, &desc.model)
                .and_then(|entry| entry.skel.pose.as_ref())
        })
        .collect()
}

/// One animation leaf being played: its clip, which skeleton bone each
/// track drives, the time into it and its blend weight.
pub(super) struct PlayingLeaf {
    clip: Arc<xmodel_runtime::AnimClip>,
    tracks: Vec<Option<usize>>,
    time: f32,
    weight: f32,
}

/// Poses a composition (a zombie's body and head) with its animation tree's
/// leaves and skins every model; `None` when nothing is animating it (it is
/// then drawn in bind pose).
pub(super) fn pose_t6_composition(
    fpv: &asset_model::FpvMeshCatalog,
    xanims: &asset_anim::XAnimCatalog,
    state: &xmodel_runtime::DObjSemanticState,
    since_snapshot: f32,
) -> Option<Vec<Option<Arc<PosedVerts>>>> {
    let tree = state.tree.as_ref()?;
    let leaves: Vec<_> = tree
        .nodes
        .iter()
        .filter(|node| node.kind == XAnimSemanticNodeKind::Leaf && node.clip.is_some())
        .collect();
    if leaves.is_empty() {
        return None;
    }
    let entries = state
        .composition
        .models
        .iter()
        .map(|desc| fpv.get(AssetNamespace::T6, &desc.model))
        .collect::<Option<Vec<&asset_model::FpvMeshEntry>>>()?;
    let sources = entries
        .iter()
        .map(|entry| entry.skel.pose.as_ref())
        .collect::<Option<Vec<_>>>()?;
    let dobj = composition_dobj(state, &sources)?;

    let playing: Vec<PlayingLeaf> = leaves
        .iter()
        .filter_map(|leaf| {
            let clip = xanims.clip(AssetNamespace::T6, leaf.clip.as_deref()?)?;
            let phase = clip_phase(
                clip.looping,
                leaf.state.time + leaf.state.rate * since_snapshot,
            );
            Some(PlayingLeaf {
                tracks: clip.tracks.iter().map(|t| dobj.find(&t.name)).collect(),
                time: phase * clip.duration().max(1e-3),
                weight: if leaves.len() == 1 {
                    1.0
                } else {
                    leaf.state.weight
                },
                clip,
            })
        })
        .collect();
    if playing.is_empty() {
        return None;
    }
    let instances: Vec<AnimInstance<'_>> = playing
        .iter()
        .map(|leaf| AnimInstance {
            clip: &leaf.clip,
            tracks: &leaf.tracks,
            time: leaf.time,
            weight: leaf.weight,
            parts: None,
        })
        .collect();
    let world = dobj.pose(&instances, &dobj.all_parts(), Mat4::IDENTITY);
    let skin = dobj.skin_matrices(&world);
    Some(
        entries
            .iter()
            .enumerate()
            .map(|(model, entry)| {
                let base = dobj.models.get(model)?.base;
                Some(skin_model(&entry.skel, &skin, base))
            })
            .collect(),
    )
}

/// A composition's bones in model space (its base model's, then each
/// attachment's, as the server numbers them for an effect's tag), posed by its
/// animation tree's first leaf when it has one.
pub(super) fn t6_composition_bones(
    fpv: &asset_model::FpvMeshCatalog,
    xanims: Option<&asset_anim::XAnimCatalog>,
    state: &xmodel_runtime::DObjSemanticState,
) -> Option<Vec<Mat4>> {
    let sources = pose_sources(fpv, state)?;
    let dobj = composition_dobj(state, &sources)?;
    let leaf = state.tree.as_ref().and_then(|tree| {
        tree.nodes
            .iter()
            .find(|node| node.kind == XAnimSemanticNodeKind::Leaf && node.clip.is_some())
    });
    let clip = leaf.and_then(|l| xanims?.clip(AssetNamespace::T6, l.clip.as_deref()?));
    let Some((leaf, clip)) = leaf.zip(clip) else {
        return Some(dobj.pose(&[], &dobj.all_parts(), Mat4::IDENTITY));
    };
    let tracks: Vec<Option<usize>> = clip.tracks.iter().map(|t| dobj.find(&t.name)).collect();
    let instance = AnimInstance {
        clip: &clip,
        tracks: &tracks,
        time: clip_phase(clip.looping, leaf.state.time) * clip.duration().max(1e-3),
        weight: 1.0,
        parts: None,
    };
    Some(dobj.pose(&[instance], &dobj.all_parts(), Mat4::IDENTITY))
}

/// Which models have wheels, and each car's wheels and springs as it drives.
#[derive(Default)]
pub(super) struct T6Cars {
    by_model: HashMap<String, Option<Arc<DObj>>>,
    pub(super) cars: HashMap<u32, T6Car>,
}

pub(super) struct T6Car {
    wheels: script_model::VehicleWheels,
    key: u32,
    posed: Option<Arc<PosedVerts>>,
    pub(super) seen: bool,
}

/// A car with no animation playing, posed with its wheels turning and
/// steering and its body on its springs (they are posed in code, not by an
/// animation); `None` for anything without wheels.
pub(super) fn pose_t6_car(
    fpv: &asset_model::FpvMeshCatalog,
    state: &xmodel_runtime::DObjSemanticState,
    id: u32,
    transform: &Transform,
    cars: &mut T6Cars,
) -> Option<Vec<Option<Arc<PosedVerts>>>> {
    let [only] = state.composition.models.as_slice() else {
        return None;
    };
    let entry = fpv.get(AssetNamespace::T6, &only.model)?;
    let dobj = cars
        .by_model
        .entry(only.model.clone())
        .or_insert_with(|| {
            let source = entry.skel.pose.as_ref()?;
            let dobj = DObj::build(&[(source, None)]).ok()?;
            script_model::has_wheels(&dobj).then(|| Arc::new(dobj))
        })
        .clone()?;
    let car = cars.cars.entry(id).or_insert_with(|| T6Car {
        wheels: script_model::VehicleWheels::new(),
        key: u32::MAX,
        posed: None,
        seen: false,
    });
    car.seen = true;
    car.wheels.advance(
        transform.translation,
        transform.rotation,
        state.vehicle_steer,
    );
    let key = car.wheels.key();
    if key != car.key || car.posed.is_none() {
        let mut locals = dobj.bind_locals();
        car.wheels.apply(&dobj, &mut locals);
        dobj.apply_duplicates(&mut locals);
        let skin = dobj.skin_matrices(&dobj.compose(&locals, Mat4::IDENTITY));
        let base = dobj.models.first().map_or(0, |m| m.base);
        car.posed = Some(skin_model(&entry.skel, &skin, base));
        car.key = key;
    }
    Some(vec![car.posed.clone()])
}

// ---------------------------------------------------------------------------
// Debug logging for the collectors (each switched on by an environment variable)
// ---------------------------------------------------------------------------
