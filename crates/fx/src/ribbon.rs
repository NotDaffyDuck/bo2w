//! Ribbon trails. An effect that travels leaves a chain of samples behind it;
//! this file keeps the pools those chains live in, lays new samples as the
//! effect moves, and carries the older ones forward in time.

use std::ops::{ControlFlow, Range};

use fx_t6::runtime::{
    DICE_LANE_WAIT, DICE_LANE_LIFESPAN, EMITTER_REPEAT_QUEUED_BIT,
    STREAK_JOINT_BANK_LIMIT, STREAK_JOINT_RECORD_BYTES, STREAK_BANK_LIMIT,
    STREAK_RECORD_BYTES, ParticleBounceSlice, Vec3Spread, EffectFacingBasis, FacingBirthKnobs,
    EffectFacing, StreakBreak, collide_substep_schedule, collision_reflect_base_vel_delta,
    compress_basis_from_axis, effect_orient_arc, elem_dies_on_touch, elem_gravity_accel_z_sampled,
    elem_skips_position_update, elem_update_has_velocity_graph, elem_uses_collision,
    elem_uses_vel_local, elem_uses_vel_world, get_orientation, get_velocity_at_time,
    impact_child_speed_allows, integrate_velocity_graph, orientation_pos_from_world,
    orientation_pos_to_world, random_table_u16, sample_life_span_msec, sample_reflection_factor,
    spawn_origin_world, status_is_unique_done, trace_mask, trail_elem_base_vel_z_pack,
    trail_piece_ticket_for_slot, trail_elem_keep, trail_elem_norm_ages, trail_ticket_for_slot,
    trail_random_seed, trail_split_interpolant_msec, trail_split_interpolant_t,
    trail_split_lerp_axis, trail_split_lerp_origin, trail_split_skips_update, trail_split_window,
    vec3_distance, vec3_length_sq,
};

use crate::recipe::ParticleRecipeSummary;
use crate::holes::EffectHoleReason;
use crate::host::{NO_RECIPE_LISTING, LiveEffect, EffectSystemHost, QueuedTrailHit};

/// Ends a chain of trails or of trail samples.
pub const NO_STREAK_TICKET: u16 = u16::MAX;

/// The end-of-chain marker, under the name this file reads best with.
const NO_LINK: u16 = NO_STREAK_TICKET;

/// How many times one sub-step of a collision sweep may be cut short by a
/// surface before the sample is left where the last cut put it.
const MAX_BOUNCES_PER_STEP: usize = 8;

/// What the caller's world probe reports when a sample's sweep meets something.
#[derive(Clone, Copy, Debug)]
pub struct StreakBounceContact {
    pub hit_frac: f32,
    pub hit_facing: [f32; 3],
    pub startsolid: bool,
    pub fully_embedded: bool,
}

/// One trail: the chain of samples laid so far.
#[derive(Clone, Debug)]
pub(crate) struct RibbonCell {
    pub(crate) taken: bool,
    /// Which of the effect's parts this trail draws.
    pub(crate) definition_slot: i8,
    /// Counts samples laid; seeds each one's randomness.
    pub(crate) sequence: i8,
    /// Spacing carried over from the last update, in split units.
    pub(crate) split_leftover: f32,
    pub(crate) lead_particle_ticket: u16,
    pub(crate) last_elem_handle: u16,
    /// The next trail of the same effect (or the next free slot, when free).
    pub(crate) next_trail_handle: u16,
}

impl RibbonCell {
    /// A slot nobody uses, naming `link` as the slot after it.
    fn unused(link: u16) -> Self {
        Self {
            next_trail_handle: link,
            ..Self::default()
        }
    }
}

impl std::default::Default for RibbonCell {
    fn default() -> Self {
        Self {
            taken: false,
            definition_slot: i8::default(),
            sequence: i8::default(),
            split_leftover: f32::default(),
            lead_particle_ticket: NO_LINK,
            last_elem_handle: NO_LINK,
            next_trail_handle: NO_LINK,
        }
    }
}

/// One sample along a trail.
#[derive(Clone, Debug)]
pub(crate) struct RibbonPointCell {
    pub(crate) is_taken: bool,
    pub(crate) sequence: u8,
    pub(crate) base_vel_z: i16,
    pub(crate) window_open_ms: i32,
    pub(crate) spot_xyz: [f32; 3],
    pub(crate) basis: [i8; 6],
    /// How far the effect had travelled when this sample was laid.
    pub(crate) spawn_dist: f32,
    /// The next sample along the trail (or the next free slot, when free).
    pub(crate) next_trail_elem_handle: u16,
}

impl RibbonPointCell {
    /// Whether the sample was laid by `now`; one laid in the future is not there yet.
    pub(crate) fn laid_by(&self, now: i32) -> bool {
        self.window_open_ms <= now
    }

    /// A slot nobody uses, naming `link` as the slot after it.
    fn unused(link: u16) -> Self {
        Self {
            next_trail_elem_handle: link,
            ..Self::default()
        }
    }
}

impl std::default::Default for RibbonPointCell {
    fn default() -> Self {
        Self {
            is_taken: false,
            sequence: u8::default(),
            base_vel_z: i16::default(),
            window_open_ms: i32::default(),
            spot_xyz: [0.0; 3],
            basis: Default::default(),
            spawn_dist: f32::default(),
            next_trail_elem_handle: NO_LINK,
        }
    }
}

/// What a split window leaves to carry and how many cuts it asks for.
fn spare_and_cuts(window: StreakBreak) -> (f32, i32) {
    match window {
        StreakBreak::Interpolate { leftover: spare, extra: cuts } => (spare, cuts),
        StreakBreak::Hold { leftover: spare } => (spare, 0),
    }
}

// --- handles, pools, chains -------------------------------------------------

/// A handle counts 4-byte units from the start of its pool. It names a slot
/// only when that lands exactly on a slot and inside the pool.
fn slot_from_handle(handle: u16, step_len: u32, capacity: usize) -> Option<usize> {
    (handle != NO_LINK)
        .then(|| u32::from(handle) * 4)
        .filter(|bytes| bytes % step_len == 0)
        .map(|bytes| (bytes / step_len) as usize)
        .filter(|&slot| slot < capacity)
}

pub fn ribbon_index_of(h: u16) -> Option<usize> {
    slot_from_handle(
        h,
        STREAK_RECORD_BYTES as u32,
        STREAK_BANK_LIMIT as usize,
    )
}

pub fn trail_elem_slot_for_handle(h: u16) -> Option<usize> {
    slot_from_handle(
        h,
        STREAK_JOINT_RECORD_BYTES as u32,
        STREAK_JOINT_BANK_LIMIT as usize,
    )
}

/// The slot of an in-use trail.
pub(crate) fn live_trail(host: &EffectSystemHost, ticket: u16) -> Option<usize> {
    let slot = ribbon_index_of(ticket)?;
    host.trails.get(slot)?.taken.then_some(slot)
}

/// The slot of an in-use sample.
pub(crate) fn live_sample(host: &EffectSystemHost, ticket: u16) -> Option<usize> {
    let slot = trail_elem_slot_for_handle(ticket)?;
    host.trail_elems.get(slot)?.is_taken.then_some(slot)
}

/// Unused slots are chained by index through the link each carries; `head`
/// is the first of them. Pops the first one, or reports the pool as empty.
fn pop_unused<T>(slots: &[T], head: &mut Option<usize>, link_of: fn(&T) -> u16) -> Option<usize> {
    let at = (*head)?;
    let Some(entry) = slots.get(at) else {
        *head = None;
        return None;
    };
    *head = Some(link_of(entry))
        .filter(|&link| link != NO_STREAK_TICKET)
        .map(usize::from);
    Some(at)
}

/// Overwrites `slot` with the unused slot `fresh` builds and makes it the
/// first of the chain.
fn push_unused<T>(slots: &mut [T], head: &mut Option<usize>, at: usize, fresh: fn(u16) -> T) {
    let behind = head.map_or(NO_STREAK_TICKET, |first| first as u16);
    slots[at] = fresh(behind);
    *head = Some(at);
}

/// Takes a trail off the free list for the part at `definition_slot`.
pub(crate) fn claim_ribbon(host: &mut EffectSystemHost, definition_slot: i8) -> Option<u16> {
    let slot = pop_unused(&host.trails, &mut host.ribbon_free_head, |t| {
        t.next_trail_handle
    })?;
    host.trails[slot] = RibbonCell {
        taken: true,
        definition_slot,
        ..RibbonCell::unused(NO_LINK)
    };
    Some(trail_ticket_for_slot(slot as u32))
}

/// Puts a trail slot back on the free list.
pub(crate) fn release_trail(host: &mut EffectSystemHost, at: usize) {
    push_unused(
        &mut host.trails,
        &mut host.ribbon_free_head,
        at,
        RibbonCell::unused,
    );
}

