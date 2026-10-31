//! Black Ops II's slope pitch: how steeply the ground under the player rises
//! the way he faces (PM_CalcPlayerPitch), and the average of the last frames
//! of it that PM_Footsteps keeps. The legs' `slope` condition reads the
//! average (sim `t6_playeranim.rs` `t6_slope`): `up` over 30 degrees,
//! `down` under -30.
//!
//! BO2 keeps the history in the move's pmove_t, which ClientThink_real
//! clears for every command, and a command runs at most three moves (150 ms
//! in moves of at most 66 ms). The ten-frame average so never passes three
//! tenths of the steepest pitch (under 27 degrees), and at BO2's settings the
//! condition never holds. Ported as BO2 has it all the same.

use game_math::wrap_degrees;
use game_math::degree_gap;
use crate::bridge::TickLocals;
use playerstate_t6::game::AvatarState;

use crate::project::ground_normal;

/// bg_slopeFrames: the frames the pitch is averaged over.
const SLOPE_FRAMES: usize = 10;

/// Facing within this many degrees of the way the ground falls reads the
/// slope as going down; facing more than `FACING_UPHILL` away from it, as
/// going up. In between (across the slope) it reads as flat.
const FACING_DOWNHILL: f32 = 35.0;
const FACING_UPHILL: f32 = 145.0;

/// 1 / pi as BO2 writes it.
const INV_PI: f32 = 0.318_309_87;

/// The pitch history one command's moves share (pmove_t's averagePitch and
/// its frames, cleared for every command).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PitchHistory {
    average: f32,
    next: usize,
    frames: [f32; SLOPE_FRAMES],
}

impl PitchHistory {
    /// PM_Footsteps' step: the new pitch takes the oldest one's place in
    /// the running average.
    pub(crate) fn update(&mut self, pitch: f32) {
        let n = SLOPE_FRAMES as f32;
        let sum = n * self.average - self.frames[self.next];
        self.frames[self.next] = pitch;
        self.next += 1;
        self.average = (pitch + sum) / n;
        if self.next >= SLOPE_FRAMES {
            self.next = 0;
        }
    }

    /// averagePitch: the last frames' pitch, averaged.
    pub(crate) fn average(&self) -> f32 {
        self.average
    }
}

/// PM_CalcPlayerPitch: the ground's pitch along the player's facing, up a
/// slope positive and down it negative. In the air, on flat ground or
/// facing across the slope, 0.
pub(crate) fn calc_player_pitch(avatar: &AvatarState, tick_vars: &TickLocals) -> f32 {
    if tick_vars.standing_on_floor == 0 {
        return 0.0;
    }
    let yaw = wrap_degrees(avatar.look_angles[1]);
    let [ground_pitch, ground_yaw] = vectoangles(ground_normal(&tick_vars.floor_probe));
    let off = libm::fabsf(degree_gap(yaw, wrap_degrees(ground_yaw)));
    let pitch = if off < FACING_DOWNHILL {
        -(ground_pitch + 90.0)
    } else if off > FACING_UPHILL {
        ground_pitch + 90.0
    } else {
        0.0
    };
    // Into -180..180.
    degree_gap(pitch, 0.0)
}

/// BO2's vectoangles: a direction's pitch and yaw, each 0..360. Straight up
/// is pitch 270 and straight down 90 (game_math's `vect_to_angles` gives
/// these two the other way round).
fn vectoangles(v: [f32; 3]) -> [f32; 2] {
    if v[0] == 0.0 && v[1] == 0.0 {
        return [if v[2] > 0.0 { 270.0 } else { 90.0 }, 0.0];
    }
    let mut yaw = (libm::atan2(f64::from(v[1]), f64::from(v[0])) as f32) * 180.0 * INV_PI;
    if yaw < 0.0 {
        yaw += 360.0;
    }
    let forward = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    let mut pitch = (libm::atan2(f64::from(v[2]), f64::from(forward)) as f32) * -180.0 * INV_PI;
    if pitch < 0.0 {
        pitch += 360.0;
    }
    [pitch, yaw]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The player facing `yaw`, on ground with this normal (or in the air).
    fn pitch_on(normal: [f32; 3], yaw: f32, grounded_now: bool) -> f32 {
        let mut avatar = AvatarState::ZERO;
        avatar.look_angles[1] = yaw;
        let mut floor_probe = [0; 11];
        for (i, n) in normal.iter().enumerate() {
            floor_probe[1 + i] = n.to_bits();
        }
        let on = u32::from(grounded_now);
        let tick_vars = TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: on,
            standing_on_floor: on,
            near_floor_contact: on,
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        };
        calc_player_pitch(&avatar, &tick_vars)
    }

    fn pitch(normal: [f32; 3], yaw: f32) -> f32 {
        pitch_on(normal, yaw, true)
    }

    /// The ground rising `deg` degrees toward +x.
    fn rising_east(deg: f32) -> [f32; 3] {
        let r = deg.to_radians();
        [-libm::sinf(r), 0.0, libm::cosf(r)]
    }

    #[test]
    fn flat_ground_has_no_pitch() {
        for yaw in [0.0, 20.0, 90.0, 180.0, 300.0] {
            assert_eq!(pitch([0.0, 0.0, 1.0], yaw), 0.0);
        }
    }

    #[test]
    fn up_a_slope_is_positive_and_down_it_negative() {
        let n = rising_east(40.0);
        assert!((pitch(n, 0.0) - 40.0).abs() < 0.01, "{}", pitch(n, 0.0));
        assert!((pitch(n, 180.0) + 40.0).abs() < 0.01, "{}", pitch(n, 180.0));
        // Within 35 degrees of the fall line still counts.
        assert!(pitch(n, 30.0) > 39.0);
        assert!(pitch(n, -150.0) < -39.0);
        // Across the slope: flat.
        assert_eq!(pitch(n, 90.0), 0.0);
        assert_eq!(pitch(n, 40.0), 0.0);
    }

    #[test]
    fn in_the_air_there_is_no_pitch() {
        assert_eq!(pitch_on(rising_east(40.0), 0.0, false), 0.0);
    }

    #[test]
    fn straight_up_and_down_are_bo2s() {
        assert_eq!(vectoangles([0.0, 0.0, 1.0]), [270.0, 0.0]);
        assert_eq!(vectoangles([0.0, 0.0, -1.0]), [90.0, 0.0]);
        let [p, y] = vectoangles([0.0, 1.0, 0.0]);
        assert_eq!(p, 0.0);
        assert!((y - 90.0).abs() < 1e-4);
    }

    #[test]
    fn the_average_runs_over_ten_frames() {
        let mut h = PitchHistory::default();
        for _ in 0..10 {
            h.update(40.0);
        }
        assert!((h.average() - 40.0).abs() < 1e-3);
        // The oldest frames drop out as new ones come in.
        for _ in 0..5 {
            h.update(0.0);
        }
        assert!((h.average() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn one_commands_three_moves_never_reach_thirty() {
        // A command's history starts empty and runs at most three moves.
        let mut h = PitchHistory::default();
        for _ in 0..3 {
            h.update(89.0);
        }
        assert!(h.average() < 30.0, "{}", h.average());
    }
}
