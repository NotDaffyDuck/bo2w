use super::*;

pub(super) fn pose_missiles(
    occupancy: Res<MissileOccupancy>,
    projectile_meshes: Option<Res<assets::PreparedProjectileMeshes>>,
    gfx: Res<HostGfxScene>,
    prepared: Res<PreparedModelMaterials>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    mut product: ResMut<MissilePoseProduct>,
) {
    let _slow = SlowGuard::new("pose_missiles");
    product.rows.clear();
    let Some(catalog) = missile_pose_catalog(projectile_meshes.as_deref()) else {
        return;
    };
    for row in &occupancy.rows {
        let (hidden, skin_entries) = scene_entry(&gfx.scene, row.entnum);
        if hidden {
            continue;
        }
        let Some(entry) = catalog.get(row.namespace, &row.name) else {
            continue;
        };
        let Some(dobj) = prepared.projectile_dobj(row.namespace, &row.name) else {
            continue;
        };
        let Some(request) = missile_pose_request(row, xanims.as_deref()) else {
            continue;
        };
        let Some((surfaces, _)) = pose_script_dobj_with_materials(
            None,
            &[&entry.skel],
            dobj,
            &request,
            None,
            skin_entries,
        ) else {
            continue;
        };
        product.rows.push(MissilePosed {
            index: row.index,
            entnum: row.entnum,
            origin: row.origin,
            angles: row.angles,
            name: row.name.clone(),
            namespace: row.namespace,
            lighting_origin: row.lighting_origin,
            lighting_owner: row.lighting_owner,
            surfaces,
        });
    }
}

pub(super) fn append_missile_draws(
    product: Res<MissilePoseProduct>,
    projectile_meshes: Option<Res<assets::PreparedProjectileMeshes>>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    atpoint: Res<render_scene::DynAtPointLookup>,
    tess: Option<Res<TessMaterials>>,
    facts: Res<WorldPresentFacts>,
    mut plan: ResMut<MissileDrawPlan>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
    prepared: Res<PreparedModelMaterials>,
    mut staged: Local<MissileDrawPlan>,
    mut draws: Local<Vec<XModelSurfaceDraw>>,
    mut owners: Local<Vec<MissileOwnerDraw>>,
) {
    let _slow = SlowGuard::new("append_missile_draws");
    // Missiles are rebuilt from scratch every frame, so the rebuild goes to a
    // staging plan whose allocations survive the frame; `plan` keeps the
    // revisions, which is the part a consumer reads.
    let staging = &mut *staged;
    staging.clear_rebuild();
    draws.clear();
    owners.clear();
    let (Some(catalog), Some(_), Some(tess), true) = (
        missile_pose_catalog(projectile_meshes.as_deref()),
        missile_lighting_atlas(atlas.as_deref(), None),
        tess.as_deref(),
        facts.spawned,
    ) else {
        plan.publish_rebuild(staging, &mut draws, &mut owners);
        return;
    };
    for row in &product.rows {
        let Some(entry) = catalog.get(row.namespace, &row.name) else {
            continue;
        };
        let materials: Vec<Option<SmodelPassMaterial>> = row
            .surfaces
            .iter()
            .map(|surface| {
                let authored = entry.material_index(surface.surface_index)?;
                prepared
                    .projectile_material(&tess.catalog, authored)
                    .cloned()
            })
            .collect();
        if materials.iter().all(Option::is_none) {
            continue;
        }
        let radius = entry.skel.radius;
        let pending_lighting = Some(lighting_requests.request(ModelLightingRequest {
            owner: row.lighting_owner,
            origin: row.lighting_origin,
            lookup_fallback:
                atpoint.fallback(row.lighting_origin, radius.map(radius_lighting_box_half)),
        }));
        let placed = append_missile_surfaces(staging, &row.surfaces, &materials);
        if placed.is_empty() {
            continue;
        }
        let object_id = XMODEL_OBJECT_ID_MISSILE_BASE.saturating_add(row.index as u16);
        owners.push(MissileOwnerDraw {
            object_id,
            model: row.name.clone(),
        });
        let world_from_local = missile_world_from_local(row.origin, row.angles);
        let caster_bound = radius.map(|radius| render_scene::XModelCasterBound {
            origin: row.origin,
            radius: radius.max(1.0),
        });
        draws.extend(
            placed
                .into_iter()
                .map(|(surface, material)| XModelSurfaceDraw {
                    surface,
                    material,
                    world_from_local,
                    lighting_handle: 0,
                    pending_lighting,
                    colour_refusal: None,
                    object_id,
                    scene_light_index: 0,
                    reflection_probe_index: 0,
                    packed_lighting: None,
                    is_scope: false,
                    scene_entnum: row.entnum,
                    caster_bound,
                }),
        );
    }
    plan.publish_rebuild(staging, &mut draws, &mut owners);
}
