//! bo2mp: BO2's dropped guns on a map with BO2's weapon facts (the T6 brain on). The item
//! entity and the use button's aim pick stay `crate::floor_guns`'s; how a gun falls and lands,
//! what a touch takes, what a pick-up gives and swaps out, and what a thrown-down gun carries
//! follow BO2 on the T6 gun state (`weapon_t6::pickup`):
//! - G_RunItem: a falling gun moves to where it will be 50 ms on; a hit on a steep face slides
//!   along it; a gun that comes to rest on a floor lies along it on its side, and one that
//!   comes to rest on no floor, inside the world or in a no-drop volume is removed. A gun whose
//!   ground is gone falls again from where it is.
//! - LaunchItem: the item's box is 1 unit each way on every axis, and the player who threw it
//!   cannot touch or take it for a second (DroppedItemClearOwner).
//! - G_DoTouchTriggers: he touches a gun at most 36 units off his origin across, from 18 below
//!   his feet to 88 above them, and only inside his box (which follows his stance) grown 20
//!   units (BG_PlayerTouchesItem).
//! - Touch_Item / WeaponPickup_Touch: walking over a gun takes its reserve (and its magazine
//!   too when he holds that very gun) into his gun of that kind. The item keeps what is left
//!   and goes away once nothing is left on it.
//! - WeaponPickup_Grab: holding use on a gun he does not hold. With both primaries full the
//!   melee primary, else the primary in his hands, is thrown down where the item lay (lifted
//!   2 units and set back down, facing as it faced, on its ground). The new
//!   gun comes with the item's ammo only (G_GivePlayerWeapon gives none) and is raised, unless
//!   it is a grenade or mine that goes in an offhand slot.
//! - ThrowDownWeapon (`dropitem`): the gun lands with its magazine and the reserve his other
//!   guns cannot carry; a gun with nothing to carry is only taken.
//! - Player_UpdateCursorHints, Player_ActivateCmd and Player_ActivateHoldCmdAllowed: when the
//!   use button offers a gun, when a press picks it, and when holding takes it.
//!
//! Not ported: a gun lying on a
//! mover (the item traces here report movers as the world, so a gun's ground never moves) and
//! a swapped-out gun linked to its item's mover, suspended map items,
//! BG_PlayerCanPickUpWeaponType, last stand, blocksProne, the riot
//! shield's pickup notify, the cannot-carry-more-primaries hint, the grenade throwback, the
//! weapon's camo on the item, and the dropped-weapon cap's own order (`crate::floor_guns`'s stands).

use entity_t6::{
    Trajectory, evaluate_trajectory, flight::path_kind,
};
use game_math::{euler_basis, rows_to_euler};
use playerstate_t6::game::NO_ENTITY_SLOT;
use weapon_t6::facts::FactsTable;
use weapon_t6::pickup::{self, ItemAmmo, ItemSlot, ThrowDown};
use weapon_t6::player::T6WeaponPs;
use weapon_t6::pm::dual::dual_wield_weapon;
use weapon_t6::state;

