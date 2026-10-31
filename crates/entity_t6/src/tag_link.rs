//! BO2's `linkto.cpp` pose maths: the parent's rows at its tag
//! (`G_CalcTagParentAxis`), the child's fixed link (`G_SetFixedLinkNonPlayer`),
//! the offset a link stores (`G_CalcTagAxis`, `G_EntLinkToWithOffset`) and
//! the stand-still pose `G_GeneralLink` leaves behind.
//!
//! Matrices are BO2's row vectors: rows 0-2 are the rows, row 3 the spot,
//! and `a * b` puts `a` in `b`'s space.

use crate::entity_state::EntitySnapshot;
use crate::link::{EntityShared, axis_of_angles, set_angle, set_origin};
use crate::mover::axis_to_angles;
use crate::flight::path_kind;

const ZERO_VEC: [f32; 3] = [0.0; 3];
const UNIT_X: [f32; 3] = [1.0, 0.0, 0.0];
const UNIT_Y: [f32; 3] = [0.0, 1.0, 0.0];
const UNIT_Z: [f32; 3] = [0.0, 0.0, 1.0];

/// BO2's `vec3_t[4]`: an rows plus an spot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis43 {
    pub rows: [[f32; 3]; 3],
    pub spot: [f32; 3],
}

impl Axis43 {
    pub const IDENTITY: Axis43 = Axis43 {
        spot: ZERO_VEC,
        rows: [UNIT_X, UNIT_Y, UNIT_Z],
    };

    /// `AnglesToAxis` plus the spot in row 3.
    pub fn from_pose(spot: [f32; 3], heading: [f32; 3]) -> Self {
        Self { rows: axis_of_angles(heading), spot }
    }
}

/// One bone of a posed model (`PosedBone`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosedBone {
    pub spin: [f32; 4],
    pub shift_offset: [f32; 3],
    /// `2 / |spin|^2`, so a unit spin has 2.
    pub move_blend_amount: f32,
}

impl PosedBone {
    pub const IDENTITY: PosedBone = PosedBone {
        move_blend_amount: 2.0,
        shift_offset: ZERO_VEC,
        spin: [0.0, 0.0, 0.0, 1.0], // identity quaternion (x, y, z, w)
    };

