//! Black Ops II's PM_CrashLand: landing after a fall. A fall that hurts
//! slows the player for a moment (the more it hurt, the longer and slower)
//! and sounds as pain on the ground's surface. One that does not hurt sounds
//! as a step, a run step or, from 12 units up, a landing as loud as the fall
//! was long, and that hard landing takes a third off the speed.
//!
//! What BO2 adds to the shared (MW2) landing:
//! - Chest deep (water level 4 exactly) a landing does nothing at all.
//! - Past the knees (level 2 up) a fall hurts half as much.
//! - In any water the landing sounds as water.
//! - On slick ground (the surface's slick flag or the forced slick flag) a
//!   fall that hurts takes a third off the speed instead of stunning.
//!
//! From 12 units up (not out of a dive) the legs play BO2's landing
//! (ANIM_ET_LAND, asked for before the damage and the sound); the sim plays
//! it (`BodyAnims`). Not here: the fall-damage perk granted by the gun in
//! hand.

use crate::bridge::{Bo2Feel, Pml, add_predictable_event, crash_land_fall_height};
use playerstate_t6::game::{PM_TYPE_DEAD, PlayerState, pm_flags};

use crate::dive;
use crate::footstep::ground_surface_type;
use crate::slide_adjust::PMF_SLICK;

const EV_FOOTSTEP_RUN: i32 = entity_t6::event::FOOTSTEP_RUN as i32;
const EV_FOOTSTEP_WALK: i32 = entity_t6::event::FOOTSTEP_WALK as i32;
const EV_LANDING_FIRST: i32 = entity_t6::event::LANDING_FIRST as i32;
const EV_LANDING_PAIN_FIRST: i32 = entity_t6::event::LANDING_PAIN_FIRST as i32;

/// Fall heights: over the first a step sounds, then a run step, then from
/// the third a landing.
const FALL_LIGHT: f32 = 4.0;
const FALL_MEDIUM: f32 = 8.0;
const FALL_HARD: f32 = 12.0;
/// What a hard landing leaves of the speed.
const HARD_LAND_SCALE: f32 = 0.67;

/// The landing's loudness: 4 just over 12 units, rising to 24.
const LAND_VOLUME_PER_UNIT: f32 = 0.038_461_54;
const LAND_VOLUME_MAX: i32 = 24;

/// A dive to prone counts falls from these heights (dtp_fall_damage_*).
const DIVE_FALL_MIN: f32 = 65.0;
const DIVE_FALL_MAX: f32 = 200.0;

/// The slow after a fall that hurts.
const STUN_BASE_MS: i32 = 500;
const STUN_PER_DAMAGE_MS: i32 = 35;
const STUN_MAX_MS: i32 = 2000;

/// Water: exactly this deep a landing does nothing; from the second level a
/// fall hurts half as much.
const NO_LANDING_LEVEL: i32 = 4;
const SOFT_FALL_LEVEL: i32 = 2;

/// Surface flags: ground that never hurts, and slick ground.
const SURF_NODAMAGE: u32 = 0x1;
const SURF_SLICK: u32 = 0x2;

/// The player touched ground after being in the air. True when the legs
/// play the landing.
pub(crate) fn crash_land(ps: &mut PlayerState, pml: &Pml, feel: Bo2Feel) -> bool {
    if ps.waterlevel == NO_LANDING_LEVEL {
        return false;
    }
    let Some(fall_height) = crash_land_fall_height(ps, pml) else {
        return false;
    };
    let flags = pml.ground_trace[4];
    let damage = fall_damage(ps, flags, fall_height, feel);
    let land = fall_height > FALL_HARD && ps.pm_flags & dive::PMF_DIVE == 0;
    let surface = ground_surface_type(ps, flags);
    if damage > 0 {
        damage_landing(ps, flags, damage, surface);
        return land;
    }
    if fall_height <= FALL_LIGHT {
        return land;
    }
    if fall_height < FALL_MEDIUM {
        add_predictable_event(ps, EV_FOOTSTEP_WALK, surface);
        return land;
    }
    if fall_height < FALL_HARD {
        add_predictable_event(ps, EV_FOOTSTEP_RUN, surface);
        return land;
    }
    scale_velocity(ps, HARD_LAND_SCALE);
    add_predictable_event(ps, EV_LANDING_FIRST + surface, land_volume(fall_height));
    land
}

