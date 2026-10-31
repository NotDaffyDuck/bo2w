//! The skeleton a posed model draws from: the bones of every model glued
//! into one list, the hide set, and the walk from local part poses up to
//! world joint_transforms.

use core::fmt;
use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use crate::{quat16, DOBJ_MAX_PARTS, PartSet, LocalPose, any_part_hidden, PartPose};

/// The parts a DObj currently hides. Same layout as every other part set:
/// five words, part 0 the top bit of word 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HiddenParts([u32; PART_SET_WORDS]);

const PART_SET_WORDS: usize = 5;

/// Word and bit of `part` in a five-word part set, if it fits.
fn part_slot(part: usize) -> Option<(usize, u32)> {
    (part < PART_SET_WORDS * 32).then(|| (part / 32, 0x8000_0000_u32 >> (part % 32)))
}

impl HiddenParts {
    pub const BIT_CHUNK_COUNT: usize = PART_SET_WORDS;

    pub const fn from_bit_chunks(chunks: [u32; Self::BIT_CHUNK_COUNT]) -> Self {
        Self(chunks)
    }

    pub const fn words(&self) -> &[u32; Self::BIT_CHUNK_COUNT] {
        &self.0
    }

    /// Hides a bone; one past the last part is ignored.
    pub fn set(&mut self, part: usize) {
        if let Some((chunk, mask)) = part_slot(part) {
            self.0[chunk] |= mask;
        }
    }

    pub fn get(&self, part: usize) -> bool {
        part_slot(part).is_some_and(|(chunk, mask)| self.0[chunk] & mask != 0)
    }
}

/// Where a model hangs: a bone of an earlier model, found by name.
#[derive(Debug, Clone)]
pub struct MountedModel {
    pub host_model: usize,
    pub mount_tag: String,
}

/// Which stretch of the shared bone list one model owns.
#[derive(Debug, Clone)]
pub struct AttachedModelIndex {
    pub name: String,
    pub first_joint: usize,
    pub joint_total: usize,
    pub size_scale: f32,
}

#[derive(Debug, Clone)]
pub struct SkelJoint {
    pub name: String,
    pub model: usize,
    pub up: Option<usize>,
    pub rest_orientation: Quat,
    pub rest_offset: Vec3,
    pub rest_world_matrix: Mat4,
    pub ignores_size_scaling: bool,
}

/// A bone's place in the world as an origin and three unit axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePlacement {
    pub at: [f32; 3],
    pub basis_dirs: [[f32; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BonePlacementFault {
    BoneIndexInvalid {
        joint: usize,
        count: usize,
    },
    NonFinite {
        joint: usize,
    },
    CollapsedAxis {
        joint: usize,
        which_axis: usize,
    },
}

/// Reads `joint`'s origin and axes out of the posed joint_transforms; the axes are
/// scaled back to length one, and a collapsed or non-finite one is refused.
pub fn bone_orientation(posed: &[Mat4], joint: usize) -> Result<BonePlacement, BonePlacementFault> {

    let Some(matrix) = posed.get(joint) else {
        let count = posed.len();
        return Err(BonePlacementFault::BoneIndexInvalid { joint, count });
    };
    let origin = matrix.transform_point3(Vec3::ZERO);
    if !origin.to_array().into_iter().all(f32::is_finite) {
        return Err(BonePlacementFault::NonFinite { joint });
    }
    let mut axis = [[0.0_f32; 3]; 3];
    for (which, basis) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        let moved = matrix.transform_vector3(basis);
        let len = moved.length();
        if !len.is_finite() {
            return Err(BonePlacementFault::NonFinite { joint });
        }
        if len <= f32::EPSILON {
            return Err(BonePlacementFault::CollapsedAxis { joint, which_axis: which });
        }
        axis[which] = (moved / len).to_array();
    }
    let oriented = BonePlacement {
        basis_dirs: axis,
        at: origin.into(),
    };
    Ok(oriented)
}

#[derive(Debug, Clone)]
pub enum SkeletonBuildFault {
    BadModel {
        model_name: String,
        why: String,
    },
    MissingMountTag {
        model_name: String,
        tag_name: String,
        host_model: usize,
    },
    BoneLimitExceeded {
        held: usize,
        cap: usize,
    },
}

impl fmt::Display for SkeletonBuildFault {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::BadModel { model_name, why } => format!("model {model_name:?}: {why}"),
            Self::BoneLimitExceeded { cap, held } => {
                format!("a DObj holds at most {cap} bones, got {held}")
            }
            Self::MissingMountTag { host_model, tag_name, model_name } => {
                format!("attachment {model_name:?}: no bone named {tag_name:?} in model {host_model}")
            }
        };
        out.write_str(&text)
    }
}

