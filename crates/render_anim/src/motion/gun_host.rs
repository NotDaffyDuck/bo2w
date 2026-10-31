//! Resources the first-person presenter keeps between frames, and the step
//! that turns the equipped weapon's playing clips into this frame's hand poses.

use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use bevy::prelude::*;

use crate::motion::fpv::{HeldGunLoadout, ServerTruthSnapshot, ViewmodelShowState};
use crate::motion::fpv::tick_with_prediction;
use crate::motion::gun_pose::{GunTagPoseFrame, ClipPoseBinding};
use crate::motion::fpv_prepared::FpvRigSet;
use crate::motion::fpv_rig::{FpvHandPose, PreparedFpvRig};
use crate::motion::gun_rig_ctl::FirstPersonGunAnimator;
use asset_game::GunMoveRole;
use assets::FpvMeshSlot;

#[derive(Resource, Default)]
pub struct FpvShownCursor(pub ViewmodelShowState);

#[derive(Resource, Default)]
pub struct WaitingFpvSpawn(pub Option<QueuedGunSpawn>);

#[derive(Clone, Debug)]
pub struct QueuedGunSpawn {
    pub gun_index: FpvMeshSlot,
    pub catalog_id: u64,
    pub gun_catalog_no: u32,
    pub parent_weapon: u32,
}

/// Notetrack names the held weapon's clips raised, waiting for the audio and
/// effects side to pick them up.
#[derive(Resource, Default)]
pub struct QueuedGunNotes {
    pub weap: u32,
    pub labels: Vec<String>,
}

#[derive(Resource, Default)]
pub struct HoldSettledLife(pub Option<u32>);

#[derive(Resource, Default)]
pub struct HoldLifeStamp(pub Option<u32>);

/// Where the first-person attachment points are this frame, per hand, for the
/// effects that follow them.
#[derive(Resource, Default)]
pub struct AttachTagGoals {
    pub pose: [Option<GunTagPoseFrame>; 2],
    pub tracker_screen: Option<[Vec3; 3]>,
    pub tracker_light: Option<fx::EffectBoltTarget>,
    /// bo2mp: the held gun's `tag_hud` (the Target Finder screen plane).
    pub hud: Option<fx::EffectBoltTarget>,
    pub flash: [Option<fx::EffectBoltTarget>; 2],
    pub brass: [Option<fx::EffectBoltTarget>; 2],
    pub knife: [Option<fx::EffectBoltTarget>; 2],
    pub laser: [Option<fx::EffectBoltTarget>; 2],
    /// bo2mp: the gun's `tag_target` (BO2's `playviewmodelfx`).
    pub target: [Option<fx::EffectBoltTarget>; 2],
    /// bo2zm: the arms' wrists (left, right) and gun tag, for Mob's
    /// Afterlife ghost glow.
    pub wrist: [Option<fx::EffectBoltTarget>; 2],
    pub weapon: Option<fx::EffectBoltTarget>,
}

impl AttachTagGoals {
    pub fn clear(&mut self) {
        *self = AttachTagGoals::default();
    }

    pub fn put_pose(&mut self, side_ix: usize, bolt: GunTagPoseFrame) {
        let slot = &mut self.pose[side_ix];
        *slot = Some(bolt);
    }
}

/// Why a frame produced no viewmodel pose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GunPoseDecline {
    TableAbsent,
    NothingPlaying,
    EyePoseFailed {
        weapon_mesh: String,
    },
    DependencyUnresolved {
        gun_catalog_no: u32,
        purpose: &'static str,
        name: String,
    },
}

#[derive(Default)]
pub enum ViewmodelPoseVariety {
    /// Nothing is held, or it is not shown.
    #[default]
    Hide,
    Refuse(GunPoseDecline),
    Posed(GunPoseSnapshot),
}

/// What one frame asked of the rig: bones per hand, and the lens the right
/// hand carries. No geometry: the surfaces this rig draws were settled when it
/// was prepared, and the vertices are written straight into the published plan.
pub struct GunPoseSnapshot {
    pub poses: [Option<FpvHandPose>; 2],
    pub optic: Mat4,
    pub rest_pose_taken: bool,
    pub note_list: Vec<String>,
}

