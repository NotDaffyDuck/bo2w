//! The per-frame effect update: each live effect is brought up to the
//! current time, its parts move, spawn children and die, and its trails grow.
//!
//! What the host world has to say about motion, emission, child effects and
//! trails comes in as callbacks, so this module never reaches into a
//! simulation of its own.

use std::collections::HashMap;

use fx_t6::runtime::{
    FX_ELEM_EMIT_ORIENT_AXIS, FX_STATUS_HAS_PENDING_LOOP_ELEMS, FX_STATUS_REF_COUNT_LOW_MASK,
    FxElemVec3Range, FxEmitSchedule, FxOrientation, FxUpdateEffectBolt, axis_to_quat,
    begin_iterating_over_effects_exclusive, bolt_compose_orientation, bolt_mark_lost,
    elem_random_seed, elem_uses_collision, emit_lerp_origin, end_iterating_over_effects,
    end_iterating_runs_gc, get_orientation, orientation_pos_to_world, unit_quat_to_axis,
    update_effect_bolt, vec3_distance, vector_vectors,
};

use crate::def::FxElemDefInfo;
use crate::elem::{FX_ELEM_HANDLE_NONE, elem_slot_for_handle};
use crate::gaps::{ChildSpawn, FxGapCause};
use crate::spawn::{free_elem, stop_effect_non_recursive, stop_pending_loop};
use crate::system::{BOLT_NONE, FX_CATALOG_INDEX_NONE, FxEffectSlot, FxSystemHost};
use crate::trail::{
    FX_TRAIL_HANDLE_NONE, FxTrailCollideHit, apply_partial_last_trail_spawn_dist,
    update_effect_trails,
};

/// Everything the host is asked about one part's motion over a time slice.
#[derive(Clone, Copy, Debug)]
pub struct FxElemMotionQuery<'a> {
    /// The owning effect's name; empty when the effect is known by catalog index.
    pub def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
    /// Fraction of the part's life at the start and end of the slice.
    pub age0: f32,
    pub age1: f32,
    pub life_ms: f32,
    pub dt_sec: f32,
    pub base_vel: [f32; 3],
    pub elem_random_seed: u32,
    /// The part's position in its own space.
    pub origin: [f32; 3],
    pub prev_msec: i32,
    pub msec_now: i32,
    pub msec_begin: i32,
    pub effect_axis: [[f32; 3]; 3],
    /// How the part's space sits in the world.
    pub orient: FxOrientation,
    pub at_rest_fraction: u8,
}

/// The owned fields of a motion query: everything except the effect name,
/// which a query borrows from wherever the name lives.
#[derive(Clone, Copy, Debug)]
struct MotionInputs {
    catalog_index: u16,
    def_index: u8,
    age0: f32,
    age1: f32,
    life_ms: f32,
    dt_sec: f32,
    base_vel: [f32; 3],
    elem_random_seed: u32,
    origin: [f32; 3],
    prev_msec: i32,
    msec_now: i32,
    msec_begin: i32,
    effect_axis: [[f32; 3]; 3],
    orient: FxOrientation,
    at_rest_fraction: u8,
}

impl MotionInputs {
    fn new(
        part: &LivePart,
        view: &EffectView,
        orient: FxOrientation,
        seed: u32,
        prev_msec: i32,
        msec_now: i32,
    ) -> Self {
        let life_ms = part.life_msec.max(1) as f32;
        let age_at = |msec: i32| msec.saturating_sub(part.msec_begin).max(0) as f32 / life_ms;
        Self {
            catalog_index: view.catalog_index,
            def_index: part.def_index,
            age0: age_at(prev_msec),
            age1: age_at(msec_now),
            life_ms,
            dt_sec: msec_now.saturating_sub(prev_msec).max(0) as f32 * 0.001,
            base_vel: part.base_vel,
            elem_random_seed: seed,
            origin: part.origin,
            prev_msec,
            msec_now,
            msec_begin: part.msec_begin,
            effect_axis: view.now.axis,
            orient,
            at_rest_fraction: part.at_rest_fraction,
        }
    }