/// One model as the DObj reads it: its bones, how they hang together and
/// their bind poses.
#[derive(Debug, Clone)]
pub struct PoseDonorModel {
    pub name: String,
    pub skeleton_joint_total: usize,
    pub top_joint_total: usize,
    pub size_scale: f32,
    pub fixed_size_parts: [u32; 5],
    pub joint_labels: Vec<String>,
    /// For each non-root bone, how many bones back its parent sits.
    pub ancestor_chain: Vec<u8>,
    pub rotation_quaternions: Vec<[i16; 4]>,
    pub shift_offset: Vec<[f32; 3]>,
    pub foundation_surface: Vec<(Quat, Vec3)>,
}

#[derive(Debug, Clone)]
pub struct PosedSkeleton {
    pub bones: Vec<SkelJoint>,
    pub models: Vec<AttachedModelIndex>,
    /// `(write_target, source)`: bones that share a name with an earlier model's
    /// bone and copy its pose.
    pub mirrored_part_pairs: Vec<(usize, usize)>,
}

fn refuse(model: &PoseDonorModel, detail: String) -> SkeletonBuildFault {
    SkeletonBuildFault::BadModel {
        model_name: model.name.clone(),
        why: detail,
    }
}

/// The table entry for `model`, whose bones start at `base` in the shared list.
fn slot_for(model: &PoseDonorModel, base: usize) -> AttachedModelIndex {
    let PoseDonorModel {
        name,
        size_scale: scale,
        skeleton_joint_total,
        ..
    } = model;
    AttachedModelIndex {
        first_joint: base,
        joint_total: *skeleton_joint_total,
        name: String::clone(name),
        size_scale: *scale,
    }
}

/// The bone a model's roots hang on, named by its attach.
fn anchor_bone(
    model: &PoseDonorModel,
    attach: &MountedModel,
    slots: &[AttachedModelIndex],
    bones: &[SkelJoint],
) -> Result<usize, SkeletonBuildFault> {
    let Some(slot) = slots.get(attach.host_model) else {
        return Err(refuse(
            model,
            format!(
                "attach target model {} has not been built yet",
                attach.host_model
            ),
        ));
    };
    let owned = &bones[slot.first_joint..slot.first_joint + slot.joint_total];
    match owned.iter().position(|bone| bone.name == attach.mount_tag) {
        Some(at) => Ok(slot.first_joint + at),
        None => Err(SkeletonBuildFault::MissingMountTag {
            model_name: model.name.clone(),
            tag_name: attach.mount_tag.clone(),
            host_model: attach.host_model,
        }),
    }
}

