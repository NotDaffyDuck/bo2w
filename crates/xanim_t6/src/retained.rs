//! A model kept loaded for posing and hit tests: its skeleton, the boxes its bones and
//! surfaces collide with, and its bounds. Posing plays a resolved anim tree on a DObj.

use std::array::from_fn;

use dobj_t6::{Attach, DObj, DObjError, HidePartBits, LocalPose, ModelPoseSrc, PartBits, PartPose};
use glam::{Mat4, Quat, Vec3};

use crate::tree::{ActiveAdditiveLayer, ActiveXAnimLeaf};
use crate::{AnimInstance, DObjAnim, XAnimTreeError, XAnimTreeRuntime};

/// A bone's hit box in the bone's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneCollision {
    pub midpoint: [f32; 3],
    pub half_size: [f32; 3],
    pub radius_sq: f32,
    /// Which body part a hit on it counts as.
    pub part_classification: u8,
}

/// What to pose a DObj with.
#[derive(Clone, Debug, Default)]
pub struct DObjPoseRequest {
    /// The anim tree playing; `None` holds the bind pose.
    pub tree: Option<XAnimTreeRuntime>,
    /// The parts posed; every part when `None`.
    pub requested_parts: Option<PartBits>,
    pub hide_part_bits: HidePartBits,
}

impl DObjPoseRequest {
    /// The bind pose, every part posed, nothing hidden.
    pub const fn bind_pose() -> Self {
        Self {
            tree: None,
            requested_parts: None,
            hide_part_bits: HidePartBits::from_words([0; HidePartBits::WORDS]),
        }
    }
}

/// A loaded model, ready to pose and hit-test.
#[derive(Clone, Debug)]
pub struct RetainedModelCapability {
    pub key: String,
    pub pose: ModelPoseSrc,
    /// A hit box per bone, by the bone's index in the model.
    pub bone_collision: Vec<Option<BoneCollision>>,
    pub contents: Option<u32>,
    pub coll_lod: i16,
    pub coll_surfs: Vec<CollSurfCollision>,
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
pub struct CollTri {
    pub plane: [f32; 4],
    pub svec: [f32; 4],
    pub tvec: [f32; 4],
}

/// A collision surface of a model, riding one bone.
#[derive(Clone, Debug, PartialEq)]
pub struct CollSurfCollision {
    pub bone: u16,
    pub contents: u32,
    pub surf_flags: u32,
    pub midpoint: [f32; 3],
    pub half_size: [f32; 3],
    pub tris: Vec<CollTri>,
}

/// A hit box placed in the world: its middle, its unit axes and its half-size along each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionBone {
    pub bone: u16,
    pub part_classification: u8,
    pub center: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub half_size: [f32; 3],
}

/// Why a model could not be posed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterializeError {
    DObj(String),
    /// Anims played, but none of their tracks names a bone of the model.
    AnimatedClipHasNoMatchingTrack,
    NonFiniteBoneTransform {
        bone: usize,
    },
    /// A bone past what a `u16` can name.
    BoneIndexOverflow {
        bone: usize,
    },
    XAnimTree(XAnimTreeError),
}

impl From<DObjError> for MaterializeError {
    fn from(error: DObjError) -> Self {
        Self::DObj(error.to_string())
    }
}

impl From<XAnimTreeError> for MaterializeError {
    fn from(error: XAnimTreeError) -> Self {
        Self::XAnimTree(error)
    }
}

impl RetainedModelCapability {
    /// Each bone's world matrix.
    pub fn pose(
        &self,
        request: &DObjPoseRequest,
        world_from_model: Mat4,
    ) -> Result<Vec<Mat4>, MaterializeError> {
        self.pose_with_controller(request, world_from_model, |_, _, _| {})
    }

    /// Each bone's world matrix, with `controller` changing the local pose after the anims.
    pub fn pose_with_controller(
        &self,
        request: &DObjPoseRequest,
        world_from_model: Mat4,
        controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
    ) -> Result<Vec<Mat4>, MaterializeError> {
        let dobj = DObj::build(&[(&self.pose, None)])?;
        pose_dobj_with_controller(&dobj, request, world_from_model, controller)
    }

    /// The bones' hit boxes, posed.
    pub fn collision(
        &self,
        request: &DObjPoseRequest,
        world_from_model: Mat4,
    ) -> Result<Vec<CollisionBone>, MaterializeError> {
        collision_models(&[(self, None)], request, world_from_model)
    }

