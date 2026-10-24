use super::*;

/// Every scripted model publishes its bones (bind pose or animated) where it
/// stands, so an effect played on one of its tags (a power-up's glow, a perk
/// machine's light) rides it and goes when it goes.
pub(super) fn publish_t6_script_model_poses(
    fpv: Option<Res<assets::PreparedFpvMeshes>>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    owners: Query<(&render_scene::WorldScriptModelInstance, &Transform)>,
    mut dobj_poses: ResMut<HostDObjPoseFrame>,
    (cg_clock, presented): (
        Option<Res<net::FrameClock>>,
        Option<Res<net::PresentedSnapshot>>,
    ),
) {
    let Some(fpv) = fpv else {
        return;
    };
    // Bones at the client's clock, as the body is drawn.
    let ahead = script_model::anim_ahead_secs(cg_clock.as_deref(), presented.as_deref());
    let snapshot_states = presented
        .as_deref()
        .map(script_model::presented_dobj_states)
        .unwrap_or_default();
    for (owner, transform) in &owners {
        let Some(entnum) = owner.gentity_number else {
            continue;
        };
        let current = owner
            .authority_owner
            .and_then(|o| snapshot_states.get(&o).copied())
            .unwrap_or(&owner.dobj_state);
        let advanced = script_model::advance_script_anim(current, ahead, |clip| {
            xanims
                .as_deref()?
                .0
                .clip(AssetNamespace::T6, clip)
                .map(|c| c.looping)
        });
        let state = advanced.as_ref().unwrap_or(current);
        let Some(bones) = t6_composition_bones(&fpv.0, xanims.as_deref().map(|x| &x.0), state)
        else {
            continue;
        };
        let _ = dobj_poses.publish(u32::from(entnum), true, 0, transform.to_matrix(), &bones);
    }
}

pub(super) fn publish_missile_dobj_poses(
    occupancy: Res<MissileOccupancy>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    prepared: Res<PreparedModelMaterials>,
    mut dobj_poses: ResMut<HostDObjPoseFrame>,
) {
    for row in &occupancy.rows {
        let Some(entnum) = row.entnum else {
            continue;
        };
        let Some(dobj) = prepared.projectile_dobj(row.namespace, &row.name) else {
            continue;
        };
        let Some(request) = missile_pose_request(row, xanims.as_deref()) else {
            continue;
        };
        let Ok(bones) =
            xmodel_runtime::pose_dobj_with_controller(dobj, &request, Mat4::IDENTITY, |_, _, _| {})
        else {
            continue;
        };
        let entity_world = missile_world_from_local(row.origin, row.angles);
        let _ = dobj_poses.publish(entnum, true, 0, entity_world, &bones);
    }
}

// ---------------------------------------------------------------------------
// Posing and drawing
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub(super) struct MissilePoseProduct {
    pub(super) rows: Vec<MissilePosed>,
}

pub(super) struct MissilePosed {
    pub(super) index: usize,
    pub(super) entnum: Option<u32>,
    pub(super) origin: [f32; 3],
    pub(super) angles: [f32; 3],
    pub(super) name: String,
    pub(super) namespace: AssetNamespace,
    pub(super) lighting_origin: [f32; 3],
    pub(super) lighting_owner: ModelLightingOwner,
    pub(super) surfaces: Vec<PosedModelSurface>,
}

/// Whether a missile's scene entity is hidden, and the skin entries it holds.
pub(super) fn scene_entry(
    scene: &GfxScene,
    entnum: Option<u32>,
) -> (bool, &[dpvs_t6::SceneEntSkinEntry]) {
    let Some(entnum) = entnum else {
        return (false, &[]);
    };
    (
        scene.scene_ent_skips_draw(entnum),
        scene
            .scene_ent_skinned_surfs(entnum)
            .map_or(&[][..], |surfs| surfs.entries.as_slice()),
    )
}
