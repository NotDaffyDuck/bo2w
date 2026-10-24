//! Ribbon trails. A trail is a chain of samples laid down as its effect
//! moves. This module owns the two pools they live in, adds samples as the
//! effect travels, and ages and moves the ones already taken.

use fx_t6::runtime::{
    FX_RAND_CH_DELAY, FX_RAND_CH_LIFE, FX_STATUS_HAS_PENDING_LOOP_ELEMS,
    FX_TRAIL_ELEM_POOL_CAPACITY, FX_TRAIL_ELEM_RUNTIME_STRIDE, FX_TRAIL_POOL_CAPACITY,
    FX_TRAIL_RUNTIME_STRIDE, FxElemVec3Range, FxOrientFrame, FxOrientSpawnParams, FxOrientation,
    FxTrailSplit, collide_substep_schedule, collision_reflect_base_vel_delta,
    compress_basis_from_axis, effect_orient_arc, elem_dies_on_touch, elem_gravity_accel_z_sampled,
    elem_skips_position_update, elem_update_has_velocity_graph, elem_uses_collision,
    elem_uses_vel_local, elem_uses_vel_world, get_orientation, get_velocity_at_time,
    impact_child_speed_allows, integrate_velocity_graph, orientation_pos_from_world,
    orientation_pos_to_world, random_table_u16, sample_life_span_msec, sample_reflection_factor,
    spawn_origin_world, status_is_unique_done, trace_mask, trail_elem_base_vel_z_pack,
    trail_elem_handle_for_slot, trail_elem_keep, trail_elem_norm_ages, trail_handle_for_slot,
    trail_random_seed, trail_split_interpolant_msec, trail_split_interpolant_t,
    trail_split_lerp_axis, trail_split_lerp_origin, trail_split_skips_update, trail_split_window,
    vec3_distance, vec3_length_sq,
};

use crate::def::FxElemDefInfo;
use crate::gaps::FxGapCause;
use crate::system::{FX_CATALOG_INDEX_NONE, FxSystemHost, PendingTrailImpact};

/// Ends a chain of trails or of trail samples.
pub const FX_TRAIL_HANDLE_NONE: u16 = 0xffff;

/// A collision sweep splits a time slice into at most this many pieces per
/// sub-step.
const MAX_BOUNCES_PER_STEP: usize = 8;

/// What the caller's world probe reports when a sample's sweep meets something.
#[derive(Clone, Copy, Debug)]
pub struct FxTrailCollideHit {
    pub fraction: f32,
    pub normal: [f32; 3],
    pub startsolid: bool,
    pub allsolid: bool,
}

/// One trail: the chain of samples laid so far.
#[derive(Clone, Debug)]
pub struct FxTrailSlot {
    pub occupied: bool,
    /// The next trail of the same effect (or the next free slot, when free).
    pub next_trail_handle: u16,
    pub first_elem_handle: u16,
    pub last_elem_handle: u16,
    /// Which of the effect's parts this trail draws.
    pub def_index: i8,
    /// Counts samples laid; seeds each one's randomness.
    pub sequence: i8,
    /// Spacing carried over from the last update, in split units.
    pub split_leftover: f32,
}

impl Default for FxTrailSlot {
    fn default() -> Self {
        Self {
            occupied: false,
            next_trail_handle: FX_TRAIL_HANDLE_NONE,
            first_elem_handle: FX_TRAIL_HANDLE_NONE,
            last_elem_handle: FX_TRAIL_HANDLE_NONE,
            def_index: 0,
            sequence: 0,
            split_leftover: 0.0,
        }
    }
}

/// One sample along a trail.
#[derive(Clone, Debug)]
pub struct FxTrailElemSlot {
    pub occupied: bool,
    pub origin: [f32; 3],
    /// How far the effect had travelled when this sample was laid.
    pub spawn_dist: f32,
    pub msec_begin: i32,
    /// The next sample along the trail (or the next free slot, when free).
    pub next_trail_elem_handle: u16,
    pub base_vel_z: i16,
    pub basis: [i8; 6],
    pub sequence: u8,
}

