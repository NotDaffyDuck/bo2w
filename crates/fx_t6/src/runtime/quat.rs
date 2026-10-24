//! Turning a rotation frame into a quaternion and back.
//!
//! Quaternions are `[x, y, z, w]`; a frame is three rows `[forward, right,
//! up]`.

fn length_sq(q: [f32; 4]) -> f32 {
    q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]
}

/// The quaternion for a rotation frame.
///
/// Four candidate quaternions each lead with a different diagonal term; the
/// first whose squared length reaches one is the well-conditioned one and is
/// normalized. If none does, the last is used, and a frame that gives it no
/// length at all comes out as the identity.
pub fn axis_to_quat(axis: [[f32; 3]; 3]) -> [f32; 4] {
    let [[m00, m01, m02], [m10, m11, m12], [m20, m21, m22]] = axis;

    let by_w = [m12 - m21, m20 - m02, m01 - m10, m11 + m00 + m22 + 1.0];
    let by_z = [m20 + m02, m21 + m12, (m22 - m11 - m00) + 1.0, by_w[2]];
    let by_x = [(m00 - m11 - m22) + 1.0, m10 + m01, by_z[0], by_w[0]];
    let by_y = [by_x[1], (m11 - m00 - m22) + 1.0, by_z[1], by_w[1]];

    let chosen = [by_w, by_z, by_x]
        .into_iter()
        .find(|candidate| length_sq(*candidate) >= 1.0)
        .unwrap_or(by_y);
    let chosen_sq = length_sq(chosen);
    if chosen_sq <= 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let scale = 1.0 / libm::sqrtf(chosen_sq);
    [
        chosen[0] * scale,
        chosen[1] * scale,
        chosen[2] * scale,
        chosen[3] * scale,
    ]
}

/// `q` scaled to length one; a quaternion with no length is handed back as is.
pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let sq = length_sq(q);
    if sq <= 0.0 {
        return q;
    }
    let scale = 1.0 / libm::sqrtf(sq);
    [q[0] * scale, q[1] * scale, q[2] * scale, q[3] * scale]
}

/// The rotation frame for a unit quaternion.
pub fn unit_quat_to_axis(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, yy, zz) = (x2 * x, y2 * y, z2 * z);
    let (xy, xz, xw) = (x2 * y, x2 * z, x2 * w);
    let (yz, yw, zw) = (y2 * z, y2 * w, z2 * w);
    [
        [1.0 - (yy + zz), xy + zw, xz - yw],
        [xy - zw, 1.0 - (xx + zz), xw + yz],
        [xz + yw, yz - xw, 1.0 - (yy + xx)],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn the_identity_frame_is_the_identity_quaternion() {
        assert_eq!(axis_to_quat(IDENTITY), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_frame_survives_the_round_trip_through_a_quaternion() {
        let frames = [
            IDENTITY,
            // A half turn about z: the w candidate collapses, the x one is used.
            [[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
            [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
        ];
        for frame in frames {
            let back = unit_quat_to_axis(axis_to_quat(frame));
            for (row, expect) in back.iter().zip(frame.iter()) {
                for (got, want) in row.iter().zip(expect.iter()) {
                    assert!((got - want).abs() < 1e-5, "{back:?} vs {frame:?}");
                }
            }
        }
    }

    #[test]
    fn normalizing_leaves_a_zero_quaternion_alone() {
        assert_eq!(quat_normalize([0.0; 4]), [0.0; 4]);
        let q = quat_normalize([0.0, 0.0, 3.0, 4.0]);
        assert!((length_sq(q) - 1.0).abs() < 1e-6);
    }
}
