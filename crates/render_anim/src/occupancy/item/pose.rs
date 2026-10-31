use super::*;

/// Bind-poses a composition's models into drawable surfaces.
pub(super) fn pose_composition(
    composition: &ItemComposition,
    catalog: &asset_model::LevelWeaponRoster,
    skins: &[dpvs_t6::EntitySkinSlotRecord],
) -> Option<ItemPosedAsset> {
    let wanted = 1 + composition.attachments.len();
    let mut skels = Vec::with_capacity(wanted);
    let mut models = Vec::with_capacity(wanted);
    for index in std::iter::once(&composition.mdl).chain(&composition.attachments) {
        let Some(entry) = catalog.at_row(index.order()) else {
            break;
        };
        skels.push(&*entry.bone_rig);
        models.push(*index);
    }
    if skels.len() != wanted {
        return None;
    }
    let state = xmodel_runtime::RigPoseRecord::neutral_pose(composition.name.clone(), 1, 1);
    let request = state.look_up_pose_query(|_| None).ok()?;
    let (surfaces, _) =
        pose_script_skeleton_and_materials(None, &skels, &composition.dobj, &request, None, skins)?;
    let key = composition.key.clone();
    Some(ItemPosedAsset {
        surfaces,
        models,
        key,
    })
}

pub(super) fn pose_items(
    item_occupancy: Res<ItemRows>,
    prepared_world: Option<Res<assets::ReadiedWorldWeapons>>,
    host_gfx: Res<HostVisualStage>,
    mut out: ResMut<ItemPosed>,
) {
    let _slow = SlowGuard::new("pose_items");
    out.owners.clear();
    if prepared_world.as_ref().is_some_and(|fresh| fresh.is_changed()) {
        out.assets.clear();
    }
    let loaded = prepared_world
        .as_ref()
        .map(|prepared_in| &prepared_in.0)
        .filter(|loaded| !loaded.is_empty());
    let Some(catalog) = loaded else {
        out.assets.clear();
        return;
    };
    for row in item_occupancy.rows.iter() {
        let entnum = u32::try_from(row.entnum).unwrap_or_default();
        let slot = scene_slot(&host_gfx.scene, entnum);
        let visible_in = !slot.concealed;
        if !visible_in {
            continue;
        }
        let known = out
            .assets
            .iter()
            .position(|posed| posed.key == row.composition.key);
        let draw_asset_slot = if let Some(index) = known {
            index
        } else {
            let Some(posed) = pose_composition(&row.composition, catalog, slot.skin_jobs)
            else {
                continue;
            };
            out.assets.push(posed);
            out.assets.len() - 1
        };
        let origin = row.pos;
        let angles = row.angs;
        let illum_origin = row.illum_origin;
        let (name, model) = (row.composition.name.clone(), row.composition.mdl);
        out.owners.push(ItemPosedOwner {
            spot: origin,
            rot: angles,
            illum_origin,
            name,
            mdl: model,
            ent_num: entnum,
            draw_asset_slot,
            ix: row.seat,
        });
    }
}

/// Empties the draw plan but keeps its revision counters, advancing them
/// first when `advance` is set.
pub(super) fn restart_plan(draw_plan: &mut PickupPaintPlan, advance: bool) {
    let step = u64::from(advance);
    *draw_plan = PickupPaintPlan {
        plan_rev: draw_plan.plan_rev.wrapping_add(step),
        generation: draw_plan.generation.wrapping_add(step),
        ..Default::default()
    };
}

/// Adds a posed composition's surfaces to the plan, returning where they sit.
pub(super) fn plan_asset(
    draw_plan: &mut PickupPaintPlan,
    posed_in: &ItemPosedAsset,
    catalog: &asset_model::LevelWeaponRoster,
    tess: &StitchMaterials,
    model_materials: &PreparedModelMaterials,
) -> Option<usize> {
    let existing_in = draw_plan.assets.iter().position(|a| a.xmodel_name == posed_in.key);
    if existing_in.is_some() {
        return existing_in;
    }
    let mats = posed_in
        .surfaces
        .iter()
        .map(|piece| {
            let entry = catalog.at_row(posed_in.models.get(usize::from(piece.mdl))?.order())?;
            let name = entry.material_name_of(piece.surface_slot_number)?;
            model_materials.mat(&tess.catalog, name).cloned()
        })
        .collect::<Vec<Option<PropPassMaterial>>>();
    if mats.iter().flatten().next().is_none() {
        return None;
    }
    let surfaces = push_pickup_surfaces(draw_plan, &posed_in.surfaces, &mats);
    let landing = draw_plan.assets.len();
    (!surfaces.is_empty()).then(|| {
        let model = posed_in.key.clone();
        draw_plan.assets.push(PickupMeshPaint { surf_list: surfaces, xmodel_name: model });
        landing
    })
}