impl Default for FxTrailElemSlot {
    fn default() -> Self {
        Self {
            occupied: false,
            origin: [0.0; 3],
            spawn_dist: 0.0,
            msec_begin: 0,
            next_trail_elem_handle: FX_TRAIL_HANDLE_NONE,
            base_vel_z: 0,
            basis: [0; 6],
            sequence: 0,
        }
    }
}

// --- handles and pools ------------------------------------------------------

/// Handles count 4-byte units from the pool start; one that does not land on
/// a slot boundary, or lands past the pool, names no slot.
fn slot_from_handle(handle: u16, stride: u32, capacity: usize) -> Option<usize> {
    if handle == FX_TRAIL_HANDLE_NONE {
        return None;
    }
    let bytes = u32::from(handle) << 2;
    if bytes % stride != 0 {
        return None;
    }
    let slot = (bytes / stride) as usize;
    (slot < capacity).then_some(slot)
}

pub fn trail_slot_for_handle(handle: u16) -> Option<usize> {
    slot_from_handle(
        handle,
        FX_TRAIL_RUNTIME_STRIDE as u32,
        FX_TRAIL_POOL_CAPACITY as usize,
    )
}

pub fn trail_elem_slot_for_handle(handle: u16) -> Option<usize> {
    slot_from_handle(
        handle,
        FX_TRAIL_ELEM_RUNTIME_STRIDE as u32,
        FX_TRAIL_ELEM_POOL_CAPACITY as usize,
    )
}

/// The slot of an in-use trail.
pub(crate) fn live_trail(host: &FxSystemHost, handle: u16) -> Option<usize> {
    trail_slot_for_handle(handle).filter(|&s| host.trails.get(s).is_some_and(|t| t.occupied))
}

/// The slot of an in-use sample.
pub(crate) fn live_sample(host: &FxSystemHost, handle: u16) -> Option<usize> {
    trail_elem_slot_for_handle(handle)
        .filter(|&s| host.trail_elems.get(s).is_some_and(|e| e.occupied))
}

/// Takes a trail off the free list for the part at `def_index`.
pub(crate) fn alloc_trail(host: &mut FxSystemHost, def_index: i8) -> Option<u16> {
    let slot = host.trail_first_free?;
    if slot >= host.trails.len() {
        host.trail_first_free = None;
        return None;
    }
    let link = host.trails[slot].next_trail_handle;
    host.trail_first_free = (link != FX_TRAIL_HANDLE_NONE).then_some(usize::from(link));
    host.trails[slot] = FxTrailSlot {
        occupied: true,
        def_index,
        ..FxTrailSlot::default()
    };
    Some(trail_handle_for_slot(slot as u32))
}

/// Puts a trail slot back on the free list.
pub(crate) fn release_trail(host: &mut FxSystemHost, slot: usize) {
    let link = host
        .trail_first_free
        .map_or(FX_TRAIL_HANDLE_NONE, |s| s as u16);
    host.trails[slot] = FxTrailSlot {
        next_trail_handle: link,
        ..FxTrailSlot::default()
    };
    host.trail_first_free = Some(slot);
}

/// Takes a sample slot off the free list.
pub(crate) fn alloc_trail_elem(host: &mut FxSystemHost) -> Option<usize> {
    let slot = host.trail_elem_first_free?;
    if slot >= host.trail_elems.len() {
        host.trail_elem_first_free = None;
        return None;
    }
    let link = host.trail_elems[slot].next_trail_elem_handle;
    host.trail_elem_first_free = (link != FX_TRAIL_HANDLE_NONE).then_some(usize::from(link));
    Some(slot)
}

/// Puts a sample slot back on the free list.
pub(crate) fn release_trail_elem(host: &mut FxSystemHost, slot: usize) {
    let link = host
        .trail_elem_first_free
        .map_or(FX_TRAIL_HANDLE_NONE, |s| s as u16);
    host.trail_elems[slot] = FxTrailElemSlot {
        next_trail_elem_handle: link,
        ..FxTrailElemSlot::default()
    };
    host.trail_elem_first_free = Some(slot);
}