use crate::GrabbedItemNote;
use crate::shot_trace::{BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::step_view::StepWorldView;
use crate::floor_guns::{
    LootOnGround, aabb_overlap, grab_scavenger, has_scavenger_perk, launch_dropped_from_ps,
    push_dropped_item, walker_can_touch,
};
use crate::client_script;
use crate::gun_brain::{script_edit, synced_copy};
use crate::sim_parts::{SeatNo, StepNo, script_gun_wants_two_hands};

/// LaunchItem: a dropped gun's box.
pub const LOOT_BOX_LOW: [f32; 3] = [-1.0; 3];
pub const LOOT_BOX_HIGH: [f32; 3] = [1.0; 3];
/// DroppedItemClearOwner runs this long after LaunchItem.
pub const OWNER_CLEAR_MS: i32 = 1000;
/// G_ItemClipMask for an item with no clip mask of its own: solid, glass, missile clip and
/// item clip.
pub const ITEM_CLIP_MASK: u32 = 0x0491;
/// The no-drop contents: a gun that comes to rest in it is removed.
const CONTENTS_NODROP: u32 = 0x8000_0000;
/// G_RunItem moves a gun to where it will be this far on, and slides a hit over this long.
const RUN_AHEAD_MS: i32 = 50;
/// The swapped-out gun is lifted this far before it is set back down.
const SWAP_LIFT: f32 = 2.0;
/// The pickup record's swapped-entity slot when no gun was put down.
const NOTHING_SWAPPED: i32 = NO_ENTITY_SLOT;

fn clock_millis(world: &StepWorldView) -> i32 {
    world.entity_kernel().level_time_ms()
}

/// BG_GetMaxAmmo for every gun the rules may ask about: the held ones and `extra`, with their
/// alternate modes and left-hand guns.
fn max_table(
    world: &StepWorldView,
    facts: &FactsTable,
    t6: Option<&T6WeaponPs>,
    extra: &[u32],
) -> Vec<(u32, i32)> {
    let mut ws: Vec<u32> = t6.map_or_else(Vec::new, |p| {
        p.held
            .iter()
            .map(|h| h.weapon)
            .filter(|&w| w != 0)
            .collect()
    });
    for &w in extra {
        if w != 0 && !ws.contains(&w) {
            ws.push(w);
        }
    }
    let mut i = 0;
    while i < ws.len() && ws.len() < 64 {
        let w = ws[i];
        for n in [facts.get(w).core.alt_weapon, dual_wield_weapon(facts, w)] {
            if n != 0 && !ws.contains(&n) {
                ws.push(n);
            }
        }
        i += 1;
    }
    ws.into_iter()
        .map(|w| (w, world.damage_profile_of(w).map_or(0, |f| f.reserve_cap)))
        .collect()
}

fn max_of(table: &[(u32, i32)], w: u32) -> i32 {
    table.iter().find(|(h, _)| *h == w).map_or(0, |(_, m)| *m)
}

/// BG_CanItemBeGrabbed: the player who threw it down cannot have it until
/// DroppedItemClearOwner.
fn owner_blocks(item: &LootOnGround, toucher: SeatNo, now: i32) -> bool {
    i32::from(item.state.client_num) == toucher.0 as i32 && now < item.owner_clear_ms
}

/// The item's two ammo slots: the gun, and its left-hand gun or alternate mode.
fn item_slots(facts: &FactsTable, entry: &LootOnGround) -> ItemAmmo {
    let w = u32::try_from(entry.state.index).unwrap_or(0);
    [
        ItemSlot { weapon: w, clip: entry.right_mag, stock: entry.spare },
        ItemSlot { weapon: pickup::slot2_weapon(facts, w), clip: entry.left_mag, stock: entry.stock2 },
    ]
}

/// BG_CanItemBeGrabbed's and WeaponPickup's gates that do not depend on the touch or the use:
/// weapons taken away, a riot shield going up or down, a carried killstreak in his hands.
fn gates(t6: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    !pickup::weapons_disabled(t6)
        && !(state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&t6.gun_phase)
        && !pickup::killstreak_blocks(t6, facts, w)
}

/// The use button's pick: whether holding use on `item` would take it (WeaponPickup_Grab's
/// checks before anything changes).
pub(crate) fn use_candidate(
    t6: &T6WeaponPs,
    facts: &FactsTable,
    item: &LootOnGround,
    toucher: SeatNo,
    now: i32,
) -> bool {
    let w = u32::try_from(item.state.index).unwrap_or(0);
    !item.scavenge
        && w != 0
        && !owner_blocks(item, toucher, now)
        && gates(t6, facts, w)
        && pickup::use_can_be_grabbed(t6, facts, w)
        && !pickup::add_weapon_refused(t6)
        && !(pickup::needs_swap(t6, facts, w) && pickup::swap_target(t6, facts) == 0)
}

/// Player_UpdateCursorHints: the use button offers nothing while he climbs a ledge or a
/// ladder, sprints, dives, cannot change guns, readies or throws an offhand, sets up or packs
/// up a deployed gun, or is in last stand.
pub(crate) fn hint_blocked(t6: &T6WeaponPs, ps: &playerstate_t6::game::AvatarState) -> bool {
    use playerstate_t6::game::movement_state_bits::{LADDER, MANTLE, SPRINTING};
    let moving = MANTLE | LADDER | SPRINTING | movement_t6::bridge::PMF_DIVE;
    ps.movement_state_bits & moving != 0
        || t6.weapon_state_bits & weapon_t6::player::wf::CHANGE_LOCKED != 0
        || ps.movement_mode == playerstate_t6::game::MOVE_MODE_DOWNED
        || (state::OFFHAND_INIT..=state::OFFHAND).contains(&t6.gun_phase)
        || (state::DEPLOYING..=state::BREAKING_DOWN).contains(&t6.gun_phase)
}

/// Player_ActivateCmd: a press does not pick the offered gun while an offhand or the riot
/// shield is in his hands.
pub(crate) fn press_picks(t6: &T6WeaponPs) -> bool {
    !(state::OFFHAND_INIT..=state::RIOTSHIELD_END).contains(&t6.gun_phase)
}

/// Player_ActivateHoldCmdAllowed: holding either use button takes nothing while his gun is
/// coming up or going away, while a deployed gun sets up or packs up, or while he climbs a
/// ledge.
pub(crate) fn hold_takes(t6: &T6WeaponPs, ps: &playerstate_t6::game::AvatarState) -> bool {
    !(state::RAISING..=state::DROPPING_ALTSWITCH).contains(&t6.gun_phase)
        && !(state::DEPLOYING..=state::BREAKING_DOWN).contains(&t6.gun_phase)
        && ps.movement_state_bits & playerstate_t6::game::movement_state_bits::MANTLE == 0
}

/// The player's gun `w` goes, as `takeweapon` takes it: BO2's take on the T6 state, the
/// engine's take on the legacy state, then the gun the take leaves in his hands.
fn take(world: &mut StepWorldView, id: SeatNo, w: u32) {
    let taken = script_edit(world, id, |ps, facts, _| {
        weapon_t6::script::take_weapon(ps, facts, w)
    });
    client_script::take_weapon(world, id, w);
    if let Some(follow_up) = taken {
        script_edit(world, id, |_, _, _| ());
        if let Some(to) = follow_up.select.or(follow_up.bot_switch) {
            client_script::switch_to_weapon(world, id, to);
        }
    }
}

/// ThrowDownWeapon for `dropitem`: the gun lands with what it carries (or is only taken).
/// Returns the gun thrown down (an alternate mode's or a left hand's gun for those) and the
/// item's number.
pub(crate) fn drop_weapon(
    world: &mut StepWorldView,
    tick: StepNo,
    player: SeatNo,
    w: u32,
) -> (u32, Option<i32>) {
    let facts = world.gun_facts_shared();
    let t6 = synced_copy(world, player);
    let max = max_table(world, &facts, t6.as_ref(), &[w]);
    let Some(throw) = script_edit(world, player, |ps, facts, _| {
        pickup::throw_down(ps, facts, &|g| max_of(&max, g), w)
    }) else {
        return (w, None);
    };
    match throw {
        ThrowDown::Nothing => (w, None),
        ThrowDown::TakeOnly(t) => {
            take(world, player, t);
            (t, None)
        }
        ThrowDown::Drop(t, a) => {
            let Some(ps) = world.avatar(player).copied() else {
                return (t, None);
            };
            let item_num = launch_dropped_from_ps(
                world,
                tick,
                &ps,
                player.0 as i32,
                t,
                a[0].clip,
                a[1].clip,
                a[0].stock,
                false,
            );
            let clear = crate::bodies::level_time_ms(tick) + OWNER_CLEAR_MS;
            if let Some(item) = world.loot_at_slot_mut(item_num) {
                item.stock2 = a[1].stock;
                item.owner_clear_ms = clear;
            }
            // BO2 takes the gun whether or not the item could be spawned.
            take(world, player, t);
            (t, (item_num != NO_ENTITY_SLOT).then_some(item_num))
        }
    }
}

/// The touch phase on a T6 map: every living player against every gun and bag his box touches.
pub(crate) fn phase_touch(world: &mut StepWorldView) {
    let mut toucher_ids = world.ordered_player_ids();
    toucher_ids.sort_unstable_by_key(|id| id.0);
    for toucher in toucher_ids {
        for item_num in world.ordered_loot_slots() {
            touch_one(world, toucher, item_num);
        }
    }
}

/// Touch_Item: one player over one item.
fn touch_one(level: &mut StepWorldView, toucher: SeatNo, item_num: i32) {
    let Some(pstate) = level.avatar(toucher).copied() else {
        return;
    };
    if !walker_can_touch(level, toucher, &pstate) {
        return;
    }
    let Some(item) = level.loot_at_slot(item_num) else {
        return;
    };
    if !touches_item(&pstate, &item, clock_millis(level)) {
        return;
    }
    let Some(t6) = synced_copy(level, toucher) else {
        return;
    };
    if owner_blocks(&item, toucher, clock_millis(level)) || pickup::weapons_disabled(&t6) {
        return;
    }
    let is_bag = item.scavenge;
    if is_bag {
        if has_scavenger_perk(&pstate) {
            grab_scavenger(level, toucher, item_num);
        }
        return;
    }
    let w = u32::try_from(item.state.index).unwrap_or(0);
    if w == 0 {
        return;
    }
    let facts = level.gun_facts_shared();
    let slots = item_slots(&facts, &item);
    let max = max_table(
        level,
        &facts,
        Some(&t6),
        &[slots[0].weapon, slots[1].weapon],
    );
    let max_fn = |g: u32| max_of(&max, g);
    if !pickup::can_be_grabbed(&t6, &facts, &max_fn, &slots, false, false) || !gates(&t6, &facts, w)
    {
        return;
    }
    let Some(exact) = pickup::touch_gate(&t6, &facts, w) else {
        return;
    };
    // A look first: a touch that moves nothing changes nothing (no T6 state is made for it).
    let mut peek = t6;
    let mut peek_slots = slots;
    let l = pickup::leech(&mut peek, &facts, &max_fn, &mut peek_slots, exact);
    if !l.added && !l.take {
        return;
    }
    let mut after = slots;
    let Some(l) = script_edit(level, toucher, |pstate, facts, _| {
        pickup::leech(pstate, facts, &max_fn, &mut after, exact)
    }) else {
        return;
    };
    if let Some(it) = level.loot_at_slot_mut(item_num) {
        it.right_mag = after[0].clip;
        it.spare = after[0].stock;
        it.left_mag = after[1].clip;
        it.stock2 = after[1].stock;
    }
    if l.added {
        let taken_stock = slots[0].stock + slots[1].stock - after[0].stock - after[1].stock;
        let record = GrabbedItemNote {
            picker: toucher.0 as i32,
            weapon: w,
            from_entnum: item_num,
            clip_r: slots[0].clip - after[0].clip,
            clip_l: slots[1].clip - after[1].clip,
            stock: taken_stock,
            swapped_entnum: NOTHING_SWAPPED,
            picker_pm_type: pstate.movement_mode,
            ammo_only: true,
        };
        level.item_pickups_mut().push(record);
        let tick = StepNo((clock_millis(level) / crate::SIM_STEP_MILLIS as i32) as u32);
        let audience = crate::EventRecipients::All;
        level.post_entity_event(
            tick,
            audience,
            entity_t6::EventCode::AMMO_PICKUP,
            crate::HappeningBody {
                number: toucher.0 as i32,
                event_parm: w as i32,
                weapon: w,
                origin: pstate.origin,
                ..Default::default()
            },
        );
        perf::pickup(pstate.movement_mode);
    }
    if l.take && level.take_loot_at_slot(item_num).is_some() {
        level.free_dynamic_entity_number(item_num);
    }
}

/// G_DoTouchTriggers on a gun or bag: it must lie in his box grown 20 units, and
/// BG_PlayerTouchesItem must pass where its path has it now: at most 36 units off his
/// origin across, from 18 below his feet to 88 above them.
fn touches_item(ps: &playerstate_t6::game::AvatarState, entry: &LootOnGround, now: i32) -> bool {
    let grown_mins = BODY_BOX_LOWER.map(|v| v - 20.0);
    let grown_maxs = [
        BODY_BOX_UPPER[0] + 20.0,
        BODY_BOX_UPPER[1] + 20.0,
        crate::floor_guns::stance_maxs_z(ps) + 20.0,
    ];
    let boxes_meet = aabb_overlap(ps.origin, grown_mins, grown_maxs, entry.origin, LOOT_BOX_LOW, LOOT_BOX_HIGH);
    if !boxes_meet {
        return false;
    }
    let at = evaluate_trajectory(&entry.state.lerp.pos, now);
    let d: [f32; 3] = core::array::from_fn(|i| ps.origin[i] - at[i]);
    d[0].abs() <= 36.0 && d[1].abs() <= 36.0 && (-88.0..=18.0).contains(&d[2])
}

/// WeaponPickup_Grab: holding use on dropped item `item_num`.
pub(crate) fn grab(level: &mut StepWorldView, toucher: SeatNo, item_num: i32) {
    let Some(item) = level.loot_at_slot(item_num) else {
        return;
    };
    let w = u32::try_from(item.state.index).unwrap_or(0);
    if item.scavenge || w == 0 {
        return;
    }
    let Some(pstate) = level.avatar(toucher).copied() else {
        return;
    };
    let Some(t6) = synced_copy(level, toucher) else {
        crate::floor_guns::grab_number(level, toucher, item_num);
        return;
    };
    let now = clock_millis(level);
    let facts = level.gun_facts_shared();
    if !use_candidate(&t6, &facts, &item, toucher, now)
        || level.damage_profile_of(w).is_none()
        || level.weapon_is_melee_only(w)
    {
        return;
    }
    let swap = pickup::needs_swap(&t6, &facts, w).then(|| pickup::swap_target(&t6, &facts));
    if swap.is_none() && !pstate.weapons.contains(&0) {
        return;
    }
    let slots = item_slots(&facts, &item);
    // The item goes first: the swapped-out gun's spawn may not take its place in the cap.
    if level.take_loot_at_slot(item_num).is_some() {
        level.free_dynamic_entity_number(item_num);
    }
    let mut swapped_out = NOTHING_SWAPPED;
    if let Some(target) = swap {
        let max = max_table(level, &facts, Some(&t6), &[target]);
        let throw = script_edit(level, toucher, |pstate, facts, _| {
            pickup::throw_down(pstate, facts, &|g| max_of(&max, g), target)
        });
        match throw {
            Some(ThrowDown::Drop(t, a)) => {
                swapped_out = drop_at(level, toucher, &item, t, &a, now);
                take(level, toucher, t);
            }
            Some(ThrowDown::TakeOnly(t)) => take(level, toucher, t),
            Some(ThrowDown::Nothing) | None => {}
        }
    }
    let held_before: Vec<u32> = synced_copy(level, toucher)
        .map(|p| {
            p.held
                .iter()
                .map(|h| h.weapon)
                .filter(|&h| h != 0)
                .collect()
        })
        .unwrap_or_default();
    let akimbo = script_gun_wants_two_hands(level.weapon_script_label(w));
    if let Err(e) = client_script::give_weapon(level, toucher, w, akimbo) {
        diag::warn!(Sim, "bo2mp item {item_num}: the pick-up gave nothing: {e}");
        return;
    }
    let after = synced_copy(level, toucher);
    let max = max_table(
        level,
        &facts,
        after.as_ref(),
        &[slots[0].weapon, slots[1].weapon],
    );
    script_edit(level, toucher, |pstate, facts, _| {
        // G_GivePlayerWeapon gives the gun with no ammo; the item's ammo is all it gets.
        let held = pstate.held;
        for h in held {
            let n = h.weapon;
            if n == 0 || held_before.contains(&n) {
                continue;
            }
            if facts.ammo_clip_weapon(n) == n {
                pstate.set_clip_slot(n, 0);
            }
            if facts.ammo_pool_weapon(n) == n {
                pstate.set_pool_slot(n, 0);
            }
        }
        pickup::add_ammo_for_new_weapon(pstate, facts, &|g| max_of(&max, g), &slots);
    });
    if pickup::grab_selects(&facts, w) {
        client_script::switch_to_weapon(level, toucher, w);
    }
    let next = level.avatar(toucher).copied().expect("picker exists");
    {
        let holds = |id: &u32| next.weapons.contains(&(*id as i32));
        let meta = level.client_meta_mut(toucher);
        meta.rounds_per_gun.retain(|(id, _, _)| holds(id));
        meta.taped_mag_spent.retain(holds);
        meta.set_quick_reload_ready(w, true);
        (meta.shots_per_gun, meta.burst_held) = (0, false);
        meta.burst_latch_secondary = false;
        (meta.bolt_due, meta.rechamber_pending_secondary) = (false, false);
    }
    let (clip_r, clip_l) = (item.right_mag, item.left_mag);
    let record = GrabbedItemNote {
        picker: toucher.0 as i32,
        weapon: w,
        from_entnum: item_num,
        clip_r,
        clip_l,
        stock: item.spare + item.stock2,
        swapped_entnum: swapped_out,
        picker_pm_type: pstate.movement_mode,
        ammo_only: false,
    };
    level.item_pickups_mut().push(record);
    let tick = StepNo((now / crate::SIM_STEP_MILLIS as i32) as u32);
    let audience = crate::EventRecipients::All;
    level.post_entity_event(
        tick,
        audience,
        entity_t6::EventCode::ITEM_PICKUP,
        crate::HappeningBody {
            number: toucher.0 as i32,
            event_parm: w as i32,
            weapon: w,
            origin: pstate.origin,
            ..Default::default()
        },
    );
    perf::pickup(pstate.movement_mode);
}

/// A trajectory that stays put at `base`.
fn resting_at(base: [f32; 3]) -> Trajectory {
    let mut traj = Trajectory::default();
    traj.tr_type = path_kind::AT_REST;
    traj.base_point = base;
    traj
}

/// The swapped-out gun is put down where the picked-up one lay: lifted 2 units and traced back
/// down (so it sits on what that one sat on), facing as that one faced, on that one's ground.
/// It lies still; G_RunItem drops it from there if that one was still in the air.
fn drop_at(
    world: &mut StepWorldView,
    toucher: SeatNo,
    at: &LootOnGround,
    t: u32,
    a: &ItemAmmo,
    now: i32,
) -> i32 {
    let lifted = [at.origin[0], at.origin[1], at.origin[2] + SWAP_LIFT];
    let upward = world.trace_clip(at.origin, lifted, LOOT_BOX_LOW, LOOT_BOX_HIGH, ITEM_CLIP_MASK);
    let origin = if upward.fully_embedded {
        at.origin
    } else {
        let top = lerp(at.origin, lifted, upward.fraction);
        let below = world.trace_clip(top, at.origin, LOOT_BOX_LOW, LOOT_BOX_HIGH, ITEM_CLIP_MASK);
        lerp(top, at.origin, below.fraction)
    };
    let pos = resting_at(origin);
    let apos = resting_at(at.state.lerp.apos.base_point);
    let item_num = push_dropped_item(
        world,
        t,
        origin,
        pos,
        apos,
        toucher.0 as i32,
        a[0].clip,
        a[1].clip,
        a[0].stock,
        false,
        false,
    );
    if let Some(item) = world.loot_at_slot_mut(item_num) {
        item.stock2 = a[1].stock;
        item.owner_clear_ms = now + OWNER_CLEAR_MS;
        item.state.ground_entity_num = at.state.ground_entity_num;
    }
    item_num
}

/// BO2's G_RunItem for a gun that is not linked to anything (`crate::floor_guns::think_item_move`
/// hands it over when the T6 brain is on).
pub(crate) fn run_item(world: &mut StepWorldView, clock_stamp_ms: i32, item_num: i32) {
    let Some(item) = world.loot_at_slot(item_num) else {
        return;
    };
    let mut ent_state = item.state;
    let origin = item.origin;
    // Its ground is gone: it falls from where it is. (A gun only ever comes to rest on the
    // world here, and the world never moves.)
    if !grounded_now(&ent_state) && ent_state.lerp.pos.tr_type != path_kind::FALLING {
        ent_state.lerp.pos.tr_type = path_kind::FALLING;
        ent_state.lerp.pos.began_at = clock_stamp_ms;
        ent_state.lerp.pos.span_ms = 0;
        ent_state.lerp.pos.base_point = origin;
        ent_state.lerp.pos.rate = [0.0; 3];
    }
    if ent_state.lerp.pos.tr_type == path_kind::AT_REST {
        return;
    }
    let target = ahead(&ent_state, origin, clock_stamp_ms);
    let mut struck = world.trace_clip(origin, target, LOOT_BOX_LOW, LOOT_BOX_HIGH, ITEM_CLIP_MASK);
    let now_at = if struck.fraction >= 1.0 {
        target
    } else {
        let mut at = lerp(origin, target, struck.fraction);
        if !struck.starts_inside && struck.fraction < 0.01 && struck.normal[2] < 0.5 {
            let slid = slide(origin, target, struck.normal);
            struck = world.trace_clip(at, slid, LOOT_BOX_LOW, LOOT_BOX_HIGH, ITEM_CLIP_MASK);
            at = lerp(at, slid, struck.fraction);
        }
        ent_state.lerp.pos.tr_type = path_kind::STRAIGHT_THEN_STOP;
        ent_state.lerp.pos.began_at = clock_stamp_ms;
        ent_state.lerp.pos.span_ms = RUN_AHEAD_MS;
        ent_state.lerp.pos.base_point = origin;
        ent_state.lerp.pos.rate = scale(sub(at, origin), 1000.0 / RUN_AHEAD_MS as f32);
        at
    };
    if struck.fraction < 0.01 {
        if struck.normal[2] <= 0.0 || in_nodrop(world, now_at) {
            diag::info!(
                Sim,
                "bo2mp item {item_num}: came to rest off the floor, removed"
            );
            world.take_loot_at_slot(item_num);
            world.free_dynamic_entity_number(item_num);
            return;
        }
        let angles = ground_angles(ent_state.lerp.apos.base_point, struck.normal, ent_state.index > 0);
        ent_state.lerp.pos.tr_type = path_kind::AT_REST;
        ent_state.lerp.pos.began_at = 0;
        ent_state.lerp.pos.span_ms = 0;
        ent_state.lerp.pos.base_point = now_at;
        ent_state.lerp.pos.rate = [0.0; 3];
        ent_state.lerp.apos.tr_type = path_kind::AT_REST;
        ent_state.lerp.apos.began_at = 0;
        ent_state.lerp.apos.span_ms = 0;
        ent_state.lerp.apos.base_point = angles;
        ent_state.lerp.apos.rate = [0.0; 3];
        ent_state.ground_entity_num = trace_t6::WORLD_ENTITY_SLOT as i16;
    }
    if let Some(item) = world.loot_at_slot_mut(item_num) {
        item.state = ent_state;
        item.origin = now_at;
        item.dropping = ent_state.lerp.pos.tr_type != path_kind::AT_REST;
    }
}

/// The world is the only ground a gun comes to rest on here.
fn grounded_now(state: &entity_t6::EntitySnapshot) -> bool {
    state.ground_entity_num == trace_t6::WORLD_ENTITY_SLOT as i16
}

/// Where G_RunItem moves a gun: its trajectory 50 ms on, or 1 unit down when it would barely
/// move.
fn ahead(state: &entity_t6::EntitySnapshot, spot: [f32; 3], clock_stamp_ms: i32) -> [f32; 3] {
    let mut target = evaluate_trajectory(&state.lerp.pos, clock_stamp_ms + RUN_AHEAD_MS);
    let d = sub(target, spot);
    if dot(d, d) < 0.1 {
        target[2] -= 1.0;
    }
    target
}

/// G_RunItem's slide off a steep face it hit at once: the move loses its part into the face
/// and is pushed 1 unit out of it.
fn slide(origin: [f32; 3], aim_at: [f32; 3], face: [f32; 3]) -> [f32; 3] {
    let f = 1.0 - dot(face, sub(aim_at, origin));
    [
        aim_at[0] + face[0] * f,
        aim_at[1] + face[1] * f,
        aim_at[2] + face[2] * f,
    ]
}

/// G_OrientItemToGround: the gun keeps its heading along the ground it lies on, and a gun
/// lies on its side.
fn ground_angles(angles: [f32; 3], upward: [f32; 3], gun: bool) -> [f32; 3] {
    let (ahead, _, _) = euler_basis(angles);
    let left = cross(upward, ahead);
    let ahead = cross(left, upward);
    let mut out = rows_to_euler([ahead, left, upward]);
    if gun {
        out[2] += 90.0;
    }
    out
}

/// SV_PointContents for the no-drop contents.
fn in_nodrop(world: &StepWorldView, at: [f32; 3]) -> bool {
    let t = world.trace_clip(at, at, [0.0; 3], [0.0; 3], CONTENTS_NODROP);
    t.starts_inside || t.fully_embedded
}

fn sub(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| lhs[k] - rhs[k])
}

