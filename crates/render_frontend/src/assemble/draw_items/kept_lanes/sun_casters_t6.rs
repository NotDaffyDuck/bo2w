//! The draws that cast the sun's shadow, built once per frame for the near and the far partition.
//!
//! Each partition bakes from its own visibility: the world surfaces that cast, then the static
//! models, which go through the game's surface buckets like the colour pass's models do (see
//! `model_filing_t6`). The two partitions then merge, with the animated models that cast, into
//! one list: near partition first, each part in draw order.

use bevy::prelude::{Mat4, Resource, Vec3};
use dpvs_t6::SortedDraw;
use dpvs_t6::SmodelShadowInst;
use dpvs_t6::SunShadowLodParms;
use dpvs_t6::encode_model_bone_driven_sort_key;
use lighting_t6::PropFacetBucketLists;
use render_material::SurfaceSortKeyPair;
use render_scene::XModelCasterBound;

use super::model_sorting_t6::BucketKey;
use super::model_sorting_t6::Filer;
use super::model_sorting_t6::Pass;
use super::model_sorting_t6::Waiting;
use super::model_sorting_t6::cache_slot_no;
use super::model_sorting_t6::has_lighting;
use super::model_sorting_t6::lod_is_rigid;
use super::model_sorting_t6::lod_smc_flag;
use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind, catalog_item, draw_order};
use crate::assemble::draw_items::material_live::surface_add_gains_technique;
use crate::mesh_build::prop_mesh::{DetailRampArgs, PropGpuPlan, prop_camera_detail};
use crate::assemble::draw_items::mesh_build::figure_mesh::{MODEL_OBJECT_ID_FIRST_PERSON_WEAPON, FigureDrawPlan};
use crate::MaterialEpochId;
use crate::LiveMaterialLedger;
use crate::DAYLIGHT_CASTER_TECH;
use crate::SurfaceCubeProbeId;
use crate::FacetSamplerInputs;
use crate::MethodType;
use crate::WorldGpuSubmission;
use crate::prop_casts_daylight_shadow;
use crate::scene_light_index_of;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::world::LevelHide;
use crate::prepare::scene::world::LevelSight;

/// The game draws light-volume static models into the sun shadow unconditionally; this port has
/// no dvar for it.
const DRAW_SMODELS_LV: bool = true;

/// What the sun shadow draws this frame. A partition bakes one; the merge of both partitions is
/// the plan the frame products read.
#[derive(Resource, Clone, Debug, Default)]
pub struct DaylightShadowCasterPlan {
    pub revision_stamp: MaterialEpochId,
    /// Near partition first, then far; each part is in draw order.
    pub item_list: Vec<HeldDrawItem>,
    /// How many of `items` belong to the near partition.
    pub daylight_near_tally: usize,
    /// World surfaces that cast, and those of them that could not be given a draw key.
    pub level_eligible: u32,
    pub level_missing_key: u32,
    /// Static models that cast, and those of them that drew nothing and did not opt out.
    pub prop_eligible: u32,
    pub prop_missing_key: u32,
    /// The index list the pre-tessellated and cached model draws read.
    pub prop_prebuilt_indices: Vec<u16>,
}

/// Buffers a partition's bake reuses from frame to frame.
#[derive(Default)]
pub(crate) struct SunScratch {
    world: Vec<u16>,
    fixed_props: Vec<u16>,
}

/// What a partition's bake reads besides its visibility.
#[derive(Clone, Copy)]
pub(crate) struct SunBakeInputs<'a> {
    pub(crate) cull: &'a LevelHide,
    pub(crate) level_plan: &'a WorldGpuSubmission,
    pub(crate) prop_plan: &'a PropGpuPlan,
    pub(crate) catalog: &'a LiveMaterialLedger,
    pub(crate) cache: Option<&'a LevelPropCache>,
    pub(crate) lighting: Option<&'a LevelPropIllum>,
    pub(crate) pretess_on: bool,
    /// The camera, which static models pick their detail level and cull distance against.
    pub(crate) eye: Option<Vec3>,
    pub(crate) ramp: DetailRampArgs,
}