/// Takes a sample slot off the free list.
pub(crate) fn claim_ribbon_point(host: &mut EffectSystemHost) -> Option<usize> {
    pop_unused(&host.trail_elems, &mut host.ribbon_head_free, |e| {
        e.next_trail_elem_handle
    })
}

/// Puts a sample slot back on the free list.
pub(crate) fn release_trail_elem(host: &mut EffectSystemHost, at: usize) {
    push_unused(
        &mut host.trail_elems,
        &mut host.ribbon_head_free,
        at,
        RibbonPointCell::unused,
    );
}

/// The sample after `handle` along its trail; None when `handle` names no
/// sample slot.
fn link_after(host: &EffectSystemHost, handle: u16) -> Option<u16> {
    let at = trail_elem_slot_for_handle(handle)?;
    host.trail_elems
        .get(at)
        .map(|sample| sample.next_trail_elem_handle)
}

/// The in-use samples from `first` on, in chain order, up to the first one
/// that is not in use.
fn live_chain(host: &EffectSystemHost, first: u16) -> Vec<u16> {
    std::iter::successors(Some(first), |&at| {
        live_sample(host, at).map(|slot| host.trail_elems[slot].next_trail_elem_handle)
    })
    .map_while(|at| live_sample(host, at).map(|_| at))
    .collect()
}

/// The in-use trails from `first` on, as (handle, slot), up to the first
/// that is not in use.
fn live_trails(host: &EffectSystemHost, first: u16) -> Vec<(u16, usize)> {
    std::iter::successors(Some(first), |&at| {
        live_trail(host, at).map(|slot| host.trails[slot].next_trail_handle)
    })
    .map_while(|at| live_trail(host, at).map(|slot| (at, slot)))
    .collect()
}

// --- the effect's claim on its samples ---------------------------------------

/// Every sample on a trail counts as one claim on its effect.
fn take_claim(host: &mut EffectSystemHost, fx_index: usize) {
    if let Some(owner) = host.live_effect_mut(fx_index) {
        owner.state_bits = owner.state_bits.wrapping_add(1);
    }
}

/// Gives a claim back; when it was the effect's last, the effect is queued
/// for collection.
fn give_up_claim(host: &mut EffectSystemHost, fx_index: usize) {
    let released = host.live_effect_mut(fx_index).map(|owner| {
        let last = status_is_unique_done(owner.state_bits);
        owner.state_bits = owner.state_bits.wrapping_sub(1);
        (last, owner.self_ticket)
    });
    if let Some((true, owner_handle)) = released {
        host.drop_effect_ref(owner_handle);
    }
}

// --- taking samples off a trail -----------------------------------------------

/// Where the link that points at a sample is kept: in the trail itself when
/// the sample is first, otherwise in the sample ahead of it.
#[derive(Clone, Copy)]
enum Anchor {
    TrailStart,
    Sample(usize),
}

impl Anchor {
    fn of(before: u16) -> Option<Self> {
        if before == NO_STREAK_TICKET {
            return Some(Self::TrailStart);
        }
        trail_elem_slot_for_handle(before).map(Self::Sample)
    }

    fn target(self, host: &EffectSystemHost, trail_idx: usize) -> Option<u16> {
        match self {
            Self::TrailStart => Some(host.trails[trail_idx].lead_particle_ticket),
            Self::Sample(slot) => host.trail_elems.get(slot).map(|s| s.next_trail_elem_handle),
        }
    }

    fn retarget(self, host: &mut EffectSystemHost, trail_idx: usize, to: u16) {
        match self {
            Self::TrailStart => host.trails[trail_idx].lead_particle_ticket = to,
            Self::Sample(slot) => host.trail_elems[slot].next_trail_elem_handle = to,
        }
    }
}

/// Removes one sample from its trail. `before` is the sample ahead of it, or
/// `NO_STREAK_TICKET` when it is the first; a link that does not match is
/// left alone. The effect gives up the sample's claim on it.
fn drop_sample(
    host: &mut EffectSystemHost,
    fx_index: usize,
    trail_idx: usize,
    sample: u16,
    before: u16,
) {
    let Some(anchor) = Anchor::of(before) else {
        return;
    };
    if anchor.target(host, trail_idx) != Some(sample) {
        return;
    }
    let Some(slot) = live_sample(host, sample) else {
        return;
    };

    let behind = host.trail_elems[slot].next_trail_elem_handle;
    let trail = &mut host.trails[trail_idx];
    if trail.last_elem_handle == sample {
        trail.last_elem_handle = before;
    }
    anchor.retarget(host, trail_idx, behind);
    release_trail_elem(host, slot);
    give_up_claim(host, fx_index);
}

// --- shared sampling of a part's randomness --------------------------------------

/// How long a sample of `def` lives, given its seed.
fn life_for(def: &ParticleRecipeSummary, seed: u32) -> i32 {
    let roll = random_table_u16(seed, DICE_LANE_LIFESPAN);
    sample_life_span_msec(def.lifespan_floor, def.lifespan_spread, roll)
}

/// Where a sample of `def` sits when its effect is at `origin`/`axis`.
fn sample_spawn_origin(
    def: &ParticleRecipeSummary,
    pos: [f32; 3],
    frame_axes: [[f32; 3]; 3],
    seed: u32,
) -> [f32; 3] {
    spawn_origin_world(
        pos,
        frame_axes,
        def.birth_point,
        def.bit_set,
        def.birth_ring_reach_floor,
        def.birth_ring_reach_spread,
        def.birth_lift_floor,
        def.birth_lift_spread,
        seed,
    )
}

// --- carrying samples through time ----------------------------------------------

/// The part's spawn offsets as the orientation lookup wants them.
fn orient_params(def: &ParticleRecipeSummary, seed: u32) -> FacingBirthKnobs {
    let ParticleRecipeSummary {
        birth_point: origin_kind,
        birth_ring_reach_floor: radius_lo,
        birth_ring_reach_spread: radius_span,
        birth_lift_floor: height_lo,
        birth_lift_spread: height_span,
        ..
    } = *def;
    FacingBirthKnobs {
        seed,
        birth_lift: [height_lo, height_span],
        birth_ring_reach: [radius_lo, radius_span],
        birth_point: origin_kind,
    }
}

/// A sample's place and speed while it is being moved.
#[derive(Clone, Copy)]
struct Pose {
    spot: [f32; 3],
    vz: f32,
}

/// What stays fixed about one sample while it is carried over a time slice.
struct SampleKinematics<'a> {
    flags: i32,
    lifespan_ms: f32,
    seed: u32,
    fall_pull: f32,
    frame: EffectFacing,
    vel_local: &'a [Vec3Spread],
    vel_world: &'a [Vec3Spread],
}

fn displaced(p: [f32; 3], by: [f32; 3]) -> [f32; 3] {
    [p[0] + by[0], p[1] + by[1], p[2] + by[2]]
}

impl SampleKinematics<'_> {
    fn to_world(&self, p: [f32; 3]) -> [f32; 3] {
        orientation_pos_to_world(self.frame.origin, self.frame.axis, p)
    }

    fn to_frame(&self, p: [f32; 3]) -> [f32; 3] {
        orientation_pos_from_world(self.frame.origin, self.frame.axis, p)
    }

    /// How far a velocity graph carries a sample between two ages; None when
    /// the graph is not in play for this sample.
    fn graph_drift(
        &self,
        graph: &[Vec3Spread],
        in_play: bool,
        (age0, age1): (f32, f32),
    ) -> Option<[f32; 3]> {
        (in_play && graph.len() >= 2)
            .then(|| integrate_velocity_graph(graph, age0, age1, self.lifespan_ms, self.seed))
    }

    /// Where the velocity graphs alone carry `origin` between two ages: the
    /// local graph acts in the effect's frame, the world graph outside it.
    fn graph_shift(&self, pos: [f32; 3], ages: (f32, f32)) -> [f32; 3] {
        let local = self.graph_drift(self.vel_local, elem_uses_vel_local(self.flags), ages);
        let in_frame = local.map_or(pos, |d| displaced(pos, d));

        let world = self.to_world(in_frame);
        let world_drift = self.graph_drift(self.vel_world, elem_uses_vel_world(self.flags), ages);
        self.to_frame(world_drift.map_or(world, |d| displaced(world, d)))
    }

    /// The graphs, then the vertical base velocity and gravity over `dt`
    /// seconds.
    fn advance(&self, from: Pose, dt: f32, ages: (f32, f32)) -> Pose {
        let mut world = self.to_world(self.graph_shift(from.spot, ages));
        world[2] += from.vz * dt;
        world[2] -= self.fall_pull * dt * dt * 0.5;
        Pose {
            spot: self.to_frame(world),
            vz: from.vz - self.fall_pull * dt,
        }
    }
}