#[derive(Resource, Default)]
pub struct GunPoseOutput {
    pub viewmodel_pass_flag: Option<i32>,
    pub kind: ViewmodelPoseVariety,
}

pub struct GunPoseInputs<'a> {
    pub dt: f32,
    pub equipped: &'a mut HeldGunLoadout,
    pub rigs: &'a FpvRigSet,
    pub active: &'a mut Option<Arc<PreparedFpvRig>>,
    pub cursor: &'a mut ViewmodelShowState,
    pub rocket: bool,
    pub melee: bool,
    /// bo2mp: aimed far enough that the sight shows its stripped model.
    pub ads: bool,
    pub sample: Option<ServerTruthSnapshot>,
    pub locally_guessed_shot: bool,
    pub akimbo: bool,
    pub akimbo_shift: Option<f32>,
    /// bo2zm: how far through a low climb that plays the gun's climb camera
    /// animation (0 to 1), else `None`.
    pub mantle_camera: Option<f32>,
}

/// The clips a controller has playing, as the rig wants them.
fn playing_clips(controller: &FirstPersonGunAnimator) -> Vec<ClipPoseBinding<'_>> {
    let mut clips = Vec::new();
    for playing in controller.active_anims() {
        clips.push(ClipPoseBinding {
            wt: playing.level,
            time: playing.time,
            reel: playing.reel,
            node_ix: playing.slot_no,
        });
    }
    clips
}

/// The climb camera clip, `fraction` of the way through, when the weapon has one.
fn climb_camera_clip(controller: &FirstPersonGunAnimator, fraction: f32) -> Option<ClipPoseBinding<'_>> {
    let camera_slot = GunMoveRole::CameraMantle;
    let clip = controller.weapon().reel(camera_slot)?;
    let time = clip.duration() * fraction.clamp(0.0, 1.0);
    Some(ClipPoseBinding {
        wt: 1.0,
        time,
        reel: clip,
        node_ix: camera_slot.index(),
    })
}

/// How far the off hand's gun is pushed aside, in eye space.
fn off_hand_shift(akimbo_shift: Option<f32>) -> Vec3 {
    let Some(distance) = akimbo_shift.filter(|distance| *distance != 0.0) else {
        return Vec3::ZERO;
    };
    let base = [-1.0, 0.0, 0.0];
    Vec3::from_array(weapon_t6::placement::dual_wield_gun_offset(
        1, base, distance,
    ))
}

fn eye_pose_failed(equipped: &HeldGunLoadout) -> ViewmodelPoseVariety {
    let weapon_mesh = String::clone(&equipped.weapon_mesh);
    ViewmodelPoseVariety::Refuse(GunPoseDecline::EyePoseFailed { weapon_mesh })
}

