//! The per-frame effect update: each live effect is brought up to the
//! current time, its parts move, spawn children and die, and its trails grow.
//!
//! What the host world has to say about motion, emission, child effects and
//! trails comes in as callbacks, so this module never reaches into a
//! simulation of its own.

use std::collections::HashMap;
use std::iter::successors;
use std::ops::{Deref, Range};

use fx_t6::runtime::{
    PARTICLE_EMISSION_FACES_BASIS, EMITTER_REPEAT_QUEUED_BIT, Vec3Spread, EmissionTimetable,
    EffectFacingBasis, EffectFacing, EmitterAttachRefresh, axis_to_quat,
    begin_iterating_over_effects_exclusive, bolt_compose_orientation, bolt_mark_lost,
    elem_random_seed, elem_uses_collision, emit_lerp_origin, end_iterating_over_effects,
    end_iterating_runs_gc, get_orientation, orientation_pos_to_world, unit_quat_to_axis,
    update_effect_bolt, vec3_distance, contact_facing_basis,
};

use crate::recipe::ParticleRecipeSummary;
use crate::particle_bin::{NO_PARTICLE_TICKET, ParticleBinSlot, particle_bin_of_ticket};
use crate::holes::{ChildKind, EffectHoleReason};
use crate::depth_order::order_fresh_particles;
use crate::birth::{release_particle, halt_effect_alone, drop_queued_repeat};
use crate::host::{
    BOLT_NONE, NO_RECIPE_LISTING, LiveEffect, EffectSystemHost, QueuedTrailHit,
};
use crate::ribbon::{
    NO_STREAK_TICKET, StreakBounceContact, apply_partial_last_trail_spawn_dist,
    advance_effect_ribbons,
};

/// A looping effect to top up: its slot, the span since it last advanced and its name.
pub struct LoopTopUpAsk<'a> {
    pub slot: usize,
    pub from_ms: i32,
    pub to_ms: i32,
    pub definition_name: &'a str,
}

/// Tops up a looping effect for the span it asks about; says how long the
/// effect loops, or `None` when it has no recipe.
pub trait LoopTopUp: FnMut(&mut EffectSystemHost, LoopTopUpAsk<'_>) -> Option<i32> {}

impl<F> LoopTopUp for F where F: FnMut(&mut EffectSystemHost, LoopTopUpAsk<'_>) -> Option<i32> {}

/// Everything the host is asked about one part's motion over a time slice.
#[derive(Clone, Copy, Debug)]
pub struct ParticleMoveAsk<'a> {
    /// The owning effect's name; empty when the effect is known by catalog index.
    pub definition_name: &'a str,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub age_from: f32, // fraction of life at the slice start
    pub age_to: f32, // fraction of life at the slice end
    pub lifespan_ms: f32, // whole life of the part, at least 1
    pub step_sec: f32, // slice length in seconds
    pub launch_speed_vec: [f32; 3], // velocity the part carries in
    pub part_seed: u32,
    /// The part's position in its own space.
    pub anchor: [f32; 3],
    pub last_ms: i32, // slice start, game clock
    pub clock_ms_now: i32, // slice end, game clock
    pub window_open_ms: i32,
    pub fx_basis: [[f32; 3]; 3], // the owning effect's axes now
    /// How the part's space sits in the world.
    pub orient: EffectFacing,
    pub settled_share: u8, // 0 while still moving
}

/// A colliding part's motion, worked out ahead of time so the host can trace
/// all of them together before any part is moved.
#[derive(Clone, Debug)]
pub struct QueuedBounceCheck {
    pub fx_ref: u16, // the part's element handle
    owner_name: String,
    /// The question as it was put, with the name left blank; `query` lends
    /// the name back.
    asked: ParticleMoveAsk<'static>,
}

impl QueuedBounceCheck {
    pub fn query(&self) -> ParticleMoveAsk<'_> {
        let mut lent = self.asked;
        lent.definition_name = &self.owner_name;
        lent
    }
}

/// What the host worked out for one part's motion.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParticleMoveOutcome {
    /// How far the part moved, in its own space.
    pub position_shift: [f32; 3],
    pub launch_speed_vec: [f32; 3], // velocity after the slice
    pub died_on_touch: bool, // the part died on touch
    pub settled_share: Option<u8>, // set when the part came to rest in the slice
    pub birth_on_contact: Option<ContactBirth>, // an impact child to play, if any
}

/// Where and how fast a part hit something, for its impact child.
#[derive(Clone, Copy, Debug)]
pub struct ContactBirth {
    pub spot: [f32; 3],
    pub vel_before: [f32; 3], // velocity just before the hit
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffspringSort {
    Impact,
    Death,
    Emitted,
}

impl From<OffspringSort> for ChildKind {
    fn from(kind: OffspringSort) -> Self {
        match kind {
            OffspringSort::Impact => ChildKind::Impact,
            OffspringSort::Death => ChildKind::Death,
            OffspringSort::Emitted => ChildKind::Emitted,
        }
    }
}

/// A child effect a part wants played.
#[derive(Clone, Copy, Debug)]
pub struct OffspringBirthAsk<'a> {
    pub kind: OffspringSort, // which of the part's children this is
    pub parent_recipe: &'a str, // the playing effect's name, blank if catalogued
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub spot_xyz: [f32; 3],
    pub turn_rows: [[f32; 3]; 3],
    pub clock_ms: i32,
}

/// What a world trace found.
#[derive(Clone, Copy, Debug)]
pub struct ParticleRayContact {
    pub hit_frac: f32,
    pub facing: [f32; 3],
    pub startsolid: bool,
    pub fully_embedded: bool, // the whole trace stayed inside solid
}

/// A moving part asking when it should emit.
#[derive(Clone, Copy, Debug)]
pub struct EmissionAsk<'a> {
    pub definition_name: &'a str,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub start_point: [f32; 3], // where the part was
    pub closing_spot: [f32; 3], // where it ended up
    pub refresh_window_open_ms: i32, // slice start
    pub refresh_window_close_ms: i32, // slice end
    pub emission_carry: u8, // fraction of an emission carried over
    pub seed_bits: u32, // seed the emission draws from
    pub mode_bits: i32,
}


/// The host callbacks used while effects are applied, gathered so they travel
/// as one argument.
struct Hooks<'a> {
    motion: &'a mut dyn FnMut(ParticleMoveAsk<'_>) -> Option<ParticleMoveOutcome>,
    emit: &'a mut dyn FnMut(EmissionAsk<'_>) -> Option<EmissionTimetable>,
    child: &'a mut dyn FnMut(&mut EffectSystemHost, OffspringBirthAsk<'_>) -> bool,
    trail_def: &'a mut dyn FnMut(u16, u8) -> Option<ParticleRecipeSummary>,
    trail_trace: &'a mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
    trail_vel_graphs: &'a mut dyn FnMut(u16, u8) -> (Vec<Vec3Spread>, Vec<Vec3Spread>),
}

/// The stretch of time one effect is brought across this frame.
#[derive(Clone, Copy, Debug)]
struct Slice {
    cell: usize,
    prior_ms: i32,
    clock_ms_now: i32,
}

