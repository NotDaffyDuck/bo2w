//! Bringing an effect to life and putting parts into it: the part pool, the
//! trails an effect owns, and the ways an effect and its owned effects end.

use std::ops::ControlFlow;

use fx_t6::runtime::{
    FX_ELEM_RUN_RELATIVE_TO_EFFECT, FX_ELEM_RUN_RELATIVE_TO_SPAWN, ELEM_RUNNER_ROLLS_AT_START,
    PARTICLE_SHAPE_STREAK, DICE_LANE_WAIT, DICE_LANE_LIFESPAN, DICE_LANE_BURST_TOTAL,
    EMITTER_REPEAT_QUEUED_BIT, ParticleShape, begin_iterating_over_effects_exclusive,
    cull_elem_for_spawn_allows, elem_random_seed, elem_run_mode, elem_runs_relative_to_offset,
    end_iterating_over_effects, end_iterating_runs_gc, looping_catchup_begin,
    looping_spawn_schedule, random_table_u16, randomly_rotate_axis, runner_rand_rot_degrees,
    sample_life_span_msec, sample_oneshot_spawn_count, spawn_effect_status, spawn_origin_world,
    status_is_unique_done, stop_effect_has_owned, stop_effect_non_recursive_allows, vec3_distance,
    world_delta_to_local,
};

use crate::recipe::{EmitterRecipeSummary, ParticleRecipeSummary};
use crate::particle_bin::{NO_PARTICLE_TICKET, ParticleBinSlot};
use crate::particle_bin::{particle_category_of_shape, particle_ticket_of_bin, particle_bin_of_ticket};
use crate::depth_order::order_fresh_particles;
use crate::host::{EffectSystemHost, QueuedDecalBirth, QueuedSpawnerBirth, QueuedSound};
use crate::ribbon::{
    NO_STREAK_TICKET, claim_ribbon, live_sample, live_trail, release_trail, release_trail_elem,
    ribbon_index_of,
};

// ===== claims on an effect ============================================================

/// Takes `count` claims off the effect in `effect_slot`. Says whether the
/// effect had been down to its last claim before this.
fn release_claims(sys: &mut EffectSystemHost, fx_index: usize, count: u32) -> bool {
    let Some(effect) = sys.live_effect_mut(fx_index) else {
        return false;
    };
    let was_unique = status_is_unique_done(effect.state_bits);
    effect.state_bits = effect.state_bits.wrapping_sub(count);
    was_unique
}

// ===== the part pool ===================================================================

/// Pops the head of the free list, or reports the pool as used up.
fn take_free_elem(sys: &mut EffectSystemHost) -> Option<usize> {
    let head = sys.first_spare_particle?;
    let Some(entry) = sys.elems.get(head) else {
        sys.first_spare_particle = None;
        return None;
    };
    sys.first_spare_particle = match entry.following_particle_ticket {
        NO_PARTICLE_TICKET => None,
        next => Some(usize::from(next)),
    };
    sys.live_parts = sys.live_parts.saturating_add(1);
    Some(head)
}

/// Makes `slot` the new head of the free list. Neighbours in a live list
/// are not touched; unlink first when they are still in use.
fn give_back_elem(sys: &mut EffectSystemHost, at: usize) {
    let behind = match sys.first_spare_particle {
        Some(head) => head as u16,
        None => NO_PARTICLE_TICKET,
    };
    let entry = &mut sys.elems[at];
    *entry = ParticleBinSlot::default();
    entry.following_particle_ticket = behind;
    sys.first_spare_particle = Some(at);
    sys.live_parts = sys.live_parts.saturating_sub(1);
}

/// Cuts one part out of its effect's list and frees it. If that took the
/// effect's last claim, a collection is requested.
pub(crate) fn release_particle(sys: &mut EffectSystemHost, elem_handle: u16) {
    let handle = elem_handle;
    let Some(slot) = particle_bin_of_ticket(handle).filter(|&s| sys.elems[s].is_taken) else {
        return;
    };
    let entry = &sys.elems[slot];
    let (prev, next) = (entry.preceding_particle_ticket, entry.following_particle_ticket);
    let (owner, class) = (usize::from(entry.parent_emitter_slot), usize::from(entry.category_slot));

    if let Some(effect) = sys.live_effect_mut(owner) {
        let list_head = effect.lead_particle_ticket.get_mut(class);
        if let Some(head) = list_head.filter(|head| prev == NO_PARTICLE_TICKET && **head == handle) {
            *head = next;
        }
        let sorted_head = &mut effect.first_part_ticket;
        if class == 0 && *sorted_head == handle {
            *sorted_head = next;
        }
    }
    if let Some(before) = particle_bin_of_ticket(prev) {
        sys.elems[before].following_particle_ticket = next;
    }
    if let Some(after) = particle_bin_of_ticket(next) {
        sys.elems[after].preceding_particle_ticket = prev;
    }

    give_back_elem(sys, slot);
    if release_claims(sys, owner, 1) {
        sys.gc_pending = true;
    }
}

/// Frees every part an effect has. The lists are not kept tidy part by
/// part; the effect's list heads are cleared once at the end.
pub(crate) fn free_all_elems_for_effect(sys: &mut EffectSystemHost, owner_slot: usize) {
    let effect_slot = owner_slot;
    let heads = sys
        .emitter_in_slot(effect_slot)
        .map_or([NO_PARTICLE_TICKET; 3], |e| e.lead_particle_ticket);
    let walk = |head: u16| {
        std::iter::successors(particle_bin_of_ticket(head), |&at| {
            particle_bin_of_ticket(sys.elems[at].following_particle_ticket)
        })
    };
    let doomed: Vec<usize> = heads.into_iter().flat_map(walk).collect();

    for slot in doomed {
        if !sys.elems[slot].is_taken {
            continue;
        }
        let owner = usize::from(sys.elems[slot].parent_emitter_slot);
        give_back_elem(sys, slot);
        release_claims(sys, owner, 1);
    }
    if let Some(effect) = sys.live_effect_mut(effect_slot) {
        effect.lead_particle_ticket = [NO_PARTICLE_TICKET; 3];
        effect.first_part_ticket = NO_PARTICLE_TICKET;
    }
}

/// Frees the samples along one trail; says how many there were.
fn free_trail_samples(sys: &mut EffectSystemHost, ribbon_at: usize) -> u32 {
    let first = sys.trails[ribbon_at].lead_particle_ticket;
    let samples: Vec<usize> = std::iter::successors(live_sample(sys, first), |&at| {
        live_sample(sys, sys.trail_elems[at].next_trail_elem_handle)
    })
    .collect();
    for &sample in &samples {
        release_trail_elem(sys, sample);
    }
    let trail = &mut sys.trails[ribbon_at];
    trail.lead_particle_ticket = NO_STREAK_TICKET;
    trail.last_elem_handle = NO_STREAK_TICKET;
    samples.len() as u32
}

