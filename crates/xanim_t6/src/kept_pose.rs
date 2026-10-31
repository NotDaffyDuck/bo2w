//! A model kept loaded for posing and hit tests: its skeleton, the boxes its bones and
//! surfaces collide with, and its bounds. Posing plays a resolved anim tree on a DObj.

use std::array::from_fn;

use dobj_t6::{MountedModel, PosedSkeleton, SkeletonBuildFault, HiddenParts, LocalPose, PoseDonorModel, PartSet, PartPose};
use glam::{Mat4, Quat, Vec3};

use crate::tree::{LiveOverlayLayer, LiveClipLeaf};
use crate::{ClipPlayback, DObjAnim, AnimGraphFault, AnimGraphInstance};

/// A request that names no parts poses every one of them.
const EVERY_PART: Option<PartSet> = None;

/// The controller that changes nothing.
fn leave_pose_alone(_: &PosedSkeleton, _: &PartSet, _: &mut LocalPose) {}

/// A bone's hit box in the bone's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneHitShape {
    pub midpoint: [f32; 3],
    pub half_ext: [f32; 3],
    pub reach_squared: f32,
    /// Which body part a hit on it counts as.
    pub body_part_category: u8,
}

/// What to pose a DObj with.
#[derive(Clone, Debug, Default)]
pub struct ModelPoseQuery {
    /// The anim tree playing; `None` holds the bind pose.
    pub tree: Option<AnimGraphInstance>,
    /// The parts posed; every part when `None`.
    pub wanted_mask: Option<PartSet>,
    pub hidden_mask: HiddenParts,
}

impl ModelPoseQuery {
    /// The bind pose, every part posed, nothing hidden.
    pub const fn neutral_pose() -> Self {
        Self {
            hidden_mask: HiddenParts::from_bit_chunks([0; HiddenParts::BIT_CHUNK_COUNT]),
            wanted_mask: EVERY_PART,
            tree: None,
        }
    }
}

/// A loaded model, ready to pose and hit-test.
#[derive(Clone, Debug)]
pub struct KeptModelAbility {
    pub key: String,
    pub pose: PoseDonorModel,
    /// A hit box per bone, by the bone's index in the model.
    pub joint_hit_volume: Vec<Option<BoneHitShape>>,
    pub contents: Option<u32>,
    pub collision_detail_tier: i16,
    pub collision_faces: Vec<SurfaceHitMesh>,
    /// What a hit on a bone box shows when the model has no collision
    /// surface there: its paint's surface (a mannequin is plastic). 0 plays
    /// blood.
    pub face_flags: u32,
    /// The model's mins and maxs.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub radius: Option<f32>,
    /// bo2mp: the model's physics collmap, what BO2 traces a script model against
    /// (DObjGetCollmap) instead of its bounds; None when it has none.
    pub collmap: Option<ModelCollmap>,
}

/// bo2mp: a model's physics collmap (BO2 `Collmap`): the contents of every collmap
/// (XModelGetContents adds them to the model's own) and the first collmap's pieces as
/// boxes in model space.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCollmap {
    pub contents: u32,
    pub boxes: Vec<CollmapBox>,
    /// The first piece's brush corners in model space when it is a brush (empty
    /// otherwise): what a vehicle that moves by its collmap sweeps.
    pub brush_verts: Vec<[f32; 3]>,
}

/// bo2mp: one collmap piece as a turned box in model space: its middle, its three axes
/// and its half-size along each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollmapBox {
    pub center: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub half: [f32; 3],
}

/// One triangle of a collision surface: its plane and its two edge planes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTriangle {
    pub plane: [f32; 4],
    pub surface_tangent: [f32; 4],
    pub surface_bitangent: [f32; 4],
}

/// A collision surface of a model, riding one bone.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceHitMesh {
    pub bone: u16,
    pub contents: u32,
    pub face_property_bits: u32,
    pub midpoint: [f32; 3],
    pub half_ext: [f32; 3],
    pub triangle_list: Vec<HitTriangle>,
}

