//! The sun-shadow caster list.
//!
//! Three systems run in order each frame:
//!
//! 1. [`bake_static_sun_shadow_casters`] culls the static world against both
//!    sun cascades and bakes a caster plan per cascade;
//! 2. [`bake_sun_shadow_casters`] adds the models, merges the two cascades
//!    into one ordered list (near cascade first) and says whether a sun
//!    shadow map is wanted at all;
//! 3. [`execute_sun_product`] collapses that list into the sun-shadow
//!    product and packs it for the renderer, once per cascade.
//!
//! Packing is the expensive step, so it is skipped for as long as the
//! [`SunStamp`] says nothing it depends on has changed.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::ComputeTaskPool;
use render_frame::{FrameProductKind, PackedFrontendLists, ProductTarget, SunShadowClipPlanes};

use super::super::retained::{
    RetainedDrawItem, StaticDrawLane, SunBakeInputs, SunScratch, SunVisibility, bake_partition,
    merge_partitions, pack_draw,
};
use super::super::setup::TechType;
use super::super::sun_shadow::SunShadowStaging;
use super::super::tess::world::WorldDrawGpuPlan;
use super::super::{
    SUN_SHADOW_CASTER_TECH, SmodelGpuPlan, SunShadowCasterPlan, SunShadowMapPresent, XModelDrawPlan,
};
use super::bank::SUN_MISSING;
use super::slot::{Build, ListSlot};
use super::stamps::{SunStamp, caster_membership};
use super::tech::TechRule;
use super::{FrameAssemblyInputs, MaterialFrameInputs, MaterialGeneration};
use crate::assemble::pack::PackDraw;
use crate::prepare::scene::cull::add_world_surfaces_frustum_only;
use crate::prepare::scene::smodel_geom_cache::{LodRampDvar, PretessDvar, WorldStaticModelCache};
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::world::WorldScene;

/// What the static bake hands to the model bake: one caster plan per cascade.
#[derive(Resource, Default)]
pub(crate) struct StaticSunCasters {
    partitions: Option<[SunShadowCasterPlan; 2]>,
}

/// The sun-shadow product and what its packed form was made from.
#[derive(Resource, Debug)]
pub struct SunProduct {
    pub(super) slot: ListSlot,
    last: Option<SunStamp>,
    last_packed: Option<Arc<[PackedFrontendLists; 2]>>,
}

impl Default for SunProduct {
    fn default() -> Self {
        Self {
            slot: ListSlot::new(FrameProductKind::SunShadow),
            last: None,
            last_packed: None,
        }
    }
}

/// Clears a visibility buffer to `len` zero bytes without reallocating when
/// it already has that length.
fn zero_filled(buf: &mut Vec<u8>, len: usize) {
    if buf.len() == len {
        buf.fill(0);
    } else {
        buf.clear();
        buf.resize(len, 0);
    }
}