/// One bone of `model`, which is number `owner` and starts at `first` in
/// the shared list. Roots hang on `anchor` at `root_bind`; the rest hang on
/// the bone their parent list points back to.
fn make_bone(
    model: &PoseDonorModel,
    owner: usize,
    first: usize,
    joint: usize,
    anchor: Option<usize>,
    root_bind: (Quat, Vec3),
) -> Result<SkelJoint, SkeletonBuildFault> {
    let (rest_rotation, rest_origin) = model.foundation_surface[joint];
    let (turn_matrix, shift_offset, parent) = if joint < model.top_joint_total {
        (root_bind.0, root_bind.1, anchor)
    } else {
        let child = joint - model.top_joint_total;
        let Some(&back) = model.ancestor_chain.get(child) else {
            return Err(refuse(model, format!("bone {joint} has no parent entry")));
        };
        let back = usize::from(back);
        let parent = match joint.checked_sub(back) {
            Some(p) if back > 0 && p < joint => p,
            _ => {
                return Err(refuse(
                    model,
                    format!("bone {joint} parent step {back} escapes the skeleton"),
                ));
            }
        };
        let Some(&packed) = model.rotation_quaternions.get(child) else {
            return Err(refuse(model, format!("bone {joint} has no bind rotation")));
        };
        let Some(&offset) = model.shift_offset.get(child) else {
            return Err(refuse(model, format!("bone {joint} has no bind translation")));
        };
        // The stored rotation is not unit length (the game folds the length
        // into a weight); the direction is what matters.
        let stored = Quat::from_array(quat16(packed));
        let rotation = match stored.length_squared() > 0.001 {
            true => stored.normalize(),
            false => Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
        };
        (rotation, Vec3::from_array(offset), Some(first + parent))
    };
    let label = model.joint_labels.get(joint).map_or_else(String::new, Clone::clone);
    let fixed_scale = match part_slot(joint) {
        Some((chunk, mask)) => model.fixed_size_parts[chunk] & mask != 0,
        None => false,
    };
    Ok(SkelJoint {
        up: parent,
        ignores_size_scaling: fixed_scale,
        rest_world_matrix: Mat4::from_rotation_translation(rest_rotation.normalize(), rest_origin),
        rest_orientation: turn_matrix,
        rest_offset: shift_offset,
        model: owner,
        name: label,
    })
}

/// Pairs each bone that repeats the name of an earlier model's bone with
/// that first bone, unless it is the attach bone's own child.
fn name_twins(bones: &[SkelJoint]) -> Vec<(usize, usize)> {
    let mut first_with: HashMap<&str, usize> = HashMap::new();
    let mut twins = Vec::new();
    for (at, bone) in bones.iter().enumerate() {
        if bone.name.is_empty() {
            continue;
        }
        let earliest = *first_with.entry(bone.name.as_str()).or_insert(at);
        if bones[earliest].model != bone.model && bone.up != Some(earliest) {
            twins.push((at, earliest));
        }
    }
    twins
}

/// A controlled bone below `up`: it keeps `turn` as measured in the root's
/// frame, then follows its parent's position.
fn controlled_matrix(posed: &[Mat4], up: usize, turn: Quat, offset: Vec3) -> Mat4 {
    let root_turn = rotation_of(&posed[0]);
    let parent_turn = rotation_of(&posed[up]);
    let rotation = root_turn * turn * root_turn.conjugate() * parent_turn;
    let position = posed[up].transform_point3(offset);
    Mat4::from_rotation_translation(rotation, position)
}

/// A bone's rotation as the world matrix carries it.
fn rotation_of(matrix: &Mat4) -> Quat {
    matrix.to_scale_rotation_translation().1
}

impl PosedSkeleton {
    pub fn build(models: &[(&PoseDonorModel, Option<MountedModel>)]) -> Result<Self, SkeletonBuildFault> {
        Self::build_with_root_binds(models, &[])
    }

