use bevy::prelude::*;
use dpvs_t6::{
    GfxDrawSurf, SF_XMODEL_RIGID_SKINNED, pack_xmodel_rigid_skinned_draw_surf,
    with_reflection_probe_index,
};

use super::{RetainedDrawItem, RetainedDrawKind, catalog_item, draw_order, route_to_products};
use crate::assemble::drawsurf::list::{CONTENT_ID_SEED, mix_content_id};
use crate::assemble::drawsurf::tess::sky::SkyModelDrawPlan;
use crate::assemble::drawsurf::tess::xmodel::{
    DynEntDrawPlan, FpvDrawPlan, FxModelDrawPlan, ItemDrawPlan, MissileDrawPlan,
    RemoteBodyDrawPlan, ScriptModelDrawPlan, XMODEL_OBJECT_ID_VIEWMODEL, XModelColourRefusal,
    XModelDrawPlan, XModelSurfaceDraw, merge_xmodel_draw_plan,
};
use crate::assemble::drawsurf::{
    MaterialGeneration, RuntimeMaterialCatalog, SurfaceReflectionProbeId, SurfaceSamplerInputs,
    with_scene_light_index,
};
use crate::prepare::scene::gfx_scene::HostGfxScene;
use crate::prepare::scene::view_parms::PreparedSceneView;

/// Which plan draw each item of a product was made from, in the product's order.
#[derive(Clone, Debug, Default)]
struct Origins {
    colour: Vec<u32>,
    emissive: Vec<u32>,
    distortion: Vec<u32>,
}

/// Every posed model draw of the frame (arms, bodies, script models, missiles, items, fx models,
/// dynents, scene entities, the sky model) as items sorted for drawing.
///
/// The items are rebuilt only when the plan's layout changes. While it holds, each frame writes
/// the new poses and lighting into the items already there.
#[derive(Resource, Clone, Debug, Default)]
pub struct XModelDrawLane {
    pub colour: Vec<RetainedDrawItem>,
    pub emissive: Vec<RetainedDrawItem>,
    pub distortion: Vec<RetainedDrawItem>,

    origins: Origins,

    pub membership_revision: u64,
    layout: u64,

    pub payload_revision: u64,
    pub merge_packed_n: Option<u32>,
    pub sorted: u32,
    pub object_id_standin: u32,
    pub fx_object_id_exhausted: u32,
    pub skipped_no_ordinal: u32,
    pub skipped_no_baked_key: u32,
    pub skipped_camera_frustum: u32,
    pub skipped_no_lighting: u32,
}

/// Why a plan draw makes no item.
enum Skip {
    CameraFrustum,
    NoMaterial,
    NoLighting,
    NoOrdinal,
    NoBakedKey,
}

impl XModelDrawLane {
    fn reset(&mut self) {
        self.colour.clear();
        self.emissive.clear();
        self.distortion.clear();
        self.origins = Origins::default();
        self.sorted = 0;
        self.object_id_standin = 0;
        self.skipped_no_ordinal = 0;
        self.skipped_no_baked_key = 0;
        self.skipped_camera_frustum = 0;
        self.skipped_no_lighting = 0;
    }

    fn count_skip(&mut self, skip: Skip) {
        let counter = match skip {
            Skip::CameraFrustum => &mut self.skipped_camera_frustum,
            Skip::NoLighting => &mut self.skipped_no_lighting,
            Skip::NoOrdinal => &mut self.skipped_no_ordinal,
            Skip::NoBakedKey => &mut self.skipped_no_baked_key,
            Skip::NoMaterial => return,
        };
        *counter = counter.saturating_add(1);
    }

    /// Files an item made from plan draw `origin`; every product that takes it notes the origin.
    fn file(&mut self, item: RetainedDrawItem, origin: u32) {
        route_to_products(
            &mut self.colour,
            &mut self.emissive,
            Some(&mut self.distortion),
            item,
        );
        self.origins.colour.resize(self.colour.len(), origin);
        self.origins.emissive.resize(self.emissive.len(), origin);
        self.origins.distortion.resize(self.distortion.len(), origin);
    }
}