/// A hit box placed in the world: its middle, its unit axes and its half-size along each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitShapeBone {
    pub bone: u16,
    pub body_part_category: u8,
    pub center: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub half_ext: [f32; 3],
}

/// Why a model could not be posed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebuildFault {
    Skeleton(String),
    /// Anims played, but none of their tracks names a bone of the model.
    NoTrackForClip,
    NonFiniteBoneTransform {
        bone: usize,
    },
    /// A bone past what a `u16` can name.
    BoneIndexOverflow {
        bone: usize,
    },
    XAnimTree(AnimGraphFault),
}

impl core::convert::From<SkeletonBuildFault> for RebuildFault {
    fn from(error: SkeletonBuildFault) -> Self {
        Self::Skeleton(error.to_string())
    }
}

impl core::convert::From<AnimGraphFault> for RebuildFault {
    fn from(error: AnimGraphFault) -> Self {
        Self::XAnimTree(error)
    }
}

impl KeptModelAbility {
    /// The model alone as a DObj.
    fn lone_dobj(&self) -> Result<PosedSkeleton, SkeletonBuildFault> {
        PosedSkeleton::build(&[(&self.pose, None)])
    }

    /// Each bone's world matrix.
    pub fn pose(&self, request: &ModelPoseQuery, placement_matrix: Mat4) -> Result<Vec<Mat4>, RebuildFault> {
        self.pose_using_aim(request, placement_matrix, leave_pose_alone)
    }

    /// Each bone's world matrix, with `drive` changing the local pose after the anims.
    pub fn pose_using_aim(
        &self,
        request: &ModelPoseQuery,
        placement_matrix: Mat4,
        drive: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
    ) -> Result<Vec<Mat4>, RebuildFault> {
        let dobj = self.lone_dobj()?;
        pose_skeleton_with_aim(&dobj, request, placement_matrix, drive)
    }

    /// The bones' hit boxes, posed.
    pub fn hit_shapes(
        &self,
        request: &ModelPoseQuery,
        placement_matrix: Mat4,
    ) -> Result<Vec<HitShapeBone>, RebuildFault> {
        let single = [(self, None)];
        hit_shape_models(&single, request, placement_matrix)
    }

    /// The collision surfaces whose contents meet `hit_contents`, as boxes on their posed
    /// bones. A model without collision surfaces gives its bones' hit boxes.
    pub fn mesh_hit_probe(
        &self,
        request: &ModelPoseQuery,
        placement_matrix: Mat4,
        hit_contents: u32,
    ) -> Result<Vec<HitShapeBone>, RebuildFault> {
        if self.collision_faces.is_empty() {
            return Self::hit_shapes(self, request, placement_matrix);
        }
        let joint_transforms = pose_dobj(&self.lone_dobj()?, request, placement_matrix)?;
        let mut boxes = Vec::new();
        let meeting = self.collision_faces.iter().filter(|s| s.contents & hit_contents != 0);
        for face in meeting {
            let index = usize::from(face.bone);
            if let Some(&matrix) = joint_transforms.get(index) {
                boxes.push(placed_box(index, face.midpoint, face.half_ext, 0, matrix)?);
            }
        }
        Ok(boxes)
    }

    /// The model's bounds as one box on bone 0, or a cube of its radius when it has no
    /// bounds with size. `None` when it has neither.
    pub fn bounds_collision_bone(&self, placement_matrix: Mat4) -> Option<HitShapeBone> {
        let from_bounds = self.bounds.and_then(|(lo, hi)| {
            let half_ext: [f32; 3] = from_fn(|ax_ix| (hi[ax_ix] - lo[ax_ix]) * 0.5);
            let has_extent = half_ext.iter().any(|&extent| extent > 0.0);
            has_extent.then(|| (from_fn(|axis| (lo[axis] + hi[axis]) * 0.5), half_ext))
        });
        let radius_cube = || {
            self.radius
                .filter(|r| r.is_finite() && *r > 0.0)
                .map(|r| ([0.0; 3], [r; 3]))
        };
        let (centre, half) = from_bounds.or_else(radius_cube)?;
        collision_bone_from_local_box(0, centre, half, placement_matrix)
    }
}

