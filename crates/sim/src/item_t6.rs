//! bo2mp: BO2's dropped guns on a map with BO2's weapon facts (the T6 brain on). The item
//! entity and the use button's aim pick stay `crate::item`'s; how a gun falls and lands,
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
//! weapon's camo on the item, and the dropped-weapon cap's own order (`crate::item`'s stands).

use entity_t6::{
    Trajectory, evaluate_trajectory, trajectory::tr_type::GRAVITY as TR_GRAVITY,
    trajectory::tr_type::LINEAR_STOP as TR_LINEAR_STOP,
    trajectory::tr_type::STATIONARY as TR_STATIONARY,
};
use game_math::{angle_vectors, axis_to_angles};
use playerstate_t6::game::ENTITYNUM_NONE;
use weapon_t6::facts::FactsTable;
use weapon_t6::pickup::{self, ItemAmmo, ItemSlot, ThrowDown};
use weapon_t6::player::T6WeaponPs;
use weapon_t6::pm::dual::dual_wield_weapon;
use weapon_t6::state;

use crate::ItemPickupRecord;
use crate::bullet_collision::{PLAYER_MAXS, PLAYER_MINS};
use crate::frame::FrameWorld;
use crate::item::{
    DroppedItem, aabb_overlap, grab_scavenger, has_scavenger_perk, launch_dropped_from_ps,
    push_dropped_item, walker_can_touch,
};
use crate::script_player;
use crate::t6_gun::{script_edit, synced_copy};
use crate::world::{ClientId, Tick, gsc_give_weapon_is_akimbo};

/// LaunchItem: a dropped gun's box.
pub const ITEM_MINS: [f32; 3] = [-1.0; 3];
pub const ITEM_MAXS: [f32; 3] = [1.0; 3];
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

fn now_ms(world: &FrameWorld) -> i32 {
    world.entity_kernel().level_time_ms()
}

/// BG_GetMaxAmmo for every gun the rules may ask about: the held ones and `extra`, with their
/// alternate modes and left-hand guns.
fn max_table(
    world: &FrameWorld,
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
        .map(|w| (w, world.combat_facts_for(w).map_or(0, |f| f.max_ammo)))
        .collect()
}

fn max_of(table: &[(u32, i32)], w: u32) -> i32 {
    table.iter().find(|(h, _)| *h == w).map_or(0, |(_, m)| *m)
}

/// BG_CanItemBeGrabbed: the player who threw it down cannot have it until
/// DroppedItemClearOwner.
fn owner_blocks(item: &DroppedItem, walker: ClientId, now: i32) -> bool {
    i32::from(item.state.client_num) == walker.0 as i32 && now < item.owner_clear_ms
}

/// The item's two ammo slots: the gun, and its left-hand gun or alternate mode.
fn item_slots(facts: &FactsTable, item: &DroppedItem) -> ItemAmmo {
    let w = u32::try_from(item.state.index).unwrap_or(0);
    [
        ItemSlot {
            weapon: w,
            clip: item.clip_r,
            stock: item.stock,
        },
        ItemSlot {
            weapon: pickup::slot2_weapon(facts, w),
            clip: item.clip_l,
            stock: item.stock2,
        },
    ]
}

/// BG_CanItemBeGrabbed's and WeaponPickup's gates that do not depend on the touch or the use:
/// weapons taken away, a riot shield going up or down, a carried killstreak in his hands.
fn gates(t6: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    !pickup::weapons_disabled(t6)
        && !(state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&t6.weaponstate)
        && !pickup::killstreak_blocks(t6, facts, w)
}

