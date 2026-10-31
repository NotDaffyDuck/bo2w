//! The sun-shadow caster list.
//!
//! Three systems run in order each frame:
//!
//! 1. [`bake_static_sun_shadow_casters`] culls the static world against both
//!    sun cascades and bakes a caster plan per cascade;
//! 2. [`bake_daylight_shadow_casters`] adds the models, merges the two cascades
//!    into one ordered list (near cascade first) and says whether a sun
//!    shadow map is wanted at all;
//! 3. [`execute_daylight_output`] collapses that list into the sun-shadow
//!    product and packs it for the renderer, once per cascade.
//!
//! Packing is the expensive step, so it is skipped for as long as the
//! [`SunStamp`] says nothing it depends on has changed.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::ComputeTaskPool;
use render_frame::{FrameOutputKind, CompactFrontendLists, OutputTarget, SunShadowClipPlanes};

use super::super::kept_lanes::{
    HeldDrawItem, StaticDrawTrack, SunBakeInputs, SunScratch, SunVisibility, bake_partition,
    merge_partitions, pack_draw,
};
use super::super::material_setup::MethodType;
use super::super::daylight_shadow::DaylightShadowStaging;
use super::super::mesh_build::world::WorldGpuSubmission;
use super::super::{
    DAYLIGHT_CASTER_TECH, PropGpuPlan, DaylightShadowCasterPlan, DaylightShadowMapPresent, FigureDrawPlan,
};
use super::bank::SUN_MISSING;
use super::seat::{Build, ListSlot};
use super::stamps::{SunStamp, caster_membership};
use super::technique_ix::TechRule;
use crate::assemble::draw_items::frame_outputs::{FrameBuildFeed, MaterialFrameFeed, MaterialEpoch};
use crate::assemble::packed_word::PackableDrawCall;
use crate::prepare::scene::cull::add_level_facets_view_volume_only;
use crate::prepare::scene::prop_geom_memo::DetailRampTunable;
use crate::prepare::scene::prop_geom_memo::PrebuiltTunable;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::world::LevelStage;

/// What the static bake hands to the model bake: one caster plan per cascade.
#[derive(Resource, Default)]
pub(crate) struct StaticSunCasters {
    partitions: Option<[DaylightShadowCasterPlan; 2]>,
}

/// The sun-shadow product and what its packed form was made from.
#[derive(Resource, Debug)]
pub struct DaylightOutput {
    pub(super) slot: ListSlot,
    last: Option<SunStamp>,
    last_compact: Option<Arc<[CompactFrontendLists; 2]>>,
}

impl Default for DaylightOutput {
    fn default() -> Self {
        Self {
            slot: ListSlot::new(FrameOutputKind::SunDepth),
            last: None,
            last_compact: None,
        }
    }
}

/// Clears a visibility buffer to `wanted` zero bytes without reallocating when
/// it already has that length.
fn zero_filled(bytes_out: &mut Vec<u8>, wanted: usize) {
    if bytes_out.len() == wanted {
        bytes_out.fill(0);
    } else {
        bytes_out.clear();
        bytes_out.resize(wanted, 0);
    }
}

