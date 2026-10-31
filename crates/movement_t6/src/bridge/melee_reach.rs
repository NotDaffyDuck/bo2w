//! The knife charge: a short rush toward a target that slows to a stop.

use game_math::flat_basis;
use playerstate_t6::game::{AvatarState, movement_state_bits};

use crate::bridge::hull_hits::SolidProbe;
use crate::bridge::pml::TickLocals;
use crate::bridge::slide::step_slide_move;

const MILLIS_PER_SEC_INV: f32 = 0.001;

/// Closer than this the charge is just a melee swing.
pub const STOCK_KNIFE_REACH: f32 = 64.0;

/// How long the held weapon takes to land a swing, and to land one that had
/// to be charged to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LungeGunTimings {
    pub strike_wait_millis: i32,
    pub lunge_windup_wait_millis: i32,
}

/// Cancel the charge and forget its heading.
pub(crate) fn melee_charge_clear(avatar: &mut AvatarState) {
    avatar.movement_state_bits &= !movement_state_bits::MELEE_CHARGE;
    avatar.lunge_heading = 0.0;
    avatar.lunge_reach = 0;
    avatar.lunge_clock = 0;
}

/// Work out how long a freshly started charge lasts and launch the player
/// along its heading at twice the average speed (the charge slows to a stop,
/// so the mean comes out right). Does nothing once the time is set; clears
/// the charge when the flag is off or the weapon gives no time.
pub fn calc_melee_charge_time(
    avatar: &mut AvatarState,
    delays: LungeGunTimings,
    knife_reach: f32,
) {
    if avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE == 0 {
        melee_charge_clear(avatar);
        return;
    }
    if avatar.lunge_clock != 0 {
        return;
    }

    let distance = avatar.lunge_reach as f32;
    let delay_ms = if distance <= knife_reach {
        delays.strike_wait_millis
    } else {
        delays.lunge_windup_wait_millis
    };
    let seconds = delay_ms as f32 * MILLIS_PER_SEC_INV;
    if seconds <= 0.0 {
        melee_charge_clear(avatar);
        return;
    }

    let launch_speed = (distance / seconds) * 2.0;
    let (ahead, _) = flat_basis(avatar.lunge_heading);
    avatar.velocity[0] = launch_speed * ahead[0];
    avatar.velocity[1] = launch_speed * ahead[1];
    avatar.lunge_clock = delay_ms;
}

/// Slide the velocity along the floor the last ground trace found.
fn along_ground(velocity: &mut [f32; 3], floor_probe: &[u32; 11]) {
    let floor_n = [
        f32::from_bits(floor_probe[1]),
        f32::from_bits(floor_probe[2]),
        f32::from_bits(floor_probe[3]),
    ];
    crate::bridge::project_velocity(velocity, &floor_n);
}

