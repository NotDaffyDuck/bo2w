use std::collections::HashMap;

use bevy::prelude::*;
use xmodel_runtime::{BonePlacement, bone_orientation};

/// Where a posed player's head ended up this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimatedHeadMode {
    Exact(Vec3),
    MissingBodyOrSkull,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimatedBody {
    pub ent_ix: u16,
    pub head: AnimatedHeadMode,
}

/// The players posed so far this frame; emptied by [`start_skeleton_pose_tick`].
#[derive(Resource, Default)]
pub struct AnimatedBodyFrame {
    posed: Vec<AnimatedBody>,
}

impl AnimatedBodyFrame {
    pub fn clear(&mut self) {
        self.posed = Vec::new();
    }

    pub fn post_body(&mut self, player: AnimatedBody) {
        self.posed.extend(Some(player));
    }

    pub fn iter(&self) -> impl Iterator<Item = &AnimatedBody> {
        self.posed.as_slice().iter()
    }
}

/// One DObj's pose as FX bolting reads it: the entity's own orientation and
/// one orientation per bone, all in world space.
#[derive(Clone, Debug)]
struct PublishedPose {
    root: BonePlacement,
    valid_now: bool,
    bone_poses: Vec<BonePlacement>,
    teleported: bool,
}

/// Why a bone orientation could not be published or looked up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPoseDecline {
    DObjAbsent,
    BoneTotalTooBig,
    Bone(fx_t6::runtime::BoneOrientationRefuse),
    Frame(xmodel_runtime::BonePlacementFault),
}

/// The DObj poses published this frame, by DObj number.
#[derive(Resource, Default)]
pub struct HostSkeletonFrame {
    poses: HashMap<u32, PublishedPose>,
}

impl HostSkeletonFrame {
    pub fn clear(&mut self) {
        self.poses = HashMap::new();
    }

    /// Records `dobj`'s pose: `joint_global_transform` holds each bone relative to the
    /// entity, `ent_world_mat` places the entity. Nothing is stored when any
    /// matrix cannot be turned into an orientation.
    pub fn publish(
        &mut self,
        dobj: u32,
        pose_now_flag: bool,
        upcoming_body_bits: u32,
        ent_world_mat: Mat4,
        joint_global_transform: &[Mat4],
    ) -> Result<(), HostPoseDecline> {
        let orient =
            |placed: Mat4| bone_orientation(&[placed], 0).map_err(HostPoseDecline::Frame);
        let root = orient(ent_world_mat)?;
        let mut bone_poses = Vec::with_capacity(joint_global_transform.len());
        for local in joint_global_transform {
            bone_poses.push(orient(ent_world_mat * *local)?);
        }
        let teleported = fx_t6::runtime::bolt_spawn_teleport_bit(dobj, upcoming_body_bits);
        let published = PublishedPose {
            bone_poses,
            root,
            teleported,
            valid_now: pose_now_flag,
        };
        self.poses.insert(dobj, published);
        Ok(())
    }

    /// The orientation a bolt on `bone` of `dobj` follows.
    pub fn find(
        &self,
        dobj: u32,
        bone: i32,
    ) -> Result<fx::AttachPointFrame, HostPoseDecline> {
        use fx_t6::runtime::{JointFacingPath, get_bone_orientation_route};

        let Some(pose) = self.poses.get(&dobj) else {
            return Err(HostPoseDecline::DObjAbsent);
        };
        let Ok(bone_total) = u8::try_from(pose.bone_poses.len()) else {
            return Err(HostPoseDecline::BoneTotalTooBig);
        };
        let routed = get_bone_orientation_route(dobj, pose.valid_now, bone, Some(bone_total));
        let found = match routed.map_err(HostPoseDecline::Bone)? {
            JointFacingPath::SkeletonJoint(index) => pose.bone_poses[usize::from(index)],
            JointFacingPath::OwnPose => pose.root,
        };
        Ok(fx::AttachPointFrame {
            axis: found.basis_dirs,
            origin: found.at,
        })
    }

    /// The bolt's state for FX, or `None` when `dobj` has no published pose.
    /// A refused bone still reports the teleport flag, with no orientation.
    pub fn find_live_mount(&self, dobj_no: u32, bone_no: u16) -> Option<fx::SettledAttachFrame> {
        let teleported = self.poses.get(&dobj_no)?.teleported;
        Some(fx::SettledAttachFrame {
            orientation: self.find(dobj_no, i32::from(bone_no)).ok(),
            client_actor_teleport: teleported,
        })
    }
}

/// Start of a frame: forget last frame's poses and players.
pub fn start_skeleton_pose_tick(mut bolts_in: ResMut<'_, HostSkeletonFrame>, mut heads: ResMut<'_, AnimatedBodyFrame>) {
    bolts_in.clear();
    heads.clear();
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn players_are_listed_in_publishing_order() {
        let mut players = AnimatedBodyFrame::default();
        let nobody = AnimatedHeadMode::MissingBodyOrSkull;
        players.post_body(AnimatedBody {
            head: nobody,
            ent_ix: 4,
        });
        players.post_body(AnimatedBody {
            head: nobody,
            ent_ix: 2,
        });
        let listed: Vec<_> = players.iter().map(|p| p.ent_ix).collect();
        assert_eq!(listed, [4, 2]);
        players.clear();
        assert_eq!(players.iter().count(), 0);
    }

    #[test]
    fn a_dobj_nobody_published_cannot_be_resolved() {
        let frame = HostSkeletonFrame::default();
        assert_eq!(
            frame.find(7, 0).err(),
            Some(HostPoseDecline::DObjAbsent)
        );
        assert!(frame.find_live_mount(7, 0).is_none());
    }

    #[test]
    fn the_teleport_bit_lasts_until_the_frame_is_cleared() {
        let mut poses = HostSkeletonFrame::default();
        let (dobj, eflags) = (9, 0x1234);
        let ent_mat = Mat4::IDENTITY;
        let two_bones = vec![ent_mat, ent_mat];
        let published = poses.publish(dobj, true, eflags, ent_mat, &two_bones);
        assert!(published.is_ok());
        let live = poses.find_live_mount(dobj, 1).expect("published");
        assert_eq!(
            live.client_actor_teleport,
            fx_t6::runtime::bolt_spawn_teleport_bit(dobj, eflags)
        );
        poses.clear();
        assert!(poses.find_live_mount(dobj, 1).is_none());
    }

    #[test]
    fn a_bone_past_the_published_ones_has_no_orientation() {
        let mut poses = HostSkeletonFrame::default();
        poses
            .publish(3, true, 0, Mat4::IDENTITY, &[Mat4::IDENTITY])
            .expect("matrices are rigid");
        let live = poses.find_live_mount(3, 40).expect("published");
        assert!(live.orientation.is_none());
    }
}
