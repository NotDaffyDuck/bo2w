use super::*;

/// Poses every missile that is in the scene this frame.
pub(super) fn pose_shells(
    mut product: ResMut<RocketPoseOutput>,
    occupancy: Res<RocketSceneSlot>,
    gfx: Res<HostVisualStage>,
    prepared: Res<PreparedModelMaterials>,
    animation_refs: Option<Res<assets::StagedXAnims>>,
    shell_visual_models: Option<Res<assets::ReadiedProjectileMeshes>>,
) {
    let _slow = SlowGuard::new("pose_missiles");
    product.rows.truncate(0);
    let Some(catalog) = rocket_pose_table(shell_visual_models.as_deref()) else {
        return;
    };
    for row_in in occupancy.rows.iter() {
        let (hidden, skin_jobs) = scene_entry(&gfx.scene, row_in.entnum);
        let found = catalog
            .by_label(row_in.namespace, &row_in.name)
            .zip(prepared.projectile_dobj(row_in.namespace, &row_in.name))
            .zip(missile_pose_request(row_in, animation_refs.as_deref()));
        let Some(((entry, dobj), request)) = found.filter(|_| !hidden) else {
            continue;
        };
        let posed = pose_script_skeleton_and_materials(
            None,
            &[&entry.bone_rig],
            dobj,
            &request,
            None,
            skin_jobs,
        );
        let Some((surfs, _)) = posed else {
            continue;
        };
        product.rows.push(RocketPoseDone {
            surfaces: surfs,
            index: row_in.index,
            entnum: row_in.entnum,
            name: row_in.name.clone(),
            namespace: row_in.namespace,
            origin: row_in.origin,
            angles: row_in.rot,
            illum_origin: row_in.illum_origin,
            lit_by_owner: row_in.lit_by_owner,
        });
    }
}

/// Builds the draw list for the posed missiles and publishes it to the plan.
pub(super) fn push_missile_draws(
    mut plan_in: ResMut<ThrownPaintPlan>,
    mut illum_requests: ResMut<ModelIllumRequests>,
    mut staged: Local<ThrownPaintPlan>,
    mut draws: Local<Vec<FigureFacetDraw>>,
    mut owners: Local<Vec<ModelOwnerPaint>>,
    product: Res<RocketPoseOutput>,
    prepared: Res<PreparedModelMaterials>,
    facts: Res<LevelPresentFacts>,
    at_spot: Res<render_scene::LooseAtSpotLookup>,
    tess: Option<Res<StitchMaterials>>,
    atlas: Option<Res<LevelModelIllumAtlas>>,
    shell_visual_models: Option<Res<assets::ReadiedProjectileMeshes>>,
) {
    let _slow = SlowGuard::new("append_missile_draws");
    // Missiles are rebuilt from nothing every frame. The rebuild goes into a
    // staging plan that keeps its allocations between frames, while `plan`
    // holds the revisions that a consumer reads.
    let staging = &mut *staged;
    staging.clear_rebuild();
    draws.truncate(0);
    owners.truncate(0);
    let ready = match facts.spawned {
        true => rocket_pose_table(shell_visual_models.as_deref())
            .zip(rocket_light_atlas(atlas.as_deref(), None))
            .zip(tess.as_deref()),
        false => None,
    };
    let Some(((catalog, _atlas), tess)) = ready else {
        plan_in.publish_rebuild(staging, &mut draws, &mut owners);
        return;
    };
    for row_v in product.rows.iter() {
        let Some(entry) = catalog.by_label(row_v.namespace, &row_v.name) else {
            continue;
        };
        let mats: Vec<Option<PropPassMaterial>> = row_v
            .surfaces
            .iter()
            .map(|surf| {
                let authored = entry.bound_material(surf.surface_slot_number)?;
                prepared.projectile_material(&tess.catalog, authored).cloned()
            })
            .collect::<_>();
        if mats.iter().all(|slot| slot.is_none()) {
            continue;
        }
        let radius = entry.bone_rig.radius;
        let fallback = at_spot.fallback(row_v.illum_origin, radius.map(illum_query_half_size));
        let queued_illum = Some(illum_requests.queue(ModelIllumRequest {
            owner: row_v.lit_by_owner,
            origin: row_v.illum_origin,
            lookup_backup: fallback,
        }));
        let placed = push_projectile_surfaces(staging, &row_v.surfaces, &mats);
        if placed.is_empty() {
            continue;
        }
        let object_tag = DRAW_ID_START_ROCKETS.saturating_add(row_v.index as u16);
        owners.push(ModelOwnerPaint {
            xmodel: row_v.name.clone(),
            object_tag,
        });
        let scene_from_local = rocket_model_to_world(row_v.origin, row_v.angles);
        let caster_bound = radius.map(|r| render_scene::XModelCasterBound {
            reach: r.max(1.0),
            origin: row_v.origin,
        });
        let new_draws = placed.into_iter().map(|(surface, material)| FigureFacetDraw {
            object_tag,
            stage_actor_num: row_v.entnum,
            surface,
            material,
            map_from_self: scene_from_local,
            queued_illum,
            caster_bound,
            illum_handle: 0,
            stage_light_index: 0,
            env_probe_index: 0,
            colour_refusal: None,
            compact_illum: None,
            is_scoped_view: false,
        });
        draws.extend(new_draws);
    }
    plan_in.publish_rebuild(staging, &mut draws, &mut owners);
}
