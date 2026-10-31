//! 3x3 axes and axis-plus-origin frames. The sums run in the order BO2's compiled code adds them, which differs
//! between some entries; changing an order changes the last bit.

/// Three axis rows.
type Axes = [[f32; 3]; 3];

/// Length of a vector, its squares summed z, x, y.
#[inline]
pub fn vec3_norm(vec: [f32; 3]) -> f32 {
    libm::sqrtf(vec[2] * vec[2] + vec[0] * vec[0] + vec[1] * vec[1])
}

/// `a * b`. Entry (i, j) sums the third, first, then second products, except (0, 0), which sums third, second, first.
pub fn axis_product(a: Axes, b: Axes) -> Axes {
    let mut out = [[0.0; 3]; 3];
    for (row, out_row) in a.iter().zip(&mut out) {
        for (j, cell) in out_row.iter_mut().enumerate() {
            *cell = row[2] * b[2][j] + row[0] * b[0][j] + row[1] * b[1][j];
        }
    }
    out[0][0] = a[0][2] * b[2][0] + a[0][1] * b[1][0] + a[0][0] * b[0][0];
    out
}

/// The point `v` given in the frame (`axis`, `at_pos`), in world terms. x sums the z, x, y terms; y and z sum z, y, x.
pub fn frame_apply(point: [f32; 3], along: Axes, at_pos: [f32; 3]) -> [f32; 3] {
    let [px, py, pz] = point;
    [
        along[2][0] * pz + along[0][0] * px + along[1][0] * py + at_pos[0],
        along[2][1] * pz + along[1][1] * py + along[0][1] * px + at_pos[1],
        along[2][2] * pz + along[1][2] * py + along[0][2] * px + at_pos[2],
    ]
}

/// The frame `a` placed inside the frame `b`: axes multiplied, `a`'s origin carried into `b` (z, x, y order on every
/// row) plus `b`'s origin.
pub fn frame_product(a_axis: Axes, inner_pos: [f32; 3], outer_axis: Axes, outer_pos: [f32; 3]) -> (Axes, [f32; 3]) {
    let [x, y, z] = inner_pos;
    let origin: [f32; 3] =
        core::array::from_fn(|j| outer_axis[2][j] * z + outer_axis[0][j] * x + outer_axis[1][j] * y + outer_pos[j]);
    (axis_product(a_axis, outer_axis), origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURNED: Axes = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    const SAME: Axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn identity_changes_nothing() {
        assert_eq!(axis_product(TURNED, SAME), TURNED);
        assert_eq!(axis_product(SAME, TURNED), TURNED);
        assert_eq!(vec3_norm([3.0, 4.0, 12.0]), 13.0);
    }

    #[test]
    fn a_point_moves_into_the_frame() {
        assert_eq!(frame_apply([1.0, 0.0, 0.0], TURNED, [5.0, 5.0, 5.0]), [5.0, 6.0, 5.0]);
        let (axis, origin) = frame_product(SAME, [1.0, 0.0, 0.0], TURNED, [5.0, 5.0, 5.0]);
        assert_eq!((axis, origin), (TURNED, [5.0, 6.0, 5.0]));
    }
}
