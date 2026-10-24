//! Ray-against-box tests and the world-space bounds that let the tracer skip things a line cannot reach.

use super::collider::ColliderId;
use super::entity_caps::{EntityCollisionTraceGeom, LinkedBrushCollisionBrush};
use super::logs::hitbox_log_on;
use super::posed_coll::{AuthorityDObjCollision, AuthorityDObjCollisionBone, matrix_from_angles};
use super::poses::PlayerCollisionPose;
use crate::world::SimClipCmodels;

/// Bounds are grown by this much on every side so a ray grazing a box is never culled by rounding.
const BOUNDS_PAD: f32 = 1.0;

/// Smallest and largest corner of an axis-aligned box.
pub(super) type Bounds = ([f32; 3], [f32; 3]);

/// Where a line meets a box.
#[derive(Clone, Copy, Debug)]
pub(super) struct BoxHit {
    pub fraction: f32,
    pub endpos: [f32; 3],
    pub normal: [f32; 3],
}

pub(super) enum RayBox {
    Miss,
    /// The line begins inside the box.
    Inside,
    Hit(BoxHit),
}

pub(super) fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scaled(v: [f32; 3], by: f32) -> [f32; 3] {
    [v[0] * by, v[1] * by, v[2] * by]
}

/// The first point along `start + dir * t` (t in 0..=1) that lies on the box `lo..hi`, with the normal of the face it
/// enters through. `face(axis, sign)` builds that normal; `sign` is -1 when the line travels toward +axis.
fn enter_box(
    lo: [f32; 3],
    hi: [f32; 3],
    start: [f32; 3],
    dir: [f32; 3],
    face: impl Fn(usize, f32) -> [f32; 3],
) -> Option<(f32, [f32; 3])> {
    let mut enter = 0.0_f32;
    let mut leave = 1.0_f32;
    let mut normal = [0.0_f32; 3];
    for axis in 0..3 {
        if dir[axis].abs() < 1e-8 {
            // Parallel to this slab: it either lies inside it all the way or never touches the box.
            if start[axis] < lo[axis] || start[axis] > hi[axis] {
                return None;
            }
            continue;
        }
        let inverse = dir[axis].recip();
        let mut near = (lo[axis] - start[axis]) * inverse;
        let mut far = (hi[axis] - start[axis]) * inverse;
        let sign = if inverse < 0.0 { 1.0 } else { -1.0 };
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        if near > enter {
            enter = near;
            normal = face(axis, sign);
        }
        leave = leave.min(far);
        if enter > leave {
            return None;
        }
    }
    (0.0..=1.0).contains(&enter).then_some((enter, normal))
}

fn point_along(start: [f32; 3], dir: [f32; 3], fraction: f32) -> [f32; 3] {
    [start[0] + dir[0] * fraction, start[1] + dir[1] * fraction, start[2] + dir[2] * fraction]
}

/// A line against an axis-aligned box.
pub(super) fn ray_aabb_box(start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) -> RayBox {
    if (0..3).all(|i| start[i] >= mins[i] && start[i] <= maxs[i]) {
        return RayBox::Inside;
    }
    let dir = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let axis_face = |axis: usize, sign: f32| {
        let mut normal = [0.0_f32; 3];
        normal[axis] = sign;
        normal
    };
    match enter_box(mins, maxs, start, dir, axis_face) {
        Some((fraction, normal)) => RayBox::Hit(BoxHit { fraction, endpos: point_along(start, dir, fraction), normal }),
        None => RayBox::Miss,
    }
}

/// The box a player without part boxes is tested as: his mins and maxs around his origin.
pub(super) fn pose_box(pose: &PlayerCollisionPose) -> Bounds {
    let corner = |offset: [f32; 3]| [0, 1, 2].map(|i| pose.origin[i] + offset[i]);
    (corner(pose.mins), corner(pose.maxs))
}