/// The unit directions `m` turns the three axes to, and how far it stretches each.
/// `None` when an axis collapses or is not finite.
fn box_axes(m: Mat4) -> Option<([[f32; 3]; 3], [f32; 3])> {
    let mut stretch = [0.0f32; 3];
    let mut dirs = [[0.0f32; 3]; 3];
    for (axis, unit) in Vec3::AXES.into_iter().enumerate() {
        let turned = m.transform_vector3(unit);
        let length = turned.length();
        if !(length.is_finite() && length > 0.0) {
            return None;
        }
        stretch[axis] = length;
        dirs[axis] = (turned / length).to_array();
    }
    Some((dirs, stretch))
}

/// A box in a bone's space placed by the bone's world matrix.
fn placed_box(
    joint_ix: usize,
    mid: [f32; 3],
    half: [f32; 3],
    class: u8,
    bone_matrix: Mat4,
) -> Result<HitShapeBone, RebuildFault> {
    let Some((axes, stretch)) = box_axes(bone_matrix) else {
        return Err(RebuildFault::NonFiniteBoneTransform { bone: joint_ix });
    };
    let Ok(bone_index) = u16::try_from(joint_ix) else {
        return Err(RebuildFault::BoneIndexOverflow { bone: joint_ix });
    };
    Ok(HitShapeBone {
        half_ext: from_fn(|axis| half[axis] * stretch[axis]),
        axes,
        center: bone_matrix.transform_point3(mid.into()).to_array(),
        body_part_category: class,
        bone: bone_index,
    })
}

/// A box in model space placed in the world. `None` when the box has no size or a bad
/// number, or the placement squashes an axis.
pub fn collision_bone_from_local_box(
    bone: u16,
    mid: [f32; 3],
    half: [f32; 3],
    placement_matrix: Mat4,
) -> Option<HitShapeBone> {
    let sane = mid.iter().all(|c| c.is_finite())
        && half.iter().all(|h| h.is_finite() && *h >= 0.0)
        && half != [0.0; 3];
    if !sane {
        return None;
    }
    let center = placement_matrix.transform_point3(mid.into());
    let (axes, stretch) = box_axes(placement_matrix).filter(|_| center.is_finite())?;
    Some(HitShapeBone {
        bone,
        body_part_category: Default::default(),
        center: center.to_array(),
        axes,
        half_ext: from_fn(|axis| half[axis] * stretch[axis]),
    })
}

/// The hit boxes of models built into one DObj, posed.
pub fn hit_shape_models(
    models: &[(&KeptModelAbility, Option<MountedModel>)],
    request: &ModelPoseQuery,
    placement_matrix: Mat4,
) -> Result<Vec<HitShapeBone>, RebuildFault> {
    hit_shape_models_with_aim(models, request, placement_matrix, leave_pose_alone)
}

/// [`hit_shape_models`], with `after_anims` changing the local pose after the anims.
pub fn hit_shape_models_with_aim(
    models: &[(&KeptModelAbility, Option<MountedModel>)],
    request: &ModelPoseQuery,
    placement_matrix: Mat4,
    after_anims: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
) -> Result<Vec<HitShapeBone>, RebuildFault> {
    let Some(_) = models.first() else {
        return Ok(Vec::new());
    };
    let parts: Vec<_> = models
        .iter()
        .map(|&(model, ref tag_name)| (&model.pose, tag_name.clone()))
        .collect();
    let dobj = PosedSkeleton::build(&parts)?;
    hit_skeleton_with_aim(&dobj, models, request, placement_matrix, after_anims)
}

