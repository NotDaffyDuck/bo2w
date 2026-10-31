//! Advancing live effects one frame: wiring the effect simulation to the loaded catalog
//! and to the world's collision.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use asset_game::FxDefinitions;
use bevy::tasks::ComputeTaskPool;
use fx::{
    NO_RECIPE_LISTING, OffspringSort, OffspringBirthAsk, ParticleMoveAsk,
    ParticleMoveOutcome, ParticleRayContact, EffectHoleReason, EffectStartFrame, EffectSystemHost, StreakBounceContact,
    QueuedBounceCheck, PlaybackResult, compute_bouncing_particle_path, compute_particle_path,
    play_hit_or_death_effect, birth_repeat_fraction, advance_effects,
};
use fx_t6::runtime::{elem_uses_collision, emit_dist_range, process_emitting_schedule};
use sim::SimWorld;

use super::EffectStage;
use super::catalog::{EffectElementInfoMemo, Launchable, recipe_lookup};
use super::queued::settle;

/// Which effects a tick pass advances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Starts the frame: bumps the frame stamp and advances only the effects that are not
    /// bolted to anything (bolted ones depend on poses that are not final yet).
    Unbolted,
    /// Finishes the frame: advances everything the first pass left.
    Everything,
}

/// Advance the effects that do not depend on any bolt pose, and start a new frame.
pub fn tick_effect_non_dependent(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    msec: i32,
    eye_at: [f32; 3],
    world_clip: Option<&SimWorld>,
    scene: Option<&dyn EffectStage>,
) {
    tick(host, catalog, cache, msec, eye_at, world_clip, scene, Pass::Unbolted);
}

/// Advance the effects `tick_effect_non_dependent` left, now that bolt poses are final.
pub fn tick_effect_remaining(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    msec: i32,
    eye_at: [f32; 3],
    world_clip: Option<&SimWorld>,
    scene: Option<&dyn EffectStage>,
) {
    tick(host, catalog, cache, msec, eye_at, world_clip, scene, Pass::Everything);
}

/// How one element moved this step.
struct Stepped {
    motion: Option<ParticleMoveOutcome>,
    /// Set when the element collides but there was no world to collide with: the element
    /// that was left unsimulated.
    unclipped_elem: Option<u8>,
}

impl Stepped {
    /// A step that ran (or found nothing to run) and so left no element unclipped.
    fn moved(motion: Option<ParticleMoveOutcome>) -> Self {
        Self {
            motion,
            unclipped_elem: None,
        }
    }
}

/// Move one element for the span the query covers, colliding it with `clip_world` when
/// its definition asks for collision.
fn step_elem(
    catalog: &FxDefinitions,
    world_clip: Option<&SimWorld>,
    query: ParticleMoveAsk<'_>,
) -> Stepped {
    let Some(elem) = recipe_lookup(catalog, query.ledger_index)
        .and_then(|effect| effect.elems.get(usize::from(query.definition_slot)))
    else {
        return Stepped::moved(None);
    };
    let view = &elem.view;
    let (local_graph, world_graph) = (elem.vel_graph_local.as_slice(), elem.vel_graph_world.as_slice());
    match (elem_uses_collision(view.flags), world_clip) {
        (false, _) => Stepped::moved(compute_particle_path(
            view.flags,
            view.fall_pull_floor,
            view.fall_pull_swing,
            query.part_seed,
            local_graph,
            world_graph,
            query,
        )),
        (true, None) => Stepped {
            motion: None,
            unclipped_elem: Some(query.definition_slot),
        },
        (true, Some(world)) => Stepped::moved(compute_bouncing_particle_path(
            view.flags,
            view.fall_pull_floor,
            view.fall_pull_swing,
            view.mirror_strength[0],
            view.mirror_strength[1],
            view.bounce_box_bottom,
            view.bounce_box_top,
            view.collide_with_pickups != 0,
            !elem.effect_upon_strike.lacks_entry(),
            query.part_seed,
            local_graph,
            world_graph,
            query,
            |from_pt, end, mins, maxs, mask| {
                let hit = world.trace_world(from_pt, end, mins, maxs, mask);
                ParticleRayContact {
                    hit_frac: hit.fraction,
                    facing: hit.normal,
                    startsolid: hit.starts_inside,
                    fully_embedded: hit.fully_embedded,
                }
            },
        )),
    }
}

/// Every extra thread handles at least this many collision jobs, or it is not worth waking.
const JOBS_PER_WORKER: usize = 16;

/// How many threads should share `jobs` collision jobs. Leaves one core for the caller.
pub(super) fn worker_count(jobs: usize) -> usize {
    static SPARE_CORES: OnceLock<usize> = OnceLock::new();
    let spare = *SPARE_CORES.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|cores| cores.get() - 1)
            .unwrap_or(0)
            .max(1)
    });
    (jobs / JOBS_PER_WORKER).clamp(1, spare)
}

/// A stepped batch: each effect's motion by handle, and the elements left unclipped.
struct BatchSteps {
    motions: HashMap<u16, Option<ParticleMoveOutcome>>,
    unclipped: Vec<u8>,
}

/// Step a batch of collision jobs, spreading them over worker threads when there are
/// enough.
fn step_batch(
    catalog: &FxDefinitions,
    world_clip: Option<&SimWorld>,
    work_items: &[QueuedBounceCheck],
) -> BatchSteps {
    let step = move |job: &QueuedBounceCheck| (job.fx_ref, step_elem(catalog, world_clip, job.query()));
    let workers = worker_count(work_items.len());
    let stepped: Vec<(u16, Stepped)> = match (workers, work_items.len()) {
        (0..=1, _) | (_, 1) => work_items.iter().map(step).collect(),
        (workers, count) => ComputeTaskPool::get()
            .scope(|pool_scope| {
                work_items.chunks(count.div_ceil(workers)).for_each(|share| {
                    pool_scope.spawn(async move { share.iter().map(step).collect::<Vec<_>>() });
                });
            })
            .into_iter()
            .flatten()
            .collect(),
    };

    let mut batch = BatchSteps { motions: HashMap::with_capacity(work_items.len()), unclipped: Vec::new() };
    for (key, step) in stepped {
        batch.unclipped.extend(step.unclipped_elem);
        batch.motions.insert(key, step.motion);
    }
    batch
}

