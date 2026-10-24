//! The draws that cast the sun's shadow, built once per frame for the near and the far partition.
//!
//! Each partition bakes from its own visibility: the world surfaces that cast, then the static
//! models, which go through the game's surface buckets like the colour pass's models do (see
//! `model_filing_t6`). The two partitions then merge, with the animated models that cast, into
//! one list: near partition first, each part in draw order.

use bevy::prelude::{Mat4, Resource, Vec3};
use dpvs_t6::{GfxDrawSurf, SmodelShadowInst, SunShadowLodParms, pack_xmodel_rigid_skinned_draw_surf};
use lighting_t6::SmodelSurfBucketLists;
use render_material::MaterialDrawKey;
use render_scene::XModelCasterBound;

use super::model_filing_t6::{
    BucketKey, Filer, Pass, Queued, cache_index, lighting_nonzero, lod_is_rigid, lod_smc_flag,
};
use super::{RetainedDrawItem, RetainedDrawKind, catalog_item, draw_order};
use crate::assemble::drawsurf::material_runtime::add_surf_has_technique;
use crate::assemble::drawsurf::tess::smodel::{LodRampArgs, SmodelGpuPlan, smodel_camera_lod};
use crate::assemble::drawsurf::tess::xmodel::{XMODEL_OBJECT_ID_VIEWMODEL, XModelDrawPlan};
use crate::assemble::drawsurf::{
    MaterialGenerationId, RuntimeMaterialCatalog, SUN_SHADOW_CASTER_TECH, SurfaceReflectionProbeId,
    SurfaceSamplerInputs, TechType, WorldDrawGpuPlan, smodel_casts_sun_shadow,
    with_scene_light_index,
};
use crate::prepare::scene::smodel_geom_cache::WorldStaticModelCache;
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::world::{WorldCull, WorldDpvs};

/// The game draws light-volume static models into the sun shadow unconditionally; this port has
/// no dvar for it.
const DRAW_SMODELS_LV: bool = true;

/// What the sun shadow draws this frame. A partition bakes one; the merge of both partitions is
/// the plan the frame products read.
#[derive(Resource, Clone, Debug, Default)]
pub struct SunShadowCasterPlan {
    pub generation_id: MaterialGenerationId,
    /// Near partition first, then far; each part is in draw order.
    pub items: Vec<RetainedDrawItem>,
    /// How many of `items` belong to the near partition.
    pub sun_near_n: usize,
    /// World surfaces that cast, and those of them that could not be given a draw key.
    pub world_eligible: u32,
    pub world_missing_key: u32,
    /// Static models that cast, and those of them that drew nothing and did not opt out.
    pub smodel_eligible: u32,
    pub smodel_missing_key: u32,
    /// The index list the pre-tessellated and cached model draws read.
    pub smodel_pretess_indices: Vec<u16>,
}

/// Buffers a partition's bake reuses from frame to frame.
#[derive(Default)]
pub(crate) struct SunScratch {
    world: Vec<u16>,
    smodels: Vec<u16>,
}

/// What a partition's bake reads besides its visibility.
#[derive(Clone, Copy)]
pub(crate) struct SunBakeInputs<'a> {
    pub(crate) cull: &'a WorldCull,
    pub(crate) world_plan: &'a WorldDrawGpuPlan,
    pub(crate) smodel_plan: &'a SmodelGpuPlan,
    pub(crate) catalog: &'a RuntimeMaterialCatalog,
    pub(crate) cache: Option<&'a WorldStaticModelCache>,
    pub(crate) lighting: Option<&'a WorldSmodelLighting>,
    pub(crate) pretess_on: bool,
    /// The camera, which static models pick their detail level and cull distance against.
    pub(crate) eye: Option<Vec3>,
    pub(crate) ramp: LodRampArgs,
}

/// What the partition's frustum lets through, one byte per world surface and per static model.
#[derive(Clone, Copy)]
pub(crate) struct SunVisibility<'a> {
    pub(crate) surfaces: &'a [u8],
    pub(crate) smodels: &'a [u8],
}