    /// The bone's rows, as BO2 builds it inline from the spin.
    pub fn rows(self) -> [[f32; 3]; 3] {
        let [x, y, z, w] = self.spin;
        let tw = self.move_blend_amount;
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
pub fn matrix_multiply(lhs: [[f32; 3]; 3], rhs: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut product = [[0.0; 3]; 3];
    for (out_row, l_row) in product.iter_mut().zip(lhs) {
        for (col, cell) in out_row.iter_mut().enumerate() {
            *cell = l_row[0] * rhs[0][col] + l_row[1] * rhs[1][col] + l_row[2] * rhs[2][col];
        }
    }
    product
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
pub fn matrix_transform_vector43(point: [f32; 3], frame: &Axis43) -> [f32; 3] {
    let mut out = frame.spot;
    for (c, o) in out.iter_mut().enumerate() {
        *o += point[0] * frame.rows[0][c] + point[1] * frame.rows[1][c] + point[2] * frame.rows[2][c];
    }
    out
}

/// `MatrixMultiply43`: `a * b`, `a`'s spot carried through `b`.
pub fn matrix_multiply43(a: &Axis43, b: &Axis43) -> Axis43 {
    Axis43 { rows: matrix_multiply(a.rows, b.rows), spot: matrix_transform_vector43(a.spot, b) }
}

/// `MatrixInverseOrthogonal43`: the inverse of a rotation plus spot.
pub fn matrix_inverse_orthogonal43(m: &Axis43) -> Axis43 {
    let rows = matrix_transpose(m.rows);
    let neg = [-m.spot[0], -m.spot[1], -m.spot[2]];
    let stow_kind = Axis43 { rows, spot: [0.0; 3] };
    Axis43 { rows, spot: matrix_transform_vector43(neg, &stow_kind) }
}

/// `G_CalcTagParentAxis`: the frame a child rides in. With no tag (a
/// tag index below 0, or a player linked with
/// `link_useBaseAnglesForViewClamp`) it is the parent's own pose;
/// otherwise the bone's matrix in the parent's space.
pub fn tag_parent_axis(
    parent_pos: [f32; 3],
    parent_ang: [f32; 3],
    tag: Option<PosedBone>,
) -> Axis43 {
    let parent = Axis43::from_pose(parent_pos, parent_ang);
    match tag {
        None => parent,
        Some(mat) => Axis43 {
            rows: matrix_multiply(mat.rows(), parent.rows),
            spot: matrix_transform_vector43(mat.shift_offset, &parent),
        },
    }
}

/// `G_CalcTagParentRelAxis`: the parent frame seen from the frame the
/// link stored at link time (`tagInfo->parentInvAxis`).
pub fn tag_parent_rel_axis(parent_inv_axis: &Axis43, host_axis: &Axis43) -> Axis43 {
    matrix_multiply43(parent_inv_axis, host_axis)
}

/// How `G_SetFixedLinkNonPlayer` turns the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedLinkAngles {
    /// `FIXED_LINK_ANGLES`: all three heading follow.
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

/// `G_SetFixedLinkNonPlayer`: the child's world spot and heading from its
/// stored link offset and the parent frame. `heading` are the child's
/// current heading, kept for pitch and roll when only yaw follows.
pub fn fixed_link_non_player(
    link_axis: &Axis43,
    host_axis: &Axis43,
    mode: FixedLinkAngles,
    heading: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let world = matrix_multiply43(link_axis, host_axis);
    let heading = match mode {
        FixedLinkAngles::Full => axis_to_angles(world.rows),
        FixedLinkAngles::YawOnly => [heading[0], vectoyaw([world.rows[0][0], world.rows[0][1]]), heading[2]],
    };
    (world.spot, heading)
}

/// `G_CalcTagAxis`: the offset a link stores, the child's pose in the
/// parent frame. `angles_only` keeps the stored spot row as it was and
/// writes only the rows, as BO2 does.
pub fn calc_tag_axis(
    child_origin: [f32; 3],
    child_angles: [f32; 3],
    host_axis: &Axis43,
    angles_only: bool,
    stored: &Axis43,
) -> Axis43 {
    let child = Axis43::from_pose(child_origin, child_angles);
    if angles_only {
        return Axis43 {
            rows: matrix_multiply(child.rows, matrix_transpose(host_axis.rows)),
            spot: stored.spot,
        };
    }
    matrix_multiply43(&child, &matrix_inverse_orthogonal43(host_axis))
}

/// `G_EntLinkToWithOffset`: the stored offset is the given pose itself.
pub fn link_offset(origin_offset: [f32; 3], angles_offset: [f32; 3]) -> Axis43 {
    Axis43::from_pose(origin_offset, angles_offset)
}

/// What `G_GeneralLink` leaves on a linked non-player once it has placed
/// it: stood at the pose, with both trajectories `TR_INTERPOLATE` so the
/// client lerps between snapshots.
pub fn general_link_pose(es: &mut EntitySnapshot, r: &mut EntityShared, spot: [f32; 3], heading: [f32; 3]) {
    set_origin(es, r, spot);
    set_angle(es, r, heading);
    es.lerp.pos.tr_type = path_kind::BLEND;
    es.lerp.apos.tr_type = path_kind::BLEND;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| libm::fabsf(a[i] - b[i]) < 1.0e-3)
    }

    #[test]
    fn identity_bone_is_identity_axis() {
        assert_eq!(PosedBone::IDENTITY.rows(), Axis43::IDENTITY.rows);
        // A quarter turn about z, unit spin (move_blend_amount 2).
        let h = libm::sqrtf(0.5);
        let m = PosedBone { spin: [0.0, 0.0, h, h], shift_offset: [0.0; 3], move_blend_amount: 2.0 };
        let a = m.rows();
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
        let bone = PosedBone { shift_offset: [0.0, 0.0, 4.0], ..PosedBone::IDENTITY };
        let tagged = tag_parent_axis([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], Some(bone));
        assert!(close3(tagged.spot, [100.0, 0.0, 4.0]));
    }

    #[test]
    fn stored_offset_round_trips() {
        let parent = tag_parent_axis([30.0, -20.0, 8.0], [10.0, 45.0, 5.0], None);
        let stored = calc_tag_axis([40.0, -5.0, 12.0], [0.0, 80.0, 0.0], &parent, false, &Axis43::IDENTITY);
        let (o, a) = fixed_link_non_player(&stored, &parent, FixedLinkAngles::Full, [0.0; 3]);
        assert!(close3(o, [40.0, -5.0, 12.0]) && close3(a, [0.0, 80.0, 0.0]));
        let rel = tag_parent_rel_axis(&matrix_inverse_orthogonal43(&parent), &parent);
        assert!(close3(rel.spot, [0.0; 3]) && close3(rel.rows[0], [1.0, 0.0, 0.0]));
        let kept = calc_tag_axis([0.0; 3], [0.0; 3], &parent, true, &stored);
        assert_eq!(kept.spot, stored.spot);
    }

    #[test]
    fn general_link_interpolates() {
        let mut es = EntitySnapshot::default();
        let mut r = EntityShared::default();
        general_link_pose(&mut es, &mut r, [1.0, 2.0, 3.0], [0.0, 90.0, 0.0]);
        assert_eq!(es.lerp.pos.tr_type, path_kind::BLEND);
        assert_eq!(es.lerp.apos.tr_type, path_kind::BLEND);
        assert_eq!(es.lerp.pos.base_point, [1.0, 2.0, 3.0]);
        assert_eq!(r.live_angles, [0.0, 90.0, 0.0]);
        assert_eq!(es.lerp.pos.began_at, 0);
    }
}
