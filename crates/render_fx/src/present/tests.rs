//! Checks for the present module: catalog lookups, the element-table cache, the lighting
//! tally, createfx boot, queue settling and the empty-host paths.

use std::sync::Arc;

use asset_audio::CreateFxOneshot;
use asset_core::{AssetEdge, AssetNamespace};
use asset_game::{FxDefinitions, FxName, OwnedFxEffectDef, OwnedFxElemDef, OwnedFxTrailDef};
use fx::{
    FX_CATALOG_INDEX_NONE, FxGap, FxPackedLightingSrc, FxSystemHost, PendingDecalSpawn,
    PendingRunnerSpawn,
};
use fx_t6::runtime::{FxEffectDefView, FxElemDefView, apply_lighting_frac_bgra};

use super::catalog::elem_table;
use super::draw::{light_bgra, light_counted};
use super::launch::delayed_msec;
use super::t6::{light_scale, swap_red_blue};
use super::tick::worker_count;
use super::*;

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn blank_view() -> FxElemDefView {
    FxElemDefView {
        flags: 0,
        spawn_a: 0,
        spawn_b: 0,
        spawn_range_base: 0.0,
        spawn_range_amplitude: 0.0,
        fade_in_range: [0.0; 2],
        fade_out_range: [0.0; 2],
        spawn_frustum_cull_radius: 0.0,
        spawn_delay_msec_base: 0,
        spawn_delay_msec_amplitude: 0,
        life_span_msec_base: 0,
        life_span_msec_amplitude: 0,
        spawn_origin: [[0.0; 2]; 3],
        spawn_offset_radius_base: 0.0,
        spawn_offset_radius_amplitude: 0.0,
        spawn_offset_height_base: 0.0,
        spawn_offset_height_amplitude: 0.0,
        spawn_angles: [[0.0; 2]; 3],
        angular_velocity: [[0.0; 2]; 3],
        initial_rotation: [0.0; 2],
        gravity_base: 0.0,
        gravity_amplitude: 0.0,
        reflection_factor: [0.0; 2],
        coll_mins: [0.0; 3],
        coll_maxs: [0.0; 3],
        elem_type: 0,
        visual_count: 0,
        vel_interval_count: 0,
        vis_state_interval_count: 0,
        lighting_frac: 0,
        use_item_clip: 0,
        line: false,
        sort_order: 0,
        emit_dist: [0.0; 2],
        emit_dist_variance: [0.0; 2],
    }
}

fn elem_with(view: FxElemDefView) -> OwnedFxElemDef {
    OwnedFxElemDef {
        view,
        atlas: [0; 8],
        vel_samples: Vec::new(),
        vel_graph_local: Vec::new(),
        vel_graph_world: Vec::new(),
        vis_samples: Vec::new(),
        visuals: Vec::new(),
        effect_on_impact: AssetEdge::Absent,
        effect_on_death: AssetEdge::Absent,
        effect_emitted: AssetEdge::Absent,
        on_impact_name: None,
        on_death_name: None,
        emitted_name: None,
        trail_def: None,
        cloud_density: None,
    }
}

fn effect_named(name: &str, elems: Vec<OwnedFxElemDef>) -> OwnedFxEffectDef {
    OwnedFxEffectDef {
        namespace: AssetNamespace::T6,
        name: name.to_owned(),
        view: FxEffectDefView {
            flags: 0,
            msec_looping_life: 0,
            looping_count: 0,
            one_shot_count: 0,
            emission_count: 0,
        },
        elems,
    }
}

fn catalog_of(effects: Vec<OwnedFxEffectDef>) -> FxDefinitions {
    let mut catalog = FxDefinitions::default();
    for effect in effects {
        catalog.insert_owned(effect);
    }
    catalog
}

fn runner(parent: &str, catalog_index: u16, def_index: u8) -> PendingRunnerSpawn {
    PendingRunnerSpawn {
        mark_entity: None,
        parent_name: parent.to_owned(),
        catalog_index,
        def_index,
        msec_begin: 0,
        random_seed: 0,
        origin: [0.0; 3],
        axis: IDENTITY,
        rot_deg: None,
    }
}

#[test]
fn lookup_rejects_the_none_index_and_out_of_range() {
    let catalog = catalog_of(vec![effect_named("fx/a", Vec::new())]);
    assert!(catalog_lookup(&catalog, FX_CATALOG_INDEX_NONE).is_none());
    assert!(catalog_lookup(&catalog, 5).is_none());
    assert_eq!(catalog_lookup(&catalog, 0).map(|e| e.name.as_str()), Some("fx/a"));
}