    /// As `build`, with a model's roots bound to their parent at
    /// `root_binds[model]` (rotation, translation) instead of on it. A gun's
    /// magazine and scope sit at an offset from the gun's root.
    pub fn build_with_root_binds(
        parts: &[(&PoseDonorModel, Option<MountedModel>)],
        root_binds: &[Option<(Quat, Vec3)>],
    ) -> Result<PosedSkeleton, SkeletonBuildFault> {
        let mut bones = Vec::<SkelJoint>::new();
        let mut models = Vec::<AttachedModelIndex>::with_capacity(parts.len());

        for (owner, (model, attach)) in parts.iter().enumerate() {
            if model.foundation_surface.len() < model.skeleton_joint_total {
                return Err(refuse(
                    model,
                    format!(
                        "{} bind matrices for {} bones",
                        model.foundation_surface.len(),
                        model.skeleton_joint_total
                    ),
                ));
            }
            let anchor = match attach {
                Some(attach) => Some(anchor_bone(model, attach, &models, &bones)?),
                None => None,
            };
            let first = bones.len();
            models.push(slot_for(model, first));
            let root_bind = match root_binds.get(owner) {
                Some(Some(bind)) => *bind,
                _ => (Quat::from_xyzw(0.0, 0.0, 0.0, 1.0), Vec3::ZERO),
            };
            let made: Result<Vec<SkelJoint>, SkeletonBuildFault> = (0..model.skeleton_joint_total)
                .map(|bone| make_bone(model, owner, first, bone, anchor, root_bind))
                .collect();
            bones.extend(made?);
        }

        let overflow = Some(bones.len())
            .filter(|&held| held > DOBJ_MAX_PARTS)
            .map(|held| SkeletonBuildFault::BoneLimitExceeded {
                held,
                cap: DOBJ_MAX_PARTS,
            });
        if let Some(too_many) = overflow {
            return Err(too_many);
        }
        Ok(PosedSkeleton {
            mirrored_part_pairs: name_twins(&bones),
            models,
            bones,
        })
    }

    pub fn joint_total(&self) -> usize {
        self.bones.iter().count()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|bone| bone.name == name)
    }

    pub fn full_part_set(&self) -> PartSet {
        PartSet::first(self.bones.len())
    }

    /// Hands every twin bone the pose (and control bit) of the bone it copies.
    pub fn copy_mirrored_parts(&self, locals: &mut LocalPose) {
        self.mirrored_part_pairs
            .iter()
            .for_each(|&(twin, original)| locals.copy_part(twin, original));
    }

    /// Turns local part poses into world joint_transforms, parents before children.
    /// A controlled bone keeps its authored angle in the root's frame instead
    /// of inheriting its parent's turn.
    pub fn world_pose(&self, locals: &LocalPose, root_frame: Mat4) -> Vec<Mat4> {
        let mut posed = Vec::<Mat4>::with_capacity(self.bones.len());
        for (at, joint) in self.bones.iter().enumerate() {
            let pose = &locals[at];
            let stretch = self.stretch_of(joint);
            let offset = joint.rest_offset + Vec3::from_array(pose.translation) * stretch;
            let turn = Quat::from_array(pose.rotation);
            let local = Mat4::from_rotation_translation(turn, offset);
            let base = joint.up.map_or(root_frame, |up| posed[up]);
            let matrix = match joint.up.filter(|_| locals.control.get(at)) {
                Some(up) => controlled_matrix(&posed, up, turn, offset),
                None => base.mul_mat4(&local),
            };
            posed.push(matrix);
        }
        posed
    }

    /// How much a bone's translation is scaled: its model's scale unless the
    /// bone opts out.
    fn stretch_of(&self, bone: &SkelJoint) -> f32 {
        (!bone.ignores_size_scaling)
            .then(|| self.models[bone.model].size_scale)
            .unwrap_or(1.0)
    }

    /// Whether a surface of `model` still draws given what is hidden.
    pub fn facet_visible(
        &self,
        owner: usize,
        surface_bits: &[u32; PART_SET_WORDS],
        hide: &HiddenParts,
    ) -> bool {
        let Some(slot) = self.models.get(owner) else {
            return false;
        };
        let shift = u32::try_from(slot.first_joint).unwrap_or(u32::MAX);
        !any_part_hidden(surface_bits, hide.words(), shift)
    }

    /// The joint_transforms that carry a bind-pose vertex to where its bone is now.
    pub fn skin_matrices(&self, joint_world: &[Mat4]) -> Vec<Mat4> {
        joint_world
            .iter()
            .zip(&self.bones)
            .map(|(posed, bone)| *posed * bone.rest_world_matrix.inverse())
            .collect()
    }

    /// Every part resting at its bind rotation.
    pub fn rest_local_pose(&self) -> LocalPose {
        let rest = self
            .bones
            .iter()
            .map(|bone| PartPose::at_rest(bone.rest_orientation.to_array()));
        LocalPose::new(rest.collect())
    }
}

