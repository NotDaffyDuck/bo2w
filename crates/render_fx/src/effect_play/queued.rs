//! Work an effect leaves behind for the frontend to finish: runner effects spawned by
//! other effects, and decals that turn into surface marks.
//!
//! Starting or ticking effects only queues this work on the host. `settle` empties both
//! queues, running whatever the settled work queues in turn.

use std::sync::OnceLock;

use asset_game::{FxDefinitions, SelfContainedEffectDef};
use fx::{EffectHoleReason, EffectStartFrame, EffectSystemHost, QueuedDecalBirth, QueuedSpawnerBirth};
use fx_t6::runtime::{
    DICE_LANE_TINT, DICE_LANE_FIRST_EXTENT, elem_visual_index, evaluate_color_bgra,
    evaluate_rotation_total, evaluate_size0, random_table_f32,
};

use super::catalog::{EffectElementInfoMemo, Launchable, recipe_lookup};
use super::launch::play_at;
use super::EffectStage;

/// How many times a queue may refill itself while being emptied. Effects that spawn
/// effects that spawn effects stop here instead of looping forever.
const MAX_ROUNDS: usize = 256;

/// Empty the runner queue, then the decal queue.
pub(super) fn settle(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    scene: Option<&dyn EffectStage>,
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
fn note_runner_skipped(host: &mut EffectSystemHost, request: &QueuedSpawnerBirth) {
    host.last_emitter_source = non_empty(&request.source_name);
    host.last_emitter_part = Some(request.definition_slot);
    host.gaps.log_hole(EffectHoleReason::RunnerNeverStarted {
        definition_slot: request.definition_slot,
    });
}

fn non_empty(name: &str) -> Option<String> {
    (!name.is_empty()).then(|| name.to_owned())
}

/// The effect a runner element names as its child, if that effect has anything to play.
fn runner_child<'a>(
    catalog: &'a FxDefinitions,
    request: &QueuedSpawnerBirth,
) -> Result<Option<&'a SelfContainedEffectDef>, ()> {
    let edge = recipe_lookup(catalog, request.ledger_index)
        .and_then(|parent| parent.elems.get(usize::from(request.definition_slot)))
        .and_then(|elem| elem.runner_kid_link(request.dice_start))
        .ok_or(())?;
    if edge.lacks_entry() {
        // The element deliberately spawns nothing.
        return Ok(None);
    }
    let index = edge.extent_position().ok_or(())?;
    catalog
        .recipe_at(index)
        .filter(|child| !child.elems.is_empty())
        .map(Some)
        .ok_or(())
}

fn start_runner(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    scene: Option<&dyn EffectStage>,
    request: &QueuedSpawnerBirth,
) {
    let child = match runner_child(catalog, request) {
        Ok(Some(child)) => child,
        Ok(None) => return,
        Err(()) => return note_runner_skipped(host, request),
    };

    // Marks the child leaves belong to the same entity as the runner's.
    let outer_mark_entity = std::mem::replace(&mut host.stain_target_entity, request.stain_ent);
    let launchable = Launchable::new(catalog, cache, child);
    let pose = EffectStartFrame {
        anchor: request.spot,
        frame: request.orient_rows,
        ms_now: request.window_open_ms,
    };
    play_at(host, catalog, scene, &launchable, pose);
    host.stain_target_entity = outer_mark_entity;

    host.last_emitter_child = Some(child.name.clone());
    host.last_emitter_ms = Some(request.window_open_ms);
    host.last_emitter_turn_deg = request.turn_deg;
}

/// What the decal debug overlay shows about the element that produced a decal.
fn describe_decal_elem(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    request: &QueuedDecalBirth,
) {
    let Some(elem) = recipe_lookup(catalog, request.ledger_index)
        .and_then(|parent| parent.elems.get(usize::from(request.definition_slot)))
    else {
        return;
    };
    let seed = request.dice_start;
    let samples = elem.visibility_readings.as_slice();
    let intervals = elem.view.look_keyframe_total;

    host.prior_decal_seen = Some(elem_visual_index(elem.view.look_variant_total, seed) as u8);
    host.last_stain_size0 = evaluate_size0(
        samples,
        intervals,
        0.0,
        random_table_f32(seed, DICE_LANE_FIRST_EXTENT),
    );
    host.last_stain_rotation = evaluate_rotation_total(
        samples,
        intervals,
        0.0,
        seed,
        elem.view.birth_spin_angle,
        1.0,
    );
    host.last_stain_color = evaluate_color_bgra(
        samples,
        intervals,
        0.0,
        random_table_f32(seed, DICE_LANE_TINT),
    )
    .map(u32::from_le_bytes);
    if let Some(visual) = elem.decal_pair_look(seed) {
        host.prior_decal_first_border = visual.decal_edge_kind(0).map(str::to_owned);
        host.prior_decal_second_border = visual.decal_edge_kind(1).map(str::to_owned);
        host.last_stain_mat0 = visual.decal_bind_name(0).map(str::to_owned);
        host.prior_decal_second_surface = visual.decal_bind_name(1).map(str::to_owned);
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
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn EffectStage>,
    request: &QueuedDecalBirth,
) {
    host.last_stain_alloc_slot = None;
    host.last_stain_source = non_empty(&request.src_name);
    host.last_stain_part = Some(request.definition_slot);
    host.last_stain_ms = Some(request.window_open_ms);
    host.last_stain_origin = Some(request.where_at);
    host.last_stain_axis = Some(request.turn_rows);
    host.last_stain_tether = Some(request.tether_ix);
    host.last_stain_entity = request.stain_on_ent;
    host.prior_decal_seen = None;
    host.last_stain_size0 = None;
    host.last_stain_rotation = None;
    host.prior_decal_first_border = None;
    host.prior_decal_second_border = None;
    host.last_stain_mat0 = None;
    host.prior_decal_second_surface = None;
    host.last_stain_color = None;
    describe_decal_elem(host, catalog, request);

    let outcome = host.log_stain_from_decal(request.tether_ix);
    if decal_logging() {
        log_decal_outcome(format!(
            "entered={} world={} models={} scene={} mat0={:?}/{:?} mat1={:?}/{:?}",
            outcome.went_inside,
            outcome.against_level,
            outcome.versus_models,
            scene.is_some(),
            host.last_stain_mat0,
            host.prior_decal_first_border,
            host.prior_decal_second_surface,
            host.prior_decal_second_border,
        ));
    }

    if outcome.against_level || outcome.versus_models {
        match scene {
            Some(scene) => scene.finish_impact_stains(
                host,
                request.definition_slot,
                outcome.against_level,
                outcome.versus_models,
            ),
            None => {
                if outcome.against_level {
                    host.note_fragments_skipped(request.definition_slot);
                }
                if outcome.versus_models {
                    host.note_fragments_skipped(request.definition_slot);
                }
            }
        }
    }
    if !outcome.went_inside {
        host.count_stain_outer_skip(request.definition_slot);
    }
    host.log_stain_trace(&outcome);
}