/// Fingerprint of everything that decides which items exist and in what order. Poses and
/// lighting handles that do not change an item's existence are left out.
fn layout_of(plan: &XModelDrawPlan, catalog: &RuntimeMaterialCatalog) -> u64 {
    let mut id = CONTENT_ID_SEED;
    let mut mix = |value: u64| mix_content_id(&mut id, value);
    mix(catalog.generation_id.0);
    mix(plan.topology_revision);
    mix(plan.draws.len() as u64);
    for draw in &plan.draws {
        let material = plan.materials.get(draw.material as usize);
        mix(u64::from(draw.surface));
        mix(u64::from(draw.material));
        mix(u64::from(draw.object_id));
        mix(u64::from(draw.scene_light_index));
        mix(u64::from(draw.reflection_probe_index));
        mix(u64::from(draw.colour_refusal.is_some()));
        mix(u64::from(
            material.is_some_and(|m| m.model_lighting_required && draw.lighting_handle == 0),
        ));
        mix(material
            .and_then(|m| m.material_sorted_index)
            .map_or(u64::MAX, u64::from));
    }
    id
}

/// Makes the item for one plan draw, or says why there is none. The checks run in the order
/// the skip counters rely on.
fn draw_item(
    plan: &XModelDrawPlan,
    catalog: &RuntimeMaterialCatalog,
    draw: &XModelSurfaceDraw,
) -> Result<RetainedDrawItem, Skip> {
    if matches!(draw.colour_refusal, Some(XModelColourRefusal::CameraFrustum)) {
        return Err(Skip::CameraFrustum);
    }
    let material = plan
        .materials
        .get(draw.material as usize)
        .ok_or(Skip::NoMaterial)?;
    if material.model_lighting_required && draw.lighting_handle == 0 {
        return Err(Skip::NoLighting);
    }
    let rank = material.material_sorted_index.ok_or(Skip::NoOrdinal)?;
    let baked = catalog
        .material_for_sorted_ordinal(rank)
        .and_then(|row| row.baked_draw_surf)
        .ok_or(Skip::NoBakedKey)?;
    let surf = pack_xmodel_rigid_skinned_draw_surf(GfxDrawSurf::from_packed(baked), draw.object_id);
    let surf = with_reflection_probe_index(surf, draw.reflection_probe_index);
    let key = with_scene_light_index(surf.packed, draw.scene_light_index);
    let keyed = GfxDrawSurf::from_packed(key);
    debug_assert_eq!(keyed.surf_type(), SF_XMODEL_RIGID_SKINNED);
    Ok(catalog_item(
        key,
        rank,
        RetainedDrawKind::XModel {
            surface: draw.surface,
            material: draw.material,
            object_id: draw.object_id,
            world_from_local: draw.world_from_local,
            lighting_handle: draw.lighting_handle,
            packed_lighting: draw.packed_lighting,
            is_scope: draw.is_scope,
            scene_entnum: draw.scene_entnum,
        },
        SurfaceSamplerInputs {
            reflection_probe: Some(SurfaceReflectionProbeId(keyed.reflection_probe_index())),
            ..Default::default()
        },
        catalog,
    ))
}

/// Sorts `items` into draw order and puts `origins` in the same order.
fn sort_with_origins(items: &mut [RetainedDrawItem], origins: &mut [u32]) {
    debug_assert_eq!(items.len(), origins.len());
    let mut order: Vec<u32> = (0..u32::try_from(items.len()).unwrap_or(u32::MAX)).collect();
    order.sort_unstable_by_key(|&at| draw_order(&items[at as usize]));
    let sorted_items: Vec<RetainedDrawItem> = order.iter().map(|&at| items[at as usize]).collect();
    let sorted_origins: Vec<u32> = order.iter().map(|&at| origins[at as usize]).collect();
    items.copy_from_slice(&sorted_items);
    origins.copy_from_slice(&sorted_origins);
}

fn rebuild(plan: &XModelDrawPlan, catalog: &RuntimeMaterialCatalog, lane: &mut XModelDrawLane) {
    lane.reset();
    for (index, draw) in plan.draws.iter().enumerate() {
        let origin = u32::try_from(index).unwrap_or(u32::MAX);
        match draw_item(plan, catalog, draw) {
            Ok(item) => {
                lane.file(item, origin);
                lane.sorted = lane.sorted.saturating_add(1);
                if draw.object_id >= XMODEL_OBJECT_ID_VIEWMODEL {
                    lane.object_id_standin = lane.object_id_standin.saturating_add(1);
                }
            }
            Err(skip) => lane.count_skip(skip),
        }
    }
    let Origins {
        colour,
        emissive,
        distortion,
    } = &mut lane.origins;
    sort_with_origins(&mut lane.colour, colour);
    sort_with_origins(&mut lane.emissive, emissive);
    sort_with_origins(&mut lane.distortion, distortion);
}

