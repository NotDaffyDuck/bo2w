use super::*;

pub(super) fn occupy_remote_scene_ents(
    mut scene_skels: ResMut<AnimDObjSceneSkels>,
    mut scene_submissions: MessageWriter<AnimDObjSceneSubmission>,
    lod_skinned: Res<LodRampSkinnedDvar>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    local: Res<LocalPresentClient>,
    presented: Res<PresentedSnapshot>,
    subject: Option<Res<ViewSubject>>,
    bodies: Option<Res<PreparedBodies>>,
    weapons: Option<Res<PreparedWeapons>>,
    world_weapons: Option<Res<PreparedWorldWeapons>>,
    prepared_kits: Res<PreparedRemoteKits>,
    remotes: Query<(&CEntity, &CEntityRuntime, &Transform), RemoteQueryFilter>,
) {
    let Some(bodies) = bodies.as_deref() else {
        return;
    };
    if let Some(world) = world_weapons.as_deref() {
        scene_skels.seed(bodies, world);
    }
    // The kits are looked up in the prepared table once it matches the
    // loaded assets, and built on the spot until then.
    let prepared = match (weapons.as_deref(), world_weapons.as_deref()) {
        (Some(weapons), Some(world)) if prepared_kits.owned_by(bodies, weapons, world) => {
            Some((&*prepared_kits, world))
        }
        _ => None,
    };
    let kits = bodies.0.kits();
    if kits.allies.is_none() && kits.axis.is_none() {
        return;
    }
    let eye = cameras
        .single()
        .ok()
        .map(|transform| transform.translation());
    let gate = draw_gate(&local, &presented, subject.as_deref());
    let ramp = lod_skinned.args();

    for (identity, runtime, transform) in &remotes {
        if !remote_body_submits(identity.number(), runtime, gate) {
            continue;
        }
        let Some(client) = identity.client() else {
            continue;
        };
        let (ffa_team, client_state_team) = client_team(&presented, client);
        let axis = kit_assignment_is_axis(client_state_team, ffa_team);
        let held = remote_pose_sample(runtime).weapon;
        let (kit_models, radius, hide_part_bits) = match prepared {
            Some((kits, world)) => {
                let Some(kit) = kits.get(axis, held) else {
                    continue;
                };
                let Some(models) = kit.models(bodies, world) else {
                    continue;
                };
                (models, kit.radius, kit.hide_part_bits)
            }
            None => {
                let Some((models, radius)) = occupy_remote_kit_dobj(
                    bodies,
                    weapons.as_deref(),
                    world_weapons.as_deref(),
                    axis,
                    held,
                    true,
                ) else {
                    continue;
                };
                let bits = kit_hide_part_bits(&models);
                (models, radius, bits)
            }
        };
        let origin = transform.translation.to_array();
        let models: Vec<_> = kit_models
            .iter()
            .map(|model| {
                let lod = occupy_lod_byte(skel_camera_lod(model.skel, origin, eye, ramp));
                scene_skels.model(model.name, model.skel, lod)
            })
            .collect();
        scene_submissions.write(AnimDObjSceneSubmission {
            render_fx_flags: 0,
            has_tree: true,
            origin,
            lighting_origin: origin,
            radius,
            entnum: u32::from(identity.number()),
            quat: Some(transform.rotation.to_array()),
            occupy_model_n: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            hide_part_bits,
            store_skin: true,
        });
    }
}

/// Takes the player marker off an entity that no longer has a body.
pub(super) fn drop_remote_marker(commands: &mut Commands, entity: Entity, marked: bool) {
    if marked {
        commands
            .entity(entity)
            .remove::<(RemotePlayer, RemoteFxBolts)>();
    }
}

pub(super) fn sync_remote_bodies(
    mut commands: Commands,
    local: Res<LocalPresentClient>,
    presented: Res<PresentedSnapshot>,
    subject: Option<Res<ViewSubject>>,
    bodies: Option<Res<PreparedBodies>>,
    existing: Query<(Entity, &CEntity, &CEntityRuntime, Option<&RemotePlayer>)>,
) {
    let _slow = SlowGuard::new("sync_remote_bodies");
    let Some(bodies) = bodies else {
        return;
    };
    let kits = bodies.0.kits();
    if kits.allies.is_none() && kits.axis.is_none() {
        return;
    }
    let gate = draw_gate(&local, &presented, subject.as_deref());

    for (entity, identity, runtime, marker) in &existing {
        let marked = marker.is_some();
        let Some(client) = identity.client() else {
            drop_remote_marker(&mut commands, entity, marked);
            continue;
        };
        if !remote_body_submits(identity.number(), runtime, gate) {
            drop_remote_marker(&mut commands, entity, marked);
            continue;
        }
        let (ffa_team, client_state_team) = client_team(&presented, client);
        let usable_body = kits
            .kit(kit_assignment_is_axis(client_state_team, ffa_team))
            .and_then(|kit| bodies.0.get(&kit.body))
            .is_some_and(|entry| !entry.skel.positions.is_empty() && !entry.skel.bones.is_empty());
        if !usable_body {
            drop_remote_marker(&mut commands, entity, marked);
            continue;
        }

        let pose = Transform {
            translation: Vec3::from_array(runtime.origin),
            rotation: Quat::from_rotation_z(runtime.angles[1].to_radians()),
            scale: Vec3::ONE,
        };
        let remote = RemotePlayer {
            ffa_team,
            client_state_team,
        };
        if marked {
            commands.entity(entity).insert((pose, remote));
        } else {
            commands
                .entity(entity)
                .insert((remote, pose, Visibility::default()));
        }
    }
}
