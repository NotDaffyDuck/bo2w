//! sim's own entity helpers: link bounds, parent links, and grenade and
//! missile trajectories, on BO2's types (`entity_t6`).

use entity_t6::missile::{
    GRENADE_APOS_PITCH_OFFSET, LAND_EASE_PITCH_DEG, LAND_KEEP_PITCH_DEG, LAND_WALL_NORMAL_Z,
};
use entity_t6::trajectory::tr_type;
use entity_t6::{EntityState, Trajectory, evaluate_trajectory};
use game_math::{angle_normalize_360, angle_subtract, pitch_for_yaw_on_normal, vect_to_angles};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkBounds {
    pub mid: [f32; 3],

    pub half: [f32; 3],
}

pub fn link_entity_world_bounds(
    origin: [f32; 3],
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> LinkBounds {
    LinkBounds {
        mid: [
            origin[0] + box_mid[0],
            origin[1] + box_mid[1],
            origin[2] + box_mid[2],
        ],
        half: [box_half[0] + 1.0, box_half[1] + 1.0, box_half[2] + 1.0],
    }
}

pub fn link_entity_needs_rotated_radius(snapped_angles: [f32; 3], box_half: [f32; 3]) -> bool {
    box_half != [0.0, 0.0, 0.0] && snapped_angles != [0.0, 0.0, 0.0]
}

fn set_origin(es: &mut EntityState, origin: [f32; 3]) {
    es.lerp.pos.tr_base = origin;
    es.lerp.pos.tr_type = tr_type::STATIONARY;
    es.lerp.pos.tr_time = 0;
    es.lerp.pos.tr_duration = 0;
    es.lerp.pos.tr_delta = [0.0; 3];
}

fn set_angle(es: &mut EntityState, angles: [f32; 3]) {
    es.lerp.apos.tr_base = angles;
    es.lerp.apos.tr_type = tr_type::STATIONARY;
    es.lerp.apos.tr_time = 0;
    es.lerp.apos.tr_duration = 0;
    es.lerp.apos.tr_delta = [0.0; 3];
}

pub const PARENT_LINK_AXIS_IDENTITY: [[f32; 3]; 3] =
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn parent_link_pose(es: &mut EntityState, origin: [f32; 3], angles: [f32; 3]) {
    set_origin(es, origin);
    set_angle(es, angles);
    es.lerp.pos.tr_type = tr_type::INTERPOLATE;
    es.lerp.apos.tr_type = tr_type::INTERPOLATE;
}

pub fn parent_link_apply_local(
    world_origin: [f32; 3],
    world_angles: [f32; 3],
    local_axis: [[f32; 3]; 3],
    local_origin: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let parent_axis = game_math::angles_to_axis(world_angles);
    let (axis, origin) =
        game_math::matrix_multiply43(local_axis, local_origin, parent_axis, world_origin);
    (origin, game_math::axis_to_angles(axis))
}

pub fn entnum_matched(entnums: &[i32], s_number: i32) -> bool {
    let mut i = 0;
    while i < entnums.len() {
        if entnums[i] == s_number {
            return true;
        }
        i += 1;
    }
    false
}

/// `G_FireGrenade`'s launch: the stored velocity is cut to whole units.
pub fn truncated_tr_delta(delta: [f32; 3]) -> [f32; 3] {
    [delta[0] as i32 as f32, delta[1] as i32 as f32, delta[2] as i32 as f32]
}

/// A grenade's position trajectory: gravity from `start` at `dir`.
///
/// TODO(fact): BO2's whole launch is `entity_t6::missile::init_grenade_movement`
/// (moon gravity in Zombies, the rotate types); sim still builds the two
/// trajectories itself.
pub fn init_grenade_pos(start: [f32; 3], dir: [f32; 3], level_time_ms: i32) -> Trajectory {
    Trajectory {
        tr_time: level_time_ms,
        tr_type: tr_type::GRAVITY,
        tr_duration: 0,
        tr_delta: truncated_tr_delta(dir),
        tr_base: start,
    }
}

/// A spinning grenade's angle trajectory: the facing tipped back by BO2's
/// pitch offset, turning at the given rates.
pub fn init_grenade_apos(dir: [f32; 3], level_time_ms: i32, pitch_rate: f32, roll_rate: f32) -> Trajectory {
    let mut angles = vect_to_angles(dir);
    angles[0] = angle_normalize_360(angles[0] - GRENADE_APOS_PITCH_OFFSET);
    Trajectory {
        tr_time: level_time_ms,
        tr_type: tr_type::LINEAR,
        tr_duration: 0,
        tr_delta: [pitch_rate, 0.0, roll_rate],
        tr_base: angles,
    }
}

/// A missile's angles: still, facing along `dir`.
pub fn fire_missile_apos(dir: [f32; 3]) -> Trajectory {
    Trajectory {
        tr_time: 0,
        tr_type: tr_type::STATIONARY,
        tr_duration: 0,
        tr_delta: [0.0; 3],
        tr_base: vect_to_angles(dir),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MissileLandAnglesIn {
    pub apos: Trajectory,
    pub normal: [f32; 3],
    pub hit_time_ms: i32,
    pub force_align: bool,
    pub spin_random: f32,
    pub wall_spin_addend: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissileLandAnglesOut {
    pub angles: [f32; 3],
    pub apos: Trajectory,
}

/// A grenade's angles where it lands, and its spin after the bounce.
///
/// TODO(fact): BO2's `MissileLandAngles` is `entity_t6::missile::missile_land_angles`
/// (keep-rolling weapons, blade bounces, cylinder roll); this is the older
/// landing sim still uses, on BO2's thresholds.
pub fn missile_land_angles(input: MissileLandAnglesIn) -> MissileLandAnglesOut {
    let mut apos = input.apos;
    let mut angles = evaluate_trajectory(&apos, input.hit_time_ms);
    if input.normal[2] <= LAND_WALL_NORMAL_Z {
        if !input.force_align {
            apos.tr_delta[0] = angle_normalize_360(input.wall_spin_addend + apos.tr_delta[0]);
        }
        return MissileLandAnglesOut { angles, apos };
    }
    let surface_pitch = pitch_for_yaw_on_normal(angles[1], input.normal);
    let angle_delta = angle_subtract(surface_pitch, angles[0]);
    let abs_delta = angle_delta.abs();
    if !input.force_align {
        apos.tr_base = angles;
        apos.tr_time = input.hit_time_ms;
        let keep = input.spin_random * 0.3 + 0.85;
        apos.tr_delta[0] = if LAND_KEEP_PITCH_DEG <= abs_delta {
            keep * apos.tr_delta[0]
        } else {
            -(keep * apos.tr_delta[0])
        };
    }
    angles[0] = angle_subtract(angles[0], 0.0);
    if input.force_align || LAND_EASE_PITCH_DEG > abs_delta {
        angles[0] = if angles[0].abs() <= 90.0 {
            angle_normalize_360(surface_pitch)
        } else {
            angle_normalize_360(surface_pitch + 180.0)
        };
    } else if LAND_KEEP_PITCH_DEG <= abs_delta {
        angles[0] = angle_normalize_360(angles[0]);
    } else {
        angles[0] = angle_normalize_360(angle_delta * 0.25 + angles[0]);
    }
    MissileLandAnglesOut { angles, apos }
}
