//! Numbers that must come out bit for bit as BO2's: angle wrapping and easing (wrap), Euler angles against vectors and
//! axes (euler), axis and frame products (frames), the lean offset (peek), and view and projection joint_transforms (view).
#![no_std]
#![forbid(unsafe_code)]
mod euler;
mod frames;
mod peek;
mod camera_math;
mod wrap;
pub use euler::{euler_basis, euler_to_rows, rows_to_euler, slope_pitch, heading_of, dir_to_euler, flat_basis};
pub use frames::{axis_product, frame_product, frame_apply, vec3_norm};
pub use peek::{lean_offset, lean_amount};
pub use camera_math::{compose_view_projection, matrix_for_viewer, matrix_multiply44, perpendicular_vector, setup_finite_projection_matrix};
pub use wrap::{wrap_degrees, degree_gap, round_degrees, ease_toward, ease_angle_toward};
