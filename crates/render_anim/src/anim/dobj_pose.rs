use std::collections::HashMap;

use bevy::prelude::*;
use xmodel_runtime::{DObjBoneOrientation, bone_orientation};

/// Where a posed player's head ended up this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PosedPlayerHead {
    Exact(Vec3),
    NoDObjOrHead,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosedPlayer {
    pub entnum: u16,
    pub head: PosedPlayerHead,
}

/// The players posed so far this frame; emptied by [`begin_dobj_pose_frame`].
#[derive(Resource, Default)]
pub struct PosedPlayerFrame {
    players: Vec<PosedPlayer>,
}

impl PosedPlayerFrame {
    pub fn clear(&mut self) {
        self.players.clear();
    }

    pub fn publish(&mut self, player: PosedPlayer) {
        self.players.push(player);
    }

    pub fn iter(&self) -> impl Iterator<Item = &PosedPlayer> {
        self.players.iter()
    }
}

/// One DObj's pose as FX bolting reads it: the entity's own orientation and
/// one orientation per bone, all in world space.
#[derive(Clone, Debug)]
struct PublishedPose {
    current_valid: bool,
    centity_teleport: bool,
    entity: DObjBoneOrientation,
    bones: Vec<DObjBoneOrientation>,
}

/// Why a bone orientation could not be published or looked up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostDObjPoseRefuse {
    MissingDObj,
    BoneCountOverflow,
    Bone(fx_t6::runtime::BoneOrientationRefuse),
    Frame(xmodel_runtime::DObjBoneOrientationError),
}

/// The DObj poses published this frame, by DObj number.
#[derive(Resource, Default)]
pub struct HostDObjPoseFrame {
    by_dobj: HashMap<u32, PublishedPose>,
}

impl HostDObjPoseFrame {
    pub fn clear(&mut self) {
        self.by_dobj.clear();
    }

    /// Records `dobj`'s pose: `bone_world` holds each bone relative to the
    /// entity, `entity_world` places the entity. Nothing is stored when any
    /// matrix cannot be turned into an orientation.
    pub fn publish(
        &mut self,
        dobj: u32,
        current_valid: bool,
        next_state_eflags: u32,
        entity_world: Mat4,
        bone_world: &[Mat4],
    ) -> Result<(), HostDObjPoseRefuse> {
        let orient = |matrix: Mat4| {
            bone_orientation(&[matrix], 0).map_err(HostDObjPoseRefuse::Frame)
        };
        let entity = orient(entity_world)?;
        let bones = bone_world
            .iter()
            .map(|matrix| orient(entity_world * *matrix))
            .collect::<Result<Vec<_>, _>>()?;
        let centity_teleport = fx_t6::runtime::bolt_spawn_teleport_bit(dobj, next_state_eflags);
        self.by_dobj.insert(
            dobj,
            PublishedPose {
                current_valid,
                centity_teleport,
                entity,
                bones,
            },
        );
        Ok(())
    }

    /// The orientation a bolt on `bone` of `dobj` follows.
    pub fn resolve(
        &self,
        dobj: u32,
        bone: i32,
    ) -> Result<fx::FxBoltOrientation, HostDObjPoseRefuse> {
        use fx_t6::runtime::{FxGetBoneOrientationRoute as Route, get_bone_orientation_route};

        let pose = self
            .by_dobj
            .get(&dobj)
            .ok_or(HostDObjPoseRefuse::MissingDObj)?;
        let bone_count = u8::try_from(pose.bones.len())
            .map_err(|_| HostDObjPoseRefuse::BoneCountOverflow)?;
        let route = get_bone_orientation_route(dobj, pose.current_valid, bone, Some(bone_count))
            .map_err(HostDObjPoseRefuse::Bone)?;
        let found = match route {
            Route::EntityPose => pose.entity,
            Route::DObjBone(index) => pose.bones[usize::from(index)],
        };
        Ok(fx::FxBoltOrientation {
            origin: found.origin,
            axis: found.axis,
        })
    }

    /// The bolt's state for FX, or `None` when `dobj` has no published pose.
    /// A refused bone still reports the teleport flag, with no orientation.
    pub fn resolve_live_bolt(&self, dobj: u32, bone: u16) -> Option<fx::FxResolvedBoltPose> {
        let pose = self.by_dobj.get(&dobj)?;
        Some(fx::FxResolvedBoltPose {
            centity_teleport: pose.centity_teleport,
            orientation: self.resolve(dobj, i32::from(bone)).ok(),
        })
    }
}

/// Start of a frame: forget last frame's poses and players.
pub fn begin_dobj_pose_frame(
    mut dobj_poses: ResMut<HostDObjPoseFrame>,
    mut posed_players: ResMut<PosedPlayerFrame>,
) {
    dobj_poses.clear();
    posed_players.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_kept_in_publish_order_until_cleared() {
        let mut frame = PosedPlayerFrame::default();
        for entnum in [4, 2] {
            frame.publish(PosedPlayer {
                entnum,
                head: PosedPlayerHead::NoDObjOrHead,
            });
        }
        assert_eq!(frame.iter().map(|p| p.entnum).collect::<Vec<_>>(), [4, 2]);
        frame.clear();
        assert_eq!(frame.iter().count(), 0);
    }

    #[test]
    fn an_unpublished_dobj_is_missing() {
        let frame = HostDObjPoseFrame::default();
        assert_eq!(
            frame.resolve(7, 0).err(),
            Some(HostDObjPoseRefuse::MissingDObj)
        );
        assert!(frame.resolve_live_bolt(7, 0).is_none());
    }

    #[test]
    fn a_published_dobj_reports_its_teleport_bit_and_goes_when_cleared() {
        let mut frame = HostDObjPoseFrame::default();
        let eflags = 0x1234;
        frame
            .publish(9, true, eflags, Mat4::IDENTITY, &[Mat4::IDENTITY; 2])
            .unwrap();
        let live = frame.resolve_live_bolt(9, 1).expect("published");
        assert_eq!(
            live.centity_teleport,
            fx_t6::runtime::bolt_spawn_teleport_bit(9, eflags)
        );
        frame.clear();
        assert!(frame.resolve_live_bolt(9, 1).is_none());
    }

    #[test]
    fn a_bone_past_the_published_ones_has_no_orientation() {
        let mut frame = HostDObjPoseFrame::default();
        frame
            .publish(3, true, 0, Mat4::IDENTITY, &[Mat4::IDENTITY])
            .unwrap();
        let live = frame.resolve_live_bolt(3, 40).expect("published");
        assert!(live.orientation.is_none());
    }
}
