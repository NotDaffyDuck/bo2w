//! Frame products: revision counters, packed-segment layout and the
//! per-kind product table.

use std::sync::Arc;

use render_frame::{
    FrameOutputItem, FrameOutputKind, FrameOutputState, FrameOutputsSnapshot, AbsentOutputReason,
    COMPACT_SEGMENT_CLAIMANTS, CompactSegment, CompactSegments, OutputTarget, FocusedFrameSnapshot,
    InputChangeCounters, publish_rows,
};
use render_material::{MaterialEpochId, MethodType};

#[test]
fn bumping_a_counter_touches_only_that_counter() {
    let mut rev = InputChangeCounters::default();
    rev.advance_vertexdata_stamp();
    rev.advance_drawlist_stamp();
    rev.advance_drawlist_stamp();
    rev.advance_acceptance_stamp();
    assert_eq!((rev.vertices, rev.materials, rev.draws, rev.admission), (1, 0, 2, 1));
    rev.bump_materials();
    assert_eq!(rev.materials, 1);
}

#[test]
fn surfaces_move_vertices_and_materials_together() {
    let mut rev = InputChangeCounters::default();
    rev.bump_surfaces();
    assert_eq!((rev.vertices, rev.materials, rev.draws), (1, 1, 0));
}

#[test]
fn a_packed_write_bumps_surfaces_draws_and_the_callers_revision() {
    let mut rev = InputChangeCounters::default();
    let mut packed_revision = u64::MAX;
    rev.note_packed_data_rewritten(&mut packed_revision);
    assert_eq!((rev.vertices, rev.materials, rev.draws), (1, 1, 1));
    assert_eq!(packed_revision, 0);
}

#[test]
fn counters_wrap_instead_of_overflowing() {
    let mut rev = InputChangeCounters { vertices: u64::MAX, ..InputChangeCounters::default() };
    rev.advance_vertexdata_stamp();
    assert_eq!(rev.vertices, 0);
}

#[test]
fn topology_hash_follows_its_inputs() {
    let mut a = InputChangeCounters::default();
    let mut b = InputChangeCounters::default();
    a.stamp_topology(&[1, 2, 3], &[(0, 3)], 4);
    b.stamp_topology(&[1, 2, 3], &[(0, 3)], 4);
    assert_eq!(a.topology, b.topology);
    b.stamp_topology(&[1, 2, 4], &[(0, 3)], 4);
    assert_ne!(a.topology, b.topology);
    b.stamp_topology(&[1, 2, 3], &[(0, 2)], 4);
    assert_ne!(a.topology, b.topology);
    b.stamp_topology(&[1, 2, 3], &[(0, 3)], 5);
    assert_ne!(a.topology, b.topology);
}

#[test]
fn publishing_identical_rows_changes_nothing_and_recycles_the_buffer() {
    let mut published = vec![1, 2, 3];
    let mut rebuilt = Vec::with_capacity(8);
    rebuilt.extend([1, 2, 3]);
    assert!(!publish_rows(&mut published, &mut rebuilt));
    assert_eq!(published, [1, 2, 3]);
    assert!(rebuilt.is_empty());
    assert!(rebuilt.capacity() >= 8);
}

#[test]
fn publishing_different_rows_swaps_them_in() {
    let mut published = vec![1, 2, 3];
    let mut rebuilt = vec![4, 5];
    assert!(publish_rows(&mut published, &mut rebuilt));
    assert_eq!(published, [4, 5]);
    assert!(rebuilt.is_empty());
}

fn owners(shape: &[(u32, u32)]) -> [CompactSegment; COMPACT_SEGMENT_CLAIMANTS] {
    let mut out = [CompactSegment::default(); COMPACT_SEGMENT_CLAIMANTS];
    for (slot, &(start, rows)) in out.iter_mut().zip(shape) {
        *slot = CompactSegment { start, rows, revision: 0 };
    }
    out
}

#[test]
fn the_first_publish_is_a_layout_change() {
    let mut segments = CompactSegments::default();
    assert!(!segments.live);
    segments.publish(owners(&[(0, 4)]));
    assert!(segments.live);
    assert_eq!(segments.bind_layout, 1);
}

#[test]
fn a_republish_with_the_same_extents_keeps_the_layout() {
    let mut segments = CompactSegments::default();
    segments.publish(owners(&[(0, 4), (4, 2)]));
    let mut again = owners(&[(0, 4), (4, 2)]);
    again[0].revision = 9;
    segments.publish(again);
    assert_eq!(segments.bind_layout, 1);
    assert_eq!(segments.spans[0].revision, 9);
}

#[test]
fn moving_a_segment_or_changing_its_rows_changes_the_layout() {
    let mut segments = CompactSegments::default();
    segments.publish(owners(&[(0, 4), (4, 2)]));
    segments.publish(owners(&[(0, 4), (5, 2)]));
    assert_eq!(segments.bind_layout, 2);
    segments.publish(owners(&[(0, 4), (5, 3)]));
    assert_eq!(segments.bind_layout, 3);
}

