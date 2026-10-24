//! BO2's `linkto.cpp` pose maths: the parent's axis at its tag
//! (`G_CalcTagParentAxis`), the child's fixed link (`G_SetFixedLinkNonPlayer`),
//! the offset a link stores (`G_CalcTagAxis`, `G_EntLinkToWithOffset`) and
//! the stand-still pose `G_GeneralLink` leaves behind.
//!
//! Matrices are BO2's row vectors: rows 0-2 are the axis, row 3 the origin,
//! and `a * b` puts `a` in `b`'s space.

use crate::entity_state::EntityState;
use crate::link::{EntityShared, angles_to_axis, set_angle, set_origin};
use crate::mover::axis_to_angles;
use crate::trajectory::tr_type;

/// BO2's `vec3_t[4]`: an axis plus an origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis43 {
    pub axis: [[f32; 3]; 3],
    pub origin: [f32; 3],
}

impl Axis43 {
    pub const IDENTITY: Self = Self {
        axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        origin: [0.0; 3],
    };

    /// `AnglesToAxis` plus the origin in row 3.
    pub fn from_pose(origin: [f32; 3], angles: [f32; 3]) -> Self {
        Self { axis: angles_to_axis(angles), origin }
    }
}

/// One bone of a posed model (`DObjAnimMat`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DObjAnimMat {
    pub quat: [f32; 4],
    pub trans: [f32; 3],
    /// `2 / |quat|^2`, so a unit quat has 2.
    pub trans_weight: f32,
}

impl DObjAnimMat {
    pub const IDENTITY: Self = Self {
        quat: [0.0, 0.0, 0.0, 1.0],
        trans: [0.0; 3],
        trans_weight: 2.0,
    };

    /// The bone's axis, as BO2 builds it inline from the quat.
    pub fn axis(self) -> [[f32; 3]; 3] {
        let [x, y, z, w] = self.quat;
        let tw = self.trans_weight;
        let xw = x * tw;
        let yw = y * tw;
        let xx = x * xw;
        let zz = z * z * tw;
        let zw = w * z * tw;
        [
            [1.0 - (zz + y * yw), zw + y * xw, z * xw - w * yw],
            [y * xw - zw, 1.0 - (zz + xx), z * yw + w * xw],
            [w * yw + z * xw, z * yw - w * xw, 1.0 - (y * yw + xx)],
        ]
    }
}

/// `MatrixMultiply`: `a * b`.
pub fn matrix_multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (o, r) in out.iter_mut().zip(a) {
        for (c, v) in o.iter_mut().enumerate() {
            *v = r[0] * b[0][c] + r[1] * b[1][c] + r[2] * b[2][c];
        }
    }
    out
}

/// `MatrixTranspose`.
pub fn matrix_transpose(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

/// `MatrixTransformVector43`: the point `v` in `m`'s space, to world.
pub fn matrix_transform_vector43(v: [f32; 3], m: &Axis43) -> [f32; 3] {
    let mut out = m.origin;
    for (c, o) in out.iter_mut().enumerate() {
        *o += v[0] * m.axis[0][c] + v[1] * m.axis[1][c] + v[2] * m.axis[2][c];
    }
    out
}

/// `MatrixMultiply43`: `a * b`, `a`'s origin carried through `b`.
pub fn matrix_multiply43(a: &Axis43, b: &Axis43) -> Axis43 {
    Axis43 { axis: matrix_multiply(a.axis, b.axis), origin: matrix_transform_vector43(a.origin, b) }
}

/// `MatrixInverseOrthogonal43`: the inverse of a rotation plus origin.
pub fn matrix_inverse_orthogonal43(m: &Axis43) -> Axis43 {
    let axis = matrix_transpose(m.axis);
    let neg = [-m.origin[0], -m.origin[1], -m.origin[2]];
    let inv = Axis43 { axis, origin: [0.0; 3] };
    Axis43 { axis, origin: matrix_transform_vector43(neg, &inv) }
}

/// `G_CalcTagParentAxis`: the frame a child rides in. With no tag (a
/// tag index below 0, or a player linked with
/// `link_useBaseAnglesForViewClamp`) it is the parent's own pose;
/// otherwise the bone's matrix in the parent's space.
pub fn tag_parent_axis(
    parent_origin: [f32; 3],
    parent_angles: [f32; 3],
    tag: Option<DObjAnimMat>,
) -> Axis43 {
    let parent = Axis43::from_pose(parent_origin, parent_angles);
    match tag {
        None => parent,
        Some(mat) => Axis43 {
            axis: matrix_multiply(mat.axis(), parent.axis),
            origin: matrix_transform_vector43(mat.trans, &parent),
        },
    }
}

/// `G_CalcTagParentRelAxis`: the parent frame seen from the frame the
/// link stored at link time (`tagInfo->parentInvAxis`).
pub fn tag_parent_rel_axis(parent_inv_axis: &Axis43, parent_axis: &Axis43) -> Axis43 {
    matrix_multiply43(parent_inv_axis, parent_axis)
}

/// How `G_SetFixedLinkNonPlayer` turns the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedLinkAngles {
    /// `FIXED_LINK_ANGLES`: all three angles follow.
    Full,
    /// `FIXED_LINK_ANGLES_YAW_ONLY`: only yaw follows.
    YawOnly,
}

/// `vectoyaw`: yaw of `v` in [0, 360); a zero vector gives 0.
pub fn vectoyaw(v: [f32; 2]) -> f32 {
    if v[1] == 0.0 && v[0] == 0.0 {
        return 0.0;
    }
    let yaw = (libm::atan2(v[1] as f64, v[0] as f64) as f32) * 180.0 * crate::angles::INV_PI;
    (if yaw < 0.0 { 360.0 } else { 0.0 }) + yaw
}

