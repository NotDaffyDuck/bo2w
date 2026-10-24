//! Starting an effect and filling it with parts, taking parts and trails
//! back out, and stopping an effect together with whatever it owns.

use fx_t6::runtime::{
    FX_ELEM_AT_REST_NONE, FX_ELEM_RUN_RELATIVE_TO_EFFECT, FX_ELEM_RUN_RELATIVE_TO_SPAWN,
    FX_ELEM_RUNNER_USES_RAND_ROT, FX_ELEM_TYPE_TRAIL, FX_RAND_CH_DELAY, FX_RAND_CH_LIFE,
    FX_RAND_CH_ONESHOT_COUNT, FX_STATUS_HAS_PENDING_LOOP_ELEMS, FX_STATUS_REF_COUNT_LOW_MASK,
    FxElemType, begin_iterating_over_effects_exclusive, cull_elem_for_spawn_allows,
    elem_random_seed, elem_run_mode, elem_runs_relative_to_offset, end_iterating_over_effects,
    end_iterating_runs_gc, looping_catchup_begin, looping_spawn_schedule, random_table_u16,
    randomly_rotate_axis, runner_rand_rot_degrees, sample_life_span_msec,
    sample_oneshot_spawn_count, spawn_effect_status, stop_effect_has_owned,
    stop_effect_non_recursive_allows, vec3_distance, world_delta_to_local,
};

use crate::def::{FxEffectDefInfo, FxElemDefInfo};
use crate::elem::{
    FX_ELEM_HANDLE_NONE, FxElemSlot, elem_class_for_type, elem_handle_for_slot,
    elem_slot_for_handle,
};
use crate::sort::sort_new_elems_in_effect;
use crate::system::{
    FxSystemHost, PendingDecalSpawn, PendingRunnerSpawn, PendingSoundSpawn,
};
use crate::trail::{
    FX_TRAIL_HANDLE_NONE, alloc_trail, live_sample, live_trail, release_trail, release_trail_elem,
    trail_slot_for_handle,
};

// --- starting an effect -----------------------------------------------------------

/// Brings a freshly allocated effect to life: sets its status, gives it its
/// trails, and starts its looping parts and its one-shot bursts.
pub fn start_new_effect(host: &mut FxSystemHost, effect_handle: u16, def: FxEffectDefInfo<'_>) {
    let Some(effect_slot) = host.slot_index_for_handle(effect_handle) else {
        return;
    };
    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.status = spawn_effect_status(def.msec_looping_life);
    }
    give_trails(host, effect_slot, &def);

    let spawn_msec = host
        .effect_at(effect_slot)
        .map_or(host.msec_now, |e| e.msec_begin);
    let looping = def.looping_count.max(0) as usize;
    let one_shot = def.one_shot_count.max(0) as usize;

    for (i, part) in def.elems.iter().copied().enumerate().take(looping) {
        if part.elem_type != FX_ELEM_TYPE_TRAIL {
            spawn_elem(host, effect_slot, i as u8, part, 0, spawn_msec);
        }
    }
    for (i, part) in def
        .elems
        .iter()
        .copied()
        .enumerate()
        .skip(looping)
        .take(one_shot)
    {
        spawn_burst(host, effect_slot, i as u8, part, spawn_msec);
    }

    sort_new_elems_in_effect(host, effect_slot, [0.0; 3]);
}

/// One trail per trail-type part, newest at the head of the effect's list.
/// When the pool runs dry the remaining trail parts go without.
fn give_trails(host: &mut FxSystemHost, effect_slot: usize, def: &FxEffectDefInfo<'_>) {
    let parts = def.total_elem_defs().max(0) as usize;
    let trail_parts = def
        .elems
        .iter()
        .take(parts)
        .enumerate()
        .filter(|(_, part)| part.elem_type == FX_ELEM_TYPE_TRAIL)
        .map(|(i, _)| i);
    for i in trail_parts {
        let Some(handle) = alloc_trail(host, i as i8) else {
            break;
        };
        let Some(trail_slot) = trail_slot_for_handle(handle) else {
            continue;
        };
        let Some(effect) = host.effect_at_mut(effect_slot) else {
            release_trail(host, trail_slot);
            return;
        };
        let older = std::mem::replace(&mut effect.first_trail_handle, handle);
        host.trails[trail_slot].next_trail_handle = older;
    }
}

