//! What the player's own commands may do this frame: the script-set restrictions (frozen, stunned, linked, the
//! `allow*` switches) applied to a command before movement and weapons see it.

use crate::frame::FrameWorld;
use crate::match_state::ScriptControls;
use crate::world::ClientId;
use playerstate_t6::game::{UserCmd, buttons as btn, pm_flags as pmf};

/// The buttons the client holds this frame: every command it sent, ORed together, or its last buttons when it sent
/// none.
pub(crate) fn buttons(world: &mut FrameWorld, id: ClientId) -> u32 {
    let sent: Vec<u32> = world
        .ecs()
        .resource::<crate::step::StepRequest>()
        .input
        .cmds
        .iter()
        .filter(|(client, _)| *client == id)
        .map(|(_, cmd)| cmd.buttons)
        .collect();
    if !sent.is_empty() {
        return sent.into_iter().fold(0, |all, held| all | held);
    }
    world
        .old_buttons_mut()
        .iter()
        .find(|(client, _)| *client == id)
        .map_or(0, |(_, held)| *held)
}

/// The script switches `allowprone` / `allowcrouch` / `allowstand` / `allowsprint` / `allowmelee` / `allowads`
/// (when false) drop the request before movement runs. A stance request is a held button (no button is a stand), so
/// a stand that is not allowed keeps the player in the stance he has, or else the first one that is allowed.
pub(crate) fn apply_allow_controls(controls: &ScriptControls, pm_flags: u32, cmd: &mut UserCmd) {
    let drops = [
        (controls.prone_disabled, btn::PRONE),
        (controls.crouch_disabled, btn::CROUCH),
    ];
    for (disabled, button) in drops {
        if disabled {
            cmd.buttons &= !button;
        }
    }
    if controls.stand_disabled && cmd.buttons & (btn::PRONE | btn::CROUCH) == 0 {
        let may_prone = !controls.prone_disabled;
        let may_crouch = !controls.crouch_disabled;
        let was_prone = pm_flags & pmf::PRONE != 0;
        cmd.buttons |= if was_prone && may_prone {
            btn::PRONE
        } else if may_crouch {
            btn::CROUCH
        } else if may_prone {
            btn::PRONE
        } else {
            0
        };
    }
    let later = [
        (controls.sprint_disabled, btn::SPRINT),
        (controls.melee_disabled, btn::MELEE_CHARGE),
        (controls.ads_disabled, btn::ADS),
    ];
    for (disabled, button) in later {
        if disabled {
            cmd.buttons &= !button;
        }
    }
}

/// Black Ops II Zombies' rule for a stance the scripts disallow (the TranZit bus, Mob's afterlife, Origins' tank and
/// claw): the stance asked for becomes the nearest allowed one, judged by the stance he is in now. Without prone
/// there is no dive either (a dive starts on a prone press).
pub(crate) fn bo2_zombies_stance(controls: &ScriptControls, pm_flags: u32, cmd: &mut UserCmd) {
    let (no_stand, no_crouch, no_prone) = (
        controls.stand_disabled,
        controls.crouch_disabled,
        controls.prone_disabled,
    );
    let prone_now = pm_flags & pmf::PRONE != 0;
    let down_now = pm_flags & (pmf::PRONE | pmf::CROUCH) != 0;
    let asks_prone = cmd.buttons & btn::PRONE != 0;
    let asks_crouch = cmd.buttons & btn::CROUCH != 0;
    let result = if asks_prone {
        if !no_prone {
            return;
        }
        if (!down_now && !no_stand) || no_crouch { 0 } else { btn::CROUCH }
    } else if asks_crouch {
        if !no_crouch {
            return;
        }
        if (down_now || no_prone) && !no_stand { 0 } else { btn::PRONE }
    } else {
        if !no_stand {
            return;
        }
        if (!prone_now || no_prone) && !no_crouch { btn::CROUCH } else { btn::PRONE }
    };
    cmd.buttons = (cmd.buttons & !(btn::PRONE | btn::CROUCH)) | result;
}

