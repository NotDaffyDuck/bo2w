/// gm-16: sqrt(2), as its f32 bits.
pub(crate) const RADIUS_AREA_HALF_SCALE: f32 = f32::from_bits(0x3fb5_04f3);

/// gm-17: distance from `origin` to the nearest point of a box.
pub fn radius_damage_distance_to_aabb(origin: [f32; 3], center: [f32; 3], half: [f32; 3]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..3 {
        let mut axis = libm::fabsf(origin[i] - center[i]) - half[i];
        if axis < 0.0 {
            axis = 0.0;
        }
        sum += axis * axis;
    }
    libm::sqrtf(sum)
}

/// gm-16: radius below 1 counts as 1; the area query reaches radius * sqrt(2).
pub fn radius_damage_area_half_extent(radius: f32) -> f32 {
    let radius = if radius < 1.0 { 1.0 } else { radius };
    radius * RADIUS_AREA_HALF_SCALE
}

/// gm-17, gm-18: in range when dist² < radius²; linear falloff from inner to outer, then
/// truncated to an int.
pub fn radius_damage_amount(inner: f32, outer: f32, radius: f32, dist: f32, vis_scale: f32) -> i32 {
    let radius = if radius < 1.0 { 1.0 } else { radius };
    if !(dist * dist < radius * radius) || vis_scale <= 0.0 {
        return 0;
    }
    let falloff = (1.0 - dist / radius) * (inner - outer) + outer;
    (vis_scale * falloff) as i32
}

/// gm-19.
pub const G_CAN_DAMAGE_CONTENTS_MASK: u32 = 0x802013;

/// gm-23.
pub(crate) const G_CAN_DAMAGE_HALF_WIDTH: f32 = 15.0;

/// gm-25.
pub(crate) const G_CAN_DAMAGE_PARTIAL_SCALE: f32 = 0.333_333_34;

/// gm-21 to gm-23: the centre of the body and its four corners, seen side-on from the inflictor.
pub(crate) fn can_damage_player_sample_points(
    origin: [f32; 3],
    view_origin: [f32; 3],
    inflictor: [f32; 3],
) -> [[f32; 3]; 5] {
    let half_height = (view_origin[2] - origin[2]) * 0.5;
    let mid = [
        (view_origin[0] + origin[0]) * 0.5,
        (view_origin[1] + origin[1]) * 0.5,
        (origin[2] + view_origin[2]) * 0.5,
    ];
    let dx = inflictor[0] - origin[0];
    let dy = inflictor[1] - origin[1];
    let mut len = libm::sqrtf(dy * dy + dx * dx);
    if 0.0 <= -len {
        len = 1.0;
    }
    let inv = 1.0 / len;
    let side_x = -(dy * inv) * G_CAN_DAMAGE_HALF_WIDTH;
    let side_y = dx * inv * G_CAN_DAMAGE_HALF_WIDTH;
    [
        mid,
        [mid[0] + side_x, mid[1] + side_y, mid[2] + half_height],
        [mid[0] + side_x, mid[1] + side_y, mid[2] - half_height],
        [mid[0] - side_x, mid[1] - side_y, mid[2] + half_height],
        [mid[0] - side_x, mid[1] - side_y, mid[2] - half_height],
    ]
}

/// gm-25.
pub(crate) fn can_damage_hits_to_scale(hits: u32) -> f32 {
    if hits == 0 {
        0.0
    } else if hits > 3 {
        1.0
    } else {
        hits as f32 * G_CAN_DAMAGE_PARTIAL_SCALE
    }
}

/// gm-24: true when the point is inside the cone (or there is no cone).
fn in_cone(cone: Option<([f32; 3], f32)>, inflictor: [f32; 3], point: [f32; 3]) -> bool {
    let Some((dir, cos)) = cone else {
        return true;
    };
    if cos == -1.0 {
        return true;
    }
    let d = [
        point[0] - inflictor[0],
        point[1] - inflictor[1],
        point[2] - inflictor[2],
    ];
    let mut len = libm::sqrtf(d[1] * d[1] + d[0] * d[0] + d[2] * d[2]);
    if 0.0 <= -len {
        len = 1.0;
    }
    let inv = 1.0 / len;
    cos <= dir[1] * d[1] * inv + d[0] * inv * dir[0] + dir[2] * d[2] * inv
}

/// gm-21 to gm-25: how much of a player the inflictor sees. `origin` is the body origin,
/// `view_origin` the eye, `cone` an optional (direction, cos) limit, and `trace_passed`
/// sights from the inflictor to a point.
pub fn can_damage_player_vis_scale(
    origin: [f32; 3],
    view_origin: [f32; 3],
    inflictor: [f32; 3],
    cone: Option<([f32; 3], f32)>,
    mut trace_passed: impl FnMut([f32; 3], [f32; 3]) -> bool,
) -> f32 {
    let samples = can_damage_player_sample_points(origin, view_origin, inflictor);
    let mut hits = 0u32;
    for sample in samples {
        if in_cone(cone, inflictor, sample) && trace_passed(inflictor, sample) {
            hits += 1;
        }
    }
    can_damage_hits_to_scale(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_truncates() {
        // 1 - 50/100 = 0.5 → 0.5 * (100 - 1) + 1 = 50.5 → 50, where roundf gave 51.
        assert_eq!(radius_damage_amount(100.0, 1.0, 100.0, 50.0, 1.0), 50);
        assert_eq!(radius_damage_amount(100.0, 1.0, 100.0, 100.0, 1.0), 0);
    }

    #[test]
    fn partial_cover_scales_by_thirds() {
        assert_eq!(can_damage_hits_to_scale(0), 0.0);
        assert_eq!(can_damage_hits_to_scale(2), 2.0 * 0.333_333_34);
        assert_eq!(can_damage_hits_to_scale(3), 3.0 * 0.333_333_34);
        assert_eq!(can_damage_hits_to_scale(4), 1.0);
    }

    #[test]
    fn corners_sit_across_the_inflictor_line() {
        // Inflictor due +x: the side vector is +y, so the corners sit at y = ±15.
        let p = can_damage_player_sample_points([0.0; 3], [0.0, 0.0, 60.0], [100.0, 0.0, 0.0]);
        assert_eq!(p[0], [0.0, 0.0, 30.0]);
        assert_eq!(p[1], [0.0, 15.0, 60.0]);
        assert_eq!(p[2], [0.0, 15.0, 0.0]);
        assert_eq!(p[3], [0.0, -15.0, 60.0]);
        assert_eq!(p[4], [0.0, -15.0, 0.0]);
    }

    #[test]
    fn cone_skips_points_behind() {
        let seen = can_damage_player_vis_scale(
            [0.0; 3],
            [0.0, 0.0, 60.0],
            [100.0, 0.0, 30.0],
            Some(([1.0, 0.0, 0.0], 0.0)),
            |_, _| true,
        );
        assert_eq!(seen, 0.0);
    }
}