/// Which bone a model joins its parent at when the attach names none: the
/// child's own root bone if the parent has one of that name (a head meets
/// its body at the spine and the neck follows the body's), else the parent's
/// first bone.
pub fn empty_tag_attach(parent: &PoseDonorModel, child: &PoseDonorModel) -> String {
    let has = |name: &String| parent.joint_labels.contains(name);
    match child.joint_labels.first() {
        Some(own_root) if has(own_root) => own_root.clone(),
        _ => parent.joint_labels.first().cloned().unwrap_or_default(),
    }
}

/// How far the torso may bend up or down, either way.
pub const AIM_PITCH_CLAMP_RAD: f32 = 85.0_f32 * (core::f32::consts::PI / 180.0);

/// Swings the pelvis one way and the torso stabiliser the other, so the legs
/// turn while the upper body keeps facing the same way.
pub fn turn_legs(dobj: &PosedSkeleton, locals: &mut [PartPose], leg_turn: f32) {
    if leg_turn.abs() >= 1e-5 {
        for (name, direction) in [("pelvis", 1.0), ("torso_stabilizer", -1.0)] {
            yaw_joint(dobj, locals, name, direction * leg_turn);
        }
    }
}

/// Splits the look pitch over the spine: the waist takes its share, and what
/// is left of the torso's is shared by the two upper spine bones.
pub fn pitch_spine_aim(dobj: &PosedSkeleton, locals: &mut [PartPose], chest_tilt: f32, hip_tilt: f32) {
    let limit = AIM_PITCH_CLAMP_RAD;
    let waist = hip_tilt.clamp(-limit, limit);
    let torso = chest_tilt.clamp(-limit, limit);
    let each_upper = (torso - waist) * 0.5;
    for (bone, share) in [
        ("j_spinelower", waist),
        ("j_spineupper", each_upper),
        ("j_spine4", each_upper),
    ] {
        pitch_joint(dobj, locals, bone, share);
    }
}

/// The bones the player controller steers, in the order its plan lists them.
pub const SPINE_AIM_BONES: [&str; 4] = ["back_low", "back_mid", "back_up", "pelvis"];

/// Euler angles in degrees (pitch, yaw, roll) as a quaternion `[x, y, z, w]`,
/// turning yaw first, then pitch, then roll.
pub fn set_angles(rotation_degrees: [f32; 3]) -> [f32; 4] {
    const DEG_TO_HALF_RAD: f32 = 0.008_726_646;
    let [pitch, yaw, roll] = rotation_degrees.map(|deg| (deg * DEG_TO_HALF_RAD).sin_cos());
    let (ps, pc) = pitch;
    let (ys, yc) = yaw;
    let (rs, rc) = roll;
    let pitch_yaw = [-ps * ys, ps * yc, pc * ys, pc * yc];
    [
        rs * pitch_yaw[3] + rc * pitch_yaw[0],
        rc * pitch_yaw[1] + rs * pitch_yaw[2],
        rc * pitch_yaw[2] - rs * pitch_yaw[1],
        rc * pitch_yaw[3] - rs * pitch_yaw[0],
    ]
}

/// Aims a controlled bone at `rotation_degrees` in the root's frame.
pub fn steer_joint(locals: &mut LocalPose, joint: usize, rotation_degrees: [f32; 3]) {
    locals.set_control_tag(joint, Some(set_angles(rotation_degrees)));
}

/// Puts a bone at a translation and angles of its own, releasing it from
/// control. A bone past the end is left alone.
pub fn set_local_tag(
    locals: &mut LocalPose,
    part: usize,
    shift: [f32; 3],
    turn_degrees: [f32; 3],
) {
    let Some(pose) = locals.get_mut(part) else {
        return;
    };
    pose.translation = shift;
    pose.rotation = set_angles(turn_degrees);
    locals.control.clear(part);
}

