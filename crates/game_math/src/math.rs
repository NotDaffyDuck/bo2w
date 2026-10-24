//! Numbers that must come out bit for bit as BO2's: angle wrapping and easing (wrap), Euler angles against vectors and
//! axes (euler), axis and frame products (frames), the lean offset (peek), and view and projection matrices (view).
#![no_std]
#![forbid(unsafe_code)]
mod euler;
mod frames;
mod peek;
mod view;
mod wrap;
pub use euler::{angle_vectors, angles_to_axis, axis_to_angles, pitch_for_yaw_on_normal, vec_to_yaw, vect_to_angles, yaw_vectors_2d};
pub use frames::{matrix_multiply, matrix_multiply43, matrix_transform_vector43, vec3_length};
pub use peek::{add_lean_to_position, get_lean_fraction};
pub use view::{compose_view_projection, matrix_for_viewer, matrix_multiply44, perpendicular_vector, setup_finite_projection_matrix};
pub use wrap::{angle_normalize_360, angle_subtract, snap_angles, track, track_angle};