/// The use button's pick: whether holding use on `item` would take it (WeaponPickup_Grab's
/// checks before anything changes).
pub(crate) fn use_candidate(
    t6: &T6WeaponPs,
    facts: &FactsTable,
    item: &DroppedItem,
    walker: ClientId,
    now: i32,
) -> bool {
    let w = u32::try_from(item.state.index).unwrap_or(0);
    !item.scavenger
        && w != 0
        && !owner_blocks(item, walker, now)
        && gates(t6, facts, w)
        && pickup::use_can_be_grabbed(t6, facts, w)
        && !pickup::add_weapon_refused(t6)
        && !(pickup::needs_swap(t6, facts, w) && pickup::swap_target(t6, facts) == 0)
}

/// Player_UpdateCursorHints: the use button offers nothing while he climbs a ledge or a
/// ladder, sprints, dives, cannot change guns, readies or throws an offhand, sets up or packs
/// up a deployed gun, or is in last stand.
pub(crate) fn hint_blocked(t6: &T6WeaponPs, ps: &playerstate_t6::game::PlayerState) -> bool {
    use playerstate_t6::game::pm_flags::{LADDER, MANTLE, SPRINTING};
    let moving = MANTLE | LADDER | SPRINTING | movement_t6::bridge::PMF_DIVE;
    ps.pm_flags & moving != 0
        || t6.weap_flags & weapon_t6::player::wf::CHANGE_LOCKED != 0
        || ps.pm_type == playerstate_t6::game::PM_TYPE_LAST_STAND
        || (state::OFFHAND_INIT..=state::OFFHAND).contains(&t6.weaponstate)
        || (state::DEPLOYING..=state::BREAKING_DOWN).contains(&t6.weaponstate)
}

/// Player_ActivateCmd: a press does not pick the offered gun while an offhand or the riot
/// shield is in his hands.
pub(crate) fn press_picks(t6: &T6WeaponPs) -> bool {
    !(state::OFFHAND_INIT..=state::RIOTSHIELD_END).contains(&t6.weaponstate)
}

/// Player_ActivateHoldCmdAllowed: holding either use button takes nothing while his gun is
/// coming up or going away, while a deployed gun sets up or packs up, or while he climbs a
/// ledge.
pub(crate) fn hold_takes(t6: &T6WeaponPs, ps: &playerstate_t6::game::PlayerState) -> bool {
    !(state::RAISING..=state::DROPPING_ALTSWITCH).contains(&t6.weaponstate)
        && !(state::DEPLOYING..=state::BREAKING_DOWN).contains(&t6.weaponstate)
        && ps.pm_flags & playerstate_t6::game::pm_flags::MANTLE == 0
}

/// The player's gun `w` goes, as `takeweapon` takes it: BO2's take on the T6 state, the
/// engine's take on the legacy state, then the gun the take leaves in his hands.
fn take(world: &mut FrameWorld, id: ClientId, w: u32) {
    let fx = script_edit(world, id, |ps, facts, _| {
        weapon_t6::script::take_weapon(ps, facts, w)
    });
    script_player::take_weapon(world, id, w);
    if let Some(fx) = fx {
        script_edit(world, id, |_, _, _| ());
        if let Some(to) = fx.select.or(fx.bot_switch) {
            script_player::switch_to_weapon(world, id, to);
        }
    }
}

/// ThrowDownWeapon for `dropitem`: the gun lands with what it carries (or is only taken).
/// Returns the gun thrown down (an alternate mode's or a left hand's gun for those) and the
/// item's number.
pub(crate) fn drop_weapon(
    world: &mut FrameWorld,
    tick: Tick,
    player: ClientId,
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
            let Some(ps) = world.player(player).copied() else {
                return (t, None);
            };
            let number = launch_dropped_from_ps(
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
            let clear = crate::corpse::level_time_ms(tick) + OWNER_CLEAR_MS;
            if let Some(item) = world.dropped_item_mut_by_number(number) {
                item.stock2 = a[1].stock;
                item.owner_clear_ms = clear;
            }
            // BO2 takes the gun whether or not the item could be spawned.
            take(world, player, t);
            (t, (number != ENTITYNUM_NONE).then_some(number))
        }
    }
}

