use super::*;

/// The projectile the local player is flying right now, if any. It is seen
/// from the inside, so it never gets a body in the scene.
fn flown_by_local(
    presented: &net::ShownWorldFrame,
    local: Option<&net::HostViewClient>,
) -> Option<sim::ProjectileId> {
    let who = local?.0;
    let link = presented.snapshot()?.meta.for_client(who)?.remote_missile?;
    Some(link.projectile)
}

/// Whether the engine of a projectile has fired by `time`.
fn has_ignited(
    row: &ShownProjectile,
    registry: Option<&asset_game::WeaponLedger>,
    time: i32,
) -> bool {
    let ShownProjectile::HostDecided(live) = row else {
        return false;
    };
    let Some(registry) = registry else {
        return true;
    };
    match registry.summary_for(live.gun) {
        Some(facts) => time >= live.spawn_time_ms.saturating_add(facts.ignition_delay_ms),
        None => true,
    }
}

pub(super) fn occupy_projectile_scene_ents(
    mut occupancy: ResMut<RocketSceneSlot>,
    mut submissions: MessageWriter<SkeletonSceneHandoff>,
    mut skel_cache: ResMut<SceneBonePoseSet>,
    presented: Option<Res<net::ShownWorldFrame>>,
    local: Option<Res<net::HostViewClient>>,
    cg_clock: Option<Res<net::FrameClock>>,
    weapons: Option<Res<assets::StagedWeapons>>,
    shell_visual_models: Option<Res<assets::ReadiedProjectileMeshes>>,
) {
    let _slow = SlowGuard::new("occupy_missile_scene_ents");
    occupancy.rows.truncate(0);
    let Some(presented) = presented.as_deref() else {
        return;
    };
    let Some(catalog) = rocket_pose_table(shell_visual_models.as_deref()) else {
        return;
    };
    let time = missile_time(presented, cg_clock.as_deref());
    let registry = weapons.as_ref().map(|w| &w.0).filter(|reg| !reg.is_empty());
    let flown = flown_by_local(presented, local.as_deref());

    for (index, line_row) in presented.presented_projectiles().iter().enumerate() {
        let id = line_row.host_assigned_id();
        let launched = time >= line_row.spawn_moment();
        if !launched || (flown.is_some() && id == flown) {
            continue;
        }
        let weapon = line_row.weapon();
        let Some(name) = registry.and_then(|reg| reg.projectile_mesh_for(weapon)) else {
            continue;
        };
        let namespace = registry
            .and_then(|reg| reg.scope_for(weapon))
            .unwrap_or(AssetCategoryScope::T6);
        let Some(entry) = catalog.by_label(namespace, name).filter(|e| e.bone_rig.pose.is_some()) else {
            continue;
        };
        let origin = missile_origin(presented, line_row, time);
        let angles = entity_t6::evaluate_trajectory(&line_row.spin(), time);
        let entnum = match line_row {
            ShownProjectile::HostDecided(live) => u32::try_from(live.ent_no).ok(),
            ShownProjectile::Predicted { .. } => None,
        };
        let illum_origin = rocket_light_point(origin);
        if let Some(entnum) = entnum {
            let body = skel_cache.shared(name, &entry.bone_rig, 0);
            submissions.write(SkeletonSceneHandoff {
                ent_ix: entnum,
                origin,
                illum_origin,
                rot_quat: Some(orientation_quat_from_euler(angles)),
                radius: entry.bone_rig.radius,
                models: vec![body],
                occupy_model_tally: 1,
                hidden_mask: [0; 5],
                render_effect_flags: 0,
                carries_anim_graph: false,
                keep_skinned_verts: true,
            });
        }
        let lit_by_owner = match id {
            Some(live) => ModelIllumOwner::Shell(live.0),
            None => ModelIllumOwner::PredictedShell {
                shooter: line_row.owner().0,
                gun: weapon,
            },
        };
        occupancy.rows.push(LiveRocketEntry {
            index,
            entry_id: id.map(|live| live.0),
            entnum,
            weapon,
            ignited: has_ignited(line_row, registry.map(|reg| &**reg), time),
            origin,
            rot: angles,
            name: name.to_owned(),
            namespace,
            illum_origin,
            lit_by_owner,
            reel: registry.and_then(|reg| row_client_clip(line_row, reg, time)),
        });
    }
}

/// How a missile is posed: at rest, or in the clip its client script plays.
/// A clip this build has not loaded falls back to rest.
pub(super) fn missile_pose_request(
    row: &LiveRocketEntry,
    animation_refs: Option<&assets::StagedXAnims>,
) -> Option<xmodel_runtime::ModelPoseQuery> {
    let at_rest = || xmodel_runtime::RigPoseRecord::neutral_pose(row.name.clone(), 1, 1);
    let wanted = match row.reel {
        Some((anim_clip, progress)) => xmodel_runtime::RigPoseRecord::single_clip(
            row.name.clone(),
            anim_clip.to_owned(),
            1,
            1,
            progress,
        ),
        None => at_rest(),
    };
    wanted
        .look_up_pose_query(|name| animation_refs?.0.clip(AssetCategoryScope::T6, name))
        .ok()
        .or_else(|| at_rest().look_up_pose_query(|_| None).ok())
}