pub(crate) fn bake_static_sun_shadow_casters(
    generation: Res<MaterialGeneration>,
    frame: Res<MaterialFrameInputs>,
    scene: Option<Res<WorldScene>>,
    world_plan: Option<Res<WorldDrawGpuPlan>>,
    smodel: (
        Option<Res<SmodelGpuPlan>>,
        Res<LodRampDvar>,
        Option<Res<WorldStaticModelCache>>,
        Res<PretessDvar>,
        Option<Res<WorldSmodelLighting>>,
    ),
    mut staged: ResMut<StaticSunCasters>,
    mut state: Local<(SunShadowStaging, [SunScratch; 2])>,
) {
    let _span = perf::Span::HostStaticSunMs.enter();
    staged.partitions = None;
    let (smodel_plan, lod_ramp, geom_cache, pretess, lighting) = smodel;
    let (Some(sun), Some(scene), Some(world_plan), Some(smodel_plan)) = (
        frame.sun_shadow,
        scene.as_deref(),
        world_plan.as_deref(),
        smodel_plan.as_deref(),
    ) else {
        return;
    };
    let Some(cull) = scene.cull.as_ref() else {
        return;
    };

    let (staging, scratch) = &mut *state;
    let SunShadowStaging {
        surface_vis,
        smodel_vis,
        frustum_draw_msb,
        draw_cell_n,
    } = staging;
    let surface_n = cull.surface_materials.len();
    let smodel_n = cull.smodel_vis.len().max(cull.dpvs.smodel_bounds.len());
    surface_vis.iter_mut().for_each(|v| zero_filled(v, surface_n));
    smodel_vis.iter_mut().for_each(|v| zero_filled(v, smodel_n));
    let [near_surfaces, far_surfaces] = surface_vis;
    let [near_smodels, far_smodels] = smodel_vis;
    let [near_msb, far_msb] = frustum_draw_msb;

    let eye = frame
        .view_from_world
        .map(|view| view.inverse().transform_point3(Vec3::ZERO));
    let bake_inputs = SunBakeInputs {
        cull,
        world_plan,
        smodel_plan,
        catalog: &generation.catalog,
        cache: geom_cache.as_deref(),
        lighting: lighting.as_deref(),
        pretess_on: pretess.enabled,
        eye,
        ramp: lod_ramp.args(),
    };
    let cell_vis = cull.cell_vis.as_slice();

    // One cascade: cull the world against its planes, then bake its casters.
    let bake_cascade = |planes: &SunShadowClipPlanes,
                        surfaces: &mut Vec<u8>,
                        smodels: &mut Vec<u8>,
                        msb: &mut Vec<u32>,
                        scratch: &mut SunScratch| {
        let (_, cells) = add_world_surfaces_frustum_only(
            &cull.dpvs,
            planes.as_slice(),
            cell_vis,
            cull.cell_vis_all,
            surfaces,
            smodels,
            msb,
        );
        let vis = SunVisibility {
            surfaces: surfaces.as_slice(),
            smodels: smodels.as_slice(),
        };
        (bake_partition(bake_inputs, vis, scratch), cells)
    };

    // The cascades share nothing mutable, so they run side by side; only the
    // merge afterwards needs both.
    let [near_scratch, far_scratch] = scratch;
    let [(near, near_cells), (far, far_cells)] = ComputeTaskPool::get()
        .scope(|scope| {
            let bake = &bake_cascade;
            let near_planes = &sun.partitions[0].clip_planes;
            let far_planes = &sun.partitions[1].clip_planes;
            scope.spawn(async move {
                bake(near_planes, near_surfaces, near_smodels, near_msb, near_scratch)
            });
            scope.spawn(async move {
                bake(far_planes, far_surfaces, far_smodels, far_msb, far_scratch)
            });
        })
        .try_into()
        .expect("two sun cascades");
    *draw_cell_n = near_cells.max(far_cells);
    staged.partitions = Some([near, far]);
}

pub(crate) fn bake_sun_shadow_casters(
    inputs: Res<FrameAssemblyInputs>,
    generation: Res<MaterialGeneration>,
    frame: Res<MaterialFrameInputs>,
    xmodel_plan: Option<Res<XModelDrawPlan>>,
    mut staged: ResMut<StaticSunCasters>,
    mut casters: ResMut<SunShadowCasterPlan>,
    mut present: ResMut<SunShadowMapPresent>,
) {
    present.0 = false;
    let Some([near, far]) = staged.partitions.take() else {
        return;
    };
    let Some(sun) = frame.sun_shadow else {
        return;
    };
    let catalog_changed = casters.generation_id != inputs.catalog_generation;

    *casters = merge_partitions(
        near,
        far,
        xmodel_plan.as_deref(),
        &generation.catalog,
        [
            sun.partitions[0].clip_planes.as_slice(),
            sun.partitions[1].clip_planes.as_slice(),
        ],
    );

    if catalog_changed {
        diag::info!(
            World,
            "sun-shadow casters: world {}/{} smodel {}/{} drawn={} near={}",
            casters.world_eligible,
            casters.world_missing_key,
            casters.smodel_eligible,
            casters.smodel_missing_key,
            casters.items.len(),
            casters.sun_near_n,
        );
    }
    present.0 = !casters.items.is_empty();
}

/// Packs the two cascades of `rows` (the first `near_n` rows are the near
/// one) on separate tasks.
fn pack_cascades<T: Send + 'static>(
    rows: &[RetainedDrawItem],
    near_n: usize,
    pack: impl Fn(&[PackDraw]) -> T + Sync,
) -> [T; 2] {
    let (near, far) = rows.split_at(near_n.min(rows.len()));
    let run = |part: &[RetainedDrawItem]| {
        let draws: Vec<PackDraw> = part.iter().map(pack_draw).collect();
        pack(&draws)
    };
    ComputeTaskPool::get()
        .scope(|scope| {
            scope.spawn(async { run(near) });
            scope.spawn(async { run(far) });
        })
        .try_into()
        .unwrap_or_else(|_| unreachable!("two packing tasks"))
}