/// Removes one sample from its trail. `before` is the sample ahead of it, or
/// `FX_TRAIL_HANDLE_NONE` when it is the first; a link that does not match is
/// left alone. The effect gives up the sample's claim on it.
fn drop_sample(
    host: &mut FxSystemHost,
    effect_slot: usize,
    trail_slot: usize,
    sample: u16,
    before: u16,
) {
    let linked = if before == FX_TRAIL_HANDLE_NONE {
        host.trails[trail_slot].first_elem_handle == sample
    } else {
        trail_elem_slot_for_handle(before)
            .and_then(|s| host.trail_elems.get(s))
            .is_some_and(|e| e.next_trail_elem_handle == sample)
    };
    if !linked {
        return;
    }
    let Some(slot) = live_sample(host, sample) else {
        return;
    };

    let after = host.trail_elems[slot].next_trail_elem_handle;
    if host.trails[trail_slot].last_elem_handle == sample {
        host.trails[trail_slot].last_elem_handle = before;
    }
    match trail_elem_slot_for_handle(before) {
        Some(before_slot) if before != FX_TRAIL_HANDLE_NONE => {
            host.trail_elems[before_slot].next_trail_elem_handle = after;
        }
        _ if before == FX_TRAIL_HANDLE_NONE => host.trails[trail_slot].first_elem_handle = after,
        _ => {}
    }
    release_trail_elem(host, slot);

    let Some(effect) = host.effect_at(effect_slot) else {
        return;
    };
    let (own_handle, last_claim) = (effect.own_handle, status_is_unique_done(effect.status));
    if last_claim {
        host.del_ref_to_effect(own_handle);
    }
    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.status = effect.status.wrapping_sub(1);
    }
}

// --- moving existing samples --------------------------------------------------

/// What stays fixed about one sample while it is carried over a time slice.
struct SampleKinematics<'a> {
    flags: i32,
    life_ms: f32,
    seed: u32,
    gravity: f32,
    orient: FxOrientation,
    vel_local: &'a [FxElemVec3Range],
    vel_world: &'a [FxElemVec3Range],
}

impl SampleKinematics<'_> {
    /// Where the velocity graphs alone carry `origin` between two ages.
    fn graph_shift(&self, origin: [f32; 3], age0: f32, age1: f32) -> [f32; 3] {
        let o = self.orient;
        let mut stored = origin;
        if elem_uses_vel_local(self.flags) && self.vel_local.len() >= 2 {
            let d = integrate_velocity_graph(self.vel_local, age0, age1, self.life_ms, self.seed);
            for axis in 0..3 {
                stored[axis] += d[axis];
            }
        }
        let mut world = orientation_pos_to_world(o.origin, o.axis, stored);
        if elem_uses_vel_world(self.flags) && self.vel_world.len() >= 2 {
            let d = integrate_velocity_graph(self.vel_world, age0, age1, self.life_ms, self.seed);
            for axis in 0..3 {
                world[axis] += d[axis];
            }
        }
        orientation_pos_from_world(o.origin, o.axis, world)
    }

    /// The graphs, then the vertical base velocity and gravity over `dt`
    /// seconds; returns the new origin and base velocity.
    fn advance(
        &self,
        origin: [f32; 3],
        base_vel_z: f32,
        dt: f32,
        age0: f32,
        age1: f32,
    ) -> ([f32; 3], f32) {
        let o = self.orient;
        let shifted = self.graph_shift(origin, age0, age1);
        let mut world = orientation_pos_to_world(o.origin, o.axis, shifted);
        world[2] += base_vel_z * dt;
        world[2] -= self.gravity * dt * dt * 0.5;
        (
            orientation_pos_from_world(o.origin, o.axis, world),
            base_vel_z - self.gravity * dt,
        )
    }
}

/// The time slice and pose context shared by every sample of one trail.
struct TrailStep<'a> {
    def: FxElemDefInfo,
    def_index: u8,
    prev_msec: i32,
    msec_now: i32,
    now: &'a FxOrientFrame,
    alt: &'a FxOrientFrame,
    effect_seed: u16,
    vel_local: &'a [FxElemVec3Range],
    vel_world: &'a [FxElemVec3Range],
}