/// The time slice and pose context shared by every sample of one trail.
struct TrailStep<'a> {
    elem: ParticleRecipeSummary,
    definition_slot: u8,
    prev_ms: i32,
    clock_ms_now: i32,
    now: &'a EffectFacingBasis,
    alt: &'a EffectFacingBasis,
    fx_seed: u16,
    vel_local: &'a [Vec3Spread],
    vel_world: &'a [Vec3Spread],
}

/// Where a sample sits: the effect and trail it belongs to, and the sample
/// ahead of it.
#[derive(Clone, Copy)]
struct SamplePlace {
    fx_cell: usize,
    trail_idx: usize,
    handle: u16,
    before: u16,
}

/// Carries one sample from `step.prev_msec` to `step.clock_ms_now`.
fn move_sample(
    host: &mut EffectSystemHost,
    place: SamplePlace,
    step: &TrailStep<'_>,
    trace: &mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
) {
    let recipe = step.elem;
    if elem_skips_position_update(recipe.part_shape, recipe.bit_set) {
        return;
    }
    let Some(slot) = live_sample(host, place.handle) else {
        return;
    };

    let (born, start, seed) = {
        let sample = &host.trail_elems[slot];
        let start = Pose {
            spot: sample.spot_xyz,
            vz: f32::from(sample.base_vel_z),
        };
        let seed = trail_random_seed(step.fx_seed, sample.sequence as i8);
        (sample.window_open_ms, start, seed)
    };
    let life = life_for(&recipe, seed);
    let birth = orient_params(&recipe, seed);
    let kin = SampleKinematics {
        flags: recipe.bit_set,
        lifespan_ms: life as f32,
        seed,
        fall_pull: elem_gravity_accel_z_sampled(recipe.fall_pull_floor, recipe.fall_pull_spread, seed),
        frame: get_orientation(recipe.bit_set, step.now, step.alt, Some(birth)),
        vel_local: step.vel_local,
        vel_world: step.vel_world,
    };
    // Nothing moves before it exists.
    let from = step.prev_ms.max(born);
    let ages = trail_elem_norm_ages(step.prev_ms, step.clock_ms_now, born, life);

    // The new origin, and the new base velocity when the motion changes it.
    let outcome: Option<([f32; 3], Option<f32>)> = if elem_uses_collision(recipe.bit_set) {
        let probe = Collider {
            place,
            step,
            kin: &kin,
            born,
            life,
        };
        probe
            .sweep(host, start, from, trace)
            .map(|end| (end.spot, Some(end.vz)))
    } else if elem_update_has_velocity_graph(recipe.bit_set) {
        let dt = (step.clock_ms_now.wrapping_sub(from) as f32) * 0.001;
        let end = kin.advance(start, dt, ages);
        Some((end.spot, Some(end.vz)))
    } else {
        Some((kin.graph_shift(start.spot, ages), None))
    };

    if let Some((pos, vz)) = outcome {
        let sample = &mut host.trail_elems[slot];
        sample.spot_xyz = pos;
        if let Some(vz) = vz {
            sample.base_vel_z = trail_elem_base_vel_z_pack(vz);
        }
    }
}

/// Everything a colliding sample's sweep needs besides the host and the
/// world probe.
struct Collider<'a> {
    place: SamplePlace,
    step: &'a TrailStep<'a>,
    kin: &'a SampleKinematics<'a>,
    born: i32,
    life: i32,
}

impl Collider<'_> {
    /// Moves the sample through the slice, one sub-step after another,
    /// bouncing off what it meets. `None` means nothing was done: no world
    /// was supplied, or the sample died on touching something.
    fn sweep(
        &self,
        host: &mut EffectSystemHost,
        start: Pose,
        from: i32,
        trace: &mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
    ) -> Option<Pose> {
        collide_substep_schedule(from, self.step.clock_ms_now).try_fold(start, |pose, window| {
            self.substep(host, pose, window, trace)
        })
    }

    /// One sub-step: free flight, cut short by at most a few surfaces.
    fn substep(
        &self,
        host: &mut EffectSystemHost,
        pose: Pose,
        window: ParticleBounceSlice,
        trace: &mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
    ) -> Option<Pose> {
        let run = (0..MAX_BOUNCES_PER_STEP).try_fold((pose, window.window_begin_ms), |(pose, t0), _| {
            self.bounce(host, pose, t0, window, trace)
        });
        match run {
            ControlFlow::Break(settled) => settled,
            ControlFlow::Continue((pose, _)) => Some(pose),
        }
    }

    /// Flies from `t0` to the end of `window`. Breaks with the final pose
    /// when the flight settles, or with None when the sweep is abandoned;
    /// otherwise continues from the surface that was hit.
    fn bounce(
        &self,
        host: &mut EffectSystemHost,
        pose: Pose,
        t0: i32,
        window: ParticleBounceSlice,
        trace: &mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
    ) -> ControlFlow<Option<Pose>, (Pose, i32)> {
        let (def, kin) = (&self.step.elem, self.kin);
        let t_end = window.window_close_ms;
        if t_end <= t0 {
            return ControlFlow::Break(Some(pose));
        }

        let dt = 0.001 * (t_end.wrapping_sub(t0) as f32);
        let ages = trail_elem_norm_ages(t0, t_end, self.born, self.life);
        let free = kin.advance(pose, dt, ages);
        let (from_w, to_w) = (kin.to_world(pose.spot), kin.to_world(free.spot));

        let mask = trace_mask(def.collide_with_pickups != 0);
        let Some(contact) = trace(from_w, to_w, def.bounce_box_bottom, def.bounce_box_top, mask) else {
            host.gaps.log_hole(EffectHoleReason::CollideWithoutWorld {
                definition_slot: self.step.definition_slot,
            });
            return ControlFlow::Break(None);
        };
        if contact.startsolid | contact.fully_embedded {
            return ControlFlow::Break(Some(Pose { vz: 0.0, ..pose }));
        }
        if 1.0 <= contact.hit_frac {
            return ControlFlow::Break(Some(free));
        }

        let hit_w = std::array::from_fn(|i| from_w[i] + (to_w[i] - from_w[i]) * contact.hit_frac);
        let slice_ms = t_end.saturating_sub(t0).max(0) as f32;
        let age_ms = t0.saturating_sub(self.born).max(0) as f32 + slice_ms * contact.hit_frac;
        self.note_impact(host, hit_w, [0.0, 0.0, free.vz], age_ms);

        if elem_dies_on_touch(def.bit_set) {
            let place = self.place;
            drop_sample(
                host,
                place.fx_cell,
                place.trail_idx,
                place.handle,
                place.before,
            );
            return ControlFlow::Break(None);
        }

        let refl = sample_reflection_factor(def.mirror_floor, def.mirror_spread, kin.seed);
        let kick = collision_reflect_base_vel_delta([0.0, 0.0, free.vz], contact.hit_facing, refl);
        let bounced = Pose {
            spot: kin.to_frame(hit_w),
            vz: free.vz + kick[2],
        };
        // Always make some headway, so a surface at the very start cannot
        // hold the sample in place.
        let resume =
            (t0 + (slice_ms * contact.hit_frac) as i32).max(window.window_begin_ms.saturating_add(1));
        ControlFlow::Continue((bounced, resume))
    }

    /// Queues the child effect for a hit, when the part has one and the
    /// sample was going fast enough to set it off.
    fn note_impact(&self, host: &mut EffectSystemHost, at: [f32; 3], launch_speed_vec: [f32; 3], age_ms: f32) {
        let (def, kin) = (&self.step.elem, self.kin);
        if !def.spawns_on_contact {
            return;
        }
        let pre_vel = get_velocity_at_time(
            def.bit_set,
            launch_speed_vec,
            age_ms,
            kin.lifespan_ms,
            kin.vel_local,
            kin.vel_world,
            self.step.now.axis,
            kin.seed,
        );
        if !impact_child_speed_allows(vec3_length_sq(pre_vel)) {
            return;
        }
        let (parent_name, catalog) = host.emitter_in_slot(self.place.fx_cell).map_or(
            (String::new(), NO_RECIPE_LISTING),
            |e: &LiveEffect| (e.definition_name.clone(), e.ledger_index),
        );
        host.pending_trail_impacts.push(QueuedTrailHit {
            source_def_name: parent_name,
            ledger_index: catalog,
            definition_slot: self.step.definition_slot,
            place_at: at,
            prior_vel: pre_vel,
            ms_now: self.step.clock_ms_now,
        });
    }
}

// --- laying new samples ---------------------------------------------------------

/// When a new sample is born and what seeds its randomness.
struct Birth {
    begins: i32,
    seed: u32,
}

