//! What the weapon keys do to the gun request each frame: the d-pad slots, the optic key, the
//! cycle keys, the inventory key and the grenade keys. The request itself, and the cycle walk,
//! live in `gun_pick`; this is where a key press is turned into a change of it.

use std::ops::RangeInclusive;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use input_t6::weapon_select::weapon_cycle_allowed;
use input_t6::{ClientInput, Kbutton};
use playerstate_t6::game::PlayerState;
use sim::{ClientAction, ClientId, SimWorld};
use weapon_t6::combat_facts::WeaponCombatFacts;

use super::gun_pick::{
    WeaponSelect, ammo_on_hand, cycle_weapon_select, next_earned_streak, select_flags, streak_slot_weapons,
};
use crate::ActionRequestIds;
use crate::authority::inbox::ClientActionInbox;
use crate::client::input::LookState;

/// A second optic switch waits this long after the last gun pick, so one tap makes one switch.
const OPTIC_SETTLE_MS: i32 = 350;
/// Weapon states while a grenade is being readied, thrown or put away; no gun pick then.
const THROW_STATES: RangeInclusive<i32> = 22..=29;
/// The entity flag that says the player sits in a vehicle.
const EF_IN_VEHICLE: u32 = 0x4000;
/// Player-state types from here up are the dead and spectating ones.
const FIRST_NON_PLAYING_PM_TYPE: i32 = 10;

/// The server-bound action queue and its id counter, either of which may be missing before a
/// match is set up; with either missing a request is simply not made.
#[derive(SystemParam)]
pub struct ActionRequests<'w> {
    inbox: Option<ResMut<'w, ClientActionInbox>>,
    ids: Option<ResMut<'w, ActionRequestIds>>,
}

impl ActionRequests<'_> {
    /// Queues the action `build` makes from a fresh request id. A queue that refuses it is logged
    /// under `label` and the request is dropped.
    pub(super) fn push(
        &mut self,
        local: ClientId,
        label: &str,
        build: impl FnOnce(sim::ActionRequestId) -> ClientAction,
    ) {
        let (Some(inbox), Some(ids)) = (self.inbox.as_mut(), self.ids.as_mut()) else {
            return;
        };
        let action = build(ids.allocate());
        if let Err(error) = inbox.push(local, action) {
            diag::warn!(Net, "{label}: not queued — {error}");
        }
    }
}

/// What the keys need to know about the gun in hand this frame.
pub(super) struct Held<'a> {
    pub ps: &'a PlayerState,
    pub world: &'a SimWorld,
    /// Client time now.
    pub now: i32,
    /// The grenade state of the brain, when it runs: raw weapon state and the grenade.
    pub offhand: Option<(i32, u32)>,
    /// The match-begins countdown or a script has the player frozen.
    pub frozen: bool,
}

impl Held<'_> {
    fn can_switch(&self, select: &WeaponSelect) -> bool {
        weapon_cycle_allowed(select_flags(self.ps), self.now, select.time, 0, 0)
    }

    fn row(&self, weapon: u32) -> Option<WeaponCombatFacts> {
        self.world.weapon_combat_row(weapon)
    }

    fn carries(&self, weapon: u32) -> bool {
        self.ps.weapons.contains(&(weapon as i32))
    }
}

/// Keeps the base gun of the request right while the request is the gun in hand: for an alt mode
/// that is the gun the mode belongs to, otherwise the gun itself.
pub(super) fn track_held_mapping(select: &mut WeaponSelect, held: &Held) {
    let ps = held.ps;
    if select.index != ps.weapon {
        return;
    }
    let in_alt_mode = held.row(ps.weapon).is_some_and(|row| row.inventory_type == 3);
    select.mapped_index = if in_alt_mode { ps.weapon_primary } else { ps.weapon };
}

