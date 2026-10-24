//! BO2's body turn: `BG_SwingAngles` and `BG_PlayerAngles`, how a
//! player's legs, torso and torso pitch swing toward where they aim.
//!
//! The client keeps the swing state in `clientInfo_t` (two `lerpFrame_t`s
//! plus a few fields). [`ClientAngles`] is that part, by BO2's names.

pub use crate::angles::{angle_normalize_360, angle_subtract};
use crate::entity_state::EntityState;
use playerstate_t6::flags::e_flags;

/// `entityState_s::lerp.eFlags` bit 1: flipped on a teleport, so the
/// client can tell a new teleport from an old one.
pub const TELEPORT_BIT: i32 = 0x2;

/// `XAnimParts` animation flags that lock the legs' yaw to the body
/// (`animation_s::flags & 0x30`).
pub const ANIM_FLAGS_LEGS_NO_TURN: u32 = 0x30;

/// `entityState_s::un2.legsAnim` bit that only toggles to restart an
/// animation; masked off before the animation lookup.
pub const ANIM_TOGGLEBIT: i32 = 0x400;

/// The yaw and pitch half of BO2's `lerpFrame_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LerpAngles {
    pub yaw_angle: f32,
    pub yawing: bool,
    pub pitch_angle: f32,
    pub pitching: bool,
}

/// The parts of BO2's `clientInfo_t` that `BG_PlayerAngles` reads and
/// writes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientAngles {
    pub legs: LerpAngles,
    pub torso: LerpAngles,
    pub lerp_move_dir: f32,
    /// `playerAngles`: pitch, yaw, roll.
    pub player_angles: [f32; 3],
    /// `needsRevive`: in last stand, the torso clamps to 45 degrees.
    pub needs_revive: bool,
    /// `clientConditions[ANIM_COND_FIRING]` is set.
    pub firing: bool,
    pub teleport_time: i32,
    pub teleport_bit: bool,
}

/// The dvars `BG_PlayerAngles` reads, with BO2's registered defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerAngleDvars {
    pub swing_speed: f32,
    pub prone_swing_speed: f32,
    pub leg_yaw_tolerance: f32,
    pub leg_yaw_crouch_tolerance: f32,
    pub leg_yaw_prone_tolerance: f32,
    pub move_factor_on_torso: f32,
    pub teleport_align_time: i32,
    /// `IKImport_GetVar_IK_Enable()`.
    pub ik_enable: bool,
}

impl Default for PlayerAngleDvars {
    fn default() -> Self {
        Self {
            swing_speed: 0.15,
            prone_swing_speed: 0.08,
            leg_yaw_tolerance: 40.0,
            leg_yaw_crouch_tolerance: 10.0,
            leg_yaw_prone_tolerance: 2.0,
            move_factor_on_torso: 0.0,
            teleport_align_time: 500,
            ik_enable: false,
        }
    }
}

/// `IK_MAX_PITCH_FRACTION`, as BO2's binary holds it.
pub const IK_MAX_PITCH_FRACTION: f32 = 1.0;
/// The pitch multiplier without IK.
pub const PITCH_FRACTION_NO_IK: f32 = 2.0;
/// The torso pitch swing: speed and clamp.
pub const PITCH_SWING_SPEED: f32 = 0.15;
pub const PITCH_CLAMP: f32 = 45.0;
/// The legs' yaw clamp.
pub const LEGS_CLAMP: f32 = 150.0;