/// Writes the plan's current poses and lighting into the items. Fails, leaving the caller to
/// rebuild, when the lane and the plan no longer line up.
fn refresh_in_place(plan: &XModelDrawPlan, lane: &mut XModelDrawLane) -> bool {
    let XModelDrawLane {
        colour,
        emissive,
        distortion,
        origins,
        ..
    } = lane;
    let products = [
        (colour, &origins.colour),
        (emissive, &origins.emissive),
        (distortion, &origins.distortion),
    ];
    if products.iter().any(|(items, from)| items.len() != from.len()) {
        return false;
    }
    for (items, from) in products {
        for (item, &origin) in items.iter_mut().zip(from) {
            let RetainedDrawKind::XModel {
                world_from_local,
                lighting_handle,
                packed_lighting,
                is_scope,
                scene_entnum,
                ..
            } = &mut item.kind
            else {
                continue;
            };
            let Some(draw) = plan.draws.get(origin as usize) else {
                return false;
            };
            *world_from_local = draw.world_from_local;
            *lighting_handle = draw.lighting_handle;
            *packed_lighting = draw.packed_lighting;
            *is_scope = draw.is_scope;
            *scene_entnum = draw.scene_entnum;
        }
    }
    true
}

/// Brings the lane up to date with the merged plan: in place when the layout is unchanged,
/// by a full rebuild otherwise.
pub(crate) fn refresh_xmodel_lane(
    plan: &XModelDrawPlan,
    catalog: &RuntimeMaterialCatalog,
    lane: &mut XModelDrawLane,
) {
    let layout = layout_of(plan, catalog);
    lane.merge_packed_n = plan.packed_rows().map(|rows| rows.len() as u32);
    lane.fx_object_id_exhausted = plan.fx_object_id_exhausted;
    if layout == lane.layout && refresh_in_place(plan, lane) {
        lane.payload_revision = plan.revision;
        perf::Counter::XmodelLayoutOverlay.emit(1.0);
        return;
    }
    perf::Counter::XmodelLayoutOverlay.emit(0.0);
    rebuild(plan, catalog, lane);
    lane.layout = layout;
    lane.membership_revision = lane.membership_revision.wrapping_add(1);
    lane.payload_revision = plan.revision;
    perf::Counter::XmodelColourCameraFrustum.emit(f64::from(lane.skipped_camera_frustum));
    perf::Counter::XmodelColourNoLighting.emit(f64::from(lane.skipped_no_lighting));
}

/// Merges every model source into the plan, then refreshes the lane from it. A source that is
/// not present counts as empty.
pub(crate) fn rebuild_xmodel_draw_lane(
    sources: (
        Option<Res<FpvDrawPlan>>,
        Option<Res<RemoteBodyDrawPlan>>,
        Option<Res<ScriptModelDrawPlan>>,
        Option<Res<MissileDrawPlan>>,
        Option<Res<ItemDrawPlan>>,
        Option<Res<FxModelDrawPlan>>,
        Option<Res<DynEntDrawPlan>>,
        Option<Res<HostGfxScene>>,
    ),
    mut plan: ResMut<XModelDrawPlan>,
    sky: Option<Res<SkyModelDrawPlan>>,
    view: Option<Res<PreparedSceneView>>,
    generation: Res<MaterialGeneration>,
    mut lane: ResMut<XModelDrawLane>,
) {
    let (fpv, bodies, scripts, missiles, items, fx_models, dynents, scene) = sources;
    merge_xmodel_draw_plan(
        &mut plan,
        fpv.as_deref().unwrap_or(&Default::default()),
        bodies.as_deref().unwrap_or(&Default::default()),
        scripts.as_deref().unwrap_or(&Default::default()),
        missiles.as_deref().unwrap_or(&Default::default()),
        items.as_deref().unwrap_or(&Default::default()),
        fx_models.as_deref().unwrap_or(&Default::default()),
        dynents.as_deref().unwrap_or(&Default::default()),
        scene.as_ref().map(|scene| &scene.scene),
        sky.as_deref()
            .zip(view.as_ref().filter(|view| view.ready).map(|view| view.eye)),
    );
    refresh_xmodel_lane(&plan, &generation.catalog, &mut lane);
}