/// What the partition's frustum lets through, one byte per world surface and per static model.
#[derive(Clone, Copy)]
pub(crate) struct SunVisibility<'a> {
    pub(crate) surfaces: &'a [u8],
    pub(crate) fixed_props: &'a [u8],
}

/// Bakes the casters one partition sees.
pub(crate) fn bake_partition(
    inputs: SunBakeInputs<'_>,
    vis: SunVisibility<'_>,
    scratch: &mut SunScratch,
) -> DaylightShadowCasterPlan {
    let mut plan = DaylightShadowCasterPlan {
        revision_stamp: inputs.catalog.revision_stamp,
        ..Default::default()
    };
    bake_world(&inputs, vis.surfaces, &mut scratch.world, &mut plan);

    let mut models = SmodelPass::new(inputs);
    let insts = &inputs.prop_plan.shadow_draw_instances;
    let cap = vis.fixed_props.len().min(insts.len());
    if scratch.fixed_props.len() < cap {
        scratch.fixed_props.resize(cap, 0);
    }
    // The game culls by cull distance right here, against the camera's detail-level origin.
    let lod = inputs.eye.map(|camera| SunShadowLodParms {
        origin: camera.to_array(),
        scale: inputs.ramp.t5.scale,
        bias: inputs.ramp.t5.bias,
    });
    let found = dpvs_t6::queue_static_range_sun_shadow(
        vis.fixed_props,
        insts,
        lod,
        DRAW_SMODELS_LV,
        &mut scratch.fixed_props[..cap],
    );
    for &slot in &scratch.fixed_props[..found] {
        models.file_model(slot);
    }
    let (model_items, indices) = models.finish(&mut plan);
    plan.item_list.extend(model_items);
    plan.prop_prebuilt_indices = indices;
    plan.item_list.sort_unstable_by_key(draw_order);
    plan
}

/// Merges the partitions and the animated models that cast into the plan for the frame.
/// `planes` are the clip planes of the near and far partition.
pub(crate) fn merge_partitions(
    near: DaylightShadowCasterPlan,
    far: DaylightShadowCasterPlan,
    xmodel: Option<&FigureDrawPlan>,
    catalog: &LiveMaterialLedger,
    planes: [&[[f32; 4]]; 2],
) -> DaylightShadowCasterPlan {
    let mut dynamic = xmodel
        .map(|xmodel| animated_casters(xmodel, catalog))
        .unwrap_or_default();
    dynamic.sort_unstable_by_key(|caster| draw_order(&caster.item));
    let kept = |planes: &[[f32; 4]]| -> Vec<HeldDrawItem> {
        dynamic
            .iter()
            // A caster reaches a partition unless its own sphere is wholly outside it. A partition
            // that states no planes and a caster that states no sphere each widen the volume
            // instead of dropping a shadow.
            .filter(|caster| planes.is_empty() || caster.bound.is_none_or(|sphere| !sphere.outside(planes)))
            .map(|caster| caster.item)
            .collect()
    };
    let mut items = merge_sorted(near.item_list, &kept(planes[0]));
    let daylight_near_tally = items.len();
    items.extend(merge_sorted(far.item_list, &kept(planes[1])));

    let mut pretess = near.prop_prebuilt_indices;
    pretess.extend(far.prop_prebuilt_indices);
    DaylightShadowCasterPlan {
        revision_stamp: near.revision_stamp,
        item_list: items,
        daylight_near_tally,
        level_eligible: near.level_eligible.saturating_add(far.level_eligible),
        level_missing_key: near.level_missing_key.saturating_add(far.level_missing_key),
        prop_eligible: near.prop_eligible.saturating_add(far.prop_eligible),
        prop_missing_key: near.prop_missing_key.saturating_add(far.prop_missing_key),
        prop_prebuilt_indices: pretess,
    }
}