#[test]
fn cache_rebuilds_only_when_the_catalog_changes_shape() {
    let two = vec![elem_with(blank_view()), elem_with(blank_view())];
    let mut catalog = catalog_of(vec![effect_named("fx/a", two)]);
    let mut cache = FxElemInfoCache::default();
    cache.sync(&catalog);

    // The catalog knows an effect by its address, so always borrow it from the catalog.
    let first = cache.arc_for(&catalog, catalog.def_at(0).unwrap());
    assert_eq!(first.len(), 2);
    assert!(Arc::ptr_eq(&first, &cache.arc_for(&catalog, catalog.def_at(0).unwrap())));

    cache.sync(&catalog);
    assert!(Arc::ptr_eq(&first, &cache.arc_for(&catalog, catalog.def_at(0).unwrap())));

    // A copy that is not the catalog's own entry is built fresh every time.
    let copy = catalog.def_at(0).unwrap().clone();
    assert!(!Arc::ptr_eq(&first, &cache.arc_for(&catalog, &copy)));

    catalog.insert_owned(effect_named("fx/b", Vec::new()));
    cache.sync(&catalog);
    let rebuilt = cache.arc_for(&catalog, catalog.def_at(0).unwrap());
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert_eq!(rebuilt.len(), 2);

    let stranger = effect_named("fx/not_in_catalog", vec![elem_with(blank_view())]);
    assert_eq!(cache.arc_for(&catalog, &stranger).len(), 1);

    cache.clear();
    let fresh = cache.arc_for(&catalog, catalog.def_at(0).unwrap());
    assert!(!Arc::ptr_eq(&rebuilt, &fresh));
}

#[test]
fn elem_table_carries_the_runtime_fields() {
    let mut view = blank_view();
    view.elem_type = 3;
    view.spawn_delay_msec_base = 11;
    view.spawn_delay_msec_amplitude = 12;
    view.life_span_msec_base = 13;
    view.life_span_msec_amplitude = 14;
    view.gravity_base = 1.5;
    view.gravity_amplitude = 2.5;
    view.reflection_factor = [0.25, 0.75];
    view.coll_mins = [-1.0, -2.0, -3.0];
    view.use_item_clip = 1;
    view.spawn_offset_radius_amplitude = 6.0;
    view.sort_order = 9;
    let mut trailed = elem_with(view);
    trailed.trail_def = Some(OwnedFxTrailDef {
        scroll_time_msec: 0,
        repeat_dist: 0,
        inv_split_dist: 0.5,
        inv_split_arc_dist: 0.25,
        inv_split_time: 0.125,
        verts: Vec::new(),
        inds: Vec::new(),
    });
    let effect = effect_named("fx/table", vec![trailed, elem_with(blank_view())]);
    let table = elem_table(&effect);

    let t = table[0];
    assert_eq!(t.elem_type, 3);
    assert_eq!((t.delay_base, t.delay_amp, t.life_base, t.life_amp), (11, 12, 13, 14));
    assert_eq!((t.gravity_base, t.gravity_amp), (1.5, 2.5));
    assert_eq!((t.reflection_base, t.reflection_amp), (0.25, 0.75));
    assert_eq!(t.coll_mins, [-1.0, -2.0, -3.0]);
    assert_eq!(t.use_item_clip, 1);
    assert_eq!(t.spawn_offset_radius_amp, 6.0);
    assert_eq!(t.sort_order, 9);
    assert_eq!((t.inv_split_dist, t.inv_split_arc_dist, t.inv_split_time), (0.5, 0.25, 0.125));
    assert!(!t.has_effect_on_impact && !t.has_effect_on_death && !t.has_effect_emitted);
    assert_eq!(table[1].inv_split_dist, 0.0);
}

#[test]
fn lighting_tally_counts_each_source() {
    let bgra = [40, 80, 120, 200];
    let sample = [10, 200, 90];
    let mut tally = FxVertsGaps::default();

    let unlit = light_counted(&mut tally, 1, bgra, 0, sample, FxPackedLightingSrc::White);
    assert_eq!(unlit, bgra);
    assert_eq!(tally.lighting_frac, 0);

    let missing = light_counted(&mut tally, 7, bgra, 128, sample, FxPackedLightingSrc::Missing);
    assert_eq!(missing, bgra);
    assert_eq!((tally.lighting_frac, tally.lighting_miss, tally.lighting_applied), (1, 1, 0));
    assert_eq!(tally.lighting_frac_def_indices, vec![7]);

    let lit = apply_lighting_frac_bgra(bgra, sample, 128);
    assert_eq!(light_counted(&mut tally, 2, bgra, 128, sample, FxPackedLightingSrc::White), lit);
    assert_eq!(
        light_counted(&mut tally, 3, bgra, 128, sample, FxPackedLightingSrc::AtPointRgb),
        lit
    );
    assert_eq!((tally.lighting_frac, tally.lighting_applied), (3, 2));
    assert_eq!((tally.lighting_white, tally.lighting_atpoint), (1, 1));
    assert_eq!(tally.lighting_frac_def_indices, vec![7]);

    assert_eq!(light_bgra(bgra, 128, sample, FxPackedLightingSrc::Missing), bgra);
    assert_eq!(light_bgra(bgra, 128, sample, FxPackedLightingSrc::AtPointRgb), lit);
}

