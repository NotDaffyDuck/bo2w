//! Checks for the present module: catalog lookups, the element-table cache, the lighting
//! tally, createfx boot, queue settling and the empty-host paths.

use std::sync::Arc;

use asset_audio::SpawnSingleShotEffect;
use asset_core::{AssetDependencyLink, AssetCategoryScope};
use asset_game::{FxDefinitions, FxName, SelfContainedEffectDef, SelfContainedEffectElement, SelfContainedTrailDef};
use fx::{
    NO_RECIPE_LISTING, EffectHole, BakedShadeSource, EffectSystemHost, QueuedDecalBirth,
    QueuedSpawnerBirth,
};
use fx_t6::runtime::{EmitterRecipeLens, ParticleRecipeLens, apply_lighting_frac_bgra};

use super::catalog::elem_table;
use super::draw::{light_bgra, light_counted};
use super::launch::delayed_msec;
use super::t6::{light_scale, swap_red_blue};
use super::tick::worker_count;
use super::*;

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn blank_view() -> ParticleRecipeLens {
    ParticleRecipeLens {
        flags: 0,
        birth_corner_a: 0,
        birth_corner_b: 0,
        birth_span_floor: 0.0,
        birth_span_swing: 0.0,
        appear_blend_reach: [0.0; 2],
        vanish_blend_reach: [0.0; 2],
        birth_view_reject_reach: 0.0,
        birth_wait_floor_ms: 0,
        birth_wait_swing_ms: 0,
        lifespan_window_floor: 0,
        lifespan_window_swing: 0,
        birth_point: [[0.0; 2]; 3],
        birth_ring_reach_floor: 0.0,
        birth_ring_reach_swing: 0.0,
        birth_lift_floor: 0.0,
        birth_lift_swing: 0.0,
        birth_orientation: [[0.0; 2]; 3],
        twirl_rate: [[0.0; 2]; 3],
        birth_spin_angle: [0.0; 2],
        fall_pull_floor: 0.0,
        fall_pull_swing: 0.0,
        mirror_strength: [0.0; 2],
        bounce_box_bottom: [0.0; 3],
        bounce_box_top: [0.0; 3],
        shape_kind: 0,
        look_variant_total: 0,
        speed_keyframe_total: 0,
        look_keyframe_total: 0,
        shading_blend: 0,
        collide_with_pickups: 0,
        line: false,
        paint_rank: 0,
        emission_spacing: [0.0; 2],
        emission_spacing_spread: [0.0; 2],
    }
}

fn elem_with(view: ParticleRecipeLens) -> SelfContainedEffectElement {
    SelfContainedEffectElement {
        view,
        atlas: [0; 8],
        speed_readings: Vec::new(),
        vel_graph_local: Vec::new(),
        vel_graph_world: Vec::new(),
        visibility_readings: Vec::new(),
        visuals: Vec::new(),
        effect_upon_strike: AssetDependencyLink::NothingThere,
        effect_upon_demise: AssetDependencyLink::NothingThere,
        effect_spawned: AssetDependencyLink::NothingThere,
        on_impact_name: None,
        on_death_name: None,
        emitted_name: None,
        trail_kind: None,
        cloud_density: None,
    }
}

fn effect_named(name: &str, elems: Vec<SelfContainedEffectElement>) -> SelfContainedEffectDef {
    SelfContainedEffectDef {
        namespace: AssetCategoryScope::T6,
        name: name.to_owned(),
        view: EmitterRecipeLens {
            flags: 0,
            loop_lifetime_millis: 0,
            repeat_total: 0,
            single_play_total: 0,
            spawn_total: 0,
        },
        elems,
    }
}

fn catalog_of(effects: Vec<SelfContainedEffectDef>) -> FxDefinitions {
    let mut catalog = FxDefinitions::default();
    for recipe in effects {
        catalog.add_owned_fx(recipe);
    }
    catalog
}