/// `BG_SwingAngles`: turn `angle` toward `dest`.
///
/// Once the gap passes `swing_tolerance` it swings until it lands, at
/// `|gap| * 0.05` (at least 0.5) degrees per millisecond times `speed`,
/// then never trails `dest` by more than `clamp_tolerance`.
pub fn swing_angles(
    dest: f32,
    swing_tolerance: f32,
    clamp_tolerance: f32,
    speed: f32,
    frametime_ms: i32,
    angle: &mut f32,
    swinging: &mut bool,
) {
    if !*swinging {
        let d = angle_subtract(*angle, dest);
        if d <= swing_tolerance && -swing_tolerance <= d {
            return;
        }
        *swinging = true;
    }
    let swing = angle_subtract(dest, *angle);
    let mut scale = libm::fabsf(swing) * 0.05;
    if scale < 0.5 {
        scale = 0.5;
    }
    let mut step;
    let more;
    if swing < 0.0 {
        step = -(frametime_ms as f32 * scale * speed);
        more = swing < step;
    } else {
        step = frametime_ms as f32 * scale * speed;
        more = step < swing;
    }
    if more {
        *swinging = true;
    } else {
        *swinging = false;
        step = swing;
    }
    *angle = angle_normalize_360(*angle + step);

    let after = angle_subtract(dest, *angle);
    if after > clamp_tolerance {
        *angle = angle_normalize_360(dest - clamp_tolerance);
    } else if after < -clamp_tolerance {
        *angle = angle_normalize_360(dest + clamp_tolerance);
    }
}

/// `bgs->time` and `bgs->frametime` for this client frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimClock {
    pub time: i32,
    pub frametime_ms: i32,
}

