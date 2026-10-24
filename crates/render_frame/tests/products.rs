//! Frame products: revision counters, packed-segment layout and the
//! per-kind product table.

use std::sync::Arc;

use render_frame::{
    FrameProduct, FrameProductKind, FrameProductStatus, FrameProductsSnapshot, MissingProductCause,
    PACKED_SEGMENT_OWNERS, PackedSegment, PackedSegments, ProductTarget, RenderFocusFrame,
    SourceRevisions, publish_rows,
};
use render_material::{MaterialGenerationId, TechType};

#[test]
fn bumping_a_counter_touches_only_that_counter() {
    let mut rev = SourceRevisions::default();
    rev.bump_vertices();
    rev.bump_draws();
    rev.bump_draws();
    rev.bump_admission();
    assert_eq!((rev.vertices, rev.materials, rev.draws, rev.admission), (1, 0, 2, 1));
    rev.bump_materials();
    assert_eq!(rev.materials, 1);
}

#[test]
fn surfaces_move_vertices_and_materials_together() {
    let mut rev = SourceRevisions::default();
    rev.bump_surfaces();
    assert_eq!((rev.vertices, rev.materials, rev.draws), (1, 1, 0));
}

#[test]
fn a_packed_write_bumps_surfaces_draws_and_the_callers_revision() {
    let mut rev = SourceRevisions::default();
    let mut packed_revision = u64::MAX;
    rev.bump_packed_write(&mut packed_revision);
    assert_eq!((rev.vertices, rev.materials, rev.draws), (1, 1, 1));
    assert_eq!(packed_revision, 0);
}

#[test]
fn counters_wrap_instead_of_overflowing() {
    let mut rev = SourceRevisions { vertices: u64::MAX, ..SourceRevisions::default() };
    rev.bump_vertices();
    assert_eq!(rev.vertices, 0);
}