/// The touch phase on a T6 map: every living player against every gun and bag his box touches.
pub(crate) fn phase_touch(world: &mut FrameWorld) {
    let mut walkers = world.client_ids_sorted();
    walkers.sort_unstable_by_key(|id| id.0);
    for walker in walkers {
        for number in world.dropped_item_numbers_sorted() {
            touch_one(world, walker, number);
        }
    }
}

/// Touch_Item: one player over one item.
fn touch_one(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    if !walker_can_touch(world, walker, &ps) {
        return;
    }
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    if !touches_item(&ps, &item, now_ms(world)) {
        return;
    }
    let Some(t6) = synced_copy(world, walker) else {
        return;
    };
    if owner_blocks(&item, walker, now_ms(world)) || pickup::weapons_disabled(&t6) {
        return;
    }
    if item.scavenger {
        if has_scavenger_perk(&ps) {
            grab_scavenger(world, walker, number);
        }
        return;
    }
    let w = u32::try_from(item.state.index).unwrap_or(0);
    if w == 0 {
        return;
    }
    let facts = world.gun_facts_shared();
    let slots = item_slots(&facts, &item);
    let max = max_table(
        world,
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
    let Some(l) = script_edit(world, walker, |ps, facts, _| {
        pickup::leech(ps, facts, &max_fn, &mut after, exact)
    }) else {
        return;
    };
    if let Some(it) = world.dropped_item_mut_by_number(number) {
        it.clip_r = after[0].clip;
        it.stock = after[0].stock;
        it.clip_l = after[1].clip;
        it.stock2 = after[1].stock;
    }
    if l.added {
        world.item_pickups_mut().push(ItemPickupRecord {
            picker: walker.0 as i32,
            weapon: w,
            from_entnum: number,
            clip_r: slots[0].clip - after[0].clip,
            clip_l: slots[1].clip - after[1].clip,
            stock: slots[0].stock + slots[1].stock - after[0].stock - after[1].stock,
            swapped_entnum: ENTITYNUM_NONE,
            picker_pm_type: ps.pm_type,
            ammo_only: true,
        });
        let tick = Tick((now_ms(world) / crate::MATCH_TICK_MS as i32) as u32);
        world.push_entity_event(
            tick,
            crate::EventAudience::All,
            entity_t6::EntityEventKind::AMMO_PICKUP,
            crate::EntityEventPayload {
                number: walker.0 as i32,
                event_parm: w as i32,
                weapon: w,
                origin: ps.origin,
                ..Default::default()
            },
        );
        perf::pickup(ps.pm_type);
    }
    if l.take && world.remove_dropped_item_by_number(number).is_some() {
        world.free_dynamic_entity_number(number);
    }
}

/// G_DoTouchTriggers on a gun or bag: it must lie in his box grown 20 units, and
/// BG_PlayerTouchesItem must pass where its path has it now: at most 36 units off his
/// origin across, from 18 below his feet to 88 above them.
fn touches_item(ps: &playerstate_t6::game::PlayerState, item: &DroppedItem, now: i32) -> bool {
    let grown_mins = PLAYER_MINS.map(|v| v - 20.0);
    let grown_maxs = [
        PLAYER_MAXS[0] + 20.0,
        PLAYER_MAXS[1] + 20.0,
        crate::item::stance_maxs_z(ps) + 20.0,
    ];
    if !aabb_overlap(
        ps.origin,
        grown_mins,
        grown_maxs,
        item.origin,
        ITEM_MINS,
        ITEM_MAXS,
    ) {
        return false;
    }
    let pos = Trajectory {
        tr_time: item.state.lerp.pos.tr_time,
        tr_type: item.state.lerp.pos.tr_type,
        tr_duration: item.state.lerp.pos.tr_duration,
        tr_delta: item.state.lerp.pos.tr_delta,
        tr_base: item.state.lerp.pos.tr_base,
    };
    let at = evaluate_trajectory(&pos, now);
    let d: [f32; 3] = core::array::from_fn(|i| ps.origin[i] - at[i]);
    d[0].abs() <= 36.0 && d[1].abs() <= 36.0 && (-88.0..=18.0).contains(&d[2])
}

/// WeaponPickup_Grab: holding use on item `number`.
pub(crate) fn grab(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    let w = u32::try_from(item.state.index).unwrap_or(0);
    if item.scavenger || w == 0 {
        return;
    }
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    let Some(t6) = synced_copy(world, walker) else {
        crate::item::grab_number(world, walker, number);
        return;
    };
    let now = now_ms(world);
    let facts = world.gun_facts_shared();
    if !use_candidate(&t6, &facts, &item, walker, now)
        || world.combat_facts_for(w).is_none()
        || world.weapon_is_melee_only(w)
    {
        return;
    }
    let swap = pickup::needs_swap(&t6, &facts, w).then(|| pickup::swap_target(&t6, &facts));
    if swap.is_none() && !ps.weapons.contains(&0) {
        return;
    }
    let slots = item_slots(&facts, &item);
    // The item goes first: the swapped-out gun's spawn may not take its place in the cap.
    if world.remove_dropped_item_by_number(number).is_some() {
        world.free_dynamic_entity_number(number);
    }
    let mut swapped_entnum = ENTITYNUM_NONE;
    if let Some(target) = swap {
        let max = max_table(world, &facts, Some(&t6), &[target]);
        let throw = script_edit(world, walker, |ps, facts, _| {
            pickup::throw_down(ps, facts, &|g| max_of(&max, g), target)
        });
        match throw {
            Some(ThrowDown::Drop(t, a)) => {
                swapped_entnum = drop_at(world, walker, &item, t, &a, now);
                take(world, walker, t);
            }
            Some(ThrowDown::TakeOnly(t)) => take(world, walker, t),
            Some(ThrowDown::Nothing) | None => {}
        }
    }
    let held_before: Vec<u32> = synced_copy(world, walker)
        .map(|p| {
            p.held
                .iter()
                .map(|h| h.weapon)
                .filter(|&h| h != 0)
                .collect()
        })
        .unwrap_or_default();
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(w));
    if let Err(e) = script_player::give_weapon(world, walker, w, akimbo) {
        diag::warn!(Sim, "bo2mp item {number}: the pick-up gave nothing: {e}");
        return;
    }
    let after = synced_copy(world, walker);
    let max = max_table(
        world,
        &facts,
        after.as_ref(),
        &[slots[0].weapon, slots[1].weapon],
    );
    script_edit(world, walker, |ps, facts, _| {
        // G_GivePlayerWeapon gives the gun with no ammo; the item's ammo is all it gets.
        let held = ps.held;
        for h in held {
            let n = h.weapon;
            if n == 0 || held_before.contains(&n) {
                continue;
            }
            if facts.ammo_clip_weapon(n) == n {
                ps.set_clip_slot(n, 0);
            }
            if facts.ammo_pool_weapon(n) == n {
                ps.set_pool_slot(n, 0);
            }
        }
        pickup::add_ammo_for_new_weapon(ps, facts, &|g| max_of(&max, g), &slots);
    });
    if pickup::grab_selects(&facts, w) {
        script_player::switch_to_weapon(world, walker, w);
    }
    let next = world.player(walker).copied().expect("picker exists");
    {
        let meta = world.client_meta_mut(walker);
        meta.ammo_by_weapon
            .retain(|(id, _, _)| next.weapons.contains(&(*id as i32)));
        meta.taped_mag_spent
            .retain(|id| next.weapons.contains(&(*id as i32)));
        meta.set_quick_reload_ready(w, true);
        meta.weapon_shot_count = 0;
        meta.burst_latch = false;
        meta.burst_latch_secondary = false;
        meta.rechamber_pending = false;
        meta.rechamber_pending_secondary = false;
    }
    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon: w,
        from_entnum: number,
        clip_r: item.clip_r,
        clip_l: item.clip_l,
        stock: item.stock + item.stock2,
        swapped_entnum,
        picker_pm_type: ps.pm_type,
        ammo_only: false,
    });
    let tick = Tick((now / crate::MATCH_TICK_MS as i32) as u32);
    world.push_entity_event(
        tick,
        crate::EventAudience::All,
        entity_t6::EntityEventKind::ITEM_PICKUP,
        crate::EntityEventPayload {
            number: walker.0 as i32,
            event_parm: w as i32,
            weapon: w,
            origin: ps.origin,
            ..Default::default()
        },
    );
    perf::pickup(ps.pm_type);
}