/// Starts the parts that a looping effect still owes for the time slice
/// `[msec_update_begin, msec_update_end]`.
pub fn spawn_looping_partial(
    host: &mut FxSystemHost,
    effect_slot: usize,
    def: FxEffectDefInfo<'_>,
    msec_update_begin: i32,
    msec_update_end: i32,
) {
    let played_at = host
        .effect_at(effect_slot)
        .map_or(msec_update_begin, |e| e.msec_begin);
    let looping = def.looping_count.max(0) as usize;

    for (i, part) in def.elems.iter().copied().enumerate().take(looping) {
        if part.elem_type == FX_ELEM_TYPE_TRAIL {
            continue;
        }
        let lifetime = part
            .delay_base
            .wrapping_add(part.delay_amp)
            .wrapping_add(part.life_base)
            .wrapping_add(part.life_amp);
        let from = looping_catchup_begin(msec_update_begin, msec_update_end, lifetime);
        let due = looping_spawn_schedule(played_at, from, msec_update_end, part.spawn_a, part.spawn_b);
        for spawn in due {
            let sequence = spawn.sequence.min(i32::from(u8::MAX)) as u8;
            spawn_elem(host, effect_slot, i as u8, part, sequence, spawn.msec);
        }
    }
}

/// Re-sorts an effect's newly added parts by distance from `camera_origin`.
pub fn sort_effect_elems(host: &mut FxSystemHost, effect_handle: u16, camera_origin: [f32; 3]) {
    if let Some(slot) = host.slot_index_for_handle(effect_handle) {
        sort_new_elems_in_effect(host, slot, camera_origin);
    }
}

/// Starts a one-shot part: a burst of however many the part's count rolls.
fn spawn_burst(
    host: &mut FxSystemHost,
    effect_slot: usize,
    def_index: u8,
    part: FxElemDefInfo,
    spawn_msec: i32,
) {
    if part.elem_type == FX_ELEM_TYPE_TRAIL {
        return;
    }
    let seed = host.effect_at(effect_slot).map_or(0, |e| e.random_seed);
    let roll = random_table_u16(u32::from(seed), FX_RAND_CH_ONESHOT_COUNT);
    let count = sample_oneshot_spawn_count(part.spawn_a, part.spawn_b, roll);
    for sequence in 0..count.max(0) {
        spawn_elem(host, effect_slot, def_index, part, sequence as u8, spawn_msec);
    }
}

// --- placing one part -------------------------------------------------------------

/// Where an effect is and which way it faces.
#[derive(Clone, Copy)]
struct Pose {
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
}

/// What a part spawn reads from the effect that owns it.
struct Parent {
    pose: Pose,
    random_seed: u16,
    catalog_index: u16,
    bolt: u8,
    mark_entity: Option<u16>,
}

/// One part about to be placed.
struct Part {
    effect_slot: usize,
    def_index: u8,
    def: FxElemDefInfo,
    sequence: u8,
    msec_begin: i32,
    /// Seeds everything random about this part.
    seed: u32,
}

impl Part {
    fn world_point(&self, parent: &Parent) -> [f32; 3] {
        let def = &self.def;
        fx_t6::runtime::spawn_origin_world(
            parent.pose.origin,
            parent.pose.axis,
            def.spawn_origin,
            def.flags,
            def.spawn_offset_radius_base,
            def.spawn_offset_radius_amp,
            def.spawn_offset_height_base,
            def.spawn_offset_height_amp,
            self.seed,
        )
    }

    /// Where the part starts, in the space it is simulated in.
    fn start_origin(&self, parent: &Parent) -> [f32; 3] {
        if elem_runs_relative_to_offset(self.def.flags) {
            return [0.0; 3];
        }
        let world = self.world_point(parent);
        match elem_run_mode(self.def.flags) {
            FX_ELEM_RUN_RELATIVE_TO_SPAWN | FX_ELEM_RUN_RELATIVE_TO_EFFECT => {
                world_delta_to_local(world, parent.pose.origin, parent.pose.axis)
            }
            _ => world,
        }
    }
}

fn read_parent(host: &FxSystemHost, effect_slot: usize) -> Option<Parent> {
    let e = host.effect_at(effect_slot).filter(|e| e.ring_resident)?;
    Some(Parent {
        pose: Pose {
            origin: e.origin,
            axis: e.axis,
        },
        random_seed: e.random_seed,
        catalog_index: e.catalog_index,
        bolt: e.bolt,
        mark_entity: e.mark_entity,
    })
}

fn parent_def_name(host: &FxSystemHost, effect_slot: usize) -> String {
    host.effect_at(effect_slot)
        .map(|e| e.def_name.clone())
        .unwrap_or_default()
}

