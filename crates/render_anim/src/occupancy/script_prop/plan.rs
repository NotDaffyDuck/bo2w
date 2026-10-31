//! Turning the frame's posed script models into the draw plan: the shared
//! mesh data per posed asset, and one draw of its surfaces per owner.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use render_frame::FocusedFrameSnapshot;
use render_scene::{ModelIllumOwner, ModelIllumRequest, ModelIllumRequests};
use render_scene::{PropPassMaterial, StitchMaterials, LevelModelIllumAtlas};
use render_scene::{LevelPresentFacts, ModelTintRejection, FigureFacetDraw};

use super::focus::DrawFocus;
use super::product::{Compaction, ScriptPropPoseOutput, ScriptOwnerRow, ScriptPosedAsset};
use super::product::{recyclable_slot, topology_matches};
use crate::motion::model_materials::PreparedModelMaterials;
use crate::occupancy::t6_body::SlowGuard;
use crate::{ScriptedModelPaintPlan, ScriptPropOwnerRun, push_script_prop_mesh};
use crate::{swap_script_prop_mesh, keep_script_model_assets};

/// Owner draws are numbered from here, below it being the ids of other kinds
/// of model.
const OBJECT_ID_BASE: u16 = 0x200;

fn object_id_for(drawn_so_far: usize) -> u16 {
    let offset = u16::try_from(drawn_so_far).unwrap_or(u16::MAX);
    OBJECT_ID_BASE.saturating_add(offset)
}

/// What the plan system remembers from one frame to the next.
#[derive(Default)]
struct Memory {
    unbound_seen: usize,
    material_generation: Option<render_material::MaterialEpochId>,
}

/// Where the posed assets landed in the plan.
#[derive(Default)]
struct SlotTables {
    in_use: Vec<usize>,
    by_product: Vec<usize>,
    compaction: Compaction,
}

impl SlotTables {
    /// The plan index, after compaction, of the asset at `product_index` in the
    /// pose product.
    fn find(&self, product_index: usize) -> Option<usize> {
        let planned = *self.by_product.get(product_index)?;
        self.compaction.new_index(planned)
    }
}

/// The draws being collected for the frame.
#[derive(Default)]
struct FrameRows {
    owners: Vec<ScriptPropOwnerRun>,
    draws: Vec<FigureFacetDraw>,
}

/// What the plan system keeps between frames, and its working space.
#[derive(Default)]
pub(super) struct PlanScratch {
    memory: Memory,
    slots: SlotTables,
    frame: FrameRows,
}

/// What planning reads and does not change.
#[derive(SystemParam)]
pub(super) struct PlanSources<'w> {
    map: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    light_atlas: Option<Res<'w, LevelModelIllumAtlas>>,
    tess: Option<Res<'w, StitchMaterials>>,
    present: Res<'w, LevelPresentFacts>,
    posed_now: Res<'w, ScriptPropPoseOutput>,
    shaders: Res<'w, PreparedModelMaterials>,
}

/// Puts a posed asset in the plan, over an unused asset of the same shape if
/// there is one (so its buffers are reused), else after the rest.
fn place_in_plan(
    plan_in: &mut ScriptedModelPaintPlan,
    in_use: &[usize],
    posed: &ScriptPosedAsset,
    pass_materials: &[Option<PropPassMaterial>],
) -> usize {
    let ScriptPosedAsset {
        key: k,
        puppet_state,
        viewer_detail_levels,
        surfaces,
        ..
    } = posed;
    let reusable = recyclable_slot(&plan_in.assets, in_use, |old| {
        topology_matches(&old.puppet_state, puppet_state)
            && old.viewer_detail_levels == viewer_detail_levels.as_slice()
            && old.key == *k
    });
    let overwritten = reusable.filter(|&index| {
        swap_script_prop_mesh(plan_in, index, puppet_state.clone(), surfaces, pass_materials)
    });
    if let Some(index) = overwritten {
        return index;
    }
    push_script_prop_mesh(
        plan_in,
        k.clone(),
        puppet_state.clone(),
        viewer_detail_levels.clone(),
        surfaces,
        pass_materials,
    )
}

/// The materials a posed asset's surfaces draw with, where the session's
/// catalog has them.
fn pass_materials_of(
    posed: &ScriptPosedAsset,
    prepared: &PreparedModelMaterials,
    tess: &StitchMaterials,
) -> Vec<Option<PropPassMaterial>> {
    let mut found = Vec::with_capacity(posed.authored.len());
    for authored in &posed.authored {
        let material = authored.and_then(|index| prepared.authored(&tess.catalog, index));
        found.push(material.cloned());
    }
    found
}

/// Empties the plan but lets the counters it publishes with run on.
fn restart_plan(plan: &mut ScriptedModelPaintPlan) {
    let ScriptedModelPaintPlan {
        plan_rev,
        generation: epoch,
        revisions,
        ..
    } = *plan;
    *plan = ScriptedModelPaintPlan {
        plan_rev,
        generation: epoch,
        revisions,
        ..Default::default()
    };
}

/// Says so in the log when the number of surfaces with no bound material
/// changes.
fn note_unbound_surfaces(seen: &mut usize, posed_now: &ScriptPropPoseOutput, resolved: bool) {
    if posed_now.surface_material_unbound == *seen {
        return;
    }
    *seen = posed_now.surface_material_unbound;
    let first = posed_now.surface_material_first.as_deref().unwrap_or("-");
    diag::warn!(
        World,
        "script model surfaces with no material bound: {} (first {}); MapXModelSceneCatalog found={}",
        posed_now.surface_material_unbound,
        first,
        u8::from(resolved),
    );
}