/// Where a sample sits: the effect and trail it belongs to, and the sample
/// ahead of it.
#[derive(Clone, Copy)]
struct SamplePlace {
    effect_slot: usize,
    trail_slot: usize,
    handle: u16,
    before: u16,
}

type TrailTrace<'a> =
    dyn FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<FxTrailCollideHit> + 'a;

/// Carries one sample from `step.prev_msec` to `step.msec_now`.
fn move_sample(
    host: &mut FxSystemHost,
    place: SamplePlace,
    step: &TrailStep<'_>,
    trace: &mut TrailTrace<'_>,
) {
    let def = step.def;
    if elem_skips_position_update(def.elem_type, def.flags) {
        return;
    }
    let Some(slot) = live_sample(host, place.handle) else {
        return;
    };
    let sample = &host.trail_elems[slot];
    let seed = trail_random_seed(step.effect_seed, sample.sequence as i8);
    let life = sample_life_span_msec(
        def.life_base,
        def.life_amp,
        random_table_u16(seed, FX_RAND_CH_LIFE),
    );
    let (born, start_origin, start_vz) = (
        sample.msec_begin,
        sample.origin,
        f32::from(sample.base_vel_z),
    );

    let spawn = FxOrientSpawnParams {
        spawn_origin: def.spawn_origin,
        spawn_offset_radius: [def.spawn_offset_radius_base, def.spawn_offset_radius_amp],
        spawn_offset_height: [def.spawn_offset_height_base, def.spawn_offset_height_amp],
        seed,
    };
    let kin = SampleKinematics {
        flags: def.flags,
        life_ms: life as f32,
        seed,
        gravity: elem_gravity_accel_z_sampled(def.gravity_base, def.gravity_amp, seed),
        orient: get_orientation(def.flags, step.now, step.alt, Some(spawn)),
        vel_local: step.vel_local,
        vel_world: step.vel_world,
    };
    // A sample is not moved before it exists.
    let from = step.prev_msec.max(born);

    if elem_uses_collision(def.flags) {
        let Some((origin, vz)) =
            sweep_with_collision(host, place, step, &kin, (born, life), (start_origin, start_vz), from, trace)
        else {
            return;
        };
        if let Some(sample) = host.trail_elems.get_mut(slot) {
            sample.origin = origin;
            sample.base_vel_z = trail_elem_base_vel_z_pack(vz);
        }
        return;
    }

    let ages = trail_elem_norm_ages(step.prev_msec, step.msec_now, born, life);
    if elem_update_has_velocity_graph(def.flags) {
        let dt = (step.msec_now.wrapping_sub(from) as f32) * 0.001;
        let (origin, vz) = kin.advance(start_origin, start_vz, dt, ages.0, ages.1);
        if let Some(sample) = host.trail_elems.get_mut(slot) {
            sample.origin = origin;
            sample.base_vel_z = trail_elem_base_vel_z_pack(vz);
        }
    } else {
        let origin = kin.graph_shift(start_origin, ages.0, ages.1);
        if let Some(sample) = host.trail_elems.get_mut(slot) {
            sample.origin = origin;
        }
    }
}