impl Slice {
    fn dt_sec(self) -> f32 {
        self.clock_ms_now.saturating_sub(self.prior_ms).max(0) as f32 * 0.001
    }
}

/// Runs `body` as one pass over the effects: nobody reclaims effects while
/// it runs, and the collection that was asked for meanwhile happens after.
fn while_iterating(host: &mut EffectSystemHost, body: impl FnOnce(&mut EffectSystemHost)) {
    host.walker_total = begin_iterating_over_effects_exclusive(host.walker_total);
    body(host);
    host.walker_total = end_iterating_over_effects(host.walker_total);
    let collect_now = end_iterating_runs_gc(host.walker_total, host.gc_pending);
    if collect_now {
        host.collect_garbage()
    }
}

/// The handles of the effects now in the ring's active window, oldest first.
fn ring_handles(host: &EffectSystemHost) -> Vec<u16> {
    let oldest = host.first_active_effect;
    let span = host.first_new_effect.wrapping_sub(oldest) as u32;
    (0..span)
        .map(|age| host.ring_entry_handle(oldest.wrapping_add(age as i32) as u32))
        .collect()
}

/// Brings every live effect up to `host.clock_ms_now`.
///
/// Effects not attached to a bolt go first, then (unless `skip_bolted`)
/// the bolted ones. Each effect that is due is first stamped and has its
/// pending loop checked; the colliding parts among them are then traced
/// together through `trace_batch`; only then is each effect applied.
pub fn advance_effects(
    host: &mut EffectSystemHost,
    skip_bolted: bool,
    eye_pos: [f32; 3],
    mut loop_life: impl LoopTopUp,
    mut on_motion: impl FnMut(ParticleMoveAsk<'_>) -> Option<ParticleMoveOutcome>,
    mut on_emit: impl FnMut(EmissionAsk<'_>) -> Option<EmissionTimetable>,
    mut on_child_spawn: impl FnMut(&mut EffectSystemHost, OffspringBirthAsk<'_>) -> bool,
    mut on_trail_def: impl FnMut(u16, u8) -> Option<ParticleRecipeSummary>,
    mut on_trail_cast: impl FnMut(
        [f32; 3],
        [f32; 3],
        [f32; 3],
        [f32; 3],
        u32,
    ) -> Option<StreakBounceContact>,
    mut on_trail_vel_graphs: impl FnMut(u16, u8) -> (Vec<Vec3Spread>, Vec<Vec3Spread>),
    trace_batch: impl FnOnce(&[QueuedBounceCheck]) -> HashMap<u16, Option<ParticleMoveOutcome>>,
) {
    while_iterating(host, |host| {
        host.camera_origin = Some(eye_pos);

        let passes: &[bool] = if skip_bolted { &[false] } else { &[false, true] };
        let handles = ring_handles(host);
        let mut due_at = Vec::new();
        let mut batch = Vec::new();
        for &bolted in passes {
            for &handle in &handles {
                let Some(slot) = host.ring_index_of(handle) else {
                    continue;
                };
                if let Some(slice) = begin_effect_update(host, slot, bolted, &mut loop_life) {
                    gather_collide_jobs(host, slice, &mut batch);
                    due_at.push(slice);
                }
            }
        }

        let traced = trace_batch(&batch);

        let mut hooks = Hooks {
            trail_vel_graphs: &mut on_trail_vel_graphs,
            trail_trace: &mut on_trail_cast,
            trail_def: &mut on_trail_def,
            child: &mut on_child_spawn,
            emit: &mut on_emit,
            motion: &mut on_motion,
        };
        due_at.into_iter()
            .for_each(|slice| apply_effect(host, slice, eye_pos, &mut hooks, &traced));
    });
}

// --- deciding who is due -----------------------------------------------------------

/// Stamps an effect as seen this frame and, when it is due, answers with the
/// time slice to update it over. A looping effect past the end of its
/// looping life stops looping here.
fn begin_effect_update(
    host: &mut EffectSystemHost,
    at: usize,
    bolted_pass: bool,
    pending_loop: &mut dyn LoopTopUp,
) -> Option<Slice> {
    let last = claim_for_frame(host, at, bolted_pass)?;
    let slice = Slice {
        cell: at,
        prior_ms: last,
        clock_ms_now: host.clock_ms_now,
    };
    refresh_bolt(host, at);
    end_spent_loop(host, slice, pending_loop);
    Some(slice)
}

/// Marks the effect as visited this frame. Answers when it was last updated
/// if it is in this pass, is still claimed, has not been updated into the
/// future and has not already been visited.
fn claim_for_frame(host: &mut EffectSystemHost, at: usize, bolted_pass: bool) -> Option<i32> {
    let (frame, clock) = (host.frame_tick_no, host.clock_ms_now);
    let effect = host.live_effect_mut(at)?;
    if !effect.in_ring || (effect.tether != BOLT_NONE) != bolted_pass {
        return None;
    }
    let already_seen = std::mem::replace(&mut effect.frame_tick_no, frame) == frame;
    let runnable = effect.holds_refs() && effect.ms_last_tick <= clock;
    (runnable && !already_seen).then_some(effect.ms_last_tick)
}

/// Asks the host how long an effect with pending loop parts keeps looping,
/// and ends the looping when the effect has outlived that.
fn end_spent_loop(
    host: &mut EffectSystemHost,
    slice: Slice,
    pending_loop: &mut dyn LoopTopUp,
) {
    let Slice {
        cell: slot,
        prior_ms: prev_msec,
        clock_ms_now,
    } = slice;
    let looping = host
        .emitter_in_slot(slot)
        .filter(|e| e.state_bits & EMITTER_REPEAT_QUEUED_BIT != 0)
        .map(|e| (catalog_free_name(e), clock_ms_now.wrapping_sub(e.window_open_ms)));
    let Some((name, age)) = looping else {
        return;
    };
    let ask = LoopTopUpAsk { slot, from_ms: prev_msec, to_ms: clock_ms_now, definition_name: &name };
    let allowed = pending_loop(host, ask);
    if allowed.is_some_and(|life| life < age) {
        drop_queued_repeat(host, slot)
    }
}

/// An effect carries its definition's name only when it is not in the
/// catalog; queries from catalogued effects name them by index instead.
fn catalog_free_name(effect: &LiveEffect) -> String {
    match effect.ledger_index {
        NO_RECIPE_LISTING => effect.definition_name.clone(),
        _ => String::new(),
    }
}

/// Moves a bolted effect to where its bone is now, or stops it when the
/// thing it was bolted to is gone.
fn refresh_bolt(host: &mut EffectSystemHost, at: usize) {
    let Some(fx) = host.live_effect_mut(at) else {
        return;
    };
    let pose = fx.tether_pose;
    let (bolt, teleport) = (fx.tether, fx.bolt_teleported);
    let verdict = update_effect_bolt(bolt, fx.tether_bits, teleport, pose.is_some());
    let lost = match (verdict, pose) {
        (EmitterAttachRefresh::Lost, _) => {
            fx.tether_bits = bolt_mark_lost(fx.tether_bits);
            true
        }
        (EmitterAttachRefresh::Refresh, Some((bone_origin, bone_axis))) => {
            let parent = (fx.bolt_parent_rot, fx.tether_spot);
            let (quat, pos) = bolt_compose_orientation(
                parent.0,
                parent.1,
                axis_to_quat(bone_axis),
                bone_origin,
            );
            fx.where_at = pos;
            fx.orient_rows = unit_quat_to_axis(quat);
            false
        }
        _ => false,
    };
    if lost {
        halt_effect_alone(host, at)
    }
}

// --- what an effect and its parts look like to the update --------------------------

/// What the effect owning a run of parts contributes to updating them.
struct EffectView {
    now: EffectFacingBasis,
    when_played: EffectFacingBasis,
    owner_tag: String,
    catalog: u16,
    emitter_dice_start: u16,
}

impl EffectView {
    fn of(host: &EffectSystemHost, slot: usize) -> Option<Self> {
        let e = host.emitter_in_slot(slot)?;
        Some(Self {
            now: e.current_frame(),
            when_played: e.spawn_frame(),
            owner_tag: catalog_free_name(e),
            catalog: e.ledger_index,
            emitter_dice_start: e.dice_start,
        })
    }

    /// A child request on behalf of one of this effect's parts.
    fn child_request(
        &self,
        kind: OffspringSort,
        definition_slot: u8,
        pos: [f32; 3],
        frame: [[f32; 3]; 3],
        msec: i32,
    ) -> OffspringBirthAsk<'_> {
        OffspringBirthAsk {
            parent_recipe: &self.owner_tag,
            ledger_index: self.catalog,
            kind,
            definition_slot,
            spot_xyz: pos,
            turn_rows: frame,
            clock_ms: msec,
        }
    }
}

/// Where a part stands in its life at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Pending,
    Live,
    Over,
}

/// One part's state, copied out so the pool can be changed while it is used.
struct LivePart {
    cell_at: usize,
    elem: ParticleBinSlot,
}

impl Deref for LivePart {
    type Target = ParticleBinSlot;

    fn deref(&self) -> &ParticleBinSlot {
        &self.elem
    }
}

impl LivePart {
    fn read(host: &EffectSystemHost, handle: u16) -> Option<Self> {
        particle_bin_of_ticket(handle).and_then(|slot| {
            let found = host.elems.get(slot)?;
            found.is_taken.then(|| Self {
                cell_at: slot,
                elem: found.clone(),
            })
        })
    }

    /// The milliseconds the part exists for.
    fn lifetime(&self) -> Range<i32> {
        self.window_open_ms..self.window_open_ms.wrapping_add(self.lifespan_window_ms)
    }

    fn phase_at(&self, msec: i32) -> Phase {
        let life = self.lifetime();
        if life.contains(&msec) {
            Phase::Live
        } else if msec < life.start {
            Phase::Pending
        } else {
            Phase::Over
        }
    }

    /// How far through its life the part is at `msec`.
    fn age_at(&self, msec: i32) -> f32 {
        msec.saturating_sub(self.window_open_ms).max(0) as f32 / self.lifespan_ms()
    }

    fn lifespan_ms(&self) -> f32 {
        self.lifespan_window_ms.max(1) as f32
    }

    fn seed(&self, view: &EffectView) -> u32 {
        elem_random_seed(view.emitter_dice_start, self.nth, self.window_open_ms)
    }

    /// The space the part is simulated in, relative to the world.
    fn orientation(&self, view: &EffectView, seed: u32) -> EffectFacing {
        let birth = self.elem.spawn_facing(seed);
        get_orientation(self.bits, &view.now, &view.when_played, Some(birth))
    }

    /// The question the host is asked about this part's motion.
    fn motion_query<'a>(
        &self,
        definition_name: &'a str,
        view: &EffectView,
        slice: Slice,
        orient: EffectFacing,
        rng: u32,
    ) -> ParticleMoveAsk<'a> {
        let me = &self.elem;
        // bo2zm: an element born inside this update only moves from its birth: without this,
        // gravity ran over the whole gap since the effect last updated and a big smoke sheet
        // fell through the camera at the start of Nuketown.
        let slice = Slice {
            prior_ms: slice.prior_ms.max(me.window_open_ms),
            ..slice
        };
        ParticleMoveAsk {
            definition_name,
            ledger_index: view.catalog,
            definition_slot: me.definition_slot,
            orient,
            fx_basis: view.now.axis,
            part_seed: rng,
            age_from: self.age_at(slice.prior_ms),
            age_to: self.age_at(slice.clock_ms_now),
            lifespan_ms: self.lifespan_ms(),
            step_sec: slice.dt_sec(),
            last_ms: slice.prior_ms,
            clock_ms_now: slice.clock_ms_now,
            window_open_ms: me.window_open_ms,
            launch_speed_vec: me.launch_speed_vec,
            anchor: me.at_point,
            settled_share: me.settled_share,
        }
    }
}