/// The hit boxes of `models` on a DObj already built from them, posed.
pub fn hit_skeleton_with_aim(
    skel: &PosedSkeleton,
    models: &[(&KeptModelAbility, Option<MountedModel>)],
    request: &ModelPoseQuery,
    placement_matrix: Mat4,
    tweak_pose: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
) -> Result<Vec<HitShapeBone>, RebuildFault> {
    match models {
        [] => return Ok(Vec::new()),
        _ => {}
    }
    let joint_transforms = pose_skeleton_with_aim(skel, request, placement_matrix, tweak_pose)?;
    let mut found = Vec::new();
    for (slot, &(model, _)) in skel.models.iter().zip(models) {
        for (local, entry) in model.joint_hit_volume.iter().enumerate().take(slot.joint_total) {
            let index = slot.first_joint + local;
            let (Some(hit), Some(&matrix)) = (entry, joint_transforms.get(index)) else {
                continue;
            };
            found.push(placed_box(
                index,
                hit.midpoint,
                hit.half_ext,
                hit.body_part_category,
                matrix,
            )?);
        }
    }
    Ok(found)
}

/// Each bone's world matrix for a DObj.
pub fn pose_dobj(dobj: &PosedSkeleton, request: &ModelPoseQuery, placement_matrix: Mat4) -> Result<Vec<Mat4>, RebuildFault> {
    pose_skeleton_with_aim(dobj, request, placement_matrix, leave_pose_alone)
}

/// Each bone's world matrix for a DObj: the tree's leaves blended, its additive layers
/// laid on, then `then_pose`, then the duplicate bones copied.
pub fn pose_skeleton_with_aim(
    rig: &PosedSkeleton,
    request: &ModelPoseQuery,
    placement_matrix: Mat4,
    then_pose: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
) -> Result<Vec<Mat4>, RebuildFault> {
    let wanted = request.wanted_mask.unwrap_or_else(|| rig.full_part_set());
    let mut pose = if let Some(tree) = request.tree.as_ref() {
        let (clips_on, layers) = tree.active_pose();
        let (mut pose, _driven) = blend(rig, &clips_on, &wanted)?;
        add_layers(rig, &layers, &wanted, &mut pose)?;
        pose
    } else {
        rig.rest_local_pose()
    };
    then_pose(rig, &wanted, &mut pose);
    rig.copy_mirrored_parts(&mut pose);
    Ok(rig.world_pose(&pose, placement_matrix))
}

