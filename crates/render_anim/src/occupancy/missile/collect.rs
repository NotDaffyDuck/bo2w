use super::*;
use bevy::ecs::system::SystemParam;
use diag::info;

/// Seconds the frame is past the snapshot that the animation times came in
/// with (at most a quarter second), so an animated model plays on smoothly
/// between snapshots.
pub(super) fn seconds_past_snapshot(
    shown: Option<&net::ShownWorldFrame>,
    clock: Option<&net::FrameClock>,
) -> f32 {
    let Some(clock) = clock.filter(|clock| clock.started()) else {
        return 0.0;
    };
    let Some(snapshot) = shown.and_then(|shown| shown.snapshot()) else {
        return 0.0;
    };
    let elapsed_ms = clock.time() - sim::level_time_ms(snapshot.tick);
    (elapsed_ms as f32 / 1000.0).clamp(0.0, 0.25)
}

/// Midpoint of a model's bounds, placed in the world; `None` without bounds.
pub(super) fn bounds_middle(
    catalog: &asset_model::FirstPersonMeshRoster,
    name: &str,
    placement: &Mat4,
) -> Option<[f32; 3]> {
    let (lo, hi) = catalog.by_label(AssetCategoryScope::T6, name)?.bone_rig.bounds?;
    let centre = Vec3::from_array(lo).midpoint(Vec3::from_array(hi));
    Some(placement.transform_point3(centre).to_array())
}

/// What the script-model collection reads and does not change.
#[derive(SystemParam)]
pub(super) struct ScriptModelSources<'w, 's> {
    models: Query<
        'w,
        's,
        (
            &'static render_scene::LevelScriptModelInstance,
            &'static Transform,
            &'static Visibility,
        ),
    >,
    loose: Query<
        'w,
        's,
        (
            &'static render_scene::LevelDebrisInstance,
            &'static Transform,
            &'static Visibility,
        ),
    >,
    map_catalog: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    clips: Option<Res<'w, assets::StagedXAnims>>,
    cat: Option<Res<'w, assets::ReadiedFpvMeshes>>,
    shown: Option<Res<'w, net::ShownWorldFrame>>,
    clock: Option<Res<'w, net::FrameClock>>,
}

/// One frame's view of the assets and the time the script models are posed at.
struct ScriptFrame<'a> {
    fpv: &'a asset_model::FirstPersonMeshRoster,
    clips: Option<&'a assets::StagedXAnims>,
    clock: Option<&'a net::FrameClock>,
    late: f32,
}

impl ScriptFrame<'_> {
    /// The pose of each part of a script model: from its animation, else (for
    /// a lone vehicle body) from its wheels.
    fn pose_parts(
        &self,
        state: &xmodel_runtime::RigPoseRecord,
        entnum_in: Option<u32>,
        at: &Transform,
        cars: &mut T6Cars,
    ) -> Option<Vec<Option<Arc<PosedVerts>>>> {
        let animated = self
            .clips
            .and_then(|clips| pose_t6_composition(self.fpv, &clips.0, state, self.late));
        animated.or_else(|| entnum_in.and_then(|id| pose_t6_car(self.fpv, state, id, at, cars)))
    }

    /// Adds one entry per part of `state` that the catalog has a model for.
    fn push_parts(
        &self,
        items: &mut Vec<T6MissileModel>,
        state: &xmodel_runtime::RigPoseRecord,
        posed: Option<Vec<Option<Arc<PosedVerts>>>>,
        (placement, origin, light_at): (Mat4, [f32; 3], [f32; 3]),
    ) {
        let hidden = *state.hidden_mask.words();
        let hides_any = hidden.iter().any(|word| *word != 0);
        let mut bones_before = 0u32;
        for (slot, part) in state.composition.models.iter().enumerate() {
            let Some(entry) = self.fpv.by_label(AssetCategoryScope::T6, &part.model) else {
                warn_model_missing_once(&part.model);
                continue;
            };
            let joint_start_offset = bones_before;
            bones_before += u32::try_from(entry.bone_rig.bones.len()).unwrap_or(0);
            let part_pose = posed
                .as_ref()
                .and_then(|all| all.get(slot).cloned().flatten());
            items.push(T6MissileModel {
                hide: hides_any.then_some((hidden, joint_start_offset)),
                posed: part_pose,
                light_at,
                eye_luminance: state.eye_luminance,
                ..T6MissileModel::from_entry(entry, placement, origin)
            });
        }
    }
}

