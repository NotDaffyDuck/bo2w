use super::*;

pub(super) fn entity_origin(ent: &EntitySnapshot, time: i32) -> Vec3 {
    Vec3::from_array(evaluate_trajectory(&ent.lerp.pos, time))
}

/// Horizontal unit vector 90 degrees to the left of `direction`, or +X when
/// `direction` is vertical or empty.
pub(super) fn flat_left(direction: Vec3) -> Vec3 {
    Vec3::new(-direction.y, direction.x, 0.0)
        .try_normalize()
        .unwrap_or(Vec3::X)
}

/// Pitch/yaw/roll that make the view face `direction`.
pub(super) fn facing_angles(direction: Vec3) -> [f32; 3] {
    let forward = direction.normalize_or_zero();
    let left = flat_left(forward);
    game_math::rows_to_euler([
        forward.to_array(),
        left.to_array(),
        forward.cross(left).to_array(),
    ])
}

pub(super) fn sweep(clip_in: &CollisionClipData, from: Vec3, to: Vec3, half: f32) -> SolidsTraceContact {
    clip_in.sweep_box(
        from.to_array(),
        to.to_array(),
        [-half; 3],
        [half; 3],
        CAMERA_CLIP_MASK,
    )
}

/// Where a camera wanting to be at `end` can actually stand when it starts at
/// `start`: out of any solid it starts in, then slid along whatever it hits.
pub(super) fn pull_back(solids: Option<&CollisionClipData>, begin: Vec3, end: Vec3) -> Vec3 {
    let Some(solids) = solids else {
        return end;
    };
    let (mut from, mut to) = (begin, end);
    let mut hit_in = sweep(solids, from, to, CAMERA_HALF_BOX);
    for _ in 0..3 {
        if !hit_in.began_inside {
            break;
        }
        let push = if hit_in.fraction == 1.0 {
            Vec3::Z
        } else {
            Vec3::from_array(hit_in.normal)
        };
        from += push * 20.0;
        hit_in = sweep(solids, from, to, CAMERA_HALF_BOX);
    }
    if hit_in.began_inside {
        return from;
    }
    for _ in 0..3 {
        if hit_in.fraction >= 1.0 {
            return to;
        }
        from = from.lerp(to, hit_in.fraction);
        let normal = Vec3::from_array(hit_in.normal);
        let left_over = to - from;
        to = from + left_over + normal * (0.1 - left_over.dot(normal));
        hit_in = sweep(solids, from, to, CAMERA_HALF_BOX);
    }
    from.lerp(to, hit_in.fraction)
}

/// Unit direction from `camera` that keeps both the victim and the followed
/// thing in view, favouring the victim.
pub(super) fn look_at_both(victim: Vec3, subject: Vec3, camera: Vec3) -> Vec3 {
    let to_victim = (victim - camera).normalize_or_zero();
    let to_subject = (subject - camera).normalize_or_zero();
    let middle = (to_victim + to_subject).normalize_or_zero();
    let agreement = middle.dot(to_victim);
    let mut aim = middle;
    if agreement < COS_AIM_FREE {
        aim = if agreement < COS_AIM_LIMIT {
            to_victim
        } else {
            let axis = to_victim.cross(to_subject).normalize_or_zero();
            let tilted = Quat::from_axis_angle(axis, 10.0_f32.to_radians()) * to_victim;
            let share = (agreement - COS_AIM_FREE) / (COS_AIM_LIMIT - COS_AIM_FREE);
            tilted.lerp(to_victim, share)
        };
    }
    if aim.z < AIM_Z_EASE_START {
        aim = if aim.z < AIM_Z_EASE_END {
            to_victim
        } else {
            let share = (aim.z - AIM_Z_EASE_END) / (AIM_Z_EASE_START - AIM_Z_EASE_END);
            to_victim.lerp(aim, share)
        };
    }
    aim.normalize_or_zero()
}

/// Which kill-cam mode a missile gets, from its weapon's guidance, class and
/// type numbers.
pub(super) fn mode_for_weapon(guidance: i32, class: i32, kind: i32) -> DeathReplayStyle {
    match (guidance, class, kind) {
        (3, _, _) => DeathReplayStyle::AntiTankView,
        (2, _, _) => DeathReplayStyle::RemoteDeviceView,
        (_, 7, _) => DeathReplayStyle::RocketView,
        (_, _, 2) => DeathReplayStyle::AltMissileView,
        _ => DeathReplayStyle::GuidedMissileView,
    }
}

/// Mode for a followed entity that is not a projectile; `None` for the kind
/// that has no kill cam.
pub(super) fn mode_for_entity_type(entity_type: i32) -> Option<DeathReplayStyle> {
    match entity_type {
        12 => None,
        11 => Some(DeathReplayStyle::TurretView),
        _ => Some(DeathReplayStyle::AirstrikeView),
    }
}

/// (height above, distance behind) the followed thing for the chase modes.
pub(super) fn chase_offsets(mode: DeathReplayStyle, resting_on_ground: bool) -> (f32, f32) {
    if resting_on_ground {
        return match mode {
            DeathReplayStyle::GuidedMissileView | DeathReplayStyle::AltMissileView => (15.0, 30.0),
            _ => (0.0, 0.0),
        };
    }
    match mode {
        DeathReplayStyle::RocketView => (10.0, 70.0),
        DeathReplayStyle::RemoteDeviceView => (60.0, 300.0),
        _ => (5.0, 35.0),
    }
}

pub(super) fn has_focus_distance(mode: DeathReplayStyle) -> bool {
    matches!(
        mode,
        DeathReplayStyle::AirstrikeView
            | DeathReplayStyle::GuidedMissileView
            | DeathReplayStyle::AltMissileView
            | DeathReplayStyle::RocketView
            | DeathReplayStyle::RemoteDeviceView
    )
}