#[test]
fn topology_hash_follows_its_inputs() {
    let mut a = SourceRevisions::default();
    let mut b = SourceRevisions::default();
    a.set_topology_from(&[1, 2, 3], &[(0, 3)], 4);
    b.set_topology_from(&[1, 2, 3], &[(0, 3)], 4);
    assert_eq!(a.topology, b.topology);
    b.set_topology_from(&[1, 2, 4], &[(0, 3)], 4);
    assert_ne!(a.topology, b.topology);
    b.set_topology_from(&[1, 2, 3], &[(0, 2)], 4);
    assert_ne!(a.topology, b.topology);
    b.set_topology_from(&[1, 2, 3], &[(0, 3)], 5);
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

fn owners(shape: &[(u32, u32)]) -> [PackedSegment; PACKED_SEGMENT_OWNERS] {
    let mut out = [PackedSegment::default(); PACKED_SEGMENT_OWNERS];
    for (slot, &(start, rows)) in out.iter_mut().zip(shape) {
        *slot = PackedSegment { start, rows, revision: 0 };
    }
    out
}

#[test]
fn the_first_publish_is_a_layout_change() {
    let mut segments = PackedSegments::default();
    assert!(!segments.live);
    segments.publish(owners(&[(0, 4)]));
    assert!(segments.live);
    assert_eq!(segments.layout, 1);
}

#[test]
fn a_republish_with_the_same_extents_keeps_the_layout() {
    let mut segments = PackedSegments::default();
    segments.publish(owners(&[(0, 4), (4, 2)]));
    let mut again = owners(&[(0, 4), (4, 2)]);
    again[0].revision = 9;
    segments.publish(again);
    assert_eq!(segments.layout, 1);
    assert_eq!(segments.owners[0].revision, 9);
}

#[test]
fn moving_a_segment_or_changing_its_rows_changes_the_layout() {
    let mut segments = PackedSegments::default();
    segments.publish(owners(&[(0, 4), (4, 2)]));
    segments.publish(owners(&[(0, 4), (5, 2)]));
    assert_eq!(segments.layout, 2);
    segments.publish(owners(&[(0, 4), (5, 3)]));
    assert_eq!(segments.layout, 3);
}

#[test]
fn forgetting_clears_the_owners_and_moves_the_layout() {
    let mut segments = PackedSegments::default();
    segments.publish(owners(&[(0, 4)]));
    segments.forget();
    assert!(!segments.live);
    assert_eq!(segments.owners, [PackedSegment::default(); PACKED_SEGMENT_OWNERS]);
    assert_eq!(segments.layout, 2);
    segments.publish(owners(&[(0, 4)]));
    assert_eq!(segments.layout, 3);
}

#[test]
fn every_kind_is_listed_once() {
    use FrameProductKind::*;
    assert_eq!(
        FrameProductKind::ALL,
        [Depth, FloatZ, SunShadow, SpotShadow, Colour, Emissive, Distortion, Light, TwoD]
    );
}

#[test]
fn an_empty_snapshot_names_why_each_product_is_missing() {
    let snapshot = FrameProductsSnapshot::empty();
    assert_eq!((snapshot.frame_id, snapshot.world_run_revision), (0, 0));
    assert!(snapshot.focus().is_none());
    assert!(snapshot.world_run_surfs.is_empty());
    let cause = |kind| match snapshot.product(kind).status {
        FrameProductStatus::Missing(cause) => cause,
        other => panic!("{kind:?} should start missing, found {other:?}"),
    };
    use FrameProductKind::*;
    assert_eq!(cause(SunShadow), MissingProductCause::MissingSunShadowProducer);
    assert_eq!(cause(SpotShadow), MissingProductCause::MissingSpotShadowProducer);
    assert_eq!(cause(TwoD), MissingProductCause::HostOwnedTwoD);
    for kind in [Depth, FloatZ, Colour, Emissive, Distortion, Light] {
        assert_eq!(cause(kind), MissingProductCause::MissingFrontendEmitter);
    }
    for kind in FrameProductKind::ALL {
        assert_eq!(snapshot.product(kind).kind, kind);
    }
}

fn blank_product() -> FrameProduct {
    FrameProductsSnapshot::empty().products.swap_remove(0)
}

#[test]
fn only_a_ready_product_has_a_tech_type() {
    let mut product = blank_product();
    assert_eq!(product.tech_type(), None);
    product.status = FrameProductStatus::ResolveReady { target: ProductTarget::ResolvedPostSun };
    assert_eq!(product.tech_type(), None);
    product.status =
        FrameProductStatus::Ready { tech_type: TechType(7), target: ProductTarget::Core3dViewColour };
    assert_eq!(product.tech_type(), Some(TechType(7)));
}

#[test]
fn code_texture_bits_stop_at_sixty_four() {
    let mut product = blank_product();
    product.code_sampler_mask = (1 << 3) | (1 << 63);
    assert!(product.binds_code_texture(3));
    assert!(product.binds_code_texture(63));
    assert!(!product.binds_code_texture(4));
    assert!(!product.binds_code_texture(64));
    assert!(!product.binds_code_texture(200));
}

#[test]
fn beginning_a_fill_resets_the_product_but_keeps_its_generation() {
    let mut product = blank_product();
    product.generation_id = MaterialGenerationId(5);
    product.status =
        FrameProductStatus::Ready { tech_type: TechType(1), target: ProductTarget::SpotShadowMaps };
    product.sun_near_n = 3;
    product.sun_packed = Some(Arc::new(Default::default()));
    product.list_digest = 9;
    product.world_pretess_id = 8;
    product.draw_tech.push(TechType(2));
    product.code_sampler_mask = 0xff;
    product.has_codemesh = true;
    product.begin_fill(MissingProductCause::MissingDistortionSortKey);
    assert_eq!(
        product.status,
        FrameProductStatus::Missing(MissingProductCause::MissingDistortionSortKey)
    );
    assert_eq!(product.generation_id, MaterialGenerationId(5));
    assert!(product.ordered_draws.is_empty() && product.draw_tech.is_empty());
    assert!(product.spot_slots.is_empty() && product.sun_packed.is_none());
    assert_eq!((product.sun_near_n, product.list_digest, product.world_pretess_id), (0, 0, 0));
    assert_eq!(product.code_sampler_mask, 0);
    assert!(!product.has_codemesh);
}

#[test]
fn an_unresolved_focus_has_only_an_owner_and_an_outcome() {
    let focus = RenderFocusFrame::unresolved(12, "no model");
    assert_eq!((focus.owner_id, focus.outcome, focus.planned_surfaces), (12, "no model", 0));
    assert!(focus.model.is_none() && focus.object_id.is_none());
    assert!(focus.camera_hidden.is_none() && focus.lighting_handle.is_none());
}