/// Asks for `target`, a gun or one of its alt modes. `walked` is the base gun a walk already found;
/// otherwise the base is the target itself when carried, else the gun the request maps to. The
/// pick only stands if the player carries that base and the target is it or its alt mode.
fn switch_into_mode(select: &mut WeaponSelect, held: &Held, target: u32, walked: u32) -> bool {
    let base = if walked != 0 {
        walked
    } else if held.carries(target) {
        target
    } else {
        select.mapped_index
    };
    let is_base_or_its_mode =
        base == target || held.row(base).is_some_and(|row| row.alternate_weapon == target);
    if !(held.carries(base) && is_base_or_its_mode) {
        return false;
    }
    select.index = target;
    select.mapped_index = base;
    select.time = held.now;
    sim::remember_mode(held.world, &held.ps.weapons, &mut select.last_mode, target);
    true
}

/// The gun an alt-mode slot press asks for, with the base gun a walk found (0 when none was
/// needed). Nothing while the gun is busy; from an alt mode it goes back to the base; a gun with
/// a mode goes into it; any other gun walks on to the next one that has a mode.
fn alt_mode_pick(held: &Held, select: &WeaponSelect) -> Option<(u32, u32)> {
    if !sim::alt_toggle_allowed_by_state(held.world, held.ps) {
        return None;
    }
    let current = held.row(select.index)?;
    let (target, base) = if current.inventory_type == 3 {
        (select.mapped_index, 0)
    } else if current.alternate_weapon != 0 {
        (current.alternate_weapon, 0)
    } else {
        let ps = held.ps;
        let is_streak = |weapon: u32| streak_slot_weapons(ps).any(|streak| streak == weapon);
        let has_ammo = |weapon: u32| held.row(weapon).is_some_and(|row| ammo_on_hand(ps, weapon, &row) != 0);
        match sim::alt_toggle_walk(held.world, &ps.weapons, select.index, is_streak, has_ammo) {
            Some(base) => (held.row(base).map_or(0, |row| row.alternate_weapon), base),
            None => (0, 0),
        }
    };
    (target != 0).then_some((target, base))
}

/// The d-pad slot presses of this frame, in order. A slot holding a scorestreak brings out the
/// next earned one; a slot holding the attachment key switches mode. Each press needs the player
/// able to switch at that moment, so an earlier press that took effect can stop a later one.
pub(super) fn press_slots(select: &mut WeaponSelect, input: &mut ClientInput, held: &Held, slots: &[usize]) {
    for &slot in slots {
        if !held.can_switch(select) {
            continue;
        }
        let picked = match held.ps.action_slot_type.get(slot).copied() {
            Some(1) => next_earned_streak(held.ps, held.world, select.index).map(|gun| (gun, 0)),
            Some(2) => alt_mode_pick(held, select),
            _ => None,
        };
        let Some((target, base)) = picked.filter(|&(target, _)| target != 0) else {
            continue;
        };
        if switch_into_mode(select, held, target, base) {
            input_t6::set_ads(input, false);
        }
    }
}

/// Whether a press of the hold-breath key is a request for the other optic: only for a scope that
/// has one (the look state says so), and not while frozen.
pub(super) fn breath_key_switches_optic(client: &ClientInput, look: &LookState, frozen: bool) -> bool {
    look.breath_is_switch && !frozen && client.kb.holdbreath.was_pressed
}

/// Switches to the held gun's other optic, which stays aimed. The press flag outlives a frame, so
/// this waits until the last pick has landed (the gun in hand is the one requested) and a beat
/// after it.
pub(super) fn hybrid_optic(select: &mut WeaponSelect, held: &Held) {
    if select.index != held.ps.weapon || held.now.wrapping_sub(select.time) < OPTIC_SETTLE_MS {
        return;
    }
    let other_optic = held.row(select.index).map_or(0, |row| row.alternate_weapon);
    if other_optic != 0 && held.can_switch(select) {
        switch_into_mode(select, held, other_optic, 0);
    }
}

