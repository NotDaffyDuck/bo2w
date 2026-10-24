//! Work an effect leaves behind for the frontend to finish: runner effects spawned by
//! other effects, and decals that turn into surface marks.
//!
//! Starting or ticking effects only queues this work on the host. `settle` empties both
//! queues, running whatever the settled work queues in turn.

use std::sync::OnceLock;

use asset_game::{FxDefinitions, OwnedFxEffectDef};
use fx::{FxGapCause, FxPlayPose, FxSystemHost, PendingDecalSpawn, PendingRunnerSpawn};
use fx_t6::runtime::{
    FX_RAND_CH_COLOR, FX_RAND_CH_SIZE0, elem_visual_index, evaluate_color_bgra,
    evaluate_rotation_total, evaluate_size0, random_table_f32,
};

use super::catalog::{FxElemInfoCache, Launchable, catalog_lookup};
use super::launch::play_at;
use super::FxScene;

/// How many times a queue may refill itself while being emptied. Effects that spawn
/// effects that spawn effects stop here instead of looping forever.
const MAX_ROUNDS: usize = 256;

/// Empty the runner queue, then the decal queue.
pub(super) fn settle(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    scene: Option<&dyn FxScene>,
) {
    for _ in 0..MAX_ROUNDS {
        if host.pending_runners.is_empty() {
            break;
        }
        for request in std::mem::take(&mut host.pending_runners) {
            start_runner(host, catalog, cache, scene, &request);
        }
    }
    for _ in 0..MAX_ROUNDS {
        if host.pending_decals.is_empty() {
            break;
        }
        for request in std::mem::take(&mut host.pending_decals) {
            record_decal(host, catalog, scene, &request);
        }
    }
}

/// Remember which runner could not start so the debug overlay can say so, and raise the gap.
fn note_runner_skipped(host: &mut FxSystemHost, request: &PendingRunnerSpawn) {
    host.last_runner_parent = non_empty(&request.parent_name);
    host.last_runner_elem = Some(request.def_index);
    host.gaps.raise(FxGapCause::ElemRunnerSpawnSkipped {
        def_index: request.def_index,
    });
}

fn non_empty(name: &str) -> Option<String> {
    (!name.is_empty()).then(|| name.to_owned())
}

/// The effect a runner element names as its child, if that effect has anything to play.
fn runner_child<'a>(
    catalog: &'a FxDefinitions,
    request: &PendingRunnerSpawn,
) -> Result<Option<&'a OwnedFxEffectDef>, ()> {
    let edge = catalog_lookup(catalog, request.catalog_index)
        .and_then(|parent| parent.elems.get(usize::from(request.def_index)))
        .and_then(|elem| elem.runner_child_edge(request.random_seed))
        .ok_or(())?;
    if edge.is_absent() {
        // The element deliberately spawns nothing.
        return Ok(None);
    }
    let index = edge.bound_index().ok_or(())?;
    catalog
        .def_at(index)
        .filter(|child| !child.elems.is_empty())
        .map(Some)
        .ok_or(())
}

fn start_runner(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    scene: Option<&dyn FxScene>,
    request: &PendingRunnerSpawn,
) {
    let child = match runner_child(catalog, request) {
        Ok(Some(child)) => child,
        Ok(None) => return,
        Err(()) => return note_runner_skipped(host, request),
    };

    // Marks the child leaves belong to the same entity as the runner's.
    let outer_mark_entity = std::mem::replace(&mut host.spawn_mark_entity, request.mark_entity);
    let launchable = Launchable::new(catalog, cache, child);
    let pose = FxPlayPose {
        origin: request.origin,
        axis: request.axis,
        msec: request.msec_begin,
    };
    play_at(host, catalog, scene, &launchable, pose);
    host.spawn_mark_entity = outer_mark_entity;

    host.last_runner_child = Some(child.name.clone());
    host.last_runner_msec = Some(request.msec_begin);
    host.last_runner_rot_deg = request.rot_deg;
}