pub(crate) fn bake_static_sun_shadow_casters(
    generation: Res<MaterialEpoch>,
    feed: Res<MaterialFrameFeed>,
    stage: Option<Res<LevelStage>>,
    level_plan: Option<Res<WorldGpuSubmission>>,
    smodel: (
        Option<Res<PropGpuPlan>>,
        Res<DetailRampTunable>,
        Option<Res<LevelPropCache>>,
        Res<PrebuiltTunable>,
        Option<Res<LevelPropIllum>>,
    ),
    mut staged: ResMut<StaticSunCasters>,
    mut state: Local<(DaylightShadowStaging, [SunScratch; 2])>,
) {
    let _span = perf::Span::HostStaticSunMs.enter();
    staged.partitions = None;
    let (prop_plan, detail_ramp, geom_cache, pretess, lighting) = smodel;
    let (Some(sun), Some(stage), Some(level_plan), Some(prop_plan)) = (
        feed.sun_shadow,
        stage.as_deref(),
        level_plan.as_deref(),
        prop_plan.as_deref(),
    ) else {
        return;
    };
    let Some(cull) = stage.culling.as_ref() else {
        return;
    };

    let (staging, scratch) = &mut *state;
    let DaylightShadowStaging {
        facet_vis,
        prop_vis,
        view_volume_draw_msb,
        draw_room_tally,
    } = staging;
    let surface_n = cull.facet_materials.len();
    let smodel_n = cull.prop_vis.len().max(cull.dpvs.prop_bounds.len());
    facet_vis.iter_mut().for_each(|v| zero_filled(v, surface_n));
    prop_vis.iter_mut().for_each(|v| zero_filled(v, smodel_n));
    let [near_surfaces, far_surfaces] = facet_vis;
    let [near_smodels, far_smodels] = prop_vis;
    let [near_msb, far_msb] = view_volume_draw_msb;

    let eye = feed
        .world_to_view
        .map(|view| view.inverse().transform_point3(Vec3::ZERO));
    let bake_inputs = SunBakeInputs {
        cull,
        level_plan,
        prop_plan,
        catalog: &generation.catalog,
        cache: geom_cache.as_deref(),
        lighting: lighting.as_deref(),
        pretess_on: pretess.enabled,
        eye,
        ramp: detail_ramp.args(),
    };
    let room_vis = cull.room_vis.as_slice();

    // One cascade: cull the world against its planes, then bake its casters.
    let bake_cascade = |planes: &SunShadowClipPlanes,
                        surfaces: &mut Vec<u8>,
                        fixed_props: &mut Vec<u8>,
                        msb: &mut Vec<u32>,
                        scratch: &mut SunScratch| {
        let (_, cells) = add_level_facets_view_volume_only(
            &cull.dpvs,
            planes.as_slice(),
            room_vis,
            cull.room_vis_all,
            surfaces,
            fixed_props,
            msb,
        );
        let vis = SunVisibility {
            surfaces: surfaces.as_slice(),
            fixed_props: fixed_props.as_slice(),
        };
        (bake_partition(bake_inputs, vis, scratch), cells)
    };

    // The cascades share nothing mutable, so they run side by side; only the
    // merge afterwards needs both.
    let [near_scratch, far_scratch] = scratch;
    let cascades: Vec<_> = ComputeTaskPool::get()
        .scope(|sc| {
            let bake = &bake_cascade;
            let near_planes = &sun.partitions[0].frustum_clip_planes_for;
            let far_planes = &sun.partitions[1].frustum_clip_planes_for;
            sc.spawn(async move {
                bake(near_planes, near_surfaces, near_smodels, near_msb, near_scratch)
            });
            sc.spawn(async move {
                bake(far_planes, far_surfaces, far_smodels, far_msb, far_scratch)
            });
        });
    let [(near, near_cells), (far, far_cells)] = <[_; 2]>::try_from(cascades).expect("two sun cascades");
    *draw_room_tally = near_cells.max(far_cells);
    staged.partitions = Some([near, far]);
}

pub(crate) fn bake_daylight_shadow_casters(
    inputs: Res<FrameBuildFeed>,
    epoch: Res<MaterialEpoch>,
    frame: Res<MaterialFrameFeed>,
    figure_plan: Option<Res<FigureDrawPlan>>,
    mut staged: ResMut<StaticSunCasters>,
    mut casters: ResMut<DaylightShadowCasterPlan>,
    mut shown_frame: ResMut<DaylightShadowMapPresent>,
) {
    shown_frame.0 = false;
    let Some([near, far]) = staged.partitions.take() else {
        return;
    };
    let Some(sun) = frame.sun_shadow else {
        return;
    };
    let catalog_changed = casters.revision_stamp != inputs.ledger_generation;

    *casters = merge_partitions(
        near,
        far,
        figure_plan.as_deref(),
        &epoch.catalog,
        [
            sun.partitions[0].frustum_clip_planes_for.as_slice(),
            sun.partitions[1].frustum_clip_planes_for.as_slice(),
        ],
    );

    if catalog_changed {
        diag::info!(
            World,
            "sun-shadow casters: world {}/{} smodel {}/{} drawn={} near={}",
            casters.level_eligible,
            casters.level_missing_key,
            casters.prop_eligible,
            casters.prop_missing_key,
            casters.item_list.len(),
            casters.daylight_near_tally,
        );
    }
    shown_frame.0 = !casters.item_list.is_empty();
}