    fn query<'a>(&self, def_name: &'a str) -> FxElemMotionQuery<'a> {
        FxElemMotionQuery {
            def_name,
            catalog_index: self.catalog_index,
            def_index: self.def_index,
            age0: self.age0,
            age1: self.age1,
            life_ms: self.life_ms,
            dt_sec: self.dt_sec,
            base_vel: self.base_vel,
            elem_random_seed: self.elem_random_seed,
            origin: self.origin,
            prev_msec: self.prev_msec,
            msec_now: self.msec_now,
            msec_begin: self.msec_begin,
            effect_axis: self.effect_axis,
            orient: self.orient,
            at_rest_fraction: self.at_rest_fraction,
        }
    }
}

/// A colliding part's motion, worked out ahead of time so the host can trace
/// all of them together before any part is moved.
#[derive(Clone, Debug)]
pub struct PendingCollide {
    pub handle: u16,
    def_name: String,
    inputs: MotionInputs,
}

impl PendingCollide {
    pub fn query(&self) -> FxElemMotionQuery<'_> {
        self.inputs.query(&self.def_name)
    }
}

/// What the host worked out for one part's motion.
#[derive(Clone, Copy, Debug)]
pub struct FxElemMotionResult {
    /// How far the part moved, in its own space.
    pub origin_delta: [f32; 3],
    pub base_vel: [f32; 3],
    /// The part died on touch.
    pub remove: bool,
    /// Set when the part came to rest during the slice.
    pub at_rest_fraction: Option<u8>,
    pub spawn_impact: Option<FxImpactSpawn>,
}

/// Where and how fast a part hit something, for its impact child.
#[derive(Clone, Copy, Debug)]
pub struct FxImpactSpawn {
    pub origin: [f32; 3],
    /// Velocity just before the hit.
    pub pre_vel: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxChildKind {
    Impact,
    Death,
    Emitted,
}

impl From<FxChildKind> for ChildSpawn {
    fn from(kind: FxChildKind) -> Self {
        match kind {
            FxChildKind::Impact => ChildSpawn::Impact,
            FxChildKind::Death => ChildSpawn::Death,
            FxChildKind::Emitted => ChildSpawn::Emitted,
        }
    }
}

/// A child effect a part wants played.
#[derive(Clone, Copy, Debug)]
pub struct FxChildSpawnRequest<'a> {
    pub kind: FxChildKind,
    pub parent_def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub msec: i32,
}

/// What a world trace found.
#[derive(Clone, Copy, Debug)]
pub struct FxElemTraceHit {
    pub fraction: f32,
    pub normal: [f32; 3],
    pub startsolid: bool,
    pub allsolid: bool,
}

/// A moving part asking when it should emit.
#[derive(Clone, Copy, Debug)]
pub struct FxEmitQuery<'a> {
    pub def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
    pub origin_begin: [f32; 3],
    pub origin_end: [f32; 3],
    pub msec_update_begin: i32,
    pub msec_update_end: i32,
    pub emit_residual: u8,
    pub elem_random_seed: u32,
    pub flags: i32,
}

type TrailTraceHook<'a> =
    dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<FxTrailCollideHit> + 'a;
type TrailVelGraphsHook<'a> =
    dyn FnMut(u16, u8) -> (Vec<FxElemVec3Range>, Vec<FxElemVec3Range>) + 'a;
type ChildHook<'a> = dyn FnMut(&mut FxSystemHost, FxChildSpawnRequest<'_>) -> bool + 'a;