fn dot(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

fn scale(vec_in: [f32; 3], factor: f32) -> [f32; 3] {
    vec_in.map(|c| c * factor)
}

fn cross(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    [
        lhs[1] * rhs[2] - lhs[2] * rhs[1],
        lhs[2] * rhs[0] - lhs[0] * rhs[2],
        lhs[0] * rhs[1] - lhs[1] * rhs[0],
    ]
}

fn lerp(lhs: [f32; 3], b: [f32; 3], t_mix: f32) -> [f32; 3] {
    [
        (b[0] - lhs[0]) * t_mix + lhs[0],
        (b[1] - lhs[1]) * t_mix + lhs[1],
        (b[2] - lhs[2]) * t_mix + lhs[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(owner: i32, clear: i32) -> LootOnGround {
        let mut state = entity_t6::EntitySnapshot::default();
        state.client_num = owner as u8;
        state.index = 5;
        LootOnGround {
            state,
            origin: [0.0; 3],
            dropping: Default::default(),
            right_mag: 3,
            left_mag: 2,
            spare: 40,
            stock2: 7,
            scavenge: false,
            owner_clear_ms: clear,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        let d = (a - b).rem_euclid(360.0);
        d < 0.01 || d > 359.99
    }

    #[test]
    fn a_gun_lies_on_its_side_along_its_heading() {
        let a = ground_angles([0.0, 90.0, 0.0], [0.0, 0.0, 1.0], true);
        assert!(
            near(a[0], 0.0) && near(a[1], 90.0) && near(a[2], 90.0),
            "{a:?}"
        );
        let b = ground_angles([30.0, 45.0, 0.0], [0.0, 0.0, 1.0], false);
        assert!(
            near(b[0], 0.0) && near(b[1], 45.0) && near(b[2], 0.0),
            "{b:?}"
        );
    }

    #[test]
    fn a_resting_gun_feels_1_unit_down_a_falling_one_moves_50ms_on() {
        let mut s = entity_t6::EntitySnapshot::default();
        s.lerp.pos.tr_type = path_kind::STRAIGHT_THEN_STOP;
        s.lerp.pos.began_at = 0;
        s.lerp.pos.span_ms = RUN_AHEAD_MS;
        s.lerp.pos.base_point = [0.0, 0.0, 10.0];
        s.lerp.pos.rate = [0.0, 0.0, -100.0];
        // The stop is done: it would not move, so it looks 1 unit down.
        assert_eq!(ahead(&s, [0.0, 0.0, 5.0], 100), [0.0, 0.0, 4.0]);
        s.lerp.pos.tr_type = path_kind::FALLING;
        s.lerp.pos.rate = [0.0; 3];
        let t = ahead(&s, [0.0, 0.0, 10.0], 0);
        assert!(t[2] < 10.0 - 0.5 && t[2] > 10.0 - 1.5, "{t:?}");
    }

    #[test]
    fn a_steep_hit_slides_along_the_face() {
        // Moving +x into a wall facing -x: the push into it is gone, 1 unit out of it.
        let s = slide([0.0; 3], [4.0, 3.0, -2.0], [-1.0, 0.0, 0.0]);
        assert_eq!(s, [-1.0, 3.0, -2.0]);
    }

    #[test]
    fn the_thrower_waits_a_second_others_do_not() {
        let it = item(2, 1000);
        assert!(owner_blocks(&it, SeatNo(2), 999));
        assert!(!owner_blocks(&it, SeatNo(2), 1000));
        assert!(!owner_blocks(&it, SeatNo(3), 0));
        // legacy items (no clear time) never block.
        assert!(!owner_blocks(&item(2, 0), SeatNo(2), 0));
    }

    #[test]
    fn the_use_button_follows_bo2s_gates() {
        use playerstate_t6::game::movement_state_bits::{LADDER, MANTLE, SPRINTING};
        let ps = playerstate_t6::game::AvatarState::ZERO;
        let t6 = T6WeaponPs::default();
        assert!(!hint_blocked(&t6, &ps));
        assert!(press_picks(&t6) && hold_takes(&t6, &ps));
        for flag in [MANTLE, LADDER, SPRINTING, movement_t6::bridge::PMF_DIVE] {
            let mut flagged = ps;
            flagged.movement_state_bits = flag;
            assert!(hint_blocked(&t6, &flagged), "{flag:#x}");
        }
        let mut last_stand = ps;
        last_stand.movement_mode = playerstate_t6::game::MOVE_MODE_DOWNED;
        assert!(hint_blocked(&t6, &last_stand));
        let mut locked = t6;
        locked.weapon_state_bits = weapon_t6::player::wf::CHANGE_LOCKED;
        assert!(hint_blocked(&locked, &ps));
        fn at(t6: &T6WeaponPs, weaponstate: i32) -> T6WeaponPs {
            let mut held = *t6;
            held.gun_phase = weaponstate;
            held
        }
        assert!(hint_blocked(&at(&t6, state::OFFHAND_INIT), &ps));
        assert!(hint_blocked(&at(&t6, state::OFFHAND), &ps));
        assert!(!hint_blocked(&at(&t6, state::OFFHAND_END), &ps));
        assert!(!press_picks(&at(&t6, state::OFFHAND_END)));
        assert!(!press_picks(&at(&t6, state::RIOTSHIELD_END)));
        assert!(press_picks(&at(&t6, state::RAISING)));
        assert!(!hold_takes(&at(&t6, state::RAISING), &ps));
        assert!(!hold_takes(&at(&t6, state::DROPPING_ALTSWITCH), &ps));
        assert!(!hold_takes(&at(&t6, state::DEPLOYING), &ps));
        assert!(hint_blocked(&at(&t6, state::BREAKING_DOWN), &ps));
        let mut mantle = ps;
        mantle.movement_state_bits = MANTLE;
        assert!(!hold_takes(&t6, &mantle));
    }

    #[test]
    fn he_touches_a_gun_36_across_and_18_below_to_88_above() {
        let lying = |p: [f32; 3]| {
            let mut it = item(0, 0);
            it.origin = p;
            it.state.lerp.pos.base_point = p;
            it
        };
        let mut ps = playerstate_t6::game::AvatarState::ZERO;
        let touches = |ps: &playerstate_t6::game::AvatarState, p| touches_item(ps, &lying(p), 0);
        assert!(touches(&ps, [36.0, -36.0, 0.0]));
        assert!(!touches(&ps, [36.5, 0.0, 0.0]));
        assert!(touches(&ps, [0.0, 0.0, -18.0]));
        assert!(!touches(&ps, [0.0, 0.0, -18.5]));
        assert!(touches(&ps, [0.0, 0.0, 88.0]));
        assert!(!touches(&ps, [0.0, 0.0, 88.5]));
        // Crouched his box is 50 high: grown 20, and the gun's own unit, 71 up at most.
        ps.eye_height_goal = movement_t6::bridge::eye_level::DUCK_BUTTON_BIT;
        assert!(touches(&ps, [0.0, 0.0, 71.0]));
        assert!(!touches(&ps, [0.0, 0.0, 71.5]));
    }

    #[test]
    fn the_second_slot_is_the_left_hand_or_alt_mode() {
        let mut facts = FactsTable::new();
        let mut gun = weapon_t6::facts::T6WeaponFacts::default();
        gun.core.alt_weapon = 6;
        facts.set(5, "an94_mp", gun);
        facts.set(6, "gl_an94_mp", weapon_t6::facts::T6WeaponFacts::default());
        let s = item_slots(&facts, &item(0, 0));
        assert_eq!(
            s[0],
            ItemSlot {
                weapon: 5,
                clip: 3,
                stock: 40
            }
        );
        assert_eq!(
            s[1],
            ItemSlot {
                weapon: 6,
                clip: 2,
                stock: 7
            }
        );
    }
}
