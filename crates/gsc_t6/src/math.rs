//! Angle and vector math the scripts use (Quake/IW conventions: angles are
//! pitch, yaw, roll in degrees; forward = +x at yaw 0, left = +y, up = +z).

type Vec3 = [f32; 3];

pub fn facing_axes(a: Vec3) -> (Vec3, Vec3, Vec3) {
    let (sin_pitch, cos_pitch) = a[0].to_radians().sin_cos();
    let (yaw_sin, yaw_cos) = a[1].to_radians().sin_cos();
    let (sin_roll, cos_roll) = a[2].to_radians().sin_cos();
    let forward = [cos_pitch * yaw_cos, cos_pitch * yaw_sin, -sin_pitch];
    let right = [-sin_roll * sin_pitch * yaw_cos + cos_roll * yaw_sin, -sin_roll * sin_pitch * yaw_sin - cos_roll * yaw_cos, -sin_roll * cos_pitch];
    let up = [cos_roll * sin_pitch * yaw_cos + sin_roll * yaw_sin, cos_roll * sin_pitch * yaw_sin - sin_roll * yaw_cos, cos_roll * cos_pitch];
    (forward, right, up)
}

pub fn vector_to_angles(d: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = d;
    // Straight up or down: no heading, pitch 270 up / 90 down.
    let (pitch, yaw) = if x == 0.0 && y == 0.0 {
        (if z > 0.0 { 270.0 } else { 90.0 }, 0.0)
    } else {
        // atan2 answers in (-180, 180]; scripts want [0, 360).
        let wrap = |deg: f32| if deg < 0.0 { deg + 360.0 } else { deg };
        let ground = (x * x + y * y).sqrt();
        (wrap((-z).atan2(ground).to_degrees()), wrap(y.atan2(x).to_degrees()))
    };
    let mut out = [0.0; 3];
    (out[0], out[1]) = (pitch, yaw);
    out
}

/// An angle in (-180, 180].
pub fn angle_clamp180(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a <= -180.0 {
        a += 360.0;
    }
    a
}

/// An angle in [0, 360).
pub fn angle_clamp(a: f32) -> f32 {
    let a = a % 360.0;
    if a < 0.0 { a + 360.0 } else { a }
}

/// `atoi`: leading integer, 0 if none.
pub fn parse_int(s: &str) -> i32 {
    let s = s.trim();
    // An optional sign, then digits (all ASCII, so byte counts are char counts).
    let sign = usize::from(s.starts_with(['-', '+']));
    let digits = s[sign..].bytes().take_while(u8::is_ascii_digit).count();
    let lead = &s[..sign + digits];
    match lead.parse::<i32>() {
        Ok(n) => n,
        // No usable integer prefix: try the whole thing as a float, else 0.
        Err(_) => s.parse::<f32>().map_or(0, |f| f as i32),
    }
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [x, y, z]: [f32; 3] = core::array::from_fn(|k| a[k] * b[k]);
    x + y + z
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| a[k] - b[k])
}

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| a[k] + b[k])
}

pub fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|c| c * k)
}

pub fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

pub fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { [0.0; 3] }
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| {
        let (j, l) = ((k + 1) % 3, (k + 2) % 3);
        a[j] * b[l] - a[l] * b[j]
    })
}