fn runner(parent: &str, ledger_index: u16, definition_slot: u8) -> QueuedSpawnerBirth {
    QueuedSpawnerBirth {
        stain_ent: None,
        source_name: parent.to_owned(),
        ledger_index,
        definition_slot,
        window_open_ms: 0,
        dice_start: 0,
        spot: [0.0; 3],
        orient_rows: IDENTITY,
        turn_deg: None,
    }
}

#[test]
fn lookup_rejects_the_none_index_and_out_of_range() {
    let catalog = catalog_of(vec![effect_named("fx/a", Vec::new())]);
    assert!(recipe_lookup(&catalog, NO_RECIPE_LISTING).is_none());
    assert!(recipe_lookup(&catalog, 5).is_none());
    assert_eq!(recipe_lookup(&catalog, 0).map(|e| e.name.as_str()), Some("fx/a"));
}

#[test]
fn cache_rebuilds_only_when_the_catalog_changes_shape() {
    let two = vec![elem_with(blank_view()), elem_with(blank_view())];
    let mut catalog = catalog_of(vec![effect_named("fx/a", two)]);
    let mut cache = EffectElementInfoMemo::default();
    cache.restock(&catalog);

    // The catalog knows an effect by its address, so always borrow it from the catalog.
    let first = cache.shared_handle_for(&catalog, catalog.recipe_at(0).unwrap());
    assert_eq!(first.len(), 2);
    assert!(Arc::ptr_eq(&first, &cache.shared_handle_for(&catalog, catalog.recipe_at(0).unwrap())));

    cache.restock(&catalog);
    assert!(Arc::ptr_eq(&first, &cache.shared_handle_for(&catalog, catalog.recipe_at(0).unwrap())));

    // A copy that is not the catalog's own entry is built fresh every time.
    let copy = catalog.recipe_at(0).unwrap().clone();
    assert!(!Arc::ptr_eq(&first, &cache.shared_handle_for(&catalog, &copy)));

    catalog.add_owned_fx(effect_named("fx/b", Vec::new()));
    cache.restock(&catalog);
    let rebuilt = cache.shared_handle_for(&catalog, catalog.recipe_at(0).unwrap());
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert_eq!(rebuilt.len(), 2);

    let stranger = effect_named("fx/not_in_catalog", vec![elem_with(blank_view())]);
    assert_eq!(cache.shared_handle_for(&catalog, &stranger).len(), 1);

    cache.clear();
    let fresh = cache.shared_handle_for(&catalog, catalog.recipe_at(0).unwrap());
    assert!(!Arc::ptr_eq(&rebuilt, &fresh));
}

#[test]
fn elem_table_carries_the_runtime_fields() {
    let mut view = blank_view();
    view.shape_kind = 3;
    view.birth_wait_floor_ms = 11;
    view.birth_wait_swing_ms = 12;
    view.lifespan_window_floor = 13;
    view.lifespan_window_swing = 14;
    view.fall_pull_floor = 1.5;
    view.fall_pull_swing = 2.5;
    view.mirror_strength = [0.25, 0.75];
    view.bounce_box_bottom = [-1.0, -2.0, -3.0];
    view.collide_with_pickups = 1;
    view.birth_ring_reach_swing = 6.0;
    view.paint_rank = 9;
    let mut trailed = elem_with(view);
    trailed.trail_kind = Some(SelfContainedTrailDef {
        scroll_period_millis: 0,
        recurrence_distance: 0,
        reciprocal_split_distance: 0.5,
        reciprocal_split_arc_length: 0.25,
        reciprocal_split_duration: 0.125,
        vertex_points: Vec::new(),
        index_run_values: Vec::new(),
    });
    let effect = effect_named("fx/table", vec![trailed, elem_with(blank_view())]);
    let table = elem_table(&effect);

    let t = table[0];
    assert_eq!(t.part_shape, 3);
    assert_eq!((t.wait_floor, t.wait_spread, t.lifespan_floor, t.lifespan_spread), (11, 12, 13, 14));
    assert_eq!((t.fall_pull_floor, t.fall_pull_spread), (1.5, 2.5));
    assert_eq!((t.mirror_floor, t.mirror_spread), (0.25, 0.75));
    assert_eq!(t.bounce_box_bottom, [-1.0, -2.0, -3.0]);
    assert_eq!(t.collide_with_pickups, 1);
    assert_eq!(t.birth_ring_reach_spread, 6.0);
    assert_eq!(t.paint_rank, 9);
    assert_eq!((t.reciprocal_split_distance, t.reciprocal_split_arc_length, t.reciprocal_split_duration), (0.5, 0.25, 0.125));
    assert!(!t.spawns_on_contact && !t.spawns_on_finish && !t.emitted_once);
    assert_eq!(table[1].reciprocal_split_distance, 0.0);
}