/// Decides what happens to one part of an effect at `spawn_msec`: sounds,
/// decals and runners are handed to the layers that play them; anything else
/// becomes a live part in the pool.
fn spawn_elem(
    host: &mut FxSystemHost,
    effect_slot: usize,
    def_index: u8,
    def: FxElemDefInfo,
    sequence: u8,
    spawn_msec: i32,
) {
    let Some(parent) = read_parent(host, effect_slot) else {
        return;
    };

    let delay = if def.delay_amp != 0 {
        let rolled = elem_random_seed(
            parent.random_seed,
            sequence,
            spawn_msec.wrapping_add(def.delay_base),
        );
        let roll = random_table_u16(rolled, FX_RAND_CH_DELAY);
        sample_life_span_msec(def.delay_base, def.delay_amp, roll)
    } else {
        def.delay_base
    };
    let msec_begin = spawn_msec.wrapping_add(delay);
    let part = Part {
        effect_slot,
        def_index,
        def,
        sequence,
        msec_begin,
        seed: elem_random_seed(parent.random_seed, sequence, msec_begin),
    };

    // A part with a spawn range only appears while the camera is inside the
    // distance tier its spawn point falls in.
    if let Some(camera) = host.camera_origin
        && def.spawn_range_amp != 0.0
    {
        let dist = vec3_distance(camera, part.world_point(&parent));
        if !cull_elem_for_spawn_allows(def.spawn_range_base, def.spawn_range_amp, dist, 0, false) {
            return;
        }
    }

    match FxElemType::from_u8(def.elem_type) {
        Some(FxElemType::Sound) => queue_sound(host, &part, &parent),
        Some(FxElemType::Decal) => queue_decal(host, &part, &parent),
        Some(FxElemType::Runner) => queue_runner(host, &part, &parent),
        _ => place_elem(host, &part, &parent),
    }
}

fn queue_sound(host: &mut FxSystemHost, part: &Part, parent: &Parent) {
    let parent_name = parent_def_name(host, part.effect_slot);
    host.pending_sounds.push(PendingSoundSpawn {
        parent_name,
        catalog_index: parent.catalog_index,
        def_index: part.def_index,
        msec_begin: part.msec_begin,
        random_seed: part.seed,
        origin: part.world_point(parent),
    });
}

fn queue_decal(host: &mut FxSystemHost, part: &Part, parent: &Parent) {
    let parent_name = parent_def_name(host, part.effect_slot);
    host.pending_decals.push(PendingDecalSpawn {
        mark_entity: parent.mark_entity,
        parent_name,
        catalog_index: parent.catalog_index,
        def_index: part.def_index,
        msec_begin: part.msec_begin,
        random_seed: part.seed,
        origin: parent.pose.origin,
        bolt: parent.bolt,
        axis: parent.pose.axis,
    });
}

fn queue_runner(host: &mut FxSystemHost, part: &Part, parent: &Parent) {
    let (axis, rot_deg) = if part.def.flags & FX_ELEM_RUNNER_USES_RAND_ROT != 0 {
        (
            randomly_rotate_axis(parent.pose.axis, part.seed),
            Some(runner_rand_rot_degrees(part.seed)),
        )
    } else {
        (parent.pose.axis, None)
    };
    let parent_name = parent_def_name(host, part.effect_slot);
    host.pending_runners.push(PendingRunnerSpawn {
        mark_entity: parent.mark_entity,
        parent_name,
        catalog_index: parent.catalog_index,
        def_index: part.def_index,
        msec_begin: part.msec_begin,
        random_seed: part.seed,
        origin: part.world_point(parent),
        axis,
        rot_deg,
    });
}