impl Birth {
    /// Plans the sample a trail at `sequence` lays at `msec`; None when it
    /// would already be dead at the host's clock and nothing keeps it.
    fn plan(
        host: &EffectSystemHost,
        part: &ParticleRecipeSummary,
        effect_roll: u16,
        sequence: i8,
        msec: i32,
    ) -> Option<Birth> {
        let seed = trail_random_seed(effect_roll, sequence);
        let jitter = if part.wait_spread == 0 {
            0
        } else {
            sample_life_span_msec(0, part.wait_spread, random_table_u16(seed, DICE_LANE_WAIT))
        };
        let begins = msec.wrapping_add(jitter).wrapping_add(part.wait_floor);
        let expires = begins.wrapping_add(life_for(part, seed));
        if expires <= host.clock_ms_now && !part.held_open_by_offspring() {
            return None;
        }
        Some(Birth { begins, seed })
    }
}

/// Lays a sample on a trail at the given pose. Returns whether one was added.
pub(crate) fn advance_ribbon(
    host: &mut EffectSystemHost,
    fx_index: usize,
    trail: u16,
    elem_def: ParticleRecipeSummary,
    at: [f32; 3],
    axes: [[f32; 3]; 3],
    msec: i32,
    travelled: f32,
) -> bool {
    let effect_roll = match host.emitter_in_slot(fx_index) {
        Some(owner) if owner.in_ring => owner.dice_start,
        Some(_) | None => return false,
    };
    let Some(trail_idx) = live_trail(host, trail) else {
        return false;
    };
    let sequence = host.trails[trail_idx].sequence;
    let Some(birth) = Birth::plan(host, &elem_def, effect_roll, sequence, msec) else {
        return false;
    };
    let Some(slot) = claim_ribbon_point(host) else {
        return false;
    };

    take_claim(host, fx_index);
    let handle = trail_piece_ticket_for_slot(slot as u32);
    if let Some(behind) = Anchor::of(host.trails[trail_idx].last_elem_handle) {
        behind.retarget(host, trail_idx, handle);
    }
    host.trail_elems[slot] = RibbonPointCell {
        is_taken: true,
        sequence: sequence as u8,
        window_open_ms: birth.begins,
        spot_xyz: sample_spawn_origin(&elem_def, at, axes, birth.seed),
        basis: compress_basis_from_axis(axes),
        spawn_dist: travelled,
        ..RibbonPointCell::unused(NO_LINK)
    };
    let lane = &mut host.trails[trail_idx];
    lane.sequence = sequence.wrapping_add(1);
    lane.last_elem_handle = handle;
    true
}

/// Drops the newest sample of a trail, which stood in for the moving end of
/// the ribbon, so a fresh one can take its place. A trail with a single
/// sample keeps it.
fn retire_moving_end(host: &mut EffectSystemHost, fx_index: usize, lane: usize) {
    let newest = host.trails[lane].last_elem_handle;
    let mut cursor = host.trails[lane].lead_particle_ticket;
    let ahead = loop {
        match link_after(host, cursor) {
            Some(next) if next == newest => break cursor,
            Some(next) => cursor = next,
            None => return,
        }
    };
    drop_sample(host, fx_index, lane, newest, ahead);
    let trail = &mut host.trails[lane];
    trail.sequence = trail.sequence.wrapping_sub(1);
}

/// What one pass over an effect's trails needs to know about the effect.
struct Travel {
    head: u16,
    catalog: u16,
    born: i32,
    loops: bool,
    odometer: f32,
    from_frame: EffectFacingBasis,
    to_frame: EffectFacingBasis,
}

impl Travel {
    fn of(host: &EffectSystemHost, effect_slot: usize) -> Option<Self> {
        let fx = host.emitter_in_slot(effect_slot).filter(|e| e.in_ring)?;
        Some(Self {
            head: fx.first_ribbon_ticket,
            catalog: fx.ledger_index,
            born: fx.window_open_ms,
            loops: fx.state_bits & EMITTER_REPEAT_QUEUED_BIT != 0,
            odometer: fx.range_to_eye,
            from_frame: EffectFacingBasis {
                origin: fx.spot_before,
                axis: fx.frame_before,
            },
            to_frame: fx.current_frame(),
        })
    }
}

/// How many samples one update may add to a trail: the pool's worth. A huge
/// jump, or a split distance read as nearly zero, once asked for billions
/// and stalled the game for minutes.
fn capped_cut_count(wanted: i32, moved: f32, reciprocal_split_distance: f32) -> i32 {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let room = STREAK_JOINT_BANK_LIMIT as i32;
    if wanted > room {
        ONCE.call_once(|| {
            diag::warn!(
                World,
                "fx: a trail wanted {wanted} samples in one update (split distance {reciprocal_split_distance}, moved {moved}); limited to {room}"
            );
        });
    }
    wanted.min(room)
}

/// Which end of the slice a sample is laid at.
#[derive(Clone, Copy)]
enum Edge {
    Start,
    Finish,
}

/// One update of an effect's trails over the slice `prev_msec..clock_ms_now`.
struct TrailRefresh {
    fx_cell: usize,
    travel: Travel,
    parts: Range<i32>,
    prev_ms: i32,
    clock_ms_now: i32,
    eye_at: [f32; 3],
    /// How far the effect moved and turned over the slice.
    moved: f32,
    turned: f32,
}

impl TrailRefresh {
    fn advancing(&self) -> bool {
        self.prev_ms < self.clock_ms_now && self.travel.loops
    }

    /// Lays a sample at the effect's start pose, or the end pose.
    fn lay_at(&self, host: &mut EffectSystemHost, trail: u16, part: ParticleRecipeSummary, edge: Edge) {
        let tr = &self.travel;
        let (frame, msec, dist) = match edge {
            Edge::Start => (tr.from_frame, self.prev_ms, tr.odometer),
            Edge::Finish => (tr.to_frame, self.clock_ms_now, tr.odometer + self.moved),
        };
        let slot = self.fx_cell;
        advance_ribbon(
            host,
            slot,
            trail,
            part,
            frame.origin,
            frame.axis,
            msec,
            dist,
        );
    }

    fn refresh(
        &self,
        host: &mut EffectSystemHost,
        (trail, lane): (u16, usize),
        trail_def: &mut impl FnMut(u16, u8) -> Option<ParticleRecipeSummary>,
    ) {
        let part = host.trails[lane].definition_slot as u8;
        let fresh = host.trails[lane].lead_particle_ticket == NO_LINK;
        let tr = &self.travel;
        // A one-shot effect lays its first sample on its first update only.
        let one_shot_over = !tr.loops && (!fresh || self.prev_ms != tr.born);
        if one_shot_over || !self.parts.contains(&i32::from(part)) {
            return;
        }
        let Some(part_def) = trail_def(tr.catalog, part) else {
            host.gaps
                .log_hole(EffectHoleReason::TrailDefAbsent { definition_slot: part });
            return;
        };

        let advancing = self.advancing();
        if fresh {
            self.lay_at(host, trail, part_def, Edge::Start);
        } else if advancing {
            retire_moving_end(host, self.fx_cell, lane);
        }
        if advancing {
            self.add_cuts(host, (trail, lane), part_def);
        }
    }

    /// Adds the samples the distance, time and turning of the slice call
    /// for, then one at the effect's present pose.
    fn add_cuts(&self, host: &mut EffectSystemHost, (trail, lane): (u16, usize), part: ParticleRecipeSummary) {
        let carried = host.trails[lane].split_leftover;
        let elapsed = (self.clock_ms_now - self.prev_ms) as f32;
        let (spare, wanted) = spare_and_cuts(trail_split_window(
            carried,
            part.reciprocal_split_duration,
            elapsed,
            part.reciprocal_split_distance,
            self.moved,
            part.reciprocal_split_arc_length,
            self.turned,
        ));
        let kept = if spare.is_finite() { spare } else { 0.0 };
        host.trails[lane].split_leftover = kept;

        let cuts = capped_cut_count(wanted, self.moved, part.reciprocal_split_distance);
        let total = kept + cuts as f32;
        for t in (1..=cuts).map(|k| trail_split_interpolant_t(k as f32, carried, total)) {
            self.cut(host, (trail, lane), part, t);
        }
        self.lay_at(host, trail, part, Edge::Finish);
    }

    /// The sample `t` of the way along the slice, unless it is too far from
    /// the camera to be worth keeping; either way it uses up a sequence number.
    fn cut(
        &self,
        host: &mut EffectSystemHost,
        (trail, lane): (u16, usize),
        part: ParticleRecipeSummary,
        t: f32,
    ) {
        let tr = &self.travel;
        let spot = trail_split_lerp_origin(tr.from_frame.origin, tr.to_frame.origin, t);
        let seq = host.trails[lane].sequence;
        let too_far = trail_split_skips_update(
            seq as u8,
            part.birth_span_floor,
            part.birth_span_spread,
            self.eye_at,
            spot,
        );
        if too_far {
            host.trails[lane].sequence = seq.wrapping_add(1);
            return;
        }
        let axes = trail_split_lerp_axis(tr.from_frame.axis, tr.to_frame.axis, t);
        let msec = trail_split_interpolant_msec(self.prev_ms, self.clock_ms_now, t);
        let dist = tr.odometer + self.moved * t;
        advance_ribbon(host, self.fx_cell, trail, part, spot, axes, msec, dist);
    }
}

