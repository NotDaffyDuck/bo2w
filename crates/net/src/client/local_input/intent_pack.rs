//! Building the command the host runs from this frame's input.

use input_t6::{
    DEGREES_TO_WORD16, IntentBuildRequest, scale_pointer_motion, stick_to_thrust, clamp_char, build_input_command,
    pointer_turn_units, pad_move_stretched, read_move_buttons,
};
use playerstate_t6::button;
use playerstate_t6::game::{InputCommand, buttons};

use super::{PlayerControlSnapshot, ViewAimAccumulator};

/// Command bit for BO2's offhand-switch button (weapon_t6 `cmd::button::OFFHAND_SWITCH`).
const OFFHAND_SWITCH_BIT: u32 = 0x20000;

/// Black Ops II's buttons and the command bit each one is in the sim.
const SIM_BUTTON_BITS: [(u32, u32); 17] = [
    (button::FIRE_BIT, buttons::ATTACK),
    (button::SPRINT, buttons::SPRINT),
    (button::CLOSE_COMBAT, buttons::MELEE_CHARGE),
    (button::INTERACT_KEY, buttons::INTERACT_KEY),
    (button::RELOAD, buttons::RELOAD),
    (button::INTERACT_RELOAD_KEY, buttons::INTERACT_RELOAD_KEY),
    (button::LYING_FLAT, buttons::LYING_FLAT),
    (button::DUCK_BUTTON_BIT, buttons::DUCK_BUTTON_BIT),
    (button::LEAP, buttons::LEAP),
    (button::AIM_BIT, buttons::ADS),
    (button::STANCE_LOCK_BIT, buttons::STANCE_LOCK_BIT),
    (button::STEADY_AIM_HOLD_BIT, buttons::STEADY_AIM_HOLD_BIT),
    (button::SAMPLE_FRAG_ASSET, buttons::SAMPLE_FRAG_ASSET),
    (button::SECONDARY_OFFHAND, buttons::OBSCURANT),
    (button::THROW, buttons::THROW),
    (button::JAM, buttons::JAM),
    (button::DOUBLE_TAP, buttons::DOUBLE_TAP),
];

/// The command for this frame: keys, mouse and pad folded into one move and one set of
/// buttons, stamped with `authority_clock_ms`. One-shot offhand requests are taken from `input`.
pub fn assemble_input_record(mouse_cfg: &mut PlayerControlSnapshot, aim_state: &ViewAimAccumulator, authority_clock_ms: i32) -> InputCommand {
    let now = mouse_cfg.present_tick_ms.max(1);
    let frame = mouse_cfg.tick_span_ms.max(1);
    let (button_bits, held_axes) = read_move_buttons(&mut mouse_cfg.client, now, frame);

    let mut extra = 0;
    if std::mem::take(&mut mouse_cfg.client.throw_grip_abort) {
        extra |= buttons::OFFHAND_HOLD_CANCEL;
    }
    if std::mem::take(&mut mouse_cfg.client.offhand_switch_t6) {
        extra |= OFFHAND_SWITCH_BIT;
    }

    let (pointer_delta_horizontal, pointer_delta_vertical) = scale_pointer_motion(
        mouse_cfg.pointer_delta_horizontal,
        mouse_cfg.pointer_delta_vertical,
        mouse_distance(mouse_cfg.pointer_delta_horizontal, mouse_cfg.pointer_delta_vertical),
        frame as f32,
        mouse_cfg.sensitivity,
        mouse_cfg.pointer_acceleration,
        mouse_cfg.view_angle_multiplier,
    );
    let (nudge_pitch, nudge_yaw) = pointer_turn_units(pointer_delta_horizontal, pointer_delta_vertical, mouse_cfg.yaw_sensitivity, mouse_cfg.pitch_per_count);

    // The pad's look only counts while the mouse is still.
    let mouse_moving = mouse_cfg.pointer_delta_horizontal != 0.0 || mouse_cfg.pointer_delta_vertical != 0.0;
    let pad_look = |degrees: f32| if mouse_moving { 0 } else { (degrees * DEGREES_TO_WORD16) as i32 };
    let pad_pitch = pad_look(mouse_cfg.pad_look_delta[0]);
    let pad_yaw = pad_look(mouse_cfg.pad_look_delta[1]);

    // The key move plus the pad move stretched to the square, held to a move byte.
    let (pad_forward, pad_right) = pad_move_stretched(mouse_cfg.pad_move[0], mouse_cfg.pad_move[1]);
    let thrust_ahead = clamp_char(i32::from(stick_to_thrust(held_axes.forward)) + pad_forward);
    let thrust_lateral = clamp_char(i32::from(stick_to_thrust(held_axes.right)) + pad_right);

    let mut cmd = sim_cmd(&build_input_command(&IntentBuildRequest {
        authority_clock_ms,
        angles: aim_state.angles,
        buttons: button_bits,
        thrust_ahead,
        thrust_lateral,
        pointer_tilt_change: nudge_pitch + pad_pitch,
        pointer_turn_change: nudge_yaw + pad_yaw,
        keyed_tilt_change: 0,
        keyed_turn_change: 0,
        frozen: false,
    }));
    cmd.buttons |= extra;
    cmd
}

