//! The knife charge: a short rush toward a target that slows to a stop.

use game_math::yaw_vectors_2d;
use playerstate_t6::game::{PlayerState, pm_flags};

use crate::bridge::collision::CollisionBackend;
use crate::bridge::pml::Pml;
use crate::bridge::slide::step_slide_move;

const MS_TO_SECONDS: f32 = 0.001;

/// Closer than this the charge is just a melee swing.
pub const PLAYER_MELEE_RANGE_DEFAULT: f32 = 64.0;

/// How long the held weapon takes to land a swing, and to land one that had
/// to be charged to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeleeChargeWeaponDelays {
    pub melee_delay_ms: i32,
    pub melee_charge_delay_ms: i32,
}

/// Cancel the charge and forget its heading.
pub(crate) fn melee_charge_clear(ps: &mut PlayerState) {
    ps.pm_flags &= !pm_flags::MELEE_CHARGE;
    ps.melee_charge_yaw = 0.0;
    ps.melee_charge_dist = 0;
    ps.melee_charge_time = 0;
}

/// Work out how long a freshly started charge lasts and launch the player
/// along its heading at twice the average speed (the charge slows to a stop,
/// so the mean comes out right). Does nothing once the time is set; clears
/// the charge when the flag is off or the weapon gives no time.
pub fn calc_melee_charge_time(
    ps: &mut PlayerState,
    delays: MeleeChargeWeaponDelays,
    player_melee_range: f32,
) {
    if ps.pm_flags & pm_flags::MELEE_CHARGE == 0 {
        melee_charge_clear(ps);
        return;
    }
    if ps.melee_charge_time != 0 {
        return;
    }

    let distance = ps.melee_charge_dist as f32;
    let delay_ms = if distance <= player_melee_range {
        delays.melee_delay_ms
    } else {
        delays.melee_charge_delay_ms
    };
    let seconds = delay_ms as f32 * MS_TO_SECONDS;
    if seconds <= 0.0 {
        melee_charge_clear(ps);
        return;
    }

    let launch_speed = (distance / seconds) * 2.0;
    let (forward, _) = yaw_vectors_2d(ps.melee_charge_yaw);
    ps.velocity[0] = launch_speed * forward[0];
    ps.velocity[1] = launch_speed * forward[1];
    ps.melee_charge_time = delay_ms;
}

/// Slide the velocity along the floor the last ground trace found.
fn along_ground(velocity: &mut [f32; 3], ground_trace: &[u32; 11]) {
    let normal = [
        f32::from_bits(ground_trace[1]),
        f32::from_bits(ground_trace[2]),
        f32::from_bits(ground_trace[3]),
    ];
    crate::bridge::project_velocity(velocity, &normal);
}