/// Frees every trail an effect has, samples included.
pub(crate) fn free_all_trails_for_effect(sys: &mut EffectSystemHost, owner_slot: usize) {
    let fx_index = owner_slot;
    let first = sys
        .emitter_in_slot(fx_index)
        .map_or(NO_STREAK_TICKET, |e| e.first_ribbon_ticket);
    let trails: Vec<usize> = std::iter::successors(live_trail(sys, first), |&at| {
        live_trail(sys, sys.trails[at].next_trail_handle)
    })
    .collect();

    for trail in trails {
        let samples = free_trail_samples(sys, trail);
        if samples > 0 {
            release_claims(sys, fx_index, samples);
        }
        release_trail(sys, trail);
    }
    if let Some(effect) = sys.live_effect_mut(fx_index) {
        effect.first_ribbon_ticket = NO_STREAK_TICKET;
    }
}

// ===== what a spawn knows ==============================================================

/// A position and facing.
#[derive(Clone, Copy)]
struct Frame {
    pos: [f32; 3],
    basis: [[f32; 3]; 3],
}

/// The effect a part comes from, as read when the spawn starts.
struct EmitSource {
    cell: usize,
    frame: Frame,
    seed: u16,
    catalog: u16,
    bolt: u8,
    mark: Option<u16>,
}

impl EmitSource {
    /// None when the slot does not hold an effect named in the ring.
    fn read(sys: &EffectSystemHost, slot: usize) -> Option<Self> {
        let fx = sys.emitter_in_slot(slot)?;
        fx.in_ring.then(|| Self {
            cell: slot,
            frame: Frame {
                pos: fx.where_at,
                basis: fx.orient_rows,
            },
            seed: fx.dice_start,
            catalog: fx.ledger_index,
            bolt: fx.tether,
            mark: fx.stain_owner,
        })
    }

    fn name(&self, sys: &EffectSystemHost) -> String {
        sys.emitter_in_slot(self.cell)
            .map(|e| e.definition_name.clone())
            .unwrap_or_default()
    }
}

/// One part about to start: which definition, which of its copies, when.
struct Placement {
    definition_slot: u8,
    def: ParticleRecipeSummary,
    order_nth: u8,
    begin: i32,
    /// Everything random about the part comes from this.
    seed: u32,
}

impl Placement {
    fn new(src: &EmitSource, definition_slot: u8, def: ParticleRecipeSummary, nth: u8, birth_stamp_ms: i32) -> Self {
        let begin = birth_stamp_ms.wrapping_add(delay_before_start(src.seed, &def, nth, birth_stamp_ms));
        Self {
            definition_slot,
            def,
            order_nth: nth,
            begin,
            seed: elem_random_seed(src.seed, nth, begin),
        }
    }

    /// The part's spawn point in world space.
    fn scene_anchor(&self, src: &EmitSource) -> [f32; 3] {
        let d = &self.def;
        let offsets = (
            d.birth_ring_reach_floor,
            d.birth_ring_reach_spread,
            d.birth_lift_floor,
            d.birth_lift_spread,
        );
        spawn_origin_world(
            src.frame.pos,
            src.frame.basis,
            d.birth_point,
            d.bit_set,
            offsets.0,
            offsets.1,
            offsets.2,
            offsets.3,
            self.seed,
        )
    }

    /// Where the part begins, in the space it is simulated in.
    fn local_start(&self, src: &EmitSource) -> [f32; 3] {
        let flags = self.def.bit_set;
        if elem_runs_relative_to_offset(flags) {
            return [0.0; 3];
        }
        let world = self.scene_anchor(src);
        let follows_effect = [FX_ELEM_RUN_RELATIVE_TO_SPAWN, FX_ELEM_RUN_RELATIVE_TO_EFFECT]
            .contains(&elem_run_mode(flags));
        if follows_effect {
            world_delta_to_local(world, src.frame.pos, src.frame.basis)
        } else {
            world
        }
    }
}

/// How long after `birth_stamp_ms` the part begins: fixed, or rolled between
/// the base and the base plus amplitude.
fn delay_before_start(emitter_dice_start: u16, def: &ParticleRecipeSummary, nth: u8, birth_stamp_ms: i32) -> i32 {
    if def.wait_spread == 0 {
        return def.wait_floor;
    }
    let rolled = elem_random_seed(emitter_dice_start, nth, birth_stamp_ms.wrapping_add(def.wait_floor));
    let roll = random_table_u16(rolled, DICE_LANE_WAIT);
    sample_life_span_msec(def.wait_floor, def.wait_spread, roll)
}

// ===== placing one part ================================================================

/// Where a part goes once it has been decided to make it.
enum Route {
    Sound,
    Decal,
    ScenarioDriver,
    Pool,
}

fn route_of(elem_type: u8) -> Route {
    match ParticleShape::from_u8(elem_type) {
        Some(ParticleShape::Sound) => Route::Sound,
        Some(ParticleShape::Decal) => Route::Decal,
        Some(ParticleShape::ScenarioDriver) => Route::ScenarioDriver,
        _ => Route::Pool,
    }
}

/// A part with a spawn range exists only while the camera is within the
/// distance band its spawn point falls in.
fn outside_spawn_range(sys: &EffectSystemHost, src: &EmitSource, place: &Placement) -> bool {
    let (Some(camera), true) = (sys.camera_origin, place.def.birth_span_spread != 0.0) else {
        return false;
    };
    let distance = vec3_distance(camera, place.scene_anchor(src));
    let (base, amp) = (place.def.birth_span_floor, place.def.birth_span_spread);
    !cull_elem_for_spawn_allows(base, amp, distance, 0, false)
}

/// Starts one part of an effect at `birth_stamp_ms`. Sounds, decals and runners
/// are queued for the layers that play them; the rest become pooled parts.
fn birth_particle(
    sys: &mut EffectSystemHost,
    fx_index: usize,
    (definition_slot, def): (u8, ParticleRecipeSummary),
    nth: u8,
    birth_stamp_ms: i32,
) {
    let Some(src) = EmitSource::read(sys, fx_index) else {
        return;
    };
    let place = Placement::new(&src, definition_slot, def, nth, birth_stamp_ms);
    if outside_spawn_range(sys, &src, &place) {
        return;
    }
    match route_of(def.part_shape) {
        Route::Sound => queue_sound(sys, &src, &place),
        Route::Decal => queue_decal(sys, &src, &place),
        Route::ScenarioDriver => queue_runner(sys, &src, &place),
        Route::Pool => place_in_pool(sys, &src, &place),
    }
}

fn queue_sound(sys: &mut EffectSystemHost, src: &EmitSource, place: &Placement) {
    let request = QueuedSound {
        anchor: place.scene_anchor(src),
        dice_start: place.seed,
        window_open_ms: place.begin,
        definition_slot: place.definition_slot,
        ledger_index: src.catalog,
        owner_name: src.name(sys),
    };
    sys.pending_sounds.push(request);
}