    /// The collision surfaces whose contents meet `contentmask`, as boxes on their posed
    /// bones. A model without collision surfaces gives its bones' hit boxes.
    pub fn geom_collision(
        &self,
        request: &DObjPoseRequest,
        world_from_model: Mat4,
        contentmask: u32,
    ) -> Result<Vec<CollisionBone>, MaterializeError> {
        if self.coll_surfs.is_empty() {
            return self.collision(request, world_from_model);
        }
        let dobj = DObj::build(&[(&self.pose, None)])?;
        let posed = pose_dobj(&dobj, request, world_from_model)?;
        let mut boxes = Vec::new();
        for surf in &self.coll_surfs {
            if surf.contents & contentmask == 0 {
                continue;
            }
            let bone = usize::from(surf.bone);
            let Some(&bone_matrix) = posed.get(bone) else {
                continue;
            };
            boxes.push(placed_box(
                bone,
                surf.midpoint,
                surf.half_size,
                0,
                bone_matrix,
            )?);
        }
        Ok(boxes)
    }

    /// The model's bounds as one box on bone 0, or a cube of its radius when it has no
    /// bounds with size. `None` when it has neither.
    pub fn bounds_collision_bone(&self, world_from_model: Mat4) -> Option<CollisionBone> {
        let sized = self.bounds.and_then(|(mins, maxs)| {
            let half: [f32; 3] = from_fn(|k| (maxs[k] - mins[k]) * 0.5);
            half.iter()
                .any(|&h| h > 0.0)
                .then(|| (from_fn(|k| (mins[k] + maxs[k]) * 0.5), half))
        });
        let (mid, half) = match (sized, self.radius) {
            (Some(sized), _) => sized,
            (None, Some(r)) if r.is_finite() && r > 0.0 => ([0.0; 3], [r; 3]),
            _ => return None,
        };
        collision_bone_from_local_box(0, mid, half, world_from_model)
    }
}

/// The unit directions `m` turns the three axes to, and how far it stretches each.
/// `None` when an axis collapses or is not finite.
fn box_axes(m: Mat4) -> Option<([[f32; 3]; 3], [f32; 3])> {
    let mut axes = [[0.0; 3]; 3];
    let mut scales = [0.0; 3];
    for k in 0..3 {
        let dir = m.transform_vector3(Vec3::AXES[k]);
        let scale = dir.length();
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        axes[k] = (dir / scale).to_array();
        scales[k] = scale;
    }
    Some((axes, scales))
}

/// A box in a bone's space placed by the bone's world matrix.
fn placed_box(
    bone: usize,
    mid: [f32; 3],
    half: [f32; 3],
    part_classification: u8,
    bone_matrix: Mat4,
) -> Result<CollisionBone, MaterializeError> {
    let (axes, scales) =
        box_axes(bone_matrix).ok_or(MaterializeError::NonFiniteBoneTransform { bone })?;
    let index = u16::try_from(bone).map_err(|_| MaterializeError::BoneIndexOverflow { bone })?;
    Ok(CollisionBone {
        bone: index,
        part_classification,
        center: bone_matrix
            .transform_point3(Vec3::from_array(mid))
            .to_array(),
        axes,
        half_size: from_fn(|k| half[k] * scales[k]),
    })
}

/// A box in model space placed in the world. `None` when the box has no size or a bad
/// number, or the placement squashes an axis.
pub fn collision_bone_from_local_box(
    bone: u16,
    mid: [f32; 3],
    half: [f32; 3],
    world_from_model: Mat4,
) -> Option<CollisionBone> {
    let bad_half = half.iter().any(|&h| !h.is_finite() || h < 0.0);
    if !mid.iter().all(|c| c.is_finite()) || bad_half || half == [0.0; 3] {
        return None;
    }
    let center = world_from_model.transform_point3(Vec3::from_array(mid));
    if !center.is_finite() {
        return None;
    }
    let (axes, scales) = box_axes(world_from_model)?;
    Some(CollisionBone {
        bone,
        part_classification: 0,
        center: center.to_array(),
        axes,
        half_size: from_fn(|k| half[k] * scales[k]),
    })
}

/// The hit boxes of models built into one DObj, posed.
pub fn collision_models(
    models: &[(&RetainedModelCapability, Option<Attach>)],
    request: &DObjPoseRequest,
    world_from_model: Mat4,
) -> Result<Vec<CollisionBone>, MaterializeError> {
    collision_models_with_controller(models, request, world_from_model, |_, _, _| {})
}

