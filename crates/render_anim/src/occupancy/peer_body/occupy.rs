use super::*;

/// Enters every remote player's body into the scene as a skinned DObj.
pub(super) fn claim_body_scene_ents(
    mut submissions: MessageWriter<SkeletonSceneHandoff>,
    mut skels: ResMut<SceneBonePoseSet>,
    remotes_in: Query<(&Transform, &ClientEntityRecord, &ClientEntityPoseState), (Without<LevelScriptModelInstance>, Without<FirstPersonLens>, Without<FreeRoamCamera>)>,
    cameras_in: Query<&GlobalTransform, With<bevy::camera::Camera3d>>,
    detail_skinned: Res<DetailRampSkinnedTunable>,
    prepared_kits: Res<PreparedRemoteKits>,
    local: Res<HostViewClient>,
    presented_in: Res<ShownWorldFrame>,
    world_gear: Option<Res<ReadiedWorldWeapons>>,
    gear: Option<Res<StagedWeapons>>,
    body_set: Option<Res<ReadiedCharacterBodies>>,
    subject: Option<Res<EyeSubject>>,
) {
    let bodies = match body_set.as_deref() {
        Some(set) => set,
        None => return,
    };
    if let Some(world) = world_gear.as_deref() {
        skels.seed(bodies, world);
    }
    // Once the prepared table matches the loaded assets the kits come from it.
    // Until then each one is built on the spot.
    let table = match (gear.as_deref(), world_gear.as_deref()) {
        (Some(weapons), Some(world)) if prepared_kits.owned_by(bodies, weapons, world) => {
            Some((&*prepared_kits, world))
        }
        _ => None,
    };
    if !any_kit_loaded(bodies) {
        return;
    }
    let eye = cameras_in.single().ok().map(GlobalTransform::translation);
    let gate = draw_gate(&local, &presented_in, subject.as_deref());
    let detail_ramp = detail_skinned.args();
    let find_kit = |axis: bool, held| match table {
        Some((table, world)) => table
            .kit_for(axis, held)
            .and_then(|kit| Some((kit.models(bodies, world)?, kit.rad, kit.mask_part_bits))),
        None => {
            let built =
                build_body_kit_dobj(bodies, gear.as_deref(), world_gear.as_deref(), axis, held, true);
            built.map(|(models, radius)| {
                let hidden = kit_hide_part_bits(&models);
                (models, radius, hidden)
            })
        }
    };

    for (transform, ident, runtime) in &remotes_in {
        let visible = remote_body_draws(ident.number(), runtime, gate);
        let client = match ident.client() {
            Some(client) if visible => client,
            _ => continue,
        };
        let (ffa_team, client_state_team) = client_team(&presented_in, client);
        let held = other_player_pose(runtime).weapon;
        let axis = kit_belongs_to_axis(client_state_team, ffa_team);
        let Some((models_kit, radius, hidden_part_bits)) = find_kit(axis, held) else {
            continue;
        };
        let pos = transform.translation.to_array();
        let mut models = Vec::new();
        for model in &models_kit {
            let level = skel_camera_lod(model.bone_rig, pos, eye, detail_ramp);
            models.push(skels.xmodel(model.name, model.bone_rig, choose_lod_byte(level)));
        }
        submissions.write(SkeletonSceneHandoff {
            occupy_model_tally: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            keep_skinned_verts: true,
            carries_anim_graph: true,
            render_effect_flags: 0,
            hidden_mask: hidden_part_bits,
            radius,
            rot_quat: Some(transform.rotation.to_array()),
            illum_origin: pos,
            origin: pos,
            ent_ix: u32::from(ident.number()),
        });
    }
}

/// True once the body set has a kit for either side.
fn any_kit_loaded(bodies: &ReadiedCharacterBodies) -> bool {
    let pair = bodies.0.kits();
    pair.allies.is_some() || pair.axis.is_some()
}

/// Removes the player marker from an entity that has no body any more.
pub(super) fn drop_remote_marker(commands: &mut Commands, ent: Entity, marked: bool) {
    if marked {
        commands.entity(ent).remove::<(OtherPlayerBody, OtherBodyEffectTags)>();
    }
}

/// Marks the entities that should have a remote body and unmarks the rest.
pub(super) fn refresh_body_entities(
    mut cmds: Commands,
    existing_in: Query<(Entity, &ClientEntityRecord, &ClientEntityPoseState, Option<&OtherPlayerBody>)>,
    local: Res<HostViewClient>,
    shown: Res<ShownWorldFrame>,
    body_set: Option<Res<ReadiedCharacterBodies>>,
    subject: Option<Res<EyeSubject>>,
) {
    let _slow = SlowGuard::new("sync_remote_bodies");
    let Some(set) = body_set.filter(|set| any_kit_loaded(set)) else {
        return;
    };
    let sides = set.0.kits();
    let gate = draw_gate(&local, &shown, subject.as_deref());
    let has_drawable_body = |team_ffa, client_state_team| {
        sides
            .kit(kit_belongs_to_axis(client_state_team, team_ffa))
            .and_then(|kit| set.0.by_label(&kit.torso))
            .is_some_and(|entry| !entry.bone_rig.positions.is_empty() && !entry.bone_rig.bones.is_empty())
    };

    for (id, identity, runtime, marker) in &existing_in {
        let marked = marker.is_some();
        let teams = identity
            .client()
            .filter(|_| remote_body_draws(identity.number(), runtime, gate))
            .map(|client| client_team(&shown, client))
            .filter(|&(ffa_team, client_state_team)| has_drawable_body(ffa_team, client_state_team));
        let Some((ffa_team, client_state_team)) = teams else {
            drop_remote_marker(&mut cmds, id, marked);
            continue;
        };

        let pose = Transform::from_translation(Vec3::from_array(runtime.origin))
            .with_rotation(Quat::from_rotation_z(runtime.angles[1].to_radians()));
        let remote = OtherPlayerBody { ffa_side: ffa_team, player_state_team: client_state_team };
        if marked {
            cmds.entity(id).insert((pose, remote));
        } else {
            cmds.entity(id).insert((remote, pose, Visibility::Inherited));
        }
    }
}
