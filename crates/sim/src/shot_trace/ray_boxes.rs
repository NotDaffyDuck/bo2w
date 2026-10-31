//! Ray-against-box tests and the world-space bounds that let the tracer skip things a line cannot reach.

use super::collider::HitTargetId;
use super::entity_caps::{EntityRayShape, AttachedSolidShape};
use super::logs::hitbox_log_on;
use super::posed_coll::{ServerModelCollision, matrix_from_angles};
use super::poses::PlayerHitboxPose;
use crate::sim_parts::CollisionSubmodels;

/// Bounds are grown by this much on every side so a ray grazing a box is never culled by rounding.
const BOUNDS_PAD: f32 = 1.0;

/// Where a line meets a box.
#[derive(Clone, Copy, Debug)]
pub(super) struct BoxHit {
    pub share: f32,
    pub finish_point: [f32; 3],
    pub face_dir: [f32; 3],
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

fn offset_between(from: [f32; 3], till: [f32; 3]) -> [f32; 3] {
    [till[0] - from[0], till[1] - from[1], till[2] - from[2]]
}

fn scaled(v: [f32; 3], by: f32) -> [f32; 3] {
    [v[0] * by, v[1] * by, v[2] * by]
}

/// The first point along `start + dir * t` (t in 0..=1) that lies on the box `lo..hi`, with the normal of the face it
/// enters through. `face(axis, sign)` builds that normal; `sign` is -1 when the line travels toward +axis.
fn enter_box(
    lo: [f32; 3],
    hi: [f32; 3],
    from: [f32; 3],
    dir: [f32; 3],
    face: impl Fn(usize, f32) -> [f32; 3],
) -> Option<(f32, [f32; 3])> {
    let mut enter = 0.0_f32;
    let mut leave = 1.0_f32;
    let mut entry_normal = [0.0_f32; 3];
    for (axis, &travel) in dir.iter().enumerate() {
        if travel.abs() < 1e-8 {
            // Parallel to this slab: it either lies inside it all the way or never touches the box.
            if from[axis] < lo[axis] || from[axis] > hi[axis] {
                return None;
            }
            continue;
        }
        let inverse = travel.recip();
        let mut close = (lo[axis] - from[axis]) * inverse;
        let mut distant = (hi[axis] - from[axis]) * inverse;
        let sign = if inverse < 0.0 { 1.0 } else { -1.0 };
        if distant < close {
            std::mem::swap(&mut close, &mut distant);
        }
        if close > enter {
            enter = close;
            entry_normal = face(axis, sign);
        }
        leave = leave.min(distant);
        if enter > leave {
            return None;
        }
    }
    (0.0..=1.0).contains(&enter).then_some((enter, entry_normal))
}

fn point_along(start: [f32; 3], heading: [f32; 3], fraction: f32) -> [f32; 3] {
    [start[0] + heading[0] * fraction, start[1] + heading[1] * fraction, start[2] + heading[2] * fraction]
}

/// A line against an axis-aligned box.
pub(super) fn ray_aabb_box(start: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> RayBox {
    if (0..3).all(|i| start[i] >= lo[i] && start[i] <= hi[i]) {
        return RayBox::Inside;
    }
    let dir = offset_between(start, till);
    let axis_face = |axis: usize, sign: f32| {
        let mut unit = [0.0_f32; 3];
        unit[axis] = sign;
        unit
    };
    match enter_box(lo, hi, start, dir, axis_face) {
        Some((fraction, normal)) => RayBox::Hit(BoxHit { share: fraction, finish_point: point_along(start, dir, fraction), face_dir: normal }),
        None => RayBox::Miss,
    }
}

/// The box a player without part boxes is tested as: his mins and maxs around his origin.
pub(super) fn pose_box(pose: &PlayerHitboxPose) -> ([f32; 3], [f32; 3]) {
    let corner = |offset: [f32; 3]| [0, 1, 2].map(|i| pose.origin[i] + offset[i]);
    (corner(pose.lo), corner(pose.hi))
}

/// A line against an oriented box. `None` means it misses.
///
/// BO2's hit-box test never reports "started solid". A line that begins inside the box and leaves it gets no hit
/// from that box; one that starts inside and stays inside while heading in toward the body's middle (`body_center`) hits
/// at once, facing outward from that middle.
pub(super) fn ray_obb(
    from: [f32; 3],
    end: [f32; 3],
    obb: &xmodel_runtime::HitShapeBone,
    owner: HitTargetId,
    body_center: Option<[f32; 3]>,
) -> Option<BoxHit> {
    let to_local = |point: [f32; 3]| {
        let from_center = [point[0] - obb.center[0], point[1] - obb.center[1], point[2] - obb.center[2]];
        obb.axes.map(|axis| dot3(from_center, axis))
    };
    let near_from = to_local(from);
    let near_stop = to_local(end);
    let within = |local: [f32; 3]| (0..3).all(|axis| local[axis].abs() <= obb.half_ext[axis]);

    if within(near_from) {
        let away = body_center.map(|o| [from[0] - o[0], from[1] - o[1]]);
        let heading_in = away.is_some_and(|a| (end[0] - from[0]) * a[0] + (end[1] - from[1]) * a[1] < 0.0);
        let verdict = match away {
            Some(a) if within(near_stop) && heading_in => {
                let len = (a[0] * a[0] + a[1] * a[1]).sqrt();
                let normal = if len > 0.0 { [a[0] / len, a[1] / len, 0.0] } else { [0.0; 3] };
                Some(BoxHit { share: 0.0, finish_point: from, face_dir: normal })
            }
            _ => None,
        };
        if hitbox_log_on() {
            let outcome = if verdict.is_some() { "hit at once" } else { "no hit from this box" };
            diag::info!(Sim, "hitbox: ray starts inside {owner:?} at {from:?}: {}", outcome);
        }
        return verdict;
    }

    let local_dir = offset_between(near_from, near_stop);
    let lo = obb.half_ext.map(|h| -h);
    let axis_face = |axis: usize, sign: f32| scaled(obb.axes[axis], sign);
    let (hit_t, normal) = enter_box(lo, obb.half_ext, near_from, local_dir, axis_face)?;
    let dir = offset_between(from, end);
    Some(BoxHit { share: hit_t, finish_point: point_along(from, dir, hit_t), face_dir: normal })
}

fn padded((lo, hi): ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
    (lo.map(|v| v - BOUNDS_PAD), hi.map(|v| v + BOUNDS_PAD))
}

/// Folds `more` into the running union.
fn include(union: &mut Option<([f32; 3], [f32; 3])>, more: ([f32; 3], [f32; 3])) {
    match union {
        Some((low, high)) => {
            for i in 0..3 {
                low[i] = low[i].min(more.0[i]);
                high[i] = high[i].max(more.1[i]);
            }
        }
        None => *union = Some(more),
    }
}

/// The axis-aligned box around an oriented one.
fn bone_bounds(bone: &xmodel_runtime::HitShapeBone) -> ([f32; 3], [f32; 3]) {
    let reach = [0, 1, 2].map(|w| {
        bone.axes[0][w].abs() * bone.half_ext[0]
            + bone.axes[1][w].abs() * bone.half_ext[1]
            + bone.axes[2][w].abs() * bone.half_ext[2]
    });
    ([0, 1, 2].map(|i| bone.center[i] - reach[i]), [0, 1, 2].map(|i| bone.center[i] + reach[i]))
}

/// Padded bounds around all of a player's part boxes, or `None` when he has none.
pub(super) fn pose_bones_bounds(pose: &PlayerHitboxPose) -> Option<([f32; 3], [f32; 3])> {
    let mut union = None;
    for part in &pose.joints {
        include(&mut union, bone_bounds(part));
    }
    union.map(padded)
}

/// Padded bounds around a brush model placed by origin and angles.
fn cmodel_bounds(collision_geometry: &asset_world::CollisionSubmodel, spot: [f32; 3], aim: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    if aim == [0.0; 3] {
        return padded(([0, 1, 2].map(|i| spot[i] + collision_geometry.lo[i]), [0, 1, 2].map(|i| spot[i] + collision_geometry.hi[i])));
    }
    let placed = matrix_from_angles(glam::Vec3::from_array(spot), aim);
    let mut union = None;
    for corner in 0..8_u32 {
        let pick = |axis: u32| if corner >> axis & 1 == 0 { collision_geometry.lo[axis as usize] } else { collision_geometry.hi[axis as usize] };
        let point = placed.transform_point3(glam::Vec3::new(pick(0), pick(1), pick(2))).to_array();
        include(&mut union, (point, point));
    }
    padded(union.expect("eight corners were folded in"))
}

/// Bounds around everything an entity offers a bullet (its brush models and its posed part boxes), or `None` when it
/// offers nothing.
pub(super) fn box_of_thing(
    attached_solids: &[AttachedSolidShape],
    posed: Option<&ServerModelCollision>,
    cmodels: &CollisionSubmodels,
) -> Option<([f32; 3], [f32; 3])> {
    let mut union = None;
    for solid in attached_solids {
        if let Some(collision_geometry) = cmodels.models.get(solid.collision_model_ref as usize) {
            include(&mut union, cmodel_bounds(collision_geometry, solid.origin, solid.turn));
        }
    }
    for bone in posed.iter().flat_map(|collision| collision.bones.iter()) {
        include(&mut union, bone_bounds(bone));
    }
    union
}

pub(super) fn geom_bounds(shape_data: &EntityRayShape, cmodels: &CollisionSubmodels) -> Option<([f32; 3], [f32; 3])> {
    box_of_thing(&shape_data.attached_solids, shape_data.collision.as_ref(), cmodels)
}
