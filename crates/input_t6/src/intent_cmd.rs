//! A frame's buttons and move from the held keys, and the user command they go out in.

use playerstate_t6::{ButtonBits, InputCommand, button};

use crate::client::{DUCK_BUTTON_BIT, LYING_FLAT, stance_hold, weapnext_hold};
use crate::{PlayerIntentFeed, HeldKeyLatch, HeldKeyLatchBank};

/// The key move this frame, each axis -1 to 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct StrideBasis {
    pub forward: f32,
    pub right: f32,
}

/// The buttons that go out exactly as their key is held (or was tapped since the last command).
fn plain_bits(pressed_key_latch: &HeldKeyLatchBank) -> ButtonBits {
    let pairs: [(&HeldKeyLatch, u32); 11] = [
        (&pressed_key_latch.attack, button::FIRE_BIT),
        (&pressed_key_latch.holdbreath, button::STEADY_AIM_HOLD_BIT),
        (&pressed_key_latch.frag, button::SAMPLE_FRAG_ASSET),
        (&pressed_key_latch.smoke, button::SECONDARY_OFFHAND),
        (&pressed_key_latch.melee, button::CLOSE_COMBAT),
        (&pressed_key_latch.activate, button::INTERACT_KEY),
        (&pressed_key_latch.reload, button::RELOAD),
        (&pressed_key_latch.usereload, button::INTERACT_RELOAD_KEY),
        (&pressed_key_latch.gostand, button::LEAP),
        (&pressed_key_latch.throw_btn, button::THROW),
        (&pressed_key_latch.jam, button::JAM),
    ];
    let mut bits = ButtonBits::default();
    for (held, bit) in pairs {
        if held.down_or_tapped() {
            bits.set(bit);
        }
    }
    bits
}

/// The share of the frame `held` was down; a button still down counts the whole frame.
fn axis_share(held: &mut HeldKeyLatch, now: i32, tick_span_ms: u32) -> f32 {
    let share = held.fraction(now, tick_span_ms);
    if held.active { 1.0 } else { share }
}

/// This frame's buttons and key move. The stance and switch-weapon holds run first, so a
/// hold that has just come due counts in this command.
pub fn read_move_buttons(
    client: &mut PlayerIntentFeed,
    present_tick_ms: i32,
    tick_span_ms: u32,
) -> (ButtonBits, StrideBasis) {
    stance_hold(client, present_tick_ms);
    weapnext_hold(client, present_tick_ms);

    let mut bit_set = plain_bits(&client.pressed_key_latch);
    let pressed_key_latch = &client.pressed_key_latch;
    let held_stance = if pressed_key_latch.prone.active {
        Some(LYING_FLAT)
    } else if pressed_key_latch.movedown.active {
        Some(DUCK_BUTTON_BIT)
    } else {
        None
    };
    // Aiming is the toggle, flipped while the speed key is held.
    if pressed_key_latch.speed.active != client.scoped_in {
        bit_set.set(button::AIM_BIT);
    }
    if pressed_key_latch.sprint.down_or_tapped() {
        bit_set.set(button::SPRINT);
    }
    if pressed_key_latch.gostand.down_or_tapped() {
        client.posture_hold = 0;
    }
    match held_stance {
        Some(stance) => {
            bit_set.set(stance as u32);
            bit_set.set(button::STANCE_LOCK_BIT);
        }
        None if client.posture_hold != 0 => bit_set.set(client.posture_hold as u32),
        None => {}
    }
    if client.holds.use_tap.take_double() {
        bit_set.set(button::DOUBLE_TAP);
    }

    let pressed_key_latch = &mut client.pressed_key_latch;
    let forward = axis_share(&mut pressed_key_latch.forward, present_tick_ms, tick_span_ms)
        - axis_share(&mut pressed_key_latch.back, present_tick_ms, tick_span_ms);
    let right = axis_share(&mut pressed_key_latch.moveright, present_tick_ms, tick_span_ms)
        - axis_share(&mut pressed_key_latch.moveleft, present_tick_ms, tick_span_ms);
    let axes = StrideBasis {
        forward: forward.clamp(-1.0, 1.0),
        right: right.clamp(-1.0, 1.0),
    };
    (bit_set, axes)
}

/// A key axis as a move byte, rounded half away from zero.
pub fn stick_to_thrust(frame: f32) -> i8 {
    let v = frame * 127.0;
    let rounded = if v < 0.0 { v - 0.5 } else { v + 0.5 };
    (rounded as i32).clamp(-127, 127) as i8
}

/// The pad stick's move, stretched from the circle to the square: a full diagonal reaches the
/// corner instead of 0.7 of each axis.
pub fn pad_move_stretched(forward: f32, right: f32) -> (i32, i32) {
    let (long, short) = if right.abs() <= forward.abs() {
        (forward, right)
    } else {
        (right, forward)
    };
    let stretch = if long == 0.0 {
        1.0
    } else {
        let ratio = short / long;
        libm::sqrtf(ratio * ratio + 1.0)
    };
    let scale = stretch * 127.0;
    ((scale * forward) as i32, (scale * right) as i32)
}

pub fn clamp_char(v: i32) -> i8 {
    v.clamp(-128, 127) as i8
}

/// The most a command's pitch may turn in one step, a quarter turn.
pub const PITCH_STEP_MAX: i32 = 16384;

/// The pitch turn to add this command. A turn that reads as more than a quarter turn when
/// seen as a short angle is cut to a quarter turn its way.
fn pitch_step(delta: i32) -> i32 {
    let seen = match delta as i16 {
        i16::MIN => 32768,
        s => i32::from(s),
    };
    if seen.abs() > PITCH_STEP_MAX {
        PITCH_STEP_MAX * seen.signum()
    } else {
        delta
    }
}

/// What one command is built from.
#[derive(Clone, Debug, Default)]
pub struct IntentBuildRequest {
    pub authority_clock_ms: i32,
    /// The view angles so far, in short-angle units.
    pub angles: [i32; 3],
    pub buttons: ButtonBits,
    pub thrust_ahead: i8,
    pub thrust_lateral: i8,
    pub pointer_tilt_change: i32,
    pub pointer_turn_change: i32,
    pub keyed_tilt_change: i32,
    pub keyed_turn_change: i32,
    /// The player cannot act: only the time and the view go out.
    pub frozen: bool,
}

/// The user command for this frame: the view turned by this frame's keys and mouse, then the
/// buttons and move.
pub fn build_input_command(req: &IntentBuildRequest) -> InputCommand {
    let mut rot = req.angles;
    if req.frozen {
        return InputCommand {
            authority_clock_ms: req.authority_clock_ms,
            angles: rot,
            ..InputCommand::default()
        };
    }
    let pitch = req.keyed_tilt_change.wrapping_add(req.pointer_tilt_change);
    rot[0] = rot[0].wrapping_add(pitch_step(pitch));
    rot[1] = rot[1]
        .wrapping_add(req.keyed_turn_change)
        .wrapping_add(req.pointer_turn_change);
    InputCommand {
        authority_clock_ms: req.authority_clock_ms,
        angles: rot,
        button_bits: req.buttons,
        thrust_ahead: req.thrust_ahead,
        thrust_lateral: req.thrust_lateral,
        ..InputCommand::default()
    }
}