/// Records what became of the focused owner's model this frame.
fn record_planned(
    focus: &mut DrawFocus,
    owner_key: u32,
    row: &ScriptOwnerRow,
    (object_tag, facet_tally): (u16, usize),
) {
    let outcome = match facet_tally {
        0 => "planned_empty",
        _ => "planned",
    };
    focus.frame = Some(FocusedFrameSnapshot {
        owner_no: owner_key,
        outcome,
        model: Some(row.mdl.0.clone()),
        object_tag: Some(object_tag),
        hidden_from_viewer: Some(row.hidden_from_viewer),
        illum_handle: None,
        planned_facets: u32::try_from(facet_tally).unwrap_or(u32::MAX),
    });
}

pub(super) fn commit_scripted_model_draw_plan(
    sources: PlanSources,
    mut plan_v: ResMut<ScriptedModelPaintPlan>,
    mut log: ResMut<DrawFocus>,
    mut light_requests: ResMut<ModelIllumRequests>,
    mut work: Local<PlanScratch>,
) {
    let _slow = SlowGuard::new("commit_script_model_draw_plan");
    let PlanSources {
        map,
        light_atlas,
        tess,
        present,
        posed_now,
        shaders: mats,
    } = sources;
    let usable = present.spawned && !posed_now.unavailable_producer;
    let (Some(map), Some(light_atlas), Some(tess), true) = (map, light_atlas, tess, usable) else {
        plan_v.publish_no_rows();
        return;
    };
    let PlanScratch {
        memory,
        slots,
        frame,
    } = &mut *work;

    // New catalogs mean every mesh built from the old ones is stale; the
    // counters the plan publishes with keep counting.
    let generation = tess.catalog.revision_stamp;
    let inputs_moved = [
        map.is_changed(),
        light_atlas.is_changed(),
        tess.is_changed(),
    ];
    if inputs_moved.contains(&true) || memory.material_generation != Some(generation) {
        restart_plan(&mut plan_v);
        memory.material_generation = Some(generation);
    }

    frame.draws.clear();
    frame.owners.clear();
    slots.in_use.clear();
    slots.by_product.clear();
    for posed in &posed_now.assets {
        let slot = plan_v
            .draw_asset_slot(&posed.key, &posed.puppet_state, &posed.viewer_detail_levels)
            .unwrap_or_else(|| {
                let pass_materials = pass_materials_of(posed, &mats, &tess);
                place_in_plan(&mut plan_v, &slots.in_use, posed, &pass_materials)
            });
        slots.by_product.push(slot);
        slots.in_use.push(slot);
    }

    note_unbound_surfaces(
        &mut memory.unbound_seen,
        &posed_now,
        map.surfaces_linked(),
    );

    slots.compaction.plan(plan_v.assets.len(), &slots.in_use);
    keep_script_model_assets(&mut plan_v, slots.compaction.keep());

    for line_row in &posed_now.rows {
        let Some(draw_asset_slot) = slots.find(line_row.draw_asset_slot) else {
            if let Some(id) = line_row.spectated_entity_id {
                let why = "posed_asset_unavailable";
                log.decline(id, &line_row.mdl.0, why);
            }
            continue;
        };
        let surfaces = &plan_v.assets[draw_asset_slot].surfs;
        let queued_illum = if line_row.hidden_from_viewer {
            None
        } else {
            Some(light_requests.queue(ModelIllumRequest {
                origin: line_row.illum_origin,
                owner: ModelIllumOwner::ScriptModel(line_row.ent),
                lookup_backup: line_row.lookup_backup,
            }))
        };
        let object_tag = object_id_for(frame.owners.len());
        if let Some(id) = line_row.spectated_entity_id {
            record_planned(&mut log, id, line_row, (object_tag, surfaces.len()));
        }
        frame.owners.push(ScriptPropOwnerRun {
            active_mesh: line_row.mdl.clone(),
            object_tag,
            ent: line_row.ent,
        });
        let colour_refusal = line_row
            .hidden_from_viewer
            .then_some(ModelTintRejection::CameraViewVolume);
        for &(surf, material) in surfaces {
            frame.draws.push(FigureFacetDraw {
                surface: surf,
                material,
                is_scoped_view: false,
                map_from_self: line_row.world_from_local,
                stage_actor_num: line_row.entnum,
                object_tag,
                compact_illum: None,
                colour_refusal,
                caster_bound: Some(line_row.caster_bound),
                illum_handle: 0,
                queued_illum,
                stage_light_index: 0,
                env_probe_index: 0,
            });
        }
    }
    plan_v.publish_frame_rows(&mut frame.draws, &mut frame.owners);
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn the_first_owner_draw_gets_the_base_id() {
        assert_eq!(object_id_for(0), OBJECT_ID_BASE);
        assert_eq!(object_id_for(5), OBJECT_ID_BASE + 5);
    }

    #[test]
    fn ids_run_out_at_the_top_of_the_range() {
        for count in [70_000, usize::from(u16::MAX)] {
            assert_eq!(object_id_for(count), u16::MAX);
        }
    }
}
