use super::*;

/// The time projectiles are placed at this frame: the client clock once it
/// runs, else the snapshot's own time, moved onto the trajectory timeline.
pub(super) fn missile_time(snapshot: &net::ShownWorldFrame, clock: Option<&net::FrameClock>) -> i32 {
    let running = clock.filter(|clock| clock.started());
    let now = match running {
        Some(clock) => clock.time(),
        None => snapshot.tick().map_or(0, sim::level_time_ms),
    };
    snapshot.trajectory_time_ms(now)
}

pub(super) fn missile_origin(snapshot: &net::ShownWorldFrame, row: &ShownProjectile, at: i32) -> [f32; 3] {
    if let ShownProjectile::HostDecided(authoritative) = row {
        return snapshot.projectile_origin_at(authoritative, at);
    }
    row.point_at(at)
}

pub(super) fn clip_phase(looping: bool, phase: f32) -> f32 {
    match looping {
        true => phase.rem_euclid(1.0),
        false => phase.clamp(0.0, 1.0),
    }
}

// ---------------------------------------------------------------------------
// Skeletons and animation for scripted models
// ---------------------------------------------------------------------------

/// Builds the combined skeleton of a composition from its models' pose
/// sources, attaching each child at its tag.
pub(super) fn composition_dobj(
    state: &xmodel_runtime::RigPoseRecord,
    sources: &[&xmodel_runtime::PoseDonorModel],
) -> Option<PosedSkeleton> {
    let mut parts = Vec::with_capacity(sources.len());
    for (desc, &source) in state.composition.models.iter().zip(sources) {
        let attach = desc.host_index.map(|parent_in| {
            let parent_in = usize::from(parent_in);
            // An empty tag joins at the child's root bone (a zombie's
            // head at the body's spine); same-named bones then follow
            // the parent's.
            let authored = desc.mount_tag.as_deref().filter(|tag| !tag.is_empty());
            let tag = match authored {
                Some(tag) => tag.to_owned(),
                None => sources
                    .get(parent_in)
                    .map(|owner| xmodel_runtime::empty_tag_attach(owner, source))
                    .unwrap_or_default(),
            };
            MountedModel { host_model: parent_in, mount_tag: tag }
        });
        parts.push((source, attach));
    }
    PosedSkeleton::build(&parts).ok()
}

pub(super) fn pose_sources<'a>(
    fpv: &'a asset_model::FirstPersonMeshRoster,
    state: &xmodel_runtime::RigPoseRecord,
) -> Option<Vec<&'a xmodel_runtime::PoseDonorModel>> {
    let mut found = Vec::new();
    for desc in &state.composition.models {
        let entry = fpv.by_label(AssetCategoryScope::T6, &desc.model)?;
        found.push(entry.bone_rig.pose.as_ref()?);
    }
    Some(found)
}

/// One animation leaf being played: its clip, which skeleton bone each
/// track drives, the time into it and its blend weight.
pub(super) struct PlayingLeaf {
    reel: Arc<xmodel_runtime::SkeletalClip>,
    tracks: Vec<Option<usize>>,
    time: f32,
    wt: f32,
}

