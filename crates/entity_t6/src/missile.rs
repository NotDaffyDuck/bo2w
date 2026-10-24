//! BO2's grenade angles from `g_missile.cpp` and `cg_ents.cpp`:
//! `G_InitGrenadeMovement`, the no-draw delay `G_FireGrenade` adds,
//! `CG_Missile`'s hide checks, `MissileLandAngles` and its two flat
//! variants, and which of them `BounceMissile` picks.
//!
//! BO2's random draws come through [`MissileRng`], in BO2's order, so a
//! caller with BO2's generator gets BO2's spins.

use crate::angles::{
    angle_normalize_360, angle_subtract, nearest_pitch_and_yaw_on_plane, pitch_for_yaw_on_normal,
    roll_onto_normal, vectoangles,
};
use crate::entity_state::EntityState;
use crate::link::{EntityShared, set_angle};
use crate::trajectory::{Gravity, Trajectory, UnknownTrType, tr_type};

/// `WeapRotateType`.
pub mod rotate_type {
    pub const GRENADE: u8 = 0;
    pub const BLADE: u8 = 1;
    pub const CYLINDER: u8 = 2;
}

/// `WeapStickinessType`.
pub mod stickiness {
    pub const NONE: u8 = 0;
    pub const ALL: u8 = 1;
    pub const ALL_NO_SENTIENTS: u8 = 2;
    pub const GROUND: u8 = 3;
    pub const GROUND_WITH_YAW: u8 = 4;
    pub const FLESH: u8 = 5;
}

/// `entityState_s::lerp.eFlags` bit the client never draws.
pub const EF_NODRAW: i32 = 0x20;
/// `lerp.eFlags2` bit a demo playback hides.
pub const EF2_DEMO_HIDDEN: i32 = 0x2_0000;

/// A spinning grenade starts pitched this far back from its flight path.
pub const GRENADE_APOS_PITCH_OFFSET: f32 = 120.0;
/// Pitch spin, degrees per second, drawn in this range with a random sign.
pub const GRENADE_SPIN_PITCH_MIN: f32 = 340.0;
pub const GRENADE_SPIN_PITCH_MAX: f32 = 800.0;
/// Roll spin, degrees per second, drawn in this range with a random sign.
pub const GRENADE_SPIN_ROLL_MIN: f32 = 180.0;
pub const GRENADE_SPIN_ROLL_MAX: f32 = 540.0;
/// A blade (throwing knife, combat axe) only pitches, at this rate.
pub const BLADE_SPIN_PITCH: f32 = 1500.0;

/// A hit surface this flat or flatter (`normal.z`) counts as a wall.
pub const LAND_WALL_NORMAL_Z: f32 = 0.1;
/// Off the surface pitch by this much or more: keep the pitch as it is.
pub const LAND_KEEP_PITCH_DEG: f32 = 80.0;
/// Off by this much or more: ease a quarter of the way there.
pub const LAND_EASE_PITCH_DEG: f32 = 45.0;

/// `BounceMissile` treats a bounce as a hit when the new velocity runs
/// into the surface faster than this.
pub const BOUNCE_LAND_SPEED: f32 = 25.0;
/// `BounceMissile` rests a grenade when it is this slow, or the floor's
/// `normal.z` is above [`REST_NORMAL_Z`].
pub const REST_SPEED: f32 = 1.0;
pub const REST_NORMAL_Z: f32 = 0.7;

/// BO2's game-side random numbers.
pub trait MissileRng {
    /// `G_rand`.
    fn rand(&mut self) -> u32;
    /// `G_random`: [0, 1].
    fn random(&mut self) -> f32;
    /// `G_irand(min, max)`: [min, max).
    fn irand(&mut self, min: i32, max: i32) -> i32;
    /// `G_flrand(min, max)`.
    fn flrand(&mut self, min: f32, max: f32) -> f32;
}