/// Brings every trail of an effect up to date for the time slice: starts
/// empty ones, and while the effect loops, swaps the moving end and adds the
/// samples the distance and time travelled call for.
pub(crate) fn advance_effect_ribbons(
    host: &mut EffectSystemHost,
    fx_index: usize,
    first_part: i32,
    past_last_part: i32,
    latest_ms: i32,
    clock_ms_now: i32,
    eye_pos: [f32; 3],
    mut trail_def: impl FnMut(u16, u8) -> Option<ParticleRecipeSummary>,
) {
    let Some(travel) = Travel::of(host, fx_index) else {
        return;
    };
    let lanes = live_trails(host, travel.head);
    let pass = TrailRefresh {
        fx_cell: fx_index,
        parts: first_part..past_last_part,
        prev_ms: latest_ms,
        clock_ms_now,
        eye_at: eye_pos,
        moved: vec3_distance(travel.from_frame.origin, travel.to_frame.origin),
        turned: effect_orient_arc(travel.from_frame.axis, travel.to_frame.axis),
        travel,
    };
    lanes
        .into_iter()
        .for_each(|lane| pass.refresh(host, lane, &mut trail_def));
}

// --- the mid-frame pass ---------------------------------------------------------

/// Whether a sample has outlived its part's life at `clock_ms_now`; a part whose
/// definition is unknown never expires.
fn sample_expired(
    part: Option<ParticleRecipeSummary>,
    roll: u16,
    sample: &RibbonPointCell,
    now: i32,
) -> bool {
    let Some(part) = part else {
        return false;
    };
    let life = life_for(&part, trail_random_seed(roll, sample.sequence as i8));
    !trail_elem_keep(now, sample.window_open_ms, life)
}

/// Frees the run of expired samples at the front of a trail, keeping the
/// last expired one so the ribbon's old edge has something to fade from.
/// Returns true when the whole trail had expired and its last sample went.
fn prune_expired_front(
    host: &mut EffectSystemHost,
    fx_index: usize,
    lane: usize,
    part: Option<ParticleRecipeSummary>,
    roll: u16,
    now: i32,
) -> bool {
    let chain = live_chain(host, host.trails[lane].lead_particle_ticket);
    let lapsed = chain
        .iter()
        .position(|&h| {
            let slot = live_sample(host, h);
            !slot.is_some_and(|ix| sample_expired(part, roll, &host.trail_elems[ix], now))
        })
        .unwrap_or(chain.len());

    for &doomed in chain.iter().take(lapsed.saturating_sub(1)) {
        drop_sample(host, fx_index, lane, doomed, NO_LINK);
    }
    let newest = host.trails[lane].last_elem_handle;
    let whole_trail = chain
        .last()
        .copied()
        .filter(|&h| lapsed == chain.len() && h == newest);
    let Some(last) = whole_trail else {
        return false;
    };
    drop_sample(host, fx_index, lane, last, NO_LINK);
    true
}

/// Carries every sample of one trail through the slice, tracking which
/// sample is ahead of the next one as dead ones drop out.
fn carry_chain(
    host: &mut EffectSystemHost,
    fx_index: usize,
    lane: usize,
    step: &TrailStep<'_>,
    trace: &mut dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
) {
    let mut ahead = NO_LINK;
    for handle in live_chain(host, host.trails[lane].lead_particle_ticket) {
        let place = SamplePlace {
            fx_cell: fx_index,
            trail_idx: lane,
            handle,
            before: ahead,
        };
        move_sample(host, place, step, trace);
        if live_sample(host, handle).is_some() {
            ahead = handle;
        }
    }
}

/// Ages the samples of every trail of an effect to `clock_ms_now` and, while the
/// effect loops, points its newest sample at where the effect is now.
pub(crate) fn apply_partial_last_trail_spawn_dist(
    host: &mut EffectSystemHost,
    fx_index: usize,
    latest_ms: i32,
    clock_ms_now: i32,
    pin_dist: f32,
    pin_newest: bool,
    mut trail_def: impl FnMut(u16, u8) -> Option<ParticleRecipeSummary>,
    mut bounce_probe: impl FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>,
    mut vel_graphs: impl FnMut(u16, u8) -> (Vec<Vec3Spread>, Vec<Vec3Spread>),
) {
    let Some(fx) = host.emitter_in_slot(fx_index).filter(|x| x.in_ring) else {
        return;
    };
    let (head, roll, catalog) = (fx.first_ribbon_ticket, fx.dice_start, fx.ledger_index);
    let (now, alt) = (fx.current_frame(), fx.spawn_frame());

    for (_, lane) in live_trails(host, head) {
        let part = host.trails[lane].definition_slot as u8;
        let elem = trail_def(catalog, part);
        if prune_expired_front(host, fx_index, lane, elem, roll, clock_ms_now) {
            continue;
        }
        if let Some(elem) = elem.filter(|_| clock_ms_now != latest_ms) {
            let (local_vel, vel_world) = vel_graphs(catalog, part);
            let step = TrailStep {
                elem,
                definition_slot: part,
                prev_ms: latest_ms,
                clock_ms_now,
                now: &now,
                alt: &alt,
                fx_seed: roll,
                vel_local: &local_vel,
                vel_world: &vel_world,
            };
            carry_chain(host, fx_index, lane, &step, &mut bounce_probe);
        }
        if pin_newest {
            retarget_newest_sample(host, lane, elem, roll, &now, pin_dist);
        }
    }
}