/// The fall's damage, 0 to 100 percent of health.
fn fall_damage(ps: &PlayerState, flags: u32, fall_height: f32, feel: Bo2Feel) -> i32 {
    let mut damage = if ps.pm_flags & dive::PMF_DIVE != 0 {
        damage_between(fall_height, DIVE_FALL_MIN, DIVE_FALL_MAX)
    } else {
        damage_between(fall_height, feel.fall_damage_min_height, feel.fall_damage_max_height)
    };
    let has = |perk: u32| perk != 0 && ps.perks[0] & perk != 0;
    // Zombies: lighter than normal gravity never hurts.
    if feel.zombies && ps.gravity < feel.gravity {
        damage = 0;
    }
    if has(feel.fall_damage_perk) {
        damage = 0;
    }
    // Zombies' flak jacket: any fall that hurts takes 1.
    if feel.zombies && has(feel.flak_jacket_perk) && damage > 0 {
        damage = 1;
    }
    if flags & SURF_NODAMAGE != 0 || ps.pm_type >= PM_TYPE_DEAD {
        damage = 0;
    }
    if ps.waterlevel >= SOFT_FALL_LEVEL {
        damage = (damage as f32 * 0.5) as i32;
    }
    damage
}

fn damage_between(fall_height: f32, min: f32, max: f32) -> i32 {
    if fall_height <= min {
        0
    } else if fall_height < max {
        (((fall_height - min) / (max - min) * 100.0) as i32).clamp(0, 100)
    } else {
        100
    }
}

/// A fall that hurts: the slow (none on slick ground or a killing fall,
/// which only lose a third of the speed) and the pain sound.
fn damage_landing(ps: &mut PlayerState, flags: u32, damage: i32, surface: i32) {
    let slick = ps.pm_flags & PMF_SLICK != 0 || flags & SURF_SLICK != 0;
    let scale = if damage >= 100 || slick {
        HARD_LAND_SCALE
    } else {
        let stun = (STUN_PER_DAMAGE_MS * damage + STUN_BASE_MS).min(STUN_MAX_MS);
        ps.pm_time = stun;
        ps.pm_flags |= pm_flags::TIME_HARDLANDING;
        if stun <= 500 {
            0.5
        } else if stun < 1500 {
            0.5 - (stun as f32 - 500.0) * 0.001 * 0.3
        } else {
            0.2
        }
    };
    scale_velocity(ps, scale);
    add_predictable_event(ps, EV_LANDING_PAIN_FIRST + surface, damage);
}

fn land_volume(fall_height: f32) -> i32 {
    if fall_height <= FALL_HARD {
        return 0;
    }
    ((((fall_height - FALL_HARD) * LAND_VOLUME_PER_UNIT + 1.0) * 4.0) as i32).min(LAND_VOLUME_MAX)
}

