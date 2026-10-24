//! Small vector moves BO2's walk and air share: PM_ProjectVelocity (lay a
//! move along the ground and keep its length), PM_ClipVelocity (take off the
//! part going into a plane) and Vec3Normalize.

/// Below this, the ground is too steep to lay a move along.
const PROJECT_NORMAL_Z_EPSILON: f32 = 0.001;

/// How much of the push into a plane PM_ClipVelocity also keeps off it.
const CLIP_OVERBOUNCE: f32 = 0.001;

/// PM_ProjectVelocity: turns the move to run along the ground, keeping its
/// length. A move already going down a slope at least as steep is left
/// alone.
pub(crate) fn project_velocity(velocity: &mut [f32; 3], normal: &[f32; 3]) {
    let length_sq_2d = velocity[0] * velocity[0] + velocity[1] * velocity[1];
    if libm::fabsf(normal[2]) < PROJECT_NORMAL_Z_EPSILON || length_sq_2d == 0.0 {
        return;
    }
    let new_z = -(normal[1] * velocity[1] + normal[0] * velocity[0]) / normal[2];
    let original_length_sq = velocity[2] * velocity[2] + length_sq_2d;
    let adjusted_length_sq = new_z * new_z + length_sq_2d;
    let length_scale = libm::sqrtf(original_length_sq / adjusted_length_sq);
    if length_scale >= 1.0 && new_z >= 0.0 && velocity[2] <= 0.0 {
        return;
    }
    velocity[0] *= length_scale;
    velocity[1] *= length_scale;
    velocity[2] = new_z * length_scale;
}

/// The ground plane from a ground trace (its normal, stored as bits).
pub(crate) fn ground_normal(ground_trace: &[u32; 11]) -> [f32; 3] {
    [
        f32::from_bits(ground_trace[1]),
        f32::from_bits(ground_trace[2]),
        f32::from_bits(ground_trace[3]),
    ]
}

/// PM_ProjectVelocity onto this frame's ground.
pub(crate) fn clip_to_ground_plane(vector: &mut [f32; 3], ground_trace: &[u32; 11]) {
    project_velocity(vector, &ground_normal(ground_trace));
}

/// PM_ClipVelocity: the move with its push into the plane taken off.
pub(crate) fn clip_velocity(input: &[f32; 3], normal: &[f32; 3]) -> [f32; 3] {
    let dot = input[0] * normal[0] + input[1] * normal[1] + input[2] * normal[2];
    let backoff = -(dot - dot.abs() * CLIP_OVERBOUNCE);
    [
        input[0] + backoff * normal[0],
        input[1] + backoff * normal[1],
        input[2] + backoff * normal[2],
    ]
}

/// Vec3Normalize: scales the vector to length 1 and returns its old length
/// (a zero vector stays zero).
pub(crate) fn normalize(vector: &mut [f32; 3]) -> f32 {
    let length = length(vector);
    let divisor = if length <= 0.0 { 1.0 } else { length };
    let scale = 1.0 / divisor;
    vector[0] *= scale;
    vector[1] *= scale;
    vector[2] *= scale;
    length
}

pub(crate) fn length(vector: &[f32; 3]) -> f32 {
    libm::sqrtf(vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2])
}

/// The keys' push along the ground: forward and right flattened, the keys
/// mixed in, then made length 1. Returns the direction and the push's size.
pub(crate) fn wish_dir(
    forward: [f32; 3],
    right: [f32; 3],
    forwardmove: i8,
    rightmove: i8,
) -> ([f32; 3], f32) {
    let mut forward = [forward[0], forward[1], 0.0];
    let mut right = [right[0], right[1], 0.0];
    normalize(&mut forward);
    normalize(&mut right);
    let (f, r) = (forwardmove as f32, rightmove as f32);
    let mut wishdir = [
        r * right[0] + f * forward[0],
        r * right[1] + f * forward[1],
        r * right[2] + f * forward[2],
    ];
    let wishspeed = normalize(&mut wishdir);
    (wishdir, wishspeed)
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
