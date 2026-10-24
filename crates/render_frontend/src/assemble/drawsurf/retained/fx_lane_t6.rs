use bevy::prelude::*;
use dpvs_t6::{pack_code_mesh_draw_surf, pack_mark_mesh_draw_surf, pack_particle_cloud_draw_surf};
use render_material::sort_band;

use super::{RetainedDrawItem, RetainedDrawKind, catalog_item, draw_order, route_to_products};
use crate::assemble::drawsurf::list::{CONTENT_ID_SEED, mix_content_id};
use crate::assemble::drawsurf::tess::fx::{FxCodeMeshPlan, FxSurfaceDraw};
use crate::assemble::drawsurf::tess::mark::{
    GfxMarkMeshPlan, GfxMarkSubKey, mark_mesh_surface_samplers,
};
use crate::assemble::drawsurf::tess::particle_cloud::FxParticleCloudPlan;
use crate::assemble::drawsurf::{MaterialGeneration, RuntimeMaterialCatalog, SurfaceSamplerInputs};
use crate::prepare::scene::model_lighting_cache::{
    ModelLightingOwner, ResolvedModelLighting, ResolvedModelLightingTable,
};
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::world::WorldScriptModelInstance;

/// The effect meshes of the frame (code meshes, particle clouds and decal marks) as items
/// sorted for drawing.
///
/// The items are rebuilt when the plans' layout changes. While it holds, each frame copies the
/// new arguments into the items already there. Glass marks take their lighting from a table
/// that moves every frame, so a lane that holds one is rebuilt every frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct FxDrawLane {
    pub colour: Vec<RetainedDrawItem>,
    pub emissive: Vec<RetainedDrawItem>,
    pub distortion: Vec<RetainedDrawItem>,

    pub membership_revision: u64,
    layout: u64,

    pub payload_revision: u64,
    pub code_sorted: u32,
    pub particle_cloud_sorted: u32,
    pub mark_mesh_sorted: u32,
    pub code_skipped_no_ordinal: u32,
    pub particle_cloud_skipped_no_ordinal: u32,
    pub mark_mesh_skipped_no_ordinal: u32,
    pub mark_mesh_skipped_no_lighting: u32,
}

impl FxDrawLane {
    fn products(&mut self) -> [&mut Vec<RetainedDrawItem>; 3] {
        [&mut self.colour, &mut self.emissive, &mut self.distortion]
    }

    fn reset(&mut self) {
        for product in self.products() {
            product.clear();
        }
        self.code_sorted = 0;
        self.particle_cloud_sorted = 0;
        self.mark_mesh_sorted = 0;
        self.code_skipped_no_ordinal = 0;
        self.particle_cloud_skipped_no_ordinal = 0;
        self.mark_mesh_skipped_no_ordinal = 0;
        self.mark_mesh_skipped_no_lighting = 0;
    }

    fn sort(&mut self) {
        for product in self.products() {
            product.sort_unstable_by_key(draw_order);
        }
    }
}

/// The three effect plans of a frame; any of them may be absent.
#[derive(Clone, Copy)]
struct Plans<'a> {
    code: Option<&'a FxCodeMeshPlan>,
    clouds: Option<&'a FxParticleCloudPlan>,
    marks: Option<&'a GfxMarkMeshPlan>,
}