/// What the decal debug overlay shows about the element that produced a decal.
fn describe_decal_elem(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    request: &PendingDecalSpawn,
) {
    let Some(elem) = catalog_lookup(catalog, request.catalog_index)
        .and_then(|parent| parent.elems.get(usize::from(request.def_index)))
    else {
        return;
    };
    let seed = request.random_seed;
    let samples = elem.vis_samples.as_slice();
    let intervals = elem.view.vis_state_interval_count;

    host.last_decal_vis = Some(elem_visual_index(elem.view.visual_count, seed) as u8);
    host.last_decal_size0 = evaluate_size0(
        samples,
        intervals,
        0.0,
        random_table_f32(seed, FX_RAND_CH_SIZE0),
    );
    host.last_decal_rotation = evaluate_rotation_total(
        samples,
        intervals,
        0.0,
        seed,
        elem.view.initial_rotation,
        1.0,
    );
    host.last_decal_color = evaluate_color_bgra(
        samples,
        intervals,
        0.0,
        random_table_f32(seed, FX_RAND_CH_COLOR),
    )
    .map(u32::from_le_bytes);
    if let Some(visual) = elem.decal_mark_pair(seed) {
        host.last_decal_mat0_edge = visual.mark_edge_kind(0).map(str::to_owned);
        host.last_decal_mat1_edge = visual.mark_edge_kind(1).map(str::to_owned);
        host.last_decal_mat0 = visual.mark_bound_name(0).map(str::to_owned);
        host.last_decal_mat1 = visual.mark_bound_name(1).map(str::to_owned);
    }
}

/// Set `BO2MP_RENDER_LOG` to log where decals stop on their way to becoming marks.
fn decal_logging() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BO2MP_RENDER_LOG").is_some())
}

/// Log a decal outcome, but only when it differs from the previous line: nearly every shot
/// produces one, and most are identical.
fn log_decal_outcome(line: String) {
    use std::sync::Mutex;
    static PREVIOUS: Mutex<Option<String>> = Mutex::new(None);
    if let Ok(mut previous) = PREVIOUS.lock()
        && previous.as_deref() != Some(line.as_str())
    {
        diag::info!(World, "bo2zm decal: {line}");
        *previous = Some(line);
    }
}

fn record_decal(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn FxScene>,
    request: &PendingDecalSpawn,
) {
    host.last_mark_alloc_slot = None;
    host.last_decal_parent = non_empty(&request.parent_name);
    host.last_decal_elem = Some(request.def_index);
    host.last_decal_msec = Some(request.msec_begin);
    host.last_decal_origin = Some(request.origin);
    host.last_decal_axis = Some(request.axis);
    host.last_decal_bolt = Some(request.bolt);
    host.last_decal_mark_entity = request.mark_entity;
    host.last_decal_vis = None;
    host.last_decal_size0 = None;
    host.last_decal_rotation = None;
    host.last_decal_mat0_edge = None;
    host.last_decal_mat1_edge = None;
    host.last_decal_mat0 = None;
    host.last_decal_mat1 = None;
    host.last_decal_color = None;
    describe_decal_elem(host, catalog, request);

    let outcome = host.record_impact_mark_from_decal(request.bolt);
    if decal_logging() {
        log_decal_outcome(format!(
            "entered={} world={} models={} scene={} mat0={:?}/{:?} mat1={:?}/{:?}",
            outcome.entered,
            outcome.against_world,
            outcome.against_models,
            scene.is_some(),
            host.last_decal_mat0,
            host.last_decal_mat0_edge,
            host.last_decal_mat1,
            host.last_decal_mat1_edge,
        ));
    }

    if outcome.against_world || outcome.against_models {
        match scene {
            Some(scene) => scene.finish_impact_marks(
                host,
                request.def_index,
                outcome.against_world,
                outcome.against_models,
            ),
            None => {
                if outcome.against_world {
                    host.note_world_go_skip(request.def_index);
                }
                if outcome.against_models {
                    host.note_models_go_skip(request.def_index);
                }
            }
        }
    }
    if !outcome.entered {
        host.note_impact_mark_outer_skip(request.def_index);
    }
    host.push_mark_trace(&outcome);
}