/// What the player controller needs to know about the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyAimInput {
    pub look_pitch_degrees: f32,
    pub lying: bool,
    pub crouched: bool,
    /// Lean from -1 (left) to 1 (right).
    pub body_lean_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyAimOutcome {
    /// How many of the controller's bones the DObj had.
    pub steered_count: u8,
    pub root_steered: bool,
    pub origin_tag_shift: [f32; 3],
    pub origin_tag_turn: [f32; 3],
}

/// Per-stance lean numbers: `[left, right]`, for standing then crouched.
const LEAN_SHIFT: [[f32; 2]; 2] = [[5.0, 2.5], [12.5, 13.0]];
const LEAN_ROTATE: [[f32; 2]; 2] = [[1.25, 1.25], [1.25, 1.0]];
const TORSO_LEAN_ROLL: f32 = 46.25;
const ORIGIN_LEAN_ROLL: f32 = 3.75;

/// Picks the left or right entry of a stance table; no lean counts as left.
fn lean_entry(table: &[[f32; 2]; 2], crouched: bool, lean: f32) -> f32 {
    table[usize::from(crouched)][usize::from(lean > 0.0)]
}

/// Applies the controller to the DObj's `back_*` and `pelvis` bones and to
/// the root (`tag_origin`), which carries the lean offset.
pub fn pose_body_aim(
    skeleton: &PosedSkeleton,
    locals: &mut LocalPose,
    stance: BodyAimInput,
) -> BodyAimOutcome {
    let LeanPlan {
        bone_angles,
        origin_shift,
        origin_tilt,
    } = steer_plan(stance);
    let present = SPINE_AIM_BONES
        .iter()
        .zip(bone_angles)
        .filter_map(|(label, angles)| Some((skeleton.find(label)?, angles)));
    let mut found = 0u8;
    for (bone, angles) in present {
        steer_joint(locals, bone, angles);
        found += 1;
    }
    let has_root = !locals.is_empty();
    if has_root {
        set_local_tag(locals, 0, origin_shift, origin_tilt);
    }
    BodyAimOutcome {
        steered_count: found,
        root_steered: has_root,
        origin_tag_shift: origin_shift,
        origin_tag_turn: origin_tilt,
    }
}

/// Just the root's lean offset and angles, for callers that move the whole
/// body and need no bone poses.
pub fn player_controller_tag_origin(input: BodyAimInput) -> ([f32; 3], [f32; 3]) {
    let LeanPlan {
        origin_shift,
        origin_tilt,
        ..
    } = steer_plan(input);
    (origin_shift, origin_tilt)
}

struct LeanPlan {
    /// One angle triple per entry of `SPINE_AIM_BONES`.
    bone_angles: [[f32; 3]; 4],
    origin_shift: [f32; 3],
    origin_tilt: [f32; 3],
}

/// The torso's pitch: twice the view pitch; lying down it bends less, and
/// less still looking down.
fn chest_tilt(look_pitch_degrees: f32, prone: bool) -> f32 {
    let doubled = 2.0 * angle_normalize_180(look_pitch_degrees);
    if !prone {
        return doubled;
    }
    let wrapped = angle_normalize_180(doubled);
    wrapped * if wrapped <= 0.0 { 0.25 } else { 0.5 }
}

fn steer_plan(input: BodyAimInput) -> LeanPlan {
    let (prone, crouch, body_lean_ratio) = (input.lying, input.crouched, input.body_lean_ratio);
    let pitch = chest_tilt(input.look_pitch_degrees, prone);
    let roll = body_lean_ratio * TORSO_LEAN_ROLL;
    let leaning = body_lean_ratio != 0.0;
    let sideways = if leaning {
        -body_lean_ratio * lean_entry(&LEAN_SHIFT, crouch, body_lean_ratio)
    } else {
        0.0
    };
    let origin_shift = [0.0, sideways, 0.0];

    if prone {
        return LeanPlan {
            bone_angles: [
                [0.0, roll * -1.2, roll * 0.3],
                [0.0, -roll * 0.2, roll * 0.2],
                [pitch, roll, roll * -0.2],
                [0.0; 3],
            ],
            origin_shift,
            origin_tilt: [0.0; 3],
        };
    }
    let roll = if leaning {
        roll * lean_entry(&LEAN_ROTATE, crouch, body_lean_ratio)
    } else {
        roll
    };
    LeanPlan {
        bone_angles: [
            [pitch * 0.2, 0.0, roll * 0.5],
            [pitch * 0.3, 0.0, roll * 0.5],
            [pitch * 0.5, 0.0, roll * -0.6],
            [0.0; 3],
        ],
        origin_shift,
        origin_tilt: [0.0, 0.0, body_lean_ratio * ORIGIN_LEAN_ROLL],
    }
}