fn scale_velocity(ps: &mut PlayerState, scale: f32) {
    for v in &mut ps.velocity {
        *v *= scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STONE: u32 = 18 << 20;
    const GRAVITY: i32 = 800;

    fn feel() -> Bo2Feel {
        Bo2Feel {
            fall_damage_min_height: 128.0,
            fall_damage_max_height: 300.0,
            ..Bo2Feel::BO2_MP
        }
    }

    fn pml(fall_speed: f32) -> Pml {
        let mut ground_trace = [0; 11];
        ground_trace[4] = STONE;
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: 0.05,
            msec: 50,
            walking: 1,
            ground_plane: 1,
            almost_ground_plane: 1,
            ground_trace,
            // Landed this frame at that speed: no distance covered.
            previous_origin: [0.0; 3],
            previous_velocity: [0.0, 0.0, -fall_speed],
            holdrand: 0,
        }
    }

    /// The landing speed of a fall from `height`.
    fn speed_from(height: f32) -> f32 {
        libm::sqrtf(2.0 * GRAVITY as f32 * height)
    }

    fn player() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.gravity = GRAVITY;
        ps.velocity = [100.0, 0.0, -50.0];
        ps
    }

    fn land(ps: &mut PlayerState, height: f32) {
        crash_land(ps, &pml(speed_from(height)), feel());
    }

    fn event(ps: &PlayerState) -> Option<(i32, i32)> {
        (ps.predictable_events[0] != 0).then_some((ps.predictable_events[0], ps.predictable_event_parms[0] as i32))
    }

    #[test]
    fn a_short_drop_sounds_as_a_step() {
        let mut ps = player();
        land(&mut ps, 6.0);
        assert_eq!(event(&ps), Some((EV_FOOTSTEP_WALK, 18)));
        let mut ps = player();
        land(&mut ps, 10.0);
        assert_eq!(event(&ps), Some((EV_FOOTSTEP_RUN, 18)));
        let mut ps = player();
        land(&mut ps, 3.0);
        assert_eq!(event(&ps), None);
    }

    #[test]
    fn a_hard_landing_is_as_loud_as_the_fall() {
        let mut ps = player();
        land(&mut ps, 64.0);
        let (kind, volume) = event(&ps).expect("a landing");
        assert_eq!(kind, EV_LANDING_FIRST + 18);
        assert!((11..=12).contains(&volume), "{volume}");
        assert!((ps.velocity[0] - 67.0).abs() < 0.01);
    }

    #[test]
    fn a_fall_that_hurts_slows_the_player() {
        let mut ps = player();
        land(&mut ps, 214.0);
        let (kind, damage) = event(&ps).expect("pain");
        assert_eq!(kind, EV_LANDING_PAIN_FIRST + 18);
        assert!((49..=50).contains(&damage), "{damage}");
        assert_eq!(ps.pm_time, (35 * damage + 500).min(2000));
        assert!(ps.pm_flags & pm_flags::TIME_HARDLANDING != 0);
        assert!(ps.velocity[0] < 50.0);
    }

    #[test]
    fn slick_ground_skips_the_slow() {
        let mut ps = player();
        ps.pm_flags = PMF_SLICK;
        land(&mut ps, 214.0);
        assert_eq!(ps.pm_time, 0);
        assert!((ps.velocity[0] - 67.0).abs() < 0.01);
        let mut ps = player();
        let mut slick = pml(speed_from(214.0));
        slick.ground_trace[4] |= SURF_SLICK;
        crash_land(&mut ps, &slick, feel());
        assert_eq!(ps.pm_time, 0);
    }

    #[test]
    fn chest_deep_a_landing_does_nothing() {
        let mut ps = player();
        ps.waterlevel = 4;
        land(&mut ps, 400.0);
        assert_eq!(event(&ps), None);
        assert_eq!(ps.velocity, [100.0, 0.0, -50.0]);
    }

    #[test]
    fn water_softens_the_fall_and_sounds_as_water() {
        // Ankle deep: full damage, but a water sound.
        let mut ps = player();
        ps.waterlevel = 1;
        land(&mut ps, 300.0);
        assert_eq!(event(&ps), Some((EV_LANDING_PAIN_FIRST + 20, 100)));
        // Knee deep and up: half.
        let mut ps = player();
        ps.waterlevel = 2;
        land(&mut ps, 300.0);
        assert_eq!(event(&ps), Some((EV_LANDING_PAIN_FIRST + 20, 50)));
        let mut ps = player();
        ps.waterlevel = 5;
        land(&mut ps, 64.0);
        assert_eq!(event(&ps).map(|e| e.0), Some(EV_LANDING_FIRST + 20));
    }

    #[test]
    fn a_dive_counts_its_own_heights() {
        let mut ps = player();
        ps.pm_flags = dive::PMF_DIVE;
        land(&mut ps, 100.0);
        let (_, damage) = event(&ps).expect("pain");
        assert!((25..=26).contains(&damage), "{damage}");
    }

    #[test]
    fn no_damage_ground_and_the_dead_take_none() {
        let mut ps = player();
        let mut soft = pml(speed_from(300.0));
        soft.ground_trace[4] |= SURF_NODAMAGE;
        crash_land(&mut ps, &soft, feel());
        assert_eq!(event(&ps).map(|e| e.0), Some(EV_LANDING_FIRST + 18));
        let mut ps = player();
        ps.pm_type = PM_TYPE_DEAD;
        land(&mut ps, 300.0);
        assert_eq!(event(&ps).map(|e| e.0), Some(EV_LANDING_FIRST + 18));
    }

    #[test]
    fn the_legs_land_from_over_12_units_but_not_out_of_a_dive() {
        let lands = |height: f32, flags: u32, water: i32| {
            let mut ps = player();
            ps.pm_flags = flags;
            ps.waterlevel = water;
            crash_land(&mut ps, &pml(speed_from(height)), feel())
        };
        assert!(!lands(10.0, 0, 0));
        assert!(lands(14.0, 0, 0));
        assert!(lands(214.0, 0, 0), "a fall that hurts lands too");
        assert!(!lands(214.0, dive::PMF_DIVE, 0));
        assert!(!lands(214.0, 0, NO_LANDING_LEVEL), "chest deep a landing does nothing");
    }
}
