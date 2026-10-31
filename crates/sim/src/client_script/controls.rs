//! What the player's own commands may do this frame: the script-set restrictions (frozen, stunned, linked, the
//! `allow*` switches) applied to a command before movement and weapons see it.

use crate::step_view::StepWorldView;
use crate::player_ledger::ScriptControls;
use crate::sim_parts::SeatNo;
use playerstate_t6::game::{InputCommand, buttons, movement_state_bits};

/// The buttons the client holds this frame: every command it sent, ORed together, or its last buttons when it sent
/// none.
pub(crate) fn buttons(world: &mut StepWorldView, seat: SeatNo) -> u32 {
    let sent: Vec<u32> = world
        .ecs()
        .resource::<crate::frame_step::FrameTicket>()
        .input
        .cmds
        .iter()
        .filter(|(who, _)| *who == seat)
        .map(|(_, cmd)| cmd.buttons)
        .collect();
    if !sent.is_empty() {
        return sent.into_iter().fold(0, |all, held| all | held);
    }
    world
        .prev_button_state_mut()
        .iter()
        .find(|(who, _)| *who == seat)
        .map_or(0, |(_, held)| *held)
}

/// `getnormalizedmovement`: this frame's move stick as (forward, right, 0),
/// each -1..1 (127 = full). No command this frame reads as standing still.
pub(crate) fn movement(world: &mut StepWorldView, seat: SeatNo) -> [f32; 3] {
    let input = &world.ecs().resource::<crate::frame_step::FrameTicket>().input;
    input
        .cmds
        .iter()
        .filter(|(who, _)| *who == seat)
        .last()
        .map_or([0.0; 3], |(_, cmd)| {
            [
                (cmd.thrust_ahead as f32 / 127.0).clamp(-1.0, 1.0),
                (cmd.thrust_lateral as f32 / 127.0).clamp(-1.0, 1.0),
                0.0,
            ]
        })
}