fn queue_decal(sys: &mut EffectSystemHost, src: &EmitSource, place: &Placement) {
    let request = QueuedDecalBirth {
        turn_rows: src.frame.basis,
        tether_ix: src.bolt,
        where_at: src.frame.pos,
        dice_start: place.seed,
        window_open_ms: place.begin,
        definition_slot: place.definition_slot,
        ledger_index: src.catalog,
        src_name: src.name(sys),
        stain_on_ent: src.mark,
    };
    sys.pending_decals.push(request);
}

fn queue_runner(sys: &mut EffectSystemHost, src: &EmitSource, place: &Placement) {
    let spun = (place.def.bit_set & ELEM_RUNNER_ROLLS_AT_START != 0).then(|| {
        (
            randomly_rotate_axis(src.frame.basis, place.seed),
            runner_rand_rot_degrees(place.seed),
        )
    });
    let request = QueuedSpawnerBirth {
        orient_rows: spun.map_or(src.frame.basis, |(axis, _)| axis),
        turn_deg: spun.map(|(_, degrees)| degrees),
        spot: place.scene_anchor(src),
        dice_start: place.seed,
        window_open_ms: place.begin,
        definition_slot: place.definition_slot,
        ledger_index: src.catalog,
        source_name: src.name(sys),
        stain_ent: src.mark,
    };
    sys.pending_runners.push(request);
}

/// The pool entry for a part that is about to go live.
fn live_elem(place: &Placement, src: &EmitSource, kind_no: usize, life: i32) -> ParticleBinSlot {
    let d = &place.def;
    let mut elem = ParticleBinSlot::default();
    elem.is_taken = true;
    elem.parent_emitter_slot = src.cell as u16;
    elem.category_slot = kind_no as u8;
    elem.definition_slot = place.definition_slot;
    elem.nth = place.order_nth;
    elem.part_kind = d.part_shape;
    elem.bits = d.bit_set;
    elem.look_variant_total = d.look_variant_total;
    elem.paint_rank = d.paint_rank;
    elem.window_open_ms = place.begin;
    elem.lifespan_window_ms = life;
    elem.at_point = place.local_start(src);
    elem.birth_point = d.birth_point;
    elem.birth_ring_reach = [d.birth_ring_reach_floor, d.birth_ring_reach_spread];
    elem.birth_lift = [d.birth_lift_floor, d.birth_lift_spread];
    elem
}

/// Takes a pool slot for a part and pushes it onto the front of its class
/// list in the owning effect. A part that would already be over is skipped.
fn place_in_pool(sys: &mut EffectSystemHost, src: &EmitSource, place: &Placement) {
    let Some(class) = particle_category_of_shape(place.def.part_shape) else {
        return;
    };
    let roll = random_table_u16(place.seed, DICE_LANE_LIFESPAN);
    let life = sample_life_span_msec(place.def.lifespan_floor, place.def.lifespan_spread, roll);
    let over_already = sys.clock_ms_now >= place.begin.wrapping_add(life);
    if over_already && !place.def.held_open_by_offspring() {
        return;
    }

    let Some(slot) = take_free_elem(sys) else {
        sys.part_take_failures = sys.part_take_failures.saturating_add(1);
        return;
    };
    let Some(fx) = sys.live_effect_mut(src.cell) else {
        give_back_elem(sys, slot);
        return;
    };
    let handle = particle_ticket_of_bin(slot as u32);
    let old_front = fx.lead_particle_ticket[class];
    fx.lead_particle_ticket[class] = handle;
    fx.state_bits = fx.state_bits.wrapping_add(1);

    let mut entry = live_elem(place, src, class, life);
    entry.following_particle_ticket = old_front;
    sys.elems[slot] = entry;
    if let Some(behind) = particle_bin_of_ticket(old_front) {
        sys.elems[behind].preceding_particle_ticket = handle;
    }
}

// ===== starting an effect ==============================================================

/// The one-shot count of a part, rolled from the effect's seed, spawned as
/// a burst at `birth_stamp_ms`.
fn spawn_burst(
    sys: &mut EffectSystemHost,
    fx_index: usize,
    definition_slot: u8,
    elem: ParticleRecipeSummary,
    birth_stamp_ms: i32,
) {
    let seed = sys.emitter_in_slot(fx_index).map_or(0, |e| e.dice_start);
    let roll = random_table_u16(u32::from(seed), DICE_LANE_BURST_TOTAL);
    let count = sample_oneshot_spawn_count(elem.birth_corner_a, elem.birth_corner_b, roll);
    for copy in 0..count.max(0) {
        birth_particle(sys, fx_index, (definition_slot, elem), copy as u8, birth_stamp_ms);
    }
}

/// Gives the effect a trail for part `definition_slot`, ahead of the trails it has.
/// Break means no more trails can be given.
fn attach_trail(sys: &mut EffectSystemHost, fx_index: usize, definition_slot: usize) -> ControlFlow<()> {
    let Some(handle) = claim_ribbon(sys, definition_slot as i8) else {
        return ControlFlow::Break(());
    };
    let Some(trail) = ribbon_index_of(handle) else {
        return ControlFlow::Continue(());
    };
    match sys.live_effect_mut(fx_index) {
        Some(effect) => {
            let previous_first = std::mem::replace(&mut effect.first_ribbon_ticket, handle);
            sys.trails[trail].next_trail_handle = previous_first;
            ControlFlow::Continue(())
        }
        None => {
            release_trail(sys, trail);
            ControlFlow::Break(())
        }
    }
}

/// A trail for every trail-type part. When the trail pool runs dry the
/// remaining trail parts go without.
fn give_trails(sys: &mut EffectSystemHost, fx_index: usize, def: &EmitterRecipeSummary<'_>) {
    let in_range = def.part_def_count().max(0) as usize;
    let trail_defs: Vec<usize> = def.elems[..in_range.min(def.elems.len())]
        .iter()
        .enumerate()
        .filter_map(|(k, elem)| (elem.part_shape == PARTICLE_SHAPE_STREAK).then_some(k))
        .collect();
    let _ = trail_defs
        .into_iter()
        .try_for_each(|k| attach_trail(sys, fx_index, k));
}

/// Where a part sits in an effect definition: looping parts come first,
/// then the one-shot ones, and the rest is not spawned at start.
enum StartRole {
    Looping,
    OneShot,
    Later,
}

fn start_role_of(def: &EmitterRecipeSummary<'_>, index: usize) -> StartRole {
    let looping_end = usize::try_from(def.repeat_total).unwrap_or(0);
    let one_shot_end = looping_end + usize::try_from(def.single_play_total).unwrap_or(0);
    match index {
        i if i < looping_end => StartRole::Looping,
        i if i < one_shot_end => StartRole::OneShot,
        _ => StartRole::Later,
    }
}