/// Poses a composition (a zombie's body and head) with its animation tree's
/// leaves and skins every model; `None` when nothing is animating it (it is
/// then drawn in bind pose).
pub(super) fn pose_t6_composition(
    fpv: &asset_model::FirstPersonMeshRoster,
    animation_refs: &anim_bank::MotionClipRoster,
    state: &xmodel_runtime::RigPoseRecord,
    since_snapshot: f32,
) -> Option<Vec<Option<Arc<PosedVerts>>>> {
    let nodes = &state.tree.as_ref()?.nodes;
    let leaves: Vec<_> = nodes
        .iter()
        .filter(|node| node.clip.is_some() && matches!(node.kind, AnimGraphRole::Leaf))
        .collect::<_>();
    if leaves.is_empty() {
        return None;
    }
    let mut entries: Vec<&asset_model::FirstPersonMeshRecord> = Vec::new();
    let mut sources_in = Vec::new();
    for desc in &state.composition.models {
        let entry = fpv.by_label(AssetCategoryScope::T6, &desc.model)?;
        entries.push(entry);
        sources_in.push(entry.bone_rig.pose.as_ref()?);
    }
    let dobj = composition_dobj(state, &sources_in)?;

    let sole = leaves.len() == 1;
    let mut playing = Vec::with_capacity(leaves.len());
    for leaf_in in &leaves {
        let Some(clip) = leaf_in
            .clip
            .as_deref()
            .and_then(|name| animation_refs.clip(AssetCategoryScope::T6, name))
        else {
            continue;
        };
        let phase = clip_phase(clip.looping, leaf_in.state.time + leaf_in.state.rate * since_snapshot);
        playing.push(PlayingLeaf {
            tracks: clip.tracks.iter().map(|track| dobj.find(&track.name)).collect(),
            time: phase * clip.duration().max(1e-3),
            wt: if sole { 1.0 } else { leaf_in.state.weight },
            reel: clip,
        });
    }
    if playing.is_empty() {
        return None;
    }
    let mut instances = Vec::with_capacity(playing.len());
    for leaf_in in &playing {
        instances.push(ClipPlayback {
            clip: &leaf_in.reel,
            tracks: &leaf_in.tracks,
            time: leaf_in.time,
            weight: leaf_in.wt,
            parts: None,
        });
    }
    let skin = dobj.skin_matrices(&dobj.pose(&instances, &dobj.full_part_set(), Mat4::IDENTITY));
    let skinned = entries
        .iter()
        .zip(0usize..)
        .map(|(entry, slot)| {
            let placed = dobj.models.get(slot)?;
            Some(skin_model(&entry.bone_rig, &skin, placed.first_joint))
        })
        .collect::<_>();
    Some(skinned)
}

/// A composition's bones in model space (its base model's, then each
/// attachment's, as the server numbers them for an effect's tag), posed by its
/// animation tree's first leaf when it has one.
pub(super) fn t6_composition_bones(
    fpv: &asset_model::FirstPersonMeshRoster,
    animation_refs: Option<&anim_bank::MotionClipRoster>,
    state: &xmodel_runtime::RigPoseRecord,
) -> Option<Vec<Mat4>> {
    let sources = pose_sources(fpv, state)?;
    let dobj = composition_dobj(state, &sources)?;
    let first_leaf = state.tree.as_ref().and_then(|tree| {
        tree.nodes
            .iter()
            .find(|node| node.clip.is_some() && matches!(node.kind, AnimGraphRole::Leaf))
    });
    let playing = first_leaf.and_then(|leaf| {
        let name = leaf.clip.as_deref()?;
        Some((leaf, animation_refs?.clip(AssetCategoryScope::T6, name)?))
    });
    let Some((leaf, clip)) = playing else {
        return Some(dobj.pose(&[], &dobj.full_part_set(), Mat4::IDENTITY));
    };
    let tracks: Vec<Option<usize>> = clip.tracks.iter().map(|track| dobj.find(&track.name)).collect();
    let time = clip_phase(clip.looping, leaf.state.time) * clip.duration().max(1e-3);
    let only = ClipPlayback {
        clip: &clip,
        tracks: &tracks,
        time,
        weight: 1.0,
        parts: None,
    };
    Some(dobj.pose(&[only], &dobj.full_part_set(), Mat4::IDENTITY))
}

/// Which models have wheels or rotors, and each vehicle's wheels and springs
/// as it drives and rotors as they turn.
#[derive(Default)]
pub(super) struct T6Cars {
    by_model: HashMap<String, Option<Arc<PosedSkeleton>>>,
    pub(super) cars: HashMap<u32, T6Car>,
}

