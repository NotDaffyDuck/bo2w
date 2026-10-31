//! Small vector moves BO2's walk and air share: PM_ProjectVelocity (lay a
//! move along the ground and keep its length), PM_ClipVelocity (take off the
//! part going into a plane) and Vec3Normalize.

/// Below this, the ground is too steep to lay a move along.
const SLIDE_UPRIGHT_TOLERANCE: f32 = 0.001;

/// How much of the push into a plane PM_ClipVelocity also keeps off it.
const CLIP_OVERBOUNCE: f32 = 0.001;

/// PM_ProjectVelocity: turns the move to run along the ground, keeping its
/// length. A move already going down a slope at least as steep is left
/// alone.
pub(crate) fn project_velocity(velocity: &mut [f32; 3], facing: &[f32; 3]) {
    let planar_mag_squared = velocity[0] * velocity[0] + velocity[1] * velocity[1];
    if libm::fabsf(facing[2]) < SLIDE_UPRIGHT_TOLERANCE || planar_mag_squared == 0.0 {
        return;
    }
    let updated_height = -(facing[1] * velocity[1] + facing[0] * velocity[0]) / facing[2];
    let original_length_sq = velocity[2] * velocity[2] + planar_mag_squared;
    let adjusted_length_sq = updated_height * updated_height + planar_mag_squared;
    let magnitude_ratio = libm::sqrtf(original_length_sq / adjusted_length_sq);
    if magnitude_ratio >= 1.0 && updated_height >= 0.0 && velocity[2] <= 0.0 {
        return;
    }
    velocity[0] *= magnitude_ratio;
    velocity[1] *= magnitude_ratio;
    velocity[2] = updated_height * magnitude_ratio;
}

/// The ground plane from a ground trace (its normal, stored as bits).
pub(crate) fn ground_normal(floor_probe: &[u32; 11]) -> [f32; 3] {
    [
        f32::from_bits(floor_probe[1]),
        f32::from_bits(floor_probe[2]),
        f32::from_bits(floor_probe[3]),
    ]
}

/// PM_ProjectVelocity onto this frame's ground.
pub(crate) fn clip_to_ground_plane(vector: &mut [f32; 3], floor_probe: &[u32; 11]) {
    project_velocity(vector, &ground_normal(floor_probe));
}

/// Sum of the component products, x first.
fn inner(lhs: &[f32; 3], rhs: &[f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

/// The move with the part pushing into the plane removed, plus a sliver
/// (`CLIP_OVERBOUNCE` of it) so it does not keep grazing the plane.
pub(crate) fn clip_velocity(input: &[f32; 3], facing: &[f32; 3]) -> [f32; 3] {
    let into_plane = inner(input, facing);
    let backoff = -(into_plane - into_plane.abs() * CLIP_OVERBOUNCE);
    [0, 1, 2].map(|k| input[k] + backoff * facing[k])
}

/// Rescales `vector` to unit length and hands back the length it had; a
/// zero vector is left as it is.
pub(crate) fn scale_to_unit(vector: &mut [f32; 3]) -> f32 {
    let old_length = length(vector);
    let inverse = if old_length <= 0.0 { 1.0 } else { 1.0 / old_length };
    *vector = vector.map(|component| component * inverse);
    old_length
}

pub(crate) fn length(vector: &[f32; 3]) -> f32 {
    libm::sqrtf(inner(vector, vector))
}

/// Direction and strength of the movement keys on level ground: the view's
/// forward and right axes are flattened and made unit length, mixed by the
/// key amounts, and the mix is normalised.
pub(crate) fn wish_dir(
    look_fwd: [f32; 3],
    look_side: [f32; 3],
    thrust_ahead: i8,
    thrust_lateral: i8,
) -> ([f32; 3], f32) {
    let level = |axis: [f32; 3]| {
        let mut flat = [axis[0], axis[1], 0.0];
        scale_to_unit(&mut flat);
        flat
    };
    let (ahead, side) = (level(look_fwd), level(look_side));
    let (f, r) = (f32::from(thrust_ahead), f32::from(thrust_lateral));
    let mut heading = [0, 1, 2].map(|k| r * side[k] + f * ahead[k]);
    let strength = scale_to_unit(&mut heading);
    (heading, strength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projecting_up_a_ramp_keeps_the_speed() {
        let n = 1.0 / libm::sqrtf(2.0);
        let mut v = [100.0, 0.0, 0.0];
        project_velocity(&mut v, &[-n, 0.0, n]);
        assert!((length(&v) - 100.0).abs() < 1e-3, "{v:?}");
        assert!(v[2] > 0.0);
    }

    #[test]
    fn steep_ground_and_no_flat_move_are_left_alone() {
        let mut v = [100.0, 0.0, -5.0];
        project_velocity(&mut v, &[1.0, 0.0, 0.0005]);
        assert_eq!(v, [100.0, 0.0, -5.0]);
        let mut v = [0.0, 0.0, -5.0];
        project_velocity(&mut v, &[0.0, 0.0, 1.0]);
        assert_eq!(v, [0.0, 0.0, -5.0]);
    }

    #[test]
    fn clip_takes_off_the_push_into_the_floor() {
        let out = clip_velocity(&[50.0, 0.0, -100.0], &[0.0, 0.0, 1.0]);
        assert_eq!(out[0], 50.0);
        assert!((out[2] - 0.1).abs() < 1e-4, "{out:?}");
    }

    #[test]
    fn wish_dir_mixes_the_keys_flat() {
        let (dir, speed) = wish_dir([1.0, 0.0, 0.5], [0.0, -1.0, 0.0], 127, 127);
        assert!((speed - libm::sqrtf(2.0) * 127.0).abs() < 1e-3);
        assert!((dir[0] - dir[1].abs()).abs() < 1e-6 && dir[2] == 0.0);
    }
}