/// Takes a pool slot for a part and links it at the head of its class list
/// in the owning effect. A part that would already be dead is not made.
fn place_elem(host: &mut FxSystemHost, part: &Part, parent: &Parent) {
    let def = &part.def;
    let Some(class) = elem_class_for_type(def.elem_type) else {
        return;
    };
    let roll = random_table_u16(part.seed, FX_RAND_CH_LIFE);
    let life = sample_life_span_msec(def.life_base, def.life_amp, roll);
    if !def.keep_alive_by_child() && host.msec_now >= part.msec_begin.wrapping_add(life) {
        return;
    }

    let Some(slot) = alloc_elem(host) else {
        host.elem_alloc_failures = host.elem_alloc_failures.saturating_add(1);
        return;
    };
    let handle = elem_handle_for_slot(slot as u32);
    let Some(effect) = host.effect_at_mut(part.effect_slot) else {
        release_elem(host, slot);
        return;
    };
    effect.status = effect.status.wrapping_add(1);
    let older = std::mem::replace(&mut effect.first_elem_handle[class], handle);

    host.elems[slot] = FxElemSlot {
        occupied: true,
        def_index: part.def_index,
        elem_type: def.elem_type,
        flags: def.flags,
        visual_count: def.visual_count,
        sequence: part.sequence,
        at_rest_fraction: FX_ELEM_AT_REST_NONE,
        emit_residual: 0,
        next_elem_handle: older,
        prev_elem_handle: FX_ELEM_HANDLE_NONE,
        msec_begin: part.msec_begin,
        life_span_msec: life,
        base_vel: [0.0; 3],
        origin: part.start_origin(parent),
        spawn_origin: def.spawn_origin,
        spawn_offset_radius: [def.spawn_offset_radius_base, def.spawn_offset_radius_amp],
        spawn_offset_height: [def.spawn_offset_height_base, def.spawn_offset_height_amp],
        owner_effect_slot: part.effect_slot as u16,
        class_index: class as u8,
        sort_order: def.sort_order,
    };
    if let Some(older_slot) = elem_slot_for_handle(older) {
        host.elems[older_slot].prev_elem_handle = handle;
    }
}

// --- the part pool ------------------------------------------------------------------

fn alloc_elem(host: &mut FxSystemHost) -> Option<usize> {
    let slot = host.elem_first_free?;
    if slot >= host.elems.len() {
        host.elem_first_free = None;
        return None;
    }
    let link = host.elems[slot].next_elem_handle;
    host.elem_first_free = (link != FX_ELEM_HANDLE_NONE).then_some(usize::from(link));
    host.elem_live_count = host.elem_live_count.saturating_add(1);
    Some(slot)
}

/// Puts a slot back on the free list. The slot's links are not touched
/// beforehand; callers unlink first if the neighbours are still in use.
fn release_elem(host: &mut FxSystemHost, slot: usize) {
    let link = host.elem_first_free.map_or(FX_ELEM_HANDLE_NONE, |s| s as u16);
    host.elems[slot] = FxElemSlot {
        next_elem_handle: link,
        ..FxElemSlot::default()
    };
    host.elem_first_free = Some(slot);
    host.elem_live_count = host.elem_live_count.saturating_sub(1);
}

/// Unlinks one part from its effect's list and frees it. When that drops the
/// effect's last claim the host is flagged for a garbage collection.
pub(crate) fn free_elem(host: &mut FxSystemHost, handle: u16) {
    let Some(slot) = elem_slot_for_handle(handle) else {
        return;
    };
    let part = &host.elems[slot];
    if !part.occupied {
        return;
    }
    let (after, before) = (part.next_elem_handle, part.prev_elem_handle);
    let effect_slot = usize::from(part.owner_effect_slot);
    let class = usize::from(part.class_index);

    if before == FX_ELEM_HANDLE_NONE {
        if let Some(effect) = host.effect_at_mut(effect_slot)
            && class < 3
            && effect.first_elem_handle[class] == handle
        {
            effect.first_elem_handle[class] = after;
        }
    } else if let Some(before_slot) = elem_slot_for_handle(before) {
        host.elems[before_slot].next_elem_handle = after;
    }
    if let Some(after_slot) = elem_slot_for_handle(after) {
        host.elems[after_slot].prev_elem_handle = before;
    }
    if class == 0
        && let Some(effect) = host.effect_at_mut(effect_slot)
        && effect.first_sorted_elem_handle == handle
    {
        effect.first_sorted_elem_handle = after;
    }

    release_elem(host, slot);

    if let Some(effect) = host.effect_at_mut(effect_slot) {
        let last_claim = effect.status & FX_STATUS_REF_COUNT_LOW_MASK == 1;
        effect.status = effect.status.wrapping_sub(1);
        if last_claim {
            host.needs_garbage_collection = true;
        }
    }
}

/// Frees every part of an effect without bothering to keep the lists
/// consistent along the way; the lists are reset at the end.
pub(crate) fn free_all_elems_for_effect(host: &mut FxSystemHost, effect_slot: usize) {
    for class in 0..3 {
        let mut handle = host
            .effect_at(effect_slot)
            .map_or(FX_ELEM_HANDLE_NONE, |e| e.first_elem_handle[class]);
        while let Some(slot) = elem_slot_for_handle(handle) {
            let part = &host.elems[slot];
            let after = part.next_elem_handle;
            if part.occupied {
                let owner = usize::from(part.owner_effect_slot);
                release_elem(host, slot);
                if let Some(effect) = host.effect_at_mut(owner) {
                    effect.status = effect.status.wrapping_sub(1);
                }
            }
            handle = after;
        }
        if let Some(effect) = host.effect_at_mut(effect_slot) {
            effect.first_elem_handle[class] = FX_ELEM_HANDLE_NONE;
            if class == 0 {
                effect.first_sorted_elem_handle = FX_ELEM_HANDLE_NONE;
            }
        }
    }
}