pub(super) struct T6Car {
    wheels: Option<script_prop::VehicleWheels>,
    /// bo2mp: a helicopter's (the Care Package drone's), which this pass
    /// draws when the map's own models do not hold it.
    rotors: Option<script_prop::HeliRotors>,
    dobj: Arc<PosedSkeleton>,
    key: (u32, u32),
    posed: Option<Arc<PosedVerts>>,
    pub(super) seen: bool,
}

impl T6Cars {
    /// `state` with the still rotor blades hidden while vehicle `id`'s rotors
    /// spin (its blur discs show); `None` for anything without rotors.
    pub(super) fn spun_state(
        &self,
        id: u32,
        state: &xmodel_runtime::RigPoseRecord,
    ) -> Option<xmodel_runtime::RigPoseRecord> {
        let car = self.cars.get(&id).filter(|car| car.seen)?;
        let rotors = car.rotors.as_ref()?;
        let mut hidden = state.clone();
        rotors.hide(&car.dobj, &mut hidden.hidden_mask);
        Some(hidden)
    }
}

/// A car or helicopter with no animation playing, posed with its wheels
/// turning and steering and its body on its springs, or its rotors turning
/// (they are posed in code, not by an animation); `None` for anything with
/// neither.
pub(super) fn pose_t6_car(
    fpv: &asset_model::FirstPersonMeshRoster,
    state: &xmodel_runtime::RigPoseRecord,
    id: u32,
    xform: &Transform,
    cars: &mut T6Cars,
) -> Option<Vec<Option<Arc<PosedVerts>>>> {
    let [only] = state.composition.models.as_slice() else {
        return None;
    };
    let entry = fpv.by_label(AssetCategoryScope::T6, &only.model)?;
    let dobj = cars
        .by_model
        .entry(only.model.clone())
        .or_insert_with(|| {
            let source = entry.bone_rig.pose.as_ref()?;
            let dobj = PosedSkeleton::build(&[(source, None)]).ok()?;
            let driven = script_prop::has_wheels(&dobj) || script_prop::has_rotors(&dobj);
            driven.then(|| Arc::new(dobj))
        })
        .clone()?;
    let car = cars.cars.entry(id).or_insert_with(|| T6Car {
        wheels: script_prop::has_wheels(&dobj).then(script_prop::VehicleWheels::new),
        rotors: script_prop::has_rotors(&dobj).then(script_prop::HeliRotors::new),
        dobj: Arc::clone(&dobj),
        key: (u32::MAX, u32::MAX),
        posed: None,
        seen: false,
    });
    car.seen = true;
    if let Some(wheels) = car.wheels.as_mut() {
        wheels.advance(xform.translation, xform.rotation, state.vehicle_steer);
    }
    if let Some(rotors) = car.rotors.as_mut() {
        rotors.advance();
    }
    let key = (
        car.wheels.as_ref().map_or(0, |wheels| wheels.key()),
        car.rotors.as_ref().map_or(0, |rotors| rotors.key()),
    );
    if key != car.key || car.posed.is_none() {
        let mut locals_in = dobj.rest_local_pose();
        if let Some(wheels) = car.wheels.as_ref() {
            wheels.apply(&dobj, &mut locals_in);
        }
        if let Some(rotors) = car.rotors.as_ref() {
            rotors.apply(&dobj, &mut locals_in);
            script_prop::rotor_log(&dobj, rotors, "fallback");
        }
        dobj.copy_mirrored_parts(&mut locals_in);
        let skin = dobj.skin_matrices(&dobj.world_pose(&locals_in, Mat4::IDENTITY));
        let base = dobj.models.first().map_or(0, |m| m.first_joint);
        car.posed = Some(skin_model(&entry.bone_rig, &skin, base));
        car.key = key;
    }
    Some(vec![car.posed.clone()])
}

// ---------------------------------------------------------------------------
// Debug logging for the collectors (each switched on by an environment variable)
// ---------------------------------------------------------------------------