/// Frozen, stunned, linked and jump-disabled players: movement and the buttons that need it.
fn restrict_movement(controls: &ScriptControls, cmd: &mut UserCmd) {
    if controls.frozen {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
        cmd.buttons &= !(btn::JUMP
            | btn::SPRINT
            | btn::ATTACK
            | btn::MELEE_CHARGE
            | btn::FRAG
            | btn::SMOKE
            | btn::RELOAD
            | btn::USE_RELOAD);
    }
    if controls.stunned {
        cmd.forwardmove /= 2;
        cmd.rightmove /= 2;
        cmd.buttons &= !(btn::JUMP | btn::SPRINT);
    }
    if controls.linked {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
        cmd.buttons &= !(btn::JUMP | btn::SPRINT);
    }
    if controls.jump_disabled {
        cmd.buttons &= !btn::JUMP;
    }
}

/// Disabled weapons, firing, offhands and use.
fn restrict_actions(controls: &ScriptControls, cmd: &mut UserCmd) {
    if controls.weapons_disabled {
        cmd.buttons &= !(btn::ATTACK | btn::THROW | btn::ADS | btn::MELEE_CHARGE);
    }
    if controls.fire_disabled {
        cmd.buttons &= !(btn::ATTACK | btn::THROW | btn::MELEE_CHARGE);
    }
    if controls.offhands_disabled || controls.weapons_disabled || controls.fire_disabled {
        cmd.buttons &= !(btn::FRAG | btn::SMOKE);
    }
    if controls.usability_disabled {
        cmd.buttons &= !(btn::USE | btn::USE_RELOAD);
    }
}

/// Whether the player is steering a scorestreak (the Hellstorm's missile, a drone or vehicle, the Lodestar). His keys
/// switch no weapon then: the engine holds the weapon, though the streak's own script does not (the Hellstorm clears
/// `usingremote` and turns weapon cycling back on while he still rides the missile, so a streak raised then would
/// start beside it).
fn is_steering(world: &FrameWorld, id: ClientId) -> bool {
    let missile_in_flight = world
        .client_meta(id)
        .and_then(|meta| meta.remote_missile)
        .is_some_and(|link| link.unlink_at_ms.is_none());
    missile_in_flight || crate::t6::remote_control(world.ecs_read(), id.0)
}

/// Applies every script-set restriction to a command before the step uses it. `zombies` selects Zombies' stance rule.
pub(crate) fn constrain_cmd(world: &mut FrameWorld, id: ClientId, cmd: &mut UserCmd, zombies: bool) {
    let held = world.player(id).map_or(0, |ps| ps.weapon);
    let Some(mut controls) = world.client_meta(id).map(|meta| meta.controls) else {
        return;
    };
    // A weapon switch set by script stays asked for until his own command asks for it as well: his client selects
    // the gun (G_SelectWeapon), so a gun set at once (setSpawnWeapon, an immediate switch) is not put away by a
    // stale select still in flight.
    if controls.switch_to != 0 && controls.switch_to == held && u32::from(cmd.weapon) == held {
        controls.switch_to = 0;
        world.client_meta_mut(id).controls.switch_to = 0;
    }
    // The use/reload button reloads when there is nothing to use (his client adds this to his own commands; a bot's
    // come here), or when a tap of it let go before it took an item.
    let nothing_to_use = world.player(id).is_some_and(|ps| ps.cursor_hint == 0);
    if cmd.buttons & btn::USE_RELOAD != 0 && nothing_to_use {
        cmd.buttons |= btn::RELOAD;
    }
    if world.client_meta(id).is_some_and(|meta| meta.item_use_reload) {
        world.client_meta_mut(id).item_use_reload = false;
        cmd.buttons |= btn::RELOAD;
    }
    restrict_movement(&controls, cmd);
    let pm_flags = world.player(id).map_or(0, |ps| ps.pm_flags);
    if zombies {
        bo2_zombies_stance(&controls, pm_flags, cmd);
    }
    apply_allow_controls(&controls, pm_flags, cmd);
    restrict_actions(&controls, cmd);
    // A guest weapon (Bow, Stick, Gravity Gun, Portal Gun) runs its own right button in `t6::mp_specials` (the Portal
    // Gun's orange); the Ray Gun under it never aims down its sights.
    if crate::t6::mp_specials::owns_trigger(world.weapon_script_name(held)) {
        cmd.buttons &= !btn::ADS;
    }
    let steering = is_steering(world, id);
    if controls.switch_to != 0 {
        cmd.weapon = controls.switch_to as u16;
    } else if controls.switch_disabled || controls.frozen || steering {
        cmd.weapon = held as u16;
        if steering {
            cmd.buttons &= !btn::INVENTORY;
        }
    }
}

