//! Collision for animated models: triangles that follow the bones they are skinned to, and the helpers that carry
//! a point between world space and a model's own space.

use std::sync::Arc;

use super::SHOT_WORLD_FILTER;

/// What a model's collision looks like in the world right now: boxes per bone, and (when the model has collision
/// triangles) the triangle set with the pose to test it in.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerModelCollision {
    pub bones: Vec<xmodel_runtime::HitShapeBone>,
    pub coll: Option<ServerModelHitTrace>,
    /// The boxes' surface where no collision surface says (the model's paint: a mannequin is plastic); 0 plays blood.
    pub face_bits: u32,
}

/// An animated model's triangles plus everything needed to trace a line through them.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerModelHitTrace {
    pub model: Arc<DObjColl>,
    pub placement_matrix: glam::Mat4,
    pub bones: Vec<DObjCollBone>,
    pub hidden_parts: [u32; xmodel_runtime::HiddenParts::BIT_CHUNK_COUNT],
}

/// A rigid transform: three axis rows and an origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidPlacement {
    pub frame: [[f32; 3]; 3],
    pub shift_offset: [f32; 3],
}

fn inner(u: [f32; 3], v: [f32; 3]) -> f32 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

const AXES_UNROTATED: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

impl RigidPlacement {
    /// Leaves every point where it is.
    pub const UNMOVED: Self = Self { frame: AXES_UNROTATED, shift_offset: [0.0; 3] };

    /// Undoes this transform: the axes transpose, and the origin is run back through them.
    fn inverse(&self) -> Self {
        let rows = &self.frame;
        let axis = [0, 1, 2].map(|i| [rows[0][i], rows[1][i], rows[2][i]]);
        let shift_offset = axis.map(|row| -inner(row, self.shift_offset));
        Self { frame: axis, shift_offset }
    }

    /// The transform that does `self` first and `next` second.
    fn followed_by(&self, next: &Self) -> Self {
        let column = |j: usize| [next.frame[0][j], next.frame[1][j], next.frame[2][j]];
        let axis = self.frame.map(|row| [0, 1, 2].map(|j| inner(row, column(j))));
        let shift_offset = [0, 1, 2].map(|j| inner(self.shift_offset, column(j)) + next.shift_offset[j]);
        Self { frame: axis, shift_offset }
    }

    /// Where `point` was before this transform moved it.
    fn undo_point(&self, at: [f32; 3]) -> [f32; 3] {
        let offset = [at[0] - self.shift_offset[0], at[1] - self.shift_offset[1], at[2] - self.shift_offset[2]];
        self.frame.map(|row| inner(offset, row))
    }
}

/// One bone of an animated model's collision: where it stands now, and where it stood in the bind pose the
/// triangles were authored in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DObjCollBone {
    pub posed: RigidPlacement,
    pub bind: RigidPlacement,
}

/// One surface of an animated model: a run of its triangles, skinned to a single bone.
#[derive(Clone, Debug, PartialEq)]
pub struct DObjCollSurf {
    pub leading_triangle: usize,
    pub tri_tally: usize,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub joint_position: i32,
    pub solid_bits: u32,
    pub face_property_bits: u32,
}

/// An animated model's collision triangles, owned outright (the game's own table borrows from the model).
#[derive(Clone, Debug, PartialEq)]
pub struct DObjColl {
    pub collision_detail_tier: i32,
    pub solid_bits: u32,
    pub triangle_list: Vec<clipmap_t6::ModelHitTriangle>,
    pub surfs: Vec<DObjCollSurf>,
}

/// Whether the bit for `bone` is set in a hide-part word array. Bone 0 is the top bit of word 0.
pub(super) fn part_is_hidden(words: &[u32], joint: usize) -> bool {
    words.get(joint / 32).is_some_and(|word| word & (0x8000_0000 >> (joint % 32)) != 0)
}

impl DObjColl {
    /// Traces the line `start..end` through the posed model. Every visible surface whose contents match is tested
    /// in its bone's bind space (the line is carried there from the pose). Returns the bone of the last surface
    /// the line struck, or -1 if it struck none.
    pub(super) fn trace_line_animated(
        &self,
        trace: &mut trace_t6::Trace,
        begin_at: [f32; 3],
        end: [f32; 3],
        stuff_filter: u32,
        bones: &[DObjCollBone],
        hidden: &[u32],
    ) -> i32 {
        if self.collision_detail_tier < 0 {
            return -1;
        }
        let mut struck = -1;
        for run in self.surfs.iter().filter(|run| run.solid_bits & stuff_filter != 0) {
            let bone = usize::try_from(run.joint_position).unwrap_or(0);
            if part_is_hidden(hidden, bone) {
                continue;
            }
            let (from, to) = match bones.get(bone) {
                Some(b) if b.posed != b.bind => {
                    let into_bind = b.bind.inverse().followed_by(&b.posed);
                    (into_bind.undo_point(begin_at), into_bind.undo_point(end))
                }
                _ => (begin_at, end),
            };
            let triangle_list = self.triangle_list.get(run.leading_triangle..run.leading_triangle + run.tri_tally).unwrap_or(&[]);
            let one_surf = [clipmap_t6::XModelCollSurf {
                triangle_list,
                mins: run.lo,
                maxs: run.hi,
                joint_position: run.joint_position,
                contents: run.solid_bits,
                face_property_bits: run.face_property_bits,
            }];
            let model = clipmap_t6::XModelColl { collision_detail_tier: self.collision_detail_tier, surfs: &one_surf };
            if model.trace_line(trace, from, to, stuff_filter) >= 0 {
                struck = run.joint_position;
            }
        }
        struck
    }
}