/// The swapped-out gun is put down where the picked-up one lay: lifted 2 units and traced back
/// down (so it sits on what that one sat on), facing as that one faced, on that one's ground.
/// It lies still; G_RunItem drops it from there if that one was still in the air.
fn drop_at(
    world: &mut FrameWorld,
    walker: ClientId,
    at: &DroppedItem,
    t: u32,
    a: &ItemAmmo,
    now: i32,
) -> i32 {
    let lifted = [at.origin[0], at.origin[1], at.origin[2] + SWAP_LIFT];
    let up = world.trace_clip(at.origin, lifted, ITEM_MINS, ITEM_MAXS, ITEM_CLIP_MASK);
    let origin = if up.allsolid {
        at.origin
    } else {
        let top = lerp(at.origin, lifted, up.fraction);
        let down = world.trace_clip(top, at.origin, ITEM_MINS, ITEM_MAXS, ITEM_CLIP_MASK);
        lerp(top, at.origin, down.fraction)
    };
    let pos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: origin,
    };
    let apos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: at.state.lerp.apos.tr_base,
    };
    let number = push_dropped_item(
        world,
        t,
        origin,
        pos,
        apos,
        walker.0 as i32,
        a[0].clip,
        a[1].clip,
        a[0].stock,
        false,
        false,
    );
    if let Some(item) = world.dropped_item_mut_by_number(number) {
        item.stock2 = a[1].stock;
        item.owner_clear_ms = now + OWNER_CLEAR_MS;
        item.state.ground_entity_num = at.state.ground_entity_num;
    }
    number
}