pub fn generate_fpv_posture(args: GunPoseInputs<'_>) -> ViewmodelPoseVariety {
    let GunPoseInputs {
        dt: step,
        equipped,
        rigs,
        active: active_rig,
        cursor,
        rocket,
        melee,
        ads,
        sample,
        locally_guessed_shot: fired,
        akimbo: dual,
        akimbo_shift: off_hand_gap,
        mantle_camera: climb_fraction,
    } = args;
    let (_pose, notetracks) = tick_with_prediction(equipped, cursor, sample, fired, step);

    let right = playing_clips(&equipped.controller);
    if right.is_empty() {
        return ViewmodelPoseVariety::Refuse(GunPoseDecline::NothingPlaying);
    }
    let left = match equipped.left.as_ref().filter(|_| dual) {
        Some(off_hand) => playing_clips(off_hand),
        None => Vec::new(),
    };
    let left_drawn = !left.is_empty();
    let camera = climb_fraction.and_then(|at| climb_camera_clip(&equipped.controller, at));

    let Some(rig) = rigs.pick(rocket, left_drawn, melee, ads) else {
        *active_rig = None;
        return eye_pose_failed(equipped);
    };
    if active_rig.as_ref().map(|held| held.generation()) != Some(rig.generation()) {
        *active_rig = Some(Arc::clone(rig));
    }
    let logging = crate::motion::fpv::vm_log_enabled();

    // bo2zm: Black Ops II dual wield: both hands' clips on the one rig.
    if rigs.combined && left_drawn {
        let Some(pose) = rig.pose_combined(&right, &left, camera.as_ref()) else {
            return eye_pose_failed(equipped);
        };
        if logging {
            let everything: Vec<ClipPoseBinding<'_>> = right.iter().chain(&left).cloned().collect();
            crate::motion::fpv::vm_log_note(&format!(
                "POSE dual {}",
                rig.debug_pose_line(&everything, &pose)
            ));
        }
        let lens = pose.lens_mat;
        return ViewmodelPoseVariety::Posed(GunPoseSnapshot {
            note_list: notetracks,
            rest_pose_taken: true,
            poses: [Some(pose), None],
            optic: lens,
        });
    }

    let no_shift = Vec3::ZERO;
    let Some(right_pose) = rig.pose_hand(0, &right, no_shift, camera.as_ref()) else {
        return eye_pose_failed(equipped);
    };
    if logging {
        crate::motion::fpv::vm_log_note(&format!(
            "POSE right dual={dual} {}",
            rig.debug_pose_line(&right, &right_pose)
        ));
    }
    let lens = right_pose.lens_mat;
    // The rig laid out a left hand whenever one is drawn, so a left hand that
    // cannot be posed is a plan with a hole in it: refuse the frame.
    let mut left_pose = None;
    if left_drawn {
        let shifted = off_hand_shift(off_hand_gap);
        let Some(posed) = rig.pose_hand(1, &left, shifted, None) else {
            return eye_pose_failed(equipped);
        };
        left_pose = Some(posed);
    }
    ViewmodelPoseVariety::Posed(GunPoseSnapshot {
        note_list: notetracks,
        rest_pose_taken: true,
        optic: lens,
        poses: [Some(right_pose), left_pose],
    })
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn three_bone_frame() -> GunTagPoseFrame {
        let mut frame = GunTagPoseFrame::default();
        frame.joints = vec![Mat4::IDENTITY; 3];
        frame
    }

    #[test]
    fn nothing_is_shown_before_the_first_pose() {
        let fresh = GunPoseOutput::default();
        assert_eq!(fresh.viewmodel_pass_flag, None);
        assert!(matches!(fresh.kind, ViewmodelPoseVariety::Hide));
    }

    #[test]
    fn clear_empties_both_hands_and_the_trackers() {
        let mut bolts_in = AttachTagGoals::default();
        for hand_in in 0..2 {
            bolts_in.put_pose(hand_in, three_bone_frame());
        }
        bolts_in.tracker_screen = Some([Vec3::ONE; 3]);
        assert!(bolts_in.pose.iter().flatten().count() == 2);

        bolts_in.clear();
        assert_eq!(bolts_in.pose.iter().flatten().count(), 0);
        assert_eq!(bolts_in.tracker_screen, None);
        assert_eq!(bolts_in.flash.iter().flatten().count(), 0);
    }

    #[test]
    fn set_pose_touches_one_hand_only() {
        let mut bolts = AttachTagGoals::default();
        bolts.put_pose(1, three_bone_frame());
        assert!(bolts.pose[0].is_none());
        let kept = bolts.pose[1].as_ref().map(|frame| frame.joints.len());
        assert_eq!(kept, Some(3));
    }

    #[test]
    fn no_gap_or_a_zero_gap_leaves_the_off_hand_alone() {
        for gap in [None, Some(0.0)] {
            assert_eq!(off_hand_shift(gap), Vec3::ZERO);
        }
    }

    #[test]
    fn a_gap_moves_the_off_hand_by_the_dual_wield_table() {
        let gap = 2.5;
        let from_table =
            weapon_t6::placement::dual_wield_gun_offset(1, [-1.0, 0.0, 0.0], gap);
        assert_eq!(off_hand_shift(Some(gap)), Vec3::from_array(from_table));
    }

    #[test]
    fn a_refusal_equals_itself_and_not_another() {
        let eye = GunPoseDecline::EyePoseFailed {
            weapon_mesh: String::from("gun"),
        };
        assert_eq!(eye.clone(), eye);
        assert_ne!(eye, GunPoseDecline::NothingPlaying);
    }
}