/// The per-instance data the static model sun cull reads, one entry per authored model slot.
pub(crate) fn fill_prop_draw_inst_shadow(
    out: &mut Vec<SmodelShadowInst>,
    hide_dists: &[u16],
    prop_plan: &PropGpuPlan,
) {
    out.clear();
    out.extend(
        prop_plan
            .designer_placement_indices
            .iter()
            .enumerate()
            .map(|(seat, placement)| {
                let Some(spot) = placement.and_then(|i| prop_plan.placements.get(i)) else {
                    return SmodelShadowInst::default();
                };
                let (pose, flags) = (spot.local_to_world, spot.mode_bits);
                SmodelShadowInst {
                    hide_dist: f32::from(hide_dists.get(seat).copied().unwrap_or(0)),
                    origin: pose.w_axis.truncate().to_array(),
                    flags,
                    key_light_index: spot.key_light_index,
                }
            }),
    );
}

// ---- world surfaces ----

/// Adds the world surfaces the partition sees that cast: the level's own range, then the
/// brush-model surfaces outside it, which draw at their model's pose.
fn bake_world(
    inputs: &SunBakeInputs<'_>,
    vis: &[u8],
    ids: &mut Vec<u16>,
    plan: &mut DaylightShadowCasterPlan,
) {
    let cull = inputs.cull;
    let casters = &cull.capture.casters;
    let facet_sum = cull.facet_materials.len();
    let (begin, end) = world_range(&cull.dpvs, facet_sum);
    if cull.capture.compact_facet_draws.is_empty() {
        for surf in begin as usize..(end as usize).min(facet_sum) {
            if vis.get(surf).is_some_and(|&seen| seen != 0) && casters.get(surf) {
                add_world_surface(inputs, surf, Mat4::default(), plan);
            }
        }
    } else {
        render_frontend::add_level_tree_daylight_shadow_partition(
            begin,
            end,
            vis,
            casters.bit_words(),
            &cull.capture.compact_facet_draws,
            ids,
        );
        for &surf in ids.iter() {
            let facet_ix = usize::from(surf);
            add_world_surface(inputs, facet_ix, Mat4::default(), plan);
        }
    }
    for (surf, pose) in brush_model_extras(cull) {
        if vis.get(surf).is_some_and(|&seen| seen != 0) {
            add_world_surface(inputs, surf, pose, plan);
        }
    }
}

/// The surfaces the level's own sun shadow range covers.
fn world_range(dpvs: &LevelSight, facet_amount: usize) -> (u32, u32) {
    let begin = dpvs.lit_solid_begin;
    let end = if dpvs.emissive_facets_end > begin {
        dpvs.emissive_facets_end
    } else if dpvs.lit_solid_end > begin {
        dpvs.lit_solid_end
    } else {
        facet_amount as u32
    };
    (begin, end.min(facet_amount as u32))
}

/// The surfaces of the brush models (all but the level itself) that the level's cast set leaves
/// out, with the pose of the model each belongs to.
fn brush_model_extras(cull: &LevelHide) -> Vec<(usize, Mat4)> {
    let casters = &cull.capture.casters;
    let mut extras = Vec::new();
    for (index, model) in cull.solid_models.iter().enumerate().skip(1) {
        let pose = cull
            .solid_model_level_from_local
            .get(index)
            .copied()
            .unwrap_or_default();
        let first = usize::from(model.start_facet);
        let last = first
            .saturating_add(usize::from(model.facet_tally))
            .min(casters.len());
        extras.extend(
            (first..last)
                .filter(|&surf| !casters.get(surf))
                .map(|surf| (surf, pose)),
        );
    }
    extras
}

fn add_world_surface(
    inputs: &SunBakeInputs<'_>,
    facet: usize,
    pose: Mat4,
    plan: &mut DaylightShadowCasterPlan,
) {
    plan.level_eligible = plan.level_eligible.saturating_add(1);
    match world_item(inputs, facet, pose) {
        Some(item) => plan.item_list.push(item),
        None => plan.level_missing_key = plan.level_missing_key.saturating_add(1),
    }
}