/// Makes a freshly allocated effect live: its status, its trails, the first
/// copy of each looping part and the bursts of the one-shot ones.
pub fn launch_fresh_effect(sys: &mut EffectSystemHost, fx_ref: u16, def: EmitterRecipeSummary<'_>) {
    let Some(slot) = sys.ring_index_of(fx_ref) else {
        return;
    };
    if let Some(effect) = sys.live_effect_mut(slot) {
        effect.state_bits = spawn_effect_status(def.loop_lifetime_millis);
    }
    give_trails(sys, slot, &def);

    let began = sys.emitter_in_slot(slot).map_or(sys.clock_ms_now, |e| e.window_open_ms);
    let startable = def
        .elems
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, elem)| elem.births_as_particle());
    for (index, elem) in startable {
        match start_role_of(&def, index) {
            StartRole::Looping => birth_particle(sys, slot, (index as u8, elem), 0, began),
            StartRole::OneShot => spawn_burst(sys, slot, index as u8, elem, began),
            StartRole::Later => {}
        }
    }

    order_fresh_particles(sys, slot, [0.0; 3]);
}

/// Spawns the looping parts that fall due in `[window_start, window_end]`.
pub fn birth_repeat_fraction(
    sys: &mut EffectSystemHost,
    target: usize,
    effect_def: EmitterRecipeSummary<'_>,
    window_start: i32,
    window_end: i32,
) {
    let played_at = sys.emitter_in_slot(target).map_or(window_start, |e| e.window_open_ms);
    let looping_parts = usize::try_from(effect_def.repeat_total).unwrap_or(0);

    let due: Vec<(u8, ParticleRecipeSummary, i32, i32)> = effect_def
        .elems
        .iter()
        .copied()
        .enumerate()
        .take(looping_parts)
        .filter(|(_, elem)| elem.births_as_particle())
        .flat_map(|(index, elem)| {
            let span = [elem.wait_floor, elem.wait_spread, elem.lifespan_floor, elem.lifespan_spread]
                .into_iter()
                .fold(0i32, i32::wrapping_add);
            let from = looping_catchup_begin(window_start, window_end, span);
            looping_spawn_schedule(played_at, from, window_end, elem.birth_corner_a, elem.birth_corner_b)
                .map(move |spawn| (index as u8, elem, spawn.sequence, spawn.due_ms))
        })
        .collect();

    for (index, elem, sequence, msec) in due {
        birth_particle(sys, target, (index, elem), sequence.min(0xff) as u8, msec);
    }
}

/// Re-sorts the effect's newest parts by distance from `from`.
pub fn order_effect_parts(sys: &mut EffectSystemHost, fx_ref: u16, from: [f32; 3]) {
    if let Some(slot) = sys.ring_index_of(fx_ref) {
        order_fresh_particles(sys, slot, from);
    }
}

// ===== stopping ========================================================================

/// Ends an effect's looping so it spawns no more parts, giving up the claim
/// the looping held.
pub fn drop_queued_repeat(sys: &mut EffectSystemHost, target: usize) {
    let fx_index = target;
    let Some(effect) = sys.live_effect_mut(fx_index) else {
        return;
    };
    if effect.state_bits & EMITTER_REPEAT_QUEUED_BIT == 0 {
        return;
    }
    effect.state_bits ^= EMITTER_REPEAT_QUEUED_BIT;
    let own = effect.self_ticket;
    if release_claims(sys, fx_index, 1) {
        sys.drop_effect_ref(own);
    }
}

/// Stops every live effect owned by `owner`, except the one holding that
/// handle itself.
fn stop_owned_by(sys: &mut EffectSystemHost, owner: u16) {
    sys.walker_total = begin_iterating_over_effects_exclusive(sys.walker_total);
    let oldest = sys.first_active_effect;
    let live_span = sys.first_new_effect.wrapping_sub(oldest) as u32;
    let owned_slots: Vec<usize> = (0..live_span)
        .map(|age| sys.ring_entry_handle(oldest.wrapping_add(age as i32) as u32))
        .filter(|&handle| handle != owner)
        .filter_map(|handle| sys.ring_index_of(handle))
        .filter(|&slot| sys.emitter_in_slot(slot).is_some_and(|e| e.self_ticket == owner))
        .collect();
    for slot in owned_slots {
        halt_effect_alone(sys, slot);
    }
    sys.walker_total = end_iterating_over_effects(sys.walker_total);
    if end_iterating_runs_gc(sys.walker_total, sys.gc_pending) {
        sys.collect_garbage();
    }
}

