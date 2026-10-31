/// gm-16: sqrt(2), as its f32 bits.
pub(crate) const RADIUS_AREA_HALF_SCALE: f32 = f32::from_bits(0x3fb5_04f3);

/// A radius below 1 is treated as 1 (gm-16, gm-17).
fn radius_floor_one(reach: f32) -> f32 {
    if reach < 1.0 { 1.0 } else { reach }
}

/// A vector length that is safe to divide by: zero (or negative) becomes 1.
fn length_for_division(len: f32) -> f32 {
    if 0.0 <= -len { 1.0 } else { len }
}

/// gm-17: distance from `from` to the nearest point of a box.
pub fn blast_gap_to_box(
    from: [f32; 3],
    box_centre: [f32; 3],
    half_size: [f32; 3],
) -> f32 {
    let squared = (0..3)
        .map(|k| {
            let outside = libm::fabsf(from[k] - box_centre[k]) - half_size[k];
            let gap = if outside < 0.0 { 0.0 } else { outside };
            gap * gap
        })
        .fold(0.0f32, |total, term| total + term);
    libm::sqrtf(squared)
}

/// gm-16: the area query reaches radius * sqrt(2), with the radius floored at 1.
pub fn radius_damage_area_half_extent(reach: f32) -> f32 {
    radius_floor_one(reach) * RADIUS_AREA_HALF_SCALE
}

/// gm-17, gm-18: in range when dist² < radius²; linear falloff from inner to outer, then
/// truncated to an int.
pub fn radius_damage_amount(inner: f32, outer: f32, blast_r: f32, dist: f32, seen_frac: f32) -> i32 {
    let reach = radius_floor_one(blast_r);
    let within = dist * dist < reach * reach;
    if !within || seen_frac <= 0.0 {
        return 0;
    }
    let blend = (1.0 - dist / reach) * (inner - outer) + outer;
    (seen_frac * blend) as i32
}

/// gm-19.
pub const BLAST_BLOCK_MASK: u32 = 0x802013;

/// gm-23.
pub(crate) const G_CAN_DAMAGE_HALF_WIDTH: f32 = 15.0;

/// gm-25.
pub(crate) const G_CAN_DAMAGE_PARTIAL_SCALE: f32 = 0.333_333_34;

/// gm-21 to gm-23: the centre of the body and its four corners, seen side-on from the inflictor.
pub(crate) fn can_damage_player_sample_points(
    torso_at: [f32; 3],
    eye_origin: [f32; 3],
    source: [f32; 3],
) -> [[f32; 3]; 5] {
    let half_height = (eye_origin[2] - torso_at[2]) * 0.5;
    let centre = [
        (eye_origin[0] + torso_at[0]) * 0.5,
        (eye_origin[1] + torso_at[1]) * 0.5,
        (torso_at[2] + eye_origin[2]) * 0.5,
    ];
    let to_x = source[0] - torso_at[0];
    let to_y = source[1] - torso_at[1];
    let flat_len = length_for_division(libm::sqrtf(to_y * to_y + to_x * to_x));
    let recip = 1.0 / flat_len;
    // The corners sit to either side of the line from the body towards the inflictor.
    let off_x = -(to_y * recip) * G_CAN_DAMAGE_HALF_WIDTH;
    let off_y = to_x * recip * G_CAN_DAMAGE_HALF_WIDTH;
    [
        centre,
        [centre[0] + off_x, centre[1] + off_y, centre[2] + half_height],
        [centre[0] + off_x, centre[1] + off_y, centre[2] - half_height],
        [centre[0] - off_x, centre[1] - off_y, centre[2] + half_height],
        [centre[0] - off_x, centre[1] - off_y, centre[2] - half_height],
    ]
}

/// gm-25.
pub(crate) fn can_damage_hits_to_scale(seen_points: u32) -> f32 {
    match seen_points {
        0 => 0.0,
        1..=3 => seen_points as f32 * G_CAN_DAMAGE_PARTIAL_SCALE,
        _ => 1.0,
    }
}

/// gm-24: true when the point is inside the cone (or there is no cone).
fn in_cone(cone: Option<([f32; 3], f32)>, source: [f32; 3], probe_at: [f32; 3]) -> bool {
    let Some((axis, cos_limit)) = cone else {
        return true;
    };
    if cos_limit == -1.0 {
        return true;
    }
    let offset = [probe_at[0] - source[0], probe_at[1] - source[1], probe_at[2] - source[2]];
    let dist = length_for_division(libm::sqrtf(
        offset[1] * offset[1] + offset[0] * offset[0] + offset[2] * offset[2],
    ));
    let recip = 1.0 / dist;
    let along = axis[1] * offset[1] * recip + offset[0] * recip * axis[0] + axis[2] * offset[2] * recip;
    cos_limit <= along
}

/// gm-21 to gm-25: how much of a player the inflictor sees. `body_at` is the body origin,
/// `eye_origin` the eye, `cone` an optional (direction, cos) limit, and `line_clear`
/// sights from the inflictor to a point.
pub fn can_damage_player_vis_scale(
    body_at: [f32; 3],
    eye_origin: [f32; 3],
    source: [f32; 3],
    cone: Option<([f32; 3], f32)>,
    mut line_clear: impl FnMut([f32; 3], [f32; 3]) -> bool,
) -> f32 {
    let seen = can_damage_player_sample_points(body_at, eye_origin, source)
        .into_iter()
        .filter(|&point| in_cone(cone, source, point) && line_clear(source, point))
        .count();
    can_damage_hits_to_scale(seen as u32)
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
