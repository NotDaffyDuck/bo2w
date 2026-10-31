//! sim's own entity helpers: link bounds, parent links, and grenade and
//! missile trajectories, on BO2's types (`entity_t6`).

use entity_t6::missile::{
    LOB_TILT_START, LAND_EASE_PITCH_DEG, LAND_KEEP_PITCH_DEG, LAND_WALL_NORMAL_Z,
};
use entity_t6::flight::path_kind;
use entity_t6::{EntitySnapshot, Trajectory, evaluate_trajectory};
use game_math::{wrap_degrees, degree_gap, slope_pitch, dir_to_euler};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkBounds {
    pub centre: [f32; 3],

    pub half: [f32; 3],
}

pub fn link_entity_world_bounds(origin: [f32; 3], box_centre: [f32; 3], box_reach: [f32; 3]) -> LinkBounds {
    LinkBounds {
        centre: std::array::from_fn(|axis| origin[axis] + box_centre[axis]),
        half: box_reach.map(|extent| extent + 1.0),
    }
}

pub fn link_entity_needs_rotated_radius(snapped_angles: [f32; 3], box_reach: [f32; 3]) -> bool {
    const NONE: [f32; 3] = [0.0; 3];
    box_reach != NONE && snapped_angles != NONE
}

/// Turns a trajectory into a fixed pose at `base`: it stays put and carries no motion.
fn hold_still(traj: &mut Trajectory, base: [f32; 3]) {
    traj.base_point = base;
    traj.tr_type = path_kind::AT_REST;
    traj.began_at = 0;
    traj.span_ms = 0;
    traj.rate = [0.0; 3];
}

pub const BOUND_FRAME_UNTURNED: [[f32; 3]; 3] = {
    const X_AXIS: [f32; 3] = [1.0, 0.0, 0.0];
    const Y_AXIS: [f32; 3] = [0.0, 1.0, 0.0];
    const Z_AXIS: [f32; 3] = [0.0, 0.0, 1.0];
    [X_AXIS, Y_AXIS, Z_AXIS]
};

pub fn parent_link_pose(es: &mut EntitySnapshot, spot: [f32; 3], aim: [f32; 3]) {
    hold_still(&mut es.lerp.pos, spot);
    hold_still(&mut es.lerp.apos, aim);
    es.lerp.pos.tr_type = path_kind::BLEND;
    es.lerp.apos.tr_type = path_kind::BLEND;
}

pub fn parent_link_apply_local(
    host_spot: [f32; 3],
    host_facing: [f32; 3],
    offset_axis: [[f32; 3]; 3],
    offset_origin: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let parent_axis = game_math::euler_to_rows(host_facing);
    let (combined_axis, combined_origin) =
        game_math::frame_product(offset_axis, offset_origin, parent_axis, host_spot);
    (combined_origin, game_math::rows_to_euler(combined_axis))
}

pub fn entnum_matched(entnums: &[i32], slot_no: i32) -> bool {
    entnums.contains(&slot_no)
}

/// `G_FireGrenade`'s launch: each velocity component is cut down to a whole number.
pub fn clipped_shift(shift: [f32; 3]) -> [f32; 3] {
    shift.map(|component| component as i32 as f32)
}

/// A grenade's position trajectory: gravity pulls it from `launch_point`, thrown with velocity `heading`.
///
/// TODO(fact): BO2 does the whole launch in `entity_t6::missile::init_grenade_movement`
/// (moon gravity in Zombies, the rotate types); sim still builds the two
/// trajectories itself.
pub fn init_grenade_pos(launch_point: [f32; 3], heading: [f32; 3], clock_millis: i32) -> Trajectory {
    Trajectory {
        base_point: launch_point,
        rate: clipped_shift(heading),
        tr_type: path_kind::FALLING,
        began_at: clock_millis,
        span_ms: 0,
    }
}

/// A spinning grenade's angle trajectory: the facing tipped back by BO2's
/// pitch offset, turning at the given rates.
pub fn init_grenade_apos(heading: [f32; 3], clock_millis: i32, pitch_spin: f32, roll_spin: f32) -> Trajectory {
    let mut facing = dir_to_euler(heading);
    facing[0] = wrap_degrees(facing[0] - LOB_TILT_START);
    Trajectory {
        base_point: facing,
        rate: [pitch_spin, 0.0, roll_spin],
        tr_type: path_kind::STRAIGHT,
        began_at: clock_millis,
        span_ms: 0,
    }
}

/// A missile's angles: still, facing along `dir`.
pub fn fire_missile_apos(dir: [f32; 3]) -> Trajectory {
    let mut traj = Trajectory::default();
    hold_still(&mut traj, dir_to_euler(dir));
    traj
}

#[derive(Clone, Copy, Debug)]
pub struct LandingSpin {
    pub turn_path: Trajectory,
    pub face_dir: [f32; 3],
    pub impact_ms: i32,
    pub lay_flat: bool,
    pub spin_random: f32,
    pub wall_spin_extra: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandedSpin {
    pub turn: [f32; 3],
    pub turn_path: Trajectory,
}

/// A grenade's angles where it lands, and its spin after the bounce.
///
/// TODO(fact): BO2's `MissileLandAngles` is `entity_t6::missile::missile_land_angles`
/// (keep-rolling weapons, blade bounces, cylinder roll); this is the older
/// landing sim still uses, on BO2's thresholds.
pub fn landing_spin_of(landing: LandingSpin) -> LandedSpin {
    let LandingSpin { turn_path: mut spin, face_dir: normal, impact_ms, lay_flat, spin_random, wall_spin_extra } =
        landing;
    let mut aim = evaluate_trajectory(&spin, impact_ms);
    if normal[2] <= LAND_WALL_NORMAL_Z {
        if !lay_flat {
            spin.rate[0] = wrap_degrees(wall_spin_extra + spin.rate[0]);
        }
        return LandedSpin { turn: aim, turn_path: spin };
    }
    let ground_pitch = slope_pitch(aim[1], normal);
    let pitch_gap = degree_gap(ground_pitch, aim[0]);
    let gap_size = pitch_gap.abs();
    if !lay_flat {
        spin.base_point = aim;
        spin.began_at = impact_ms;
        let damping = spin_random * 0.3 + 0.85;
        let factor = if LAND_KEEP_PITCH_DEG <= gap_size { damping } else { -damping };
        spin.rate[0] *= factor;
    }
    let current_pitch = degree_gap(aim[0], 0.0);
    aim[0] = if lay_flat || LAND_EASE_PITCH_DEG > gap_size {
        let flat = if current_pitch.abs() <= 90.0 { ground_pitch } else { ground_pitch + 180.0 };
        wrap_degrees(flat)
    } else if LAND_KEEP_PITCH_DEG <= gap_size {
        wrap_degrees(current_pitch)
    } else {
        wrap_degrees(pitch_gap * 0.25 + current_pitch)
    };
    LandedSpin { turn: aim, turn_path: spin }
}
