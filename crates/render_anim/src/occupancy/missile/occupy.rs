use super::*;

pub(super) fn occupy_missile_scene_ents(
    presented: Option<Res<net::PresentedSnapshot>>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    projectile_meshes: Option<Res<assets::PreparedProjectileMeshes>>,
    mut occupancy: ResMut<MissileOccupancy>,
    mut scene_skels: ResMut<AnimDObjSceneSkels>,
    mut submissions: MessageWriter<AnimDObjSceneSubmission>,
    cg_clock: Option<Res<net::FrameClock>>,
    local: Option<Res<net::LocalPresentClient>>,
) {
    let _slow = SlowGuard::new("occupy_missile_scene_ents");
    occupancy.rows.clear();
    let Some(presented) = presented.as_deref() else {
        return;
    };
    let rows = presented.presented_projectiles();
    let time = missile_time(presented, cg_clock.as_deref());
    let registry = weapons.as_ref().map(|w| &w.0).filter(|reg| !reg.is_empty());
    let Some(catalog) = missile_pose_catalog(projectile_meshes.as_deref()) else {
        return;
    };
    // The missile the local player is flying is seen from inside, not drawn.
    let piloted = local
        .and_then(|local| {
            presented
                .snapshot()?
                .meta
                .for_client(local.0)?
                .remote_missile
        })
        .map(|link| link.projectile);

    for (index, row) in rows.iter().enumerate() {
        if time < row.launch_time() {
            continue;
        }
        if piloted.is_some() && row.authoritative_id() == piloted {
            continue;
        }
        let Some(name) = registry.and_then(|reg| reg.projectile_model_of(row.weapon())) else {
            continue;
        };
        let namespace = registry
            .and_then(|reg| reg.namespace_of(row.weapon()))
            .unwrap_or(AssetNamespace::T6);
        let Some(entry) = catalog.get(namespace, name) else {
            continue;
        };
        if entry.skel.pose.is_none() {
            continue;
        }
        let origin = missile_origin(presented, row, time);
        let lighting_origin = missile_lighting_origin(origin);
        let angles = entity_t6::evaluate_trajectory(&row.apos(), time);
        let entnum = match row {
            PresentedProjectile::Authoritative(p) => u32::try_from(p.entnum).ok(),
            PresentedProjectile::Predicted { .. } => None,
        };
        if let Some(entnum) = entnum {
            submissions.write(AnimDObjSceneSubmission {
                render_fx_flags: 0,
                has_tree: false,
                origin,
                lighting_origin,
                radius: entry.skel.radius,
                entnum,
                quat: Some(scene_quat_from_angles(angles)),
                occupy_model_n: 1,
                models: vec![scene_skels.shared(name, &entry.skel, 0)],
                hide_part_bits: [0; 5],
                store_skin: true,
            });
        }
        let ignited = match (row, registry) {
            (PresentedProjectile::Authoritative(p), Some(reg)) => {
                reg.facts_of(p.weapon).is_none_or(|facts| {
                    time >= p.spawn_time_ms.saturating_add(facts.ignition_delay_ms)
                })
            }
            (PresentedProjectile::Authoritative(_), None) => true,
            (PresentedProjectile::Predicted { .. }, _) => false,
        };
        occupancy.rows.push(OccupiedMissile {
            index,
            id: row.authoritative_id().map(|id| id.0),
            entnum,
            weapon: row.weapon(),
            ignited,
            origin,
            angles,
            name: name.to_owned(),
            namespace,
            lighting_origin,
            lighting_owner: match row.authoritative_id() {
                Some(id) => ModelLightingOwner::Missile(id.0),
                None => ModelLightingOwner::PredictedMissile {
                    owner: row.owner().0,
                    weapon: row.weapon(),
                },
            },
            clip: registry.and_then(|reg| row_client_clip(row, reg, time)),
        });
    }
}

/// How a missile is posed: at rest, or in the clip its client script plays.
/// A clip this build has not loaded falls back to rest.
pub(super) fn missile_pose_request(
    row: &OccupiedMissile,
    xanims: Option<&assets::PreparedXAnims>,
) -> Option<xmodel_runtime::DObjPoseRequest> {
    let at_rest = || xmodel_runtime::DObjSemanticState::bind_pose(row.name.clone(), 1, 1);
    let wanted = match row.clip {
        Some((clip, progress)) => xmodel_runtime::DObjSemanticState::one_leaf(
            row.name.clone(),
            clip.to_owned(),
            1,
            1,
            progress,
        ),
        None => at_rest(),
    };
    wanted
        .resolve_request(|name| xanims?.0.clip(AssetNamespace::T6, name))
        .ok()
        .or_else(|| at_rest().resolve_request(|_| None).ok())
}