/// [`collision_models`], with `controller` changing the local pose after the anims.
pub fn collision_models_with_controller(
    models: &[(&RetainedModelCapability, Option<Attach>)],
    request: &DObjPoseRequest,
    world_from_model: Mat4,
    controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
) -> Result<Vec<CollisionBone>, MaterializeError> {
    if models.is_empty() {
        return Ok(Vec::new());
    }
    let sources: Vec<_> = models
        .iter()
        .map(|(model, attach)| (&model.pose, attach.clone()))
        .collect();
    let dobj = DObj::build(&sources)?;
    collision_dobj_with_controller(&dobj, models, request, world_from_model, controller)
}

/// The hit boxes of `models` on a DObj already built from them, posed.
pub fn collision_dobj_with_controller(
    dobj: &DObj,
    models: &[(&RetainedModelCapability, Option<Attach>)],
    request: &DObjPoseRequest,
    world_from_model: Mat4,
    controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
) -> Result<Vec<CollisionBone>, MaterializeError> {
    if models.is_empty() {
        return Ok(Vec::new());
    }
    let posed = pose_dobj_with_controller(dobj, request, world_from_model, controller)?;
    let mut boxes = Vec::new();
    for (slot, (model, _)) in dobj.models.iter().zip(models) {
        for local in 0..slot.bone_count {
            let Some(Some(hit)) = model.bone_collision.get(local) else {
                continue;
            };
            let bone = slot.base + local;
            let Some(&bone_matrix) = posed.get(bone) else {
                continue;
            };
            boxes.push(placed_box(
                bone,
                hit.midpoint,
                hit.half_size,
                hit.part_classification,
                bone_matrix,
            )?);
        }
    }
    Ok(boxes)
}

/// Each bone's world matrix for a DObj.
pub fn pose_dobj(
    dobj: &DObj,
    request: &DObjPoseRequest,
    world_from_model: Mat4,
) -> Result<Vec<Mat4>, MaterializeError> {
    pose_dobj_with_controller(dobj, request, world_from_model, |_, _, _| {})
}

/// Each bone's world matrix for a DObj: the tree's leaves blended, its additive layers
/// laid on, then `controller`, then the duplicate bones copied.
pub fn pose_dobj_with_controller(
    dobj: &DObj,
    request: &DObjPoseRequest,
    world_from_model: Mat4,
    controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
) -> Result<Vec<Mat4>, MaterializeError> {
    let requested = request.requested_parts.unwrap_or_else(|| dobj.all_parts());
    let mut locals = match &request.tree {
        Some(tree) => {
            let (leaves, layers) = tree.active_pose();
            let (mut locals, _) = blend(dobj, &leaves, &requested)?;
            add_layers(dobj, &layers, &requested, &mut locals)?;
            locals
        }
        None => dobj.bind_locals(),
    };
    controller(dobj, &requested, &mut locals);
    dobj.apply_duplicates(&mut locals);
    Ok(dobj.compose(&locals, world_from_model))
}