#[cfg(test)]
mod allow_tests {
    use super::*;

    fn cmd_with(held: u32) -> UserCmd {
        let mut cmd = UserCmd::default();
        cmd.buttons = held;
        cmd
    }

    #[test]
    fn nothing_is_dropped_by_default() {
        let all = btn::PRONE | btn::CROUCH | btn::SPRINT | btn::MELEE_CHARGE | btn::ADS;
        let mut cmd = cmd_with(all);
        apply_allow_controls(&ScriptControls::default(), 0, &mut cmd);
        assert_eq!(cmd.buttons, all);
    }

    #[test]
    fn each_switch_drops_its_own_request_only() {
        let all = btn::PRONE | btn::CROUCH | btn::SPRINT | btn::MELEE_CHARGE | btn::ADS;
        let cases = [
            (ScriptControls { prone_disabled: true, ..Default::default() }, btn::PRONE),
            (ScriptControls { crouch_disabled: true, ..Default::default() }, btn::CROUCH),
            (ScriptControls { sprint_disabled: true, ..Default::default() }, btn::SPRINT),
            (ScriptControls { melee_disabled: true, ..Default::default() }, btn::MELEE_CHARGE),
            (ScriptControls { ads_disabled: true, ..Default::default() }, btn::ADS),
        ];
        for (controls, dropped) in cases {
            let mut cmd = cmd_with(all);
            apply_allow_controls(&controls, 0, &mut cmd);
            assert_eq!(cmd.buttons, all & !dropped);
        }
    }

    #[test]
    fn a_disallowed_stand_keeps_the_stance_he_has() {
        let no_stand = ScriptControls { stand_disabled: true, ..Default::default() };
        // Standing and asking for nothing: put into a crouch.
        let mut cmd = cmd_with(0);
        apply_allow_controls(&no_stand, 0, &mut cmd);
        assert_eq!(cmd.buttons, btn::CROUCH);
        // Prone and asking for nothing: stays prone.
        let mut cmd = cmd_with(0);
        apply_allow_controls(&no_stand, pmf::PRONE, &mut cmd);
        assert_eq!(cmd.buttons, btn::PRONE);
        // No crouch either: prone.
        let neither = ScriptControls { stand_disabled: true, crouch_disabled: true, ..Default::default() };
        let mut cmd = cmd_with(0);
        apply_allow_controls(&neither, 0, &mut cmd);
        assert_eq!(cmd.buttons, btn::PRONE);
        // Already asking for a stance: left alone.
        let mut cmd = cmd_with(btn::CROUCH);
        apply_allow_controls(&no_stand, 0, &mut cmd);
        assert_eq!(cmd.buttons, btn::CROUCH);
    }

    #[test]
    fn frozen_players_cannot_move_or_act() {
        let frozen = ScriptControls { frozen: true, ..Default::default() };
        let mut cmd = cmd_with(btn::JUMP | btn::ATTACK | btn::ADS);
        cmd.forwardmove = 90;
        cmd.rightmove = -90;
        restrict_movement(&frozen, &mut cmd);
        assert_eq!((cmd.forwardmove, cmd.rightmove), (0, 0));
        assert_eq!(cmd.buttons, btn::ADS);
    }

    #[test]
    fn stunned_players_move_at_half_speed_and_cannot_jump() {
        let stunned = ScriptControls { stunned: true, ..Default::default() };
        let mut cmd = cmd_with(btn::JUMP | btn::SPRINT | btn::ATTACK);
        cmd.forwardmove = 100;
        cmd.rightmove = -50;
        restrict_movement(&stunned, &mut cmd);
        assert_eq!((cmd.forwardmove, cmd.rightmove), (50, -25));
        assert_eq!(cmd.buttons, btn::ATTACK);
    }

    #[test]
    fn disabled_fire_leaves_aiming_and_disables_offhands() {
        let no_fire = ScriptControls { fire_disabled: true, ..Default::default() };
        let mut cmd = cmd_with(btn::ATTACK | btn::THROW | btn::ADS | btn::FRAG | btn::SMOKE | btn::USE);
        restrict_actions(&no_fire, &mut cmd);
        assert_eq!(cmd.buttons, btn::ADS | btn::USE);
    }
}