/// Moves a colliding sample through the slice in sub-steps, bouncing off what
/// it meets. `None` means nothing was done: no world was supplied, or the
/// sample died on touching something.
fn sweep_with_collision(
    host: &mut FxSystemHost,
    place: SamplePlace,
    step: &TrailStep<'_>,
    kin: &SampleKinematics<'_>,
    (born, life): (i32, i32),
    (mut origin, mut vz): ([f32; 3], f32),
    from: i32,
    trace: &mut TrailTrace<'_>,
) -> Option<([f32; 3], f32)> {
    let def = step.def;
    let mask = trace_mask(def.use_item_clip != 0);
    let o = kin.orient;

    for sub in collide_substep_schedule(from, step.msec_now) {
        let mut t0 = sub.msec_start;
        let t_end = sub.msec_end;

        for _ in 0..MAX_BOUNCES_PER_STEP {
            if t0 >= t_end {
                break;
            }
            let dt = (t_end.wrapping_sub(t0) as f32) * 0.001;
            let (age0, age1) = trail_elem_norm_ages(t0, t_end, born, life);
            let (free_end, free_vz) = kin.advance(origin, vz, dt, age0, age1);
            let start_w = orientation_pos_to_world(o.origin, o.axis, origin);
            let end_w = orientation_pos_to_world(o.origin, o.axis, free_end);

            let Some(hit) = trace(start_w, end_w, def.coll_mins, def.coll_maxs, mask) else {
                host.gaps.raise(FxGapCause::NoWorldClipForCollide {
                    def_index: step.def_index,
                });
                return None;
            };
            if hit.startsolid || hit.allsolid {
                vz = 0.0;
                break;
            }
            if hit.fraction >= 1.0 {
                origin = free_end;
                vz = free_vz;
                break;
            }

            let hit_w = [
                start_w[0] + (end_w[0] - start_w[0]) * hit.fraction,
                start_w[1] + (end_w[1] - start_w[1]) * hit.fraction,
                start_w[2] + (end_w[2] - start_w[2]) * hit.fraction,
            ];
            origin = orientation_pos_from_world(o.origin, o.axis, hit_w);

            let slice_ms = (t_end.saturating_sub(t0)).max(0) as f32;
            let age_ms = (t0.saturating_sub(born)).max(0) as f32 + slice_ms * hit.fraction;
            let impact_vel = get_velocity_at_time(
                def.flags,
                [0.0, 0.0, free_vz],
                age_ms,
                kin.life_ms,
                kin.vel_local,
                kin.vel_world,
                step.now.axis,
                kin.seed,
            );
            if def.has_effect_on_impact && impact_child_speed_allows(vec3_length_sq(impact_vel)) {
                let (parent_def_name, catalog_index) = host
                    .effect_at(place.effect_slot)
                    .map_or((String::new(), FX_CATALOG_INDEX_NONE), |e| {
                        (e.def_name.clone(), e.catalog_index)
                    });
                host.pending_trail_impacts.push(PendingTrailImpact {
                    parent_def_name,
                    catalog_index,
                    def_index: step.def_index,
                    origin: hit_w,
                    pre_vel: impact_vel,
                    msec: step.msec_now,
                });
            }
            if elem_dies_on_touch(def.flags) {
                drop_sample(host, place.effect_slot, place.trail_slot, place.handle, place.before);
                return None;
            }

            let reflection =
                sample_reflection_factor(def.reflection_base, def.reflection_amp, kin.seed);
            let bounce =
                collision_reflect_base_vel_delta([0.0, 0.0, free_vz], hit.normal, reflection);
            vz = free_vz + bounce[2];

            t0 += (slice_ms * hit.fraction) as i32;
            if t0 <= sub.msec_start {
                t0 = sub.msec_start.saturating_add(1);
            }
        }
    }
    Some((origin, vz))
}

// --- laying new samples ---------------------------------------------------------

