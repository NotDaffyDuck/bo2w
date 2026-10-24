//! Resources the first-person presenter keeps between frames, and the step
//! that turns the equipped weapon's playing clips into this frame's hand poses.

use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use bevy::prelude::*;

use crate::anim::fpv::{
    EquippedFpv, FpvAuthoritySample, FpvPresentState, tick_equipped_fpv_with_predicted_fire,
};
use crate::anim::fpv_pose::{FpvBoltFrame, PosedClip};
use crate::anim::fpv_prepared::FpvRigSet;
use crate::anim::fpv_rig::{FpvHandPose, PreparedFpvRig};
use crate::anim::viewmodel_controller::ViewmodelController;
use asset_game::WeaponAnimSlot;
use assets::FpvMeshIndex;

#[derive(Resource, Default)]
pub struct FpvPresentCursor(pub FpvPresentState);

#[derive(Resource, Default)]
pub struct PendingFpvSpawn(pub Option<PendingFpvSpawnRequest>);

#[derive(Clone, Debug)]
pub struct PendingFpvSpawnRequest {
    pub gun_index: FpvMeshIndex,
    pub catalog_id: u64,
    pub weapon_id: u32,
    pub parent_weapon: u32,
}

/// Notetrack names the held weapon's clips raised, waiting for the audio and
/// effects side to pick them up.
#[derive(Resource, Default)]
pub struct PendingFpvNotetracks {
    pub weapon: u32,
    pub names: Vec<String>,
}

#[derive(Resource, Default)]
pub struct FpvHeldSettled(pub Option<u32>);

#[derive(Resource, Default)]
pub struct FpvHeldLife(pub Option<u32>);

/// Where the first-person attachment points are this frame, per hand, for the
/// effects that follow them.
#[derive(Resource, Default)]
pub struct FpvBoltTargets {
    pub pose: [Option<FpvBoltFrame>; 2],
    pub tracker_screen: Option<[Vec3; 3]>,
    pub tracker_light: Option<fx::FxBoltTarget>,
    /// bo2mp: the held gun's `tag_hud` (the Target Finder screen plane).
    pub hud: Option<fx::FxBoltTarget>,
    pub flash: [Option<fx::FxBoltTarget>; 2],
    pub brass: [Option<fx::FxBoltTarget>; 2],
    pub knife: [Option<fx::FxBoltTarget>; 2],
    pub laser: [Option<fx::FxBoltTarget>; 2],
    /// bo2mp: the gun's `tag_target` (BO2's `playviewmodelfx`).
    pub target: [Option<fx::FxBoltTarget>; 2],
}

impl FpvBoltTargets {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn set_pose(&mut self, hand: usize, bolt: FpvBoltFrame) {
        self.pose[hand] = Some(bolt);
    }
}

/// Why a frame produced no viewmodel pose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FpvPoseRefuse {
    CatalogMissing,
    NoActiveClips,
    EyePoseFailed {
        gun_xmodel: String,
    },
    DependencyUnresolved {
        weapon_id: u32,
        role: &'static str,
        name: String,
    },
}

#[derive(Default)]
pub enum FpvPoseKind {
    /// Nothing is held, or it is not shown.
    #[default]
    Hide,
    Refuse(FpvPoseRefuse),
    Posed(FpvPosedFrame),
}

/// What one frame asked of the rig: bones per hand, and the lens the right
/// hand carries. No geometry: the surfaces this rig draws were settled when it
/// was prepared, and the vertices are written straight into the published plan.
pub struct FpvPosedFrame {
    pub poses: [Option<FpvHandPose>; 2],
    pub lens: Mat4,
    pub idle_sampled: bool,
    pub notetracks: Vec<String>,
}

#[derive(Resource, Default)]
pub struct FpvPoseProduct {
    pub drawgun: Option<i32>,
    pub kind: FpvPoseKind,
}

pub struct FpvGenerateArgs<'a> {
    pub dt: f32,
    pub equipped: &'a mut EquippedFpv,
    pub rigs: &'a FpvRigSet,
    pub active: &'a mut Option<Arc<PreparedFpvRig>>,
    pub cursor: &'a mut FpvPresentState,
    pub rocket: bool,
    pub melee: bool,
    /// bo2mp: aimed far enough that the sight shows its stripped model.
    pub ads: bool,
    pub sample: Option<FpvAuthoritySample>,
    pub predicted_fire: bool,
    pub dual: bool,
    pub dual_offset: Option<f32>,
    /// bo2zm: how far through a low climb that plays the gun's climb camera
    /// animation (0 to 1), else `None`.
    pub mantle_camera: Option<f32>,
}

/// The clips a controller has playing, as the rig wants them.
fn playing_clips(controller: &ViewmodelController) -> Vec<PosedClip<'_>> {
    controller
        .active_anims()
        .map(|anim| PosedClip {
            node: anim.node,
            clip: anim.clip,
            time: anim.time,
            weight: anim.weight,
        })
        .collect()
}

/// The climb camera clip, `fraction` of the way through, when the weapon has one.
fn climb_camera_clip(controller: &ViewmodelController, fraction: f32) -> Option<PosedClip<'_>> {
    let slot = WeaponAnimSlot::CameraMantle;
    let clip = controller.weapon().clip(slot)?;
    Some(PosedClip {
        node: slot.index(),
        clip,
        time: fraction.clamp(0.0, 1.0) * clip.duration(),
        weight: 1.0,
    })
}

/// How far the off hand's gun is pushed aside, in eye space.
fn off_hand_shift(dual_offset: Option<f32>) -> Vec3 {
    match dual_offset {
        Some(offset) if offset != 0.0 => Vec3::from_array(
            weapon_t6::placement::dual_wield_view_model_origin_add(1, [-1.0, 0.0, 0.0], offset),
        ),
        _ => Vec3::ZERO,
    }
}

