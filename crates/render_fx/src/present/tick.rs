//! Advancing live effects one frame: wiring the effect simulation to the loaded catalog
//! and to the world's collision.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use asset_game::FxDefinitions;
use bevy::tasks::ComputeTaskPool;
use fx::{
    FX_CATALOG_INDEX_NONE, FxChildKind, FxChildSpawnRequest, FxElemMotionQuery,
    FxElemMotionResult, FxElemTraceHit, FxGapCause, FxPlayPose, FxSystemHost, FxTrailCollideHit,
    PendingCollide, PlayResult, evaluate_elem_collide_motion, evaluate_elem_motion,
    spawn_impact_or_death_effect, spawn_looping_partial, update as fx_update,
};
use fx_t6::runtime::{elem_uses_collision, emit_dist_range, process_emitting_schedule};
use sim::SimWorld;

use super::FxScene;
use super::catalog::{FxElemInfoCache, Launchable, catalog_lookup};
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
pub fn tick_fx_non_dependent(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    msec: i32,
    camera_origin: [f32; 3],
    clip_world: Option<&SimWorld>,
    scene: Option<&dyn FxScene>,
) {
    tick(host, catalog, cache, msec, camera_origin, clip_world, scene, Pass::Unbolted);
}

/// Advance the effects `tick_fx_non_dependent` left, now that bolt poses are final.
pub fn tick_fx_remaining(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    msec: i32,
    camera_origin: [f32; 3],
    clip_world: Option<&SimWorld>,
    scene: Option<&dyn FxScene>,
) {
    tick(host, catalog, cache, msec, camera_origin, clip_world, scene, Pass::Everything);
}

/// How one element moved this step.
struct Stepped {
    motion: Option<FxElemMotionResult>,
    /// Set when the element collides but there was no world to collide with: the element
    /// that was left unsimulated.
    unclipped_elem: Option<u8>,
}

/// Move one element for the span the query covers, colliding it with `clip_world` when
/// its definition asks for collision.
fn step_elem(
    catalog: &FxDefinitions,
    clip_world: Option<&SimWorld>,
    query: FxElemMotionQuery<'_>,
) -> Stepped {
    let Some(elem) = catalog_lookup(catalog, query.catalog_index)
        .and_then(|effect| effect.elems.get(usize::from(query.def_index)))
    else {
        return Stepped {
            motion: None,
            unclipped_elem: None,
        };
    };
    let view = &elem.view;
    if !elem_uses_collision(view.flags) {
        return Stepped {
            motion: evaluate_elem_motion(
                view.flags,
                view.gravity_base,
                view.gravity_amplitude,
                query.elem_random_seed,
                elem.vel_graph_local.as_slice(),
                elem.vel_graph_world.as_slice(),
                query,
            ),
            unclipped_elem: None,
        };
    }
    let Some(world) = clip_world else {
        return Stepped {
            motion: None,
            unclipped_elem: Some(query.def_index),
        };
    };
    let motion = evaluate_elem_collide_motion(
        view.flags,
        view.gravity_base,
        view.gravity_amplitude,
        view.reflection_factor[0],
        view.reflection_factor[1],
        view.coll_mins,
        view.coll_maxs,
        view.use_item_clip != 0,
        !elem.effect_on_impact.is_absent(),
        query.elem_random_seed,
        elem.vel_graph_local.as_slice(),
        elem.vel_graph_world.as_slice(),
        query,
        |start, end, mins, maxs, mask| {
            let hit = world.trace_world(start, end, mins, maxs, mask);
            FxElemTraceHit {
                fraction: hit.fraction,
                normal: hit.normal,
                startsolid: hit.startsolid,
                allsolid: hit.allsolid,
            }
        },
    );
    Stepped {
        motion,
        unclipped_elem: None,
    }
}

/// Every extra thread handles at least this many collision jobs, or it is not worth waking.
const JOBS_PER_WORKER: usize = 16;

/// How many threads should share `jobs` collision jobs. Leaves one core for the caller.
pub(super) fn worker_count(jobs: usize) -> usize {
    static SPARE_CORES: OnceLock<usize> = OnceLock::new();
    let spare = *SPARE_CORES.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(1).max(1))
    });
    spare.min(jobs / JOBS_PER_WORKER).max(1)
}

/// Step a batch of collision jobs, spreading them over worker threads when there are
/// enough. Returns each effect's motion by handle, and the elements left unclipped.
fn step_batch(
    catalog: &FxDefinitions,
    clip_world: Option<&SimWorld>,
    jobs: &[PendingCollide],
) -> (HashMap<u16, Option<FxElemMotionResult>>, Vec<u8>) {
    let step = move |job: &PendingCollide| (job.handle, step_elem(catalog, clip_world, job.query()));
    let workers = worker_count(jobs.len());
    let stepped: Vec<(u16, Stepped)> = if workers <= 1 || jobs.len() == 1 {
        jobs.iter().map(step).collect()
    } else {
        let per_worker = jobs.len().div_ceil(workers);
        ComputeTaskPool::get()
            .scope(|scope| {
                for share in jobs.chunks(per_worker) {
                    scope.spawn(async move { share.iter().map(step).collect::<Vec<_>>() });
                }
            })
            .into_iter()
            .flatten()
            .collect()
    };

    let mut motions = HashMap::with_capacity(stepped.len());
    let mut unclipped = Vec::new();
    for (handle, stepped) in stepped {
        unclipped.extend(stepped.unclipped_elem);
        motions.insert(handle, stepped.motion);
    }
    (motions, unclipped)
}

