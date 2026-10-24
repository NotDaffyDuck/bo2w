//! 4x4 view and projection matrices (row vectors, same in legacy and BO2): a viewer's
//! axis matrix, the finite perspective, and view-projection with its inverse.

#[must_use]
pub fn perpendicular_vector(v: [f32; 3]) -> [f32; 3] {
    let sq = [v[0] * v[0], v[1] * v[1], v[2] * v[2]];
    let mut axis = usize::from(sq[1] < sq[0]);
    if sq[2] < sq[axis] {
        axis = 2;
    }
    let neg = -v[axis];
    let mut out = [neg * v[0], neg * v[1], neg * v[2]];
    out[axis] += 1.0;
    let len_sq = out[0] * out[0] + out[1] * out[1] + out[2] * out[2];
    if len_sq <= 1e-12 {
        return [0.0; 3];
    }
    let inv = 1.0 / libm::sqrtf(len_sq);
    [out[0] * inv, out[1] * inv, out[2] * inv]
}

#[must_use]
pub fn setup_finite_projection_matrix(
    tan_half_x: f32,
    tan_half_y: f32,
    z_near: f32,
    z_far: f32,
) -> Option<[f32; 16]> {
    if z_near == z_far {
        return None;
    }
    let den = z_near - z_far;
    let mut m = [0.0f32; 16];
    m[0] = 1.0 / tan_half_x;
    m[5] = 1.0 / tan_half_y;
    m[10] = -z_far / den;
    m[11] = 1.0;
    m[14] = (z_far * z_near) / den;
    Some(m)
}

#[must_use]
pub fn matrix_for_viewer(axis: [[f32; 3]; 3]) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    m[0] = -axis[1][0];
    m[1] = axis[2][0];
    m[2] = axis[0][0];
    m[4] = -axis[1][1];
    m[5] = axis[2][1];
    m[6] = axis[0][1];
    m[8] = -axis[1][2];
    m[9] = axis[2][2];
    m[10] = axis[0][2];
    m[15] = 1.0;
    m
}

#[must_use]
pub fn matrix_multiply44(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0f32; 16];
    o[0] = b[12] * a[3] + b[8] * a[2] + a[1] * b[4] + a[0] * b[0];
    o[1] = a[3] * b[13] + b[9] * a[2] + a[0] * b[1] + b[5] * a[1];
    o[2] = a[3] * b[14] + a[2] * b[10] + b[2] * a[0] + b[6] * a[1];
    o[3] = a[3] * b[15] + b[11] * a[2] + a[0] * b[3] + b[7] * a[1];
    o[4] = b[12] * a[7] + b[8] * a[6] + a[4] * b[0] + a[5] * b[4];
    o[5] = a[7] * b[13] + b[9] * a[6] + b[5] * a[5] + a[4] * b[1];
    o[6] = b[14] * a[7] + a[6] * b[10] + a[4] * b[2] + a[5] * b[6];
    o[7] = a[7] * b[15] + b[11] * a[6] + b[7] * a[5] + a[4] * b[3];
    o[8] = b[12] * a[11] + b[8] * a[10] + a[8] * b[0] + a[9] * b[4];
    o[9] = a[11] * b[13] + b[9] * a[10] + b[5] * a[9] + a[8] * b[1];
    o[10] = b[14] * a[11] + a[10] * b[10] + a[8] * b[2] + a[9] * b[6];
    o[11] = a[11] * b[15] + b[11] * a[10] + b[7] * a[9] + a[8] * b[3];
    o[12] = b[12] * a[15] + a[14] * b[8] + a[13] * b[4] + b[0] * a[12];
    o[13] = a[15] * b[13] + a[14] * b[9] + a[12] * b[1] + b[5] * a[13];
    o[14] = b[14] * a[15] + a[14] * b[10] + b[6] * a[13] + b[2] * a[12];
    o[15] = a[15] * b[15] + a[14] * b[11] + a[12] * b[3] + b[7] * a[13];
    o
}

/// View-projection, and its inverse moved back to world space by `origin`.
#[must_use]
pub fn compose_view_projection(
    view: &[f32; 16],
    proj: &[f32; 16],
    origin: [f32; 3],
) -> Option<([f32; 16], [f32; 16])> {
    let vp = matrix_multiply44(view, proj);
    let inv = matrix_inverse44(&vp)?;
    let mut t = IDENTITY44;
    t[12] = origin[0];
    t[13] = origin[1];
    t[14] = origin[2];
    Some((vp, matrix_multiply44(&inv, &t)))
}

const IDENTITY44: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn matrix_inverse44(m: &[f32; 16]) -> Option<[f32; 16]> {
    let mut a = [[0.0f32; 8]; 4];
    for r in 0..4 {
        for c in 0..4 {
            a[r][c] = m[r * 4 + c];
            a[r][c + 4] = if r == c { 1.0 } else { 0.0 };
        }
    }
    for col in 0..4 {
        let mut pivot = col;
        let mut best = libm::fabsf(a[col][col]);
        for r in (col + 1)..4 {
            let v = libm::fabsf(a[r][col]);
            if v > best {
                best = v;
                pivot = r;
            }
        }
        if best == 0.0 {
            return None;
        }
        if pivot != col {
            a.swap(col, pivot);
        }
        let inv_p = 1.0 / a[col][col];
        for c in 0..8 {
            a[col][c] *= inv_p;
        }
        for r in 0..4 {
            if r == col {
                continue;
            }
            let f = a[r][col];
            for c in 0..8 {
                a[r][c] -= f * a[col][c];
            }
        }
    }
    let mut out = [0.0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = a[r][c + 4];
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_projection_inverse_round_trips() {
        let view = matrix_for_viewer([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let proj = setup_finite_projection_matrix(1.0, 1.0, 1.0, 512.0).unwrap();
        let (vp, _) = compose_view_projection(&view, &proj, [0.0; 3]).unwrap();
        let inv = matrix_inverse44(&vp).unwrap();
        let id = matrix_multiply44(&vp, &inv);
        for (i, v) in id.iter().enumerate() {
            assert!((v - IDENTITY44[i]).abs() < 1e-4, "{i} {v}");
        }
    }

    #[test]
    fn perpendicular_is_unit_and_orthogonal() {
        let p = perpendicular_vector([0.0, 0.0, -1.0]);
        assert!(p[2].abs() < 1e-6);
        assert!(((p[0] * p[0] + p[1] * p[1]) - 1.0).abs() < 1e-6);
    }
}
