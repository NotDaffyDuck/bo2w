use super::*;

/// Every scripted model publishes its bones (bind pose or animated) where it
/// stands, so an effect played on one of its tags (a power-up's glow, a perk
/// machine's light) rides it and goes when it goes.
pub(super) fn publish_t6_script_model_poses(
    fpv: Option<Res<assets::ReadiedFpvMeshes>>,
    animation_refs: Option<Res<assets::StagedXAnims>>,
    owners: Query<(&render_scene::LevelScriptModelInstance, &Transform)>,
    mut dobj_poses: ResMut<HostSkeletonFrame>,
    (cg_clock, shown_frame): (
        Option<Res<net::FrameClock>>,
        Option<Res<net::ShownWorldFrame>>,
    ),
) {
    let Some(meshes) = fpv.as_deref().map(|fpv| &fpv.0) else {
        return;
    };
    let clips = animation_refs.as_deref().map(|x| &x.0);
    // Bones at the client's clock, as the body is drawn.
    let ahead = script_prop::anim_ahead_secs(cg_clock.as_deref(), shown_frame.as_deref());
    let snapshot_states = shown_frame
        .as_deref()
        .map(script_prop::presented_dobj_states);
    let snapshot_states = snapshot_states.unwrap_or_default();
    let loops = |clip: &str| Some(clips?.clip(AssetCategoryScope::T6, clip)?.looping);
    for (owner, transform) in &owners {
        let Some(entnum) = owner.game_actor_number else {
            continue;
        };
        let from_world_state = owner.server_owned_model_owner.and_then(|o| snapshot_states.get(&o));
        let current = from_world_state.copied().unwrap_or(&owner.puppet_state);
        let advanced = script_prop::advance_script_anim(current, ahead, loops);
        let Some(bones) = t6_composition_bones(meshes, clips, advanced.as_ref().unwrap_or(current))
        else {
            continue;
        };
        let world = transform.to_matrix();
        let _ = dobj_poses.publish(u32::from(entnum), true, 0, world, &bones);
    }
}

pub(super) fn emit_rocket_skeleton_poses(
    occupancy: Res<RocketSceneSlot>,
    animation_refs: Option<Res<assets::StagedXAnims>>,
    prepared: Res<PreparedModelMaterials>,
    mut dobj_poses: ResMut<HostSkeletonFrame>,
) {
    for row_in in occupancy.rows.iter() {
        let posed = row_in.entnum.and_then(|entnum| {
            let dobj = prepared.projectile_dobj(row_in.namespace, &row_in.name)?;
            let request = missile_pose_request(row_in, animation_refs.as_deref())?;
            let at_origin = Mat4::IDENTITY;
            let bones =
                xmodel_runtime::pose_skeleton_with_aim(dobj, &request, at_origin, |_, _, _| {});
            Some((entnum, bones.ok()?))
        });
        if let Some((entnum, bones)) = posed {
            let world = rocket_model_to_world(row_in.origin, row_in.rot);
            let _ = dobj_poses.publish(entnum, true, 0, world, &bones);
        }
    }
}

/// bo2zm: a BO2 missile's bones under its entity number, the way the other
/// missiles' are: an effect bolted to one (the planted claymore's lasers on
/// its tag_fx) was otherwise lost the frame it began, as no pose was there.
/// Reads the model from the first-person catalog, so it covers the missiles
/// whose model the projectile catalog lacks; a missile the pass above already
/// published is left alone.
pub(super) fn publish_t6_missile_dobj_poses(
    presented: Option<Res<net::ShownWorldFrame>>,
    weapons_in: Option<Res<assets::StagedWeapons>>,
    fpv: Option<Res<assets::ReadiedFpvMeshes>>,
    cg_clock: Option<Res<net::FrameClock>>,
    occupancy: Res<RocketSceneSlot>,
    mut dobjs: Local<HashMap<String, Option<Arc<PosedSkeleton>>>>,
    mut dobj_poses: ResMut<HostSkeletonFrame>,
) {
    let (Some(snapshot), Some(weapons_in), Some(fpv)) =
        (presented.as_deref(), weapons_in.as_deref(), fpv.as_deref())
    else {
        return;
    };
    let registry = &weapons_in.0;
    let shown = snapshot.presented_projectiles();
    if shown.is_empty() { return; }
    let time = missile_time(snapshot, cg_clock.as_deref());
    for row in shown.iter() {
        let ShownProjectile::HostDecided(projectile) = row else {
            continue;
        };
        let Ok(entnum) = u32::try_from(projectile.ent_no) else {
            continue;
        };
        if registry.scope_for(row.weapon()) != Some(AssetCategoryScope::T6)
            || occupancy.rows.iter().any(|done| done.entnum == Some(entnum))
        {
            continue;
        }
        let Some(name) = registry.projectile_mesh_for(row.weapon()) else {
            continue;
        };
        let dobj = dobjs
            .entry(name.to_owned()).or_insert_with(|| {
                let pose = fpv.0.by_label(AssetCategoryScope::T6, name)?.bone_rig.pose.as_ref()?;
                PosedSkeleton::build(&[(pose, None)]).ok().map(Arc::new)
            })
            .clone();
        let Some(dobj) = dobj else {
            continue;
        };
        let mut locals = dobj.rest_local_pose();
        dobj.copy_mirrored_parts(&mut locals);
        let ident = Mat4::IDENTITY;
    let bones = dobj.world_pose(&locals, ident);
        let origin = missile_origin(snapshot, row, time);
        let angles = entity_t6::evaluate_trajectory(&row.spin(), time);
        let _ = dobj_poses.publish(
            entnum,
            true,
            0,
            rocket_model_to_world(origin, angles),
            &bones,
        );
    }
}

// ---------------------------------------------------------------------------
// Posing and drawing
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub(super) struct RocketPoseOutput {
    pub(super) rows: Vec<RocketPoseDone>,
}

pub(super) struct RocketPoseDone {
    pub(super) index: usize,
    pub(super) entnum: Option<u32>,
    pub(super) origin: [f32; 3],
    pub(super) angles: [f32; 3],
    pub(super) name: String,
    pub(super) namespace: AssetCategoryScope,
    pub(super) illum_origin: [f32; 3],
    pub(super) lit_by_owner: ModelIllumOwner,
    pub(super) surfaces: Vec<SkinnedMeshFacet>,
}

/// Whether a missile's scene entity is hidden, and the skin entries it holds.
pub(super) fn scene_entry(
    scene: &VisualStage, bay: Option<u32>,
) -> (bool, &[dpvs_t6::EntitySkinSlotRecord]) {
    match bay {
        None => (false, &[]),
        Some(entnum) => {
            let skinned = scene.entity_bone_surface_set(entnum);
            let entries = skinned.map_or(&[][..], |surfs| surfs.entries.as_slice());
            (scene.entity_not_drawn(entnum), entries)
        }
    }
}
