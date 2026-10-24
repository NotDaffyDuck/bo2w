//! BO2's angle math from `com_math.cpp`: `AngleSubtract`,
//! `AngleNormalize360`, `AngleVectors`, `vectoangles`,
//! `ProjectPointOnPlane`, `PitchForYawOnNormal` and
//! `NearestPitchAndYawOnPlane`.
//!
//! BO2 runs the trig on the x87 stack and stores floats, so the trig here
//! runs in f64 and rounds to f32 where BO2 stores.

/// BO2's degrees-to-radians factor.
pub const DEG2RAD: f32 = 0.017_453_292;
/// BO2's `1 / pi`, used as `* 180 * INV_PI` to turn radians to degrees.
pub const INV_PI: f32 = 0.318_309_87;

/// BO2's `floor(x)` as its SSE code does it: round to nearest, then step
/// down when that went up.
pub(crate) fn floor_bo2(x: f32) -> f32 {
    let r = libm::rintf(x);
    if r > x { r - 1.0 } else { r }
}

/// `AngleSubtract`: `a - b` folded into [-180, 180).
pub fn angle_subtract(a: f32, b: f32) -> f32 {
    let x = (a - b) * 0.002_777_777_8;
    (x - floor_bo2(x + 0.5)) * 360.0
}

/// `AngleNormalize360`: folded into [0, 360).
pub fn angle_normalize_360(a: f32) -> f32 {
    let x = a * 0.002_777_777_8;
    let r = (x - floor_bo2(x)) * 360.0;
    if r - 360.0 >= 0.0 { r - 360.0 } else { r }
}