/// Start the effect an element names for an impact, a death or an emission, if it names
/// one. Returns false when the child could not be started.
fn start_child(
    sys: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    request: &FxChildSpawnRequest,
) -> bool {
    let Some(elem) = catalog_lookup(catalog, request.catalog_index)
        .and_then(|parent| parent.elems.get(usize::from(request.def_index)))
    else {
        return false;
    };
    let edge = match request.kind {
        FxChildKind::Impact => &elem.effect_on_impact,
        FxChildKind::Death => &elem.effect_on_death,
        FxChildKind::Emitted => &elem.effect_emitted,
    };
    if edge.is_absent() {
        return true;
    }
    let Some(child) = edge
        .bound_index()
        .and_then(|index| catalog.def_at(index))
        .filter(|child| !child.elems.is_empty())
    else {
        return false;
    };
    let pose = FxPlayPose {
        origin: request.origin,
        axis: request.axis,
        msec: request.msec,
    };
    let child = Launchable::new(catalog, cache, child);
    match spawn_impact_or_death_effect(sys, child.request(pose)) {
        PlayResult::PlayedReleased { .. } | PlayResult::Held { .. } => true,
        PlayResult::Failed(_) => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn tick(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    msec: i32,
    camera_origin: [f32; 3],
    clip_world: Option<&SimWorld>,
    scene: Option<&dyn FxScene>,
    pass: Pass,
) {
    host.set_msec_now(msec);
    if pass == Pass::Unbolted {
        host.advance_frame_stamp();
    }

    // Elements the simulation had to skip this frame, reported as gaps afterwards.
    let unclipped = RefCell::new(Vec::<u8>::new());
    let unvaried = RefCell::new(Vec::<u8>::new());

    fx_update(
        host,
        pass == Pass::Unbolted,
        camera_origin,
        // Top up a looping effect for the span since it was last advanced.
        |sys, slot, from, to, _| {
            let index = sys
                .effect_at(slot)
                .map_or(FX_CATALOG_INDEX_NONE, |live| live.catalog_index);
            let effect = catalog_lookup(catalog, index)?;
            let launchable = Launchable::at_index(catalog, cache, effect, index);
            spawn_looping_partial(sys, slot, launchable.def_info(), from, to);
            Some(effect.view.msec_looping_life)
        },
        // Move one element.
        |query| {
            let stepped = step_elem(catalog, clip_world, query);
            unclipped.borrow_mut().extend(stepped.unclipped_elem);
            stepped.motion
        },
        // How many emissions an element makes over this span.
        |query| {
            let elem = catalog_lookup(catalog, query.catalog_index)?
                .elems
                .get(usize::from(query.def_index))?;
            if elem.effect_emitted.is_absent() {
                return None;
            }
            let view = &elem.view;
            let (base, max) = emit_dist_range(
                view.emit_dist[0],
                view.emit_dist[1],
                view.emit_dist_variance[0],
                view.emit_dist_variance[1],
                query.elem_random_seed,
            );
            // The variance is part of the range above but is never randomised per emission.
            if view.emit_dist_variance[1] != 0.0 {
                unvaried.borrow_mut().push(query.def_index);
            }
            Some(process_emitting_schedule(
                query.emit_residual,
                query.origin_begin,
                query.origin_end,
                query.msec_update_begin,
                query.msec_update_end,
                base,
                max,
                || 0.0,
            ))
        },
        // Start an impact, death or emitted child.
        |sys, request| start_child(sys, catalog, cache, &request),
        // The element table of a trail's effect.
        |index, def_index| {
            let effect = catalog_lookup(catalog, index)?;
            cache
                .arc_for(catalog, effect)
                .get(usize::from(def_index))
                .copied()
        },
        // A trail's collision trace.
        |start, end, mins, maxs, mask| {
            let world = clip_world?;
            let hit = world.trace_world(start, end, mins, maxs, mask);
            Some(FxTrailCollideHit {
                fraction: hit.fraction,
                normal: hit.normal,
                startsolid: hit.startsolid,
                allsolid: hit.allsolid,
            })
        },
        // The velocity graphs a trail element moves by.
        |index, def_index| {
            catalog_lookup(catalog, index)
                .and_then(|effect| effect.elems.get(usize::from(def_index)))
                .map(|elem| (elem.vel_graph_local.clone(), elem.vel_graph_world.clone()))
                .unwrap_or_default()
        },
        // Collision jobs the simulation batched for parallel evaluation.
        |jobs| {
            if jobs.is_empty() {
                return HashMap::new();
            }
            let (motions, unclipped_here) = step_batch(catalog, clip_world, jobs);
            unclipped.borrow_mut().extend(unclipped_here);
            motions
        },
    );

    for def_index in unclipped.into_inner() {
        host.gaps.raise(FxGapCause::NoWorldClipForCollide { def_index });
    }
    for def_index in unvaried.into_inner() {
        host.gaps
            .raise(FxGapCause::EmitRandVarianceNotApplied { def_index });
    }
    settle(host, catalog, cache, scene);
}