/// The controller for a player standing upright without leaning; returns how
/// many of its bones were found.
pub fn pose_upright_body_aim(
    skeleton: &PosedSkeleton,
    locals: &mut LocalPose,
    pitch_deg: f32,
) -> u8 {
    let (prone, crouch, body_lean_ratio) = (false, false, 0.0);
    let upright = BodyAimInput { look_pitch_degrees: pitch_deg, lying: prone, crouched: crouch, body_lean_ratio };
    pose_body_aim(skeleton, locals, upright).steered_count
}

/// Wraps an angle in degrees into [-180, 180).
fn angle_normalize_180(deg: impl Into<f32>) -> f32 {
    let revs = deg.into() * (1.0 / 360.0);
    (revs - (revs + 0.5).floor()) * 360.0
}

/// The bone called `name` and the axis, in its own frame, that points along
/// `world_axis` in the bind pose; `None` if it is missing or has no such axis.
fn bind_axis(dobj: &PosedSkeleton, name: &str, world_axis: Vec3) -> Option<(usize, Vec3)> {
    let bone = dobj.find(name)?;
    let bind_turn = rotation_of(&dobj.bones[bone].rest_world_matrix);
    let axis = (bind_turn.inverse() * world_axis).normalize_or_zero();
    (axis != Vec3::ZERO).then_some((bone, axis))
}

/// Turns a local pose by `angle` about `axis`, applied after what it holds.
fn twist_pose(pose: &mut PartPose, axis: Vec3, by: f32) {
    let held = Quat::from_array(pose.rotation);
    let turned = (held * Quat::from_axis_angle(axis, by)).normalize();
    pose.rotation = turned.to_array();
}

/// Yaws a bone about its own up axis, as the bind pose has it.
pub fn yaw_joint(dobj: &PosedSkeleton, locals: &mut [PartPose], name: &str, turn: f32) {
    if let Some((bone, axis)) = bind_axis(dobj, name, Vec3::Z) {
        twist_pose(&mut locals[bone], axis, turn);
    }
}

/// Pitches a spine bone forward by `model_pitch` about its own side axis.
pub fn pitch_joint(dobj: &PosedSkeleton, locals: &mut [PartPose], name: &str, forward_pitch: f32) {
    let steered = bind_axis(dobj, name, Vec3::Y).filter(|_| forward_pitch.abs() >= 1e-6);
    if let Some((bone, axis)) = steered {
        twist_pose(&mut locals[bone], axis, -forward_pitch);
    }
}

/// Where a held weapon attaches on a third-person body, best choice first.
pub const THIRD_PERSON_GUN_BONES: &[&str] = &["tag_weapon_right", "tag_inhand", "tag_weapon_left"];

/// Where a head attaches on a third-person body.
pub const THIRD_PERSON_HEAD_BONE: &str = "j_spine4";

/// The first of `THIRD_PERSON_GUN_BONES` the body has.
pub fn tp_weapon_attach_tag(joint_labels: &[String]) -> Option<&'static str> {
    let has = |tag: &str| joint_labels.iter().any(|name| name == tag);
    for &tag in THIRD_PERSON_GUN_BONES {
        if has(tag) {
            return Some(tag);
        }
    }
    None
}

/// `THIRD_PERSON_HEAD_BONE`, if the body has it.
pub fn tp_head_attach_tag(joint_labels: &[String]) -> Option<&'static str> {
    joint_labels
        .iter()
        .find(|name| *name == THIRD_PERSON_HEAD_BONE)
        .map(|_| THIRD_PERSON_HEAD_BONE)
}

