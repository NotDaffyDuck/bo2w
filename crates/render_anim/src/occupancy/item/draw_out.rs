use super::*;

pub(super) fn append_item_draws(
    product: Res<ItemPosed>,
    level_placed_weapons: Option<Res<assets::ReadiedWorldWeapons>>,
    atlas: Option<Res<LevelModelIllumAtlas>>,
    at_spot: Res<render_scene::LooseAtSpotLookup>,
    tess: Option<Res<StitchMaterials>>,
    facts: Res<LevelPresentFacts>,
    mut plan_in: ResMut<PickupPaintPlan>,
    mut illum_requests: ResMut<ModelIllumRequests>,
    model_materials: Res<PreparedModelMaterials>,
    mut seen_generation: Local<Option<render_material::MaterialEpochId>>,
    mut draws_in: Local<Vec<FigureFacetDraw>>,
    mut owners: Local<Vec<ModelOwnerPaint>>,
) {
    let _slow = SlowGuard::new("append_item_draws");
    draws_in.clear();
    owners.clear();
    let catalog = level_placed_weapons
        .as_ref()
        .map(|prepared_in| &prepared_in.0)
        .filter(|cat| !cat.is_empty());
    let (Some(listing), Some(_), Some(tess_ref), true) =
        (catalog, atlas.as_deref(), tess.as_deref(), facts.spawned)
    else {
        restart_plan(&mut plan_in, false);
        plan_in.publish_no_rows();
        *seen_generation = None;
        return;
    };

    let material_generation = tess_ref.catalog.revision_stamp;
    let changed = level_placed_weapons.as_ref().is_some_and(|w| w.is_changed())
        || atlas.as_ref().is_some_and(|a| a.is_changed())
        || tess.as_ref().is_some_and(|t| t.is_changed())
        || *seen_generation != Some(material_generation);
    if changed {
        restart_plan(&mut plan_in, true);
        plan_in.revisions.advance_acceptance_stamp();
        *seen_generation = Some(material_generation);
    }

    for owner in &product.owners {
        let posed = &product.assets[owner.draw_asset_slot];
        let Some(draw_asset_slot) = plan_asset(&mut plan_in, posed, listing, tess_ref, &model_materials)
        else {
            continue;
        };
        let Some(entry) = listing.at_row(owner.mdl.order()) else {
            continue;
        };
        let radius = entry.bone_rig.radius;
        let queued_illum = Some(illum_requests.queue(ModelIllumRequest {
            owner: ModelIllumOwner::Item(owner.ent_num),
            origin: owner.illum_origin,
            lookup_backup:
                at_spot.fallback(owner.illum_origin, radius.map(illum_query_half_size)),
        }));
        let object_tag = PAINT_ID_FIRST_PICKUP.saturating_add(owner.ix as u16);
        owners.push(ModelOwnerPaint {
            object_tag,
            xmodel: owner.name.clone(),
        });
        let scene_from_local = item_world_from_local(owner.spot, owner.rot);
        let caster_bound = radius.map(|radius| render_scene::XModelCasterBound {
            origin: owner.spot,
            reach: radius.max(1.0),
        });
        draws_in.extend(
            plan_in.assets[draw_asset_slot]
                .surf_list
                .iter()
                .map(|&(surface, material)| FigureFacetDraw {
                    surface,
                    material,
                    map_from_self: scene_from_local,
                    illum_handle: 0,
                    queued_illum,
                    colour_refusal: None,
                    object_tag,
                    stage_light_index: 0,
                    env_probe_index: 0,
                    compact_illum: None,
                    is_scoped_view: false,
                    stage_actor_num: Some(owner.ent_num),
                    caster_bound,
                }),
        );
        perf::item(i32::from(et::ITEM), None, None, Some("posed"));
    }
    if !draws_in.is_empty() {
        let topology_in =
            ix_layout_digest(&plan_in.indices, &plan_in.facet_ranges, plan_in.verts.len());
        let revision = plan_in.plan_rev;
        plan_in.plan_rev = bump_plan_mesh_rev(&mut plan_in.revisions, revision, topology_in);
    }
    plan_in.publish_frame_rows(&mut draws_in, &mut owners);
}