/// Whether a cycle press is also the copycat bind: while the kill replay plays, or in a
/// player-state type from 10 up.
pub(super) fn copycat_bind_active(pm_type: i32, in_killcam: bool) -> bool {
    in_killcam || pm_type >= FIRST_NON_PLAYING_PM_TYPE
}

/// The cycle presses of this frame; `true` is next gun, `false` previous. A next press may be the
/// copycat bind too. While frozen the presses are refused and the request stays as it is.
pub(super) fn run_cycles(
    select: &mut WeaponSelect,
    input: &mut ClientInput,
    held: &Held,
    cycles: &[bool],
    in_killcam: bool,
    requests: &mut ActionRequests,
    local: ClientId,
) {
    for &next in cycles {
        if next && copycat_bind_active(held.ps.pm_type, in_killcam) {
            requests.push(local, "use_copycat", |request_id| ClientAction::UseCopycat { request_id });
        }
        if held.frozen {
            diag::info!(Net, "bo2mp cycle: refused while frozen");
            continue;
        }
        cycle_weapon_select(select, input, held.ps, held.world, held.now, next, held.offhand);
    }
}

/// The player asked for the inventory weapon (the scorestreak from a care package) and it is not
/// in hand: remember the gun he leaves so a second press goes back, and ask for it.
fn take_up_inventory(select: &mut WeaponSelect, input: &mut ClientInput, held: &Held, target: u32) {
    if let Some(row) = held.row(select.index) {
        match row.inventory_type {
            0 => select.last_primary = select.index,
            3 if row.alternate_weapon != 0 => select.last_primary = row.alternate_weapon,
            _ => {}
        }
    }
    select.index = target;
    select.mapped_index = target;
    select.time = held.now;
    select.switching_to_inventory = true;
    input_t6::set_ads(input, false);
}

/// The inventory weapon is already up: a press goes back to the last gun held, if it is still
/// carried, and otherwise on to the next gun (not from a vehicle).
fn leave_inventory(select: &mut WeaponSelect, input: &mut ClientInput, held: &Held, can_switch: bool) {
    let back_to = select.last_primary;
    let inventory = held.ps.inventory_weapon.max(0) as u32;
    if back_to != 0 && back_to != inventory && held.carries(back_to) {
        if !can_switch {
            return;
        }
        diag::info!(Net, "bo2mp inventory: already up, back to {back_to}");
        let mode = sim::remembered_mode(held.world, &held.ps.weapons, &select.last_mode, back_to);
        select.mapped_index = held
            .row(mode)
            .filter(|row| row.inventory_type == 3)
            .map_or(mode, |row| row.alternate_weapon);
        select.index = mode;
        select.time = held.now;
        select.switching_to_inventory = false;
        input_t6::set_ads(input, false);
    } else if held.ps.e_flags & EF_IN_VEHICLE == 0 {
        diag::info!(Net, "bo2mp inventory: already up, cycling on");
        cycle_weapon_select(select, input, held.ps, held.world, held.now, true, held.offhand);
    }
}

/// Handles the inventory key. Returns whether this command must carry the inventory button: it
/// does from the press until the server's own snapshot has the player holding the weapon, so the
/// script that gives the weapon on the server knows the change came from the inventory.
/// `server_state` is the snapshot's player state, not the predicted one.
pub(super) fn inventory_key(
    select: &mut WeaponSelect,
    input: &mut ClientInput,
    held: &Held,
    requested: bool,
    server_state: Option<&PlayerState>,
) -> bool {
    let ps = held.ps;
    let target = ps.inventory_weapon.max(0) as u32;
    let can_switch = held.can_switch(select);
    if requested {
        diag::info!(
            Net,
            "bo2mp inventory: switch asked; inventory weapon {target}, holding {}, selected {}, carried {}, frozen {}, can switch {can_switch}",
            ps.weapon,
            select.index,
            held.carries(target),
            held.frozen
        );
    }
    let may_pick = requested && !held.frozen && !THROW_STATES.contains(&ps.weapon_state_t6) && target != 0;
    if may_pick {
        if target != ps.weapon && target != select.index {
            if held.carries(target) && can_switch {
                take_up_inventory(select, input, held, target);
            }
        } else {
            leave_inventory(select, input, held, can_switch);
        }
    }
    let seen_by_server = server_state.unwrap_or(ps);
    let still_arriving = select.switching_to_inventory
        && target != 0
        && select.index == seen_by_server.inventory_weapon.max(0) as u32
        && seen_by_server.weapon != select.index;
    if !still_arriving {
        select.switching_to_inventory = false;
    }
    still_arriving
}