/// A body with an optional head and a weapon on its attach bones. Returns the
/// DObj, the weapon's tag and the muzzle flash bone; `None` when the body has
/// no place for the weapon (or the head) or the DObj will not build.
pub fn assemble_body_head_gun_skeleton(
    torso: &PoseDonorModel,
    helmet: Option<&PoseDonorModel>,
    gun: &PoseDonorModel,
) -> Option<(PosedSkeleton, &'static str, Option<usize>)> {
    let weapon_tag = tp_weapon_attach_tag(&torso.joint_labels)?;
    let on_body = |tag: &str| {
        Some(MountedModel {
            host_model: 0,
            mount_tag: tag.to_owned(),
        })
    };
    let mut parts = vec![(torso, None)];
    if let Some(skull) = helmet {
        let neck = tp_head_attach_tag(&torso.joint_labels)?;
        parts.push((skull, on_body(neck)));
    }
    parts.push((gun, on_body(weapon_tag)));
    let built = PosedSkeleton::build(&parts).ok()?;
    let flash = built.find("tag_flash");
    Some((built, weapon_tag, flash))
}

/// `assemble_body_head_gun_skeleton` without a head.
pub fn build_body_weapon_dobj(torso: &PoseDonorModel, gun: &PoseDonorModel) -> Option<(PosedSkeleton, &'static str, Option<usize>)> {
    assemble_body_head_gun_skeleton(torso, None, gun)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(name: &str, names: &[&str]) -> PoseDonorModel {
        let count = names.len();
        PoseDonorModel {
            name: name.to_string(),
            skeleton_joint_total: count,
            top_joint_total: 1,
            size_scale: 1.0,
            fixed_size_parts: [0; 5],
            joint_labels: names.iter().map(|n| n.to_string()).collect(),
            ancestor_chain: vec![1; count - 1],
            rotation_quaternions: vec![[0, 0, 0, 32767]; count - 1],
            shift_offset: vec![[1.0, 0.0, 0.0]; count - 1],
            foundation_surface: vec![(Quat::IDENTITY, Vec3::ZERO); count],
        }
    }

    #[test]
    fn body_weapon_and_head_hang_on_their_tags() {
        let body = tiny("body", &["root", "j_spine4", "tag_weapon_right"]);
        let head = tiny("head", &["j_head"]);
        let gun = tiny("gun", &["gun_root", "tag_flash"]);

        let (dobj, tag, flash) = assemble_body_head_gun_skeleton(&body, Some(&head), &gun).unwrap();
        assert_eq!(tag, "tag_weapon_right");
        assert_eq!(dobj.models.len(), 3);
        assert_eq!(flash, Some(dobj.joint_total() - 1));
        assert_eq!(dobj.bones[3].up, Some(1));
        assert_eq!(dobj.bones[4].up, Some(2));

        let (plain, tag, flash) = build_body_weapon_dobj(&body, &gun).unwrap();
        assert_eq!((tag, plain.models.len(), flash), (tag, 2, Some(4)));
        assert_eq!(plain.bones[3].up, Some(2));

        let bare = tiny("bare", &["root", "pelvis"]);
        assert!(build_body_weapon_dobj(&bare, &gun).is_none());
        let no_neck = tiny("body", &["root", "tag_inhand"]);
        assert!(assemble_body_head_gun_skeleton(&no_neck, Some(&head), &gun).is_none());
        assert!(build_body_weapon_dobj(&no_neck, &gun).is_some());
    }

    #[test]
    fn standing_controller_counts_the_bones_it_found() {
        let model = tiny("body", &["tag_origin", "back_low", "pelvis"]);
        let dobj = PosedSkeleton::build(&[(&model, None)]).unwrap();
        let mut pose = dobj.rest_local_pose();
        assert_eq!(pose_upright_body_aim(&dobj, &mut pose, 10.0), 2);
        assert_eq!(angle_normalize_180(190.0_f32), -170.0);
        assert_eq!(angle_normalize_180(-180.0_f32), -180.0);
    }
}
