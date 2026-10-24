use super::*;

pub(super) fn pose_dyn_ents(
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    pieces: Query<(Entity, &WorldDynEntInstance, &Transform, &Visibility)>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<FpvLens>>,
    lod_skinned: Res<render_scene::LodRampSkinnedDvar>,
    prepared: Res<PreparedModelMaterials>,
    mut product: ResMut<DynEntPoseProduct>,
) {
    let _slow = SlowGuard::new("pose_dyn_ents");
    product.owners.clear();
    if catalog.as_ref().is_some_and(|c| c.is_changed()) {
        product.assets.clear();
    }
    let Some(catalog) = catalog.as_deref() else {
        product.assets.clear();
        return;
    };
    let eye = cameras
        .single()
        .ok()
        .map(|(placement, _, _)| placement.translation());
    let ramp = lod_skinned.args();
    for (entity, inst, transform, visibility) in &pieces {
        if inst.dead {
            continue;
        }
        let Some(asset_world::MapXModelSceneAsset::T5(skel)) = catalog.get(&inst.current_model)
        else {
            continue;
        };
        let skel = skel.as_ref();
        let camera_lod = smodel_camera_lod(skel.lod, inst.lighting_origin, 1.0, eye, ramp);
        if eye.is_some() && camera_lod.is_none() {
            continue;
        }
        let asset_index = match product.asset_index(&inst.current_model, camera_lod) {
            Some(index) => index,
            None => {
                let Some(dobj) = prepared.scene_dobj(&inst.current_model.0) else {
                    continue;
                };
                let bind = xmodel_runtime::DObjSemanticState::bind_pose(
                    inst.current_model.0.clone(),
                    1,
                    1,
                );
                let Ok(request) = bind.resolve_request(|_| None) else {
                    continue;
                };
                let lod_view = DObjLodView {
                    origin: inst.lighting_origin,
                    eye,
                    ramp,
                };
                let Some((surfaces, authored)) = pose_script_dobj_with_materials(
                    Some(catalog),
                    &[skel],
                    dobj,
                    &request,
                    Some(lod_view),
                    &[],
                ) else {
                    continue;
                };
                product.assets.push(DynEntPosedAsset {
                    key: inst.current_model.clone(),
                    camera_lod,
                    surfaces,
                    authored,
                });
                product.assets.len() - 1
            }
        };
        product.owners.push(DynEntPosedOwner {
            entity,
            asset_index,
            model: inst.current_model.0.clone(),
            world_from_local: transform.to_matrix(),
            lighting_origin: inst.lighting_origin,
            camera_hidden: *visibility == Visibility::Hidden,
            radius: skel.radius,
        });
    }
}

pub(super) fn append_dynent_draws(
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    atpoint: Res<DynAtPointLookup>,
    tess: Option<Res<TessMaterials>>,
    facts: Res<WorldPresentFacts>,
    mut plan: ResMut<DynEntDrawPlan>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
    product: Res<DynEntPoseProduct>,
    model_materials: Res<PreparedModelMaterials>,
    mut seen_generation: Local<Option<render_material::MaterialGenerationId>>,
    mut rows: Local<Vec<XModelSurfaceDraw>>,
    mut row_owners: Local<Vec<DynEntOwnerDraw>>,
) {
    let _slow = SlowGuard::new("append_dynent_draws");
    let (Some(catalog), Some(atlas), Some(tess)) = (&catalog, &atlas, &tess) else {
        plan.publish_no_rows();
        return;
    };
    if !facts.spawned {
        plan.publish_no_rows();
        return;
    }
    let generation = tess.catalog.generation_id;
    let inputs_changed = catalog.is_changed()
        || atlas.is_changed()
        || tess.is_changed()
        || *seen_generation != Some(generation);
    if inputs_changed {
        // Start the plan over but keep its revision counters running.
        let kept = (plan.revision, plan.generation);
        *plan = DynEntDrawPlan::default();
        (plan.revision, plan.generation) = kept;
        *seen_generation = Some(generation);
    }
    let tess_catalog = std::sync::Arc::clone(&tess.catalog);
    rows.clear();
    row_owners.clear();

    let mut next_object = 0u16;
    let mut packed_this_frame: Vec<Option<Vec<(u32, u32)>>> = vec![None; product.assets.len()];

    for owner in &product.owners {
        let posed = &product.assets[owner.asset_index];
        let surfaces = if let Some(planned) = plan.asset(&posed.key, posed.camera_lod) {
            planned.surfaces.clone()
        } else if let Some(packed) = &packed_this_frame[owner.asset_index] {
            packed.clone()
        } else {
            let materials: Vec<Option<SmodelPassMaterial>> = posed
                .authored
                .iter()
                .take(posed.surfaces.len())
                .map(|authored| {
                    model_materials
                        .authored(&tess_catalog, (*authored)?)
                        .cloned()
                })
                .collect();
            if materials.iter().all(Option::is_none) {
                continue;
            }
            let packed = append_dynent_asset(
                &mut plan,
                posed.key.clone(),
                posed.camera_lod,
                &posed.surfaces,
                &materials,
            );
            packed_this_frame[owner.asset_index] = Some(packed.clone());
            packed
        };
        if surfaces.is_empty() {
            continue;
        }
        let lighting_box = owner.radius.map(radius_lighting_box_half);
        let pending_lighting = (!owner.camera_hidden).then(|| {
            lighting_requests.request(ModelLightingRequest {
                owner: ModelLightingOwner::DynEnt(owner.entity),
                origin: owner.lighting_origin,
                lookup_fallback: atpoint.fallback(owner.lighting_origin, lighting_box),
            })
        });
        let caster_bound = owner.radius.map(|radius| XModelCasterBound {
            origin: owner.world_from_local.w_axis.truncate().to_array(),
            radius: radius.max(1.0),
        });
        let object_id = XMODEL_OBJECT_ID_DYNENT_BASE.saturating_add(next_object);
        next_object = next_object.saturating_add(1);
        row_owners.push(DynEntOwnerDraw {
            object_id,
            model: owner.model.clone(),
        });
        let refusal = owner
            .camera_hidden
            .then_some(XModelColourRefusal::CameraFrustum);
        rows.extend(
            surfaces
                .into_iter()
                .map(|(surface, material)| XModelSurfaceDraw {
                    surface,
                    material,
                    world_from_local: owner.world_from_local,
                    lighting_handle: 0,
                    pending_lighting,
                    colour_refusal: refusal,
                    object_id,
                    scene_light_index: 0,
                    reflection_probe_index: 0,
                    packed_lighting: None,
                    is_scope: false,
                    scene_entnum: None,
                    caster_bound,
                }),
        );
    }
    if !rows.is_empty() {
        let topology =
            topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.vertices.len());
        let revision = plan.revision;
        plan.revision = stamp_plan_geometry(&mut plan.revisions, revision, topology);
    }
    plan.publish_frame_rows(&mut rows, &mut row_owners);
}