/// The rotation and translation of `m` as axis rows plus an offset; any scale is dropped.
fn rigid_from_matrix(m: glam::Mat4) -> RigidPlacement {
    let (_size, rotation, shift_offset) = m.to_scale_rotation_translation();
    let turn = glam::Mat4::from_quat(rotation);
    let columns = [turn.x_axis, turn.y_axis, turn.z_axis];
    RigidPlacement {
        frame: [0, 1, 2].map(|row| columns.map(|column| column[row])),
        shift_offset: shift_offset.to_array(),
    }
}

/// Packs a kept model's collision faces into one shared triangle list, each face keeping
/// its own run of it.
fn collision_from_capability(kept: &xmodel_runtime::KeptModelAbility) -> DObjColl {
    let mut triangle_list = Vec::new();
    let mut surfs = Vec::with_capacity(kept.collision_faces.len());
    for face in &kept.collision_faces {
        let leading_triangle = triangle_list.len();
        triangle_list.extend(face.triangle_list.iter().map(|src| clipmap_t6::ModelHitTriangle {
            plane: src.plane,
            surface_tangent: src.surface_tangent,
            surface_bitangent: src.surface_bitangent,
        }));
        let low = [0, 1, 2].map(|i| face.midpoint[i] - face.half_ext[i]);
        let high = [0, 1, 2].map(|i| face.midpoint[i] + face.half_ext[i]);
        surfs.push(DObjCollSurf {
            leading_triangle,
            tri_tally: face.triangle_list.len(),
            lo: low,
            hi: high,
            joint_position: i32::from(face.bone),
            solid_bits: face.contents,
            face_property_bits: face.face_property_bits,
        });
    }
    DObjColl {
        collision_detail_tier: i32::from(kept.collision_detail_tier),
        solid_bits: kept.contents.unwrap_or_default(),
        triangle_list,
        surfs,
    }
}

/// Builds the triangle trace for the model posed as `pose_query` asks. `Ok(None)` when the
/// model has no triangles a shot could hit (no collision detail, or no face in the shot mask).
pub(super) fn build_coll_trace(
    kept: &xmodel_runtime::KeptModelAbility,
    pose_query: &xmodel_runtime::ModelPoseQuery,
    placement_matrix: glam::Mat4,
) -> Result<Option<ServerModelHitTrace>, xmodel_runtime::RebuildFault> {
    let shootable_face = kept.collision_faces.iter().any(|face| face.contents & SHOT_WORLD_FILTER != 0);
    if kept.collision_detail_tier < 0 || !shootable_face {
        return Ok(None);
    }
    let unplaced = glam::Mat4::IDENTITY;
    let current = kept.pose(pose_query, unplaced)?;
    let rest = kept.pose(&xmodel_runtime::ModelPoseQuery::neutral_pose(), unplaced)?;
    let placement_of = |poses: &[glam::Mat4], bone: usize| match poses.get(bone) {
        Some(&m) => rigid_from_matrix(m),
        None => RigidPlacement::UNMOVED,
    };
    let bone_total = current.len().max(rest.len());
    let bones = (0..bone_total)
        .map(|bone| DObjCollBone { posed: placement_of(&current, bone), bind: placement_of(&rest, bone) })
        .collect();
    let hidden_words = *pose_query.hidden_mask.words();
    Ok(Some(ServerModelHitTrace {
        model: Arc::new(collision_from_capability(kept)),
        placement_matrix,
        bones,
        hidden_parts: hidden_words,
    }))
}

/// Model matrix for `origin` and game angles in degrees (pitch, yaw, roll), applied as
/// yaw, then pitch, then roll.
pub(super) fn matrix_from_angles(origin: glam::Vec3, aim: [f32; 3]) -> glam::Mat4 {
    let radians = aim.map(f32::to_radians);
    let turn = glam::Quat::from_euler(glam::EulerRot::ZYX, radians[1], radians[0], radians[2]);
    glam::Mat4::from_rotation_translation(turn, origin)
}

/// The origin and game angles (pitch, yaw, roll) a placement made by [`matrix_from_angles`] stands at.
pub(super) fn angles_from_matrix(placement: glam::Mat4) -> ([f32; 3], [f32; 3]) {
    let (_, turn, origin) = placement.to_scale_rotation_translation();
    let (yaw, pitch, roll) = turn.to_euler(glam::EulerRot::ZYX);
    (origin.to_array(), [pitch, yaw, roll].map(f32::to_degrees))
}

/// `point` expressed in the model's own space; `None` when the matrix does not invert to a
/// finite result.
pub(super) fn world_point_to_entity(placement_matrix: glam::Mat4, at: [f32; 3]) -> Option<[f32; 3]> {
    let model_space = placement_matrix.inverse().transform_point3(glam::Vec3::from(at));
    if model_space.is_finite() { Some(model_space.into()) } else { None }
}

/// A model-space direction rotated into world space and renormalised. A result with no
/// length hands back the input unchanged.
pub(super) fn entity_normal_to_world(placement_matrix: glam::Mat4, face: [f32; 3]) -> [f32; 3] {
    let (_, rotation, _) = placement_matrix.to_scale_rotation_translation();
    let turned = rotation * glam::Vec3::from(face);
    let length = turned.length();
    if length <= 0.0 { face } else { (turned / length).into() }
}