/// Lays a sample on a trail at the given pose. Returns whether one was added.
pub(crate) fn update_trail(
    host: &mut FxSystemHost,
    effect_slot: usize,
    trail_handle: u16,
    elem_def: FxElemDefInfo,
    sample_origin: [f32; 3],
    sample_axis: [[f32; 3]; 3],
    msec: i32,
    spawn_dist: f32,
) -> bool {
    let Some(trail_slot) = live_trail(host, trail_handle) else {
        return false;
    };
    let effect_seed = match host.effect_at(effect_slot) {
        Some(e) if e.ring_resident => e.random_seed,
        _ => return false,
    };

    let sequence = host.trails[trail_slot].sequence;
    let seed = trail_random_seed(effect_seed, sequence);

    let mut msec_begin = elem_def.delay_base.wrapping_add(msec);
    if elem_def.delay_amp != 0 {
        let delay = sample_life_span_msec(
            0,
            elem_def.delay_amp,
            random_table_u16(seed, FX_RAND_CH_DELAY),
        );
        msec_begin = msec_begin.wrapping_add(delay);
    }
    let life_ms = sample_life_span_msec(
        elem_def.life_base,
        elem_def.life_amp,
        random_table_u16(seed, FX_RAND_CH_LIFE),
    );
    if !elem_def.keep_alive_by_child() && host.msec_now >= life_ms.wrapping_add(msec_begin) {
        return false;
    }

    let Some(slot) = alloc_trail_elem(host) else {
        return false;
    };
    let handle = trail_elem_handle_for_slot(slot as u32);

    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.status = effect.status.wrapping_add(1);
    }

    let tail = host.trails[trail_slot].last_elem_handle;
    if tail == FX_TRAIL_HANDLE_NONE {
        host.trails[trail_slot].first_elem_handle = handle;
    } else if let Some(tail_slot) = trail_elem_slot_for_handle(tail) {
        host.trail_elems[tail_slot].next_trail_elem_handle = handle;
    }

    host.trail_elems[slot] = FxTrailElemSlot {
        occupied: true,
        origin: spawn_origin_world(
            sample_origin,
            sample_axis,
            elem_def.spawn_origin,
            elem_def.flags,
            elem_def.spawn_offset_radius_base,
            elem_def.spawn_offset_radius_amp,
            elem_def.spawn_offset_height_base,
            elem_def.spawn_offset_height_amp,
            seed,
        ),
        spawn_dist,
        msec_begin,
        next_trail_elem_handle: FX_TRAIL_HANDLE_NONE,
        base_vel_z: 0,
        basis: compress_basis_from_axis(sample_axis),
        sequence: sequence as u8,
    };
    let trail = &mut host.trails[trail_slot];
    trail.last_elem_handle = handle;
    trail.sequence = sequence.wrapping_add(1);
    true
}

/// Drops the newest sample of a trail, which stood in for the moving end of
/// the ribbon, so a fresh one can take its place. A trail with a single
/// sample keeps it.
fn retire_moving_end(host: &mut FxSystemHost, effect_slot: usize, trail_slot: usize) {
    let newest = host.trails[trail_slot].last_elem_handle;
    let mut at = host.trails[trail_slot].first_elem_handle;
    while let Some(slot) = trail_elem_slot_for_handle(at) {
        let next = host.trail_elems[slot].next_trail_elem_handle;
        if next == newest {
            drop_sample(host, effect_slot, trail_slot, newest, at);
            let trail = &mut host.trails[trail_slot];
            trail.sequence = trail.sequence.wrapping_sub(1);
            return;
        }
        at = next;
    }
}

