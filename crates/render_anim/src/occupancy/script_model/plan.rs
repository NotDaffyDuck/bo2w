//! Turning the frame's posed script models into the draw plan: the shared
//! mesh data per posed asset, and one draw of its surfaces per owner.

use bevy::prelude::*;
use render_frame::RenderFocusFrame;
use render_scene::{
    ModelLightingOwner, ModelLightingRequest, ModelLightingRequests, SmodelPassMaterial,
    TessMaterials, WorldModelLightingAtlas, WorldPresentFacts, XModelColourRefusal,
    XModelSurfaceDraw,
};
use xmodel_runtime::DObjSemanticState;

use super::focus::RenderFocus;
use super::product::{Compaction, ScriptModelPoseProduct, recyclable_slot, topology_matches};
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::xmodel_pose::PosedModelSurface;
use crate::occupancy::t6_body::SlowGuard;
use crate::{
    ScriptModelDrawPlan, ScriptModelOwnerDraw, append_script_model_asset,
    overwrite_script_model_asset, retain_script_model_assets,
};

/// Owner draws are numbered from here, below it being the ids of other kinds
/// of model.
const OBJECT_ID_BASE: u16 = 0x200;

fn object_id_for(drawn_so_far: usize) -> u16 {
    u16::try_from(drawn_so_far)
        .unwrap_or(u16::MAX)
        .saturating_add(OBJECT_ID_BASE)
}

/// What the plan system keeps between frames, and its working space.
#[derive(Default)]
pub(super) struct PlanScratch {
    last_material_generation: Option<render_material::MaterialGenerationId>,
    last_unbound: usize,
    live_plan: Vec<usize>,
    product_to_plan: Vec<usize>,
    compaction: Compaction,
    draws: Vec<XModelSurfaceDraw>,
    owners: Vec<ScriptModelOwnerDraw>,
}