/// Re-pins a trail's newest sample to the effect's current pose.
fn retarget_newest_sample(
    host: &mut EffectSystemHost,
    lane: usize,
    part: Option<ParticleRecipeSummary>,
    roll: u16,
    frame: &EffectFacingBasis,
    travelled: f32,
) {
    let Some(slot) = live_sample(host, host.trails[lane].last_elem_handle) else {
        return;
    };
    let sample = &mut host.trail_elems[slot];
    sample.spawn_dist = travelled;
    let Some(part) = part else {
        return;
    };
    let seed = trail_random_seed(roll, sample.sequence as i8);
    sample.spot_xyz = sample_spawn_origin(&part, frame.origin, frame.axis, seed);
    sample.basis = compress_basis_from_axis(frame.axis);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UNTURNED_BASIS;
    use crate::holes::EffectHole;
    use fx_t6::runtime::{ELEM_ENDS_ON_TOUCH, ELEM_COLLIDES_WITH_WORLD, ELEM_SPEED_WORLD_AXES};


    /// A host whose effect 0 is resident, holds one claim and has no trails.
    fn host_with_effect() -> EffectSystemHost {
        let mut host = EffectSystemHost::new();
        let effect = host.live_effect_mut(0).unwrap();
        effect.in_ring = true;
        effect.dice_start = 0x4321;
        effect.state_bits = 1;
        effect.window_open_ms = 100;
        host
    }

    fn part(life: i32) -> ParticleRecipeSummary {
        ParticleRecipeSummary {
            lifespan_floor: life,
            ..ParticleRecipeSummary::default()
        }
    }

    /// A trail for part `definition_slot`, chained as effect 0's first trail.
    fn trail_for_effect(host: &mut EffectSystemHost, definition_slot: i8) -> (u16, usize) {
        let handle = claim_ribbon(host, definition_slot).unwrap();
        host.live_effect_mut(0).unwrap().first_ribbon_ticket = handle;
        (handle, ribbon_index_of(handle).unwrap())
    }

    fn lay(
        host: &mut EffectSystemHost,
        trail: u16,
        part_def: ParticleRecipeSummary,
        at: [f32; 3],
        msec: i32,
    ) -> bool {
        advance_ribbon(host, 0, trail, part_def, at, UNTURNED_BASIS, msec, 0.0)
    }

    /// The handles of a trail's samples, oldest first.
    fn chain(host: &EffectSystemHost, trail_idx: usize) -> Vec<u16> {
        let mut out = Vec::new();
        let mut at = host.trails[trail_idx].lead_particle_ticket;
        while let Some(slot) = trail_elem_slot_for_handle(at) {
            out.push(at);
            at = host.trail_elems[slot].next_trail_elem_handle;
        }
        out
    }

    fn sample_of(host: &EffectSystemHost, handle: u16) -> &RibbonPointCell {
        &host.trail_elems[trail_elem_slot_for_handle(handle).unwrap()]
    }

    fn only_sample(host: &EffectSystemHost, slot: usize) -> &RibbonPointCell {
        sample_of(host, chain(host, slot)[0])
    }

    fn no_defs(_: u16, _: u8) -> Option<ParticleRecipeSummary> {
        None
    }

    fn no_world(_: [f32; 3], _: [f32; 3], _: [f32; 3], _: [f32; 3], _: u32) -> Option<StreakBounceContact> {
        None
    }

    fn no_graphs(_: u16, _: u8) -> (Vec<Vec3Spread>, Vec<Vec3Spread>) {
        Default::default()
    }

    fn touching(fraction: f32, hit_dir: [f32; 3]) -> StreakBounceContact {
        StreakBounceContact {
            hit_frac: fraction,
            hit_facing: hit_dir,
            startsolid: false,
            fully_embedded: false,
        }
    }

    /// Runs the mid-frame pass on effect 0 for its part 0.
    fn run_pass(
        host: &mut EffectSystemHost,
        (prev, now): (i32, i32),
        pinning: bool,
        def: Option<ParticleRecipeSummary>,
        mut trace: Box<dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact>>,
    ) {
        apply_partial_last_trail_spawn_dist(
            host,
            0,
            prev,
            now,
            9.0,
            pinning,
            |_, _| def,
            &mut trace,
            no_graphs,
        );
    }

    #[test]
    fn handles_name_slots_only_on_boundaries() {
        for slot in [0u32, 1, 5, STREAK_BANK_LIMIT - 1] {
            let handle = trail_ticket_for_slot(slot);
            assert_eq!(ribbon_index_of(handle), Some(slot as usize));
        }
        for slot in [0u32, 3, STREAK_JOINT_BANK_LIMIT - 1] {
            let handle = trail_piece_ticket_for_slot(slot);
            assert_eq!(trail_elem_slot_for_handle(handle), Some(slot as usize));
        }
        assert_eq!(ribbon_index_of(NO_STREAK_TICKET), None);
        assert_eq!(trail_elem_slot_for_handle(NO_STREAK_TICKET), None);
        assert_eq!(ribbon_index_of(trail_ticket_for_slot(2) + 1), None);
        assert_eq!(
            trail_elem_slot_for_handle(trail_piece_ticket_for_slot(2) + 1),
            None
        );
        assert_eq!(
            ribbon_index_of(trail_ticket_for_slot(STREAK_BANK_LIMIT)),
            None
        );
        assert_eq!(
            trail_elem_slot_for_handle(trail_piece_ticket_for_slot(STREAK_JOINT_BANK_LIMIT)),
            None
        );
    }

    #[test]
    fn trails_leave_the_free_list_in_order_and_rejoin_at_its_front() {
        let mut host = EffectSystemHost::new();
        let first = claim_ribbon(&mut host, 3).unwrap();
        let second = claim_ribbon(&mut host, -1).unwrap();
        assert_eq!(first, trail_ticket_for_slot(0));
        assert_eq!(second, trail_ticket_for_slot(1));
        assert!(host.trails[0].taken && host.trails[1].taken);
        assert_eq!(host.trails[0].definition_slot, 3);
        assert_eq!(host.trails[1].definition_slot, -1);
        assert_eq!(host.trails[0].lead_particle_ticket, NO_STREAK_TICKET);
        assert_eq!(host.ribbon_free_head, Some(2));

        release_trail(&mut host, 0);
        assert!(!host.trails[0].taken);
        assert_eq!(host.ribbon_free_head, Some(0));
        assert_eq!(host.trails[0].next_trail_handle, 2);
        assert_eq!(claim_ribbon(&mut host, 7), Some(first));
        assert_eq!(host.trails[0].definition_slot, 7);
        assert_eq!(host.ribbon_free_head, Some(2));
    }

    #[test]
    fn a_full_trail_pool_hands_out_nothing() {
        let mut host = EffectSystemHost::new();
        for _ in 0..STREAK_BANK_LIMIT {
            assert!(claim_ribbon(&mut host, 0).is_some());
        }
        assert_eq!(host.ribbon_free_head, None);
        assert_eq!(claim_ribbon(&mut host, 0), None);
        release_trail(&mut host, 9);
        assert_eq!(host.ribbon_free_head, Some(9));
        assert_eq!(host.trails[9].next_trail_handle, NO_STREAK_TICKET);
        assert_eq!(claim_ribbon(&mut host, 0), Some(trail_ticket_for_slot(9)));
    }

    #[test]
    fn samples_leave_the_free_list_in_order_and_rejoin_at_its_front() {
        let mut host = EffectSystemHost::new();
        assert_eq!(claim_ribbon_point(&mut host), Some(0));
        assert_eq!(claim_ribbon_point(&mut host), Some(1));
        assert_eq!(host.ribbon_head_free, Some(2));
        host.trail_elems[1].is_taken = true;
        release_trail_elem(&mut host, 1);
        assert!(!host.trail_elems[1].is_taken);
        assert_eq!(host.ribbon_head_free, Some(1));
        assert_eq!(host.trail_elems[1].next_trail_elem_handle, 2);
        assert_eq!(claim_ribbon_point(&mut host), Some(1));

        host.ribbon_head_free = Some(host.trail_elems.len());
        assert_eq!(claim_ribbon_point(&mut host), None);
        assert_eq!(host.ribbon_head_free, None);
    }

    #[test]
    fn live_lookups_need_an_occupied_slot() {
        let mut host = host_with_effect();
        let (ticket, at) = trail_for_effect(&mut host, 0);
        assert_eq!(live_trail(&host, ticket), Some(at));
        assert_eq!(live_trail(&host, trail_ticket_for_slot(5)), None);
        assert!(lay(&mut host, ticket, part(100), [0.0; 3], 0));
        let sample = host.trails[at].lead_particle_ticket;
        assert!(live_sample(&host, sample).is_some());
        assert_eq!(live_sample(&host, trail_piece_ticket_for_slot(9)), None);
        assert_eq!(live_sample(&host, NO_STREAK_TICKET), None);
    }

    #[test]
    fn laying_samples_links_them_and_counts_a_claim_each() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 2);
        let def = ParticleRecipeSummary {
            wait_floor: 5,
            ..part(300)
        };
        assert!(advance_ribbon(
            &mut host,
            0,
            ticket,
            def,
            [1.0, 2.0, 3.0],
            UNTURNED_BASIS,
            40,
            7.5
        ));
        assert!(lay(&mut host, ticket, def, [4.0, 5.0, 6.0], 80));

        let samples = chain(&host, slot);
        assert_eq!(samples.len(), 2);
        assert_eq!(host.trails[slot].last_elem_handle, samples[1]);
        assert_eq!(host.trails[slot].sequence, 2);
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, 3);

        let (a, b) = (sample_of(&host, samples[0]), sample_of(&host, samples[1]));
        assert!(a.is_taken && b.is_taken);
        assert_eq!((a.window_open_ms, b.window_open_ms), (45, 85));
        assert_eq!((a.sequence, b.sequence), (0, 1));
        assert_eq!(a.spawn_dist, 7.5);
        assert_eq!(a.spot_xyz, [1.0, 2.0, 3.0]);
        assert_eq!(b.spot_xyz, [4.0, 5.0, 6.0]);
        assert_eq!(a.base_vel_z, 0);
        assert_eq!(a.basis, compress_basis_from_axis(UNTURNED_BASIS));
        assert_eq!(a.next_trail_elem_handle, samples[1]);
        assert_eq!(b.next_trail_elem_handle, NO_STREAK_TICKET);
    }

    #[test]
    fn a_delay_amplitude_pushes_the_birth_later() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = ParticleRecipeSummary {
            wait_floor: 10,
            wait_spread: 1000,
            ..part(5000)
        };
        assert!(lay(&mut host, ticket, def, [0.0; 3], 20));
        let begin = only_sample(&host, slot).window_open_ms;
        assert!((30..=1030).contains(&begin), "{begin}");
        assert_ne!(begin, 30);
    }

    #[test]
    fn laying_is_refused_when_nothing_can_be_added() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        // not a live trail
        assert!(!lay(
            &mut host,
            trail_ticket_for_slot(9),
            part(100),
            [0.0; 3],
            0
        ));
        // already dead on arrival
        host.clock_ms_now = 500;
        assert!(!lay(&mut host, ticket, part(100), [0.0; 3], 0));
        // ... unless a child effect could still appear
        let keeper = ParticleRecipeSummary {
            spawns_on_finish: true,
            ..part(100)
        };
        assert!(lay(&mut host, ticket, keeper, [0.0; 3], 0));
        host.clock_ms_now = 0;
        // no room for a sample
        let spare = host.ribbon_head_free.take();
        assert!(!lay(&mut host, ticket, part(100), [0.0; 3], 0));
        host.ribbon_head_free = spare.map(|_| 10);
        // effect not resident
        host.live_effect_mut(0).unwrap().in_ring = false;
        assert!(!lay(&mut host, ticket, part(100), [0.0; 3], 0));
        assert_eq!(chain(&host, slot).len(), 1);
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, 2);
    }

    #[test]
    fn expired_samples_are_pruned_but_the_last_expired_one_stays() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = part(100);
        for msec in [0, 10, 500] {
            assert!(lay(&mut host, ticket, def, [msec as f32; 3], msec));
        }
        let all = chain(&host, slot);
        run_pass(&mut host, (300, 300), false, Some(def), Box::new(no_world));
        assert_eq!(chain(&host, slot), vec![all[1], all[2]]);
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, 3);
        assert!(!host.trail_elems[trail_elem_slot_for_handle(all[0]).unwrap()].is_taken);
        assert_eq!(
            host.ribbon_head_free,
            trail_elem_slot_for_handle(all[0])
        );
    }

    #[test]
    fn a_wholly_expired_trail_loses_every_sample() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = part(100);
        for msec in [0, 10, 20] {
            assert!(lay(&mut host, ticket, def, [0.0; 3], msec));
        }
        run_pass(&mut host, (900, 900), false, Some(def), Box::new(no_world));
        assert!(chain(&host, slot).is_empty());
        assert_eq!(host.trails[slot].lead_particle_ticket, NO_STREAK_TICKET);
        assert_eq!(host.trails[slot].last_elem_handle, NO_STREAK_TICKET);
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, 1);
    }

    #[test]
    fn unknown_parts_keep_every_sample() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        for msec in [0, 10] {
            assert!(lay(&mut host, ticket, part(5), [0.0; 3], msec));
        }
        run_pass(&mut host, (900, 900), false, None, Box::new(no_world));
        assert_eq!(chain(&host, slot).len(), 2);
    }

    #[test]
    fn a_looping_effect_repins_its_newest_sample() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = part(10_000);
        assert!(lay(&mut host, ticket, def, [5.0, 5.0, 5.0], 0));
        assert!(lay(&mut host, ticket, def, [6.0, 6.0, 6.0], 10));
        host.live_effect_mut(0).unwrap().where_at = [10.0, 20.0, 30.0];
        let all = chain(&host, slot);

        run_pass(&mut host, (50, 50), false, Some(def), Box::new(no_world));
        assert_eq!(sample_of(&host, all[1]).spot_xyz, [6.0, 6.0, 6.0]);

        run_pass(&mut host, (50, 50), true, Some(def), Box::new(no_world));
        let (old, newest) = (sample_of(&host, all[0]), sample_of(&host, all[1]));
        assert_eq!(old.spot_xyz, [5.0, 5.0, 5.0]);
        assert_eq!(newest.spawn_dist, 9.0);
        assert_eq!(newest.spot_xyz, [10.0, 20.0, 30.0]);
        assert_eq!(newest.basis, compress_basis_from_axis(UNTURNED_BASIS));

        // without a definition only the distance moves
        host.live_effect_mut(0).unwrap().where_at = [0.0; 3];
        run_pass(&mut host, (50, 50), true, None, Box::new(no_world));
        assert_eq!(sample_of(&host, all[1]).spot_xyz, [10.0, 20.0, 30.0]);
    }

    #[test]
    fn a_one_shot_effect_lays_its_first_sample_at_its_first_update() {
        let mut host = host_with_effect();
        let (_, slot) = trail_for_effect(&mut host, 0);
        let effect = host.live_effect_mut(0).unwrap();
        effect.spot_before = [1.0, 1.0, 1.0];
        effect.where_at = [4.0, 5.0, 6.0];
        effect.range_to_eye = 2.0;
        let def = part(1000);

        advance_effect_ribbons(&mut host, 0, 0, 1, 100, 150, [0.0; 3], |_, _| Some(def));
        let samples = chain(&host, slot);
        assert_eq!(samples.len(), 1);
        let first = sample_of(&host, samples[0]);
        assert_eq!(first.spot_xyz, [1.0, 1.0, 1.0]);
        assert_eq!(first.window_open_ms, 100);
        assert_eq!(first.spawn_dist, 2.0);

        advance_effect_ribbons(&mut host, 0, 0, 1, 150, 200, [0.0; 3], |_, _| Some(def));
        assert_eq!(chain(&host, slot).len(), 1);
    }

    #[test]
    fn parts_outside_the_range_and_unknown_parts_are_left_alone() {
        let mut host = host_with_effect();
        let (_, slot) = trail_for_effect(&mut host, 2);
        let def = part(1000);
        advance_effect_ribbons(&mut host, 0, 0, 2, 100, 150, [0.0; 3], |_, _| Some(def));
        assert!(chain(&host, slot).is_empty());
        advance_effect_ribbons(&mut host, 0, 3, 9, 100, 150, [0.0; 3], |_, _| Some(def));
        assert!(chain(&host, slot).is_empty());
        assert_eq!(host.gaps.hit_total(EffectHole::TrailDefMiss), 0);

        advance_effect_ribbons(&mut host, 0, 0, 9, 100, 150, [0.0; 3], no_defs);
        assert!(chain(&host, slot).is_empty());
        assert_eq!(host.gaps.hit_total(EffectHole::TrailDefMiss), 1);
    }

    fn looping_effect(host: &mut EffectSystemHost, from: [f32; 3], dest: [f32; 3]) {
        let effect = host.live_effect_mut(0).unwrap();
        effect.state_bits |= EMITTER_REPEAT_QUEUED_BIT;
        effect.spot_before = from;
        effect.where_at = dest;
        effect.range_to_eye = 100.0;
    }

    #[test]
    fn a_moving_loop_adds_a_sample_per_split_and_one_at_the_end() {
        let mut host = host_with_effect();
        let (_, slot) = trail_for_effect(&mut host, 0);
        looping_effect(&mut host, [0.0; 3], [10.0, 0.0, 0.0]);
        let def = ParticleRecipeSummary {
            reciprocal_split_distance: 0.25,
            ..part(10_000)
        };
        advance_effect_ribbons(&mut host, 0, 0, 1, 1000, 1100, [0.0; 3], |_, _| Some(def));

        let samples = chain(&host, slot);
        let seen: Vec<(f32, f32, i32)> = samples
            .iter()
            .map(|&h| {
                let s = sample_of(&host, h);
                (s.spot_xyz[0], s.spawn_dist, s.window_open_ms)
            })
            .collect();
        // moved 10 at 0.25 a unit: cuts at 0.4 and 0.8 of the way, then the end
        let want = [
            (0.0, 100.0, 1000),
            (4.0, 104.0, 1040),
            (8.0, 108.0, 1080),
            (10.0, 110.0, 1100),
        ];
        assert_eq!(seen.len(), want.len());
        for (got, want) in seen.iter().zip(want) {
            assert!((got.0 - want.0).abs() < 1e-4 && (got.1 - want.1).abs() < 1e-4);
            assert_eq!(got.2, want.2);
        }
        assert_eq!(host.trails[slot].sequence, 4);
        assert!((host.trails[slot].split_leftover - 0.5).abs() < 1e-5);

        // The newest sample is the moving end: the next update swaps it out.
        advance_effect_ribbons(&mut host, 0, 0, 1, 1100, 1100, [0.0; 3], |_, _| Some(def));
        let after = chain(&host, slot);
        assert_eq!(after.len(), 4);
        assert_eq!(after[..3], samples[..3]);
        let swapped = sample_of(&host, after[3]);
        assert_eq!((swapped.spot_xyz[0], swapped.window_open_ms), (10.0, 1100));
        assert_eq!(swapped.sequence, 3);
    }

    #[test]
    fn a_standing_loop_swaps_its_moving_end_without_extra_samples() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        looping_effect(&mut host, [3.0; 3], [3.0; 3]);
        let def = part(10_000);
        for msec in [0, 10] {
            assert!(lay(&mut host, ticket, def, [0.0; 3], msec));
        }
        let before = chain(&host, slot);
        advance_effect_ribbons(&mut host, 0, 0, 1, 20, 40, [0.0; 3], |_, _| Some(def));
        let after = chain(&host, slot);
        assert_eq!(after.len(), 2);
        assert_eq!(after[0], before[0]);
        let newest = sample_of(&host, after[1]);
        assert_eq!(newest.spot_xyz, [3.0; 3]);
        assert_eq!(newest.window_open_ms, 40);
        assert_eq!(newest.sequence, 1);
        assert_eq!(host.trails[slot].sequence, 2);
        assert_eq!(
            host.emitter_in_slot(0).unwrap().state_bits,
            3 | EMITTER_REPEAT_QUEUED_BIT
        );
    }

    #[test]
    fn a_huge_jump_is_held_to_the_pool_size() {
        let mut host = host_with_effect();
        let (_, slot) = trail_for_effect(&mut host, 0);
        looping_effect(&mut host, [0.0; 3], [100.0, 0.0, 0.0]);
        let def = ParticleRecipeSummary {
            reciprocal_split_distance: 1.0e9,
            ..part(1_000_000)
        };
        advance_effect_ribbons(&mut host, 0, 0, 1, 0, 10, [0.0; 3], |_, _| Some(def));
        assert_eq!(
            chain(&host, slot).len(),
            STREAK_JOINT_BANK_LIMIT as usize
        );
        assert!(host.trails[slot].split_leftover.is_finite());
    }

    #[test]
    fn far_cuts_skip_their_samples_but_count_in_the_sequence() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        looping_effect(&mut host, [0.0; 3], [8.0, 0.0, 0.0]);
        let def = ParticleRecipeSummary {
            reciprocal_split_distance: 1.0,
            birth_span_floor: 1.0,
            ..part(10_000)
        };
        assert!(lay(&mut host, ticket, def, [0.0; 3], 0));
        assert!(lay(&mut host, ticket, def, [0.0; 3], 0));
        let camera = [-1000.0, 0.0, 0.0];
        advance_effect_ribbons(&mut host, 0, 0, 1, 10, 20, camera, |_, _| Some(def));
        // every cut was too far to keep, yet each used up a sequence number
        assert_eq!(chain(&host, slot).len(), 2);
        assert_eq!(host.trails[slot].sequence, 10);
    }

    /// One colliding sample at the origin, born at 0 with a long life.
    fn colliding(flags: i32, vz: i16) -> (EffectSystemHost, usize, ParticleRecipeSummary) {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = ParticleRecipeSummary {
            bit_set: flags,
            fall_pull_floor: 400.0,
            mirror_floor: 0.5,
            bounce_box_bottom: [-1.0; 3],
            bounce_box_top: [1.0; 3],
            ..part(10_000)
        };
        assert!(lay(&mut host, ticket, def, [0.0; 3], 0));
        let at = trail_elem_slot_for_handle(chain(&host, slot)[0]).unwrap();
        host.trail_elems[at].base_vel_z = vz;
        (host, slot, def)
    }

    #[test]
    fn gravity_pulls_a_sample_on_the_velocity_path() {
        let (mut host, slot, def) = colliding(ELEM_SPEED_WORLD_AXES, 100);
        run_pass(&mut host, (0, 100), false, Some(def), Box::new(no_world));
        let s = only_sample(&host, slot);
        assert_eq!(s.spot_xyz, [0.0, 0.0, -1590.0]);
        assert_eq!(s.base_vel_z, -31900);
    }

    #[test]
    fn a_still_sample_without_velocity_graphs_stays_put() {
        let (mut host, slot, def) = colliding(0, 100);
        run_pass(&mut host, (0, 100), false, Some(def), Box::new(no_world));
        let s = only_sample(&host, slot);
        assert_eq!(s.spot_xyz, [0.0; 3]);
        assert_eq!(s.base_vel_z, 100);
    }

    #[test]
    fn colliding_without_a_world_moves_nothing_and_notes_the_gap() {
        let (mut host, slot, def) = colliding(ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES, 100);
        run_pass(&mut host, (0, 100), false, Some(def), Box::new(no_world));
        let s = only_sample(&host, slot);
        assert_eq!(s.spot_xyz, [0.0; 3]);
        assert_eq!(s.base_vel_z, 100);
        assert_eq!(host.gaps.hit_total(EffectHole::CollideMotion), 1);
    }

    #[test]
    fn open_air_moves_a_colliding_sample_like_a_free_one() {
        let (mut free, slot, def) = colliding(ELEM_SPEED_WORLD_AXES, 100);
        run_pass(&mut free, (0, 100), false, Some(def), Box::new(no_world));
        let (mut hit, hit_slot, def) = colliding(ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES, 100);
        let trace = move |_s: [f32; 3], _e: [f32; 3], lo_corner: [f32; 3], hi_corner: [f32; 3], _m: u32| {
            assert_eq!((lo_corner, hi_corner), ([-1.0; 3], [1.0; 3]));
            Some(touching(1.0, [0.0; 3]))
        };
        run_pass(&mut hit, (0, 100), false, Some(def), Box::new(trace));
        let (a, b) = (only_sample(&free, slot), only_sample(&hit, hit_slot));
        assert_eq!((a.spot_xyz, a.base_vel_z), (b.spot_xyz, b.base_vel_z));
        assert_eq!(b.spot_xyz, [0.0, 0.0, -1590.0]);
    }

    #[test]
    fn starting_in_solid_zeroes_the_climb() {
        let (mut host, slot, def) = colliding(ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES, 100);
        let trace = |_: [f32; 3], _: [f32; 3], _: [f32; 3], _: [f32; 3], _: u32| {
            let mut inside = touching(0.0, [0.0; 3]);
            inside.startsolid = true;
            Some(inside)
        };
        run_pass(&mut host, (0, 100), false, Some(def), Box::new(trace));
        let s = only_sample(&host, slot);
        assert_eq!(s.spot_xyz, [0.0; 3]);
        assert_eq!(s.base_vel_z, 0);
    }

    fn bounce_once()
    -> impl FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<StreakBounceContact> {
        let mut hits = 0;
        move |start, end, _, _, mask| {
            let _ = (start, end, mask);
            hits += 1;
            Some(if hits == 1 {
                touching(0.5, [0.0, 0.0, 1.0])
            } else {
                touching(1.0, [0.0; 3])
            })
        }
    }

    #[test]
    fn a_bounce_reflects_the_fall_and_carries_on() {
        let (mut host, slot, def) = colliding(ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES, -300);
        run_pass(
            &mut host,
            (0, 100),
            false,
            Some(def),
            Box::new(bounce_once()),
        );
        let s = only_sample(&host, slot);
        assert_eq!(s.spot_xyz, [0.0, 0.0, -496.25]);
        assert_eq!(s.base_vel_z, -15850);
    }

    #[test]
    fn a_hit_by_a_part_with_an_impact_effect_is_queued() {
        let (mut host, slot, mut def) = colliding(ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES, -3000);
        def.spawns_on_contact = true;
        let effect = host.live_effect_mut(0).unwrap();
        effect.definition_name = "fx/test".into();
        effect.ledger_index = 17;
        run_pass(
            &mut host,
            (0, 100),
            false,
            Some(def),
            Box::new(bounce_once()),
        );
        let queued = &host.pending_trail_impacts;
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].source_def_name, "fx/test");
        assert_eq!(queued[0].ledger_index, 17);
        assert_eq!(queued[0].ms_now, 100);
        assert_eq!(queued[0].place_at, [0.0, 0.0, -275.0]);
        assert_eq!(queued[0].prior_vel, [0.0, 0.0, -19000.0]);
        assert!(only_sample(&host, slot).is_taken);
    }

    #[test]
    fn touching_something_kills_a_die_on_touch_sample() {
        let (mut host, slot, def) = colliding(
            ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES | ELEM_ENDS_ON_TOUCH,
            -300,
        );
        // the sample holds the effect's only claim
        host.live_effect_mut(0).unwrap().state_bits = 1;
        let trace = |_: [f32; 3], _: [f32; 3], _: [f32; 3], _: [f32; 3], _: u32| {
            Some(touching(0.5, [0.0, 0.0, 1.0]))
        };
        run_pass(&mut host, (0, 100), false, Some(def), Box::new(trace));
        assert!(chain(&host, slot).is_empty());
        assert_eq!(host.emitter_in_slot(0).unwrap().state_bits, 0);
        assert!(host.gc_pending);
    }

    #[test]
    fn a_dying_sample_leaves_its_neighbours_linked() {
        let mut host = host_with_effect();
        let (ticket, slot) = trail_for_effect(&mut host, 0);
        let def = ParticleRecipeSummary {
            bit_set: ELEM_COLLIDES_WITH_WORLD | ELEM_SPEED_WORLD_AXES | ELEM_ENDS_ON_TOUCH,
            bounce_box_bottom: [-1.0; 3],
            bounce_box_top: [1.0; 3],
            ..part(10_000)
        };
        for (i, x) in [0.0f32, 1.0, 2.0].into_iter().enumerate() {
            assert!(lay(&mut host, ticket, def, [x, 0.0, 0.0], i as i32));
        }
        let all = chain(&host, slot);
        // only the middle sample (starting at x = 1) touches
        let trace = |begin_at: [f32; 3], _: [f32; 3], _: [f32; 3], _: [f32; 3], _: u32| {
            Some(if (begin_at[0] - 1.0).abs() < 0.01 {
                touching(0.5, [0.0, 0.0, 1.0])
            } else {
                touching(1.0, [0.0; 3])
            })
        };
        run_pass(&mut host, (0, 20), false, Some(def), Box::new(trace));
        assert_eq!(chain(&host, slot), vec![all[0], all[2]]);
        assert_eq!(host.trails[slot].last_elem_handle, all[2]);
    }
}
