use super::*;

/// Seconds the frame is past the snapshot that the animation times came in
/// with (at most a quarter second), so an animated model plays on smoothly
/// between snapshots.
pub(super) fn seconds_past_snapshot(
    presented: Option<&net::PresentedSnapshot>,
    clock: Option<&net::FrameClock>,
) -> f32 {
    match (presented.and_then(|p| p.snapshot()), clock) {
        (Some(snapshot), Some(clock)) if clock.started() => {
            ((clock.time() - sim::level_time_ms(snapshot.tick)) as f32 / 1000.0).clamp(0.0, 0.25)
        }
        _ => 0.0,
    }
}

/// Midpoint of a model's bounds, placed in the world; `None` without bounds.
pub(super) fn bounds_middle(
    fpv: &asset_model::FpvMeshCatalog,
    model: &str,
    world_from_local: &Mat4,
) -> Option<[f32; 3]> {
    let (lo, hi) = fpv.get(AssetNamespace::T6, model)?.skel.bounds?;
    let middle = (Vec3::from_array(lo) + Vec3::from_array(hi)) * 0.5;
    Some(world_from_local.transform_point3(middle).to_array())
}

pub(super) fn collect_t6_script_models(
    fpv: Option<Res<assets::PreparedFpvMeshes>>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    presented: Option<Res<net::PresentedSnapshot>>,
    cg_clock: Option<Res<net::FrameClock>>,
    owners: Query<(
        &render_scene::WorldScriptModelInstance,
        &Transform,
        &Visibility,
    )>,
    map_models: Option<Res<asset_world::MapXModelSceneCatalog>>,
    pieces: Query<(&render_scene::WorldDynEntInstance, &Transform, &Visibility)>,
    mut out: ResMut<T6ScriptModels>,
    mut cars: Local<T6Cars>,
) {
    let _slow = SlowGuard::new("collect_t6_script_models");
    out.items.clear();
    for car in cars.cars.values_mut() {
        car.seen = false;
    }
    let Some(fpv) = fpv else {
        return;
    };
    let since_snapshot = seconds_past_snapshot(presented.as_deref(), cg_clock.as_deref());
    let snapshot_states = presented
        .as_deref()
        .map(script_model::presented_dobj_states)
        .unwrap_or_default();
    let log_names = script_model_log_due();

    for (owner, transform, visibility) in &owners {
        if log_names {
            let names: Vec<&str> = owner
                .dobj_state
                .composition
                .models
                .iter()
                .map(|d| d.model.as_str())
                .collect();
            log_script_model(&names, transform.translation, visibility);
        }
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let world_from_local = transform.to_matrix();
        let origin = transform.translation.to_array();
        // Animation times come from the same snapshot the frame's time is
        // measured from. The owner's own copy is refreshed a frame after a
        // new snapshot lands, so using it would step the pose back for that
        // frame and make every walk hitch at the snapshot rate.
        let state = owner
            .authority_owner
            .and_then(|o| snapshot_states.get(&o).copied())
            .unwrap_or(&owner.dobj_state);
        let light_at = state
            .composition
            .models
            .first()
            .and_then(|desc| bounds_middle(&fpv.0, &desc.model, &world_from_local))
            .unwrap_or(origin);
        log_pose_sample(
            state,
            owner.gentity_number.map_or(u32::MAX, u32::from),
            cg_clock.as_deref(),
            since_snapshot,
            origin,
        );
        let posed = xanims
            .as_ref()
            .and_then(|x| pose_t6_composition(&fpv.0, &x.0, state, since_snapshot))
            .or_else(|| {
                let id = owner.gentity_number.map(u32::from)?;
                pose_t6_car(&fpv.0, state, id, transform, &mut cars)
            });
        let hide = *state.hide_part_bits.words();
        let any_hidden = hide.iter().any(|word| *word != 0);
        let mut bone_base = 0u32;
        for (model, desc) in state.composition.models.iter().enumerate() {
            let base = bone_base;
            let Some(entry) = fpv.0.get(AssetNamespace::T6, &desc.model) else {
                warn_model_missing_once(&desc.model);
                continue;
            };
            bone_base += u32::try_from(entry.skel.bones.len()).unwrap_or(0);
            out.items.push(T6MissileModel {
                light_at,
                posed: posed.as_ref().and_then(|p| p.get(model).cloned().flatten()),
                hide: any_hidden.then_some((hide, base)),
                ..T6MissileModel::from_entry(entry, world_from_local, origin)
            });
        }
    }
    cars.cars.retain(|_, car| car.seen);

    // Loose pieces the map's own model list cannot draw (a mannequin's
    // broken-off head): their model from the same catalog, wherever the
    // piece's physics has it.
    for (piece, transform, visibility) in &pieces {
        if piece.dead || matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let map_draws_it = map_models.as_deref().is_some_and(|map| {
            !matches!(
                map.get(&piece.current_model),
                None | Some(asset_world::MapXModelSceneAsset::Unavailable { .. })
            )
        });
        if map_draws_it {
            continue;
        }
        let entry = fpv.0.get(AssetNamespace::T6, &piece.current_model.0);
        let origin = transform.translation.to_array();
        log_loose_piece(&piece.current_model.0, entry.is_some(), origin);
        if let Some(entry) = entry {
            out.items.push(T6MissileModel::from_entry(
                entry,
                transform.to_matrix(),
                origin,
            ));
        }
    }
}