/// Move one frame of the charge: the speed falls linearly to zero over the
/// charge time, the frame moves at the mean of its start and end speed, and
/// the charge ends when the time runs out. Returns how far a stair step
/// raised the camera.
pub fn melee_charge_move<C: SolidProbe>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    collide_filter: u32,
    solids: &C,
    zombies: bool,
) -> f32 {
    let [vx, vy, vz] = avatar.velocity;
    let speed = libm::sqrtf(vx * vx + vy * vy + vz * vz);

    let mut heading = [0.0_f32; 3];
    let mut end_speed = 0.0_f32;
    if 0.0 < speed {
        heading = [vx / speed, vy / speed, vz / speed];
        let seconds = avatar.lunge_clock as f32 * MILLIS_PER_SEC_INV;
        end_speed = speed - (speed / seconds) * tick_vars.step_millis_elapsed;
        if end_speed < 0.0 {
            end_speed = 0.0;
        }
        let mean_speed = (speed + end_speed) * 0.5;
        avatar.velocity = [
            mean_speed * heading[0],
            mean_speed * heading[1],
            mean_speed * heading[2],
        ];
    }

    along_ground(&mut avatar.velocity, &tick_vars.floor_probe);

    let mut step_rise = 0.0;
    if avatar.velocity[0] != 0.0 || avatar.velocity[1] != 0.0 {
        step_rise = step_slide_move(avatar, tick_vars, solids, box_lo, box_hi, collide_filter, None, zombies);
    }

    avatar.velocity = [
        end_speed * heading[0],
        end_speed * heading[1],
        end_speed * heading[2],
    ];

    avatar.lunge_clock -= tick_vars.frame_ms;
    if avatar.lunge_clock <= 0 {
        avatar.velocity[0] = 0.0;
        avatar.velocity[1] = 0.0;
        melee_charge_clear(avatar);
    }
    step_rise
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::SweepAsk;
    use crate::testing::tick_vars;
    use trace_t6::Trace;

    const DELAYS: LungeGunTimings = LungeGunTimings {
        strike_wait_millis: 300,
        lunge_windup_wait_millis: 500,
    };

    /// Nothing in the way anywhere.
    struct OpenAir;

    impl SolidProbe for OpenAir {
        fn trace(&self, _input: SweepAsk) -> Trace {
            Trace::new()
        }
    }

    fn charging(distance: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::MELEE_CHARGE;
        avatar.lunge_reach = distance;
        avatar.lunge_heading = 0.0;
        avatar
    }

    #[test]
    fn a_charge_flag_that_is_off_clears_the_charge() {
        let mut avatar = AvatarState::ZERO;
        avatar.lunge_reach = 90;
        avatar.lunge_clock = 40;
        calc_melee_charge_time(&mut avatar, DELAYS, STOCK_KNIFE_REACH);
        assert_eq!((avatar.lunge_reach, avatar.lunge_clock), (0, 0));
    }

    #[test]
    fn a_close_target_uses_the_swing_delay_and_a_far_one_the_charge_delay() {
        let mut near = charging(60);
        calc_melee_charge_time(&mut near, DELAYS, STOCK_KNIFE_REACH);
        assert_eq!(near.lunge_clock, 300);
        let mut far = charging(120);
        calc_melee_charge_time(&mut far, DELAYS, STOCK_KNIFE_REACH);
        assert_eq!(far.lunge_clock, 500);
    }

    #[test]
    fn the_launch_speed_is_twice_the_mean_needed_to_cover_the_distance() {
        let mut avatar = charging(120);
        calc_melee_charge_time(&mut avatar, DELAYS, STOCK_KNIFE_REACH);
        // 120 units in 0.5 s is 240/s on average, so it launches at 480/s.
        assert!((avatar.velocity[0] - 480.0).abs() < 1e-2, "{:?}", avatar.velocity);
        assert!(avatar.velocity[1].abs() < 1e-2);
    }

    #[test]
    fn a_charge_already_under_way_keeps_its_time() {
        let mut avatar = charging(120);
        avatar.lunge_clock = 77;
        avatar.velocity = [5.0, 0.0, 0.0];
        calc_melee_charge_time(&mut avatar, DELAYS, STOCK_KNIFE_REACH);
        assert_eq!((avatar.lunge_clock, avatar.velocity[0]), (77, 5.0));
    }

    #[test]
    fn a_weapon_with_no_swing_time_cancels_the_charge() {
        let mut avatar = charging(120);
        calc_melee_charge_time(
            &mut avatar,
            LungeGunTimings::default(),
            STOCK_KNIFE_REACH,
        );
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE, 0);
    }

    #[test]
    fn the_charge_runs_out_and_stops_the_player() {
        let mut avatar = charging(120);
        avatar.velocity = [480.0, 0.0, 0.0];
        avatar.lunge_clock = 100;
        let frame = tick_vars(true);
        // The test frame is 50 ms, so the second one ends the charge.
        melee_charge_move(&mut avatar, &frame, [-15.0; 3], [15.0; 3], 1, &OpenAir, false);
        assert!(avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE != 0);
        assert_eq!(avatar.lunge_clock, 50);
        assert!(avatar.velocity[0] > 0.0 && avatar.velocity[0] < 480.0);
        melee_charge_move(&mut avatar, &frame, [-15.0; 3], [15.0; 3], 1, &OpenAir, false);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE, 0);
        assert_eq!(avatar.velocity, [0.0; 3]);
    }
}