/// The draw of one world surface. A surface the map baked a key for uses it; any other takes
/// the key its material bakes, lit by the surface's primary light.
fn world_item(inputs: &SunBakeInputs<'_>, facet_ix: usize, pose: Mat4) -> Option<HeldDrawItem> {
    let cull = inputs.cull;
    let surf16 = u16::try_from(facet_ix).ok()?;
    let baked = cull
        .capture
        .compact_facet_draws
        .get(facet_ix)
        .map(|word| word.packed)
        .filter(|&packed| packed != 0);
    let key = match baked {
        Some(packed) => packed,
        None => {
            let material = cull.facet_materials.get(facet_ix).copied().flatten()?;
            let packed = inputs.catalog.derived(material)?.baked_facet_draw?;
            let light = cull.facet_key_lights.get(facet_ix).copied().unwrap_or(0);
            scene_light_index_of(packed, light)
        }
    };
    let samplers = inputs
        .level_plan
        .facet_sampler_inputs
        .get(facet_ix)
        .copied()
        .unwrap_or_default();
    let rank = u32::from(SortedDraw::unpack_from_bits(key).material_draw_rank_slot());
    Some(catalog_item(
        key,
        rank,
        HeldDrawKind::posed_face(surf16, pose),
        samplers,
        inputs.catalog,
    ))
}

// ---- static models ----

/// Files the static models the partition sees into the surface buckets and collects the draws.
struct SmodelPass<'a> {
    inputs: SunBakeInputs<'a>,
    filer: Filer<'a>,
    lists: PropFacetBucketLists,
    eligible: u32,
    missing: u32,
    /// Every placement that was filed, so those that drew nothing can be counted at the end.
    filed: Vec<u32>,
}

impl<'a> SmodelPass<'a> {
    fn new(inputs: SunBakeInputs<'a>) -> Self {
        Self {
            filer: Filer::new(
                inputs.prop_plan,
                inputs.catalog,
                inputs.cache,
                inputs.pretess_on,
            ),
            inputs,
            lists: PropFacetBucketLists::default(),
            eligible: 0,
            missing: 0,
            filed: Vec::new(),
        }
    }

    /// Files the model in authored slot `slot`, unless it cannot cast.
    fn file_model(&mut self, slot: u16) {
        let SunBakeInputs {
            prop_plan: plan,
            cache,
            lighting,
            eye,
            ramp,
            ..
        } = self.inputs;
        let Some(index) = plan
            .designer_placement_indices
            .get(usize::from(slot))
            .copied()
            .flatten()
        else {
            self.missing += 1;
            return;
        };
        let Some(pose_at) = plan.placements.get(index) else {
            self.missing += 1;
            return;
        };
        if !prop_casts_daylight_shadow(pose_at.mode_bits) {
            return;
        }
        self.eligible += 1;
        let Some(shape) = plan.meshes.get(pose_at.model_slot) else {
            self.missing += 1;
            return;
        };
        let Some(lod) = prop_camera_detail(shape.lod, pose_at.at_pos, pose_at.size_k, eye, ramp)
        else {
            self.missing += 1;
            return;
        };
        let detail_i = usize::from(lod);
        if shape
            .facets_by_detail
            .get(detail_i)
            .is_none_or(|surfaces| surfaces.is_empty())
        {
            self.missing += 1;
            return;
        }
        let Ok(mesh_slot_number) = u16::try_from(pose_at.illum_slot.unwrap_or(index)) else {
            return;
        };
        let packed = pose_at.compact_illum.or_else(|| {
            pose_at
                .illum_slot
                .and_then(|slot| lighting.and_then(|l| l.compact_illum_for_slot(slot)))
        });
        let key = BucketKey {
            lod: i32::from(lod),
            smc_enabled: false,
            lit: has_lighting(packed),
            smc_flag: lod_smc_flag(shape, detail_i),
            cache_slot_no: cache_slot_no(cache, pose_at.illum_slot, detail_i),
            mesh_slot_number,
            rigid: lod_is_rigid(shape, detail_i),
        };
        let placement_id = index as u32;
        self.filed.push(placement_id);
        let model = Waiting::new(
            placement_id,
            lod,
            pose_at.local_to_world,
            0,
            packed,
            Pass::SunShadow,
        );
        let filed = self.filer.file(&mut self.lists, key, model);
        if filed.bucket.is_none() {
            self.filer.note_unfiled(model);
        }
        self.filer.drain_filled(&mut self.lists, filed);
    }

