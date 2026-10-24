use super::*;

/// Bind-poses a composition's models into drawable surfaces.
pub(super) fn pose_composition(
    composition: &ItemComposition,
    catalog: &asset_model::WorldWeaponCatalog,
    skin_entries: &[dpvs_t6::SceneEntSkinEntry],
) -> Option<ItemPosedAsset> {
    let wanted = 1 + composition.attachments.len();
    let mut skels = Vec::with_capacity(wanted);
    let mut models = Vec::with_capacity(wanted);
    for index in std::iter::once(&composition.model).chain(&composition.attachments) {
        let Some(entry) = catalog.get_at(index.order()) else {
            break;
        };
        skels.push(&*entry.skel);
        models.push(*index);
    }
    if skels.len() != wanted {
        return None;
    }
    let state = xmodel_runtime::DObjSemanticState::bind_pose(composition.name.clone(), 1, 1);
    let request = state.resolve_request(|_| None).ok()?;
    let (surfaces, _) = pose_script_dobj_with_materials(
        None,
        &skels,
        &composition.dobj,
        &request,
        None,
        skin_entries,
    )?;
    Some(ItemPosedAsset {
        key: composition.key.clone(),
        models,
        surfaces,
    })
}

pub(super) fn pose_items(
    occupancy: Res<ItemOccupancy>,
    world_weapons: Option<Res<assets::PreparedWorldWeapons>>,
    gfx: Res<HostGfxScene>,
    mut product: ResMut<ItemPoseProduct>,
) {
    let _slow = SlowGuard::new("pose_items");
    product.owners.clear();
    if world_weapons.as_ref().is_some_and(|w| w.is_changed()) {
        product.assets.clear();
    }
    let Some(catalog) = world_weapons
        .as_ref()
        .map(|prepared| &prepared.0)
        .filter(|catalog| !catalog.is_empty())
    else {
        product.assets.clear();
        return;
    };
    for row in &occupancy.rows {
        let entnum = u32::try_from(row.entnum).unwrap_or(0);
        let slot = scene_slot(&gfx.scene, entnum);
        if slot.hidden {
            continue;
        }
        let known = product
            .assets
            .iter()
            .position(|posed| posed.key == row.composition.key);
        let asset_index = match known {
            Some(index) => index,
            None => {
                let Some(posed) = pose_composition(&row.composition, catalog, slot.skin_entries)
                else {
                    continue;
                };
                product.assets.push(posed);
                product.assets.len() - 1
            }
        };
        product.owners.push(ItemPosedOwner {
            asset_index,
            slot: row.slot,
            entnum,
            origin: row.origin,
            angles: row.angles,
            name: row.composition.name.clone(),
            model: row.composition.model,
            lighting_origin: row.lighting_origin,
        });
    }
}

/// Empties the draw plan but keeps its revision counters, advancing them
/// first when `advance` is set.
pub(super) fn restart_plan(plan: &mut ItemDrawPlan, advance: bool) {
    let (revision, generation) = if advance {
        (
            plan.revision.wrapping_add(1),
            plan.generation.wrapping_add(1),
        )
    } else {
        (plan.revision, plan.generation)
    };
    *plan = ItemDrawPlan::default();
    plan.revision = revision;
    plan.generation = generation;
}

/// Adds a posed composition's surfaces to the plan, returning where they sit.
pub(super) fn plan_asset(
    plan: &mut ItemDrawPlan,
    posed: &ItemPosedAsset,
    catalog: &asset_model::WorldWeaponCatalog,
    tess: &TessMaterials,
    model_materials: &PreparedModelMaterials,
) -> Option<usize> {
    if let Some(found) = plan.assets.iter().position(|a| a.model == posed.key) {
        return Some(found);
    }
    let materials: Vec<Option<SmodelPassMaterial>> = posed
        .surfaces
        .iter()
        .map(|surface| {
            let entry = catalog.get_at(posed.models.get(usize::from(surface.model))?.order())?;
            let name = entry.material_present_name(surface.surface_index)?;
            model_materials.material(&tess.catalog, name).cloned()
        })
        .collect();
    if materials.iter().all(Option::is_none) {
        return None;
    }
    let surfaces = append_item_surfaces(plan, &posed.surfaces, &materials);
    if surfaces.is_empty() {
        return None;
    }
    plan.assets.push(ItemAssetDraw {
        model: posed.key.clone(),
        surfaces,
    });
    Some(plan.assets.len() - 1)
}