/// The `pos.trType` `G_InitGrenadeMovement` gives a grenade: gravity, or
/// moon gravity in Zombies for a `moon_gravity` grenade or an owner in
/// low gravity.
pub fn grenade_pos_tr_type(zombies: bool, moon_gravity_noteworthy: bool, owner_low_gravity: bool) -> u8 {
    if zombies && (moon_gravity_noteworthy || owner_low_gravity) {
        tr_type::MOON_GRAVITY
    } else {
        tr_type::GRAVITY
    }
}

/// `G_InitGrenadeMovement`: launch from `start` at velocity `dir`.
///
/// `start_time` is `level.time` plus the grenade's start offset.
/// `rotate` is the weapon's `rotateType` when it spins (`G_FireGrenade`
/// passes it when the throw asks for spin and the weapon has `rotate`).
#[allow(clippy::too_many_arguments)]
pub fn init_grenade_movement(
    es: &mut EntityState,
    r: &mut EntityShared,
    start: [f32; 3],
    dir: [f32; 3],
    start_time: i32,
    pos_tr_type: u8,
    rotate: Option<u8>,
    rng: &mut impl MissileRng,
) {
    es.lerp.pos.tr_type = pos_tr_type;
    es.lerp.pos.tr_time = start_time;
    r.current_origin = start;
    es.lerp.pos.tr_base = start;
    // The facing comes from the exact velocity; the stored one is cut to
    // whole units.
    r.current_angles = vectoangles(dir);
    es.lerp.pos.tr_delta = [dir[0] as i32 as f32, dir[1] as i32 as f32, dir[2] as i32 as f32];
    let Some(kind) = rotate else {
        let angles = r.current_angles;
        set_angle(es, r, angles);
        return;
    };
    let apos = &mut es.lerp.apos;
    apos.tr_type = tr_type::LINEAR;
    apos.tr_time = start_time;
    apos.tr_base = r.current_angles;
    apos.tr_base[0] = angle_normalize_360(apos.tr_base[0] - GRENADE_APOS_PITCH_OFFSET);
    if kind == rotate_type::BLADE {
        apos.tr_delta = [BLADE_SPIN_PITCH, 0.0, 0.0];
    } else {
        let sign = (rng.irand(0, 2) * 2 - 1) as f32;
        let pitch = rng.flrand(GRENADE_SPIN_PITCH_MIN, GRENADE_SPIN_PITCH_MAX) * sign;
        let sign = (rng.irand(0, 2) * 2 - 1) as f32;
        let roll = rng.flrand(GRENADE_SPIN_ROLL_MIN, GRENADE_SPIN_ROLL_MAX);
        apos.tr_delta = [pitch as i32 as f32, 0.0, (roll * sign) as i32 as f32];
    }
    r.current_angles = apos.tr_base;
}

/// The ms `G_FireGrenade` adds to `launchTime` for a player's throw, so
/// the grenade stays hidden until it clears the hand: faster throws
/// show sooner, 20 to 50 ms.
pub fn fire_grenade_no_draw_ms(speed: f32) -> i32 {
    let ms = (speed * -35.0 * 0.001_666_666_7 + 85.0) as i32;
    if ms >= 50 {
        50
    } else if ms < 21 {
        20
    } else {
        ms
    }
}

/// `CG_Missile`'s early outs: true when the client draws nothing.
pub fn missile_hidden(es: &EntityState, client_time: i32, demo_playing: bool) -> bool {
    es.lerp.e_flags & EF_NODRAW != 0
        || (demo_playing && es.lerp.e_flags2 & EF2_DEMO_HIDDEN != 0)
        || client_time < es.lerp.u.missile().launch_time
}

/// The time inside this frame the trace hit, as BO2 works it out:
/// `previousTime + (time - previousTime) * fraction`, cut to ms.
pub fn land_hit_time(level_time: i32, previous_time: i32, fraction: f32) -> i32 {
    ((level_time - previous_time) as f32 * fraction) as i32 + previous_time
}

/// What `MissileLandAngles` reads off the weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LandWeapon {
    pub rotate_type: u8,
    /// `grenadeRollingEnabled` and the weapon's `bKeepRolling`, on a
    /// grenade that is not a dud: the landing leaves the spin alone.
    pub keeps_rolling: bool,
}

