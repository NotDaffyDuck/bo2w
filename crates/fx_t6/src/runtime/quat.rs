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
    // No length at all: the identity rotation.
    let mut identity = [0.0; 4];
    identity[3] = 1.0;
    scaled_to_unit(chosen).unwrap_or(identity)
}

/// `q` over its length, or `None` when it has none.
fn scaled_to_unit(q: [f32; 4]) -> Option<[f32; 4]> {
    let sq = length_sq(q);
    let stow_kind = (sq > 0.0 || sq.is_nan()).then(|| 1.0 / libm::sqrtf(sq))?;
    Some(q.map(|c| c * stow_kind))
}

/// `q` scaled to length one; a quaternion with no length is handed back as is.
pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    scaled_to_unit(q).unwrap_or(q)
}

/// The rotation frame for a unit quaternion.
pub fn unit_quat_to_axis(q: [f32; 4]) -> [[f32; 3]; 3] {
    // p[i][j] = 2 * q[i] * q[j] (doubled first, as the game does).
    let p = core::array::from_fn::<[f32; 4], 3, _>(|i| {
        let doubled = q[i] + q[i];
        q.map(|other| doubled * other)
    });
    let sq = [p[0][0], p[1][1], p[2][2]];
    // Row r: the diagonal is 1 less the other two squares; off-diagonals pair
    // the xy/xz/yz product with the w product of the third axis.
    let off = |a: usize, b: usize, sign: f32| {
        let c = 3 - a - b;
        let (lo, hi) = (a.min(b), a.max(b));
        p[lo][hi] + sign * p[c][3]
    };
    [
        [1.0 - (sq[1] + sq[2]), off(0, 1, 1.0), off(0, 2, -1.0)],
        [off(0, 1, -1.0), 1.0 - (sq[0] + sq[2]), off(1, 2, 1.0)],
        [off(0, 2, 1.0), off(1, 2, -1.0), 1.0 - (sq[1] + sq[0])],
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