#[test]
fn lighting_tally_counts_each_source() {
    let bgra = [40, 80, 120, 200];
    let sample = [10, 200, 90];
    let mut tally = EffectVertexHoles::default();

    let unlit = light_counted(&mut tally, 1, bgra, 0, sample, BakedShadeSource::White);
    assert_eq!(unlit, bgra);
    assert_eq!(tally.shading_blend, 0);

    let missing = light_counted(&mut tally, 7, bgra, 128, sample, BakedShadeSource::Missing);
    assert_eq!(missing, bgra);
    assert_eq!((tally.shading_blend, tally.light_lookup_failures, tally.lit_draw_total), (1, 1, 0));
    assert_eq!(tally.light_fraction_default_slots, vec![7]);

    let lit = apply_lighting_frac_bgra(bgra, sample, 128);
    assert_eq!(light_counted(&mut tally, 2, bgra, 128, sample, BakedShadeSource::White), lit);
    assert_eq!(
        light_counted(&mut tally, 3, bgra, 128, sample, BakedShadeSource::AtPointRgb),
        lit
    );
    assert_eq!((tally.shading_blend, tally.lit_draw_total), (3, 2));
    assert_eq!((tally.white_fallback_total, tally.point_sampled_lit_total), (1, 1));
    assert_eq!(tally.light_fraction_default_slots, vec![7]);

    assert_eq!(light_bgra(bgra, 128, sample, BakedShadeSource::Missing), bgra);
    assert_eq!(light_bgra(bgra, 128, sample, BakedShadeSource::AtPointRgb), lit);
}

#[test]
fn boot_counts_createfx_ids_the_catalog_lacks() {
    let mut host = EffectSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = EffectElementInfoMemo::default();
    let shots = vec![SpawnSingleShotEffect {
        effect_id: "fx/not_there".to_owned(),
        position_in_inches: [0.0; 3],
        rotation_degrees: [0.0; 3],
        start_after: 0.5,
    }];
    let boot = start_map_placed_effects(&mut host, &catalog, &cache, &shots, 1000, None);
    assert_eq!((boot.by_name, boot.kept_count, boot.missing_definition_total), (0, 0, 1));
    assert!(boot.refused.is_empty());
    assert_eq!(host.clock_ms_now, 1000);
}

#[test]
fn createfx_delay_is_whole_milliseconds_and_wraps() {
    assert_eq!(delayed_msec(1000, 0.25), 1250);
    assert_eq!(delayed_msec(1000, 0.0019), 1001);
    assert_eq!(delayed_msec(i32::MAX, 0.001), i32::MIN);
}

#[test]
fn runner_without_a_catalog_entry_is_noted_as_skipped() {
    let mut host = EffectSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = EffectElementInfoMemo::default();
    host.pending_runners.push(runner("fx/parent", NO_RECIPE_LISTING, 3));
    queued::settle(&mut host, &catalog, &cache, None);

    assert!(host.pending_runners.is_empty());
    assert_eq!(host.gaps.hit_total(EffectHole::RunnerChild), 1);
    assert_eq!(host.last_emitter_source.as_deref(), Some("fx/parent"));
    assert_eq!(host.last_emitter_part, Some(3));
    assert!(host.last_emitter_child.is_none());

    host.pending_runners.push(runner("", NO_RECIPE_LISTING, 4));
    queued::settle(&mut host, &catalog, &cache, None);
    assert_eq!(host.gaps.hit_total(EffectHole::RunnerChild), 2);
    assert!(host.last_emitter_source.is_none());
    assert_eq!(host.last_emitter_part, Some(4));
}