pub(crate) fn execute_sun_product(
    inputs: Res<FrameAssemblyInputs>,
    generation: Res<MaterialGeneration>,
    frame: Res<MaterialFrameInputs>,
    retained: Res<StaticDrawLane>,
    world_plan: Option<Res<WorldDrawGpuPlan>>,
    xmodel_plan: Option<Res<XModelDrawPlan>>,
    smodel_plan: Option<Res<SmodelGpuPlan>>,
    present: Res<SunShadowMapPresent>,
    mut casters: ResMut<SunShadowCasterPlan>,
    mut owner: ResMut<SunProduct>,
) {
    let SunProduct {
        slot,
        last,
        last_packed,
    } = owner.as_mut();
    slot.begin(SUN_MISSING);
    if !present.0 || casters.items.is_empty() {
        return;
    }
    let (Some(_), Some(world_plan), Some(smodel_plan)) = (
        frame.sun_shadow,
        world_plan.as_deref(),
        smodel_plan.as_deref(),
    ) else {
        return;
    };

    let xmodel_ranges = xmodel_plan.as_ref().map_or(&[][..], |plan| plan.range_rows());
    let world_ranges = world_plan.surface_ranges();
    let world_verts = world_plan.decoded_vertices().len() as u32;
    let smodel_ranges = smodel_plan.surface_ranges();

    slot.product.ordered_draws = std::mem::take(&mut casters.items);
    slot.product.sun_near_n = casters.sun_near_n;
    let stamp = SunStamp {
        membership: caster_membership(&slot.product.ordered_draws, slot.product.sun_near_n),
        catalog: inputs.catalog_generation.0,
        world_generation: inputs.world_generation.0,
        world_verts,
        world_range_n: world_ranges.len(),
        smodel_range_n: smodel_ranges.len(),
        xmodel_topology: xmodel_plan.as_ref().map_or(0, |plan| plan.topology_revision),
        xmodel_range_n: xmodel_ranges.len(),
    };
    let build = Build {
        generation_id: retained.generation_id,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let tech = TechType(SUN_SHADOW_CASTER_TECH);
    let target = ProductTarget::SunShadowFallbackAtlas;

    // Same casters as last frame: only their payload may differ, and the
    // packed form is still good.
    if *last == Some(stamp)
        && let Some(packed) = last_packed.clone()
        && slot.memo.replay(&mut slot.product)
    {
        slot.ready(&build, tech, target);
        slot.product.sun_packed = Some(packed);
        return;
    }

    slot.collapse(&build, &TechRule::Single(tech), tech, target, false);

    let world_run_surfs = retained.live_for(inputs.world_generation).4;
    let packed = Arc::new(pack_cascades(
        &slot.product.ordered_draws,
        slot.product.sun_near_n,
        |draws| {
            crate::pack_sun_shadow_frontend(
                draws,
                world_run_surfs,
                world_ranges,
                world_verts,
                smodel_ranges,
                xmodel_ranges,
            )
        },
    ));
    slot.product.sun_packed = Some(Arc::clone(&packed));
    *last_packed = Some(packed);
    *last = Some(stamp);
}

#[cfg(test)]
mod tests {
    use super::super::lanes::tests::row;
    use super::*;

    #[test]
    fn a_buffer_of_the_right_length_is_zeroed_in_place() {
        let mut buf = vec![7u8; 4];
        let ptr = buf.as_ptr();
        zero_filled(&mut buf, 4);
        assert_eq!(buf, [0; 4]);
        assert_eq!(buf.as_ptr(), ptr);
    }

    #[test]
    fn a_buffer_of_another_length_is_resized_to_zeros() {
        let mut buf = vec![7u8; 4];
        zero_filled(&mut buf, 6);
        assert_eq!(buf, [0; 6]);
        zero_filled(&mut buf, 2);
        assert_eq!(buf, [0; 2]);
    }

    fn pool() {
        ComputeTaskPool::get_or_init(Default::default);
    }

    #[test]
    fn the_cascades_are_packed_apart() {
        pool();
        let rows: Vec<_> = (0..7).map(|i| row(i, 1)).collect();
        assert_eq!(pack_cascades(&rows, 3, |d| d.len()), [3, 4]);
    }

    #[test]
    fn a_near_count_outside_the_list_leaves_the_far_cascade_empty() {
        pool();
        let rows: Vec<_> = (0..5).map(|i| row(i, 1)).collect();
        assert_eq!(pack_cascades(&rows, 5, |d| d.len()), [5, 0]);
        assert_eq!(pack_cascades(&rows, 50, |d| d.len()), [5, 0]);
        assert_eq!(pack_cascades(&rows, 0, |d| d.len()), [0, 5]);
        assert_eq!(pack_cascades(&[], 0, |d| d.len()), [0, 0]);
    }

    #[test]
    fn each_cascade_gets_its_own_rows_in_order() {
        pool();
        let rows: Vec<_> = (0..6).map(|i| row(10 + i, 1)).collect();
        let keys = pack_cascades(&rows, 2, |d| d.iter().map(|p| p.key).collect::<Vec<_>>());
        assert_eq!(keys[0], [10, 11]);
        assert_eq!(keys[1], [12, 13, 14, 15]);
    }
}