/// BO2's G_RunItem for a gun that is not linked to anything (`crate::item::think_item_move`
/// hands it over when the T6 brain is on).
pub(crate) fn run_item(world: &mut FrameWorld, time_ms: i32, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    let mut state = item.state;
    let origin = item.origin;
    // Its ground is gone: it falls from where it is. (A gun only ever comes to rest on the
    // world here, and the world never moves.)
    if !on_ground(&state) && state.lerp.pos.tr_type != TR_GRAVITY {
        state.lerp.pos.tr_type = TR_GRAVITY;
        state.lerp.pos.tr_time = time_ms;
        state.lerp.pos.tr_duration = 0;
        state.lerp.pos.tr_base = origin;
        state.lerp.pos.tr_delta = [0.0; 3];
    }
    if state.lerp.pos.tr_type == TR_STATIONARY {
        return;
    }
    let target = ahead(&state, origin, time_ms);
    let mut hit = world.trace_clip(origin, target, ITEM_MINS, ITEM_MAXS, ITEM_CLIP_MASK);
    let now_at = if hit.fraction >= 1.0 {
        target
    } else {
        let mut at = lerp(origin, target, hit.fraction);
        if !hit.startsolid && hit.fraction < 0.01 && hit.normal[2] < 0.5 {
            let slid = slide(origin, target, hit.normal);
            hit = world.trace_clip(at, slid, ITEM_MINS, ITEM_MAXS, ITEM_CLIP_MASK);
            at = lerp(at, slid, hit.fraction);
        }
        state.lerp.pos.tr_type = TR_LINEAR_STOP;
        state.lerp.pos.tr_time = time_ms;
        state.lerp.pos.tr_duration = RUN_AHEAD_MS;
        state.lerp.pos.tr_base = origin;
        state.lerp.pos.tr_delta = scale(sub(at, origin), 1000.0 / RUN_AHEAD_MS as f32);
        at
    };
    if hit.fraction < 0.01 {
        if hit.normal[2] <= 0.0 || in_nodrop(world, now_at) {
            diag::info!(
                Sim,
                "bo2mp item {number}: came to rest off the floor, removed"
            );
            world.remove_dropped_item_by_number(number);
            world.free_dynamic_entity_number(number);
            return;
        }
        let angles = ground_angles(state.lerp.apos.tr_base, hit.normal, state.index > 0);
        state.lerp.pos.tr_type = TR_STATIONARY;
        state.lerp.pos.tr_time = 0;
        state.lerp.pos.tr_duration = 0;
        state.lerp.pos.tr_base = now_at;
        state.lerp.pos.tr_delta = [0.0; 3];
        state.lerp.apos.tr_type = TR_STATIONARY;
        state.lerp.apos.tr_time = 0;
        state.lerp.apos.tr_duration = 0;
        state.lerp.apos.tr_base = angles;
        state.lerp.apos.tr_delta = [0.0; 3];
        state.ground_entity_num = trace_t6::ENTITYNUM_WORLD as i16;
    }
    if let Some(item) = world.dropped_item_mut_by_number(number) {
        item.state = state;
        item.origin = now_at;
        item.falling = state.lerp.pos.tr_type != TR_STATIONARY;
    }
}

