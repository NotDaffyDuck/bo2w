use bevy::prelude::*;
use dpvs_t6::SortedDraw;
use dpvs_t6::SURFACE_KIND_MODEL_RIGID_BONED;
use dpvs_t6::encode_model_bone_driven_sort_key;
use dpvs_t6::with_env_probe_index;

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind, catalog_item, draw_order, route_to_products};
use crate::assemble::draw_items::list::{CONTENT_KEY_SEED, blend_content_id};
use crate::assemble::draw_items::mesh_build::sky::SkyDomeDrawPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::DynEntPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::FirstPersonPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::FxModelDrawList;
use crate::assemble::draw_items::mesh_build::figure_mesh::PickupPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::ThrownPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::OtherBodyPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::ScriptedModelPaintPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::MODEL_OBJECT_ID_FIRST_PERSON_WEAPON;
use crate::assemble::draw_items::mesh_build::figure_mesh::ModelTintRejection;
use crate::assemble::draw_items::mesh_build::figure_mesh::FigureDrawPlan;
use crate::assemble::draw_items::mesh_build::figure_mesh::FigureFacetDraw;
use crate::assemble::draw_items::mesh_build::figure_mesh::merge_figure_draw_plan;
use crate::{
    MaterialEpoch, LiveMaterialLedger, SurfaceCubeProbeId, FacetSamplerInputs,
    scene_light_index_of,
};
use crate::prepare::scene::visual_stage::HostVisualStage;
use crate::prepare::scene::eye_parms::StagedStageEye;

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
pub struct FigureDrawTrack {
    pub tint_rgb: Vec<HeldDrawItem>,
    pub glow: Vec<HeldDrawItem>,
    pub warp: Vec<HeldDrawItem>,

    origins: Origins,

    pub membership_edition: u64,
    layout: u64,

    pub payload_edition: u64,
    pub packed_row_count: Option<u32>,
    pub items_filed: u32,
    pub object_tag_standin: u32,
    pub effect_object_id_exhausted: u32,
    pub skipped_unranked: u32,
    pub skipped_unbaked: u32,
    pub skipped_camera_view_volume: u32,
    pub skipped_no_illum: u32,
}

/// Why a plan draw makes no item.
enum Skip {
    CameraViewVolume,
    NoMaterial,
    NoLighting,
    NoOrdinal,
    NoBakedKey,
}

impl FigureDrawTrack {
    fn reset(&mut self) {
        self.tint_rgb.clear();
        self.glow.clear();
        self.warp.clear();
        self.origins = Origins::default();
        self.items_filed = 0;
        self.object_tag_standin = 0;
        self.skipped_unranked = 0;
        self.skipped_unbaked = 0;
        self.skipped_camera_view_volume = 0;
        self.skipped_no_illum = 0;
    }

    fn count_skip(&mut self, skip: Skip) {
        let counter = match skip {
            Skip::CameraViewVolume => &mut self.skipped_camera_view_volume,
            Skip::NoLighting => &mut self.skipped_no_illum,
            Skip::NoOrdinal => &mut self.skipped_unranked,
            Skip::NoBakedKey => &mut self.skipped_unbaked,
            Skip::NoMaterial => return,
        };
        *counter = counter.saturating_add(1);
    }

    /// Files an item made from plan draw `origin`; every product that takes it notes the origin.
    fn file(&mut self, item: HeldDrawItem, origin: u32) {
        route_to_products(
            &mut self.tint_rgb,
            &mut self.glow,
            Some(&mut self.warp),
            item,
        );
        self.origins.colour.resize(self.tint_rgb.len(), origin);
        self.origins.emissive.resize(self.glow.len(), origin);
        self.origins.distortion.resize(self.warp.len(), origin);
    }
}