/// Brings every trail of an effect up to date for the time slice: starts
/// empty ones, and while the effect loops, swaps the moving end and adds the
/// samples the distance and time travelled call for.
pub(crate) fn update_effect_trails(
    host: &mut FxSystemHost,
    effect_slot: usize,
    def_index_begin: i32,
    def_index_end: i32,
    prev_msec: i32,
    msec_now: i32,
    camera_origin: [f32; 3],
    mut trail_def: impl FnMut(u16, u8) -> Option<FxElemDefInfo>,
) {
    let (catalog_index, mut handle, begin, end, distance, looping, born) =
        match host.effect_at(effect_slot) {
            Some(e) if e.ring_resident => (
                e.catalog_index,
                e.first_trail_handle,
                FxOrientFrame {
                    origin: e.origin_last,
                    axis: e.axis_last,
                },
                e.frame_now(),
                e.distance,
                e.status & FX_STATUS_HAS_PENDING_LOOP_ELEMS != 0,
                e.msec_begin,
            ),
            _ => return,
        };

    let moved = vec3_distance(begin.origin, end.origin);
    let turned = effect_orient_arc(begin.axis, end.axis);
    let advancing = looping && prev_msec < msec_now;

    while handle != FX_TRAIL_HANDLE_NONE {
        let Some(trail_slot) = live_trail(host, handle) else {
            break;
        };
        let trail = &host.trails[trail_slot];
        let next = trail.next_trail_handle;
        let part = trail.def_index as u8;
        let empty = trail.first_elem_handle == FX_TRAIL_HANDLE_NONE;
        let this = handle;
        handle = next;

        // A one-shot effect lays its first sample on its first update only.
        if !looping && (prev_msec != born || !empty) {
            continue;
        }
        if !(def_index_begin..def_index_end).contains(&i32::from(part)) {
            continue;
        }
        let Some(elem_def) = trail_def(catalog_index, part) else {
            host.gaps
                .raise(FxGapCause::TrailDefNotFound { def_index: part });
            continue;
        };

        if empty {
            update_trail(
                host,
                effect_slot,
                this,
                elem_def,
                begin.origin,
                begin.axis,
                prev_msec,
                distance,
            );
        } else if advancing {
            retire_moving_end(host, effect_slot, trail_slot);
        }
        if !advancing {
            continue;
        }

        let carried = host.trails[trail_slot].split_leftover;
        let (leftover, mut extra) = match trail_split_window(
            carried,
            elem_def.inv_split_time,
            (msec_now - prev_msec) as f32,
            elem_def.inv_split_dist,
            moved,
            elem_def.inv_split_arc_dist,
            turned,
        ) {
            FxTrailSplit::Hold { leftover } => (leftover, 0),
            FxTrailSplit::Interpolate { leftover, extra } => (leftover, extra),
        };
        let leftover = if leftover.is_finite() { leftover } else { 0.0 };

        // One update cannot add more samples than the pool holds. A huge
        // jump, or a split distance read as nearly zero, once asked for
        // billions and stalled the game for minutes: keep the newest pool's
        // worth.
        let pool = FX_TRAIL_ELEM_POOL_CAPACITY as i32;
        if extra > pool {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                diag::warn!(
                    World,
                    "fx: a trail wanted {extra} samples in one update (split distance {}, moved {moved}); limited to {pool}",
                    elem_def.inv_split_dist
                );
            });
            extra = pool;
        }
        host.trails[trail_slot].split_leftover = leftover;

        let total = leftover + extra as f32;
        for k in 1..=extra {
            let t = trail_split_interpolant_t(k as f32, carried, total);
            let at = trail_split_lerp_origin(begin.origin, end.origin, t);
            let seq = host.trails[trail_slot].sequence as u8;
            if trail_split_skips_update(
                seq,
                elem_def.spawn_range_base,
                elem_def.spawn_range_amp,
                camera_origin,
                at,
            ) {
                host.trails[trail_slot].sequence = (seq as i8).wrapping_add(1);
                continue;
            }
            update_trail(
                host,
                effect_slot,
                this,
                elem_def,
                at,
                trail_split_lerp_axis(begin.axis, end.axis, t),
                trail_split_interpolant_msec(prev_msec, msec_now, t),
                distance + moved * t,
            );
        }
        update_trail(
            host,
            effect_slot,
            this,
            elem_def,
            end.origin,
            end.axis,
            msec_now,
            distance + moved,
        );
    }
}

// --- the mid-frame pass ---------------------------------------------------------

/// Whether a sample is still within its life at `msec_now`; a trail whose
/// definition is unknown keeps everything.
fn sample_expired(
    elem_def: Option<FxElemDefInfo>,
    effect_seed: u16,
    sample: &FxTrailElemSlot,
    msec_now: i32,
) -> bool {
    let Some(def) = elem_def else {
        return false;
    };
    let seed = trail_random_seed(effect_seed, sample.sequence as i8);
    let life = sample_life_span_msec(
        def.life_base,
        def.life_amp,
        random_table_u16(seed, FX_RAND_CH_LIFE),
    );
    !trail_elem_keep(msec_now, sample.msec_begin, life)
}

/// Frees the run of expired samples at the front of a trail, keeping the
/// last expired one so the ribbon's old edge has something to fade from.
/// Returns true when the whole trail had expired and its last sample went.
fn prune_expired_front(
    host: &mut FxSystemHost,
    effect_slot: usize,
    trail_slot: usize,
    elem_def: Option<FxElemDefInfo>,
    effect_seed: u16,
    msec_now: i32,
) -> bool {
    let mut front = FX_TRAIL_HANDLE_NONE;
    let mut cur = host.trails[trail_slot].first_elem_handle;
    let mut all_expired = true;
    while cur != FX_TRAIL_HANDLE_NONE {
        let Some(slot) = live_sample(host, cur) else {
            break;
        };
        let sample = &host.trail_elems[slot];
        let after = sample.next_trail_elem_handle;
        if !sample_expired(elem_def, effect_seed, sample, msec_now) {
            all_expired = false;
        } else if all_expired && front != FX_TRAIL_HANDLE_NONE {
            drop_sample(host, effect_slot, trail_slot, front, FX_TRAIL_HANDLE_NONE);
        }
        front = cur;
        cur = after;
    }
    let newest = host.trails[trail_slot].last_elem_handle;
    if all_expired && front != FX_TRAIL_HANDLE_NONE && front == newest {
        drop_sample(host, effect_slot, trail_slot, front, FX_TRAIL_HANDLE_NONE);
        return true;
    }
    false
}