#[test]
fn boot_counts_createfx_ids_the_catalog_lacks() {
    let mut host = FxSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = FxElemInfoCache::default();
    let shots = vec![CreateFxOneshot {
        fxid: "fx/not_there".to_owned(),
        origin_inches: [0.0; 3],
        angles_deg: [0.0; 3],
        delay: 0.5,
    }];
    let boot = boot_createfx_effects(&mut host, &catalog, &cache, &shots, 1000, None);
    assert_eq!((boot.named, boot.held, boot.miss_def), (0, 0, 1));
    assert!(boot.failed.is_empty());
    assert_eq!(host.msec_now, 1000);
}

#[test]
fn createfx_delay_is_whole_milliseconds_and_wraps() {
    assert_eq!(delayed_msec(1000, 0.25), 1250);
    assert_eq!(delayed_msec(1000, 0.0019), 1001);
    assert_eq!(delayed_msec(i32::MAX, 0.001), i32::MIN);
}

#[test]
fn runner_without_a_catalog_entry_is_noted_as_skipped() {
    let mut host = FxSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = FxElemInfoCache::default();
    host.pending_runners.push(runner("fx/parent", FX_CATALOG_INDEX_NONE, 3));
    queued::settle(&mut host, &catalog, &cache, None);

    assert!(host.pending_runners.is_empty());
    assert_eq!(host.gaps.hits(FxGap::ElemRunnerSpawn), 1);
    assert_eq!(host.last_runner_parent.as_deref(), Some("fx/parent"));
    assert_eq!(host.last_runner_elem, Some(3));
    assert!(host.last_runner_child.is_none());

    host.pending_runners.push(runner("", FX_CATALOG_INDEX_NONE, 4));
    queued::settle(&mut host, &catalog, &cache, None);
    assert_eq!(host.gaps.hits(FxGap::ElemRunnerSpawn), 2);
    assert!(host.last_runner_parent.is_none());
    assert_eq!(host.last_runner_elem, Some(4));
}

#[test]
fn decal_without_a_catalog_entry_still_records_the_request() {
    let mut host = FxSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = FxElemInfoCache::default();
    host.pending_decals.push(PendingDecalSpawn {
        parent_name: String::new(),
        catalog_index: FX_CATALOG_INDEX_NONE,
        def_index: 2,
        msec_begin: 77,
        random_seed: 5,
        origin: [1.0, 2.0, 3.0],
        bolt: 0,
        mark_entity: Some(4),
        axis: IDENTITY,
    });
    queued::settle(&mut host, &catalog, &cache, None);

    assert!(host.pending_decals.is_empty());
    assert!(host.last_decal_parent.is_none());
    assert_eq!(host.last_decal_elem, Some(2));
    assert_eq!(host.last_decal_msec, Some(77));
    assert_eq!(host.last_decal_origin, Some([1.0, 2.0, 3.0]));
    assert_eq!(host.last_decal_bolt, Some(0));
    assert_eq!(host.last_decal_mark_entity, Some(4));
    assert!(host.last_decal_vis.is_none());
    assert!(host.last_decal_size0.is_none());
    assert!(host.last_decal_color.is_none());
    assert!(host.last_decal_mat0.is_none());
}

#[test]
fn unknown_name_plays_nothing_and_leaves_the_queues_alone() {
    let mut host = FxSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = FxElemInfoCache::default();
    host.pending_runners.push(runner("fx/parent", FX_CATALOG_INDEX_NONE, 1));

    let name = FxName::engine("fx/missing");
    assert!(
        play_named_oriented_in_world(&mut host, &catalog, &cache, name, [0.0; 3], IDENTITY, None)
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
    let mut host = FxSystemHost::new();
    let catalog = FxDefinitions::default();
    let cache = FxElemInfoCache::default();
    tick_fx_non_dependent(&mut host, &catalog, &cache, 50, [0.0; 3], None, None);
    tick_fx_remaining(&mut host, &catalog, &cache, 50, [0.0; 3], None, None);
    assert_eq!(host.msec_now, 50);

    let cull = FxDrawCull { elem_draw: false, planes: &[] };
    let (out, gaps) = build_fx_verts(&mut host, &catalog, cull, [0.0; 3]);
    assert!(out.sprites.is_empty() && out.clouds.is_empty() && out.models.is_empty());
    assert_eq!((gaps.catalog_index_n, gaps.catalog_name_n, gaps.lighting_frac), (0, 0, 0));
}

#[test]
fn colour_bytes_swap_red_and_blue_only() {
    assert_eq!(swap_red_blue([1, 2, 3, 4]), [3, 2, 1, 4]);
    assert_eq!(swap_red_blue(swap_red_blue([9, 8, 7, 6])), [9, 8, 7, 6]);
}

#[test]
fn light_scale_defaults_to_one_only_for_bo2_effects() {
    assert_eq!(light_scale(0.0, AssetNamespace::T6), 1.0);
    assert_eq!(light_scale(-2.0, AssetNamespace::T6), 1.0);
    assert_eq!(light_scale(0.5, AssetNamespace::T6), 0.5);
    assert_eq!(light_scale(0.0, AssetNamespace::T5), 0.0);
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