/// Fingerprint of everything that decides which items exist and in what order. Poses and
/// lighting handles that do not change an item's existence are left out.
fn layout_of(plan: &FigureDrawPlan, catalog: &LiveMaterialLedger) -> u64 {
    let mut id = CONTENT_KEY_SEED;
    let mut mix = |value: u64| blend_content_id(&mut id, value);
    mix(catalog.revision_stamp.0);
    mix(plan.topology_edition);
    mix(plan.draws.len() as u64);
    for draw in plan.draws.iter() {
        let material = usize::try_from(draw.material).ok().and_then(|at| plan.materials.get(at));
        mix(u64::from(draw.surface));
        mix(u64::from(draw.material));
        mix(u64::from(draw.object_tag));
        mix(u64::from(draw.stage_light_index));
        mix(u64::from(draw.env_probe_index));
        mix(u64::from(draw.colour_refusal.is_some()));
        mix(u64::from(
            material.is_some_and(|m| m.model_illum_required && draw.illum_handle == 0),
        ));
        mix(material
            .and_then(|m| m.material_draw_rank_slot)
            .map_or(u64::MAX, u64::from));
    }
    id
}

/// Makes the item for one plan draw, or says why there is none. The checks run in the order
/// the skip counters rely on.
fn draw_item(
    plan: &FigureDrawPlan,
    catalog: &LiveMaterialLedger,
    draw: &FigureFacetDraw,
) -> Result<HeldDrawItem, Skip> {
    let FigureFacetDraw {
        surface,
        material: material_slot,
        map_from_self: local_to_world,
        illum_handle,
        colour_refusal,
        object_tag,
        stage_light_index,
        env_probe_index,
        compact_illum,
        is_scoped_view,
        stage_actor_num,
        ..
    } = *draw;
    if colour_refusal == Some(ModelTintRejection::CameraViewVolume) {
        return Err(Skip::CameraViewVolume);
    }
    let row = usize::try_from(material_slot).ok().and_then(|at| plan.materials.get(at));
    let material = row.ok_or(Skip::NoMaterial)?;
    if material.model_illum_required && illum_handle == 0 {
        return Err(Skip::NoLighting);
    }
    let rank = material.material_draw_rank_slot.ok_or(Skip::NoOrdinal)?;
    let baked = catalog
        .by_sort_position(rank)
        .and_then(|row| row.baked_facet_draw)
        .ok_or(Skip::NoBakedKey)?;
    let boned = encode_model_bone_driven_sort_key(SortedDraw::unpack_from_bits(baked), object_tag);
    let probed = with_env_probe_index(boned, env_probe_index);
    let key = scene_light_index_of(probed.packed, stage_light_index);
    let keyed = SortedDraw::unpack_from_bits(key);
    debug_assert_eq!(keyed.kind(), SURFACE_KIND_MODEL_RIGID_BONED);
    Ok(catalog_item(
        key,
        rank,
        HeldDrawKind::XModel {
            surface,
            material: material_slot,
            object_tag,
            model_to_world: local_to_world,
            illum_handle,
            compact_illum,
            is_scoped_view,
            stage_actor_num,
        },
        FacetSamplerInputs {
            reflection_probe: Some(SurfaceCubeProbeId(keyed.env_probe_index())),
            ..Default::default()
        },
        catalog,
    ))
}

/// Sorts `items` into draw order and puts `origins` in the same order.
fn sort_with_origins(items: &mut [HeldDrawItem], origins: &mut [u32]) {
    debug_assert_eq!(items.len(), origins.len());
    let mut order: Vec<u32> = (0..u32::try_from(items.len()).unwrap_or(u32::MAX)).collect();
    order.sort_unstable_by_key(|&at| draw_order(&items[at as usize]));
    let sorted_items: Vec<HeldDrawItem> = order.iter().map(|&at| items[at as usize]).collect();
    let sorted_origins: Vec<u32> = order.iter().map(|&at| origins[at as usize]).collect();
    items.copy_from_slice(&sorted_items);
    origins.copy_from_slice(&sorted_origins);
}

fn rebuild(plan: &FigureDrawPlan, catalog: &LiveMaterialLedger, lane: &mut FigureDrawTrack) {
    lane.reset();
    for (index, dr) in plan.draws.iter().enumerate() {
        let origin = u32::try_from(index).unwrap_or(u32::MAX);
        match draw_item(plan, catalog, dr) {
            Ok(item) => {
                lane.file(item, origin);
                lane.items_filed = lane.items_filed.saturating_add(1);
                if dr.object_tag >= MODEL_OBJECT_ID_FIRST_PERSON_WEAPON {
                    lane.object_tag_standin = lane.object_tag_standin.saturating_add(1);
                }
            }
            Err(skip) => lane.count_skip(skip),
        }
    }
    let Origins {
        colour,
        emissive,
        distortion: warp_pass,
    } = &mut lane.origins;
    sort_with_origins(&mut lane.tint_rgb, colour);
    sort_with_origins(&mut lane.glow, emissive);
    sort_with_origins(&mut lane.warp, warp_pass);
}