/// The leaves blended into a local pose, with which bones they drove. Playing leaves
/// whose tracks name none of the DObj's bones is an error.
fn blend(
    dobj: &DObj,
    leaves: &[ActiveXAnimLeaf<'_>],
    requested: &PartBits,
) -> Result<(LocalPose, Vec<bool>), MaterializeError> {
    let tracks: Vec<_> = leaves
        .iter()
        .map(|leaf| dobj.tracks_for(leaf.clip))
        .collect();
    let unmatched = tracks.iter().flatten().all(Option::is_none);
    if !leaves.is_empty() && unmatched {
        return Err(MaterializeError::AnimatedClipHasNoMatchingTrack);
    }
    let instances: Vec<_> = leaves
        .iter()
        .zip(&tracks)
        .map(|(leaf, tracks)| AnimInstance {
            clip: leaf.clip,
            tracks,
            time: leaf.time,
            weight: leaf.weight,
            parts: leaf.parts,
        })
        .collect();
    Ok(dobj.calc_anim_masked(&instances, requested))
}

/// Lays each additive layer onto `locals`: the layer's leaves blended (with its own inner
/// layers laid on them) and added at the layer's weight to every bone they drove.
/// Returns which bones any layer changed.
fn add_layers(
    dobj: &DObj,
    layers: &[ActiveAdditiveLayer<'_>],
    requested: &PartBits,
    locals: &mut LocalPose,
) -> Result<Vec<bool>, MaterializeError> {
    let mut changed = vec![false; locals.len()];
    for layer in layers {
        let (mut add, driven) = blend(dobj, &layer.leaves, requested)?;
        let inner = add_layers(dobj, &layer.inner, requested, &mut add)?;
        for bone in 0..locals.len() {
            if driven[bone] || inner[bone] {
                locals[bone] = apply_additive(locals[bone], add[bone], layer.weight);
                changed[bone] = true;
            }
        }
    }
    Ok(changed)
}

/// `add` laid onto `dest` at `weight`: the translation scaled and added, the rotation
/// eased from none toward `add` by `weight` (times its length) and turned on after
/// `dest`'s. A zero rotation leaves `dest`'s alone.
///
/// TODO(fact): BO2's additive blend is open (QUESTIONS.md 70); this is MW2's.
fn apply_additive(dest: PartPose, add: PartPose, weight: f32) -> PartPose {
    let translation = from_fn(|k| dest.translation[k] + add.translation[k] * weight);
    let [x, y, z, w] = add.rotation;
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let rotation = if length == 0.0 {
        dest.rotation
    } else {
        let eased = Quat::from_array([
            x * weight,
            y * weight,
            z * weight,
            1.0 - weight * length + w * weight,
        ]);
        (Quat::from_array(dest.rotation) * eased).to_array()
    };
    PartPose {
        rotation,
        translation,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        AnimClip, Rotation, Track, Translation, XAnimNodeDefinition, XAnimNodeId, XAnimNodeKind,
        XAnimNodeState, XAnimTreeDefinition,
    };

    /// A root and a gun bone 10 units forward of it.
    fn gun_pose() -> ModelPoseSrc {
        ModelPoseSrc {
            name: "gun".into(),
            num_bones: 2,
            num_root_bones: 1,
            scale: 1.0,
            no_scale_part_bits: [0; 5],
            bone_names: vec!["tag_origin".into(), "j_gun".into()],
            parent_list: vec![1],
            quats: vec![[0, 0, 0, 32767]],
            trans: vec![[10.0, 0.0, 0.0]],
            base_mat: vec![(Quat::IDENTITY, Vec3::ZERO); 2],
        }
    }

    fn gun() -> RetainedModelCapability {
        RetainedModelCapability {
            key: "gun".into(),
            pose: gun_pose(),
            bone_collision: vec![
                None,
                Some(BoneCollision {
                    midpoint: [1.0, 0.0, 0.0],
                    half_size: [1.0, 2.0, 3.0],
                    radius_sq: 14.0,
                    part_classification: 7,
                }),
            ],
            contents: None,
            coll_lod: -1,
            coll_surfs: Vec::new(),
            bounds: None,
            radius: None,
            collmap: None,
        }
    }

    /// A clip that moves one bone by `by`.
    fn nudge(bone: &str, by: [f32; 3]) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: format!("nudge_{bone}"),
            framerate: 30.0,
            numframes: 30,
            looping: true,
            tracks: vec![Track {
                name: bone.into(),
                rotation: Rotation::Default,
                translation: Translation::Constant(by),
            }],
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        })
    }

    /// A request playing a tree of `(parent, kind, weight)` nodes.
    fn playing(nodes: Vec<(Option<u16>, XAnimNodeKind, f32)>) -> DObjPoseRequest {
        let weights: Vec<f32> = nodes.iter().map(|node| node.2).collect();
        let definitions = nodes
            .into_iter()
            .map(|(parent, kind, _)| XAnimNodeDefinition {
                parent: parent.map(XAnimNodeId),
                kind,
            })
            .collect();
        let definition = XAnimTreeDefinition::new(definitions).unwrap();
        let mut tree = XAnimTreeRuntime::new(Arc::new(definition));
        for (node, weight) in (0..).zip(weights) {
            let state = XAnimNodeState {
                weight,
                goal_weight: weight,
                ..Default::default()
            };
            tree.set_state(XAnimNodeId(node), state).unwrap();
        }
        DObjPoseRequest {
            tree: Some(tree),
            ..DObjPoseRequest::bind_pose()
        }
    }

    fn leaf(clip: Arc<AnimClip>) -> XAnimNodeKind {
        XAnimNodeKind::Leaf { clip, parts: None }
    }

    fn origin(m: Mat4) -> [f32; 3] {
        m.transform_point3(Vec3::ZERO).to_array()
    }

    #[test]
    fn the_bind_pose_holds_the_gun_forward() {
        let posed = gun()
            .pose(&DObjPoseRequest::bind_pose(), Mat4::IDENTITY)
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
            .pose_with_controller(&request, Mat4::IDENTITY, |_, _, locals| {
                locals[1].translation = [0.0, 0.0, 7.0];
            })
            .unwrap();
        assert_eq!(origin(posed[1]), [10.0, 0.0, 7.0]);
    }

    #[test]
    fn a_clip_for_another_skeleton_is_an_error() {
        let request = playing(vec![(None, leaf(nudge("j_wheel", [1.0; 3])), 1.0)]);
        assert_eq!(
            gun().pose(&request, Mat4::IDENTITY).unwrap_err(),
            MaterializeError::AnimatedClipHasNoMatchingTrack
        );
    }

    #[test]
    fn an_additive_layer_adds_at_its_weight() {
        let request = playing(vec![
            (None, XAnimNodeKind::Blend, 1.0),
            (Some(0), leaf(nudge("j_gun", [0.0, 5.0, 0.0])), 1.0),
            (Some(0), XAnimNodeKind::Additive, 0.5),
            (Some(2), leaf(nudge("j_gun", [0.0, 0.0, 4.0])), 1.0),
        ]);
        let posed = gun().pose(&request, Mat4::IDENTITY).unwrap();
        assert_eq!(origin(posed[1]), [10.0, 5.0, 2.0]);
        assert_eq!(origin(posed[0]), [0.0; 3]);
    }

    #[test]
    fn additive_rotation_eases_in_by_weight() {
        let dest = PartPose {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [1.0, 0.0, 0.0],
        };
        let half_turn = core::f32::consts::FRAC_1_SQRT_2;
        let add = PartPose {
            rotation: [0.0, 0.0, half_turn, half_turn],
            translation: [0.0, 2.0, 0.0],
        };
        let full = apply_additive(dest, add, 1.0);
        assert_eq!(full.translation, [1.0, 2.0, 0.0]);
        assert!((0..4).all(|k| (full.rotation[k] - add.rotation[k]).abs() < 1e-6));
        assert_eq!(apply_additive(dest, add, 0.0), dest);
        let none = PartPose {
            rotation: [0.0; 4],
            translation: [0.0; 3],
        };
        assert_eq!(apply_additive(dest, none, 0.5).rotation, dest.rotation);
    }

    #[test]
    fn bone_boxes_follow_their_bones() {
        let placement = Mat4::from_translation(Vec3::new(0.0, 0.0, 100.0));
        let boxes = gun()
            .collision(&DObjPoseRequest::bind_pose(), placement)
            .unwrap();
        assert_eq!(
            boxes,
            vec![CollisionBone {
                bone: 1,
                part_classification: 7,
                center: [11.0, 0.0, 100.0],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                half_size: [1.0, 2.0, 3.0],
            }]
        );
        assert_eq!(
            collision_models(&[], &DObjPoseRequest::bind_pose(), placement),
            Ok(vec![])
        );
    }

    #[test]
    fn surfaces_hit_only_for_their_contents() {
        let mut model = gun();
        let surf = |bone, contents| CollSurfCollision {
            bone,
            contents,
            surf_flags: 0,
            midpoint: [0.0, 0.0, 1.0],
            half_size: [1.0; 3],
            tris: Vec::new(),
        };
        model.coll_surfs = vec![surf(1, 1), surf(0, 2)];
        let request = DObjPoseRequest::bind_pose();
        let boxes = model.geom_collision(&request, Mat4::IDENTITY, 1).unwrap();
        assert_eq!(boxes.len(), 1);
        let hit = boxes[0];
        assert_eq!((hit.bone, hit.part_classification), (1, 0));
        assert_eq!((hit.center, hit.half_size), ([10.0, 0.0, 1.0], [1.0; 3]));
        assert!(
            model
                .geom_collision(&request, Mat4::IDENTITY, 4)
                .unwrap()
                .is_empty()
        );
        // No surfaces: the bone boxes stand in.
        let bones = gun().geom_collision(&request, Mat4::IDENTITY, 4).unwrap();
        assert_eq!(bones[0].part_classification, 7);
    }

    #[test]
    fn a_local_box_takes_the_placement_stretch() {
        let placement = Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0))
            * Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0));
        let placed = collision_bone_from_local_box(3, [1.0, 0.0, 0.0], [1.0, 2.0, 3.0], placement);
        let placed = placed.unwrap();
        assert_eq!((placed.bone, placed.center), (3, [2.0, 0.0, 5.0]));
        assert_eq!(placed.half_size, [2.0, 2.0, 3.0]);
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
        assert_eq!((cube.center, cube.half_size), ([0.0; 3], [5.0; 3]));
        model.bounds = Some(([-1.0, -2.0, 0.0], [3.0, 2.0, 4.0]));
        let fitted = model.bounds_collision_bone(Mat4::IDENTITY).unwrap();
        assert_eq!(
            (fitted.center, fitted.half_size),
            ([1.0, 0.0, 2.0], [2.0; 3])
        );
        // Flat bounds fall back to the radius.
        model.bounds = Some(([1.0; 3], [1.0; 3]));
        let cube = model.bounds_collision_bone(Mat4::IDENTITY).unwrap();
        assert_eq!(cube.half_size, [5.0; 3]);
    }
}
