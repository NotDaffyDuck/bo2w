//! Putting the view kick onto the camera's aim.

/// The camera's pitch, yaw and roll with the current view kick added on.
///
/// Both arguments are `[pitch, yaw, roll]` in degrees; the result is a plain
/// per-axis sum with no wrapping, which the caller does if it needs to.
#[inline]
pub fn kicked_view_angles(viewangles_in: [f32; 3], recoil_tilt: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| viewangles_in[axis] + recoil_tilt[axis])
}