fn land_tail(angles: &mut [f32; 3], w: LandWeapon, force_align: bool, blade_bounce: bool) {
    if w.rotate_type == rotate_type::CYLINDER {
        angles[2] = 90.0;
    }
    if force_align && !blade_bounce && w.rotate_type == rotate_type::BLADE {
        angles[2] = 90.0;
    }
}

/// `MissileLandAngles`: the grenade's angles at `hit_time` on a surface
/// with this `normal`, turning `apos`' spin for the bounce.
///
/// `force_align` lays it flat on the surface (the at-rest call);
/// `blade_bounce` flips a blade's spin instead of damping it.
pub fn missile_land_angles(
    apos: &mut Trajectory,
    normal: [f32; 3],
    hit_time: i32,
    force_align: bool,
    blade_bounce: bool,
    w: LandWeapon,
    rng: &mut impl MissileRng,
) -> Result<[f32; 3], UnknownTrType> {
    let mut angles = apos.evaluate(hit_time, Gravity::BO2)?;
    if w.keeps_rolling {
        return Ok(angles);
    }
    if normal[2] <= LAND_WALL_NORMAL_Z {
        if !force_align {
            if !blade_bounce {
                let jitter = ((rng.rand() & 0x7f) as i32 - 63) as f32;
                apos.tr_delta[0] = angle_normalize_360(jitter + apos.tr_delta[0]);
            }
            return Ok(angles);
        }
        land_tail(&mut angles, w, force_align, blade_bounce);
        return Ok(angles);
    }
    let mut surface_pitch = pitch_for_yaw_on_normal(angles[1], normal);
    let delta = angle_subtract(surface_pitch, angles[0]);
    let off = libm::fabsf(delta);
    if !force_align || blade_bounce {
        apos.tr_base = angles;
        apos.tr_time = hit_time;
        if off >= LAND_KEEP_PITCH_DEG {
            if !blade_bounce {
                apos.tr_delta[0] *= rng.random() * 0.3 + 0.85;
            }
        } else if !blade_bounce {
            apos.tr_delta[0] = -((rng.random() * 0.3 + 0.85) * apos.tr_delta[0]);
        } else {
            apos.tr_delta[0] *= -1.0;
        }
    }
    angles[0] = angle_subtract(angles[0], 0.0);
    if !force_align && off >= LAND_EASE_PITCH_DEG {
        angles[0] = if off >= LAND_KEEP_PITCH_DEG {
            angle_normalize_360(angles[0])
        } else {
            angle_normalize_360(delta * 0.25 + angles[0])
        };
        return Ok(angles);
    }
    if libm::fabsf(angles[0]) > 90.0 {
        surface_pitch += 180.0;
    }
    angles[0] = angle_normalize_360(surface_pitch);
    if force_align {
        land_tail(&mut angles, w, force_align, blade_bounce);
    }
    Ok(angles)
}

/// `MissileLandAnglesFlat`: lie flat on the surface, facing the nearest
/// way to the flight facing.
pub fn missile_land_angles_flat(apos: &Trajectory, normal: [f32; 3], hit_time: i32) -> Result<[f32; 3], UnknownTrType> {
    let angles = apos.evaluate(hit_time, Gravity::BO2)?;
    let mut angles = nearest_pitch_and_yaw_on_plane(angles, normal);
    angles[2] = roll_onto_normal(angles, normal);
    Ok(angles)
}

/// `MissileLandAnglesFlatMaintainingDirection`: lie flat on the surface,
/// keeping the yaw it had.
pub fn missile_land_angles_flat_keep_yaw(
    apos: &Trajectory,
    normal: [f32; 3],
    hit_time: i32,
) -> Result<[f32; 3], UnknownTrType> {
    let mut angles = apos.evaluate(hit_time, Gravity::BO2)?;
    angles[0] = pitch_for_yaw_on_normal(angles[1], normal);
    angles[2] = 0.0;
    angles[2] = roll_onto_normal(angles, normal);
    Ok(angles)
}