/// Writes the plan's current poses and lighting into the items. Fails, leaving the caller to
/// rebuild, when the lane and the plan no longer line up.
fn refresh_in_place(plan: &FigureDrawPlan, lane: &mut FigureDrawTrack) -> bool {
    let FigureDrawTrack {
        tint_rgb: colour,
        glow: emissive,
        warp: distortion,
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
        for (draw_entry, &origin) in items.iter_mut().zip(from) {
            let HeldDrawKind::XModel {
                model_to_world: local_to_world,
                illum_handle,
                compact_illum,
                is_scoped_view,
                stage_actor_num,
                ..
            } = &mut draw_entry.kind
            else {
                continue;
            };
            let Some(draw) = plan.draws.get(origin as usize) else {
                return false;
            };
            *local_to_world = draw.map_from_self;
            *illum_handle = draw.illum_handle;
            *compact_illum = draw.compact_illum;
            *is_scoped_view = draw.is_scoped_view;
            *stage_actor_num = draw.stage_actor_num;
        }
    }
    true
}

/// Brings the lane up to date with the merged plan: in place when the layout is unchanged,
/// by a full rebuild otherwise.
pub(crate) fn refresh_xmodel_lane(
    plan: &FigureDrawPlan,
    catalog: &LiveMaterialLedger,
    lane: &mut FigureDrawTrack,
) {
    let layout = layout_of(plan, catalog);
    lane.packed_row_count = plan.compact_rows().map(|rows| rows.len() as u32);
    lane.effect_object_id_exhausted = plan.effect_object_id_exhausted;
    if layout == lane.layout && refresh_in_place(plan, lane) {
        lane.payload_edition = plan.revision;
        perf::Counter::FigureLayoutTopmost.emit(1.0);
        return;
    }
    perf::Counter::FigureLayoutTopmost.emit(0.0);
    rebuild(plan, catalog, lane);
    lane.layout = layout;
    lane.membership_edition = lane.membership_edition.wrapping_add(1);
    lane.payload_edition = plan.revision;
    perf::Counter::MeshShadeViewCone.emit(f64::from(lane.skipped_camera_view_volume));
    perf::Counter::MeshShadeUnlit.emit(f64::from(lane.skipped_no_illum));
}

/// Merges every model source into the plan, then refreshes the lane from it. A source that is
/// not present counts as empty.
pub(crate) fn rebuild_figure_draw_track(
    sources: (
        Option<Res<FirstPersonPaintPlan>>,
        Option<Res<OtherBodyPaintPlan>>,
        Option<Res<ScriptedModelPaintPlan>>,
        Option<Res<ThrownPaintPlan>>,
        Option<Res<PickupPaintPlan>>,
        Option<Res<FxModelDrawList>>,
        Option<Res<DynEntPaintPlan>>,
        Option<Res<HostVisualStage>>,
    ),
    mut plan: ResMut<FigureDrawPlan>,
    sky: Option<Res<SkyDomeDrawPlan>>,
    view: Option<Res<StagedStageEye>>,
    epoch: Res<MaterialEpoch>,
    mut lane: ResMut<FigureDrawTrack>,
) {
    let (fpv, bodies, scripts, missiles, items, effect_meshes, dynents, scene) = sources;
    merge_figure_draw_plan(
        &mut plan,
        fpv.as_deref().unwrap_or(&Default::default()),
        bodies.as_deref().unwrap_or(&Default::default()),
        scripts.as_deref().unwrap_or(&Default::default()),
        missiles.as_deref().unwrap_or(&Default::default()),
        items.as_deref().unwrap_or(&Default::default()),
        effect_meshes.as_deref().unwrap_or(&Default::default()),
        dynents.as_deref().unwrap_or(&Default::default()),
        scene.as_ref().map(|scene| &scene.scene),
        sky.as_deref().zip(view.as_ref().filter(|view| view.primed).map(|view| view.eye)),
    );
    refresh_xmodel_lane(&plan, &epoch.catalog, &mut lane);
}
