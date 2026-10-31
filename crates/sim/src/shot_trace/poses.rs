//! A player's collision body for one tick, and the riot-shield stand-in used when his shield has no boxes.

use super::collider::HITLOC_SHIELD;
use crate::match_ids::LifeCount;
use crate::sim_parts::SeatNo;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HitboxShapeKind {
    #[default]
    Standing,
}

/// Where a player's body is and what a bullet can hit on it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerHitboxPose {
    pub client: SeatNo,
    pub origin: [f32; 3],
    /// The way he faces (BO2's entity yaw), in degrees. His boxes are already turned by it.
    pub yaw: f32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub life_sequence: LifeCount,
    pub hit_volume: HitboxShapeKind,
    /// FaceAligned boxes for his body parts. Empty means "use the plain box between `mins` and `maxs`".
    pub joints: Vec<xmodel_runtime::HitShapeBone>,
    /// The riot shield he carries, if any.
    pub shield: Option<ShieldCover>,
    /// Where the shield's tag stands (its place and axes), if he carries one.
    pub shield_tag: Option<xmodel_runtime::HitShapeBone>,
}

/// BO2's angle wrap (`AngleDelta`, `AngleNormalize180`): `degrees` brought into -180..180.
pub(super) fn wrap_180(degrees: f32) -> f32 {
    let turns = degrees * 0.002_777_777_8;
    (turns - (turns + 0.5).floor()) * 360.0
}

impl PlayerHitboxPose {
    /// This pose carried `progress` of the way toward `later`, the way BO2's `SV_GetClientPositionsAtTimeInternal`
    /// blends a client between two kept frames: the origin in a straight line, the facing the short way round. As in
    /// BO2, only where he stands and faces moves; his boxes keep this pose's shape and go along with him.
    #[must_use]
    pub fn blended_toward(&self, later: &Self, progress: f32) -> Self {
        let origin: [f32; 3] = std::array::from_fn(|i| (later.origin[i] - self.origin[i]) * progress + self.origin[i]);
        let yaw = wrap_180(wrap_180(later.yaw - self.yaw) * progress + self.yaw);
        let turn = glam::Quat::from_rotation_z((yaw - self.yaw).to_radians());
        let carry = glam::Mat4::from_translation(glam::Vec3::from_array(origin))
            * glam::Mat4::from_quat(turn)
            * glam::Mat4::from_translation(-glam::Vec3::from_array(self.origin));
        let moved = |bone: &xmodel_runtime::HitShapeBone| xmodel_runtime::HitShapeBone {
            center: carry.transform_point3(glam::Vec3::from_array(bone.center)).to_array(),
            axes: bone.axes.map(|axis| carry.transform_vector3(glam::Vec3::from_array(axis)).to_array()),
            ..bone.clone()
        };
        let shield = self.shield.map(|cover| {
            let facing = turn * glam::Vec3::new(cover.facing[0], cover.facing[1], 0.0);
            ShieldCover { facing: [facing.x, facing.y], ..cover }
        });
        Self {
            origin,
            yaw,
            joints: self.joints.iter().map(moved).collect(),
            shield,
            shield_tag: self.shield_tag.as_ref().map(moved),
            ..self.clone()
        }
    }

    /// True when one of his boxes is the shield's own.
    pub(super) fn has_shield_box(&self) -> bool {
        self.joints.iter().any(|bone| bone.body_part_category == HITLOC_SHIELD)
    }

    /// True when a bullet travelling along `dir` should be counted as hitting his shield even though no box of the
    /// shield exists: he carries one, it has no boxes, and the bullet comes from the side it faces. (With boxes, the
    /// bullet meets the shield or the body directly and needs no help.)
    pub(super) fn shield_stands_in_for(&self, dir: [f32; 3]) -> bool {
        !self.has_shield_box() && self.shield.is_some_and(|cover| cover.stops(dir))
    }
}

/// A riot shield on a player whose model gives it no boxes. BO2's bullets meet a shield as a part of the body (a box
/// on the model in his hand or on his back that outranks the rest); this stands in for that box when it is missing.
/// A bullet coming from inside the shield's angle (his forward for a shield in his hands, his back for one stowed)
/// is taken for a hit on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShieldCover {
    /// The way the shield faces: flat, unit length.
    pub facing: [f32; 2],
    /// The cosine of the protection angle.
    pub cos_limit: f32,
}

impl ShieldCover {
    /// True when something travelling along `dir` comes at him from the side the shield faces, within its angle.
    pub fn stops(&self, dir: [f32; 3]) -> bool {
        let toward = [-dir[0], -dir[1]];
        let len = (toward[0] * toward[0] + toward[1] * toward[1]).sqrt();
        len > 1e-6 && (toward[0] * self.facing[0] + toward[1] * self.facing[1]) / len >= self.cos_limit
    }
}