impl Plans<'_> {
    /// Fingerprint of everything that decides which items exist and in what order. Draws
    /// with no indices make no item and are left out.
    fn layout(&self) -> u64 {
        let mut id = CONTENT_ID_SEED;
        let mut mix = |value: u64| mix_content_id(&mut id, value);
        if let Some(plan) = self.code {
            mix(plan.draws.len() as u64);
            for (i, draw) in plan.draws.iter().enumerate() {
                if draw.index_count == 0 {
                    continue;
                }
                let material = plan.materials.get(draw.material as usize);
                mix(i as u64);
                mix(u64::from(draw.material));
                mix(u64::from(draw.viewmodel));
                mix(u64::from(material.map_or(0, |m| m.sort_key)));
                mix(material
                    .and_then(|m| m.material_sorted_index)
                    .map_or(u64::MAX, u64::from));
            }
        }
        if let Some(plan) = self.clouds {
            mix(plan.draws.len() as u64);
            for (i, draw) in plan.draws.iter().enumerate() {
                if draw.index_count == 0 {
                    continue;
                }
                mix(i as u64);
                mix(u64::from(draw.material));
            }
        }
        if let Some(plan) = self.marks {
            mix(plan.draws.len() as u64);
            for (i, draw) in plan.draws.iter().enumerate() {
                if draw.index_count == 0 {
                    continue;
                }
                let key = &draw.sub_key;
                mix(i as u64);
                mix(u64::from(draw.material));
                mix(u64::from(key.lmap));
                mix(u64::from(key.entity.unwrap_or(u16::MAX)));
                mix(u64::from(key.smodel.unwrap_or(u16::MAX)));
                mix(u64::from(key.glass.unwrap_or(u16::MAX)));
                mix(u64::from(key.primary_light));
                mix(u64::from(key.probe));
            }
        }
        id
    }

    /// One number for the revisions of all three plans.
    fn revision(&self) -> u64 {
        let mut id = CONTENT_ID_SEED;
        mix_content_id(&mut id, self.code.map_or(0, |plan| plan.revision));
        mix_content_id(&mut id, self.clouds.map_or(0, |plan| plan.revision));
        mix_content_id(&mut id, self.marks.map_or(0, |plan| plan.revision));
        id
    }

    /// Copies the plans' current arguments into the items. Fails, leaving the caller to
    /// rebuild, when an item has no draw to read or is one whose lighting is not kept in it.
    fn refresh_in_place(&self, lane: &mut FxDrawLane) -> bool {
        for item in lane.products().into_iter().flatten() {
            match &mut item.kind {
                RetainedDrawKind::CodeMesh {
                    draw,
                    arg_count,
                    args,
                    ..
                } => {
                    let Some(plan) = self.code else {
                        return false;
                    };
                    let Some(source) = plan.draws.get(*draw as usize) else {
                        return false;
                    };
                    (*arg_count, *args) = code_args(plan, source);
                }
                RetainedDrawKind::ParticleCloud { draw, clouds, .. } => {
                    let Some(source) = self
                        .clouds
                        .and_then(|plan| plan.draws.get(*draw as usize))
                    else {
                        return false;
                    };
                    *clouds = source.clouds;
                }
                RetainedDrawKind::MarkMesh {
                    draw,
                    glass,
                    packed,
                    ..
                } => {
                    let Some(source) = self.marks.and_then(|plan| plan.draws.get(*draw as usize))
                    else {
                        return false;
                    };
                    *packed = source.sub_key.packed;
                    *glass = source.sub_key.glass.is_some();
                    if *glass {
                        return false;
                    }
                }
                RetainedDrawKind::Glass { .. } => return false,
                _ => {}
            }
        }
        true
    }
}

/// The argument rows of a code mesh draw: at most two, zero where the plan has none.
fn code_args(plan: &FxCodeMeshPlan, draw: &FxSurfaceDraw) -> (u8, [[f32; 4]; 2]) {
    let count = (draw.arg_count as usize).min(2);
    let mut args = [[0.0; 4]; 2];
    for (offset, slot) in args.iter_mut().enumerate().take(count) {
        if let Some(row) = plan.args.get(draw.arg_start as usize + offset) {
            *slot = *row;
        }
    }
    (count as u8, args)
}

/// Where a decal mark gets its lighting: the model it sits on, the glass piece, or the static
/// model it was cast onto.
struct MarkLighting<'a> {
    owner_of: &'a dyn Fn(u16) -> Option<Entity>,
    table: &'a ResolvedModelLightingTable,
    smodels: Option<&'a WorldSmodelLighting>,
}

impl MarkLighting<'_> {
    /// The lighting handle for a mark, or `None` when what it sits on has none yet. Marks on
    /// entities and glass also take their light and probe from the owner's seat.
    fn handle(&self, key: &mut GfxMarkSubKey) -> Option<u32> {
        if let Some(number) = key.entity {
            let owner = (self.owner_of)(number)?;
            return self.seat(ModelLightingOwner::ScriptModel(owner), key);
        }
        if let Some(piece) = key.glass {
            return self.seat(ModelLightingOwner::Glass(piece), key);
        }
        if let Some(index) = key.smodel {
            let handles = &self.smodels?.handles;
            return handles.get(usize::from(index)).copied().map(u32::from);
        }
        Some(0)
    }

    fn seat(&self, owner: ModelLightingOwner, key: &mut GfxMarkSubKey) -> Option<u32> {
        let ResolvedModelLighting::Seated {
            handle,
            scene_light_index,
            reflection_probe_index,
            ..
        } = self.table.get(owner)?
        else {
            return None;
        };
        key.primary_light = scene_light_index;
        key.probe = reflection_probe_index;
        Some(handle)
    }
}

fn add_code_meshes(plan: &FxCodeMeshPlan, catalog: &RuntimeMaterialCatalog, lane: &mut FxDrawLane) {
    for (i, draw) in plan.draws.iter().enumerate() {
        if draw.index_count == 0 {
            continue;
        }
        let Some(material) = plan.materials.get(draw.material as usize) else {
            continue;
        };
        let Some(rank) = material.material_sorted_index else {
            lane.code_skipped_no_ordinal = lane.code_skipped_no_ordinal.saturating_add(1);
            continue;
        };
        let key = pack_code_mesh_draw_surf(material.sort_key, sort_band(rank), i as u16).packed;
        let (arg_count, args) = code_args(plan, draw);
        let item = catalog_item(
            key,
            rank,
            RetainedDrawKind::CodeMesh {
                viewmodel: draw.viewmodel,
                draw: i as u32,
                material: draw.material,
                arg_count,
                args,
            },
            SurfaceSamplerInputs::default(),
            catalog,
        );
        route_to_products(
            &mut lane.colour,
            &mut lane.emissive,
            Some(&mut lane.distortion),
            item,
        );
        lane.code_sorted = lane.code_sorted.saturating_add(1);
    }
}