/// The part that follows `handle` in its list.
fn next_in_list(host: &EffectSystemHost, handle: u16) -> u16 {
    particle_bin_of_ticket(handle)
        .and_then(|slot| host.elems.get(slot))
        .map_or(NO_PARTICLE_TICKET, |e| e.following_particle_ticket)
}

/// Every part of an effect, class by class, each list from its head.
fn part_handles(host: &EffectSystemHost, effect_slot: usize) -> Vec<u16> {
    let heads = host
        .emitter_in_slot(effect_slot)
        .map_or([NO_PARTICLE_TICKET; 3], |e| e.lead_particle_ticket);
    heads
        .into_iter()
        .flat_map(|head| {
            successors(Some(head), |&at| Some(next_in_list(host, at)))
                .take_while(|&at| at != NO_PARTICLE_TICKET)
        })
        .collect()
}

fn vec_sum(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

fn vec_gap(from: [f32; 3], dest: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| dest[i] - from[i])
}

// --- collecting the traces ------------------------------------------------------------

/// Queues the motion of every part of the effect that collides and is alive
/// across the slice.
fn gather_collide_jobs(host: &EffectSystemHost, slice: Slice, jobs: &mut Vec<QueuedBounceCheck>) {
    let Some(view) = EffectView::of(host, slice.cell) else {
        return;
    };
    for handle in part_handles(host, slice.cell) {
        let Some(part) = LivePart::read(host, handle) else {
            continue;
        };
        if !elem_uses_collision(part.bits) || part.phase_at(slice.clock_ms_now) != Phase::Live {
            continue;
        }
        let rng = part.seed(&view);
        let orient = part.orientation(&view, rng);
        jobs.push(QueuedBounceCheck {
            fx_ref: handle,
            owner_name: view.owner_tag.clone(),
            asked: part.motion_query("", &view, slice, orient, rng),
        });
    }
}

// --- applying one effect --------------------------------------------------------------

fn apply_effect(
    host: &mut EffectSystemHost,
    slice: Slice,
    eye_pos: [f32; 3],
    hooks: &mut Hooks<'_>,
    collided: &HashMap<u16, Option<ParticleMoveOutcome>>,
) {
    let slot = slice.cell;
    let had_trails = trailing_effect(host, slot).is_some();

    if let Some(view) = EffectView::of(host, slot) {
        for handle in part_handles(host, slot) {
            let lives = advance_part(host, &view, handle, slice, hooks, collided);
            if !lives {
                release_particle(host, handle)
            }
        }
    }

    if had_trails {
        grow_trails(host, slice, eye_pos, hooks);
        play_trail_impacts(host, hooks);
    }

    order_fresh_particles(host, slot, eye_pos);
    if let Some(fx) = host.live_effect_mut(slot) {
        fx.ms_last_tick = slice.clock_ms_now;
        fx.range_to_eye += vec3_distance(fx.spot_before, fx.where_at);
        fx.carry_frame_forward();
    }
}