/// Packs the two cascades of `rows` (the first `near_n` rows are the near
/// one) on separate tasks.
fn pack_cascades<T: Send + 'static>(
    rows: &[HeldDrawItem],
    near_n: usize,
    pack: impl Fn(&[PackableDrawCall]) -> T + Sync,
) -> [T; 2] {
    let (near, far) = rows.split_at(near_n.min(rows.len()));
    let run = |part: &[HeldDrawItem]| {
        let draws: Vec<PackableDrawCall> = part.iter().map(pack_draw).collect();
        pack(&draws)
    };
    let pool = ComputeTaskPool::get();
    let packed = pool.scope(|tasks| {
        tasks.spawn(async { run(near) });
        tasks.spawn(async { run(far) });
    });
    <[T; 2]>::try_from(packed).unwrap_or_else(|_| unreachable!("two packing tasks"))
}

pub(crate) fn execute_daylight_output(
    inputs: Res<FrameBuildFeed>,
    generation: Res<MaterialEpoch>,
    feed: Res<MaterialFrameFeed>,
    retained: Res<StaticDrawTrack>,
    level_plan: Option<Res<WorldGpuSubmission>>,
    figure_plan: Option<Res<FigureDrawPlan>>,
    prop_plan: Option<Res<PropGpuPlan>>,
    present: Res<DaylightShadowMapPresent>,
    mut casters: ResMut<DaylightShadowCasterPlan>,
    mut holder: ResMut<DaylightOutput>,
) {
    let DaylightOutput {
        slot,
        last,
        last_compact,
    } = holder.as_mut();
    slot.begin(SUN_MISSING);
    if !present.0 || casters.item_list.is_empty() {
        return;
    }
    let (Some(level_plan), Some(prop_plan), Some(_)) = (
        level_plan.as_deref(),
        prop_plan.as_deref(),
        feed.sun_shadow,
    ) else {
        return;
    };

    let figure_ranges = figure_plan.as_ref().map_or(&[][..], |plan| plan.span_rows());
    let level_ranges = level_plan.facet_ranges();
    let level_vertices = level_plan.decoded_vertices().len() as u32;
    let prop_ranges = prop_plan.facet_ranges();

    slot.product.draw_sequence = std::mem::take(&mut casters.item_list);
    slot.product.daylight_near_tally = casters.daylight_near_tally;
    let stamp = SunStamp {
        membership: caster_membership(&slot.product.draw_sequence, slot.product.daylight_near_tally),
        catalog: inputs.ledger_generation.0,
        level_generation: inputs.level_generation.0,
        level_vertices,
        world_range_n: level_ranges.len(),
        smodel_range_n: prop_ranges.len(),
        xmodel_topology: figure_plan.as_ref().map_or(0, |plan| plan.topology_edition),
        xmodel_range_n: figure_ranges.len(),
    };
    let build = Build {
        revision_stamp: retained.revision_stamp,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let tech = MethodType(DAYLIGHT_CASTER_TECH);
    let target = OutputTarget::SunShadowSubstituteAtlas;

    // Same casters as last feed: only their payload may differ, and the
    // packed form is still good.
    if *last == Some(stamp)
        && let Some(packed) = last_compact.clone()
        && slot.memo.replay(&mut slot.product)
    {
        slot.ready(&build, tech, target);
        slot.product.sun_pack = Some(packed);
        return;
    }

    slot.collapse(&build, &TechRule::Single(tech), tech, target, false);

    let level_run_facets = retained.alive_for(inputs.level_generation).4;
    let packed = Arc::new(pack_cascades(
        &slot.product.draw_sequence,
        slot.product.daylight_near_tally,
        |draws| {
            crate::pack_daylight_shadow_frontend(
                draws,
                level_run_facets,
                level_ranges,
                level_vertices,
                prop_ranges,
                figure_ranges,
            )
        },
    ));
    slot.product.sun_pack = Some(Arc::clone(&packed));
    *last_compact = Some(packed);
    *last = Some(stamp);
}

#[cfg(test)]
mod tests {
    use super::super::lane_set::tests::row;
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