/// The world is the only ground a gun comes to rest on here.
fn on_ground(state: &entity_t6::EntityState) -> bool {
    state.ground_entity_num == trace_t6::ENTITYNUM_WORLD as i16
}

/// Where G_RunItem moves a gun: its trajectory 50 ms on, or 1 unit down when it would barely
/// move.
fn ahead(state: &entity_t6::EntityState, origin: [f32; 3], time_ms: i32) -> [f32; 3] {
    let traj = Trajectory {
        tr_time: state.lerp.pos.tr_time,
        tr_type: state.lerp.pos.tr_type,
        tr_duration: state.lerp.pos.tr_duration,
        tr_delta: state.lerp.pos.tr_delta,
        tr_base: state.lerp.pos.tr_base,
    };
    let mut target = evaluate_trajectory(&traj, time_ms + RUN_AHEAD_MS);
    let d = sub(target, origin);
    if dot(d, d) < 0.1 {
        target[2] -= 1.0;
    }
    target
}

/// G_RunItem's slide off a steep face it hit at once: the move loses its part into the face
/// and is pushed 1 unit out of it.
fn slide(origin: [f32; 3], target: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let f = 1.0 - dot(normal, sub(target, origin));
    [
        target[0] + normal[0] * f,
        target[1] + normal[1] * f,
        target[2] + normal[2] * f,
    ]
}

/// G_OrientItemToGround: the gun keeps its heading along the ground it lies on, and a gun
/// lies on its side.
fn ground_angles(angles: [f32; 3], up: [f32; 3], gun: bool) -> [f32; 3] {
    let (forward, _, _) = angle_vectors(angles);
    let left = cross(up, forward);
    let forward = cross(left, up);
    let mut out = axis_to_angles([forward, left, up]);
    if gun {
        out[2] += 90.0;
    }
    out
}