fn eye_pose_failed(equipped: &EquippedFpv) -> FpvPoseKind {
    FpvPoseKind::Refuse(FpvPoseRefuse::EyePoseFailed {
        gun_xmodel: equipped.gun_xmodel.clone(),
    })
}

pub fn generate_fpv_pose(args: FpvGenerateArgs<'_>) -> FpvPoseKind {
    let FpvGenerateArgs {
        dt,
        equipped,
        rigs,
        active,
        cursor,
        rocket,
        melee,
        ads,
        sample,
        predicted_fire,
        dual,
        dual_offset,
        mantle_camera,
    } = args;
    let (_pose, notetracks) =
        tick_equipped_fpv_with_predicted_fire(equipped, cursor, sample, predicted_fire, dt);

    let right = playing_clips(&equipped.controller);
    if right.is_empty() {
        return FpvPoseKind::Refuse(FpvPoseRefuse::NoActiveClips);
    }
    let left = match (dual, equipped.left.as_ref()) {
        (true, Some(controller)) => playing_clips(controller),
        _ => Vec::new(),
    };
    let left_drawn = !left.is_empty();
    let camera =
        mantle_camera.and_then(|fraction| climb_camera_clip(&equipped.controller, fraction));

    let Some(rig) = rigs.pick(rocket, left_drawn, melee, ads) else {
        *active = None;
        return eye_pose_failed(equipped);
    };
    let already_active = active
        .as_ref()
        .is_some_and(|current| current.generation() == rig.generation());
    if !already_active {
        *active = Some(Arc::clone(rig));
    }
    let logging = crate::anim::fpv::vm_log_enabled();

    // bo2zm: Black Ops II dual wield: both hands' clips on the one rig.
    if rigs.combined && left_drawn {
        let Some(pose) = rig.pose_combined(&right, &left, camera.as_ref()) else {
            return eye_pose_failed(equipped);
        };
        if logging {
            let everything: Vec<PosedClip<'_>> = right.iter().chain(&left).cloned().collect();
            crate::anim::fpv::vm_log_note(&format!(
                "POSE dual {}",
                rig.debug_pose_line(&everything, &pose)
            ));
        }
        return FpvPoseKind::Posed(FpvPosedFrame {
            lens: pose.lens,
            poses: [Some(pose), None],
            idle_sampled: true,
            notetracks,
        });
    }

    let Some(right_pose) = rig.pose_hand(0, &right, Vec3::ZERO, camera.as_ref()) else {
        return eye_pose_failed(equipped);
    };
    if logging {
        crate::anim::fpv::vm_log_note(&format!(
            "POSE right dual={dual} {}",
            rig.debug_pose_line(&right, &right_pose)
        ));
    }
    let lens = right_pose.lens;
    // The rig laid out a left hand whenever one is drawn, so a left hand that
    // cannot be posed is a plan with a hole in it: refuse the frame.
    let left_pose = if left_drawn {
        match rig.pose_hand(1, &left, off_hand_shift(dual_offset), None) {
            Some(pose) => Some(pose),
            None => return eye_pose_failed(equipped),
        }
    } else {
        None
    };
    FpvPoseKind::Posed(FpvPosedFrame {
        poses: [Some(right_pose), left_pose],
        lens,
        idle_sampled: true,
        notetracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bolt() -> FpvBoltFrame {
        FpvBoltFrame {
            bones: vec![Mat4::IDENTITY; 3],
            ..FpvBoltFrame::default()
        }
    }

    #[test]
    fn a_fresh_pose_product_hides_the_viewmodel() {
        let product = FpvPoseProduct::default();
        assert!(product.drawgun.is_none());
        assert!(matches!(product.kind, FpvPoseKind::Hide));
    }

    #[test]
    fn clearing_bolt_targets_drops_every_hand() {
        let mut targets = FpvBoltTargets::default();
        targets.set_pose(0, bolt());
        targets.set_pose(1, bolt());
        targets.tracker_screen = Some([Vec3::ONE; 3]);
        assert!(targets.pose.iter().all(Option::is_some));

        targets.clear();
        assert!(targets.pose.iter().all(Option::is_none));
        assert!(targets.tracker_screen.is_none());
        assert!(targets.flash.iter().all(Option::is_none));
    }

    #[test]
    fn set_pose_fills_only_the_hand_asked_for() {
        let mut targets = FpvBoltTargets::default();
        targets.set_pose(1, bolt());
        assert!(targets.pose[0].is_none());
        assert_eq!(targets.pose[1].as_ref().map(|b| b.bones.len()), Some(3));
    }

    #[test]
    fn the_off_hand_stays_put_without_an_offset() {
        assert_eq!(off_hand_shift(None), Vec3::ZERO);
        assert_eq!(off_hand_shift(Some(0.0)), Vec3::ZERO);
    }

    #[test]
    fn the_off_hand_shift_is_what_the_placement_table_gives() {
        let expected = Vec3::from_array(weapon_t6::placement::dual_wield_view_model_origin_add(
            1,
            [-1.0, 0.0, 0.0],
            2.5,
        ));
        assert_eq!(off_hand_shift(Some(2.5)), expected);
    }

    #[test]
    fn refusals_compare_by_what_they_name() {
        let a = FpvPoseRefuse::EyePoseFailed {
            gun_xmodel: "gun".into(),
        };
        assert_eq!(a, a.clone());
        assert_ne!(a, FpvPoseRefuse::NoActiveClips);
    }
}