/// `BG_PlayerAngles`: swing `ci`'s legs yaw, torso yaw and torso pitch for
/// player entity `es`.
///
/// `legs_anim_flags` is the `flags` of the animation that
/// `es.un2.legsAnim & !ANIM_TOGGLEBIT` names (the caller looks it up in
/// its animation table). BO2 also reads the lean fraction here and drops
/// it, so this does not.
pub fn player_angles(
    es: &EntityState,
    ci: &mut ClientAngles,
    legs_anim_flags: u32,
    clock: AnimClock,
    dvars: &PlayerAngleDvars,
) {
    let flags = es.lerp.e_flags;
    let move_type = es.lerp.u.player().move_type;
    let move_dir = ci.lerp_move_dir;

    if dvars.teleport_align_time > 0 {
        let bit = flags & TELEPORT_BIT != 0;
        if bit != ci.teleport_bit {
            ci.teleport_time = clock.time;
            ci.teleport_bit = bit;
        }
    }

    let pitch = ci.player_angles[0];
    let base = angle_normalize_360(ci.player_angles[1]);
    let speed = if flags & e_flags::PRONE != 0 {
        dvars.prone_swing_speed
    } else {
        dvars.swing_speed
    };

    let settled = flags & e_flags::TURRET_MASK == 0
        && move_type != 5
        && move_type != 6
        && flags & e_flags::MANTLE == 0
        && move_type == 1
        && (dvars.teleport_align_time < 1
            || ci.teleport_time <= clock.time - dvars.teleport_align_time);
    if !settled {
        ci.legs.yawing = true;
    }
    if !settled || ci.firing {
        ci.torso.pitching = true;
        ci.torso.yawing = true;
    }

    let moving = base + move_dir;
    let mut torso_dest = base;
    let mut legs_dest = base;
    let clamp;
    if flags & e_flags::DEAD != 0 {
        clamp = 90.0;
    } else if move_type == 5 || move_type == 6 {
        clamp = 0.0;
        torso_dest = moving;
        legs_dest = moving;
    } else if flags & e_flags::MANTLE != 0 {
        clamp = 90.0;
    } else if ci.needs_revive {
        clamp = 45.0;
    } else if flags & e_flags::PRONE != 0 {
        clamp = 90.0;
        legs_dest = f32::from(es.lerp.u.player().prone_legs_angle) * 0.003_921_569 * 360.0;
    } else {
        legs_dest = moving;
        if flags & e_flags::FIRING != 0 {
            clamp = 45.0;
        } else {
            if flags & e_flags::ADS == 0 {
                torso_dest = dvars.move_factor_on_torso * move_dir + base;
            }
            clamp = 90.0;
        }
    }
    swing_angles(
        torso_dest,
        0.0,
        clamp,
        speed,
        clock.frametime_ms,
        &mut ci.torso.yaw_angle,
        &mut ci.torso.yawing,
    );

    let tolerance;
    if flags & e_flags::DEAD == 0 && legs_anim_flags & ANIM_FLAGS_LEGS_NO_TURN != 0 {
        tolerance = 0.0;
        ci.legs.yawing = false;
        legs_dest = base;
    } else if flags & e_flags::DEAD == 0 && !ci.legs.yawing {
        tolerance = if flags & e_flags::PRONE != 0 {
            dvars.leg_yaw_prone_tolerance
        } else if flags & e_flags::CROUCHING != 0 {
            dvars.leg_yaw_crouch_tolerance
        } else {
            dvars.leg_yaw_tolerance
        };
    } else {
        tolerance = 0.0;
    }
    swing_angles(
        legs_dest,
        tolerance,
        LEGS_CLAMP,
        speed,
        clock.frametime_ms,
        &mut ci.legs.yaw_angle,
        &mut ci.legs.yawing,
    );

    // On a turret both lock to the aim; spectating or in intermission they
    // lock to the aim plus the move direction.
    if flags & e_flags::TURRET_MASK != 0 {
        ci.legs.yaw_angle = base;
        ci.torso.yaw_angle = base;
    } else if move_type == 5 || move_type == 6 {
        ci.legs.yaw_angle = moving;
        ci.torso.yaw_angle = moving;
    }

    let fraction = if dvars.ik_enable {
        IK_MAX_PITCH_FRACTION
    } else {
        PITCH_FRACTION_NO_IK
    };
    // BO2 compares the whole eFlags word to the mantle bit here, not the
    // bit alone.
    let pitch_dest = if flags & e_flags::DEAD == 0
        && flags & e_flags::TURRET_MASK == 0
        && move_type != 5
        && move_type != 6
        && flags != e_flags::MANTLE
    {
        (if pitch > 180.0 { pitch - 360.0 } else { pitch }) * fraction
    } else {
        0.0
    };
    swing_angles(
        pitch_dest,
        0.0,
        PITCH_CLAMP,
        PITCH_SWING_SPEED,
        clock.frametime_ms,
        &mut ci.torso.pitch_angle,
        &mut ci.torso.pitching,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BO2 multiplies by 1/360 in f32, so 100 comes back as 100.00001.
    fn close(a: f32, b: f32) -> bool {
        libm::fabsf(a - b) < 1.0e-3
    }

    #[test]
    fn swing_waits_inside_tolerance_then_eases_in() {
        let (mut a, mut s) = (0.0, false);
        swing_angles(30.0, 40.0, 150.0, 0.15, 50, &mut a, &mut s);
        assert_eq!((a, s), (0.0, false));

        // 60 degrees out: scale 3, step 50 * 3 * 0.15 = 22.5.
        swing_angles(60.0, 40.0, 150.0, 0.15, 50, &mut a, &mut s);
        assert!((a - 22.5).abs() < 1e-4 && s);

        // Close enough to land: snaps to dest and stops.
        let (mut a, mut s) = (59.0, true);
        swing_angles(60.0, 0.0, 150.0, 0.15, 50, &mut a, &mut s);
        assert_eq!((a, s), (60.0, false));
    }

    #[test]
    fn swing_clamps_the_trailing_angle() {
        // Speed 0 never moves, so only the clamp acts: 120 behind, clamp 90.
        let (mut a, mut s) = (0.0, true);
        swing_angles(120.0, 0.0, 90.0, 0.0, 50, &mut a, &mut s);
        assert!((a - 30.0).abs() < 1e-4);
        let (mut a, mut s) = (0.0, true);
        swing_angles(-120.0, 0.0, 90.0, 0.0, 50, &mut a, &mut s);
        assert!((a - 330.0).abs() < 1e-4);
    }

    fn player(flags: i32, move_type: i32) -> EntityState {
        let mut es = EntityState::default();
        es.lerp.e_flags = flags;
        let mut p = es.lerp.u.player();
        p.move_type = move_type;
        p.prone_legs_angle = 64;
        es.lerp.u.set_player(&p);
        es
    }

    #[test]
    fn standing_legs_hold_until_forty_degrees() {
        let es = player(0, 1);
        let mut ci = ClientAngles {
            player_angles: [0.0, 30.0, 0.0],
            ..Default::default()
        };
        let clock = AnimClock { time: 10_000, frametime_ms: 50 };
        player_angles(&es, &mut ci, 0, clock, &PlayerAngleDvars::default());
        assert_eq!(ci.legs.yaw_angle, 0.0);
        assert!(!ci.legs.yawing);
        // The torso always tracks the aim.
        assert!(ci.torso.yaw_angle > 0.0);
    }

    #[test]
    fn turret_and_spectator_lock_the_body() {
        let clock = AnimClock { time: 10_000, frametime_ms: 50 };
        let mut ci = ClientAngles {
            player_angles: [0.0, 100.0, 0.0],
            lerp_move_dir: 20.0,
            ..Default::default()
        };
        player_angles(&player(e_flags::TURRET_STAND, 1), &mut ci, 0, clock, &PlayerAngleDvars::default());
        assert!(close(ci.legs.yaw_angle, 100.0) && close(ci.torso.yaw_angle, 100.0));
        assert_eq!(ci.torso.pitch_angle, 0.0);

        let mut ci = ClientAngles {
            player_angles: [0.0, 100.0, 0.0],
            lerp_move_dir: 20.0,
            ..Default::default()
        };
        player_angles(&player(0, 5), &mut ci, 0, clock, &PlayerAngleDvars::default());
        assert!(close(ci.legs.yaw_angle, 120.0) && close(ci.torso.yaw_angle, 120.0));
    }

    #[test]
    fn prone_legs_follow_the_packed_angle() {
        // Byte 64 is 64 / 255 of a turn, about 90.35 degrees.
        let clock = AnimClock { time: 10_000, frametime_ms: 1000 };
        let mut ci = ClientAngles::default();
        player_angles(&player(e_flags::PRONE, 1), &mut ci, 0, clock, &PlayerAngleDvars::default());
        let want = 64.0 * 0.003_921_569 * 360.0;
        assert!((ci.legs.yaw_angle - want).abs() < 1e-3);
    }

    #[test]
    fn teleport_flips_hold_the_force_swing() {
        let dvars = PlayerAngleDvars::default();
        // Frame time 0 so the forced swings stay open across the call.
        let mut ci = ClientAngles {
            player_angles: [0.0, 90.0, 0.0],
            ..Default::default()
        };
        player_angles(&player(TELEPORT_BIT, 1), &mut ci, 0, AnimClock { time: 1000, frametime_ms: 0 }, &dvars);
        assert_eq!((ci.teleport_time, ci.teleport_bit), (1000, true));
        assert!(ci.legs.yawing && ci.torso.yawing);

        // 500 ms later the align window has passed: nothing forces a swing.
        let mut ci2 = ClientAngles { teleport_time: 1000, teleport_bit: true, ..Default::default() };
        player_angles(&player(TELEPORT_BIT, 1), &mut ci2, 0, AnimClock { time: 1500, frametime_ms: 0 }, &dvars);
        assert!(!ci2.legs.yawing && !ci2.torso.yawing);
    }

    #[test]
    fn legs_lock_while_the_anim_says_so() {
        let mut ci = ClientAngles {
            player_angles: [0.0, 60.0, 0.0],
            legs: LerpAngles { yawing: true, ..Default::default() },
            ..Default::default()
        };
        let clock = AnimClock { time: 10_000, frametime_ms: 1000 };
        player_angles(&player(0, 1), &mut ci, ANIM_FLAGS_LEGS_NO_TURN, clock, &PlayerAngleDvars::default());
        assert_eq!(ci.legs.yaw_angle, 60.0);
        assert!(!ci.legs.yawing);
    }
}