/// Bakes the casters one partition sees.
pub(crate) fn bake_partition(
    inputs: SunBakeInputs<'_>,
    vis: SunVisibility<'_>,
    scratch: &mut SunScratch,
) -> SunShadowCasterPlan {
    let mut plan = SunShadowCasterPlan {
        generation_id: inputs.catalog.generation_id,
        ..Default::default()
    };
    bake_world(&inputs, vis.surfaces, &mut scratch.world, &mut plan);

    let mut models = SmodelPass::new(inputs);
    let insts = &inputs.smodel_plan.shadow_draw_insts;
    let cap = vis.smodels.len().min(insts.len());
    if scratch.smodels.len() < cap {
        scratch.smodels.resize(cap, 0);
    }
    // The game culls by cull distance right here, against the camera's detail-level origin.
    let lod = inputs.eye.map(|eye| SunShadowLodParms {
        origin: eye.to_array(),
        scale: inputs.ramp.t5.scale,
        bias: inputs.ramp.t5.bias,
    });
    let found = dpvs_t6::add_smodel_range_sun_shadow(
        vis.smodels,
        insts,
        lod,
        DRAW_SMODELS_LV,
        &mut scratch.smodels[..cap],
    );
    for &slot in &scratch.smodels[..found] {
        models.add(slot);
    }
    let (items, indices) = models.finish(&mut plan);
    plan.items.extend(items);
    plan.smodel_pretess_indices = indices;
    plan.items.sort_unstable_by_key(draw_order);
    plan
}

/// Merges the partitions and the animated models that cast into the plan for the frame.
/// `planes` are the clip planes of the near and far partition.
pub(crate) fn merge_partitions(
    near: SunShadowCasterPlan,
    far: SunShadowCasterPlan,
    xmodel: Option<&XModelDrawPlan>,
    catalog: &RuntimeMaterialCatalog,
    planes: [&[[f32; 4]]; 2],
) -> SunShadowCasterPlan {
    let mut dynamic = xmodel
        .map(|xmodel| animated_casters(xmodel, catalog))
        .unwrap_or_default();
    dynamic.sort_unstable_by_key(|caster| draw_order(&caster.item));
    let kept = |planes: &[[f32; 4]]| -> Vec<RetainedDrawItem> {
        dynamic
            .iter()
            .filter(|caster| reaches(caster.bound, planes))
            .map(|caster| caster.item)
            .collect()
    };
    let mut items = merge_sorted(near.items, &kept(planes[0]));
    let sun_near_n = items.len();
    items.extend(merge_sorted(far.items, &kept(planes[1])));

    let mut pretess = near.smodel_pretess_indices;
    pretess.extend(far.smodel_pretess_indices);
    SunShadowCasterPlan {
        generation_id: near.generation_id,
        items,
        sun_near_n,
        world_eligible: near.world_eligible.saturating_add(far.world_eligible),
        world_missing_key: near.world_missing_key.saturating_add(far.world_missing_key),
        smodel_eligible: near.smodel_eligible.saturating_add(far.smodel_eligible),
        smodel_missing_key: near.smodel_missing_key.saturating_add(far.smodel_missing_key),
        smodel_pretess_indices: pretess,
    }
}