/// The effect at `slot`, when it has at least one trail.
fn trailing_effect(host: &EffectSystemHost, slot: usize) -> Option<&LiveEffect> {
    host.emitter_in_slot(slot)
        .filter(|e| e.first_ribbon_ticket != NO_STREAK_TICKET)
}

/// Lays down the samples of the effect's trails: those that follow its
/// parts, then those owed for the distance the effect itself travelled.
fn grow_trails(
    host: &mut EffectSystemHost,
    slice: Slice,
    eye_pos: [f32; 3],
    hooks: &mut Hooks<'_>,
) {
    let Slice {
        cell: slot,
        prior_ms: prev_msec,
        clock_ms_now,
    } = slice;
    advance_effect_ribbons(
        host,
        slot,
        i32::MIN,
        i32::MAX,
        prev_msec,
        clock_ms_now,
        eye_pos,
        &mut *hooks.trail_def,
    );

    let owed = trailing_effect(host, slot).map(|e| {
        let looping = e.state_bits & EMITTER_REPEAT_QUEUED_BIT != 0;
        (e.range_to_eye + vec3_distance(e.spot_before, e.where_at), looping)
    });
    if let Some((travelled, looping)) = owed {
        apply_partial_last_trail_spawn_dist(
            host,
            slot,
            prev_msec,
            clock_ms_now,
            travelled,
            looping,
            &mut *hooks.trail_def,
            &mut *hooks.trail_trace,
            &mut *hooks.trail_vel_graphs,
        );
    }
}

/// Plays the impact children that the trails queued while they were moved.
fn play_trail_impacts(host: &mut EffectSystemHost, hooks: &mut Hooks<'_>) {
    let queued = std::mem::take(&mut host.pending_trail_impacts);
    for impact in &queued {
        play_child(host, hooks, impact_request(impact));
    }
}

/// The impact child a trail queued, as the host is asked for it.
fn impact_request(impact: &QueuedTrailHit) -> OffspringBirthAsk<'_> {
    let at = impact.place_at;
    OffspringBirthAsk {
        kind: OffspringSort::Impact,
        parent_recipe: &impact.source_def_name,
        ledger_index: impact.ledger_index,
        definition_slot: impact.definition_slot,
        spot_xyz: at,
        turn_rows: contact_facing_basis(impact.prior_vel),
        clock_ms: impact.ms_now,
    }
}

// --- moving one part ------------------------------------------------------------------

/// Moves one part over the slice. Answers whether it lives on.
fn advance_part(
    host: &mut EffectSystemHost,
    view: &EffectView,
    handle: u16,
    slice: Slice,
    hooks: &mut Hooks<'_>,
    collided: &HashMap<u16, Option<ParticleMoveOutcome>>,
) -> bool {
    let Some(part) = LivePart::read(host, handle) else {
        return false;
    };
    let phase = part.phase_at(slice.clock_ms_now);
    if phase == Phase::Pending {
        return true;
    }
    let seed = part.seed(view);
    let orient = part.orientation(view, seed);
    if phase == Phase::Over {
        play_death_child(
            host,
            hooks,
            view,
            &part,
            part.at_point,
            &orient,
            slice.clock_ms_now,
        );
        return false;
    }

    let query = part.motion_query(&view.owner_tag, view, slice, orient, seed);
    let pre_traced = collided.get(&handle).copied();
    let answer = pre_traced.unwrap_or_else(|| (hooks.motion)(query));
    let Some(result) = answer else {
        host.gaps.log_hole(EffectHoleReason::ElemDefAbsent {
            definition_slot: part.definition_slot,
        });
        return true;
    };
    take_motion(host, hooks, view, &part, (seed, &orient), slice, result)
}

/// Puts what the host worked out into the part, then plays the children the
/// move gives rise to. Answers whether the part lives on.
fn take_motion(
    host: &mut EffectSystemHost,
    hooks: &mut Hooks<'_>,
    view: &EffectView,
    part: &LivePart,
    (seed, orient): (u32, &EffectFacing),
    slice: Slice,
    result: ParticleMoveOutcome,
) -> bool {
    let closing_spot = vec_sum(part.at_point, result.position_shift);
    if let Some(dst) = host.elems.get_mut(part.cell_at) {
        dst.settled_share = result.settled_share.unwrap_or(dst.settled_share);
        dst.launch_speed_vec = result.launch_speed_vec;
        dst.at_point = closing_spot;
    }

    emit_children(host, hooks, view, part, seed, closing_spot, slice);

    if let Some(ContactBirth { spot: origin, vel_before: pre_vel }) = result.birth_on_contact {
        let axis = contact_facing_basis(pre_vel);
        let request = view.child_request(
            OffspringSort::Impact,
            part.definition_slot,
            origin,
            axis,
            slice.clock_ms_now,
        );
        play_child(host, hooks, request);
    }

    let removed = result.died_on_touch;
    if removed {
        let ended_at = host.elems.get(part.cell_at).map_or(part.at_point, |e| e.at_point);
        play_death_child(host, hooks, view, part, ended_at, orient, slice.clock_ms_now);
    }
    !removed
}

/// Asks the host which children a moving part emits over the slice and plays
/// them, spaced along the way it travelled.
fn emit_children(
    host: &mut EffectSystemHost,
    hooks: &mut Hooks<'_>,
    view: &EffectView,
    part: &LivePart,
    rng: u32,
    write_target: [f32; 3],
    slice: Slice,
) {
    let carried = host.elems.get(part.cell_at).map_or(0, |e| e.emission_carry);
    let query = EmissionAsk {
        definition_name: &view.owner_tag,
        ledger_index: view.catalog,
        definition_slot: part.definition_slot,
        mode_bits: part.bits,
        start_point: part.at_point,
        closing_spot: write_target,
        refresh_window_open_ms: slice.prior_ms,
        refresh_window_close_ms: slice.clock_ms_now,
        emission_carry: carried,
        seed_bits: rng,
    };
    let Some(schedule) = (hooks.emit)(query) else {
        return;
    };
    if let Some(elem) = host.elems.get_mut(part.cell_at) {
        elem.emission_carry = schedule.fresh_carry;
    }

    let axis = emitted_axis(host, view, part, write_target);
    for birth in schedule.spawns() {
        let origin = emit_lerp_origin(part.at_point, write_target, birth.mix);
        let request = view.child_request(
            OffspringSort::Emitted,
            part.definition_slot,
            origin,
            axis,
            birth.born_at_ms,
        );
        play_child(host, hooks, request);
    }
}