/// A line against an oriented box. `None` means it misses.
///
/// BO2's hit-box test never reports "started solid". A line that begins inside the box and leaves it gets no hit
/// from that box; one that starts inside and stays inside while heading in toward the body's middle (`origin`) hits
/// at once, facing outward from that middle.
pub(super) fn ray_obb(
    start: [f32; 3],
    end: [f32; 3],
    bone: &AuthorityDObjCollisionBone,
    collider: ColliderId,
    origin: Option<[f32; 3]>,
) -> Option<BoxHit> {
    let to_local = |point: [f32; 3]| {
        let from_center = [point[0] - bone.center[0], point[1] - bone.center[1], point[2] - bone.center[2]];
        bone.axes.map(|axis| dot3(from_center, axis))
    };
    let local_start = to_local(start);
    let local_end = to_local(end);
    let within = |local: [f32; 3]| (0..3).all(|axis| local[axis].abs() <= bone.half_size[axis]);

    if within(local_start) {
        let away = origin.map(|o| [start[0] - o[0], start[1] - o[1]]);
        let heading_in = away.is_some_and(|a| (end[0] - start[0]) * a[0] + (end[1] - start[1]) * a[1] < 0.0);
        let verdict = match away {
            Some(a) if within(local_end) && heading_in => {
                let len = (a[0] * a[0] + a[1] * a[1]).sqrt();
                let normal = if len > 0.0 { [a[0] / len, a[1] / len, 0.0] } else { [0.0; 3] };
                Some(BoxHit { fraction: 0.0, endpos: start, normal })
            }
            _ => None,
        };
        if hitbox_log_on() {
            let outcome = if verdict.is_some() { "hit at once" } else { "no hit from this box" };
            diag::info!(Sim, "hitbox: ray starts inside {collider:?} at {start:?}: {}", outcome);
        }
        return verdict;
    }

    let local_dir = [local_end[0] - local_start[0], local_end[1] - local_start[1], local_end[2] - local_start[2]];
    let lo = bone.half_size.map(|h| -h);
    let axis_face = |axis: usize, sign: f32| scaled(bone.axes[axis], sign);
    let (fraction, normal) = enter_box(lo, bone.half_size, local_start, local_dir, axis_face)?;
    let dir = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    Some(BoxHit { fraction, endpos: point_along(start, dir, fraction), normal })
}

fn padded((lo, hi): Bounds) -> Bounds {
    (lo.map(|v| v - BOUNDS_PAD), hi.map(|v| v + BOUNDS_PAD))
}

/// Folds `more` into the running union.
fn include(union: &mut Option<Bounds>, more: Bounds) {
    match union {
        Some((lo, hi)) => {
            for i in 0..3 {
                lo[i] = lo[i].min(more.0[i]);
                hi[i] = hi[i].max(more.1[i]);
            }
        }
        None => *union = Some(more),
    }
}

/// The axis-aligned box around an oriented one.
fn bone_bounds(bone: &AuthorityDObjCollisionBone) -> Bounds {
    let reach = [0, 1, 2].map(|w| {
        bone.axes[0][w].abs() * bone.half_size[0]
            + bone.axes[1][w].abs() * bone.half_size[1]
            + bone.axes[2][w].abs() * bone.half_size[2]
    });
    ([0, 1, 2].map(|i| bone.center[i] - reach[i]), [0, 1, 2].map(|i| bone.center[i] + reach[i]))
}

/// Padded bounds around all of a player's part boxes, or `None` when he has none.
pub(super) fn pose_bones_bounds(pose: &PlayerCollisionPose) -> Option<Bounds> {
    let mut union = None;
    for bone in &pose.bones {
        include(&mut union, bone_bounds(bone));
    }
    union.map(padded)
}

/// Padded bounds around a brush model placed by origin and angles.
fn cmodel_bounds(cmodel: &asset_world::ClipCmodel, origin: [f32; 3], angles: [f32; 3]) -> Bounds {
    if angles == [0.0; 3] {
        return padded(([0, 1, 2].map(|i| origin[i] + cmodel.mins[i]), [0, 1, 2].map(|i| origin[i] + cmodel.maxs[i])));
    }
    let placed = matrix_from_angles(glam::Vec3::from_array(origin), angles);
    let mut union = None;
    for corner in 0..8_u32 {
        let pick = |axis: u32| if corner >> axis & 1 == 0 { cmodel.mins[axis as usize] } else { cmodel.maxs[axis as usize] };
        let point = placed.transform_point3(glam::Vec3::new(pick(0), pick(1), pick(2))).to_array();
        include(&mut union, (point, point));
    }
    padded(union.expect("eight corners were folded in"))
}

/// Bounds around everything an entity offers a bullet (its brush models and its posed part boxes), or `None` when it
/// offers nothing.
pub(super) fn entity_bounds(
    linked_brushes: &[LinkedBrushCollisionBrush],
    posed: Option<&AuthorityDObjCollision>,
    cmodels: &SimClipCmodels,
) -> Option<Bounds> {
    let mut union = None;
    for brush in linked_brushes {
        if let Some(cmodel) = cmodels.models.get(brush.cmodel_handle as usize) {
            include(&mut union, cmodel_bounds(cmodel, brush.origin, brush.angles));
        }
    }
    for bone in posed.iter().flat_map(|collision| collision.bones.iter()) {
        include(&mut union, bone_bounds(bone));
    }
    union
}

pub(super) fn geom_bounds(geom: &EntityCollisionTraceGeom, cmodels: &SimClipCmodels) -> Option<Bounds> {
    entity_bounds(&geom.linked_brushes, geom.collision.as_ref(), cmodels)
}