/// Ages the samples of every trail of an effect to `msec_now` and, while the
/// effect loops, points its newest sample at where the effect is now.
pub(crate) fn apply_partial_last_trail_spawn_dist(
    host: &mut FxSystemHost,
    effect_slot: usize,
    prev_msec: i32,
    msec_now: i32,
    spawn_dist: f32,
    looping: bool,
    mut trail_def: impl FnMut(u16, u8) -> Option<FxElemDefInfo>,
    mut trace: impl FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> Option<FxTrailCollideHit>,
    mut vel_graphs: impl FnMut(u16, u8) -> (Vec<FxElemVec3Range>, Vec<FxElemVec3Range>),
) {
    let (catalog_index, now, alt, effect_seed, mut handle) = match host.effect_at(effect_slot) {
        Some(e) if e.ring_resident => (
            e.catalog_index,
            e.frame_now(),
            e.frame_when_played(),
            e.random_seed,
            e.first_trail_handle,
        ),
        _ => return,
    };

    while handle != FX_TRAIL_HANDLE_NONE {
        let Some(trail_slot) = live_trail(host, handle) else {
            break;
        };
        handle = host.trails[trail_slot].next_trail_handle;
        let part = host.trails[trail_slot].def_index as u8;
        let elem_def = trail_def(catalog_index, part);

        if prune_expired_front(host, effect_slot, trail_slot, elem_def, effect_seed, msec_now) {
            continue;
        }

        if prev_msec != msec_now
            && let Some(def) = elem_def
        {
            let (vel_local, vel_world) = vel_graphs(catalog_index, part);
            let step = TrailStep {
                def,
                def_index: part,
                prev_msec,
                msec_now,
                now: &now,
                alt: &alt,
                effect_seed,
                vel_local: &vel_local,
                vel_world: &vel_world,
            };
            let mut before = FX_TRAIL_HANDLE_NONE;
            let mut cur = host.trails[trail_slot].first_elem_handle;
            while let Some(slot) = live_sample(host, cur) {
                let after = host.trail_elems[slot].next_trail_elem_handle;
                let place = SamplePlace {
                    effect_slot,
                    trail_slot,
                    handle: cur,
                    before,
                };
                move_sample(host, place, &step, &mut trace);
                if host.trail_elems[slot].occupied {
                    before = cur;
                }
                cur = after;
            }
        }

        if looping {
            retarget_newest_sample(host, trail_slot, elem_def, effect_seed, &now, spawn_dist);
        }
    }
}

/// Re-pins a trail's newest sample to the effect's current pose.
fn retarget_newest_sample(
    host: &mut FxSystemHost,
    trail_slot: usize,
    elem_def: Option<FxElemDefInfo>,
    effect_seed: u16,
    now: &FxOrientFrame,
    spawn_dist: f32,
) {
    let newest = host.trails[trail_slot].last_elem_handle;
    let Some(slot) = live_sample(host, newest) else {
        return;
    };
    let sample = &mut host.trail_elems[slot];
    sample.spawn_dist = spawn_dist;
    if let Some(def) = elem_def {
        sample.origin = spawn_origin_world(
            now.origin,
            now.axis,
            def.spawn_origin,
            def.flags,
            def.spawn_offset_radius_base,
            def.spawn_offset_radius_amp,
            def.spawn_offset_height_base,
            def.spawn_offset_height_amp,
            trail_random_seed(effect_seed, sample.sequence as i8),
        );
        sample.basis = compress_basis_from_axis(now.axis);
    }
}