/// Stops an effect along with the effects it owns.
pub fn halt_effect_alone(sys: &mut EffectSystemHost, target: usize) {
    let fx_index = target;
    let Some((own, status)) = sys.emitter_in_slot(fx_index).map(|e| (e.self_ticket, e.state_bits)) else {
        return;
    };
    if !stop_effect_non_recursive_allows(status) {
        return;
    }
    drop_queued_repeat(sys, fx_index);
    if stop_effect_has_owned(status) {
        stop_owned_by(sys, own);
    }
    let down_to_self = sys
        .emitter_in_slot(fx_index)
        .is_some_and(|e| status_is_unique_done(e.state_bits));
    if down_to_self {
        sys.drop_effect_ref(own);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UNTURNED_BASIS;
    use crate::ribbon::claim_ribbon_point;
    use fx_t6::runtime::{
        PARTICLE_NEVER_SETTLED, FX_ELEM_RUN_RELATIVE_TO_OFFSET, BIRTH_UNATTACHED,
        FX_STATUS_REF_COUNT_LOW_MASK, trail_piece_ticket_for_slot,
    };

    const ORIGIN: [f32; 3] = [10.0, 20.0, 30.0];
    const ALWAYS_FACING: u8 = 0;
    const PUFF: u8 = 4;
    const MESH_KIND: u8 = 7;
    const AUDIO_KIND: u8 = 10;
    const SURFACE_STICKER: u8 = 11;
    const SPRINTER: u8 = 12;
    const BASE_STATUS: u32 = 0x4000_0001;

    fn part_of(elem_type: u8) -> ParticleRecipeSummary {
        ParticleRecipeSummary {
            part_shape: elem_type,
            lifespan_floor: 1000,
            look_variant_total: 1,
            ..ParticleRecipeSummary::default()
        }
    }

    fn burst_of(elem_type: u8, count: i32) -> ParticleRecipeSummary {
        ParticleRecipeSummary {
            birth_corner_a: count,
            ..part_of(elem_type)
        }
    }

    fn looping_of(elem_type: u8, interval: i32, max: i32) -> ParticleRecipeSummary {
        ParticleRecipeSummary {
            birth_corner_a: interval,
            birth_corner_b: max,
            ..part_of(elem_type)
        }
    }

    /// A host at msec 100 with one effect "fx/a" (catalog 7) played at msec 100.
    fn host_and_effect() -> (EffectSystemHost, u16, usize) {
        let mut host = EffectSystemHost::new();
        host.clock_ms_now = 100;
        let handle = host
            .start_effect("fx/a", ORIGIN, UNTURNED_BASIS, 100, BIRTH_UNATTACHED, false, 7)
            .unwrap();
        let slot = host.ring_index_of(handle).unwrap();
        (host, handle, slot)
    }

    fn effect_def(
        looping_life: i32,
        looping: i32,
        one_shot: i32,
        emission: i32,
        elems: &[ParticleRecipeSummary],
    ) -> EmitterRecipeSummary<'_> {
        EmitterRecipeSummary {
            loop_lifetime_millis: looping_life,
            repeat_total: looping,
            single_play_total: one_shot,
            spawn_total: emission,
            elems,
        }
    }

    /// Pool slots of the parts on one class list, head first.
    fn class_chain(host: &EffectSystemHost, fx_index: usize, class: usize) -> Vec<usize> {
        let head = host.emitter_in_slot(fx_index).unwrap().lead_particle_ticket[class];
        std::iter::successors(particle_bin_of_ticket(head), |&slot| {
            particle_bin_of_ticket(host.elems[slot].following_particle_ticket)
        })
        .collect()
    }

    fn trail_chain(host: &EffectSystemHost, effect_slot: usize) -> Vec<usize> {
        let head = host.emitter_in_slot(effect_slot).unwrap().first_ribbon_ticket;
        std::iter::successors(ribbon_index_of(head), |&slot| {
            ribbon_index_of(host.trails[slot].next_trail_handle)
        })
        .collect()
    }

    fn status_of(host: &EffectSystemHost, effect_slot: usize) -> u32 {
        host.emitter_in_slot(effect_slot).unwrap().state_bits
    }

    /// (sequence, window_open_ms) of the plain parts, newest first.
    fn made_parts(host: &EffectSystemHost, fx_index: usize) -> Vec<(u8, i32)> {
        class_chain(host, fx_index, 0)
            .iter()
            .map(|&s| (host.elems[s].nth, host.elems[s].window_open_ms))
            .collect()
    }

    #[test]
    fn a_burst_makes_its_count_of_linked_parts() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 3)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));

        assert_eq!(status_of(&host, slot), BASE_STATUS + 3);
        assert_eq!(host.live_parts, 3);
        let live = class_chain(&host, slot, 0);
        let mut sequences: Vec<u8> = live.iter().map(|&s| host.elems[s].nth).collect();
        sequences.sort();
        assert_eq!(sequences, [0, 1, 2]);
        // Whatever order the sort left them in, the back links follow it.
        assert_eq!(host.elems[live[0]].preceding_particle_ticket, NO_PARTICLE_TICKET);
        for pair in live.windows(2) {
            assert_eq!(
                host.elems[pair[1]].preceding_particle_ticket,
                particle_ticket_of_bin(pair[0] as u32)
            );
        }
        let first = host
            .elems
            .iter()
            .find(|e| e.is_taken && e.nth == 0)
            .unwrap();
        assert!(first.is_taken);
        assert_eq!((first.definition_slot, first.window_open_ms), (0, 100));
        assert_eq!((first.lifespan_window_ms, first.look_variant_total), (1000, 1));
        assert_eq!(first.parent_emitter_slot as usize, slot);
        assert_eq!(first.settled_share, PARTICLE_NEVER_SETTLED);
        assert_eq!(first.emission_carry, 0);
        assert_eq!(first.launch_speed_vec, [0.0; 3]);
    }

    #[test]
    fn parts_land_on_the_list_of_their_class() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [
            looping_of(PUFF, 50, 1),
            looping_of(MESH_KIND, 50, 1),
            looping_of(ALWAYS_FACING, 50, 1),
        ];
        launch_fresh_effect(&mut host, handle, effect_def(500, 3, 0, 0, &elems));

        for class in 0..3 {
            assert_eq!(class_chain(&host, slot, class).len(), 1);
        }
        let by_class: Vec<u8> = (0..3)
            .map(|c| host.elems[class_chain(&host, slot, c)[0]].definition_slot)
            .collect();
        assert_eq!(by_class, [2, 1, 0]);
        assert_eq!(host.elems[class_chain(&host, slot, 2)[0]].category_slot, 2);
        // The pending-loop claim plus three parts.
        assert_eq!(status_of(&host, slot), BASE_STATUS + 0x8001 + 3);
    }

    #[test]
    fn a_looping_life_decides_the_starting_status() {
        let (mut host, handle, slot) = host_and_effect();
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 0, 0, &[]));
        assert_eq!(status_of(&host, slot), BASE_STATUS);
        launch_fresh_effect(&mut host, handle, effect_def(-5, 0, 0, 0, &[]));
        assert_eq!(status_of(&host, slot), BASE_STATUS + 0x8001);
    }

    #[test]
    fn an_unknown_handle_starts_nothing() {
        let (mut host, _, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 2)];
        launch_fresh_effect(&mut host, 1, effect_def(0, 0, 1, 0, &elems));
        assert_eq!(host.live_parts, 0);
        assert_eq!(status_of(&host, slot), BASE_STATUS);
    }

    #[test]
    fn trail_parts_get_trails_and_never_parts() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [
            part_of(3),
            looping_of(ALWAYS_FACING, 50, 1),
            burst_of(3, 2),
            burst_of(ALWAYS_FACING, 1),
        ];
        launch_fresh_effect(&mut host, handle, effect_def(0, 2, 2, 0, &elems));

        let trails = trail_chain(&host, slot);
        let defs: Vec<i8> = trails.iter().map(|&t| host.trails[t].definition_slot).collect();
        assert_eq!(defs, [2, 0]);
        assert!(trails.iter().all(|&t| host.trails[t].taken));
        assert_eq!(host.live_parts, 2);
        let mut defs_made: Vec<u8> = class_chain(&host, slot, 0)
            .iter()
            .map(|&s| host.elems[s].definition_slot)
            .collect();
        defs_made.sort();
        assert_eq!(defs_made, [1, 3]);
    }

    #[test]
    fn trails_stop_being_handed_out_when_the_pool_is_dry() {
        let (mut host, handle, slot) = host_and_effect();
        host.ribbon_free_head = None;
        let elems = [part_of(3), part_of(3)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 2, 0, 0, &elems));
        assert!(trail_chain(&host, slot).is_empty());
        assert_eq!(
            host.emitter_in_slot(slot).unwrap().first_ribbon_ticket,
            NO_STREAK_TICKET
        );
    }

    #[test]
    fn emission_trails_count_but_emission_parts_do_not_spawn() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [
            looping_of(ALWAYS_FACING, 50, 1),
            part_of(3),
            burst_of(ALWAYS_FACING, 4),
        ];
        launch_fresh_effect(&mut host, handle, effect_def(0, 1, 0, 2, &elems));
        assert_eq!(trail_chain(&host, slot).len(), 1);
        assert_eq!(host.live_parts, 1);
    }

    #[test]
    fn looping_catch_up_numbers_and_times_the_due_parts() {
        let (mut host, _, slot) = host_and_effect();
        let elems = [looping_of(ALWAYS_FACING, 50, 10), part_of(3)];
        birth_repeat_fraction(&mut host, slot, effect_def(500, 2, 0, 0, &elems), 100, 260);
        assert_eq!(made_parts(&host, slot), [(3, 250), (2, 200), (1, 150)]);
        assert_eq!(status_of(&host, slot), BASE_STATUS + 3);
    }

    #[test]
    fn a_long_window_is_trimmed_to_the_parts_own_lifetime() {
        let (mut host, _, slot) = host_and_effect();
        let short_lived = ParticleRecipeSummary {
            lifespan_floor: 50,
            ..looping_of(ALWAYS_FACING, 1, 1000)
        };
        let elems = [short_lived];
        birth_repeat_fraction(&mut host, slot, effect_def(500, 1, 0, 0, &elems), 100, 700);
        let made = made_parts(&host, slot);
        // Spawns 650..=700 are numbered past a byte and stay at 255.
        assert_eq!(made.len(), 51);
        assert!(made.iter().all(|&(seq, _)| seq == 255));
        assert_eq!(made.first().unwrap().1, 700);
        assert_eq!(made.last().unwrap().1, 650);
    }

    #[test]
    fn looping_catch_up_counts_from_the_effects_own_start() {
        let (mut host, _, slot) = host_and_effect();
        host.live_effect_mut(slot).unwrap().window_open_ms = 40;
        let elems = [looping_of(ALWAYS_FACING, 50, 10)];
        birth_repeat_fraction(&mut host, slot, effect_def(500, 1, 0, 0, &elems), 100, 200);
        assert_eq!(made_parts(&host, slot), [(3, 190), (2, 140)]);
    }

    #[test]
    fn an_effect_out_of_the_ring_spawns_nothing() {
        let (mut host, handle, slot) = host_and_effect();
        host.live_effect_mut(slot).unwrap().in_ring = false;
        let looping = [looping_of(AUDIO_KIND, 10, 5), looping_of(ALWAYS_FACING, 10, 5)];
        birth_repeat_fraction(&mut host, slot, effect_def(500, 2, 0, 0, &looping), 0, 500);
        launch_fresh_effect(&mut host, handle, effect_def(0, 2, 0, 0, &looping));
        assert_eq!(host.live_parts, 0);
        assert!(host.pending_sounds.is_empty());
    }

    #[test]
    fn a_delay_pushes_the_start_back() {
        let (mut host, handle, slot) = host_and_effect();
        let late = ParticleRecipeSummary {
            wait_floor: 40,
            ..burst_of(ALWAYS_FACING, 1)
        };
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &[late]));
        assert_eq!(made_parts(&host, slot), [(0, 140)]);
    }

    #[test]
    fn a_random_delay_is_rolled_from_the_effect_seed() {
        let (mut host, handle, slot) = host_and_effect();
        let late = ParticleRecipeSummary {
            wait_floor: 10,
            wait_spread: 90,
            ..burst_of(ALWAYS_FACING, 1)
        };
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &[late]));
        let seed = host.emitter_in_slot(slot).unwrap().dice_start;
        let rolled = elem_random_seed(seed, 0, 100 + 10);
        let delay = sample_life_span_msec(10, 90, random_table_u16(rolled, DICE_LANE_WAIT));
        assert_eq!(made_parts(&host, slot), [(0, 100 + delay)]);
    }

    #[test]
    fn parts_already_dead_are_not_made_unless_a_child_keeps_them() {
        let (mut host, handle, slot) = host_and_effect();
        host.clock_ms_now = 500;
        let brief = ParticleRecipeSummary {
            lifespan_floor: 10,
            ..burst_of(ALWAYS_FACING, 1)
        };
        let kept = ParticleRecipeSummary {
            spawns_on_finish: true,
            ..brief
        };
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &[brief]));
        assert_eq!(host.live_parts, 0);
        assert_eq!(status_of(&host, slot), BASE_STATUS);
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &[kept]));
        assert_eq!(host.live_parts, 1);
    }

    #[test]
    fn a_full_pool_counts_the_failure_and_changes_nothing_else() {
        let (mut host, handle, slot) = host_and_effect();
        host.first_spare_particle = None;
        let elems = [burst_of(ALWAYS_FACING, 2)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        assert_eq!(host.part_take_failures, 2);
        assert_eq!(status_of(&host, slot), BASE_STATUS);
        assert_eq!(host.live_parts, 0);
    }

    #[test]
    fn a_free_list_head_past_the_pool_is_dropped() {
        let (mut host, handle, _) = host_and_effect();
        host.first_spare_particle = Some(host.elems.len());
        let elems = [burst_of(ALWAYS_FACING, 1)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        assert_eq!(host.first_spare_particle, None);
        assert_eq!(host.part_take_failures, 1);
    }

    #[test]
    fn the_free_list_advances_and_gives_slots_back_at_its_head() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 2)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        assert_eq!(host.first_spare_particle, Some(2));
        let newest = class_chain(&host, slot, 0)[0];
        release_particle(&mut host, particle_ticket_of_bin(newest as u32));
        assert_eq!(host.first_spare_particle, Some(newest));
        assert_eq!(host.elems[newest].following_particle_ticket, 2);
        assert!(!host.elems[newest].is_taken);
        assert_eq!(host.live_parts, 1);
    }

    #[test]
    fn a_sound_goes_to_the_sound_queue_with_the_parents_name() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(AUDIO_KIND, 1), burst_of(ALWAYS_FACING, 1)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 2, 0, &elems));
        assert_eq!(host.live_parts, 1);
        assert_eq!(host.pending_sounds.len(), 1);
        let seed = host.emitter_in_slot(slot).unwrap().dice_start;
        let sound = &host.pending_sounds[0];
        assert_eq!(sound.owner_name, "fx/a");
        assert_eq!(
            (sound.ledger_index, sound.definition_slot, sound.window_open_ms),
            (7, 0, 100)
        );
        assert_eq!(sound.dice_start, elem_random_seed(seed, 0, 100));
        assert_eq!(sound.anchor, ORIGIN);
    }

    #[test]
    fn a_decal_is_queued_with_the_effects_pose_and_bolt() {
        let (mut host, handle, slot) = host_and_effect();
        {
            let effect = host.live_effect_mut(slot).unwrap();
            effect.tether = 5;
            effect.stain_owner = Some(33);
        }
        let elems = [burst_of(SURFACE_STICKER, 1)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        let decal = &host.pending_decals[0];
        assert_eq!(decal.src_name, "fx/a");
        assert_eq!((decal.tether_ix, decal.stain_on_ent), (5, Some(33)));
        assert_eq!((decal.where_at, decal.turn_rows), (ORIGIN, UNTURNED_BASIS));
        assert_eq!(
            (decal.ledger_index, decal.definition_slot, decal.window_open_ms),
            (7, 0, 100)
        );
        assert_eq!(host.live_parts, 0);
    }

    #[test]
    fn a_runner_keeps_the_axis_unless_it_asks_for_a_random_roll() {
        let (mut host, handle, slot) = host_and_effect();
        let plain = burst_of(SPRINTER, 1);
        let spun = ParticleRecipeSummary {
            bit_set: ELEM_RUNNER_ROLLS_AT_START,
            ..plain
        };
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 2, 0, &[plain, spun]));
        assert_eq!(host.pending_runners.len(), 2);
        let first = &host.pending_runners[0];
        assert_eq!((first.orient_rows, first.turn_deg), (UNTURNED_BASIS, None));
        assert_eq!(first.spot, ORIGIN);
        assert_eq!(first.stain_ent, None);
        let seed = host.emitter_in_slot(slot).unwrap().dice_start;
        let rolled = elem_random_seed(seed, 0, 100);
        let second = &host.pending_runners[1];
        assert_eq!(second.dice_start, rolled);
        assert_eq!(second.orient_rows, randomly_rotate_axis(UNTURNED_BASIS, rolled));
        assert_eq!(second.turn_deg, Some(runner_rand_rot_degrees(rolled)));
        assert_eq!(second.definition_slot, 1);
    }

    #[test]
    fn a_spawn_range_only_applies_with_a_camera() {
        let (mut host, handle, _) = host_and_effect();
        let near_only = ParticleRecipeSummary {
            birth_span_floor: 0.0,
            birth_span_spread: 100.0,
            ..burst_of(ALWAYS_FACING, 1)
        };
        let ranged = [near_only];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &ranged));
        assert_eq!(host.live_parts, 1);

        host.camera_origin = Some([ORIGIN[0] + 50.0, ORIGIN[1], ORIGIN[2]]);
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &ranged));
        assert_eq!(host.live_parts, 2);

        host.camera_origin = Some([ORIGIN[0] + 500.0, ORIGIN[1], ORIGIN[2]]);
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &ranged));
        assert_eq!(host.live_parts, 2);

        let unranged = [burst_of(ALWAYS_FACING, 1)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &unranged));
        assert_eq!(host.live_parts, 3);
    }

    #[test]
    fn a_range_culled_sound_is_not_queued() {
        let (mut host, handle, _) = host_and_effect();
        host.camera_origin = Some([ORIGIN[0] + 500.0, ORIGIN[1], ORIGIN[2]]);
        let far = ParticleRecipeSummary {
            birth_span_spread: 100.0,
            ..burst_of(AUDIO_KIND, 1)
        };
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &[far]));
        assert!(host.pending_sounds.is_empty());
    }

    #[test]
    fn a_part_starts_at_the_spawn_point_in_the_space_it_runs_in() {
        let (mut host, handle, slot) = host_and_effect();
        let world = ParticleRecipeSummary {
            birth_point: [[5.0, 5.0], [0.0, 0.0], [0.0, 0.0]],
            ..burst_of(ALWAYS_FACING, 1)
        };
        let to_spawn = ParticleRecipeSummary {
            bit_set: FX_ELEM_RUN_RELATIVE_TO_SPAWN,
            ..world
        };
        let to_offset = ParticleRecipeSummary {
            bit_set: FX_ELEM_RUN_RELATIVE_TO_OFFSET,
            ..world
        };
        let elems = [world, to_spawn, to_offset];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 3, 0, &elems));
        let live = class_chain(&host, slot, 0);
        let origin_of = |def: u8| {
            let found = live.iter().find(|&&s| host.elems[s].definition_slot == def);
            host.elems[*found.unwrap()].at_point
        };
        let seed = host.emitter_in_slot(slot).unwrap().dice_start;
        let part_seed = elem_random_seed(seed, 0, 100);
        let at = spawn_origin_world(ORIGIN, UNTURNED_BASIS, world.birth_point, 0, 0.0, 0.0, 0.0, 0.0, part_seed);
        assert_ne!(at, ORIGIN);
        assert_eq!(origin_of(0), at);
        assert_eq!(origin_of(1), world_delta_to_local(at, ORIGIN, UNTURNED_BASIS));
        assert_eq!(origin_of(2), [0.0; 3]);
    }

    #[test]
    fn a_middle_part_is_unlinked_and_its_neighbours_join_up() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 3)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        let live = class_chain(&host, slot, 0);
        release_particle(&mut host, particle_ticket_of_bin(live[1] as u32));

        assert_eq!(class_chain(&host, slot, 0), [live[0], live[2]]);
        assert_eq!(
            host.elems[live[2]].preceding_particle_ticket,
            particle_ticket_of_bin(live[0] as u32)
        );
        assert_eq!(status_of(&host, slot), BASE_STATUS + 2);
        assert!(!host.gc_pending);
        // Freeing a slot that is already free does nothing.
        release_particle(&mut host, particle_ticket_of_bin(live[1] as u32));
        release_particle(&mut host, NO_PARTICLE_TICKET);
        assert_eq!(status_of(&host, slot), BASE_STATUS + 2);
        assert_eq!(host.live_parts, 2);
    }

    #[test]
    fn the_head_part_hands_the_list_and_the_sorted_head_to_the_next() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 2)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        let live = class_chain(&host, slot, 0);
        let head_handle = particle_ticket_of_bin(live[0] as u32);
        host.live_effect_mut(slot).unwrap().first_part_ticket = head_handle;
        release_particle(&mut host, head_handle);

        let effect = host.emitter_in_slot(slot).unwrap();
        let rest = particle_ticket_of_bin(live[1] as u32);
        assert_eq!(effect.lead_particle_ticket[0], rest);
        assert_eq!(effect.first_part_ticket, rest);
        assert_eq!(host.elems[live[1]].preceding_particle_ticket, NO_PARTICLE_TICKET);
    }

    #[test]
    fn the_sorted_head_is_left_alone_for_other_classes() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [looping_of(MESH_KIND, 50, 1)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 1, 0, 0, &elems));
        let only = class_chain(&host, slot, 1)[0];
        let h = particle_ticket_of_bin(only as u32);
        host.live_effect_mut(slot).unwrap().first_part_ticket = h;
        release_particle(&mut host, h);
        let effect = host.emitter_in_slot(slot).unwrap();
        assert_eq!(effect.lead_particle_ticket[1], NO_PARTICLE_TICKET);
        assert_eq!(effect.first_part_ticket, h);
    }

    #[test]
    fn freeing_the_last_claim_asks_for_a_collection() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [burst_of(ALWAYS_FACING, 2)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 1, 0, &elems));
        host.live_effect_mut(slot).unwrap().state_bits -= 1;
        let live = class_chain(&host, slot, 0);
        release_particle(&mut host, particle_ticket_of_bin(live[0] as u32));
        assert!(!host.gc_pending);
        release_particle(&mut host, particle_ticket_of_bin(live[1] as u32));
        assert!(host.gc_pending);
        assert_eq!(status_of(&host, slot) & FX_STATUS_REF_COUNT_LOW_MASK, 0);
    }

    #[test]
    fn freeing_every_part_empties_the_lists_without_a_collection() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [
            burst_of(ALWAYS_FACING, 2),
            burst_of(MESH_KIND, 1),
            burst_of(PUFF, 3),
        ];
        launch_fresh_effect(&mut host, handle, effect_def(0, 0, 3, 0, &elems));
        host.live_effect_mut(slot).unwrap().first_part_ticket = 0x1234;
        assert_eq!(host.live_parts, 6);
        free_all_elems_for_effect(&mut host, slot);

        let effect = host.emitter_in_slot(slot).unwrap();
        assert_eq!(effect.lead_particle_ticket, [NO_PARTICLE_TICKET; 3]);
        assert_eq!(effect.first_part_ticket, NO_PARTICLE_TICKET);
        assert_eq!(effect.state_bits, BASE_STATUS);
        assert_eq!(host.live_parts, 0);
        assert!(!host.gc_pending);
        assert!(host.elems.iter().all(|e| !e.is_taken));
        free_all_elems_for_effect(&mut host, slot);
        assert_eq!(host.live_parts, 0);
    }

    #[test]
    fn freeing_all_trails_takes_the_samples_and_their_claims() {
        let (mut host, handle, slot) = host_and_effect();
        let elems = [part_of(3), part_of(3)];
        launch_fresh_effect(&mut host, handle, effect_def(0, 2, 0, 0, &elems));
        let trails = trail_chain(&host, slot);
        assert_eq!(trails.len(), 2);

        let mut samples = Vec::new();
        for _ in 0..3 {
            let s = claim_ribbon_point(&mut host).unwrap();
            host.trail_elems[s].is_taken = true;
            samples.push(s);
        }
        for pair in samples.windows(2) {
            host.trail_elems[pair[0]].next_trail_elem_handle =
                trail_piece_ticket_for_slot(pair[1] as u32);
        }
        host.trail_elems[samples[2]].next_trail_elem_handle = NO_STREAK_TICKET;
        host.trails[trails[0]].lead_particle_ticket = trail_piece_ticket_for_slot(samples[0] as u32);
        host.trails[trails[0]].last_elem_handle = trail_piece_ticket_for_slot(samples[2] as u32);
        host.live_effect_mut(slot).unwrap().state_bits += 3;

        free_all_trails_for_effect(&mut host, slot);
        assert_eq!(status_of(&host, slot), BASE_STATUS);
        assert_eq!(
            host.emitter_in_slot(slot).unwrap().first_ribbon_ticket,
            NO_STREAK_TICKET
        );
        assert!(trails.iter().all(|&t| !host.trails[t].taken));
        assert!(samples.iter().all(|&s| !host.trail_elems[s].is_taken));
        assert_eq!(host.ribbon_free_head, Some(trails[1]));
        assert!(!host.gc_pending);
    }

    #[test]
    fn ending_the_loop_gives_up_its_claim_once() {
        let (mut host, handle, slot) = host_and_effect();
        launch_fresh_effect(&mut host, handle, effect_def(500, 0, 0, 0, &[]));
        host.live_effect_mut(slot).unwrap().state_bits += 2;
        assert_eq!(status_of(&host, slot), BASE_STATUS + 0x8000 + 3);
        drop_queued_repeat(&mut host, slot);
        assert_eq!(status_of(&host, slot), BASE_STATUS + 2);
        assert!(!host.gc_pending);
        drop_queued_repeat(&mut host, slot);
        assert_eq!(status_of(&host, slot), BASE_STATUS + 2);
    }

    #[test]
    fn ending_the_loop_on_the_last_claim_asks_for_a_collection() {
        let (mut host, handle, slot) = host_and_effect();
        launch_fresh_effect(&mut host, handle, effect_def(500, 0, 0, 0, &[]));
        host.live_effect_mut(slot).unwrap().state_bits = EMITTER_REPEAT_QUEUED_BIT | 1;
        drop_queued_repeat(&mut host, slot);
        assert_eq!(status_of(&host, slot), 0);
        assert!(host.gc_pending);
        drop_queued_repeat(&mut host, 4000);
    }

    #[test]
    fn a_plain_stop_needs_a_claim_and_drops_the_loop_and_the_self_claim() {
        let (mut host, handle, slot) = host_and_effect();
        launch_fresh_effect(&mut host, handle, effect_def(500, 0, 0, 0, &[]));
        host.live_effect_mut(slot).unwrap().state_bits = 0x4000_0000;
        halt_effect_alone(&mut host, slot);
        assert!(!host.gc_pending);

        host.live_effect_mut(slot).unwrap().state_bits = BASE_STATUS + 0x8000 + 1;
        halt_effect_alone(&mut host, slot);
        assert_eq!(status_of(&host, slot), BASE_STATUS);
        assert!(host.gc_pending);
        halt_effect_alone(&mut host, 4000);
    }

    #[test]
    fn a_stop_with_other_claims_left_does_not_ask_for_a_collection() {
        let (mut host, handle, slot) = host_and_effect();
        launch_fresh_effect(&mut host, handle, effect_def(500, 0, 0, 0, &[]));
        host.live_effect_mut(slot).unwrap().state_bits += 4;
        halt_effect_alone(&mut host, slot);
        assert!(!host.gc_pending);
        assert_eq!(status_of(&host, slot), BASE_STATUS + 4);
    }

    #[test]
    fn stopping_an_owner_stops_what_it_owns_and_leaves_the_rest() {
        let (mut host, owner, owner_slot) = host_and_effect();
        let spawn = |host: &mut EffectSystemHost| {
            host.start_effect("fx/b", ORIGIN, UNTURNED_BASIS, 100, BIRTH_UNATTACHED, false, 1)
                .unwrap()
        };
        let owned = spawn(&mut host);
        let stranger = spawn(&mut host);
        let owned_slot = host.ring_index_of(owned).unwrap();
        let stranger_slot = host.ring_index_of(stranger).unwrap();
        let looping = BASE_STATUS + 0x8001;
        host.live_effect_mut(owned_slot).unwrap().self_ticket = owner;
        host.live_effect_mut(owned_slot).unwrap().state_bits = looping;
        host.live_effect_mut(stranger_slot).unwrap().state_bits = looping;
        host.live_effect_mut(owner_slot).unwrap().state_bits = BASE_STATUS | 0x0001_0000;
        let before = host.walker_total;

        halt_effect_alone(&mut host, owner_slot);

        assert_eq!(status_of(&host, owned_slot), BASE_STATUS);
        assert_eq!(status_of(&host, stranger_slot), looping);
        assert_eq!(host.walker_total, before);
        assert!(host.gc_pending);
    }
}
