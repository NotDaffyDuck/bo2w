//! Matrix and float helpers for feeding constants to shaders as raw bits.

use glam::{Mat4, Vec3};

/// The bit patterns of four floats, so constants can be compared and hashed
/// exactly.
pub fn vec4_bits(value: [f32; 4]) -> [u32; 4] {
    value.map(f32::to_bits)
}

/// The rows of `m` as bit patterns. A shader reads a column-major matrix row by
/// row, so what is returned is each row of `m` as four consecutive values.
pub fn flip_matrix_rows_to_words(m: Mat4) -> [[u32; 4]; 4] {
    let columns = m.to_cols_array_2d();
    std::array::from_fn(|row| vec4_bits(columns.map(|column| column[row])))
}

/// [`flip_matrix_rows_to_words`] as a `Vec`.
pub fn engine_transpose_matrix_rows(m: Mat4) -> Vec<[u32; 4]> {
    Vec::from(flip_matrix_rows_to_words(m))
}

/// A world-to-clip matrix that takes positions relative to `eye_origin`
/// instead of absolute ones.
pub fn eye_relative_clip(world_to_clip: Mat4, eye_origin: Vec3) -> Mat4 {
    let shift = Mat4::from_translation(eye_origin);
    world_to_clip * shift
}