/// The host callbacks used while effects are applied, gathered so they travel
/// as one argument.
struct Hooks<'a> {
    motion: &'a mut dyn FnMut(FxElemMotionQuery<'_>) -> Option<FxElemMotionResult>,
    emit: &'a mut dyn FnMut(FxEmitQuery<'_>) -> Option<FxEmitSchedule>,
    child: &'a mut ChildHook<'a>,
    trail_def: &'a mut dyn FnMut(u16, u8) -> Option<FxElemDefInfo>,
    trail_trace: &'a mut TrailTraceHook<'a>,
    trail_vel_graphs: &'a mut TrailVelGraphsHook<'a>,
}

/// Brings every live effect up to `host.msec_now`.
///
/// Effects not attached to a bolt go first, then (unless `non_bolted_only`)
/// the bolted ones. Each effect that is due is first stamped and has its
/// pending loop checked; the colliding parts among them are then traced
/// together through `collide_parallel`; only then is each effect applied.
pub fn update(
    host: &mut FxSystemHost,
    non_bolted_only: bool,
    camera_origin: [f32; 3],
    mut pending_loop: impl FnMut(&mut FxSystemHost, usize, i32, i32, &str) -> Option<i32>,
    mut motion: impl FnMut(FxElemMotionQuery<'_>) -> Option<FxElemMotionResult>,
    mut on_emit: impl FnMut(FxEmitQuery<'_>) -> Option<FxEmitSchedule>,
    mut on_child: impl FnMut(&mut FxSystemHost, FxChildSpawnRequest<'_>) -> bool,
    mut on_trail_def: impl FnMut(u16, u8) -> Option<FxElemDefInfo>,
    mut on_trail_trace: impl FnMut(
        [f32; 3],
        [f32; 3],
        [f32; 3],
        [f32; 3],
        u32,
    ) -> Option<FxTrailCollideHit>,
    mut on_trail_vel_graphs: impl FnMut(u16, u8) -> (Vec<FxElemVec3Range>, Vec<FxElemVec3Range>),
    collide_parallel: impl FnOnce(&[PendingCollide]) -> HashMap<u16, Option<FxElemMotionResult>>,
) {
    host.iterator_count = begin_iterating_over_effects_exclusive(host.iterator_count);
    host.camera_origin = Some(camera_origin);

    let ring_begin = host.first_active_effect;
    let ring_end = host.first_new_effect;
    let mut due: Vec<(usize, i32, i32)> = Vec::new();
    let mut collide_jobs: Vec<PendingCollide> = Vec::new();

    let passes: &[bool] = if non_bolted_only {
        &[false]
    } else {
        &[false, true]
    };
    for &bolted_pass in passes {
        let mut cursor = ring_begin;
        while cursor != ring_end {
            let handle = host.handle_at_ring(cursor as u32);
            cursor = cursor.wrapping_add(1);
            let Some(slot) = host.slot_index_for_handle(handle) else {
                continue;
            };
            if let Some((prev_msec, msec_now)) =
                begin_effect_update(host, slot, bolted_pass, &mut pending_loop)
            {
                gather_collide_jobs(host, slot, prev_msec, msec_now, &mut collide_jobs);
                due.push((slot, prev_msec, msec_now));
            }
        }
    }

    let collide_results = collide_parallel(&collide_jobs);

    let mut hooks = Hooks {
        motion: &mut motion,
        emit: &mut on_emit,
        child: &mut on_child,
        trail_def: &mut on_trail_def,
        trail_trace: &mut on_trail_trace,
        trail_vel_graphs: &mut on_trail_vel_graphs,
    };
    for (slot, prev_msec, msec_now) in due {
        apply_effect(
            host,
            slot,
            prev_msec,
            msec_now,
            camera_origin,
            &mut hooks,
            &collide_results,
        );
    }

    host.iterator_count = end_iterating_over_effects(host.iterator_count);
    if end_iterating_runs_gc(host.iterator_count, host.needs_garbage_collection) {
        host.run_garbage_collection();
    }
}

// --- deciding who is due -----------------------------------------------------------

/// Stamps an effect as seen this frame and, when it is due, answers with the
/// time slice to update it over. A looping effect past the end of its
/// looping life stops looping here.
fn begin_effect_update(
    host: &mut FxSystemHost,
    slot: usize,
    bolted_pass: bool,
    pending_loop: &mut dyn FnMut(&mut FxSystemHost, usize, i32, i32, &str) -> Option<i32>,
) -> Option<(i32, i32)> {
    let frame = host.frame_stamp;
    let msec_now = host.msec_now;

    let effect = host.effect_at_mut(slot).filter(|e| e.ring_resident)?;
    if (effect.bolt != BOLT_NONE) != bolted_pass {
        return None;
    }
    if std::mem::replace(&mut effect.frame_stamp, frame) == frame {
        return None;
    }
    if effect.status & FX_STATUS_REF_COUNT_LOW_MASK == 0 || effect.msec_last_update > msec_now {
        return None;
    }

    refresh_bolt(host, slot);

    let effect = host.effect_at(slot)?;
    let prev_msec = effect.msec_last_update;
    if effect.status & FX_STATUS_HAS_PENDING_LOOP_ELEMS != 0 {
        let name = catalog_free_name(effect);
        let age = msec_now.wrapping_sub(effect.msec_begin);
        if let Some(looping_life) = pending_loop(host, slot, prev_msec, msec_now, &name)
            && looping_life < age
        {
            stop_pending_loop(host, slot);
        }
    }
    Some((prev_msec, msec_now))
}

/// An effect carries its definition's name only when it is not in the
/// catalog; queries from catalogued effects name them by index instead.
fn catalog_free_name(effect: &FxEffectSlot) -> String {
    if effect.catalog_index == FX_CATALOG_INDEX_NONE {
        effect.def_name.clone()
    } else {
        String::new()
    }
}

/// Moves a bolted effect to where its bone is now, or stops it when the
/// thing it was bolted to is gone.
fn refresh_bolt(host: &mut FxSystemHost, slot: usize) {
    let Some(effect) = host.effect_at_mut(slot) else {
        return;
    };
    let verdict = update_effect_bolt(
        effect.bolt,
        effect.bolt_packed,
        effect.bolt_centity_teleport,
        effect.bolt_bone_pose.is_some(),
    );
    match verdict {
        FxUpdateEffectBolt::Skip => {}
        FxUpdateEffectBolt::Refresh => {
            if let Some((bone_origin, bone_axis)) = effect.bolt_bone_pose {
                let (quat, origin) = bolt_compose_orientation(
                    effect.bolt_parent_quat,
                    effect.bolt_parent_origin,
                    axis_to_quat(bone_axis),
                    bone_origin,
                );
                effect.origin = origin;
                effect.axis = unit_quat_to_axis(quat);
            }
        }
        FxUpdateEffectBolt::Lost => {
            effect.bolt_packed = bolt_mark_lost(effect.bolt_packed);
            stop_effect_non_recursive(host, slot);
        }
    }
}

// --- what an effect and its parts look like to the update --------------------------

/// What the effect owning a run of parts contributes to updating them.
struct EffectView {
    def_name: String,
    catalog_index: u16,
    random_seed: u16,
    now: fx_t6::runtime::FxOrientFrame,
    when_played: fx_t6::runtime::FxOrientFrame,
}

impl EffectView {
    fn of(host: &FxSystemHost, slot: usize) -> Option<Self> {
        let e = host.effect_at(slot)?;
        Some(Self {
            def_name: catalog_free_name(e),
            catalog_index: e.catalog_index,
            random_seed: e.random_seed,
            now: e.frame_now(),
            when_played: e.frame_when_played(),
        })
    }
}

/// One part's state, copied out so the pool can be changed while it is used.
struct LivePart {
    slot: usize,
    def_index: u8,
    flags: i32,
    sequence: u8,
    msec_begin: i32,
    life_msec: i32,
    base_vel: [f32; 3],
    origin: [f32; 3],
    at_rest_fraction: u8,
}

impl LivePart {
    fn read(host: &FxSystemHost, handle: u16) -> Option<Self> {
        let slot = elem_slot_for_handle(handle)?;
        let e = host.elems.get(slot).filter(|e| e.occupied)?;
        Some(Self {
            slot,
            def_index: e.def_index,
            flags: e.flags,
            sequence: e.sequence,
            msec_begin: e.msec_begin,
            life_msec: e.life_span_msec,
            base_vel: e.base_vel,
            origin: e.origin,
            at_rest_fraction: e.at_rest_fraction,
        })
    }

    fn death_msec(&self) -> i32 {
        self.msec_begin.wrapping_add(self.life_msec)
    }

    fn seed(&self, view: &EffectView) -> u32 {
        elem_random_seed(view.random_seed, self.sequence, self.msec_begin)
    }

    /// The space the part is simulated in, relative to the world.
    fn orientation(&self, host: &FxSystemHost, view: &EffectView, seed: u32) -> FxOrientation {
        let spawn = host
            .elems
            .get(self.slot)
            .map(|e| e.orient_spawn_params(seed));
        get_orientation(self.flags, &view.now, &view.when_played, spawn)
    }
}

/// The part that follows `handle` in its list.
fn next_in_list(host: &FxSystemHost, handle: u16) -> u16 {
    elem_slot_for_handle(handle)
        .and_then(|slot| host.elems.get(slot))
        .map_or(FX_ELEM_HANDLE_NONE, |e| e.next_elem_handle)
}

fn first_in_list(host: &FxSystemHost, effect_slot: usize, class: usize) -> u16 {
    host.effect_at(effect_slot)
        .map_or(FX_ELEM_HANDLE_NONE, |e| e.first_elem_handle[class])
}

// --- collecting the traces ------------------------------------------------------------

/// Queues the motion of every part of the effect that collides and is alive
/// across the slice.
fn gather_collide_jobs(
    host: &FxSystemHost,
    effect_slot: usize,
    prev_msec: i32,
    msec_now: i32,
    jobs: &mut Vec<PendingCollide>,
) {
    let Some(view) = EffectView::of(host, effect_slot) else {
        return;
    };
    for class in 0..3 {
        let mut handle = first_in_list(host, effect_slot, class);
        while handle != FX_ELEM_HANDLE_NONE {
            if let Some(part) = LivePart::read(host, handle)
                && msec_now >= part.msec_begin
                && msec_now < part.death_msec()
                && elem_uses_collision(part.flags)
            {
                let seed = part.seed(&view);
                let orient = part.orientation(host, &view, seed);
                jobs.push(PendingCollide {
                    handle,
                    def_name: view.def_name.clone(),
                    inputs: MotionInputs::new(&part, &view, orient, seed, prev_msec, msec_now),
                });
            }
            handle = next_in_list(host, handle);
        }
    }
}

// --- applying one effect --------------------------------------------------------------

fn apply_effect(
    host: &mut FxSystemHost,
    slot: usize,
    prev_msec: i32,
    msec_now: i32,
    camera_origin: [f32; 3],
    hooks: &mut Hooks<'_>,
    collide_results: &HashMap<u16, Option<FxElemMotionResult>>,
) {
    let had_trails = host
        .effect_at(slot)
        .is_some_and(|e| e.first_trail_handle != FX_TRAIL_HANDLE_NONE);

    if let Some(view) = EffectView::of(host, slot) {
        for class in 0..3 {
            let mut handle = first_in_list(host, slot, class);
            while handle != FX_ELEM_HANDLE_NONE {
                let next = next_in_list(host, handle);
                if !advance_part(
                    host,
                    &view,
                    handle,
                    prev_msec,
                    msec_now,
                    hooks,
                    collide_results,
                ) {
                    free_elem(host, handle);
                }
                handle = next;
            }
        }
    }

    if had_trails {
        grow_trails(host, slot, prev_msec, msec_now, camera_origin, hooks);
        play_trail_impacts(host, &mut *hooks.child);
    }

    crate::sort::sort_new_elems_in_effect(host, slot, camera_origin);
    if let Some(effect) = host.effect_at_mut(slot) {
        effect.distance += vec3_distance(effect.origin_last, effect.origin);
        effect.msec_last_update = msec_now;
        effect.commit_frame_last_from_now();
    }
}

/// Lays down the samples of the effect's trails: those that follow its
/// parts, then those owed for the distance the effect itself travelled.
fn grow_trails(
    host: &mut FxSystemHost,
    slot: usize,
    prev_msec: i32,
    msec_now: i32,
    camera_origin: [f32; 3],
    hooks: &mut Hooks<'_>,
) {
    update_effect_trails(
        host,
        slot,
        i32::MIN,
        i32::MAX,
        prev_msec,
        msec_now,
        camera_origin,
        &mut *hooks.trail_def,
    );

    let Some(effect) = host.effect_at(slot) else {
        return;
    };
    if effect.first_trail_handle == FX_TRAIL_HANDLE_NONE {
        return;
    }
    let looping = effect.status & FX_STATUS_HAS_PENDING_LOOP_ELEMS != 0;
    let travelled = effect.distance + vec3_distance(effect.origin_last, effect.origin);
    apply_partial_last_trail_spawn_dist(
        host,
        slot,
        prev_msec,
        msec_now,
        travelled,
        looping,
        &mut *hooks.trail_def,
        &mut *hooks.trail_trace,
        &mut *hooks.trail_vel_graphs,
    );
}

/// Plays the impact children that the trails queued while they were moved.
fn play_trail_impacts(host: &mut FxSystemHost, child: &mut ChildHook<'_>) {
    for impact in std::mem::take(&mut host.pending_trail_impacts) {
        play_child(
            host,
            &mut *child,
            FxChildSpawnRequest {
                kind: FxChildKind::Impact,
                parent_def_name: &impact.parent_def_name,
                catalog_index: impact.catalog_index,
                def_index: impact.def_index,
                origin: impact.origin,
                axis: vector_vectors(impact.pre_vel),
                msec: impact.msec,
            },
        );
    }
}

// --- moving one part ------------------------------------------------------------------

/// Moves one part over the slice. Answers whether it lives on.
fn advance_part(
    host: &mut FxSystemHost,
    view: &EffectView,
    handle: u16,
    prev_msec: i32,
    msec_now: i32,
    hooks: &mut Hooks<'_>,
    collide_results: &HashMap<u16, Option<FxElemMotionResult>>,
) -> bool {
    let Some(part) = LivePart::read(host, handle) else {
        return false;
    };
    if msec_now < part.msec_begin {
        return true;
    }
    let seed = part.seed(view);
    let orient = part.orientation(host, view, seed);

    if msec_now >= part.death_msec() {
        play_death_child(host, hooks, view, &part, part.origin, &orient, msec_now);
        return false;
    }

    let inputs = MotionInputs::new(&part, view, orient, seed, prev_msec, msec_now);
    let outcome = match collide_results.get(&handle) {
        Some(precomputed) => *precomputed,
        None => (hooks.motion)(inputs.query(&view.def_name)),
    };
    let Some(result) = outcome else {
        host.gaps.raise(FxGapCause::ElemDefNotFound {
            def_index: part.def_index,
        });
        return true;
    };

    let origin_end = [
        part.origin[0] + result.origin_delta[0],
        part.origin[1] + result.origin_delta[1],
        part.origin[2] + result.origin_delta[2],
    ];
    if let Some(slot) = host.elems.get_mut(part.slot) {
        slot.origin = origin_end;
        slot.base_vel = result.base_vel;
        if let Some(fraction) = result.at_rest_fraction {
            slot.at_rest_fraction = fraction;
        }
    }

    emit_children(
        host, hooks, view, &part, seed, origin_end, prev_msec, msec_now,
    );

    if let Some(impact) = result.spawn_impact {
        play_child(
            host,
            &mut *hooks.child,
            FxChildSpawnRequest {
                kind: FxChildKind::Impact,
                parent_def_name: &view.def_name,
                catalog_index: view.catalog_index,
                def_index: part.def_index,
                origin: impact.origin,
                axis: vector_vectors(impact.pre_vel),
                msec: msec_now,
            },
        );
    }

    if result.remove {
        let died_at = host
            .elems
            .get(part.slot)
            .map_or(part.origin, |slot| slot.origin);
        play_death_child(host, hooks, view, &part, died_at, &orient, msec_now);
        return false;
    }
    true
}

/// Asks the host which children a moving part emits over the slice and plays
/// them, spaced along the way it travelled.
fn emit_children(
    host: &mut FxSystemHost,
    hooks: &mut Hooks<'_>,
    view: &EffectView,
    part: &LivePart,
    seed: u32,
    origin_end: [f32; 3],
    prev_msec: i32,
    msec_now: i32,
) {
    let residual = host.elems.get(part.slot).map_or(0, |e| e.emit_residual);
    let schedule = (hooks.emit)(FxEmitQuery {
        def_name: &view.def_name,
        catalog_index: view.catalog_index,
        def_index: part.def_index,
        origin_begin: part.origin,
        origin_end,
        msec_update_begin: prev_msec,
        msec_update_end: msec_now,
        emit_residual: residual,
        elem_random_seed: seed,
        flags: part.flags,
    });
    let Some(schedule) = schedule else {
        return;
    };
    if let Some(slot) = host.elems.get_mut(part.slot) {
        slot.emit_residual = schedule.new_residual;
    }

    let axis = if part.flags & FX_ELEM_EMIT_ORIENT_AXIS != 0 {
        host.gaps.raise(FxGapCause::EmitOrientQuatNotUnpacked {
            def_index: part.def_index,
        });
        view.now.axis
    } else {
        vector_vectors([
            origin_end[0] - part.origin[0],
            origin_end[1] - part.origin[1],
            origin_end[2] - part.origin[2],
        ])
    };
    for spawn in schedule.spawns() {
        play_child(
            host,
            &mut *hooks.child,
            FxChildSpawnRequest {
                kind: FxChildKind::Emitted,
                parent_def_name: &view.def_name,
                catalog_index: view.catalog_index,
                def_index: part.def_index,
                origin: emit_lerp_origin(part.origin, origin_end, spawn.lerp),
                axis,
                msec: spawn.msec_at_spawn,
            },
        );
    }
}

/// Plays the child effect of a part that has ended, at the place the part
/// ended (given in the part's own space).
fn play_death_child(
    host: &mut FxSystemHost,
    hooks: &mut Hooks<'_>,
    view: &EffectView,
    part: &LivePart,
    ended_at: [f32; 3],
    orient: &FxOrientation,
    msec: i32,
) {
    play_child(
        host,
        &mut *hooks.child,
        FxChildSpawnRequest {
            kind: FxChildKind::Death,
            parent_def_name: &view.def_name,
            catalog_index: view.catalog_index,
            def_index: part.def_index,
            origin: orientation_pos_to_world(orient.origin, orient.axis, ended_at),
            axis: orient.axis,
            msec,
        },
    );
}

/// Hands a child request to the host; a refusal is recorded as a gap.
fn play_child(
    host: &mut FxSystemHost,
    child: &mut ChildHook<'_>,
    request: FxChildSpawnRequest<'_>,
) {
    let (kind, def_index) = (request.kind, request.def_index);
    if !child(host, request) {
        host.gaps.raise(FxGapCause::ChildSpawnRefused {
            child: kind.into(),
            def_index,
        });
    }
}