#[cfg(test)]
mod zombies_stance_tests {
    use super::*;

    const STAND: u32 = 0;
    const CROUCH: u32 = btn::CROUCH;
    const PRONE: u32 = btn::PRONE;

    /// The stance a command asking for `asks`, from the stance `now`, ends up with when the scripts switch off
    /// (stand, crouch, prone): Zombies' rule first, then the plain drop, as `constrain_cmd` runs them.
    fn stance(off: (bool, bool, bool), now: u32, asks: u32) -> u32 {
        let controls = ScriptControls {
            stand_disabled: off.0,
            crouch_disabled: off.1,
            prone_disabled: off.2,
            ..Default::default()
        };
        let mut cmd = UserCmd::default();
        cmd.buttons = asks | btn::SPRINT;
        bo2_zombies_stance(&controls, now, &mut cmd);
        apply_allow_controls(&controls, now, &mut cmd);
        assert_ne!(cmd.buttons & btn::SPRINT, 0, "only the stance changes");
        cmd.buttons & (btn::PRONE | btn::CROUCH)
    }

    const NO_PRONE: (bool, bool, bool) = (false, false, true);
    const NO_CROUCH: (bool, bool, bool) = (false, true, false);
    const NO_STAND: (bool, bool, bool) = (true, false, false);
    const ONLY_PRONE: (bool, bool, bool) = (true, true, false);
    const ONLY_CROUCH: (bool, bool, bool) = (true, false, true);
    const ONLY_STAND: (bool, bool, bool) = (false, true, true);

    #[test]
    fn only_zombies_moves_him_past_the_crouch() {
        // `constrain_cmd` runs Zombies' rule in Zombies only; elsewhere the press is simply dropped.
        for (zombies, want) in [(true, PRONE), (false, STAND)] {
            let mut sim = crate::carrier::SimWorld::default();
            let mut world = sim.frame();
            let id = ClientId(0);
            world.client_meta_mut(id).controls.crouch_disabled = true;
            let mut cmd = UserCmd::default();
            cmd.buttons = CROUCH;
            constrain_cmd(&mut world, id, &mut cmd, zombies);
            assert_eq!(cmd.buttons & (PRONE | CROUCH), want, "zombies {zombies}");
        }
    }

    #[test]
    fn everything_allowed_changes_nothing() {
        for now in [0, pmf::CROUCH, pmf::PRONE] {
            for asks in [STAND, CROUCH, PRONE] {
                assert_eq!(stance((false, false, false), now, asks), asks);
            }
        }
    }

    #[test]
    fn without_prone_he_stays_standing_or_crouched() {
        // The TranZit bus: a prone press (or a dive) while standing stays standing.
        assert_eq!(stance(NO_PRONE, 0, PRONE), STAND);
        // Crouched, a prone press stays crouched (the plain drop would stand him up).
        assert_eq!(stance(NO_PRONE, pmf::CROUCH, PRONE), CROUCH);
        // Prone when prone was turned off: the prone button now crouches him.
        assert_eq!(stance(NO_PRONE, pmf::PRONE, PRONE), CROUCH);
        assert_eq!(stance(ONLY_STAND, pmf::CROUCH, PRONE), STAND);
        assert_eq!(stance(NO_PRONE, 0, CROUCH), CROUCH);
    }

    #[test]
    fn without_crouch_he_goes_past_the_crouch() {
        // Standing, a crouch press goes on down to prone; down already, it stands.
        assert_eq!(stance(NO_CROUCH, 0, CROUCH), PRONE);
        assert_eq!(stance(NO_CROUCH, pmf::PRONE, CROUCH), STAND);
        assert_eq!(stance(ONLY_PRONE, 0, CROUCH), PRONE);
        assert_eq!(stance(ONLY_STAND, 0, CROUCH), STAND);
    }

    #[test]
    fn without_stand_he_stays_down() {
        assert_eq!(stance(NO_STAND, 0, STAND), CROUCH);
        assert_eq!(stance(NO_STAND, pmf::PRONE, STAND), PRONE);
        assert_eq!(stance(ONLY_CROUCH, pmf::PRONE, STAND), CROUCH);
        assert_eq!(stance(ONLY_PRONE, 0, STAND), PRONE);
        assert_eq!(stance(NO_STAND, 0, CROUCH), CROUCH);
    }
}