/// The per-instance data the static model sun cull reads, one entry per authored model slot.
pub(crate) fn fill_smodel_draw_inst_shadow(
    out: &mut Vec<SmodelShadowInst>,
    cull_dists: &[u16],
    smodel_plan: &SmodelGpuPlan,
) {
    out.clear();
    out.extend(
        smodel_plan
            .authored_placement_indices
            .iter()
            .enumerate()
            .map(|(slot, placement)| {
                let Some(placement) = placement.and_then(|i| smodel_plan.placements.get(i)) else {
                    return SmodelShadowInst::default();
                };
                SmodelShadowInst {
                    cull_dist: f32::from(cull_dists.get(slot).copied().unwrap_or(0)),
                    origin: placement.world_from_local.w_axis.truncate().to_array(),
                    flags: placement.flags,
                    primary_light_index: placement.primary_light_index,
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
    plan: &mut SunShadowCasterPlan,
) {
    let cull = inputs.cull;
    let surf_n = cull.surface_materials.len();
    let (begin, end) = world_range(&cull.dpvs, surf_n);
    if cull.capture.packed_draw_surfs.is_empty() {
        for surf in begin as usize..(end as usize).min(surf_n) {
            if vis.get(surf).is_some_and(|&seen| seen != 0) && cull.capture.casters.get(surf) {
                add_world_surface(inputs, surf, Mat4::IDENTITY, plan);
            }
        }
    } else {
        render_frontend::add_bsp_sun_shadow_partition(
            begin,
            end,
            vis,
            cull.capture.casters.words(),
            &cull.capture.packed_draw_surfs,
            ids,
        );
        for &surf in ids.iter() {
            add_world_surface(inputs, usize::from(surf), Mat4::IDENTITY, plan);
        }
    }
    for (surf, pose) in brush_model_extras(cull) {
        if vis.get(surf).is_some_and(|&seen| seen != 0) {
            add_world_surface(inputs, surf, pose, plan);
        }
    }
}

/// The surfaces the level's own sun shadow range covers.
fn world_range(dpvs: &WorldDpvs, surf_count: usize) -> (u32, u32) {
    let begin = dpvs.lit_opaque_begin;
    let end = if dpvs.emissive_surfs_end > begin {
        dpvs.emissive_surfs_end
    } else if dpvs.lit_opaque_end > begin {
        dpvs.lit_opaque_end
    } else {
        surf_count as u32
    };
    (begin, end.min(surf_count as u32))
}

/// The surfaces of the brush models (all but the level itself) that the level's cast set leaves
/// out, with the pose of the model each belongs to.
fn brush_model_extras(cull: &WorldCull) -> Vec<(usize, Mat4)> {
    let casters = &cull.capture.casters;
    let mut extras = Vec::new();
    for (index, model) in cull.brush_models.iter().enumerate().skip(1) {
        let pose = cull
            .bmodel_world_from_local
            .get(index)
            .copied()
            .unwrap_or(Mat4::IDENTITY);
        let first = usize::from(model.start_surf);
        let last = first
            .saturating_add(usize::from(model.surface_count))
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
    surf: usize,
    pose: Mat4,
    plan: &mut SunShadowCasterPlan,
) {
    plan.world_eligible = plan.world_eligible.saturating_add(1);
    match world_item(inputs, surf, pose) {
        Some(item) => plan.items.push(item),
        None => plan.world_missing_key = plan.world_missing_key.saturating_add(1),
    }
}

/// The draw of one world surface. A surface the map baked a key for uses it; any other takes
/// the key its material bakes, lit by the surface's primary light.
fn world_item(inputs: &SunBakeInputs<'_>, surf: usize, pose: Mat4) -> Option<RetainedDrawItem> {
    let cull = inputs.cull;
    let surf16 = u16::try_from(surf).ok()?;
    let baked = cull
        .capture
        .packed_draw_surfs
        .get(surf)
        .map(|word| word.packed)
        .filter(|&packed| packed != 0);
    let key = match baked {
        Some(packed) => packed,
        None => {
            let material = cull.surface_materials.get(surf).copied().flatten()?;
            let packed = inputs.catalog.derived(material)?.baked_draw_surf?;
            let light = cull.surface_primary_lights.get(surf).copied().unwrap_or(0);
            with_scene_light_index(packed, light)
        }
    };
    let samplers = inputs
        .world_plan
        .surface_sampler_inputs
        .get(surf)
        .copied()
        .unwrap_or_default();
    let rank = u32::from(GfxDrawSurf::from_packed(key).material_sorted_index());
    Some(catalog_item(
        key,
        rank,
        RetainedDrawKind::world_with_pose(surf16, pose),
        samplers,
        inputs.catalog,
    ))
}

// ---- static models ----

/// Files the static models the partition sees into the surface buckets and collects the draws.
struct SmodelPass<'a> {
    inputs: SunBakeInputs<'a>,
    filer: Filer<'a>,
    lists: SmodelSurfBucketLists,
    eligible: u32,
    missing: u32,
    /// Every placement that was filed, so those that drew nothing can be counted at the end.
    filed: Vec<u32>,
}

impl<'a> SmodelPass<'a> {
    fn new(inputs: SunBakeInputs<'a>) -> Self {
        Self {
            filer: Filer::new(
                inputs.smodel_plan,
                inputs.catalog,
                inputs.cache,
                inputs.pretess_on,
            ),
            inputs,
            lists: SmodelSurfBucketLists::default(),
            eligible: 0,
            missing: 0,
            filed: Vec::new(),
        }
    }

    /// Files the model in authored slot `slot`, unless it cannot cast.
    fn add(&mut self, slot: u16) {
        let SunBakeInputs {
            smodel_plan: plan,
            cache,
            lighting,
            eye,
            ramp,
            ..
        } = self.inputs;
        let Some(index) = plan
            .authored_placement_indices
            .get(usize::from(slot))
            .copied()
            .flatten()
        else {
            self.missing += 1;
            return;
        };
        let Some(placement) = plan.placements.get(index) else {
            self.missing += 1;
            return;
        };
        if !smodel_casts_sun_shadow(placement.flags) {
            return;
        }
        self.eligible += 1;
        let Some(mesh) = plan.meshes.get(placement.mesh) else {
            self.missing += 1;
            return;
        };
        let Some(lod) = smodel_camera_lod(mesh.lod, placement.origin, placement.scale, eye, ramp)
        else {
            self.missing += 1;
            return;
        };
        let lod_i = usize::from(lod);
        if mesh
            .surfaces_by_lod
            .get(lod_i)
            .is_none_or(|surfaces| surfaces.is_empty())
        {
            self.missing += 1;
            return;
        }
        let Ok(model_index) = u16::try_from(placement.lighting_slot.unwrap_or(index)) else {
            return;
        };
        let packed = placement.packed_lighting.or_else(|| {
            placement
                .lighting_slot
                .and_then(|slot| lighting.and_then(|l| l.packed_lighting_for_slot(slot)))
        });
        let key = BucketKey {
            lod: i32::from(lod),
            smc_enabled: false,
            lit: lighting_nonzero(packed),
            smc_flag: lod_smc_flag(mesh, lod_i),
            cache_index: cache_index(cache, placement.lighting_slot, lod_i),
            model_index,
            rigid: lod_is_rigid(mesh, lod_i),
        };
        let placement_id = index as u32;
        self.filed.push(placement_id);
        let model = Queued::new(
            placement_id,
            lod,
            placement.world_from_local,
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
    fn finish(mut self, plan: &mut SunShadowCasterPlan) -> (Vec<RetainedDrawItem>, Vec<u16>) {
        self.filer.drain_rest(&mut self.lists);
        let filings = self.filer.finish();
        let mut drawn: Vec<u32> = filings
            .items
            .iter()
            .filter_map(|item| match item.kind {
                RetainedDrawKind::Smodel { placement, .. } => Some(placement),
                _ => None,
            })
            .collect();
        drawn.sort_unstable();
        drawn.dedup();
        // A batch of cached models that re-tessellate together draws only through its first
        // model, so the rest of that batch land here as missing.
        let silent = self
            .filed
            .iter()
            .filter(|placement| {
                drawn.binary_search(placement).is_err()
                    && filings.opted_out.binary_search(placement).is_err()
            })
            .count();
        plan.smodel_eligible = plan.smodel_eligible.saturating_add(self.eligible);
        plan.smodel_missing_key = plan
            .smodel_missing_key
            .saturating_add(self.missing)
            .saturating_add(silent as u32);
        (filings.items, filings.indices)
    }
}

// ---- animated models ----

/// An animated model's draw and the sphere the partition test reads. A caster with no sphere
/// reaches both partitions.
struct AnimatedCaster {
    item: RetainedDrawItem,
    bound: Option<XModelCasterBound>,
}

/// The animated models that cast: all but the viewmodel and its scope, with a material that has
/// a sun shadow technique.
fn animated_casters(xmodel: &XModelDrawPlan, catalog: &RuntimeMaterialCatalog) -> Vec<AnimatedCaster> {
    let mut casters = Vec::new();
    for draw in &xmodel.draws {
        if draw.object_id == XMODEL_OBJECT_ID_VIEWMODEL || draw.is_scope {
            continue;
        }
        let Some(rank) = xmodel
            .materials
            .get(draw.material as usize)
            .and_then(|material| material.material_sorted_index)
        else {
            continue;
        };
        let Some(baked) = catalog
            .material_for_sorted_ordinal(rank)
            .and_then(|material| material.baked_draw_surf)
        else {
            continue;
        };
        let key =
            pack_xmodel_rigid_skinned_draw_surf(GfxDrawSurf::from_packed(baked), draw.object_id)
                .packed;
        if !add_surf_has_technique(
            catalog,
            MaterialDrawKey::new(key, rank),
            TechType(SUN_SHADOW_CASTER_TECH),
        ) {
            continue;
        }
        let item = catalog_item(
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
                reflection_probe: Some(SurfaceReflectionProbeId(draw.reflection_probe_index)),
                ..Default::default()
            },
            catalog,
        );
        casters.push(AnimatedCaster {
            item,
            bound: draw.caster_bound,
        });
    }
    casters
}

/// A caster reaches a partition unless its own sphere is wholly outside it. A partition that
/// states no planes and a caster that states no sphere each widen the volume instead of
/// dropping a shadow.
fn reaches(bound: Option<XModelCasterBound>, planes: &[[f32; 4]]) -> bool {
    planes.is_empty() || bound.is_none_or(|bound| !bound.outside(planes))
}

/// Merges two lists that are each in draw order; on a tie `base` comes first.
fn merge_sorted(base: Vec<RetainedDrawItem>, extra: &[RetainedDrawItem]) -> Vec<RetainedDrawItem> {
    if extra.is_empty() {
        return base;
    }
    let mut merged = Vec::with_capacity(base.len() + extra.len());
    let (mut a, mut b) = (0, 0);
    while a < base.len() && b < extra.len() {
        if draw_order(&base[a]) <= draw_order(&extra[b]) {
            merged.push(base[a]);
            a += 1;
        } else {
            merged.push(extra[b]);
            b += 1;
        }
    }
    merged.extend_from_slice(&base[a..]);
    merged.extend_from_slice(&extra[b..]);
    merged
}