pub(crate) fn cos_sin(deg: f32) -> (f32, f32) {
    let r = (deg * DEG2RAD) as f64;
    (libm::cos(r) as f32, libm::sin(r) as f32)
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `AngleVectors`: forward, right and up for pitch, yaw, roll in degrees.
pub fn angle_vectors(angles: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (cy, sy) = cos_sin(angles[1]);
    let (cp, sp) = cos_sin(angles[0]);
    let (cr, sr) = cos_sin(angles[2]);
    let forward = [cp * cy, cp * sy, -sp];
    let right = [
        cr * sy - sr * sp * cy,
        -cr * cy - sr * sp * sy,
        -sr * cp,
    ];
    let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
    (forward, right, up)
}

/// `vectoangles`: pitch and yaw in [0, 360) for a direction; roll is 0.
///
/// Straight up gives pitch 270 and straight down (or zero) pitch 90.
pub fn vectoangles(v: [f32; 3]) -> [f32; 3] {
    if v[1] == 0.0 && v[0] == 0.0 {
        let pitch = if -v[2] < 0.0 { 270.0 } else { 90.0 };
        return [pitch, 0.0, 0.0];
    }
    let yaw = (libm::atan2(v[1] as f64, v[0] as f64) as f32) * 180.0 * INV_PI;
    let yaw = if yaw < 0.0 { 360.0 } else { 0.0 } + yaw;
    let flat = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    let pitch = (libm::atan2(v[2] as f64, flat as f64) as f32) * -180.0 * INV_PI;
    let pitch = if pitch < 0.0 { 360.0 } else { 0.0 } + pitch;
    [pitch, yaw, 0.0]
}

/// `ProjectPointOnPlane`: `p` minus its part along the unit `normal`.
pub fn project_point_on_plane(p: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let d = -(p[1] * normal[1] + normal[0] * p[0] + p[2] * normal[2]);
    [
        normal[0] * d + p[0],
        d * normal[1] + p[1],
        d * normal[2] + p[2],
    ]
}

/// `PitchForYawOnNormal`: the pitch that keeps a heading of `yaw` on the
/// plane with this normal. A wall (`normal.z == 0`) gives 270.
pub fn pitch_for_yaw_on_normal(yaw: f32, normal: [f32; 3]) -> f32 {
    if normal[2] == 0.0 {
        return 270.0;
    }
    let (cy, sy) = cos_sin(yaw);
    let t = (normal[0] as f64 * cy as f64 + normal[1] as f64 * sy as f64) / normal[2] as f64;
    (libm::atan(t) * 180.0 * INV_PI as f64) as f32
}

/// `NearestPitchAndYawOnPlane`: the facing on the plane closest to
/// `angles`; roll comes back 0.
pub fn nearest_pitch_and_yaw_on_plane(angles: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let (forward, _, _) = angle_vectors(angles);
    vectoangles(project_point_on_plane(forward, normal))
}

/// The roll BO2's flat landings give: the one that turns `angles`' up
/// onto `normal`, `atan2(normal . right, normal . up)` in degrees.
pub fn roll_onto_normal(angles: [f32; 3], normal: [f32; 3]) -> f32 {
    let (_, right, up) = angle_vectors(angles);
    let a = dot(normal, right);
    let b = dot(normal, up);
    (libm::atan2(a as f64, b as f64) as f32) * 180.0 * INV_PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        libm::fabsf(a - b) < 1.0e-3
    }

    #[test]
    fn subtract_and_normalize() {
        assert!(close(angle_subtract(10.0, 350.0), 20.0));
        assert!(close(angle_subtract(350.0, 10.0), -20.0));
        assert!(close(angle_subtract(180.0, 0.0), -180.0));
        assert!(close(angle_normalize_360(-90.0), 270.0));
        assert!(close(angle_normalize_360(720.0), 0.0));
        assert!(close(angle_normalize_360(359.5), 359.5));
    }

    #[test]
    fn vectors_and_angles_round_trip() {
        assert_eq!(vectoangles([0.0, 0.0, 1.0]), [270.0, 0.0, 0.0]);
        assert_eq!(vectoangles([0.0, 0.0, -1.0]), [90.0, 0.0, 0.0]);
        assert_eq!(vectoangles([0.0, 0.0, 0.0]), [90.0, 0.0, 0.0]);
        let a = vectoangles([0.0, -1.0, 1.0]);
        assert!(close(a[0], 315.0) && close(a[1], 270.0) && a[2] == 0.0);
        let (f, r, u) = angle_vectors([30.0, 120.0, 0.0]);
        let back = vectoangles(f);
        assert!(close(back[0], 30.0) && close(back[1], 120.0));
        // Right-handed with BO2's right pointing to the right of forward.
        assert!(close(dot(f, r), 0.0) && close(dot(f, u), 0.0) && close(dot(r, u), 0.0));
        let (_, r0, u0) = angle_vectors([0.0, 0.0, 0.0]);
        assert_eq!((r0, u0), ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]));
    }

    #[test]
    fn pitch_on_a_slope() {
        // A 45 degree slope rising toward +x: heading +x climbs, pitch -45
        // (BO2 returns the raw atan here, not folded to 315).
        let n = [-0.707_106_77, 0.0, 0.707_106_77];
        assert!(close(pitch_for_yaw_on_normal(0.0, n), -45.0));
        assert!(close(pitch_for_yaw_on_normal(90.0, n), 0.0));
        assert_eq!(pitch_for_yaw_on_normal(0.0, [1.0, 0.0, 0.0]), 270.0);
    }

    #[test]
    fn nearest_facing_and_roll_lie_on_the_plane() {
        let n = [-0.707_106_77, 0.0, 0.707_106_77];
        let a = nearest_pitch_and_yaw_on_plane([0.0, 0.0, 0.0], n);
        assert!(close(a[1], 0.0) && close(a[0], 315.0) && a[2] == 0.0);
        let (f, _, _) = angle_vectors(a);
        assert!(close(dot(f, n), 0.0));
        // Heading across the slope, the roll tips up onto the normal.
        let side = [0.0, 90.0, 0.0];
        let roll = roll_onto_normal(side, n);
        let (_, _, up) = angle_vectors([0.0, 90.0, roll]);
        assert!(close(up[0], n[0]) && close(up[2], n[2]));
    }
}