pub(super) fn collect_t6_script_models(
    sources: ScriptModelSources,
    mut out: ResMut<T6ScriptModels>,
    mut cars: Local<T6Cars>,
) {
    let _slow = SlowGuard::new("collect_t6_script_models");
    out.items.clear();
    cars.cars.values_mut().for_each(|car| car.seen = false);
    let ScriptModelSources {
        models,
        loose,
        map_catalog,
        clips,
        cat: scene_cat,
        shown,
        clock,
    } = sources;
    let Some(scene_cat) = scene_cat else {
        return;
    };
    let frame = ScriptFrame {
        fpv: &scene_cat.0,
        clips: clips.as_deref(),
        clock: clock.as_deref(),
        late: seconds_past_snapshot(shown.as_deref(), clock.as_deref()),
    };
    let authored = shown
        .as_deref()
        .map(script_prop::presented_dobj_states)
        .unwrap_or_default();
    let naming = script_model_log_due();

    for (instance, at, vis) in &models {
        if naming {
            let parts = &instance.puppet_state.composition.models;
            let names: Vec<&str> = parts.iter().map(|part| part.model.as_str()).collect();
            log_script_model(&names, at.translation, vis);
        }
        if *vis == Visibility::Hidden {
            continue;
        }
        // Animation times come from the same snapshot the frame's time is
        // measured from. The owner's own copy is refreshed a frame after a
        // new snapshot lands, so using it would step the pose back for that
        // frame and make every walk hitch at the snapshot rate.
        let state = instance
            .server_owned_model_owner
            .and_then(|id| authored.get(&id).copied())
            .unwrap_or(&instance.puppet_state);
        let (placement, origin) = (at.to_matrix(), at.translation.to_array());
        // bo2zm: a scene prop the map animates (an fxanim: cupboards, shutters, curtains)
        // reads its light at its own light point instead, the model's `lightingOriginOffset`
        // from its origin ((0, 0, 0.5) on every Nuketown model measured): the west-house
        // kitchen cupboards' bounds reach a helper box outside the house, so their middle sat
        // behind the wall in the night air and their doors drew near black.
        let first = state.composition.models.first();
        let light_at = match first.is_some_and(|part| part.model.starts_with("fxanim_")) {
            true => placement.transform_point3(Vec3::new(0.0, 0.0, 0.5)).to_array(),
            false => first
                .and_then(|part| bounds_middle(frame.fpv, &part.model, &placement))
                .unwrap_or(origin),
        };
        let entnum = instance.game_actor_number.map(u32::from);
        log_pose_sample(
            state,
            entnum.unwrap_or(u32::MAX),
            frame.clock,
            frame.late,
            origin,
        );
        // TODO(rw-zm): a limp body (`state.ragdoll`, Thunder Gun / Jet Gun) falls by
        // how the ragdoll is posed: `zm_ragdoll::Ragdolls::pose` takes the id, the ragdoll, the dobj, its placement, the clock and the clip
        // from the pose it had when it went limp (Zombies' `T6Limp` in `pose_t6_composition`).
        // Trunk's script_model pass already poses limp bodies (script_model/limp.rs); port it
        // here only if this fallback pass still draws them on their own.
        let posed = frame.pose_parts(state, entnum, at, &mut cars);
        let spun = posed.as_ref().and(entnum).and_then(|id| cars.spun_state(id, state));
        let state = spun.as_ref().unwrap_or(state);
        frame.push_parts(&mut out.items, state, posed, (placement, origin, light_at));
    }
    cars.cars.retain(|_, car| car.seen);

    // Loose pieces the map's own model list cannot draw (a mannequin's
    // broken-off head): their model from the same catalog, wherever the
    // piece's physics has it.
    let map = map_catalog.as_deref();
    let map_draws = |piece: &render_scene::LevelDebrisInstance| {
        let known = map.and_then(|map| map.get(&piece.active_mesh));
        !matches!(
            known,
            None | Some(asset_world::LevelModelPlacementAsset::Unavailable { .. })
        )
    };
    for (piece, at, visibility) in &loose {
        if piece.gone || *visibility == Visibility::Hidden || map_draws(piece) {
            continue;
        }
        let found = frame.fpv.by_label(AssetCategoryScope::T6, &piece.active_mesh.0);
        let origin = at.translation.to_array();
        log_loose_piece(&piece.active_mesh.0, found.is_some(), origin);
        let Some(entry) = found else { continue };
        out.items
            .push(T6MissileModel::from_entry(entry, at.to_matrix(), origin));
    }
}