/// `BounceMissile`'s rest test, once the grenade may rest at all: sticky
/// grenades rest on any hit, others when slow or on a floor and either
/// ground-sticky or under `grenadeRestThreshold`.
pub fn comes_to_rest(stick: u8, speed: f32, normal_z: f32, rest_threshold: f32) -> bool {
    stick == stickiness::ALL
        || stick == stickiness::ALL_NO_SENTIENTS
        || ((speed < REST_SPEED || REST_NORMAL_Z < normal_z)
            && (stick == stickiness::GROUND || stick == stickiness::GROUND_WITH_YAW || speed < rest_threshold))
}

/// The angles `BounceMissile` stands a grenade at when it rests, before
/// it calls `G_SetAngle` with them.
pub fn rest_angles(
    apos: &mut Trajectory,
    normal: [f32; 3],
    hit_time: i32,
    stick: u8,
    w: LandWeapon,
    rng: &mut impl MissileRng,
) -> Result<[f32; 3], UnknownTrType> {
    match stick {
        stickiness::GROUND_WITH_YAW | stickiness::ALL_NO_SENTIENTS => {
            missile_land_angles_flat_keep_yaw(apos, normal, hit_time)
        }
        stickiness::GROUND => missile_land_angles_flat(apos, normal, hit_time),
        _ => missile_land_angles(apos, normal, hit_time, true, false, w, rng),
    }
}

