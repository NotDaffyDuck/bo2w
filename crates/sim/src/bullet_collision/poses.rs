//! A player's collision body for one tick, and the riot-shield stand-in used when his shield has no boxes.

use super::collider::HITLOC_SHIELD;
use super::posed_coll::AuthorityDObjCollisionBone;
use crate::identities::LifeSequence;
use crate::world::ClientId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HitVolumeKind {
    #[default]
    Standing,
}

/// Where a player's body is and what a bullet can hit on it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerCollisionPose {
    pub client: ClientId,
    pub origin: [f32; 3],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub life_sequence: LifeSequence,
    pub hit_volume: HitVolumeKind,
    /// Oriented boxes for his body parts. Empty means "use the plain box between `mins` and `maxs`".
    pub bones: Vec<AuthorityDObjCollisionBone>,
    /// The riot shield he carries, if any.
    pub shield: Option<ShieldCover>,
    /// Where the shield's tag stands (its place and axes), if he carries one.
    pub shield_tag: Option<xmodel_runtime::CollisionBone>,
}

impl PlayerCollisionPose {
    /// True when one of his boxes is the shield's own.
    pub(super) fn has_shield_box(&self) -> bool {
        self.bones.iter().any(|bone| bone.part_classification == HITLOC_SHIELD)
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