/// Puts a posed asset in the plan, over an unused asset of the same shape if
/// there is one (so its buffers are reused), else after the rest.
fn place_in_plan(
    plan: &mut ScriptModelDrawPlan,
    live: &[usize],
    key: &asset_world::MapXModelAssetKey,
    dobj_state: &DObjSemanticState,
    camera_lods: &[Option<u8>],
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> usize {
    let slot = recyclable_slot(&plan.assets, live, |old| {
        &old.key == key
            && old.camera_lods == camera_lods
            && topology_matches(&old.dobj_state, dobj_state)
    });
    if let Some(index) = slot
        && overwrite_script_model_asset(plan, index, dobj_state.clone(), surfaces, materials)
    {
        return index;
    }
    append_script_model_asset(
        plan,
        key.clone(),
        dobj_state.clone(),
        camera_lods.to_vec(),
        surfaces,
        materials,
    )
}

pub(super) fn commit_script_model_draw_plan(
    assets: Option<Res<asset_world::MapXModelSceneCatalog>>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    tess: Option<Res<TessMaterials>>,
    facts: Res<WorldPresentFacts>,
    mut plan: ResMut<ScriptModelDrawPlan>,
    product: Res<ScriptModelPoseProduct>,
    mut focus: ResMut<RenderFocus>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
    model_materials: Res<PreparedModelMaterials>,
    mut scratch: Local<PlanScratch>,
) {
    let _slow = SlowGuard::new("commit_script_model_draw_plan");
    let (Some(assets), Some(atlas), Some(tess)) = (assets, atlas, tess) else {
        plan.publish_no_rows();
        return;
    };
    if product.producer_unavailable || !facts.spawned {
        plan.publish_no_rows();
        return;
    }
    let PlanScratch {
        last_material_generation,
        last_unbound,
        live_plan,
        product_to_plan,
        compaction,
        draws,
        owners,
    } = &mut *scratch;

    // New catalogs mean every mesh built from the old ones is stale; the
    // counters the plan publishes with keep counting.
    let generation = tess.catalog.generation_id;
    if assets.is_changed()
        || atlas.is_changed()
        || tess.is_changed()
        || *last_material_generation != Some(generation)
    {
        let (revision, plan_generation, revisions) = (plan.revision, plan.generation, plan.revisions);
        *plan = ScriptModelDrawPlan::default();
        plan.revision = revision;
        plan.generation = plan_generation;
        plan.revisions = revisions;
        *last_material_generation = Some(generation);
    }

    draws.clear();
    owners.clear();
    live_plan.clear();
    product_to_plan.clear();
    for posed in &product.assets {
        let index = plan
            .asset_index(&posed.key, &posed.dobj_state, &posed.camera_lods)
            .unwrap_or_else(|| {
                let materials: Vec<_> = posed
                    .authored
                    .iter()
                    .map(|&authored| {
                        model_materials.authored(&tess.catalog, authored?).cloned()
                    })
                    .collect();
                place_in_plan(
                    &mut plan,
                    live_plan,
                    &posed.key,
                    &posed.dobj_state,
                    &posed.camera_lods,
                    &posed.surfaces,
                    &materials,
                )
            });
        product_to_plan.push(index);
        live_plan.push(index);
    }

    if product.surface_material_unbound != *last_unbound {
        *last_unbound = product.surface_material_unbound;
        diag::warn!(
            World,
            "script model surfaces without a bound material: {} (first {}); MapXModelSceneCatalog resolved={}",
            product.surface_material_unbound,
            product.surface_material_first.as_deref().unwrap_or("-"),
            u8::from(assets.materials_resolved()),
        );
    }

    compaction.plan(plan.assets.len(), live_plan);
    retain_script_model_assets(&mut plan, compaction.keep());

    for row in &product.rows {
        let Some(asset_index) = product_to_plan
            .get(row.asset_index)
            .and_then(|&planned| compaction.new_index(planned))
        else {
            if let Some(id) = row.focused_owner_id {
                focus.refuse(id, &row.model.0, "posed_asset_unavailable");
            }
            continue;
        };
        let surface_count = plan.assets[asset_index].surfaces.len();
        let pending_lighting = (!row.camera_hidden).then(|| {
            lighting_requests.request(ModelLightingRequest {
                owner: ModelLightingOwner::ScriptModel(row.entity),
                origin: row.lighting_origin,
                lookup_fallback: row.lookup_fallback,
            })
        });
        let object_id = object_id_for(owners.len());
        if let Some(id) = row.focused_owner_id {
            focus.frame = Some(RenderFocusFrame {
                owner_id: id,
                model: Some(row.model.0.clone()),
                outcome: if surface_count == 0 { "planned_empty" } else { "planned" },
                object_id: Some(object_id),
                camera_hidden: Some(row.camera_hidden),
                lighting_handle: None,
                planned_surfaces: u32::try_from(surface_count).unwrap_or(u32::MAX),
            });
        }
        owners.push(ScriptModelOwnerDraw {
            entity: row.entity,
            current_model: row.model.clone(),
            object_id,
        });
        draws.extend(plan.assets[asset_index].surfaces.iter().map(|&(surface, material)| {
            XModelSurfaceDraw {
                surface,
                material,
                world_from_local: row.world_from_local,
                lighting_handle: 0,
                pending_lighting,
                colour_refusal: row.camera_hidden.then_some(XModelColourRefusal::CameraFrustum),
                object_id,
                scene_light_index: 0,
                reflection_probe_index: 0,
                packed_lighting: None,
                is_scope: false,
                scene_entnum: row.entnum,
                caster_bound: Some(row.caster_bound),
            }
        }));
    }
    plan.publish_frame_rows(draws, owners);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_draws_are_numbered_from_the_base() {
        assert_eq!(object_id_for(0), 0x200);
        assert_eq!(object_id_for(5), 0x205);
    }

    #[test]
    fn numbering_stops_at_the_last_id() {
        assert_eq!(object_id_for(70_000), u16::MAX);
        assert_eq!(object_id_for(usize::from(u16::MAX)), u16::MAX);
    }
}