fn add_particle_clouds(
    plan: &FxParticleCloudPlan,
    catalog: &RuntimeMaterialCatalog,
    lane: &mut FxDrawLane,
) {
    for (i, draw) in plan.draws.iter().enumerate() {
        if draw.index_count == 0 {
            continue;
        }
        let Some(material) = plan.materials.get(draw.material as usize) else {
            continue;
        };
        let Some(rank) = material.material_sorted_index else {
            lane.particle_cloud_skipped_no_ordinal =
                lane.particle_cloud_skipped_no_ordinal.saturating_add(1);
            continue;
        };
        let key =
            pack_particle_cloud_draw_surf(material.sort_key, sort_band(rank), i as u16).packed;
        let item = catalog_item(
            key,
            rank,
            RetainedDrawKind::ParticleCloud {
                draw: i as u32,
                material: draw.material,
                clouds: draw.clouds,
            },
            SurfaceSamplerInputs::default(),
            catalog,
        );
        route_to_products(&mut lane.colour, &mut lane.emissive, None, item);
        lane.particle_cloud_sorted = lane.particle_cloud_sorted.saturating_add(1);
    }
}

fn add_marks(
    plan: &GfxMarkMeshPlan,
    lighting: &MarkLighting,
    catalog: &RuntimeMaterialCatalog,
    lane: &mut FxDrawLane,
) {
    for (i, draw) in plan.draws.iter().enumerate() {
        if draw.index_count == 0 {
            continue;
        }
        let Some(material) = plan.materials.get(draw.material as usize) else {
            continue;
        };
        let Some(rank) = material.material_sorted_index else {
            lane.mark_mesh_skipped_no_ordinal = lane.mark_mesh_skipped_no_ordinal.saturating_add(1);
            continue;
        };
        let mut sub_key = draw.sub_key;
        let Some(lighting_handle) = lighting.handle(&mut sub_key) else {
            lane.mark_mesh_skipped_no_lighting =
                lane.mark_mesh_skipped_no_lighting.saturating_add(1);
            continue;
        };
        let key = pack_mark_mesh_draw_surf(
            material.sort_key,
            sort_band(rank),
            i as u16,
            sub_key.lmap,
            sub_key.primary_light,
            sub_key.probe,
        )
        .packed;
        let item = catalog_item(
            key,
            rank,
            RetainedDrawKind::MarkMesh {
                glass: sub_key.glass.is_some(),
                draw: i as u32,
                material: draw.material,
                packed: sub_key.packed,
                lighting_handle,
            },
            mark_mesh_surface_samplers(sub_key),
            catalog,
        );
        route_to_products(&mut lane.colour, &mut lane.emissive, None, item);
        lane.mark_mesh_sorted = lane.mark_mesh_sorted.saturating_add(1);
    }
}

pub(crate) fn rebuild_fx_draw_lane(
    plans: (
        Option<Res<FxCodeMeshPlan>>,
        Option<Res<FxParticleCloudPlan>>,
        Option<Res<GfxMarkMeshPlan>>,
    ),
    generation: Res<MaterialGeneration>,
    mut lane: ResMut<FxDrawLane>,
    mark_owners: Query<(Entity, &WorldScriptModelInstance)>,
    lighting: Res<ResolvedModelLightingTable>,
    smodel_lighting: Option<Res<WorldSmodelLighting>>,
) {
    let plans = Plans {
        code: plans.0.as_deref(),
        clouds: plans.1.as_deref(),
        marks: plans.2.as_deref(),
    };
    let lane = &mut *lane;
    let layout = plans.layout();
    let revision = plans.revision();
    if layout == lane.layout && plans.refresh_in_place(lane) {
        lane.sort();
        lane.payload_revision = revision;
        perf::Counter::FxLayoutOverlay.emit(1.0);
        return;
    }
    perf::Counter::FxLayoutOverlay.emit(0.0);

    lane.reset();
    let owner_of = |number: u16| {
        mark_owners
            .iter()
            .find(|(_, owner)| owner.gentity_number == Some(number))
            .map(|(entity, _)| entity)
    };
    let mark_lighting = MarkLighting {
        owner_of: &owner_of,
        table: &lighting,
        smodels: smodel_lighting.as_deref(),
    };
    let catalog = &generation.catalog;
    if let Some(plan) = plans.code {
        add_code_meshes(plan, catalog, lane);
    }
    if let Some(plan) = plans.clouds {
        add_particle_clouds(plan, catalog, lane);
    }
    if let Some(plan) = plans.marks {
        add_marks(plan, &mark_lighting, catalog, lane);
    }
    lane.sort();
    lane.layout = layout;
    lane.membership_revision = lane.membership_revision.wrapping_add(1);
    lane.payload_revision = revision;
}