/// The clip a projectile's client script plays on it, for an authoritative
/// row.
pub(super) fn row_client_clip(
    row: &PresentedProjectile,
    registry: &asset_game::WeaponRegistry,
    time: i32,
) -> Option<(&'static str, f32)> {
    match row {
        PresentedProjectile::Authoritative(p) => missile_client_clip(
            registry.name_of(p.weapon),
            time.saturating_sub(p.spawn_time_ms),
        ),
        PresentedProjectile::Predicted { .. } => None,
    }
}

/// Logs each projectile weapon (and its model) the first time one is drawn.
pub(super) fn note_missile_drawn_once(
    registry: &asset_game::WeaponRegistry,
    weapon: u32,
    model: &str,
) {
    static SEEN: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut seen) = SEEN.lock()
        && !seen.contains(&weapon)
    {
        seen.push(weapon);
        diag::info!(
            World,
            "t6 missile drawn: {} as {model}",
            registry.name_of(weapon)
        );
    }
}

/// Poses a projectile in the given client-script clip, noting once whether
/// the clip was available.
pub(super) fn pose_missile_clip(
    fpv: &asset_model::FpvMeshCatalog,
    xanims: Option<&assets::PreparedXAnims>,
    model: &str,
    clip: &str,
    progress: f32,
) -> Option<Arc<PosedVerts>> {
    let state = xmodel_runtime::DObjSemanticState::one_leaf(
        model.to_owned(),
        clip.to_owned(),
        1,
        1,
        progress,
    );
    let posed = xanims
        .and_then(|x| pose_t6_composition(fpv, &x.0, &state, 0.0))
        .and_then(|models| models.into_iter().next().flatten());
    static NOTED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    if !NOTED[usize::from(posed.is_some())].swap(true, Ordering::Relaxed) {
        diag::info!(
            World,
            "t6 missile clip {clip} on {model}: {}",
            if posed.is_some() {
                "plays"
            } else {
                "not loaded, drawn at rest"
            }
        );
    }
    posed
}

/// Every projectile of this game the snapshot presents, with its model from
/// the first-person catalog (the projectile catalog has none).
pub(super) fn collect_t6_missiles(
    presented: Option<Res<net::PresentedSnapshot>>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    fpv: Option<Res<assets::PreparedFpvMeshes>>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    cg_clock: Option<Res<net::FrameClock>>,
    mut out: ResMut<T6MissileModels>,
) {
    let _slow = SlowGuard::new("collect_t6_missiles");
    out.items.clear();
    let (Some(snapshot), Some(weapons), Some(fpv)) =
        (presented.as_deref(), weapons.as_deref(), fpv.as_deref())
    else {
        return;
    };
    let registry = &weapons.0;
    let rows = snapshot.presented_projectiles();
    if rows.is_empty() {
        return;
    }
    let time = missile_time(snapshot, cg_clock.as_deref());
    for row in rows.iter() {
        // Nothing is drawn before the launch time.
        if registry.namespace_of(row.weapon()) != Some(AssetNamespace::T6)
            || time < row.launch_time()
        {
            continue;
        }
        let Some(name) = registry.projectile_model_of(row.weapon()) else {
            continue;
        };
        let Some(entry) = fpv.0.get(AssetNamespace::T6, name) else {
            continue;
        };
        let origin = missile_origin(snapshot, row, time);
        let angles = entity_t6::evaluate_trajectory(&row.apos(), time);
        note_missile_drawn_once(registry, row.weapon(), name);
        let posed = row_client_clip(row, registry, time).and_then(|(clip, progress)| {
            pose_missile_clip(&fpv.0, xanims.as_deref(), name, clip, progress)
        });
        out.items.push(T6MissileModel {
            posed,
            ..T6MissileModel::from_entry(entry, missile_world_from_local(origin, angles), origin)
        });
    }
}

// ---------------------------------------------------------------------------
// Missiles as scene entities
// ---------------------------------------------------------------------------