    /// Drains what is left and hands over the draws and the index list. Adds the model counts
    /// to `plan`: a filed model counts as missing when none of its surfaces drew and they did
    /// not all opt out of the sun pass.
    fn finish(mut self, plan: &mut DaylightShadowCasterPlan) -> (Vec<HeldDrawItem>, Vec<u16>) {
        self.filer.drain_rest(&mut self.lists);
        let filings = self.filer.finish();
        let mut shown: Vec<u32> = filings
            .items
            .iter()
            .filter_map(|item| match item.kind {
                HeldDrawKind::Smodel { placement, .. } => Some(placement),
                _ => None,
            })
            .collect();
        shown.sort_unstable();
        shown.dedup();
        // A batch of cached models that re-tessellate together draws only through its first
        // model, so the rest of that batch land here as missing.
        let silent = self
            .filed
            .iter()
            .filter(|placement| {
                shown.binary_search(placement).is_err()
                    && filings.opted_out.binary_search(placement).is_err()
            })
            .count();
        plan.prop_eligible = plan.prop_eligible.saturating_add(self.eligible);
        plan.prop_missing_key = plan
            .prop_missing_key
            .saturating_add(self.missing)
            .saturating_add(silent as u32);
        (filings.items, filings.indices)
    }
}

// ---- animated models ----

/// An animated model's draw and the sphere the partition test reads. A caster with no sphere
/// reaches both partitions.
struct AnimatedCaster {
    item: HeldDrawItem,
    bound: Option<XModelCasterBound>,
}

/// The animated models that cast: all but the viewmodel and its scope, with a material that has
/// a sun shadow technique.
fn animated_casters(xmodel: &FigureDrawPlan, catalog: &LiveMaterialLedger) -> Vec<AnimatedCaster> {
    let mut casters = Vec::new();
    for figure in &xmodel.draws {
        if figure.object_tag == MODEL_OBJECT_ID_FIRST_PERSON_WEAPON || figure.is_scoped_view {
            continue;
        }
        let mats = &xmodel.materials;
        let Some(rank) = mats
            .get(figure.material as usize)
            .and_then(|material| material.material_draw_rank_slot)
        else {
            continue;
        };
        let Some(bits) = catalog
            .by_sort_position(rank)
            .and_then(|material| material.baked_facet_draw)
        else {
            continue;
        };
        let key =
            encode_model_bone_driven_sort_key(SortedDraw::unpack_from_bits(bits), figure.object_tag)
                .packed;
        if !surface_add_gains_technique(
            catalog,
            SurfaceSortKeyPair::new(key, rank),
            MethodType(DAYLIGHT_CASTER_TECH),
        ) {
            continue;
        }
        let item = catalog_item(
            key,
            rank,
            HeldDrawKind::XModel {
                surface: figure.surface,
                material: figure.material,
                object_tag: figure.object_tag,
                model_to_world: figure.map_from_self,
                illum_handle: figure.illum_handle,
                compact_illum: figure.compact_illum,
                is_scoped_view: figure.is_scoped_view,
                stage_actor_num: figure.stage_actor_num,
            },
            FacetSamplerInputs {
                reflection_probe: Some(SurfaceCubeProbeId(figure.env_probe_index)),
                ..Default::default()
            },
            catalog,
        );
        casters.push(AnimatedCaster {
            item,
            bound: figure.caster_bound,
        });
    }
    casters
}

/// Merges two lists that are each in draw order; on a tie `base` comes first.
fn merge_sorted(first_index: Vec<HeldDrawItem>, extra: &[HeldDrawItem]) -> Vec<HeldDrawItem> {
    let [_, ..] = extra else {
        return first_index;
    };
    let mut merged = Vec::with_capacity(first_index.len() + extra.len());
    let (mut a, mut b) = (0, 0);
    while a < first_index.len() && b < extra.len() {
        if draw_order(&first_index[a]) <= draw_order(&extra[b]) {
            merged.push(first_index[a]);
            a += 1;
        } else {
            merged.push(extra[b]);
            b += 1;
        }
    }
    merged.extend_from_slice(&first_index[a..]);
    merged.extend_from_slice(&extra[b..]);
    merged
}
