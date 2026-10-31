//! Small vector helpers shared by the launch and flight code.

/// Unit vector a pitch/yaw pair looks along.
pub(super) fn look_dir(angles: [f32; 3]) -> [f32; 3] {
    let (pitch, yaw) = (angles[0].to_radians(), angles[1].to_radians());
    let flat = pitch.cos();
    [flat * yaw.cos(), flat * yaw.sin(), -pitch.sin()]
}

pub(super) fn norm(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `v` scaled to length one, or None when it is too short to have a direction.
pub(super) fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = norm(v);
    (len >= 1e-6).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn gap(a: [f32; 3], b: [f32; 3]) -> f32 {
    norm([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}