#[test]
fn decal_without_a_catalog_entry_still_records_the_request() {
    let mut host = EffectSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = EffectElementInfoMemo::default();
    host.pending_decals.push(QueuedDecalBirth {
        src_name: String::new(),
        ledger_index: NO_RECIPE_LISTING,
        definition_slot: 2,
        window_open_ms: 77,
        dice_start: 5,
        where_at: [1.0, 2.0, 3.0],
        tether_ix: 0,
        stain_on_ent: Some(4),
        turn_rows: IDENTITY,
    });
    queued::settle(&mut host, &catalog, &cache, None);

    assert!(host.pending_decals.is_empty());
    assert!(host.last_stain_source.is_none());
    assert_eq!(host.last_stain_part, Some(2));
    assert_eq!(host.last_stain_ms, Some(77));
    assert_eq!(host.last_stain_origin, Some([1.0, 2.0, 3.0]));
    assert_eq!(host.last_stain_tether, Some(0));
    assert_eq!(host.last_stain_entity, Some(4));
    assert!(host.prior_decal_seen.is_none());
    assert!(host.last_stain_size0.is_none());
    assert!(host.last_stain_color.is_none());
    assert!(host.last_stain_mat0.is_none());
}

#[test]
fn unknown_name_plays_nothing_and_leaves_the_queues_alone() {
    let mut host = EffectSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = EffectElementInfoMemo::default();
    host.pending_runners.push(runner("fx/parent", NO_RECIPE_LISTING, 1));

    let name = FxName::engine("fx/missing");
    assert!(
        play_named_oriented_in_level(&mut host, &catalog, &cache, name, [0.0; 3], IDENTITY, None)
            .is_none()
    );
    assert!(
        spawn_named_oriented_in_world(
            &mut host, &catalog, &cache, name, [0.0; 3], IDENTITY, 10, None
        )
        .is_none()
    );
    assert!(
        play_named_oriented_at_msec(&mut host, &catalog, &cache, name, [0.0; 3], IDENTITY, 10, None)
            .is_none()
    );
    assert_eq!(host.pending_runners.len(), 1);
}

#[test]
fn empty_host_ticks_and_draws_nothing() {
    let mut host = EffectSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = EffectElementInfoMemo::default();
    tick_effect_non_dependent(&mut host, &catalog, &cache, 50, [0.0; 3], None, None);
    tick_effect_remaining(&mut host, &catalog, &cache, 50, [0.0; 3], None, None);
    assert_eq!(host.clock_ms_now, 50);

    let cull = EffectDrawHide { element_drawn: false, plane_list: &[] };
    let (out, gaps) = assemble_effect_vertices(&mut host, &catalog, cull, [0.0; 3]);
    assert!(out.sprites.is_empty() && out.clouds.is_empty() && out.models.is_empty());
    assert_eq!((gaps.catalog_index_total, gaps.catalog_name_total, gaps.shading_blend), (0, 0, 0));
}

#[test]
fn colour_bytes_swap_red_and_blue_only() {
    assert_eq!(swap_red_blue([1, 2, 3, 4]), [3, 2, 1, 4]);
    assert_eq!(swap_red_blue(swap_red_blue([9, 8, 7, 6])), [9, 8, 7, 6]);
}

#[test]
fn light_scale_defaults_to_one_only_for_bo2_effects() {
    assert_eq!(light_scale(0.0, AssetCategoryScope::T6), 1.0);
    assert_eq!(light_scale(-2.0, AssetCategoryScope::T6), 1.0);
    assert_eq!(light_scale(0.5, AssetCategoryScope::T6), 0.5);
    assert_eq!(light_scale(0.0, AssetCategoryScope::T5), 0.0);
}

#[test]
fn collide_work_splits_only_past_sixteen_jobs_a_worker() {
    assert_eq!(worker_count(0), 1);
    assert_eq!(worker_count(31), 1);
    let many = worker_count(1 << 20);
    assert!(many >= 1);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    assert!(many <= cores.max(1));
}
