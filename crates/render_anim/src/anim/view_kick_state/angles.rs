//! Putting the view kick onto the camera's aim.

/// The camera's pitch, yaw and roll with the current view kick added on.
///
/// Both arguments are `[pitch, yaw, roll]` in degrees; the result is a plain
/// per-axis sum with no wrapping, which the caller does if it needs to.
#[inline]
pub fn add_kick_to_viewangles(viewangles: [f32; 3], kick_angles: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| viewangles[axis] + kick_angles[axis])
}