/// SV_PointContents for the no-drop contents.
fn in_nodrop(world: &FrameWorld, at: [f32; 3]) -> bool {
    let t = world.trace_clip(at, at, [0.0; 3], [0.0; 3], CONTENTS_NODROP);
    t.startsolid || t.allsolid
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn lerp(a: [f32; 3], b: [f32; 3], f: f32) -> [f32; 3] {
    [
        (b[0] - a[0]) * f + a[0],
        (b[1] - a[1]) * f + a[1],
        (b[2] - a[2]) * f + a[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(owner: i32, clear: i32) -> DroppedItem {
        let mut state = entity_t6::EntityState::default();
        state.client_num = owner as u8;
        state.index = 5;
        DroppedItem {
            state,
            origin: [0.0; 3],
            falling: false,
            clip_r: 3,
            clip_l: 2,
            stock: 40,
            stock2: 7,
            scavenger: false,
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
        let mut s = entity_t6::EntityState::default();
        s.lerp.pos.tr_type = TR_LINEAR_STOP;
        s.lerp.pos.tr_time = 0;
        s.lerp.pos.tr_duration = RUN_AHEAD_MS;
        s.lerp.pos.tr_base = [0.0, 0.0, 10.0];
        s.lerp.pos.tr_delta = [0.0, 0.0, -100.0];
        // The stop is done: it would not move, so it looks 1 unit down.
        assert_eq!(ahead(&s, [0.0, 0.0, 5.0], 100), [0.0, 0.0, 4.0]);
        s.lerp.pos.tr_type = TR_GRAVITY;
        s.lerp.pos.tr_delta = [0.0; 3];
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
        assert!(owner_blocks(&it, ClientId(2), 999));
        assert!(!owner_blocks(&it, ClientId(2), 1000));
        assert!(!owner_blocks(&it, ClientId(3), 0));
        // legacy items (no clear time) never block.
        assert!(!owner_blocks(&item(2, 0), ClientId(2), 0));
    }

    #[test]
    fn the_use_button_follows_bo2s_gates() {
        use playerstate_t6::game::pm_flags::{LADDER, MANTLE, SPRINTING};
        let ps = playerstate_t6::game::PlayerState::ZERO;
        let t6 = T6WeaponPs::default();
        assert!(!hint_blocked(&t6, &ps));
        assert!(press_picks(&t6) && hold_takes(&t6, &ps));
        for flag in [MANTLE, LADDER, SPRINTING, movement_t6::bridge::PMF_DIVE] {
            let mut ps = ps;
            ps.pm_flags = flag;
            assert!(hint_blocked(&t6, &ps), "{flag:#x}");
        }
        let mut last_stand = ps;
        last_stand.pm_type = playerstate_t6::game::PM_TYPE_LAST_STAND;
        assert!(hint_blocked(&t6, &last_stand));
        let mut locked = t6;
        locked.weap_flags = weapon_t6::player::wf::CHANGE_LOCKED;
        assert!(hint_blocked(&locked, &ps));
        fn at(t6: &T6WeaponPs, weaponstate: i32) -> T6WeaponPs {
            let mut held = *t6;
            held.weaponstate = weaponstate;
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
        mantle.pm_flags = MANTLE;
        assert!(!hold_takes(&t6, &mantle));
    }

    #[test]
    fn he_touches_a_gun_36_across_and_18_below_to_88_above() {
        let lying = |p: [f32; 3]| {
            let mut it = item(0, 0);
            it.origin = p;
            it.state.lerp.pos.tr_base = p;
            it
        };
        let mut ps = playerstate_t6::game::PlayerState::ZERO;
        let touches = |ps: &playerstate_t6::game::PlayerState, p| touches_item(ps, &lying(p), 0);
        assert!(touches(&ps, [36.0, -36.0, 0.0]));
        assert!(!touches(&ps, [36.5, 0.0, 0.0]));
        assert!(touches(&ps, [0.0, 0.0, -18.0]));
        assert!(!touches(&ps, [0.0, 0.0, -18.5]));
        assert!(touches(&ps, [0.0, 0.0, 88.0]));
        assert!(!touches(&ps, [0.0, 0.0, 88.5]));
        // Crouched his box is 50 high: grown 20, and the gun's own unit, 71 up at most.
        ps.view_height_target = movement_t6::bridge::view_height::CROUCH;
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
