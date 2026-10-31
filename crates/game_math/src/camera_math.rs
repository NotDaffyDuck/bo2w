//! 4x4 view and projection joint_transforms (row vectors, same in legacy and BO2): a viewer's
//! axis matrix, the finite perspective, and view-projection with its inverse.

/// A 4x4 matrix, row after row.
type Mat44 = [f32; 16];

/// A unit vector at right angles to `along`, or zero when `along` leaves nothing to build from.
#[must_use]
pub fn perpendicular_vector(along: [f32; 3]) -> [f32; 3] {
    let squares = [along[0] * along[0], along[1] * along[1], along[2] * along[2]];
    // Start from the world axis `along` leans on least.
    let mut weakest = usize::from(squares[1] < squares[0]);
    if squares[2] < squares[weakest] {
        weakest = 2;
    }
    let scale = -along[weakest];
    let mut p = [scale * along[0], scale * along[1], scale * along[2]];
    p[weakest] += 1.0;
    let length_sq = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
    if length_sq <= 1e-12 {
        return [0.0; 3];
    }
    let inv_length = 1.0 / libm::sqrtf(length_sq);
    [p[0] * inv_length, p[1] * inv_length, p[2] * inv_length]
}

/// Perspective with a far plane; `None` when near and far coincide.
#[must_use]
pub fn setup_finite_projection_matrix(
    half_fov_tan_x: f32,
    half_fov_tan_y: f32,
    near_plane: f32,
    far_plane: f32,
) -> Option<Mat44> {
    if near_plane == far_plane {
        return None;
    }
    let depth_span = near_plane - far_plane;
    let mut proj = [0.0f32; 16];
    proj[0] = 1.0 / half_fov_tan_x;
    proj[5] = 1.0 / half_fov_tan_y;
    proj[10] = -far_plane / depth_span;
    proj[11] = 1.0;
    proj[14] = (far_plane * near_plane) / depth_span;
    Some(proj)
}

/// Turns the viewer's forward/left/up rows into the engine's x-right, y-up, z-forward matrix.
#[must_use]
pub fn matrix_for_viewer(along: [[f32; 3]; 3]) -> Mat44 {
    let [forward, left, up] = along;
    let mut view = [0.0f32; 16];
    for k in 0..3 {
        view[4 * k] = -left[k];
        view[4 * k + 1] = up[k];
        view[4 * k + 2] = forward[k];
    }
    view[15] = 1.0;
    view
}

/// `lhs` times `rhs` for row-vector joint_transforms. Each sum keeps its own term order so results stay bit-identical.
#[must_use]
pub fn matrix_multiply44(lhs: &Mat44, rhs: &Mat44) -> Mat44 {
    let mut prod = [0.0f32; 16];
    prod[0] = rhs[12] * lhs[3] + rhs[8] * lhs[2] + lhs[1] * rhs[4] + lhs[0] * rhs[0];
    prod[1] = lhs[3] * rhs[13] + rhs[9] * lhs[2] + lhs[0] * rhs[1] + rhs[5] * lhs[1];
    prod[2] = lhs[3] * rhs[14] + lhs[2] * rhs[10] + rhs[2] * lhs[0] + rhs[6] * lhs[1];
    prod[3] = lhs[3] * rhs[15] + rhs[11] * lhs[2] + lhs[0] * rhs[3] + rhs[7] * lhs[1];
    prod[4] = rhs[12] * lhs[7] + rhs[8] * lhs[6] + lhs[4] * rhs[0] + lhs[5] * rhs[4];
    prod[5] = lhs[7] * rhs[13] + rhs[9] * lhs[6] + rhs[5] * lhs[5] + lhs[4] * rhs[1];
    prod[6] = rhs[14] * lhs[7] + lhs[6] * rhs[10] + lhs[4] * rhs[2] + lhs[5] * rhs[6];
    prod[7] = lhs[7] * rhs[15] + rhs[11] * lhs[6] + rhs[7] * lhs[5] + lhs[4] * rhs[3];
    prod[8] = rhs[12] * lhs[11] + rhs[8] * lhs[10] + lhs[8] * rhs[0] + lhs[9] * rhs[4];
    prod[9] = lhs[11] * rhs[13] + rhs[9] * lhs[10] + rhs[5] * lhs[9] + lhs[8] * rhs[1];
    prod[10] = rhs[14] * lhs[11] + lhs[10] * rhs[10] + lhs[8] * rhs[2] + lhs[9] * rhs[6];
    prod[11] = lhs[11] * rhs[15] + rhs[11] * lhs[10] + rhs[7] * lhs[9] + lhs[8] * rhs[3];
    prod[12] = rhs[12] * lhs[15] + lhs[14] * rhs[8] + lhs[13] * rhs[4] + rhs[0] * lhs[12];
    prod[13] = lhs[15] * rhs[13] + lhs[14] * rhs[9] + lhs[12] * rhs[1] + rhs[5] * lhs[13];
    prod[14] = rhs[14] * lhs[15] + lhs[14] * rhs[10] + rhs[6] * lhs[13] + rhs[2] * lhs[12];
    prod[15] = lhs[15] * rhs[15] + lhs[14] * rhs[11] + lhs[12] * rhs[3] + rhs[7] * lhs[13];
    prod
}

