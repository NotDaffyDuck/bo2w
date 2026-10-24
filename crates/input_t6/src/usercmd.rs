//! A frame's buttons and move from the held keys, and the user command they go out in.

use playerstate_t6::{ButtonBits, UserCmd, button};

use crate::client::{CROUCH, PRONE, stance_hold, weapnext_hold};
use crate::{ClientInput, Kbutton, KbuttonSet};

/// The key move this frame, each axis -1 to 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveAxes {
    pub forward: f32,
    pub right: f32,
}

/// The buttons that go out exactly as their key is held (or was tapped since the last command).
fn plain_bits(kb: &KbuttonSet) -> ButtonBits {
    let pairs: [(&Kbutton, u32); 11] = [
        (&kb.attack, button::ATTACK),
        (&kb.holdbreath, button::BREATH),
        (&kb.frag, button::FRAG),
        (&kb.smoke, button::SECONDARY_OFFHAND),
        (&kb.melee, button::MELEE),
        (&kb.activate, button::USE),
        (&kb.reload, button::RELOAD),
        (&kb.usereload, button::USE_RELOAD),
        (&kb.gostand, button::JUMP),
        (&kb.throw_btn, button::THROW),
        (&kb.jam, button::JAM),
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
fn axis_share(held: &mut Kbutton, now: i32, frame_msec: u32) -> f32 {
    let share = held.fraction(now, frame_msec);
    if held.active { 1.0 } else { share }
}

/// This frame's buttons and key move. The stance and switch-weapon holds run first, so a
/// hold that has just come due counts in this command.
pub fn sample_move(
    client: &mut ClientInput,
    now_msec: i32,
    frame_msec: u32,
) -> (ButtonBits, MoveAxes) {
    stance_hold(client, now_msec);
    weapnext_hold(client, now_msec);

    let mut bits = plain_bits(&client.kb);
    let kb = &client.kb;
    let held_stance = if kb.prone.active {
        Some(PRONE)
    } else if kb.movedown.active {
        Some(CROUCH)
    } else {
        None
    };
    // Aiming is the toggle, flipped while the speed key is held.
    if kb.speed.active != client.using_ads {
        bits.set(button::ADS);
    }
    if kb.sprint.down_or_tapped() {
        bits.set(button::SPRINT);
    }
    if kb.gostand.down_or_tapped() {
        client.stance_latch = 0;
    }
    match held_stance {
        Some(stance) => {
            bits.set(stance as u32);
            bits.set(button::STANCE_HELD);
        }
        None if client.stance_latch != 0 => bits.set(client.stance_latch as u32),
        None => {}
    }
    if client.holds.use_tap.take_double() {
        bits.set(button::DOUBLE_TAP);
    }

    let kb = &mut client.kb;
    let forward = axis_share(&mut kb.forward, now_msec, frame_msec)
        - axis_share(&mut kb.back, now_msec, frame_msec);
    let right = axis_share(&mut kb.moveright, now_msec, frame_msec)
        - axis_share(&mut kb.moveleft, now_msec, frame_msec);
    let axes = MoveAxes {
        forward: forward.clamp(-1.0, 1.0),
        right: right.clamp(-1.0, 1.0),
    };
    (bits, axes)
}

/// A key axis as a move byte, rounded half away from zero.
pub fn axis_to_move(axis: f32) -> i8 {
    let v = axis * 127.0;
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
pub struct CreateCmdInput {
    pub server_time: i32,
    /// The view angles so far, in short-angle units.
    pub angles: [i32; 3],
    pub buttons: ButtonBits,
    pub forwardmove: i8,
    pub rightmove: i8,
    pub mouse_pitch_delta: i32,
    pub mouse_yaw_delta: i32,
    pub key_pitch_delta: i32,
    pub key_yaw_delta: i32,
    /// The player cannot act: only the time and the view go out.
    pub frozen: bool,
}

/// The user command for this frame: the view turned by this frame's keys and mouse, then the
/// buttons and move.
pub fn create_cmd(input: &CreateCmdInput) -> UserCmd {
    let mut angles = input.angles;
    if input.frozen {
        return UserCmd {
            server_time: input.server_time,
            angles,
            ..UserCmd::default()
        };
    }
    let pitch = input.key_pitch_delta.wrapping_add(input.mouse_pitch_delta);
    angles[0] = angles[0].wrapping_add(pitch_step(pitch));
    angles[1] = angles[1]
        .wrapping_add(input.key_yaw_delta)
        .wrapping_add(input.mouse_yaw_delta);
    UserCmd {
        server_time: input.server_time,
        angles,
        button_bits: input.buttons,
        forwardmove: input.forwardmove,
        rightmove: input.rightmove,
        ..UserCmd::default()
    }
}
