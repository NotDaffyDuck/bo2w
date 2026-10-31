//! Seeing through a linked portal (Portal's look): the map as an eye
//! behind the other end sees it, so a portal shows what is past its
//! partner. The cull picks the looks each frame (`set_looks`); the world
//! renderer draws each into its own texture and the portal's oval shows it.

use std::sync::Mutex;

use glam::{Mat3, Mat4, Vec3, Vec4};

/// One portal's end, as the game shot it.
#[derive(Clone, Copy, Debug)]
pub struct PortalEnd {
    pub centre: Vec3,
    pub normal: Vec3,
    pub up: Vec3,
    pub orange: bool,
    pub linked: bool,
    pub pair: u32,
}

/// Looking into `entry` and out of its partner.
#[derive(Clone, Copy, Debug)]
pub struct PortalLook {
    /// 0 when the blue end is looked into, 1 the orange.
    pub bay: usize,
    pub pair: u32,
    pub entry: PortalEnd,
    pub exit: PortalEnd,
    /// Maps a point in front of the entry to where it comes out.
    pub through: Mat4,
    /// The exit's wall, just in front of it (map units), facing out: what
    /// is behind it is not seen.
    pub exit_plane: Vec4,
}

/// How far off a portal looking through it stops (map units).
const REACH: f32 = 6000.0;

fn frame(p: &PortalEnd) -> Mat3 {
    Mat3::from_cols(p.up.cross(p.normal), p.up, p.normal)
}

/// `mp_portals::turn`, as a whole move: in one end, out of the other,
/// turned round.
pub fn through(from: &PortalEnd, to: &PortalEnd) -> Mat4 {
    let r = frame(to) * Mat3::from_diagonal(Vec3::new(-1.0, 1.0, -1.0)) * frame(from).transpose();
    Mat4::from_translation(to.centre) * Mat4::from_mat3(r) * Mat4::from_translation(-from.centre)
}

/// The nearest linked pair's looks: one for each end the eye is in front
/// of. `BO2MP_NO_PORTAL_VIEW` turns them off.
pub fn choose_looks(ends: &[PortalEnd], eye: Vec3) -> Vec<PortalLook> {
    if std::env::var_os("BO2MP_NO_PORTAL_VIEW").is_some() {
        return Vec::new();
    }
    let mut nearest_pair: Option<(f32, [PortalEnd; 2])> = None;
    for a in ends.iter().filter(|e| e.linked && !e.orange) {
        let Some(b) = ends.iter().find(|e| e.linked && e.orange && e.pair == a.pair) else {
            continue;
        };
        let d = eye.distance(a.centre).min(eye.distance(b.centre));
        if d < REACH && nearest_pair.as_ref().is_none_or(|(bd, _)| d < *bd) {
            nearest_pair = Some((d, [*a, *b]));
        }
    }
    let Some((_, pair)) = nearest_pair else { return Vec::new() };
    let mut looks = Vec::new();
    for k in 0..2 {
        let (entry, exit) = (pair[k], pair[1 - k]);
        if (eye - entry.centre).dot(entry.normal) <= 0.5 {
            continue;
        }
        let n = exit.normal.normalize_or(Vec3::Z);
        let on = exit.centre + n;
        looks.push(PortalLook {
            bay: k,
            pair: entry.pair,
            entry,
            exit,
            through: through(&entry, &exit),
            exit_plane: n.extend(-n.dot(on)),
        });
    }
    looks
}

/// Clip planes (`n . x + w >= 0` inside) carried by a rigid move.
pub fn move_planes(planes: &[[f32; 4]], m: &Mat4) -> Vec<[f32; 4]> {
    let t = m.w_axis.truncate();
    planes
        .iter()
        .map(|p| {
            let n = m.transform_vector3(Vec3::new(p[0], p[1], p[2]));
            [n.x, n.y, n.z, p[3] - n.dot(t)]
        })
        .collect()
}

/// A reverse-Z projection whose near plane is `plane` (view space,
/// positive on the side seen): depth 1 on it, the side behind cut, far
/// still nearer 0. `k` stays under the cosine of the half diagonal view.
pub fn oblique(clip_from_view: Mat4, plane: Vec4) -> Mat4 {
    let mut rows = clip_from_view.transpose();
    rows.z_axis = rows.w_axis - plane * 0.3;
    rows.transpose()
}

static LOOKS: Mutex<Vec<PortalLook>> = Mutex::new(Vec::new());

pub fn set_looks(list: Vec<PortalLook>) {
    *LOOKS.lock().unwrap() = list;
}

pub fn looks() -> Vec<PortalLook> {
    LOOKS.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(c: Vec3, n: Vec3, orange: bool) -> PortalEnd {
        PortalEnd { centre: c, normal: n, up: Vec3::Z, orange, linked: true, pair: 0 }
    }

    #[test]
    fn into_one_wall_out_of_the_other() {
        let a = end(Vec3::new(0.0, 0.0, 50.0), Vec3::X, false);
        let b = end(Vec3::new(500.0, 300.0, 50.0), Vec3::Y, true);
        let m = through(&a, &b);
        // The entry's centre lands on the exit's; a point in front of the
        // entry lands behind the exit (the eye is behind the other end).
        assert!(m.transform_point3(a.centre).distance(b.centre) < 1e-3);
        let front = m.transform_point3(a.centre + Vec3::X * 100.0);
        assert!((front - (b.centre - Vec3::Y * 100.0)).length() < 1e-3);
        let looks = choose_looks(&[a, b], Vec3::new(200.0, 0.0, 50.0));
        assert_eq!(looks.len(), 1);
        assert_eq!(looks[0].bay, 0);
    }

    #[test]
    fn oblique_cuts_at_the_plane() {
        let f = 1.0;
        let near = 4.0;
        let p = Mat4::from_cols_array(&[
            f, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, near, 0.0,
        ]);
        // The plane z = -100 (view looks down -Z), seen side further on.
        let plane = Vec4::new(0.0, 0.0, -1.0, -100.0);
        let m = oblique(p, plane);
        let on = m * Vec4::new(0.0, 0.0, -100.0, 1.0);
        assert!((on.z / on.w - 1.0).abs() < 1e-4);
        let past = m * Vec4::new(0.0, 0.0, -400.0, 1.0);
        let further = m * Vec4::new(0.0, 0.0, -4000.0, 1.0);
        assert!(past.z / past.w < 1.0 && further.z / further.w < past.z / past.w);
        assert!(further.z >= 0.0);
        let before = m * Vec4::new(0.0, 0.0, -50.0, 1.0);
        assert!(before.z / before.w > 1.0);
    }
}
