//! Matrix and float helpers for feeding constants to shaders as raw bits.

use glam::{Mat4, Vec3};

/// The bit patterns of four floats, so constants can be compared and hashed
/// exactly.
pub fn float4_bits(value: [f32; 4]) -> [u32; 4] {
    value.map(f32::to_bits)
}

/// The rows of `m` as bit patterns. A shader reads a column-major matrix row by
/// row, so what is returned is each row of `m` as four consecutive values.
pub fn code_transpose_matrix_row4(m: Mat4) -> [[u32; 4]; 4] {
    let columns = m.to_cols_array_2d();
    std::array::from_fn(|row| float4_bits(columns.map(|column| column[row])))
}

/// [`code_transpose_matrix_row4`] as a `Vec`.
pub fn code_transpose_matrix_rows(m: Mat4) -> Vec<[u32; 4]> {
    Vec::from(code_transpose_matrix_row4(m))
}

/// A world-to-clip matrix that takes positions relative to `view_origin`
/// instead of absolute ones.
pub fn camera_relative_view_projection(clip_from_world: Mat4, view_origin: Vec3) -> Mat4 {
    let shift = Mat4::from_translation(view_origin);
    clip_from_world * shift
}