#[test]
fn forgetting_clears_the_owners_and_moves_the_layout() {
    let mut segments = CompactSegments::default();
    segments.publish(owners(&[(0, 4)]));
    segments.forget();
    assert!(!segments.live);
    assert_eq!(segments.spans, [CompactSegment::default(); COMPACT_SEGMENT_CLAIMANTS]);
    assert_eq!(segments.bind_layout, 2);
    segments.publish(owners(&[(0, 4)]));
    assert_eq!(segments.bind_layout, 3);
}

#[test]
fn every_kind_is_listed_once() {
    use FrameOutputKind::*;
    assert_eq!(
        FrameOutputKind::ALL_KINDS,
        [DepthOnly, FloatDepth, SunDepth, SpotLightShadowPass, Colour, Emissive, RefractionPass, Light, Planar]
    );
}

#[test]
fn an_empty_snapshot_names_why_each_product_is_missing() {
    let snapshot = FrameOutputsSnapshot::empty();
    assert_eq!((snapshot.frame_seq_no, snapshot.level_run_edition), (0, 0));
    assert!(snapshot.focus().is_none());
    assert!(snapshot.level_run_facets.is_empty());
    let cause = |kind| match snapshot.product(kind).state {
        FrameOutputState::Missing(cause) => cause,
        other => panic!("{kind:?} should start missing, found {other:?}"),
    };
    use FrameOutputKind::*;
    assert_eq!(cause(SunDepth), AbsentOutputReason::SunShadowSourceAbsent);
    assert_eq!(cause(SpotLightShadowPass), AbsentOutputReason::ConeShadowSourceAbsent);
    assert_eq!(cause(Planar), AbsentOutputReason::HostDrivenFlatLayer);
    for kind in [DepthOnly, FloatDepth, Colour, Emissive, RefractionPass, Light] {
        assert_eq!(cause(kind), AbsentOutputReason::DrawlistWriterAbsent);
    }
    for kind in FrameOutputKind::ALL_KINDS {
        assert_eq!(snapshot.product(kind).kind, kind);
    }
}

fn blank_product() -> FrameOutputItem {
    FrameOutputsSnapshot::empty().products.swap_remove(0)
}

#[test]
fn only_a_ready_product_has_a_tech_type() {
    let mut product = blank_product();
    assert_eq!(product.shading_technique_kind(), None);
    product.state = FrameOutputState::ResolveReady { target: OutputTarget::FinishedAfterSun };
    assert_eq!(product.shading_technique_kind(), None);
    product.state =
        FrameOutputState::Ready { shading_technique_kind: MethodType(7), target: OutputTarget::MainViewColor };
    assert_eq!(product.shading_technique_kind(), Some(MethodType(7)));
}

#[test]
fn code_texture_bits_stop_at_sixty_four() {
    let mut product = blank_product();
    product.engine_sampler_bitmask = (1 << 3) | (1 << 63);
    assert!(product.uses_engine_texture(3));
    assert!(product.uses_engine_texture(63));
    assert!(!product.uses_engine_texture(4));
    assert!(!product.uses_engine_texture(64));
    assert!(!product.uses_engine_texture(200));
}

#[test]
fn beginning_a_fill_resets_the_product_but_keeps_its_generation() {
    let mut row = blank_product();
    row.revision_stamp = MaterialEpochId(5);
    row.state =
        FrameOutputState::Ready { shading_technique_kind: MethodType(1), target: OutputTarget::SpotLightShadowAtlas };
    row.daylight_near_tally = 3;
    row.sun_pack = Some(Arc::new(Default::default()));
    row.list_fingerprint = 9;
    row.world_geometry_stamp = 8;
    row.per_draw_technique_kinds.push(MethodType(2));
    row.engine_sampler_bitmask = 0xff;
    row.has_engine_mesh = true;
    row.start_fill(AbsentOutputReason::RefractionOrderKeyAbsent);
    assert_eq!(
        row.state,
        FrameOutputState::Missing(AbsentOutputReason::RefractionOrderKeyAbsent)
    );
    assert_eq!(row.revision_stamp, MaterialEpochId(5));
    assert!(row.draw_sequence.is_empty() && row.per_draw_technique_kinds.is_empty());
    assert!(row.spot_light_frame_slots.is_empty() && row.sun_pack.is_none());
    assert_eq!((row.daylight_near_tally, row.list_fingerprint, row.world_geometry_stamp), (0, 0, 0));
    assert_eq!(row.engine_sampler_bitmask, 0);
    assert!(!row.has_engine_mesh);
}

#[test]
fn an_unresolved_focus_has_only_an_owner_and_an_outcome() {
    let focus = FocusedFrameSnapshot::never_found(12, "no model");
    assert_eq!((focus.owner_no, focus.outcome, focus.planned_facets), (12, "no model", 0));
    assert!(focus.model.is_none() && focus.object_tag.is_none());
    assert!(focus.hidden_from_viewer.is_none() && focus.illum_handle.is_none());
}