/// `G_SetFixedLinkNonPlayer`: the child's world origin and angles from its
/// stored link offset and the parent frame. `angles` are the child's
/// current angles, kept for pitch and roll when only yaw follows.
pub fn fixed_link_non_player(
    link_axis: &Axis43,
    parent_axis: &Axis43,
    mode: FixedLinkAngles,
    angles: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let world = matrix_multiply43(link_axis, parent_axis);
    let angles = match mode {
        FixedLinkAngles::Full => axis_to_angles(world.axis),
        FixedLinkAngles::YawOnly => [angles[0], vectoyaw([world.axis[0][0], world.axis[0][1]]), angles[2]],
    };
    (world.origin, angles)
}

/// `G_CalcTagAxis`: the offset a link stores, the child's pose in the
/// parent frame. `angles_only` keeps the stored origin row as it was and
/// writes only the axis, as BO2 does.
pub fn calc_tag_axis(
    child_origin: [f32; 3],
    child_angles: [f32; 3],
    parent_axis: &Axis43,
    angles_only: bool,
    stored: &Axis43,
) -> Axis43 {
    let child = Axis43::from_pose(child_origin, child_angles);
    if angles_only {
        return Axis43 {
            axis: matrix_multiply(child.axis, matrix_transpose(parent_axis.axis)),
            origin: stored.origin,
        };
    }
    matrix_multiply43(&child, &matrix_inverse_orthogonal43(parent_axis))
}

/// `G_EntLinkToWithOffset`: the stored offset is the given pose itself.
pub fn link_offset(origin_offset: [f32; 3], angles_offset: [f32; 3]) -> Axis43 {
    Axis43::from_pose(origin_offset, angles_offset)
}

/// What `G_GeneralLink` leaves on a linked non-player once it has placed
/// it: stood at the pose, with both trajectories `TR_INTERPOLATE` so the
/// client lerps between snapshots.
pub fn general_link_pose(es: &mut EntityState, r: &mut EntityShared, origin: [f32; 3], angles: [f32; 3]) {
    set_origin(es, r, origin);
    set_angle(es, r, angles);
    es.lerp.pos.tr_type = tr_type::INTERPOLATE;
    es.lerp.apos.tr_type = tr_type::INTERPOLATE;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| libm::fabsf(a[i] - b[i]) < 1.0e-3)
    }

    #[test]
    fn identity_bone_is_identity_axis() {
        assert_eq!(DObjAnimMat::IDENTITY.axis(), Axis43::IDENTITY.axis);
        // A quarter turn about z, unit quat (trans_weight 2).
        let h = libm::sqrtf(0.5);
        let m = DObjAnimMat { quat: [0.0, 0.0, h, h], trans: [0.0; 3], trans_weight: 2.0 };
        let a = m.axis();
        assert!(close3(a[0], [0.0, 1.0, 0.0]) && close3(a[1], [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn tag_frame_and_fixed_link() {
        let parent = tag_parent_axis([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], None);
        // Offset 10 forward: the parent faces +y, so the child lands at y 10.
        let offset = link_offset([10.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        let (o, a) = fixed_link_non_player(&offset, &parent, FixedLinkAngles::Full, [0.0; 3]);
        assert!(close3(o, [100.0, 10.0, 0.0]) && libm::fabsf(a[1] - 90.0) < 1.0e-3);
        let (_, a) = fixed_link_non_player(&offset, &parent, FixedLinkAngles::YawOnly, [5.0, 0.0, 7.0]);
        assert!(close3(a, [5.0, 90.0, 7.0]));
        // A bone 4 up on the parent moves the frame with it.
        let bone = DObjAnimMat { trans: [0.0, 0.0, 4.0], ..DObjAnimMat::IDENTITY };
        let tagged = tag_parent_axis([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], Some(bone));
        assert!(close3(tagged.origin, [100.0, 0.0, 4.0]));
    }

    #[test]
    fn stored_offset_round_trips() {
        let parent = tag_parent_axis([30.0, -20.0, 8.0], [10.0, 45.0, 5.0], None);
        let stored = calc_tag_axis([40.0, -5.0, 12.0], [0.0, 80.0, 0.0], &parent, false, &Axis43::IDENTITY);
        let (o, a) = fixed_link_non_player(&stored, &parent, FixedLinkAngles::Full, [0.0; 3]);
        assert!(close3(o, [40.0, -5.0, 12.0]) && close3(a, [0.0, 80.0, 0.0]));
        let rel = tag_parent_rel_axis(&matrix_inverse_orthogonal43(&parent), &parent);
        assert!(close3(rel.origin, [0.0; 3]) && close3(rel.axis[0], [1.0, 0.0, 0.0]));
        let kept = calc_tag_axis([0.0; 3], [0.0; 3], &parent, true, &stored);
        assert_eq!(kept.origin, stored.origin);
    }

    #[test]
    fn general_link_interpolates() {
        let mut es = EntityState::default();
        let mut r = EntityShared::default();
        general_link_pose(&mut es, &mut r, [1.0, 2.0, 3.0], [0.0, 90.0, 0.0]);
        assert_eq!(es.lerp.pos.tr_type, tr_type::INTERPOLATE);
        assert_eq!(es.lerp.apos.tr_type, tr_type::INTERPOLATE);
        assert_eq!(es.lerp.pos.tr_base, [1.0, 2.0, 3.0]);
        assert_eq!(r.current_angles, [0.0, 90.0, 0.0]);
        assert_eq!(es.lerp.pos.tr_time, 0);
    }
}