/// A bounce off a surface: when the new `velocity` runs into it faster
/// than [`BOUNCE_LAND_SPEED`], turn the spin by rotate type and restart
/// `apos` from the hit at `level_time`. Returns whether it did.
pub fn bounce_angles(
    apos: &mut Trajectory,
    normal: [f32; 3],
    velocity: [f32; 3],
    hit_time: i32,
    level_time: i32,
    w: LandWeapon,
    rng: &mut impl MissileRng,
) -> Result<bool, UnknownTrType> {
    let into = normal[1] * velocity[1] + velocity[0] * normal[0] + normal[2] * velocity[2];
    if into <= BOUNCE_LAND_SPEED {
        return Ok(false);
    }
    let (force_align, blade_bounce) = match w.rotate_type {
        rotate_type::BLADE => (false, true),
        rotate_type::CYLINDER => (true, false),
        _ => (false, false),
    };
    let angles = missile_land_angles(apos, normal, hit_time, force_align, blade_bounce, w, rng)?;
    apos.tr_base = angles;
    apos.tr_time = level_time;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays back fixed draws and counts them.
    #[derive(Default)]
    struct Fixed {
        rand: u32,
        random: f32,
        irand: i32,
        flrand_t: f32,
        draws: u32,
    }

    impl MissileRng for Fixed {
        fn rand(&mut self) -> u32 {
            self.draws += 1;
            self.rand
        }
        fn random(&mut self) -> f32 {
            self.draws += 1;
            self.random
        }
        fn irand(&mut self, _min: i32, _max: i32) -> i32 {
            self.draws += 1;
            self.irand
        }
        fn flrand(&mut self, min: f32, max: f32) -> f32 {
            self.draws += 1;
            min + (max - min) * self.flrand_t
        }
    }

    fn close(a: f32, b: f32) -> bool {
        libm::fabsf(a - b) < 1.0e-3
    }

    fn spinning(base: [f32; 3], delta: [f32; 3]) -> Trajectory {
        Trajectory { tr_type: tr_type::LINEAR, tr_time: 0, tr_duration: 0, tr_base: base, tr_delta: delta }
    }

    const FLOOR: [f32; 3] = [0.0, 0.0, 1.0];
    const WALL: [f32; 3] = [1.0, 0.0, 0.0];

    #[test]
    fn launch_spins_a_grenade_and_a_blade() {
        let mut es = EntityState::default();
        let mut r = EntityShared::default();
        let mut rng = Fixed { irand: 1, flrand_t: 0.0, ..Default::default() };
        init_grenade_movement(
            &mut es, &mut r, [1.0, 2.0, 3.0], [500.7, 0.0, 200.9], 1000, tr_type::GRAVITY,
            Some(rotate_type::GRENADE), &mut rng,
        );
        assert_eq!(rng.draws, 4);
        assert_eq!(es.lerp.pos.tr_delta, [500.0, 0.0, 200.0]);
        assert_eq!((es.lerp.pos.tr_type, es.lerp.pos.tr_time), (tr_type::GRAVITY, 1000));
        assert_eq!(es.lerp.apos.tr_type, tr_type::LINEAR);
        assert_eq!(es.lerp.apos.tr_delta, [340.0, 0.0, 180.0]);
        // Pitched up ~21.9 degrees, then 120 back.
        let up = vectoangles([500.7, 0.0, 200.9])[0];
        assert!(up > 338.0 && up < 338.3);
        assert!(close(es.lerp.apos.tr_base[0], up - 120.0));
        assert_eq!(r.current_angles, es.lerp.apos.tr_base);

        let mut rng = Fixed::default();
        init_grenade_movement(
            &mut es, &mut r, [0.0; 3], [100.0, 0.0, 0.0], 0, tr_type::GRAVITY, Some(rotate_type::BLADE),
            &mut rng,
        );
        assert_eq!((rng.draws, es.lerp.apos.tr_delta), (0, [1500.0, 0.0, 0.0]));

        // No spin: stand still facing the throw.
        init_grenade_movement(&mut es, &mut r, [0.0; 3], [0.0, 100.0, 0.0], 0, tr_type::GRAVITY, None, &mut rng);
        assert_eq!(es.lerp.apos.tr_type, tr_type::STATIONARY);
        let b = es.lerp.apos.tr_base;
        assert!(close(b[0], 0.0) && close(b[1], 90.0) && b[2] == 0.0);
    }

    #[test]
    fn no_draw_delay_and_hide_checks() {
        assert_eq!(fire_grenade_no_draw_ms(0.0), 50);
        assert_eq!(fire_grenade_no_draw_ms(600.0), 50);
        assert_eq!(fire_grenade_no_draw_ms(900.0), 32);
        assert_eq!(fire_grenade_no_draw_ms(1200.0), 20);
        let mut es = EntityState::default();
        assert!(!missile_hidden(&es, 0, false));
        es.lerp.e_flags = EF_NODRAW;
        assert!(missile_hidden(&es, 0, false));
        assert_eq!(grenade_pos_tr_type(false, true, true), tr_type::GRAVITY);
        assert_eq!(grenade_pos_tr_type(true, true, false), tr_type::MOON_GRAVITY);
        assert_eq!(land_hit_time(1050, 1000, 0.5), 1025);
    }

    #[test]
    fn landing_flat_on_the_floor_snaps_to_it() {
        let w = LandWeapon::default();
        let mut rng = Fixed { random: 0.5, ..Default::default() };
        // Nose 30 degrees down, spinning: within 45 so it lies flat (0),
        // and the spin turns back damped by 0.3 * 0.5 + 0.85 = 1.0.
        let mut apos = spinning([30.0, 45.0, 0.0], [400.0, 0.0, 0.0]);
        let a = missile_land_angles(&mut apos, FLOOR, 0, false, false, w, &mut rng).unwrap();
        assert!(close(a[0], 0.0) && close(a[1], 45.0));
        assert!(close(apos.tr_delta[0], -400.0));
        assert_eq!((apos.tr_base, apos.tr_time), ([30.0, 45.0, 0.0], 0));
        // Upside down (pitch 170) lands on its back: surface + 180.
        let mut apos = spinning([170.0, 0.0, 0.0], [400.0, 0.0, 0.0]);
        let a = missile_land_angles(&mut apos, FLOOR, 0, true, false, w, &mut rng).unwrap();
        assert!(close(a[0], 180.0));
    }

    #[test]
    fn steep_landings_keep_or_ease_the_pitch() {
        let w = LandWeapon::default();
        let mut rng = Fixed { random: 0.0, ..Default::default() };
        // 60 off: a quarter of the way, 60 - 15 = 45.
        let mut apos = spinning([60.0, 0.0, 0.0], [400.0, 0.0, 0.0]);
        let a = missile_land_angles(&mut apos, FLOOR, 0, false, false, w, &mut rng).unwrap();
        assert!(close(a[0], 45.0));
        assert!(close(apos.tr_delta[0], -340.0));
        // 85 off: pitch kept, spin damped but not turned.
        let mut apos = spinning([85.0, 0.0, 0.0], [400.0, 0.0, 0.0]);
        let a = missile_land_angles(&mut apos, FLOOR, 0, false, false, w, &mut rng).unwrap();
        assert!(close(a[0], 85.0));
        assert!(close(apos.tr_delta[0], 340.0));
    }

    #[test]
    fn walls_jitter_the_spin_and_blades_flip() {
        let w = LandWeapon::default();
        let mut rng = Fixed { rand: 0x7f, ..Default::default() };
        let mut apos = spinning([10.0, 0.0, 0.0], [400.0, 0.0, 0.0]);
        missile_land_angles(&mut apos, WALL, 0, false, false, w, &mut rng).unwrap();
        // 400 + 64, folded into [0, 360).
        assert!(close(apos.tr_delta[0], 104.0));
        let blade = LandWeapon { rotate_type: rotate_type::BLADE, keeps_rolling: false };
        let mut apos = spinning([10.0, 0.0, 0.0], [1500.0, 0.0, 0.0]);
        let mut rng = Fixed::default();
        assert!(bounce_angles(&mut apos, FLOOR, [0.0, 0.0, 100.0], 0, 50, blade, &mut rng).unwrap());
        assert_eq!((apos.tr_delta[0], apos.tr_time, rng.draws), (-1500.0, 50, 0));
        // Skimming the surface is not a hit.
        assert!(!bounce_angles(&mut apos, FLOOR, [300.0, 0.0, 20.0], 0, 50, blade, &mut rng).unwrap());
        // A rolling grenade's spin is left alone.
        let rolling = LandWeapon { rotate_type: rotate_type::GRENADE, keeps_rolling: true };
        let mut apos = spinning([10.0, 0.0, 0.0], [400.0, 0.0, 0.0]);
        missile_land_angles(&mut apos, FLOOR, 0, false, false, rolling, &mut rng).unwrap();
        assert_eq!(apos.tr_delta[0], 400.0);
    }

    #[test]
    fn cylinders_stand_on_end_and_rest_picks_by_stickiness() {
        let cyl = LandWeapon { rotate_type: rotate_type::CYLINDER, keeps_rolling: false };
        let mut rng = Fixed::default();
        let mut apos = spinning([10.0, 30.0, 0.0], [0.0; 3]);
        let a = missile_land_angles(&mut apos, FLOOR, 0, true, false, cyl, &mut rng).unwrap();
        assert_eq!(a[2], 90.0);

        let slope = [-0.707_106_77, 0.0, 0.707_106_77];
        let apos0 = spinning([0.0, 0.0, 0.0], [0.0; 3]);
        let mut apos = apos0;
        let a = rest_angles(&mut apos, slope, 0, stickiness::GROUND, LandWeapon::default(), &mut rng).unwrap();
        assert!(close(a[0], 315.0) && close(a[1], 0.0) && close(a[2], 0.0));
        let mut apos = spinning([0.0, 90.0, 0.0], [0.0; 3]);
        let a = rest_angles(&mut apos, slope, 0, stickiness::GROUND_WITH_YAW, LandWeapon::default(), &mut rng)
            .unwrap();
        assert!(close(a[0], 0.0) && close(a[1], 90.0) && close(a[2], -45.0));

        assert!(comes_to_rest(stickiness::ALL, 500.0, 0.0, 10.0));
        assert!(comes_to_rest(stickiness::GROUND, 500.0, 0.9, 10.0));
        assert!(!comes_to_rest(stickiness::GROUND, 500.0, 0.5, 10.0));
        assert!(comes_to_rest(stickiness::NONE, 5.0, 0.9, 10.0));
        assert!(!comes_to_rest(stickiness::NONE, 50.0, 0.9, 10.0));
    }
}