/// Frees every sample of a trail and returns how many there were.
fn free_trail_samples(host: &mut FxSystemHost, trail_slot: usize) -> u32 {
    let mut freed = 0;
    let mut at = host.trails[trail_slot].first_elem_handle;
    while let Some(slot) = live_sample(host, at) {
        at = host.trail_elems[slot].next_trail_elem_handle;
        release_trail_elem(host, slot);
        freed += 1;
    }
    let trail = &mut host.trails[trail_slot];
    trail.first_elem_handle = FX_TRAIL_HANDLE_NONE;
    trail.last_elem_handle = FX_TRAIL_HANDLE_NONE;
    freed
}

/// Frees every trail of an effect together with its samples.
pub(crate) fn free_all_trails_for_effect(host: &mut FxSystemHost, effect_slot: usize) {
    let mut handle = host
        .effect_at(effect_slot)
        .map_or(FX_TRAIL_HANDLE_NONE, |e| e.first_trail_handle);
    while let Some(trail_slot) = live_trail(host, handle) {
        let next = host.trails[trail_slot].next_trail_handle;
        let freed = free_trail_samples(host, trail_slot);
        if freed > 0
            && let Some(effect) = host.effect_at_mut(effect_slot)
        {
            effect.status = effect.status.wrapping_sub(freed);
        }
        release_trail(host, trail_slot);
        handle = next;
    }
    if let Some(effect) = host.effect_at_mut(effect_slot) {
        effect.first_trail_handle = FX_TRAIL_HANDLE_NONE;
    }
}

// --- stopping -------------------------------------------------------------------------

/// Ends the effect's looping: it will spawn no more parts, and the claim the
/// looping held on it is given up.
pub fn stop_pending_loop(host: &mut FxSystemHost, effect_slot: usize) {
    let (handle, was_last) = match host.effect_at_mut(effect_slot) {
        Some(e) if e.status & FX_STATUS_HAS_PENDING_LOOP_ELEMS != 0 => {
            let was_last = e.status & FX_STATUS_REF_COUNT_LOW_MASK == 1;
            e.status &= !FX_STATUS_HAS_PENDING_LOOP_ELEMS;
            e.status = e.status.wrapping_sub(1);
            (e.own_handle, was_last)
        }
        _ => return,
    };
    if was_last {
        host.del_ref_to_effect(handle);
    }
}

/// Stops every live effect that shares `owner`'s owner handle, other than
/// the effect at `owner` itself.
fn stop_owned_by(host: &mut FxSystemHost, owner: u16) {
    host.iterator_count = begin_iterating_over_effects_exclusive(host.iterator_count);
    let end = host.first_new_effect;
    let mut cursor = host.first_active_effect;
    while cursor != end {
        let handle = host.handle_at_ring(cursor as u32);
        cursor = cursor.wrapping_add(1);
        if handle == owner {
            continue;
        }
        if let Some(other) = host.slot_index_for_handle(handle)
            && host.effect_at(other).is_some_and(|e| e.own_handle == owner)
        {
            stop_effect_non_recursive(host, other);
        }
    }
    host.iterator_count = end_iterating_over_effects(host.iterator_count);
    if end_iterating_runs_gc(host.iterator_count, host.needs_garbage_collection) {
        host.run_garbage_collection();
    }
}

/// Stops an effect and everything it owns.
pub fn stop_effect_non_recursive(host: &mut FxSystemHost, effect_slot: usize) {
    let Some((handle, status)) = host.effect_at(effect_slot).map(|e| (e.own_handle, e.status))
    else {
        return;
    };
    if !stop_effect_non_recursive_allows(status) {
        return;
    }
    stop_pending_loop(host, effect_slot);
    if stop_effect_has_owned(status) {
        stop_owned_by(host, handle);
    }
    let claims = host
        .effect_at(effect_slot)
        .map_or(0, |e| e.status & FX_STATUS_REF_COUNT_LOW_MASK);
    if claims == 1 {
        host.del_ref_to_effect(handle);
    }
}