/// The script switches `allowprone` / `allowcrouch` / `allowstand` / `allowsprint` / `allowmelee` / `allowads`
/// (when false) drop the request before movement runs. A stance request is a held button (no button is a stand), so
/// a stand that is not allowed keeps the player in the stance he has, or else the first one that is allowed.
pub(crate) fn apply_allow_controls(controls: &ScriptControls, movement_state_bits: u32, cmd: &mut InputCommand) {
    let drops = [
        (controls.prone_disabled, buttons::LYING_FLAT),
        (controls.crouch_disabled, buttons::DUCK_BUTTON_BIT),
    ];
    for (disabled, button) in drops {
        if disabled {
            cmd.buttons &= !button;
        }
    }
    if controls.stand_disabled && cmd.buttons & (buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT) == 0 {
        let may_prone = !controls.prone_disabled;
        let may_crouch = !controls.crouch_disabled;
        let was_prone = movement_state_bits & movement_state_bits::LYING_FLAT != 0;
        cmd.buttons |= if was_prone && may_prone {
            buttons::LYING_FLAT
        } else if may_crouch {
            buttons::DUCK_BUTTON_BIT
        } else if may_prone {
            buttons::LYING_FLAT
        } else {
            0
        };
    }
    let later = [
        (controls.sprint_disabled, buttons::SPRINT),
        (controls.melee_disabled, buttons::MELEE_CHARGE),
        (controls.ads_disabled, buttons::ADS),
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
pub(crate) fn bo2_zombies_stance(controls: &ScriptControls, movement_state_bits: u32, cmd: &mut InputCommand) {
    let (no_stand, no_crouch, no_prone) = (
        controls.stand_disabled,
        controls.crouch_disabled,
        controls.prone_disabled,
    );
    let prone_now = movement_state_bits & movement_state_bits::LYING_FLAT != 0;
    let down_now = movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT) != 0;
    let asks_prone = cmd.buttons & buttons::LYING_FLAT != 0;
    let asks_crouch = cmd.buttons & buttons::DUCK_BUTTON_BIT != 0;
    let result = if asks_prone {
        if !no_prone {
            return;
        }
        if (!down_now && !no_stand) || no_crouch { 0 } else { buttons::DUCK_BUTTON_BIT }
    } else if asks_crouch {
        if !no_crouch {
            return;
        }
        if (down_now || no_prone) && !no_stand { 0 } else { buttons::LYING_FLAT }
    } else {
        if !no_stand {
            return;
        }
        if (!prone_now || no_prone) && !no_crouch { buttons::DUCK_BUTTON_BIT } else { buttons::LYING_FLAT }
    };
    cmd.buttons = (cmd.buttons & !(buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT)) | result;
}

/// Frozen, stunned, linked and jump-disabled players: movement and the buttons that need it.
fn restrict_movement(controls: &ScriptControls, cmd: &mut InputCommand) {
    if controls.stuck {
        cmd.thrust_ahead = 0;
        cmd.thrust_lateral = 0;
        cmd.buttons &= !(buttons::LEAP
            | buttons::SPRINT
            | buttons::ATTACK
            | buttons::MELEE_CHARGE
            | buttons::SAMPLE_FRAG_ASSET
            | buttons::OBSCURANT
            | buttons::RELOAD
            | buttons::INTERACT_RELOAD_KEY);
    }
    if controls.stunned {
        cmd.thrust_ahead /= 2;
        cmd.thrust_lateral /= 2;
        cmd.buttons &= !(buttons::LEAP | buttons::SPRINT);
    }
    if controls.paired {
        cmd.thrust_ahead = 0;
        cmd.thrust_lateral = 0;
        cmd.buttons &= !(buttons::LEAP | buttons::SPRINT);
    }
    if controls.jump_disabled {
        cmd.buttons &= !buttons::LEAP;
    }
}

/// Disabled weapons, firing, offhands and use.
fn restrict_actions(controls: &ScriptControls, cmd: &mut InputCommand) {
    if controls.weapons_disabled {
        cmd.buttons &= !(buttons::ATTACK | buttons::THROW | buttons::ADS | buttons::MELEE_CHARGE);
    }
    if controls.fire_disabled {
        cmd.buttons &= !(buttons::ATTACK | buttons::THROW | buttons::MELEE_CHARGE);
    }
    if controls.offhands_disabled || controls.weapons_disabled || controls.fire_disabled {
        cmd.buttons &= !(buttons::SAMPLE_FRAG_ASSET | buttons::OBSCURANT);
    }
    if controls.usability_disabled {
        cmd.buttons &= !(buttons::INTERACT_KEY | buttons::INTERACT_RELOAD_KEY);
    }
}

/// Whether the player is steering a scorestreak (the Hellstorm's missile, a drone or vehicle, the Lodestar). His keys
/// switch no weapon then: the engine holds the weapon, though the streak's own script does not (the Hellstorm clears
/// `usingremote` and turns weapon cycling back on while he still rides the missile, so a streak raised then would
/// start beside it).
fn is_steering(world: &StepWorldView, seat: SeatNo) -> bool {
    let missile_in_flight = world
        .client_meta(seat)
        .and_then(|meta| meta.remote_missile)
        .is_some_and(|link| link.unlink_at_ms.is_none());
    missile_in_flight || crate::zombie_rt::remote_operated(world.ecs_read(), seat.0)
}

/// Applies every script-set restriction to a command before the step uses it. `zombies` selects Zombies' stance rule.
pub(crate) fn constrain_cmd(world: &mut StepWorldView, id: SeatNo, cmd: &mut InputCommand, zombies: bool) {
    let held = world.avatar(id).map_or(0, |ps| ps.weapon);
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
    let nothing_to_use = world.avatar(id).is_some_and(|ps| ps.hint_kind == 0);
    if cmd.buttons & buttons::INTERACT_RELOAD_KEY != 0 && nothing_to_use {
        cmd.buttons |= buttons::RELOAD;
    }
    if world.client_meta(id).is_some_and(|meta| meta.item_use_reload) {
        world.client_meta_mut(id).item_use_reload = false;
        cmd.buttons |= buttons::RELOAD;
    }
    restrict_movement(&controls, cmd);
    let movement_state_bits = world.avatar(id).map_or(0, |ps| ps.movement_state_bits);
    if zombies {
        bo2_zombies_stance(&controls, movement_state_bits, cmd);
    }
    apply_allow_controls(&controls, movement_state_bits, cmd);
    restrict_actions(&controls, cmd);
    // A guest weapon (Bow, Stick, Gravity Gun, Portal Gun) runs its own right button in `t6::mp_specials` (the Portal
    // Gun's orange); the Ray Gun under it never aims down its sights.
    if crate::zombie_rt::mp_specials::owns_trigger(world.weapon_script_label(held)) {
        cmd.buttons &= !buttons::ADS;
    }
    let steering = is_steering(world, id);
    if controls.switch_to != 0 {
        cmd.weapon = controls.switch_to as u16;
    } else if controls.switch_disabled || controls.stuck || steering {
        cmd.weapon = held as u16;
        if steering {
            cmd.buttons &= !buttons::INVENTORY;
        }
    }
}

#[cfg(test)]
mod allow_tests {
    use super::*;

    fn cmd_with(held: u32) -> InputCommand {
        let mut cmd = InputCommand::default();
        cmd.buttons = held;
        cmd
    }

    #[test]
    fn nothing_is_dropped_by_default() {
        let all = buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT | buttons::SPRINT | buttons::MELEE_CHARGE | buttons::ADS;
        let mut cmd = cmd_with(all);
        apply_allow_controls(&ScriptControls::default(), 0, &mut cmd);
        assert_eq!(cmd.buttons, all);
    }

    #[test]
    fn each_switch_drops_its_own_request_only() {
        let all = buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT | buttons::SPRINT | buttons::MELEE_CHARGE | buttons::ADS;
        let cases = [
            (ScriptControls { prone_disabled: true, ..Default::default() }, buttons::LYING_FLAT),
            (ScriptControls { crouch_disabled: true, ..Default::default() }, buttons::DUCK_BUTTON_BIT),
            (ScriptControls { sprint_disabled: true, ..Default::default() }, buttons::SPRINT),
            (ScriptControls { melee_disabled: true, ..Default::default() }, buttons::MELEE_CHARGE),
            (ScriptControls { ads_disabled: true, ..Default::default() }, buttons::ADS),
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
        assert_eq!(cmd.buttons, buttons::DUCK_BUTTON_BIT);
        // Prone and asking for nothing: stays prone.
        let mut cmd = cmd_with(0);
        apply_allow_controls(&no_stand, movement_state_bits::LYING_FLAT, &mut cmd);
        assert_eq!(cmd.buttons, buttons::LYING_FLAT);
        // No crouch either: prone.
        let neither = ScriptControls { stand_disabled: true, crouch_disabled: true, ..Default::default() };
        let mut cmd = cmd_with(0);
        apply_allow_controls(&neither, 0, &mut cmd);
        assert_eq!(cmd.buttons, buttons::LYING_FLAT);
        // Already asking for a stance: left alone.
        let mut cmd = cmd_with(buttons::DUCK_BUTTON_BIT);
        apply_allow_controls(&no_stand, 0, &mut cmd);
        assert_eq!(cmd.buttons, buttons::DUCK_BUTTON_BIT);
    }

    #[test]
    fn frozen_players_cannot_move_or_act() {
        let frozen = ScriptControls { stuck: true, ..Default::default() };
        let mut cmd = cmd_with(buttons::LEAP | buttons::ATTACK | buttons::ADS);
        cmd.thrust_ahead = 90;
        cmd.thrust_lateral = -90;
        restrict_movement(&frozen, &mut cmd);
        assert_eq!((cmd.thrust_ahead, cmd.thrust_lateral), (0, 0));
        assert_eq!(cmd.buttons, buttons::ADS);
    }

    #[test]
    fn stunned_players_move_at_half_speed_and_cannot_jump() {
        let stunned = ScriptControls { stunned: true, ..Default::default() };
        let mut cmd = cmd_with(buttons::LEAP | buttons::SPRINT | buttons::ATTACK);
        cmd.thrust_ahead = 100;
        cmd.thrust_lateral = -50;
        restrict_movement(&stunned, &mut cmd);
        assert_eq!((cmd.thrust_ahead, cmd.thrust_lateral), (50, -25));
        assert_eq!(cmd.buttons, buttons::ATTACK);
    }

    #[test]
    fn disabled_fire_leaves_aiming_and_disables_offhands() {
        let no_fire = ScriptControls { fire_disabled: true, ..Default::default() };
        let mut cmd = cmd_with(buttons::ATTACK | buttons::THROW | buttons::ADS | buttons::SAMPLE_FRAG_ASSET | buttons::OBSCURANT | buttons::INTERACT_KEY);
        restrict_actions(&no_fire, &mut cmd);
        assert_eq!(cmd.buttons, buttons::ADS | buttons::INTERACT_KEY);
    }
}

#[cfg(test)]
mod zombies_stance_tests {
    use super::*;

    const UPRIGHT: u32 = 0;
    const DUCK_BUTTON_BIT: u32 = buttons::DUCK_BUTTON_BIT;
    const LYING_FLAT: u32 = buttons::LYING_FLAT;

    /// The stance a command asking for `asks`, from the stance `now`, ends up with when the scripts switch off
    /// (stand, crouch, prone): Zombies' rule first, then the plain drop, as `constrain_cmd` runs them.
    fn stance(off: (bool, bool, bool), now: u32, asks: u32) -> u32 {
        let controls = ScriptControls {
            stand_disabled: off.0,
            crouch_disabled: off.1,
            prone_disabled: off.2,
            ..Default::default()
        };
        let mut cmd = InputCommand::default();
        cmd.buttons = asks | buttons::SPRINT;
        bo2_zombies_stance(&controls, now, &mut cmd);
        apply_allow_controls(&controls, now, &mut cmd);
        assert_ne!(cmd.buttons & buttons::SPRINT, 0, "only the stance changes");
        cmd.buttons & (buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT)
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
        for (zombies, want) in [(true, LYING_FLAT), (false, UPRIGHT)] {
            let mut sim = crate::sim_whole::WholeSim::default();
            let mut world = sim.frame();
            let id = SeatNo(0);
            world.client_meta_mut(id).controls.crouch_disabled = true;
            let mut cmd = InputCommand::default();
            cmd.buttons = DUCK_BUTTON_BIT;
            constrain_cmd(&mut world, id, &mut cmd, zombies);
            assert_eq!(cmd.buttons & (LYING_FLAT | DUCK_BUTTON_BIT), want, "zombies {zombies}");
        }
    }

    #[test]
    fn everything_allowed_changes_nothing() {
        for now in [0, movement_state_bits::DUCK_BUTTON_BIT, movement_state_bits::LYING_FLAT] {
            for asks in [UPRIGHT, DUCK_BUTTON_BIT, LYING_FLAT] {
                assert_eq!(stance((false, false, false), now, asks), asks);
            }
        }
    }

    #[test]
    fn without_prone_he_stays_standing_or_crouched() {
        // The TranZit bus: a prone press (or a dive) while standing stays standing.
        assert_eq!(stance(NO_PRONE, 0, LYING_FLAT), UPRIGHT);
        // Crouched, a prone press stays crouched (the plain drop would stand him up).
        assert_eq!(stance(NO_PRONE, movement_state_bits::DUCK_BUTTON_BIT, LYING_FLAT), DUCK_BUTTON_BIT);
        // Prone when prone was turned off: the prone button now crouches him.
        assert_eq!(stance(NO_PRONE, movement_state_bits::LYING_FLAT, LYING_FLAT), DUCK_BUTTON_BIT);
        assert_eq!(stance(ONLY_STAND, movement_state_bits::DUCK_BUTTON_BIT, LYING_FLAT), UPRIGHT);
        assert_eq!(stance(NO_PRONE, 0, DUCK_BUTTON_BIT), DUCK_BUTTON_BIT);
    }

    #[test]
    fn without_crouch_he_goes_past_the_crouch() {
        // Standing, a crouch press goes on down to prone; down already, it stands.
        assert_eq!(stance(NO_CROUCH, 0, DUCK_BUTTON_BIT), LYING_FLAT);
        assert_eq!(stance(NO_CROUCH, movement_state_bits::LYING_FLAT, DUCK_BUTTON_BIT), UPRIGHT);
        assert_eq!(stance(ONLY_PRONE, 0, DUCK_BUTTON_BIT), LYING_FLAT);
        assert_eq!(stance(ONLY_STAND, 0, DUCK_BUTTON_BIT), UPRIGHT);
    }

    #[test]
    fn without_stand_he_stays_down() {
        assert_eq!(stance(NO_STAND, 0, UPRIGHT), DUCK_BUTTON_BIT);
        assert_eq!(stance(NO_STAND, movement_state_bits::LYING_FLAT, UPRIGHT), LYING_FLAT);
        assert_eq!(stance(ONLY_CROUCH, movement_state_bits::LYING_FLAT, UPRIGHT), DUCK_BUTTON_BIT);
        assert_eq!(stance(ONLY_PRONE, 0, UPRIGHT), LYING_FLAT);
        assert_eq!(stance(NO_STAND, 0, DUCK_BUTTON_BIT), DUCK_BUTTON_BIT);
    }
}