/// The grenade the held grenade key asks for: the lethal one while the frag key is down or was
/// pressed this frame, else the tactical one for the smoke key.
pub(super) fn offhand_choice(frag: &Kbutton, smoke: &Kbutton, lethal: u16, tactical: u16) -> Option<u16> {
    if frag.active || frag.was_pressed {
        Some(lethal)
    } else if smoke.active || smoke.was_pressed {
        Some(tactical)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: ClientId = ClientId(1);
    const RIFLE: u32 = 1;
    const RIFLE_SCOPE: u32 = 2;
    const PISTOL: u32 = 3;
    const CARE_PACKAGE_GUN: u32 = 4;
    /// The playing bit `weapon_cycle_allowed` asks for.
    const PLAYING: u32 = 4;
    const NOW: i32 = 1000;

    /// Weapon ids 1 to 4: a rifle that has a second optic, that optic as an alt mode of it, a
    /// pistol, and a gun that arrives from the inventory.
    fn world() -> SimWorld {
        let row = |inventory_type, alternate_weapon| WeaponCombatFacts {
            inventory_type,
            alternate_weapon,
            ..Default::default()
        };
        let mut content = sim::SimContentBuilder::default();
        content.set_weapon_combat_table(vec![
            WeaponCombatFacts::default(),
            row(0, RIFLE_SCOPE),
            row(3, RIFLE),
            row(0, 0),
            row(0, 0),
        ]);
        let mut world = SimWorld::new();
        world.install_content(content.finish());
        world
    }

    fn player(carried: &[u32], in_hand: u32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        for (slot, weapon) in carried.iter().enumerate() {
            ps.weapons[slot] = *weapon as i32;
        }
        ps.weapon = in_hand;
        ps.weapon_primary = in_hand;
        ps.other_flags = PLAYING;
        ps
    }

    fn requesting(index: u32) -> WeaponSelect {
        WeaponSelect {
            index,
            mapped_index: index,
            held_index: index,
            ..WeaponSelect::default()
        }
    }

    fn holding<'a>(ps: &'a PlayerState, world: &'a SimWorld) -> Held<'a> {
        Held { ps, world, now: NOW, offhand: None, frozen: false }
    }

    fn aiming() -> ClientInput {
        let mut input = ClientInput::default();
        input.using_ads = true;
        input
    }

    #[test]
    fn a_held_base_gun_is_its_own_base() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        select.mapped_index = 99;
        track_held_mapping(&mut select, &holding(&ps, &world));
        assert_eq!(select.mapped_index, RIFLE);
    }

    #[test]
    fn a_held_alt_mode_maps_to_the_primary_it_belongs_to() {
        let world = world();
        let mut ps = player(&[RIFLE, PISTOL], RIFLE_SCOPE);
        ps.weapon_primary = RIFLE;
        let mut select = requesting(RIFLE_SCOPE);
        track_held_mapping(&mut select, &holding(&ps, &world));
        assert_eq!(select.mapped_index, RIFLE);
    }

    #[test]
    fn the_mapping_is_left_while_another_gun_is_requested() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(PISTOL);
        select.mapped_index = 77;
        track_held_mapping(&mut select, &holding(&ps, &world));
        assert_eq!(select.mapped_index, 77);
    }

    #[test]
    fn a_carried_gun_can_be_asked_for_directly() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        assert!(switch_into_mode(&mut select, &holding(&ps, &world), PISTOL, 0));
        assert_eq!((select.index, select.mapped_index, select.time), (PISTOL, PISTOL, NOW));
    }

    #[test]
    fn an_alt_mode_is_asked_for_through_the_gun_the_request_maps_to() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        assert!(switch_into_mode(&mut select, &holding(&ps, &world), RIFLE_SCOPE, 0));
        assert_eq!((select.index, select.mapped_index), (RIFLE_SCOPE, RIFLE));
    }

    #[test]
    fn a_gun_found_by_a_walk_is_the_base_of_its_mode() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], PISTOL));
        let mut select = requesting(PISTOL);
        assert!(switch_into_mode(&mut select, &holding(&ps, &world), RIFLE_SCOPE, RIFLE));
        assert_eq!((select.index, select.mapped_index), (RIFLE_SCOPE, RIFLE));
    }

    #[test]
    fn a_mode_of_a_gun_the_request_does_not_map_to_is_refused() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], PISTOL));
        let mut select = requesting(PISTOL);
        let before = select;
        assert!(!switch_into_mode(&mut select, &holding(&ps, &world), RIFLE_SCOPE, 0));
        assert_eq!(select, before);
    }

    #[test]
    fn a_gun_the_player_does_not_carry_is_refused() {
        let (world, ps) = (world(), player(&[RIFLE], RIFLE));
        let mut select = requesting(RIFLE);
        let before = select;
        assert!(!switch_into_mode(&mut select, &holding(&ps, &world), PISTOL, 0));
        assert_eq!(select, before);
    }

    #[test]
    fn the_attachment_slot_switches_into_the_guns_mode_and_lowers_the_aim() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        ps.action_slot_type = [0, 0, 2, 0];
        let mut select = requesting(RIFLE);
        let mut input = aiming();
        press_slots(&mut select, &mut input, &holding(&ps, &world), &[2]);
        assert_eq!((select.index, select.mapped_index), (RIFLE_SCOPE, RIFLE));
        assert!(!input.using_ads);
    }

    #[test]
    fn the_attachment_slot_from_a_mode_goes_back_to_the_base_gun() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE_SCOPE));
        ps.action_slot_type = [2, 0, 0, 0];
        let mut select = requesting(RIFLE_SCOPE);
        select.mapped_index = RIFLE;
        press_slots(&mut select, &mut ClientInput::default(), &holding(&ps, &world), &[0]);
        assert_eq!((select.index, select.mapped_index), (RIFLE, RIFLE));
    }

    #[test]
    fn a_slot_press_does_nothing_while_the_player_cannot_switch() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        ps.other_flags = 0;
        ps.action_slot_type = [2, 0, 0, 0];
        let mut select = requesting(RIFLE);
        let mut input = aiming();
        press_slots(&mut select, &mut input, &holding(&ps, &world), &[0]);
        assert_eq!(select, requesting(RIFLE));
        assert!(input.using_ads);
    }

    #[test]
    fn a_slot_press_does_nothing_while_the_gun_is_mid_switch() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        ps.weapon_state_t6 = 3;
        ps.action_slot_type = [2, 0, 0, 0];
        let mut select = requesting(RIFLE);
        press_slots(&mut select, &mut ClientInput::default(), &holding(&ps, &world), &[0]);
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn empty_and_unknown_slots_do_nothing() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        ps.action_slot_type = [0, 7, 0, 0];
        let mut select = requesting(RIFLE);
        press_slots(&mut select, &mut ClientInput::default(), &holding(&ps, &world), &[0, 1, 9]);
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn a_streak_slot_without_an_earned_streak_does_nothing() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.action_slot_type = [1, 0, 0, 0];
        ps.action_slot_param = [CARE_PACKAGE_GUN as i32, 0, 0, 0];
        let mut select = requesting(RIFLE);
        press_slots(&mut select, &mut ClientInput::default(), &holding(&ps, &world), &[0]);
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn two_slot_presses_in_a_row_are_judged_one_at_a_time() {
        let (world, mut ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        ps.action_slot_type = [2, 2, 0, 0];
        let mut select = requesting(RIFLE);
        press_slots(&mut select, &mut ClientInput::default(), &holding(&ps, &world), &[0, 1]);
        // The second press is the mode's own slot: it goes back.
        assert_eq!((select.index, select.mapped_index), (RIFLE, RIFLE));
    }

    #[test]
    fn the_breath_key_is_the_optic_key_only_when_the_scope_says_so() {
        let mut client = ClientInput::default();
        let mut look = LookState::default();
        assert!(!breath_key_switches_optic(&client, &look, false));
        client.kb.holdbreath.was_pressed = true;
        assert!(!breath_key_switches_optic(&client, &look, false));
        look.breath_is_switch = true;
        assert!(breath_key_switches_optic(&client, &look, false));
        assert!(!breath_key_switches_optic(&client, &look, true));
    }

    #[test]
    fn the_optic_switch_takes_the_guns_other_optic() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        select.time = 0;
        hybrid_optic(&mut select, &holding(&ps, &world));
        assert_eq!((select.index, select.mapped_index, select.time), (RIFLE_SCOPE, RIFLE, NOW));
    }

    #[test]
    fn the_optic_switch_waits_a_beat_after_the_last_pick() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        select.time = NOW - 349;
        hybrid_optic(&mut select, &holding(&ps, &world));
        assert_eq!(select.index, RIFLE);

        select.time = NOW - 350;
        hybrid_optic(&mut select, &holding(&ps, &world));
        assert_eq!(select.index, RIFLE_SCOPE);
    }

    #[test]
    fn the_optic_switch_waits_until_the_last_pick_has_landed() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(PISTOL);
        select.time = 0;
        hybrid_optic(&mut select, &holding(&ps, &world));
        assert_eq!(select.index, PISTOL);
    }

    #[test]
    fn a_gun_with_one_optic_has_nothing_to_switch_to() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], PISTOL));
        let mut select = requesting(PISTOL);
        select.time = 0;
        hybrid_optic(&mut select, &holding(&ps, &world));
        assert_eq!(select, WeaponSelect { time: 0, ..requesting(PISTOL) });
    }

    #[test]
    fn the_copycat_bind_is_a_killcam_or_a_high_player_type() {
        assert!(!copycat_bind_active(0, false));
        assert!(!copycat_bind_active(9, false));
        assert!(copycat_bind_active(10, false));
        assert!(copycat_bind_active(0, true));
    }

    #[test]
    fn a_next_press_moves_the_request_on_one_gun() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        let mut input = aiming();
        let mut requests = bare_requests();
        run_cycles(&mut select, &mut input, &holding(&ps, &world), &[true], false, &mut requests, ME);
        assert_eq!((select.index, select.mapped_index, select.last_primary), (PISTOL, PISTOL, RIFLE));
        assert!(!input.using_ads);
    }

    #[test]
    fn cycle_presses_are_refused_while_frozen() {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        let held = Held { frozen: true, ..holding(&ps, &world) };
        let mut requests = bare_requests();
        run_cycles(&mut select, &mut ClientInput::default(), &held, &[true, false], false, &mut requests, ME);
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn the_inventory_weapon_is_asked_for_and_the_gun_left_is_remembered() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(RIFLE);
        let mut input = aiming();
        let carry_button = inventory_key(&mut select, &mut input, &holding(&ps, &world), true, None);
        assert!(carry_button);
        assert_eq!(
            (select.index, select.mapped_index, select.last_primary, select.time),
            (CARE_PACKAGE_GUN, CARE_PACKAGE_GUN, RIFLE, NOW)
        );
        assert!(select.switching_to_inventory);
        assert!(!input.using_ads);
    }

    #[test]
    fn without_a_press_nothing_is_asked_for() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(RIFLE);
        let carry_button = inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), false, None);
        assert!(!carry_button);
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn the_inventory_press_is_ignored_while_frozen_or_throwing_or_with_nothing_in_the_inventory() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;

        let frozen = Held { frozen: true, ..holding(&ps, &world) };
        let mut select = requesting(RIFLE);
        assert!(!inventory_key(&mut select, &mut ClientInput::default(), &frozen, true, None));
        assert_eq!(select, requesting(RIFLE));

        let mut throwing = ps.clone();
        throwing.weapon_state_t6 = 25;
        assert!(!inventory_key(&mut select, &mut ClientInput::default(), &holding(&throwing, &world), true, None));
        assert_eq!(select, requesting(RIFLE));

        let mut empty = ps.clone();
        empty.inventory_weapon = 0;
        assert!(!inventory_key(&mut select, &mut ClientInput::default(), &holding(&empty, &world), true, None));
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn an_inventory_weapon_the_player_does_not_carry_is_not_asked_for() {
        let (world, mut ps) = (world(), player(&[RIFLE], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(RIFLE);
        assert!(!inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), true, None));
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn the_inventory_press_waits_for_the_player_to_be_able_to_switch() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        ps.other_flags = 0;
        let mut select = requesting(RIFLE);
        assert!(!inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), true, None));
        assert_eq!(select, requesting(RIFLE));
    }

    #[test]
    fn a_second_inventory_press_goes_back_to_the_last_gun() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], CARE_PACKAGE_GUN));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(CARE_PACKAGE_GUN);
        select.last_primary = RIFLE;
        select.switching_to_inventory = true;
        let mut input = aiming();
        let carry_button = inventory_key(&mut select, &mut input, &holding(&ps, &world), true, None);
        assert!(!carry_button);
        assert_eq!((select.index, select.mapped_index, select.time), (RIFLE, RIFLE, NOW));
        assert!(!select.switching_to_inventory);
        assert!(!input.using_ads);
    }

    #[test]
    fn a_second_inventory_press_with_no_last_gun_cycles_on() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], CARE_PACKAGE_GUN));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(CARE_PACKAGE_GUN);
        inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), true, None);
        assert_eq!(select.index, RIFLE);
    }

    #[test]
    fn a_second_inventory_press_in_a_vehicle_stays_put() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], CARE_PACKAGE_GUN));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        ps.e_flags = EF_IN_VEHICLE;
        let mut select = requesting(CARE_PACKAGE_GUN);
        inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), true, None);
        assert_eq!(select, requesting(CARE_PACKAGE_GUN));
    }

    #[test]
    fn the_inventory_button_goes_with_every_command_until_the_server_holds_the_weapon() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(CARE_PACKAGE_GUN);
        select.switching_to_inventory = true;
        let held = holding(&ps, &world);
        let mut input = ClientInput::default();

        let server_still_on_rifle = ps.clone();
        assert!(inventory_key(&mut select, &mut input, &held, false, Some(&server_still_on_rifle)));
        assert!(select.switching_to_inventory);

        let mut server_has_it = ps.clone();
        server_has_it.weapon = CARE_PACKAGE_GUN;
        assert!(!inventory_key(&mut select, &mut input, &held, false, Some(&server_has_it)));
        assert!(!select.switching_to_inventory);
    }

    #[test]
    fn without_a_server_state_the_predicted_one_decides_the_inventory_button() {
        let (world, mut ps) = (world(), player(&[RIFLE, CARE_PACKAGE_GUN], RIFLE));
        ps.inventory_weapon = CARE_PACKAGE_GUN as i32;
        let mut select = requesting(CARE_PACKAGE_GUN);
        select.switching_to_inventory = true;
        assert!(inventory_key(&mut select, &mut ClientInput::default(), &holding(&ps, &world), false, None));
    }

    fn key(active: bool, was_pressed: bool) -> Kbutton {
        let mut button = Kbutton::default();
        button.active = active;
        button.was_pressed = was_pressed;
        button
    }

    #[test]
    fn the_frag_key_picks_the_lethal_grenade_and_the_smoke_key_the_tactical_one() {
        assert_eq!(offhand_choice(&key(true, false), &key(false, false), 7, 8), Some(7));
        assert_eq!(offhand_choice(&key(false, true), &key(false, false), 7, 8), Some(7));
        assert_eq!(offhand_choice(&key(false, false), &key(true, false), 7, 8), Some(8));
        assert_eq!(offhand_choice(&key(false, false), &key(false, true), 7, 8), Some(8));
    }

    #[test]
    fn the_frag_key_wins_when_both_are_down_and_neither_means_no_choice() {
        assert_eq!(offhand_choice(&key(true, false), &key(true, false), 7, 8), Some(7));
        assert_eq!(offhand_choice(&key(false, false), &key(false, false), 7, 8), None);
    }

    // -- The action queue ------------------------------------------------------------------

    /// A queue handle for tests that never look at what was queued.
    fn bare_requests() -> ActionRequests<'static> {
        ActionRequests { inbox: None, ids: None }
    }

    fn queue_two_slot_presses(mut requests: ActionRequests) {
        requests.push(ME, "action_slot", |request_id| ClientAction::ActionSlot { request_id, slot: 2 });
        requests.push(ME, "action_slot", |request_id| ClientAction::ActionSlot { request_id, slot: 0 });
    }

    fn press_next_and_previous_in_the_killcam(mut requests: ActionRequests) {
        let (world, ps) = (world(), player(&[RIFLE, PISTOL], RIFLE));
        let mut select = requesting(RIFLE);
        let held = holding(&ps, &world);
        run_cycles(&mut select, &mut ClientInput::default(), &held, &[true, false], true, &mut requests, ME);
    }

    fn app_with_queue() -> App {
        let mut app = App::new();
        app.init_resource::<ClientActionInbox>();
        app.init_resource::<ActionRequestIds>();
        app
    }

    fn queued(app: &App) -> Vec<(ClientId, ClientAction)> {
        app.world().resource::<ClientActionInbox>().pending().to_vec()
    }

    #[test]
    fn requests_get_one_new_id_each_and_wait_in_the_queue() {
        let mut app = app_with_queue();
        app.add_systems(Update, queue_two_slot_presses);
        app.update();
        let pending = queued(&app);
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|(who, _)| *who == ME));
        let ids: Vec<_> = pending.iter().map(|(_, action)| sim::action_request_id(action)).collect();
        assert_eq!(ids, [1, 2]);
        assert!(matches!(pending[0].1, ClientAction::ActionSlot { slot: 2, .. }));
        assert!(matches!(pending[1].1, ClientAction::ActionSlot { slot: 0, .. }));
    }

    #[test]
    fn without_the_id_counter_nothing_is_queued() {
        let mut app = App::new();
        app.init_resource::<ClientActionInbox>();
        app.add_systems(Update, queue_two_slot_presses);
        app.update();
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn without_the_queue_nothing_happens_and_nothing_panics() {
        let mut app = App::new();
        app.init_resource::<ActionRequestIds>();
        app.add_systems(Update, queue_two_slot_presses);
        app.update();
        // No id was spent on the refused requests: the counter is still at its start.
        assert_eq!(app.world_mut().resource_mut::<ActionRequestIds>().allocate(), 1);
    }

    #[test]
    fn only_a_next_press_in_the_killcam_is_the_copycat_bind() {
        let mut app = app_with_queue();
        app.add_systems(Update, press_next_and_previous_in_the_killcam);
        app.update();
        let pending = queued(&app);
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0].1, ClientAction::UseCopycat { .. }));
    }
}