/// View-projection, and its inverse moved back to world space by `eye_pos`.
#[must_use]
pub fn compose_view_projection(
    view_matrix: &Mat44,
    proj_mat: &Mat44,
    eye_pos: [f32; 3],
) -> Option<(Mat44, Mat44)> {
    let view_proj = matrix_multiply44(view_matrix, proj_mat);
    let inverse = matrix_inverse44(&view_proj)?;
    let mut shift = IDENTITY_4X4;
    shift[12..15].copy_from_slice(&eye_pos);
    Some((view_proj, matrix_multiply44(&inverse, &shift)))
}

const IDENTITY_4X4: Mat44 = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// Gauss-Jordan elimination with partial pivoting on `[m | I]`; `None` for a singular matrix.
fn matrix_inverse44(m: &Mat44) -> Option<Mat44> {
    let mut aug = [[0.0f32; 8]; 4];
    for (row, line) in aug.iter_mut().enumerate() {
        line[..4].copy_from_slice(&m[row * 4..row * 4 + 4]);
        line[4 + row] = 1.0;
    }
    for pivot_col in 0..4 {
        let mut pivot_row = pivot_col;
        let mut largest = libm::fabsf(aug[pivot_col][pivot_col]);
        for candidate in (pivot_col + 1)..4 {
            let magnitude = libm::fabsf(aug[candidate][pivot_col]);
            if magnitude > largest {
                largest = magnitude;
                pivot_row = candidate;
            }
        }
        if largest <= 0.0 {
            return None;
        }
        if pivot_row != pivot_col {
            aug.swap(pivot_col, pivot_row);
        }
        let pivot_recip = 1.0 / aug[pivot_col][pivot_col];
        for cell in aug[pivot_col].iter_mut() {
            *cell *= pivot_recip;
        }
        let pivot_line = aug[pivot_col];
        for (row, line) in aug.iter_mut().enumerate() {
            if row == pivot_col {
                continue;
            }
            let factor = line[pivot_col];
            for (cell, &pivot_cell) in line.iter_mut().zip(pivot_line.iter()) {
                *cell -= factor * pivot_cell;
            }
        }
    }
    let mut inverse = [0.0f32; 16];
    for (row, line) in aug.iter().enumerate() {
        inverse[row * 4..row * 4 + 4].copy_from_slice(&line[4..]);
    }
    Some(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverting_view_projection_gives_identity_when_multiplied_back() {
        let view = matrix_for_viewer([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let proj = setup_finite_projection_matrix(1.0, 1.0, 1.0, 512.0).unwrap();
        let (view_proj, _) = compose_view_projection(&view, &proj, [0.0; 3]).unwrap();
        let inverse = matrix_inverse44(&view_proj).unwrap();
        let product = matrix_multiply44(&view_proj, &inverse);
        for (idx, value) in product.iter().enumerate() {
            assert!((value - IDENTITY_4X4[idx]).abs() < 1e-4, "{idx} {value}");
        }
    }

    #[test]
    fn perpendicular_result_is_normalized_and_at_right_angles() {
        let result = perpendicular_vector([0.0, 0.0, -1.0]);
        assert!(result[2].abs() < 1e-6);
        assert!(((result[0] * result[0] + result[1] * result[1]) - 1.0).abs() < 1e-6);
    }
}