/// The leaves blended into a local pose, with which bones they drove. Playing leaves
/// whose tracks name none of the DObj's bones is an error.
fn blend(
    anim_obj: &PosedSkeleton,
    active: &[LiveClipLeaf<'_>],
    allowed_parts: &PartSet,
) -> Result<(LocalPose, Vec<bool>), RebuildFault> {
    let per_leaf: Vec<_> = active.iter().map(|leaf| anim_obj.tracks_for(leaf.reel)).collect();
    let some_bone_named = per_leaf.iter().flatten().any(Option::is_some);
    if !active.is_empty() && !some_bone_named {
        return Err(RebuildFault::NoTrackForClip);
    }
    let playing: Vec<_> = active
        .iter()
        .zip(&per_leaf)
        .map(|(&LiveClipLeaf { pieces: parts, weight, time, reel: clip }, tracks)| ClipPlayback {
            tracks,
            clip,
            time,
            weight,
            parts,
        })
        .collect();
    Ok(anim_obj.calc_anim_masked(&playing, allowed_parts))
}

/// Lays each additive layer onto `pose_parts`: the layer's leaves blended (with its own inner
/// layers laid on them) and added at the layer's weight to every bone they drove.
/// Returns which bones any layer changed.
fn add_layers(
    frame_obj: &PosedSkeleton,
    stack: &[LiveOverlayLayer<'_>],
    parts_on: &PartSet,
    pose_parts: &mut LocalPose,
) -> Result<Vec<bool>, RebuildFault> {
    let mut touched = vec![false; pose_parts.len()];
    stack.iter().try_for_each(|layer| -> Result<(), RebuildFault> {
        let (mut offsets, driven) = blend(frame_obj, &layer.leaves, parts_on)?;
        let nested = add_layers(frame_obj, &layer.inner, parts_on, &mut offsets)?;
        for (bone, flag) in touched.iter_mut().enumerate() {
            if driven[bone] || nested[bone] {
                pose_parts[bone] = apply_additive(pose_parts[bone], offsets[bone], layer.weight);
                *flag = true;
            }
        }
        Ok(())
    })?;
    Ok(touched)
}

/// `add` laid onto `write_target` at `weight`: the translation scaled and added, the rotation
/// eased from none toward `add` by `weight` (times its length) and turned on after
/// `write_target`'s. A zero rotation leaves `write_target`'s alone.
///
/// TODO(fact): BO2's additive blend is open (QUESTIONS.md 70); this is MW2's.
fn apply_additive(write_target: PartPose, plus: PartPose, weight: f32) -> PartPose {
    let length = plus.rotation.iter().map(|c| c * c).sum::<f32>().sqrt();
    let rotation = if length != 0.0 {
        let [x, y, z, w] = plus.rotation;
        let eased =
            Quat::from_xyzw(x * weight, y * weight, z * weight, 1.0 - weight * length + w * weight);
        (Quat::from_array(write_target.rotation) * eased).to_array()
    } else {
        write_target.rotation
    };
    let mut pose = PartPose {
        rotation,
        translation: write_target.translation,
    };
    for (axis, moved) in pose.translation.iter_mut().enumerate() {
        *moved += plus.translation[axis] * weight;
    }
    pose
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        SkeletalClip, Rotation, Track, RootShift, AnimGraphNodeSpec, GraphSlot, AnimGraphNodeType,
        NodeClock, AnimGraphBlueprint,
    };

    /// A root and a gun bone 10 units forward of it.
    fn gun_pose() -> PoseDonorModel {
        PoseDonorModel {
            name: "gun".into(),
            skeleton_joint_total: 2,
            top_joint_total: 1,
            size_scale: 1.0,
            fixed_size_parts: [0; 5],
            joint_labels: vec!["tag_origin".into(), "j_gun".into()],
            ancestor_chain: vec![1],
            rotation_quaternions: vec![[0, 0, 0, 32767]],
            shift_offset: vec![[10.0, 0.0, 0.0]],
            foundation_surface: vec![(Quat::IDENTITY, Vec3::ZERO); 2],
        }
    }

    fn gun() -> KeptModelAbility {
        KeptModelAbility {
            key: "gun".into(),
            pose: gun_pose(),
            joint_hit_volume: vec![
                None,
                Some(BoneHitShape {
                    midpoint: [1.0, 0.0, 0.0],
                    half_ext: [1.0, 2.0, 3.0],
                    reach_squared: 14.0,
                    body_part_category: 7,
                }),
            ],
            collision_detail_tier: -1,
            radius: None,
            collmap: None,
            bounds: Default::default(),
            contents: Default::default(),
            collision_faces: Default::default(),
            face_flags: 0,
        }
    }

    /// A clip that moves one bone by `by`.
    fn nudge(bone: &str, by: [f32; 3]) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: format!("nudge_{bone}"),
            fps: 30.0,
            frame_total: 30,
            looping: true,
            tracks: vec![Track {
                name: bone.into(),
                move_track: RootShift::Fixed(by),
                rotation: Rotation::Default,
            }],
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        })
    }

    /// A request playing a tree of `(parent, kind, weight)` nodes.
    fn playing(nodes: Vec<(Option<u16>, AnimGraphNodeType, f32)>) -> ModelPoseQuery {
        let weights: Vec<f32> = nodes.iter().map(|node| node.2).collect();
        let definitions = nodes
            .into_iter()
            .map(|(parent, kind, _)| AnimGraphNodeSpec {
                owner: parent.map(GraphSlot),
                kind,
            })
            .collect();
        let definition = AnimGraphBlueprint::new(definitions).unwrap();
        let mut tree = AnimGraphInstance::new(Arc::new(definition));
        for (node, weight) in (0..).zip(weights) {
            let state = NodeClock {
                weight,
                target_w: weight,
                ..Default::default()
            };
            tree.set_state(GraphSlot(node), state).unwrap();
        }
        let tree = Some(tree);
        ModelPoseQuery {
            tree,
            ..ModelPoseQuery::neutral_pose()
        }
    }

    fn leaf(reel: Arc<SkeletalClip>) -> AnimGraphNodeType {
        AnimGraphNodeType::Leaf { clip: reel, parts: None }
    }

    fn origin(m: Mat4) -> [f32; 3] {
        m.transform_point3(Vec3::ZERO).to_array()
    }

    #[test]
    fn the_bind_pose_holds_the_gun_forward() {
        let posed = gun()
            .pose(&ModelPoseQuery::neutral_pose(), Mat4::IDENTITY)
            .unwrap();
        assert_eq!(posed.len(), 2);
        assert_eq!(
            (origin(posed[0]), origin(posed[1])),
            ([0.0; 3], [10.0, 0.0, 0.0])
        );
    }

    #[test]
    fn a_playing_leaf_moves_its_bone() {
        let request = playing(vec![(None, leaf(nudge("j_gun", [0.0, 5.0, 0.0])), 1.0)]);
        let posed = gun().pose(&request, Mat4::IDENTITY).unwrap();
        assert_eq!(origin(posed[1]), [10.0, 5.0, 0.0]);
    }

    #[test]
    fn the_controller_runs_after_the_anims() {
        let request = playing(vec![(None, leaf(nudge("j_gun", [0.0, 5.0, 0.0])), 1.0)]);
        let posed = gun()
            .pose_using_aim(&request, Mat4::IDENTITY, |_, _, local_pose| {
                local_pose[1].translation = [0.0, 0.0, 7.0];
            })
            .unwrap();
        assert_eq!(origin(posed[1]), [10.0, 0.0, 7.0]);
    }

    #[test]
    fn a_clip_for_another_skeleton_is_an_error() {
        let request = playing(vec![(None, leaf(nudge("j_wheel", [1.0; 3])), 1.0)]);
        assert_eq!(
            gun().pose(&request, Mat4::IDENTITY).unwrap_err(),
            RebuildFault::NoTrackForClip
        );
    }

    #[test]
    fn an_additive_layer_adds_at_its_weight() {
        let request = playing(vec![
            (None, AnimGraphNodeType::Blend, 1.0),
            (Some(0), leaf(nudge("j_gun", [0.0, 5.0, 0.0])), 1.0),
            (Some(0), AnimGraphNodeType::Additive, 0.5),
            (Some(2), leaf(nudge("j_gun", [0.0, 0.0, 4.0])), 1.0),
        ]);
        let posed = gun().pose(&request, Mat4::IDENTITY).unwrap();
        assert_eq!(origin(posed[1]), [10.0, 5.0, 2.0]);
        assert_eq!(origin(posed[0]), [0.0; 3]);
    }

    #[test]
    fn additive_rotation_eases_in_by_weight() {
        let write_target = PartPose {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [1.0, 0.0, 0.0],
        };
        let half_turn = core::f32::consts::FRAC_1_SQRT_2;
        let add = PartPose {
            rotation: [0.0, 0.0, half_turn, half_turn],
            translation: [0.0, 2.0, 0.0],
        };
        let full = apply_additive(write_target, add, 1.0);
        assert_eq!(full.translation, [1.0, 2.0, 0.0]);
        assert!((0..4).all(|k| (full.rotation[k] - add.rotation[k]).abs() < 1e-6));
        assert_eq!(apply_additive(write_target, add, 0.0), write_target);
        let none = PartPose {
            rotation: [0.0; 4],
            translation: [0.0; 3],
        };
        assert_eq!(apply_additive(write_target, none, 0.5).rotation, write_target.rotation);
    }

    #[test]
    fn bone_boxes_follow_their_bones() {
        let world_xf = Mat4::from_translation(Vec3::new(0.0, 0.0, 100.0));
        let boxes = gun()
            .hit_shapes(&ModelPoseQuery::neutral_pose(), world_xf)
            .unwrap();
        assert_eq!(
            boxes,
            vec![HitShapeBone {
                bone: 1,
                body_part_category: 7,
                center: [11.0, 0.0, 100.0],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                half_ext: [1.0, 2.0, 3.0],
            }]
        );
        assert_eq!(
            hit_shape_models(&[], &ModelPoseQuery::neutral_pose(), world_xf),
            Ok(vec![])
        );
    }

    #[test]
    fn surfaces_hit_only_for_their_contents() {
        let mut model = gun();
        let surf = |bone, contents| SurfaceHitMesh {
            bone,
            contents,
            face_property_bits: 0,
            midpoint: [0.0, 0.0, 1.0],
            half_ext: [1.0; 3],
            triangle_list: Default::default(),
        };
        model.collision_faces = vec![surf(1, 1), surf(0, 2)];
        let request = ModelPoseQuery::neutral_pose();
        let boxes = model.mesh_hit_probe(&request, Mat4::IDENTITY, 1).unwrap();
        assert_eq!(boxes.len(), 1);
        let hit = boxes[0];
        assert_eq!((hit.bone, hit.body_part_category), (1, 0));
        assert_eq!((hit.center, hit.half_ext), ([10.0, 0.0, 1.0], [1.0; 3]));
        assert!(
            model
                .mesh_hit_probe(&request, Mat4::IDENTITY, 4)
                .unwrap()
                .is_empty()
        );
        // No surfaces: the bone boxes stand in.
        let bones = gun().mesh_hit_probe(&request, Mat4::IDENTITY, 4).unwrap();
        assert_eq!(bones[0].body_part_category, 7);
    }

    #[test]
    fn a_local_box_takes_the_placement_stretch() {
        let stretch_xf = Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0))
            * Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0));
        let placed = collision_bone_from_local_box(3, [1.0, 0.0, 0.0], [1.0, 2.0, 3.0], stretch_xf);
        let placed = placed.unwrap();
        assert_eq!((placed.bone, placed.center), (3, [2.0, 0.0, 5.0]));
        assert_eq!(placed.half_ext, [2.0, 2.0, 3.0]);
        assert_eq!(placed.axes[0], [1.0, 0.0, 0.0]);
        let place = |mid, half, m| collision_bone_from_local_box(0, mid, half, m);
        assert_eq!(place([0.0; 3], [0.0; 3], Mat4::IDENTITY), None);
        assert_eq!(place([0.0; 3], [-1.0, 1.0, 1.0], Mat4::IDENTITY), None);
        assert_eq!(place([f32::NAN, 0.0, 0.0], [1.0; 3], Mat4::IDENTITY), None);
        assert_eq!(place([0.0; 3], [1.0; 3], Mat4::ZERO), None);
    }

    #[test]
    fn bounds_or_radius_make_one_box() {
        let mut model = gun();
        assert_eq!(model.bounds_collision_bone(Mat4::IDENTITY), None);
        model.radius = Some(5.0);
        let cube = model.bounds_collision_bone(Mat4::IDENTITY).unwrap();
        assert_eq!((cube.center, cube.half_ext), ([0.0; 3], [5.0; 3]));
        model.bounds = Some(([-1.0, -2.0, 0.0], [3.0, 2.0, 4.0]));
        let fitted = model.bounds_collision_bone(Mat4::IDENTITY).unwrap();
        assert_eq!(
            (fitted.center, fitted.half_ext),
            ([1.0, 0.0, 2.0], [2.0; 3])
        );
        // Flat bounds fall back to the radius.
        model.bounds = Some(([1.0; 3], [1.0; 3]));
        let cube = model.bounds_collision_bone(Mat4::IDENTITY).unwrap();
        assert_eq!(cube.half_ext, [5.0; 3]);
    }
}