/// The axes an emitted child faces: those of the effect when the part says
/// so, otherwise the way the part travelled.
fn emitted_axis(
    host: &mut EffectSystemHost,
    view: &EffectView,
    part: &LivePart,
    write_target: [f32; 3],
) -> [[f32; 3]; 3] {
    if part.bits & PARTICLE_EMISSION_FACES_BASIS == 0 {
        return contact_facing_basis(vec_gap(part.at_point, write_target));
    }
    host.gaps.log_hole(EffectHoleReason::OrientationUnread {
        definition_slot: part.definition_slot,
    });
    view.now.axis
}

/// Plays the child effect of a part that has ended, at the place the part
/// ended (given in the part's own space).
fn play_death_child(
    host: &mut EffectSystemHost,
    hooks: &mut Hooks<'_>,
    view: &EffectView,
    part: &LivePart,
    ended_at: [f32; 3],
    basis: &EffectFacing,
    msec: i32,
) {
    let (facing, at) = (
        basis.axis,
        orientation_pos_to_world(basis.origin, basis.axis, ended_at),
    );
    let request = view.child_request(OffspringSort::Death, part.definition_slot, at, facing, msec);
    play_child(host, hooks, request);
}

/// Hands a child request to the host; a refusal is recorded as a gap.
fn play_child(host: &mut EffectSystemHost, hooks: &mut Hooks<'_>, request: OffspringBirthAsk<'_>) {
    let (kind, definition_slot) = (request.kind, request.definition_slot);
    let accepted = (hooks.child)(host, request);
    if !accepted {
        let gap = EffectHoleReason::ChildRefused {
            child: kind.into(),
            definition_slot,
        };
        host.gaps.log_hole(gap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UNTURNED_BASIS;
    use crate::particle_bin::{ParticleBinSlot, particle_ticket_of_bin};
    use crate::holes::EffectHole;
    use crate::host::QueuedTrailHit;
    use crate::ribbon::claim_ribbon;
    use fx_t6::runtime::{
        FX_ELEM_RUN_RELATIVE_TO_EFFECT, ELEM_COLLIDES_WITH_WORLD, BIRTH_UNATTACHED, EmissionBirth,
        bolt_pack,
    };
    use std::cell::RefCell;

    const NOW: i32 = 200;

    /// What the hooks were asked, in order.
    #[derive(Default)]
    struct Seen {
        motion: Vec<(String, u16, u8, f32, f32, f32, f32, u32, [f32; 3])>,
        motion_times: Vec<(i32, i32, i32)>,
        motion_orient: Vec<EffectFacing>,
        emits: Vec<(String, u8, ([f32; 3], [f32; 3]), (i32, i32), u8, u32, i32)>,
        kids: Vec<(OffspringSort, String, u16, u8, [f32; 3], [[f32; 3]; 3], i32)>,
        loops: Vec<(usize, i32, i32, String)>,
        jobs: Vec<(u16, u8, String, f32, f32)>,
    }

    /// The host's answers.
    struct Script {
        motion: Option<ParticleMoveOutcome>,
        emit: Option<EmissionTimetable>,
        accept: bool,
        loop_life: Option<i32>,
        collided: HashMap<u16, Option<ParticleMoveOutcome>>,
    }

    impl Default for Script {
        fn default() -> Self {
            Self {
                motion: Some(still()),
                emit: None,
                accept: true,
                loop_life: None,
                collided: HashMap::new(),
            }
        }
    }

    fn still() -> ParticleMoveOutcome {
        ParticleMoveOutcome::default()
    }

    fn moved(delta: [f32; 3]) -> ParticleMoveOutcome {
        let mut result = still();
        result.position_shift = delta;
        result.launch_speed_vec = [4.0, 5.0, 6.0];
        result
    }

    fn schedule(residual: u8, plan: &[(f32, i32)]) -> EmissionTimetable {
        let mut table = [EmissionBirth::default(); 32];
        for (cell, &(lerp, at)) in table.iter_mut().zip(plan) {
            *cell = EmissionBirth {
                mix: lerp,
                born_at_ms: at,
            };
        }
        EmissionTimetable {
            fresh_carry: residual,
            births: plan.len(),
            spawns: table,
        }
    }

    fn kid(kind: OffspringSort, fx_name: &str, catalog: u16, definition_slot: u8) -> (OffspringSort, String, u16, u8, [f32; 3], [[f32; 3]; 3], i32) {
        (kind, fx_name.to_owned(), catalog, definition_slot, [0.0; 3], UNTURNED_BASIS, NOW)
    }

    /// A host with effect "fx/test" (catalog `catalog`) spawned at msec 100,
    /// the clock at `NOW`, the frame stamp moved off the new-effect value.
    fn host_with_effect(catalog: u16) -> EffectSystemHost {
        let mut host = EffectSystemHost::new();
        host.start_effect(
            "fx/test",
            [0.0; 3],
            UNTURNED_BASIS,
            100,
            BIRTH_UNATTACHED,
            false,
            catalog,
        )
        .unwrap();
        host.store_effect_clock(NOW);
        new_frame(&mut host);
        host
    }

    /// Moves the frame stamp on so the effects count as unvisited.
    fn new_frame(host: &mut EffectSystemHost) {
        host.bump_tick_marker();
    }

    /// Chains a part onto the end of its class list in effect 0 and gives
    /// the effect a claim for it. Returns its handle.
    fn add_part(
        host: &mut EffectSystemHost,
        slot: u32,
        lane: usize,
        definition_slot: u8,
        (begin, span): (i32, i32),
        flags: i32,
        pos: [f32; 3],
    ) -> u16 {
        let handle = particle_ticket_of_bin(slot);
        let mut tail = host.emitter_in_slot(0).unwrap().lead_particle_ticket[lane];
        let mut last = NO_PARTICLE_TICKET;
        while let Some(at) = particle_bin_of_ticket(tail) {
            last = tail;
            tail = host.elems[at].following_particle_ticket;
        }
        host.elems[slot as usize] = ParticleBinSlot {
            is_taken: true,
            definition_slot,
            bits: flags,
            window_open_ms: begin,
            lifespan_window_ms: span,
            at_point: pos,
            preceding_particle_ticket: last,
            category_slot: lane as u8,
            nth: 3,
            ..Default::default()
        };
        match particle_bin_of_ticket(last) {
            Some(prev) => host.elems[prev].following_particle_ticket = handle,
            None => host.live_effect_mut(0).unwrap().lead_particle_ticket[lane] = handle,
        }
        host.live_effect_mut(0).unwrap().state_bits += 1;
        handle
    }

    fn run(host: &mut EffectSystemHost, first_pass_only: bool, script: &Script) -> Seen {
        let seen = RefCell::new(Seen::default());
        advance_effects(
            host,
            first_pass_only,
            [7.0, 8.0, 9.0],
            |_, ask| {
                seen.borrow_mut()
                    .loops
                    .push((ask.slot, ask.from_ms, ask.to_ms, ask.definition_name.to_owned()));
                script.loop_life
            },
            |query| {
                let mut s = seen.borrow_mut();
                s.motion.push((
                    query.definition_name.to_owned(),
                    query.ledger_index,
                    query.definition_slot,
                    query.age_from,
                    query.age_to,
                    query.lifespan_ms,
                    query.step_sec,
                    query.part_seed,
                    query.anchor,
                ));
                s.motion_times.push((query.last_ms, query.clock_ms_now, query.window_open_ms));
                s.motion_orient.push(query.orient);
                script.motion
            },
            |query| {
                seen.borrow_mut().emits.push((
                    query.definition_name.to_owned(),
                    query.definition_slot,
                    (query.start_point, query.closing_spot),
                    (query.refresh_window_open_ms, query.refresh_window_close_ms),
                    query.emission_carry,
                    query.seed_bits,
                    query.mode_bits,
                ));
                script.emit
            },
            |_, r| {
                seen.borrow_mut().kids.push((
                    r.kind,
                    r.parent_recipe.to_owned(),
                    r.ledger_index,
                    r.definition_slot,
                    r.spot_xyz,
                    r.turn_rows,
                    r.clock_ms,
                ));
                script.accept
            },
            |_, _| None,
            |_, _, _, _, _| None,
            |_, _| (Vec::new(), Vec::new()),
            |jobs| {
                seen.borrow_mut().jobs.extend(jobs.iter().map(|j| {
                    let query = j.query();
                    (j.fx_ref, query.definition_slot, query.definition_name.to_owned(), query.age_from, query.age_to)
                }));
                script.collided.clone()
            },
        );
        seen.into_inner()
    }

    fn list_of(host: &EffectSystemHost, class: usize) -> Vec<u16> {
        let mut out = Vec::new();
        let mut at = host.emitter_in_slot(0).unwrap().lead_particle_ticket[class];
        while at != NO_PARTICLE_TICKET {
            out.push(at);
            at = next_in_list(host, at);
        }
        out
    }

    #[test]
    fn a_part_that_has_not_begun_is_left_alone() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let early = add_part(&mut host, 0, 0, 1, (300, 100), 0, [1.0; 3]);
        let seen = run(&mut host, false, &Script::default());
        assert!(seen.motion.is_empty() && seen.kids.is_empty() && seen.emits.is_empty());
        assert_eq!(list_of(&host, 0), vec![early]);
        assert_eq!(host.elems[0].at_point, [1.0; 3]);
    }

    #[test]
    fn a_live_part_is_asked_for_motion_and_takes_the_answer() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let handle = add_part(&mut host, 2, 1, 4, (150, 400), 0, [1.0, 1.0, 1.0]);
        let script = Script {
            motion: Some(ParticleMoveOutcome {
                settled_share: Some(9),
                ..moved([1.0, 2.0, 3.0])
            }),
            ..Script::default()
        };
        let seen = run(&mut host, false, &script);

        assert_eq!(seen.motion.len(), 1);
        let (name, catalog, definition_slot, age0, age1, life, dt, seed, origin) = &seen.motion[0];
        assert_eq!(name, "fx/test");
        assert_eq!((*catalog, *definition_slot), (NO_RECIPE_LISTING, 4));
        assert_eq!((*age0, *age1, *life), (0.0, 0.125, 400.0));
        // Born at 150 inside the 100..200 update, it moves only from its birth.
        assert_eq!(*dt, 0.05);
        let emitter_dice_start = host.emitter_in_slot(0).unwrap().dice_start;
        assert_eq!(*seed, elem_random_seed(emitter_dice_start, 3, 150));
        assert_eq!(*origin, [1.0, 1.0, 1.0]);
        assert_eq!(seen.motion_times, vec![(150, 200, 150)]);

        let part = &host.elems[2];
        assert_eq!(part.at_point, [2.0, 3.0, 4.0]);
        assert_eq!(part.launch_speed_vec, [4.0, 5.0, 6.0]);
        assert_eq!(part.settled_share, 9);
        assert_eq!(list_of(&host, 1), vec![handle]);
    }

    #[test]
    fn a_catalogued_effect_is_named_by_index_not_by_name() {
        let mut host = host_with_effect(7);
        add_part(&mut host, 0, 0, 1, (100, 400), 0, [0.0; 3]);
        let seen = run(&mut host, false, &Script::default());
        assert_eq!((seen.motion[0].0.as_str(), seen.motion[0].1), ("", 7));
        assert_eq!(catalog_free_name(host.emitter_in_slot(0).unwrap()), "");
    }

    #[test]
    fn a_zero_life_counts_as_one_millisecond_for_ages() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (200, 0), 0, [0.0; 3]);
        add_part(&mut host, 1, 0, 2, (100, 1000), 0, [0.0; 3]);
        let seen = run(&mut host, false, &Script::default());
        // the first part dies the instant it begins; only the second is moved
        assert_eq!(seen.motion.len(), 1);
        let (_, _, definition_slot, age0, age1, life, dt, _, _) = &seen.motion[0];
        assert_eq!(
            (*definition_slot, *age0, *age1, *life, *dt),
            (2, 0.0, 0.1, 1000.0, 0.1)
        );
    }

    #[test]
    fn a_part_at_the_end_of_its_life_is_freed_with_a_death_child() {
        let mut host = host_with_effect(7);
        host.live_effect_mut(0).unwrap().where_at = [10.0, 0.0, 0.0];
        let run_in_effect = FX_ELEM_RUN_RELATIVE_TO_EFFECT;
        add_part(
            &mut host,
            0,
            0,
            5,
            (100, 100),
            run_in_effect,
            [1.0, 2.0, 3.0],
        );
        let living = add_part(&mut host, 1, 0, 6, (100, 900), 0, [0.0; 3]);
        let before = host.emitter_in_slot(0).unwrap().state_bits;
        let seen = run(&mut host, false, &Script::default());

        let mut death = kid(OffspringSort::Death, "", 7, 5);
        death.4 = [11.0, 2.0, 3.0];
        assert_eq!(seen.kids, vec![death]);
        assert_eq!(list_of(&host, 0), vec![living]);
        assert!(!host.elems[0].is_taken);
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, before - 1);
        assert_eq!(seen.motion.len(), 1);
    }

    #[test]
    fn touching_something_plays_the_impact_then_the_death_at_the_new_place() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 2, 3, (100, 900), 0, [5.0, 5.0, 5.0]);
        let script = Script {
            motion: Some(ParticleMoveOutcome {
                died_on_touch: true,
                birth_on_contact: Some(ContactBirth {
                    spot: [9.0, 9.0, 9.0],
                    vel_before: [0.0, 0.0, -2.0],
                }),
                ..moved([1.0, 0.0, 0.0])
            }),
            ..Script::default()
        };
        let seen = run(&mut host, false, &script);
        let mut impact = kid(OffspringSort::Impact, "fx/test", NO_RECIPE_LISTING, 3);
        impact.4 = [9.0, 9.0, 9.0];
        impact.5 = contact_facing_basis([0.0, 0.0, -2.0]);
        let mut death = kid(OffspringSort::Death, "fx/test", NO_RECIPE_LISTING, 3);
        death.4 = [6.0, 5.0, 5.0];
        assert_eq!(seen.kids, vec![impact, death]);
        assert!(list_of(&host, 2).is_empty());
    }

    #[test]
    fn a_part_with_no_definition_stays_put_and_is_reported() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let handle = add_part(&mut host, 0, 0, 8, (100, 900), 0, [1.0; 3]);
        let script = Script {
            motion: None,
            ..Script::default()
        };
        run(&mut host, false, &script);
        assert_eq!(host.gaps.hit_total(EffectHole::MotionLookup), 1);
        assert_eq!(host.elems[0].at_point, [1.0; 3]);
        assert_eq!(list_of(&host, 0), vec![handle]);
    }

    #[test]
    fn emitters_are_asked_and_their_children_spaced_along_the_path() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 2, (100, 900), 0x10, [0.0, 0.0, 0.0]);
        host.elems[0].emission_carry = 17;
        let script = Script {
            motion: Some(moved([4.0, 0.0, 0.0])),
            emit: Some(schedule(33, &[(0.25, 120), (0.75, 180)])),
            ..Script::default()
        };
        let seen = run(&mut host, false, &script);

        let seed = seen.motion[0].7;
        assert_eq!(
            seen.emits,
            vec![(
                "fx/test".to_owned(),
                2,
                ([0.0; 3], [4.0, 0.0, 0.0]),
                (100, NOW),
                17,
                seed,
                0x10
            )]
        );
        assert_eq!(host.elems[0].emission_carry, 33);
        let axis = contact_facing_basis([4.0, 0.0, 0.0]);
        let mut first = kid(OffspringSort::Emitted, "fx/test", NO_RECIPE_LISTING, 2);
        first.4 = [1.0, 0.0, 0.0];
        first.5 = axis;
        first.6 = 120;
        let mut second = first.clone();
        second.4 = [3.0, 0.0, 0.0];
        second.6 = 180;
        assert_eq!(seen.kids, vec![first, second]);
    }

    #[test]
    fn a_part_that_declines_to_emit_keeps_its_residual() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 2, (100, 900), 0, [0.0; 3]);
        host.elems[0].emission_carry = 17;
        let seen = run(&mut host, false, &Script::default());
        assert_eq!(seen.emits.len(), 1);
        assert!(seen.kids.is_empty());
        assert_eq!(host.elems[0].emission_carry, 17);
    }

    #[test]
    fn orient_axis_emitters_use_the_effect_axis_and_note_the_gap() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let turned = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        host.live_effect_mut(0).unwrap().orient_rows = turned;
        add_part(
            &mut host,
            0,
            0,
            2,
            (100, 900),
            PARTICLE_EMISSION_FACES_BASIS,
            [0.0; 3],
        );
        let script = Script {
            motion: Some(moved([2.0, 0.0, 0.0])),
            emit: Some(schedule(0, &[(0.5, 150)])),
            ..Script::default()
        };
        let seen = run(&mut host, false, &script);
        assert_eq!(seen.kids.len(), 1);
        assert_eq!(seen.kids[0].5, turned);
        assert_eq!(seen.kids[0].4, [1.0, 0.0, 0.0]);
        assert_eq!(host.gaps.hit_total(EffectHole::EmitOrientation), 1);
    }

    #[test]
    fn refused_children_are_reported_by_kind() {
        let refuse = Script {
            accept: false,
            ..Script::default()
        };

        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 2, (100, 100), 0, [0.0; 3]);
        run(&mut host, false, &refuse);
        assert_eq!(host.gaps.hit_total(EffectHole::DeathChild), 1);

        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 2, (100, 900), 0, [0.0; 3]);
        let script = Script {
            accept: false,
            motion: Some(ParticleMoveOutcome {
                birth_on_contact: Some(ContactBirth {
                    spot: [0.0; 3],
                    vel_before: [1.0, 0.0, 0.0],
                }),
                ..moved([1.0, 0.0, 0.0])
            }),
            emit: Some(schedule(1, &[(0.5, 150), (0.9, 190)])),
            ..Script::default()
        };
        run(&mut host, false, &script);
        assert_eq!(host.gaps.hit_total(EffectHole::ImpactChild), 1);
        assert_eq!(host.gaps.hit_total(EffectHole::EmitChild), 2);
        assert_eq!(host.gaps.hit_total(EffectHole::DeathChild), 0);
    }

    #[test]
    fn colliding_parts_are_traced_together_before_any_part_moves() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let hits = add_part(
            &mut host,
            0,
            0,
            1,
            (100, 400),
            ELEM_COLLIDES_WITH_WORLD,
            [0.0; 3],
        );
        let missing = add_part(
            &mut host,
            1,
            0,
            2,
            (100, 400),
            ELEM_COLLIDES_WITH_WORLD,
            [0.0; 3],
        );
        let unlisted = add_part(
            &mut host,
            2,
            0,
            3,
            (100, 400),
            ELEM_COLLIDES_WITH_WORLD,
            [0.0; 3],
        );
        add_part(&mut host, 3, 0, 4, (100, 400), 0, [0.0; 3]);
        add_part(
            &mut host,
            4,
            1,
            5,
            (250, 400),
            ELEM_COLLIDES_WITH_WORLD,
            [0.0; 3],
        );
        add_part(
            &mut host,
            5,
            1,
            6,
            (0, 100),
            ELEM_COLLIDES_WITH_WORLD,
            [0.0; 3],
        );
        // only the first two get an answer from the trace; the third was listed
        // too, but its entry is simply absent from the map
        let script = Script {
            collided: HashMap::from([(hits, Some(moved([0.0, 0.0, 7.0]))), (missing, None)]),
            ..Script::default()
        };
        let seen = run(&mut host, false, &script);

        let want: Vec<(u16, u8, String, f32, f32)> = [(hits, 1), (missing, 2), (unlisted, 3)]
            .iter()
            .map(|&(h, d)| (h, d, "fx/test".to_owned(), 0.0, 0.25))
            .collect();
        assert_eq!(seen.jobs, want);
        // the traced part took its precomputed answer; the unlisted one asked the host
        assert_eq!(host.elems[0].at_point, [0.0, 0.0, 7.0]);
        assert_eq!(host.gaps.hit_total(EffectHole::MotionLookup), 1);
        let asked: Vec<u8> = seen.motion.iter().map(|m| m.2).collect();
        assert_eq!(asked, vec![3, 4]);
    }

    fn two_effects_one_bolted() -> EffectSystemHost {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (100, 900), 0, [0.0; 3]);
        host.start_effect(
            "fx/bolted",
            [0.0; 3],
            UNTURNED_BASIS,
            100,
            5,
            false,
            NO_RECIPE_LISTING,
        )
        .unwrap();
        let bolted = host.live_effect_mut(1).unwrap();
        bolted.lead_particle_ticket[0] = particle_ticket_of_bin(1);
        bolted.state_bits += 1;
        bolted.tether_bits = bolt_pack(5, false, 1);
        bolted.tether_pose = Some(([3.0, 0.0, 0.0], UNTURNED_BASIS));
        host.elems[1] = ParticleBinSlot {
            is_taken: true,
            definition_slot: 2,
            window_open_ms: 100,
            lifespan_window_ms: 900,
            parent_emitter_slot: 1,
            ..Default::default()
        };
        host
    }

    #[test]
    fn bolted_effects_wait_for_the_second_pass_and_can_be_left_out() {
        let mut host = two_effects_one_bolted();
        let seen = run(&mut host, false, &Script::default());
        let names: Vec<&str> = seen.motion.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(names, vec!["fx/test", "fx/bolted"]);
        assert_eq!(host.emitter_in_slot(1).unwrap().where_at, [3.0, 0.0, 0.0]);

        let mut host = two_effects_one_bolted();
        let seen = run(&mut host, true, &Script::default());
        assert_eq!(seen.motion.len(), 1);
        assert_eq!(host.emitter_in_slot(1).unwrap().ms_last_tick, 100);
        assert_eq!(host.emitter_in_slot(1).unwrap().where_at, [0.0; 3]);
    }

    #[test]
    fn a_bolt_that_lost_its_bone_is_marked_lost_and_the_effect_asked_to_stop() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        host.start_effect(
            "fx/lost",
            [0.0; 3],
            UNTURNED_BASIS,
            100,
            5,
            false,
            NO_RECIPE_LISTING,
        )
        .unwrap();
        let packed = bolt_pack(5, false, 1);
        host.live_effect_mut(1).unwrap().tether_bits = packed;
        run(&mut host, false, &Script::default());
        let lost = host.emitter_in_slot(1).map(|e| e.tether_bits);
        assert_ne!(lost, Some(packed));
        assert_eq!(lost, Some(bolt_mark_lost(packed)));
        // its one claim is still held, so the collection leaves it standing
        assert_eq!(host.running_emitters().count(), 2);
        assert_eq!(host.emitter_in_slot(1).unwrap().ms_last_tick, NOW);
    }

    #[test]
    fn an_effect_is_updated_once_per_frame_and_only_when_due() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (100, 900), 0, [0.0; 3]);
        assert_eq!(run(&mut host, false, &Script::default()).motion.len(), 1);
        assert_eq!(host.emitter_in_slot(0).unwrap().ms_last_tick, NOW);
        assert!(run(&mut host, false, &Script::default()).motion.is_empty());
        new_frame(&mut host);
        host.store_effect_clock(NOW + 50);
        let seen = run(&mut host, false, &Script::default());
        assert_eq!(seen.motion_times, vec![(NOW, NOW + 50, 100)]);

        // an effect whose last update is ahead of the clock waits
        new_frame(&mut host);
        host.store_effect_clock(NOW);
        assert!(run(&mut host, false, &Script::default()).motion.is_empty());
    }

    #[test]
    fn effects_with_no_claims_or_no_ring_seat_are_skipped() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (100, 900), 0, [0.0; 3]);
        host.live_effect_mut(0).unwrap().state_bits = 0x4000_0000;
        assert!(run(&mut host, false, &Script::default()).motion.is_empty());
        assert_eq!(host.emitter_in_slot(0).unwrap().ms_last_tick, 100);

        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (100, 900), 0, [0.0; 3]);
        host.live_effect_mut(0).unwrap().in_ring = false;
        assert!(run(&mut host, false, &Script::default()).motion.is_empty());
    }

    #[test]
    fn a_pending_loop_past_its_looping_life_is_stopped() {
        let looping = |life: Option<i32>| {
            let mut host = host_with_effect(NO_RECIPE_LISTING);
            host.live_effect_mut(0).unwrap().state_bits |= EMITTER_REPEAT_QUEUED_BIT;
            host.live_effect_mut(0).unwrap().state_bits += 1;
            let script = Script {
                loop_life: life,
                ..Script::default()
            };
            let seen = run(&mut host, false, &script);
            (seen, host.emitter_in_slot(0).unwrap().state_bits)
        };
        let base = 0x4000_0002 | EMITTER_REPEAT_QUEUED_BIT;

        let (seen, status) = looping(Some(99));
        assert_eq!(seen.loops, vec![(0, 100, NOW, "fx/test".to_owned())]);
        assert_eq!(status, 0x4000_0001);

        assert_eq!(looping(Some(100)).1, base);
        assert_eq!(looping(Some(5000)).1, base);
        assert_eq!(looping(None).1, base);
    }

    #[test]
    fn the_effect_is_left_where_it_ended_up() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        {
            let e = host.live_effect_mut(0).unwrap();
            e.where_at = [3.0, 4.0, 0.0];
            e.range_to_eye = 1.5;
        }
        let seen = run(&mut host, false, &Script::default());
        assert!(seen.motion.is_empty());
        let e = host.emitter_in_slot(0).unwrap();
        assert_eq!(e.range_to_eye, 6.5);
        assert_eq!(e.spot_before, [3.0, 4.0, 0.0]);
        assert_eq!(e.ms_last_tick, NOW);
        assert_eq!(host.camera_origin, Some([7.0, 8.0, 9.0]));
        assert_eq!(host.walker_total, 0);
    }

    #[test]
    fn the_last_part_dying_lets_the_collection_reclaim_the_effect() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        add_part(&mut host, 0, 0, 1, (100, 100), 0, [0.0; 3]);
        host.live_effect_mut(0).unwrap().state_bits = 0x4000_0001;
        run(&mut host, false, &Script::default());
        assert!(!host.gc_pending);
        assert_eq!(host.running_emitters().count(), 0);
        assert_eq!(host.ring_fill(), 0);
    }

    fn queued_impact(name: &str, catalog: u16, definition_slot: u8, msec: i32) -> QueuedTrailHit {
        QueuedTrailHit {
            source_def_name: name.to_owned(),
            ledger_index: catalog,
            definition_slot,
            place_at: [1.0, 2.0, 3.0],
            prior_vel: [0.0, 3.0, 0.0],
            ms_now: msec,
        }
    }

    #[test]
    fn trail_impacts_queued_by_the_trails_are_played_after_the_parts() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        let trail = claim_ribbon(&mut host, 0).unwrap();
        host.live_effect_mut(0).unwrap().first_ribbon_ticket = trail;
        let hit = queued_impact("fx/hit", 12, 6, 150);
        host.pending_trail_impacts.push(hit);
        let seen = run(&mut host, false, &Script::default());
        let mut want = kid(OffspringSort::Impact, "fx/hit", 12, 6);
        want.4 = [1.0, 2.0, 3.0];
        want.5 = contact_facing_basis([0.0, 3.0, 0.0]);
        want.6 = 150;
        assert_eq!(seen.kids, vec![want]);
        assert!(host.pending_trail_impacts.is_empty());
        assert_eq!(host.gaps.hit_total(EffectHole::TrailDefMiss), 1);

        // without trails the queue is not looked at
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        host.pending_trail_impacts.push(queued_impact("", 0, 0, 0));
        assert!(run(&mut host, false, &Script::default()).kids.is_empty());
        assert_eq!(host.pending_trail_impacts.len(), 1);
    }

    #[test]
    fn a_relative_part_reports_the_orientation_it_runs_in() {
        let mut host = host_with_effect(NO_RECIPE_LISTING);
        host.live_effect_mut(0).unwrap().where_at = [10.0, 0.0, 0.0];
        let run_in_effect = FX_ELEM_RUN_RELATIVE_TO_EFFECT;
        add_part(&mut host, 0, 0, 1, (100, 900), run_in_effect, [0.0; 3]);
        add_part(&mut host, 1, 0, 2, (100, 900), 0, [0.0; 3]);
        let seen = run(&mut host, false, &Script::default());
        assert_eq!(seen.motion_orient[0].origin, [10.0, 0.0, 0.0]);
        assert_eq!(seen.motion_orient[1].origin, [0.0; 3]);
        assert_eq!(seen.motion_orient[1].axis, UNTURNED_BASIS);
    }
}
