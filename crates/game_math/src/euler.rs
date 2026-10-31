//! Pitch, yaw and roll against direction vectors and axes, BO2's way: degrees in, its own radians constant, the
//! right vector pointing right, and arctangents in the precision BO2 used for each.

/// BO2's degrees-to-radians factor (the nearest float to pi / 180).
const RADIANS_PER_DEGREE: f32 = 0.017_453_292_38;

/// BO2's 1 / pi; it turns radians into degrees as `radians * 180 * (1 / pi)`.
const ONE_OVER_PI: f32 = 0.318_309_87;

/// Radians to degrees, negated, for the roll arctangent.
const MINUS_DEGREES_PER_RADIAN: f32 = -57.295_776_367_187_5;

/// A direction vector.
type Direction = [f32; 3];

/// Three direction rows.
type AxisRows = [Direction; 3];

/// The sine and cosine of one angle given in degrees.
#[derive(Clone, Copy)]
struct Turn {
    sin: f32,
    cos: f32,
}

fn turn(degrees: f32) -> Turn {
    let radians = degrees * RADIANS_PER_DEGREE;
    Turn { sin: libm::sinf(radians), cos: libm::cosf(radians) }
}

/// A negative result moved up one turn, so angles read 0 to 360.
fn one_turn_up(degrees: f32) -> f32 {
    if degrees >= 0.0 { degrees } else { degrees + 360.0 }
}

/// Forward, right and up for `[pitch, yaw, roll]`.
pub fn euler_basis(euler: [f32; 3]) -> (Direction, Direction, Direction) {
    let [p, y, r] = euler.map(turn);
    let forward = [p.cos * y.cos, p.cos * y.sin, -p.sin];
    let side = [
        y.sin * r.cos - y.cos * r.sin * p.sin,
        -(y.cos * r.cos) - r.sin * p.sin * y.sin,
        -r.sin * p.cos,
    ];
    let up = [
        y.cos * r.cos * p.sin + y.sin * r.sin,
        r.cos * p.sin * y.sin - r.sin * y.cos,
        r.cos * p.cos,
    ];
    (forward, side, up)
}

/// The axis rows forward, left, up for `[pitch, yaw, roll]`.
pub fn euler_to_rows(euler: [f32; 3]) -> AxisRows {
    let (ahead, side, above) = euler_basis(euler);
    [ahead, side.map(|component| -component), above]
}

/// BO2's vectoyaw: degrees 0 to 360 for the direction (x, y); the arctangent is taken in double precision.
pub fn heading_of(step_along_x: f32, dy: f32) -> f32 {
    if step_along_x == 0.0 && dy == 0.0 {
        return 0.0;
    }
    let radians = libm::atan2(f64::from(dy), f64::from(step_along_x)) as f32;
    one_turn_up(radians * 180.0 * ONE_OVER_PI)
}

/// BO2's vectoangles: `[pitch, yaw, 0]`, each 0 to 360. Straight up is pitch 270 and straight down 90, matching the
/// directions just off vertical.
pub fn dir_to_euler(dir: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = dir;
    if x == 0.0 && y == 0.0 {
        return [if z > 0.0 { 270.0 } else { 90.0 }, 0.0, 0.0];
    }
    let flat = libm::sqrtf(x * x + y * y);
    let radians = libm::atan2(f64::from(z), f64::from(flat)) as f32;
    [one_turn_up(radians * -180.0 * ONE_OVER_PI), heading_of(x, y), 0.0]
}

/// `(a, b)` turned by the angle whose cosine and sine are `cos` and `sin`.
fn spin(cos: f32, sin: f32, a: f32, b: f32) -> (f32, f32) {
    (cos * a - sin * b, sin * a + cos * b)
}

/// `[pitch, yaw, roll]` of an axis (rows forward, left, up). Pitch and yaw come from forward; roll from the left row
/// once the yaw and pitch are undone.
pub fn rows_to_euler(rows: AxisRows) -> [f32; 3] {
    let [pitch, yaw, _] = dir_to_euler(rows[0]);
    let [lx, ly, lz] = rows[1];
    let undo_yaw = turn(-yaw);
    let (x, y) = spin(undo_yaw.cos, undo_yaw.sin, lx, ly);
    let undo_pitch = turn(-pitch);
    let (x, rise) = spin(undo_pitch.cos, -undo_pitch.sin, x, lz);
    let tilt = if x == 0.0 && y == 0.0 {
        if rise <= 0.0 { 90.0 } else { -90.0 }
    } else {
        libm::atan2f(rise, libm::sqrtf(x * x + y * y)) * MINUS_DEGREES_PER_RADIAN
    };
    let roll = if y >= 0.0 {
        -tilt
    } else if tilt < 0.0 {
        tilt + 180.0
    } else {
        tilt - 180.0
    };
    [pitch, yaw, roll]
}

/// The pitch that lies along a surface whose normal is `surface` when facing `heading`; 270 on a wall.
pub fn slope_pitch(heading: f32, surf: [f32; 3]) -> f32 {
    let [facing_x, facing_y, facing_z] = surf;
    if facing_z == 0.0 {
        return 270.0;
    }
    let ([fx, fy], _) = flat_basis(heading);
    let slope = (facing_x * fx + facing_y * fy) / facing_z;
    libm::atanf(slope) * 180.0 / core::f32::consts::PI
}

/// Flat forward and right for a yaw.
pub fn flat_basis(yaw: f32) -> ([f32; 2], [f32; 2]) {
    let Turn { sin, cos } = turn(yaw);
    ([cos, sin], [sin, -cos])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_directions_read_270_up_and_90_down() {
        assert_eq!(dir_to_euler([0.0, 0.0, 1.0]), [270.0, 0.0, 0.0]);
        assert_eq!(dir_to_euler([0.0, 0.0, -1.0]), [90.0, 0.0, 0.0]);
        let just_off = dir_to_euler([0.001, 0.0, 1.0])[0];
        assert!((just_off - 270.0).abs() < 0.1, "{just_off}");
    }

    #[test]
    fn yaw_covers_all_four_quarters() {
        assert_eq!(heading_of(0.0, 0.0), 0.0);
        assert_eq!(heading_of(1.0, 0.0), 0.0);
        assert!((heading_of(0.0, 1.0) - 90.0).abs() < 1e-4);
        assert!((heading_of(-1.0, 0.0) - 180.0).abs() < 1e-4);
        assert!((heading_of(0.0, -1.0) - 270.0).abs() < 1e-4);
    }

    #[test]
    fn an_axis_gives_back_its_angles() {
        let angles = [20.0, 135.0, 30.0];
        let back = rows_to_euler(euler_to_rows(angles));
        for (a, b) in angles.into_iter().zip(back) {
            assert!((a - b).abs() < 1e-3, "{angles:?} -> {back:?}");
        }
    }
}