/// The clip a projectile's client script plays on it, for an authoritative
/// row.
pub(super) fn row_client_clip(
    row: &ShownProjectile,
    registry: &asset_game::WeaponLedger,
    time: i32,
) -> Option<(&'static str, f32)> {
    let ShownProjectile::HostDecided(shot) = row else {
        return None;
    };
    let age = time.saturating_sub(shot.spawn_time_ms);
    missile_client_clip(registry.label_for(shot.gun), age)
}

/// Logs each projectile weapon (and its model) the first time one is drawn.
pub(super) fn note_missile_drawn_once(
    registry: &asset_game::WeaponLedger,
    weapon_idx: u32,
    xmodel: &str,
) {
    static ANNOUNCED: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut announced) = ANNOUNCED.lock() else {
        return;
    };
    if announced.contains(&weapon_idx) {
        return;
    }
    announced.push(weapon_idx);
    diag::info!(
        World,
        "t6 missile drawn: {} as {xmodel}",
        registry.label_for(weapon_idx)
    );
}

/// Poses a projectile in the given client-script clip, noting once whether
/// the clip was available.
pub(super) fn pose_missile_clip(
    catalog: &asset_model::FirstPersonMeshRoster,
    clips: Option<&assets::StagedXAnims>,
    mdl: &str,
    anim_clip: &str,
    progress: f32,
) -> Option<Arc<PosedVerts>> {
    let single_clip = xmodel_runtime::RigPoseRecord::single_clip(
        mdl.to_owned(),
        anim_clip.to_owned(),
        1,
        1,
        progress,
    );
    let first_model = |all: Vec<Option<Arc<PosedVerts>>>| all.into_iter().next().flatten();
    let posed = clips
        .and_then(|clips| pose_t6_composition(catalog, &clips.0, &single_clip, 0.0))
        .and_then(first_model);
    static NOTED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    let playing = posed.is_some();
    if !NOTED[usize::from(playing)].swap(true, Ordering::Relaxed) {
        let outcome = match playing {
            true => "plays",
            false => "not loaded, drawn at rest",
        };
        info!(World, "t6 missile clip {anim_clip} on {mdl}: {outcome}");
    }
    posed
}

/// What the missile collection reads and does not change.
#[derive(SystemParam)]
pub(super) struct MissileSources<'w> {
    arsenal: Option<Res<'w, assets::StagedWeapons>>,
    listing: Option<Res<'w, assets::ReadiedFpvMeshes>>,
    clips: Option<Res<'w, assets::StagedXAnims>>,
    shown: Option<Res<'w, net::ShownWorldFrame>>,
    clock: Option<Res<'w, net::FrameClock>>,
}

/// Every projectile of this game the snapshot presents, with its model from
/// the first-person catalog (the projectile catalog has none).
pub(super) fn collect_t6_missiles(sources: MissileSources, mut out: ResMut<T6MissileModels>) {
    let _slow = SlowGuard::new("collect_t6_missiles");
    out.items.clear();
    let MissileSources {
        arsenal,
        listing: catalog,
        clips: clip_list,
        shown,
        clock,
    } = sources;
    let (Some(snapshot), Some(arsenal), Some(catalog)) =
        (shown.as_deref(), arsenal.as_deref(), catalog.as_deref())
    else {
        return;
    };
    let projectiles_in = snapshot.presented_projectiles();
    if projectiles_in.is_empty() {
        return;
    }
    let registry = &arsenal.0;
    let now = missile_time(snapshot, clock.as_deref());
    for shell in projectiles_in {
        let weapon = shell.weapon();
        // Nothing is drawn before the launch time.
        let launched = now >= shell.spawn_moment();
        let of_this_game = registry.scope_for(weapon) == Some(AssetCategoryScope::T6);
        let model = registry.projectile_mesh_for(weapon);
        let (true, true, Some(model)) = (launched, of_this_game, model) else {
            continue;
        };
        let Some(entry) = catalog.0.by_label(AssetCategoryScope::T6, model) else {
            continue;
        };
        note_missile_drawn_once(registry, weapon, model);
        let origin = missile_origin(snapshot, shell, now);
        let facing = entity_t6::evaluate_trajectory(&shell.spin(), now);
        let posed = row_client_clip(shell, registry, now).and_then(|(clip, progress)| {
            pose_missile_clip(&catalog.0, clip_list.as_deref(), model, clip, progress)
        });
        out.items.push(T6MissileModel {
            posed,
            ..T6MissileModel::from_entry(entry, rocket_model_to_world(origin, facing), origin)
        });
    }
}

// ---------------------------------------------------------------------------
// Missiles as scene entities
// ---------------------------------------------------------------------------