/// Move one frame of the charge: the speed falls linearly to zero over the
/// charge time, the frame moves at the mean of its start and end speed, and
/// the charge ends when the time runs out. Returns how far a stair step
/// raised the camera.
pub fn melee_charge_move<C: CollisionBackend>(
    ps: &mut PlayerState,
    pml: &Pml,
    mins: [f32; 3],
    maxs: [f32; 3],
    tracemask: u32,
    collision: &C,
    zombies: bool,
) -> f32 {
    let [vx, vy, vz] = ps.velocity;
    let speed = libm::sqrtf(vx * vx + vy * vy + vz * vz);

    let mut heading = [0.0_f32; 3];
    let mut end_speed = 0.0_f32;
    if speed > 0.0 {
        heading = [vx / speed, vy / speed, vz / speed];
        let seconds = ps.melee_charge_time as f32 * MS_TO_SECONDS;
        end_speed = speed - (speed / seconds) * pml.frametime;
        if end_speed < 0.0 {
            end_speed = 0.0;
        }
        let mean_speed = (speed + end_speed) * 0.5;
        ps.velocity = [
            mean_speed * heading[0],
            mean_speed * heading[1],
            mean_speed * heading[2],
        ];
    }

    along_ground(&mut ps.velocity, &pml.ground_trace);

    let mut step_rise = 0.0;
    if ps.velocity[0] != 0.0 || ps.velocity[1] != 0.0 {
        step_rise = step_slide_move(ps, pml, collision, mins, maxs, tracemask, None, zombies);
    }

    ps.velocity = [
        end_speed * heading[0],
        end_speed * heading[1],
        end_speed * heading[2],
    ];

    ps.melee_charge_time -= pml.msec;
    if ps.melee_charge_time <= 0 {
        ps.velocity[0] = 0.0;
        ps.velocity[1] = 0.0;
        melee_charge_clear(ps);
    }
    step_rise
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::GroundTraceInput;
    use crate::testing::pml;
    use trace_t6::Trace;

    const DELAYS: MeleeChargeWeaponDelays = MeleeChargeWeaponDelays {
        melee_delay_ms: 300,
        melee_charge_delay_ms: 500,
    };

    /// Nothing in the way anywhere.
    struct OpenAir;

    impl CollisionBackend for OpenAir {
        fn trace(&self, _input: GroundTraceInput) -> Trace {
            Trace {
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    fn charging(distance: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::MELEE_CHARGE;
        ps.melee_charge_dist = distance;
        ps.melee_charge_yaw = 0.0;
        ps
    }

    #[test]
    fn a_charge_flag_that_is_off_clears_the_charge() {
        let mut ps = PlayerState::ZERO;
        ps.melee_charge_dist = 90;
        ps.melee_charge_time = 40;
        calc_melee_charge_time(&mut ps, DELAYS, PLAYER_MELEE_RANGE_DEFAULT);
        assert_eq!((ps.melee_charge_dist, ps.melee_charge_time), (0, 0));
    }

    #[test]
    fn a_close_target_uses_the_swing_delay_and_a_far_one_the_charge_delay() {
        let mut near = charging(60);
        calc_melee_charge_time(&mut near, DELAYS, PLAYER_MELEE_RANGE_DEFAULT);
        assert_eq!(near.melee_charge_time, 300);
        let mut far = charging(120);
        calc_melee_charge_time(&mut far, DELAYS, PLAYER_MELEE_RANGE_DEFAULT);
        assert_eq!(far.melee_charge_time, 500);
    }

    #[test]
    fn the_launch_speed_is_twice_the_mean_needed_to_cover_the_distance() {
        let mut ps = charging(120);
        calc_melee_charge_time(&mut ps, DELAYS, PLAYER_MELEE_RANGE_DEFAULT);
        // 120 units in 0.5 s is 240/s on average, so it launches at 480/s.
        assert!((ps.velocity[0] - 480.0).abs() < 1e-2, "{:?}", ps.velocity);
        assert!(ps.velocity[1].abs() < 1e-2);
    }

    #[test]
    fn a_charge_already_under_way_keeps_its_time() {
        let mut ps = charging(120);
        ps.melee_charge_time = 77;
        ps.velocity = [5.0, 0.0, 0.0];
        calc_melee_charge_time(&mut ps, DELAYS, PLAYER_MELEE_RANGE_DEFAULT);
        assert_eq!((ps.melee_charge_time, ps.velocity[0]), (77, 5.0));
    }

    #[test]
    fn a_weapon_with_no_swing_time_cancels_the_charge() {
        let mut ps = charging(120);
        calc_melee_charge_time(
            &mut ps,
            MeleeChargeWeaponDelays::default(),
            PLAYER_MELEE_RANGE_DEFAULT,
        );
        assert_eq!(ps.pm_flags & pm_flags::MELEE_CHARGE, 0);
    }

    #[test]
    fn the_charge_runs_out_and_stops_the_player() {
        let mut ps = charging(120);
        ps.velocity = [480.0, 0.0, 0.0];
        ps.melee_charge_time = 100;
        let frame = pml(true);
        // The test frame is 50 ms, so the second one ends the charge.
        melee_charge_move(&mut ps, &frame, [-15.0; 3], [15.0; 3], 1, &OpenAir, false);
        assert!(ps.pm_flags & pm_flags::MELEE_CHARGE != 0);
        assert_eq!(ps.melee_charge_time, 50);
        assert!(ps.velocity[0] > 0.0 && ps.velocity[0] < 480.0);
        melee_charge_move(&mut ps, &frame, [-15.0; 3], [15.0; 3], 1, &OpenAir, false);
        assert_eq!(ps.pm_flags & pm_flags::MELEE_CHARGE, 0);
        assert_eq!(ps.velocity, [0.0; 3]);
    }
}
