//! 3x3 axes and axis-plus-origin frames. The sums run in the order BO2's compiled code adds them, which differs
//! between some entries; changing an order changes the last bit.

type Axis = [[f32; 3]; 3];

/// Length of a vector, its squares summed z, x, y.
#[inline]
pub fn vec3_length(v: [f32; 3]) -> f32 {
    libm::sqrtf(v[2] * v[2] + v[0] * v[0] + v[1] * v[1])
}

/// `a * b`. Entry (i, j) sums the third, first, then second products, except (0, 0), which sums third, second, first.
pub fn matrix_multiply(a: Axis, b: Axis) -> Axis {
    let mut out = [[0.0; 3]; 3];
    for (row, out_row) in a.iter().zip(&mut out) {
        for (j, cell) in out_row.iter_mut().enumerate() {
            *cell = row[2] * b[2][j] + row[0] * b[0][j] + row[1] * b[1][j];
        }
    }
    out[0][0] = a[0][2] * b[2][0] + a[0][1] * b[1][0] + a[0][0] * b[0][0];
    out
}

/// The point `v` given in the frame (`axis`, `origin`), in world terms. x sums the z, x, y terms; y and z sum z, y, x.
pub fn matrix_transform_vector43(v: [f32; 3], axis: Axis, origin: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = v;
    [
        axis[2][0] * z + axis[0][0] * x + axis[1][0] * y + origin[0],
        axis[2][1] * z + axis[1][1] * y + axis[0][1] * x + origin[1],
        axis[2][2] * z + axis[1][2] * y + axis[0][2] * x + origin[2],
    ]
}

/// The frame `a` placed inside the frame `b`: axes multiplied, `a`'s origin carried into `b` (z, x, y order on every
/// row) plus `b`'s origin.
pub fn matrix_multiply43(a_axis: Axis, a_origin: [f32; 3], b_axis: Axis, b_origin: [f32; 3]) -> (Axis, [f32; 3]) {
    let [x, y, z] = a_origin;
    let origin: [f32; 3] =
        core::array::from_fn(|j| b_axis[2][j] * z + b_axis[0][j] * x + b_axis[1][j] * y + b_origin[j]);
    (matrix_multiply(a_axis, b_axis), origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURNED: Axis = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    const SAME: Axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn identity_changes_nothing() {
        assert_eq!(matrix_multiply(TURNED, SAME), TURNED);
        assert_eq!(matrix_multiply(SAME, TURNED), TURNED);
        assert_eq!(vec3_length([3.0, 4.0, 12.0]), 13.0);
    }

    #[test]
    fn a_point_moves_into_the_frame() {
        assert_eq!(matrix_transform_vector43([1.0, 0.0, 0.0], TURNED, [5.0, 5.0, 5.0]), [5.0, 6.0, 5.0]);
        let (axis, origin) = matrix_multiply43(SAME, [1.0, 0.0, 0.0], TURNED, [5.0, 5.0, 5.0]);
        assert_eq!((axis, origin), (TURNED, [5.0, 6.0, 5.0]));
    }
}