/// A command that does nothing, for a tick with no input.
pub fn resting_command(authority_clock_ms: i32) -> InputCommand {
    InputCommand { authority_clock_ms, ..InputCommand::default() }
}

/// The sim steps its own command type, so the client's BO2 command goes in as one.
fn sim_cmd(cmd: &playerstate_t6::InputCommand) -> InputCommand {
    let sim_bits = SIM_BUTTON_BITS
        .iter()
        .filter(|(held, _)| cmd.held(*held))
        .fold(0, |acc, (_, sim_bit)| acc | sim_bit);
    InputCommand {
        authority_clock_ms: cmd.authority_clock_ms,
        buttons: sim_bits,
        angles: cmd.angles,
        thrust_ahead: cmd.thrust_ahead,
        thrust_lateral: cmd.thrust_lateral,
        ..InputCommand::default()
    }
}

/// The distance the mouse moved this frame, which is what cl_mouseAccel scales by: the square
/// root of step_along_x² + dy², not the sum of the two.
pub(super) fn mouse_distance(step_along_x: f32, dy: f32) -> f32 {
    (step_along_x * step_along_x + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_bo2_button_lands_on_its_own_sim_bit() {
        for (bo2_button, sim_bit) in SIM_BUTTON_BITS {
            let mut cmd = playerstate_t6::InputCommand::default();
            cmd.button_bits.set(bo2_button);
            assert_eq!(sim_cmd(&cmd).buttons, sim_bit, "button {bo2_button}");
        }
    }

    #[test]
    fn held_buttons_combine_and_nothing_held_is_empty() {
        let mut cmd = playerstate_t6::InputCommand::default();
        assert_eq!(sim_cmd(&cmd).buttons, 0);
        cmd.button_bits.set(button::LYING_FLAT);
        cmd.button_bits.set(button::STANCE_LOCK_BIT);
        assert_eq!(sim_cmd(&cmd).buttons, buttons::LYING_FLAT | buttons::STANCE_LOCK_BIT);
    }

    #[test]
    fn the_sim_bits_are_all_different() {
        let mut seen = 0u32;
        for (_, bit) in SIM_BUTTON_BITS {
            assert_eq!(bit.count_ones(), 1);
            assert_eq!(seen & bit, 0, "bit {bit:#x} used twice");
            seen |= bit;
        }
    }

    #[test]
    fn the_mouse_distance_is_the_hypotenuse() {
        assert_eq!(mouse_distance(3.0, 4.0), 5.0);
        assert_eq!(mouse_distance(-3.0, 4.0), 5.0);
        assert_eq!(mouse_distance(0.0, 0.0), 0.0);
    }

    #[test]
    fn an_idle_command_carries_only_the_time() {
        let cmd = resting_command(777);
        assert_eq!(cmd.authority_clock_ms, 777);
        assert_eq!(cmd.buttons, 0);
        assert_eq!((cmd.thrust_ahead, cmd.thrust_lateral), (0, 0));
    }

    #[test]
    fn a_built_command_is_stamped_and_takes_the_one_shot_offhand_requests() {
        let mut input = PlayerControlSnapshot::default();
        input.client.throw_grip_abort = true;
        input.client.offhand_switch_t6 = true;
        let cmd = assemble_input_record(&mut input, &ViewAimAccumulator::default(), 4321);
        assert_eq!(cmd.authority_clock_ms, 4321);
        assert_ne!(cmd.buttons & buttons::OFFHAND_HOLD_CANCEL, 0);
        assert_ne!(cmd.buttons & OFFHAND_SWITCH_BIT, 0);
        assert!(!input.client.throw_grip_abort);
        assert!(!input.client.offhand_switch_t6);

        let again = assemble_input_record(&mut input, &ViewAimAccumulator::default(), 4371);
        assert_eq!(again.buttons & (buttons::OFFHAND_HOLD_CANCEL | OFFHAND_SWITCH_BIT), 0);
    }

    #[test]
    fn the_command_starts_from_the_look_angles() {
        let mut input = PlayerControlSnapshot::default();
        let look = ViewAimAccumulator { angles: [1000, 2000, 0], ..ViewAimAccumulator::default() };
        let cmd = assemble_input_record(&mut input, &look, 50);
        assert_eq!(cmd.angles, [1000, 2000, 0]);
        assert_eq!((cmd.thrust_ahead, cmd.thrust_lateral), (0, 0));
    }
}