/// Start the effect an element names for an impact, a death or an emission, if it names
/// one. Returns false when the child could not be started.
fn start_child(
    sys: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    request: &OffspringBirthAsk,
) -> bool {
    let Some(elem) = recipe_lookup(catalog, request.ledger_index)
        .and_then(|parent| parent.elems.get(usize::from(request.definition_slot)))
    else {
        return false;
    };
    let edge = match request.kind {
        OffspringSort::Impact => &elem.effect_upon_strike,
        OffspringSort::Death => &elem.effect_upon_demise,
        OffspringSort::Emitted => &elem.effect_spawned,
    };
    edge.lacks_entry()
        || edge
            .extent_position()
            .and_then(|index| catalog.recipe_at(index))
            .filter(|child| !child.elems.is_empty())
            .is_some_and(|child| {
                let pose = EffectStartFrame {
                    anchor: request.spot_xyz,
                    frame: request.turn_rows,
                    ms_now: request.clock_ms,
                };
                let launched = Launchable::new(catalog, cache, child);
                !matches!(
                    play_hit_or_death_effect(sys, launched.request(pose)),
                    PlaybackResult::Refused(_)
                )
            })
}

#[allow(clippy::too_many_arguments)]
fn tick(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    msec: i32,
    eye_at: [f32; 3],
    world_clip: Option<&SimWorld>,
    scene: Option<&dyn EffectStage>,
    pass: Pass,
) {
    host.store_effect_clock(msec);
    if pass == Pass::Unbolted {
        host.bump_tick_marker();
    }

    // Elements the simulation had to skip this frame, reported as gaps afterwards.
    let unclipped = RefCell::new(Vec::<u8>::new());
    let unvaried = RefCell::new(Vec::<u8>::new());

    advance_effects(
        host,
        pass == Pass::Unbolted,
        eye_at,
        // Top up a looping effect for the span since it was last advanced.
        |sys, ask| {
            let index = sys
                .emitter_in_slot(ask.slot)
                .map_or(NO_RECIPE_LISTING, |live| live.ledger_index);
            let effect = recipe_lookup(catalog, index)?;
            let launchable = Launchable::at_index(catalog, cache, effect, index);
            birth_repeat_fraction(sys, ask.slot, launchable.def_info(), ask.from_ms, ask.to_ms);
            Some(effect.view.loop_lifetime_millis)
        },
        // Move one element.
        |query| {
            let stepped = step_elem(catalog, world_clip, query);
            unclipped.borrow_mut().extend(stepped.unclipped_elem);
            stepped.motion
        },
        // How many emissions an element makes over this span.
        |query| {
            let elem = recipe_lookup(catalog, query.ledger_index)?
                .elems
                .get(usize::from(query.definition_slot))?;
            if elem.effect_spawned.lacks_entry() {
                return None;
            }
            let view = &elem.view;
            let (base, max) = emit_dist_range(
                view.emission_spacing[0],
                view.emission_spacing[1],
                view.emission_spacing_spread[0],
                view.emission_spacing_spread[1],
                query.seed_bits,
            );
            // The variance is part of the range above but is never randomised per emission.
            if view.emission_spacing_spread[1] != 0.0 {
                unvaried.borrow_mut().push(query.definition_slot);
            }
            Some(process_emitting_schedule(
                query.emission_carry,
                query.start_point,
                query.closing_spot,
                query.refresh_window_open_ms,
                query.refresh_window_close_ms,
                base,
                max,
                || 0.0,
            ))
        },
        // Start an impact, death or emitted child.
        |sys, request| start_child(sys, catalog, cache, &request),
        // The element table of a trail's effect.
        |index, definition_slot| {
            let effect = recipe_lookup(catalog, index)?;
            cache
                .shared_handle_for(catalog, effect)
                .get(usize::from(definition_slot))
                .copied()
        },
        // A trail's collision trace.
        |from_pt, end, mins, maxs, mask| {
            let world = world_clip?;
            let hit = world.trace_world(from_pt, end, mins, maxs, mask);
            Some(StreakBounceContact {
                hit_frac: hit.fraction,
                hit_facing: hit.normal,
                startsolid: hit.starts_inside,
                fully_embedded: hit.fully_embedded,
            })
        },
        // The velocity graphs a trail element moves by.
        |index, definition_slot| {
            recipe_lookup(catalog, index)
                .and_then(|effect| effect.elems.get(usize::from(definition_slot)))
                .map(|part| (part.vel_graph_local.clone(), part.vel_graph_world.clone()))
                .unwrap_or_default()
        },
        // Collision jobs the simulation batched for parallel evaluation.
        |work_items| {
            if work_items.is_empty() {
                return HashMap::new();
            }
            let BatchSteps { motions, unclipped: unclipped_here } = step_batch(catalog, world_clip, work_items);
            unclipped.borrow_mut().extend(unclipped_here);
            motions
        },
    );

    for definition_slot in unclipped.into_inner() {
        host.gaps.log_hole(EffectHoleReason::CollideWithoutWorld { definition_slot });
    }
    for definition_slot in unvaried.into_inner() {
        host.gaps
            .log_hole(EffectHoleReason::VarianceNeverApplied { definition_slot });
    }
    settle(host, catalog, cache, scene);
}
