use super::*;

pub(super) fn append_item_draws(
    product: Res<ItemPoseProduct>,
    world_weapons: Option<Res<assets::PreparedWorldWeapons>>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    atpoint: Res<render_scene::DynAtPointLookup>,
    tess: Option<Res<TessMaterials>>,
    facts: Res<WorldPresentFacts>,
    mut plan: ResMut<ItemDrawPlan>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
    model_materials: Res<PreparedModelMaterials>,
    mut seen_generation: Local<Option<render_material::MaterialGenerationId>>,
    mut draws: Local<Vec<XModelSurfaceDraw>>,
    mut owners: Local<Vec<ItemOwnerDraw>>,
) {
    let _slow = SlowGuard::new("append_item_draws");
    draws.clear();
    owners.clear();
    let catalog = world_weapons
        .as_ref()
        .map(|prepared| &prepared.0)
        .filter(|catalog| !catalog.is_empty());
    let (Some(catalog), Some(_), Some(tess_ref), true) =
        (catalog, atlas.as_deref(), tess.as_deref(), facts.spawned)
    else {
        restart_plan(&mut plan, false);
        plan.publish_no_rows();
        *seen_generation = None;
        return;
    };

    let material_generation = tess_ref.catalog.generation_id;
    let changed = world_weapons.as_ref().is_some_and(|w| w.is_changed())
        || atlas.as_ref().is_some_and(|a| a.is_changed())
        || tess.as_ref().is_some_and(|t| t.is_changed())
        || *seen_generation != Some(material_generation);
    if changed {
        restart_plan(&mut plan, true);
        plan.revisions.bump_admission();
        *seen_generation = Some(material_generation);
    }

    for owner in &product.owners {
        let posed = &product.assets[owner.asset_index];
        let Some(asset_index) = plan_asset(&mut plan, posed, catalog, tess_ref, &model_materials)
        else {
            continue;
        };
        let Some(entry) = catalog.get_at(owner.model.order()) else {
            continue;
        };
        let radius = entry.skel.radius;
        let pending_lighting = Some(lighting_requests.request(ModelLightingRequest {
            owner: ModelLightingOwner::Item(owner.entnum),
            origin: owner.lighting_origin,
            lookup_fallback:
                atpoint.fallback(owner.lighting_origin, radius.map(radius_lighting_box_half)),
        }));
        let object_id = XMODEL_OBJECT_ID_ITEM_BASE.saturating_add(owner.slot as u16);
        owners.push(ItemOwnerDraw {
            object_id,
            model: owner.name.clone(),
        });
        let world_from_local = item_world_from_local(owner.origin, owner.angles);
        let caster_bound = radius.map(|radius| render_scene::XModelCasterBound {
            origin: owner.origin,
            radius: radius.max(1.0),
        });
        draws.extend(
            plan.assets[asset_index]
                .surfaces
                .iter()
                .map(|&(surface, material)| XModelSurfaceDraw {
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
                    scene_entnum: Some(owner.entnum),
                    caster_bound,
                }),
        );
        perf::item(i32::from(et::ITEM), None, None, Some("posed"));
    }
    if !draws.is_empty() {
        let topology =
            topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.vertices.len());
        let revision = plan.revision;
        plan.revision = stamp_plan_geometry(&mut plan.revisions, revision, topology);
    }
    plan.publish_frame_rows(&mut draws, &mut owners);
}
