//! Collision for animated models: triangles that follow the bones they are skinned to, and the helpers that carry
//! a point between world space and a model's own space.

use std::sync::Arc;

use super::MASK_BULLET_WORLD;

/// A bone collision box, as the model runtime reports it.
pub type AuthorityDObjCollisionBone = xmodel_runtime::CollisionBone;

/// What a model's collision looks like in the world right now: boxes per bone, and (when the model has collision
/// triangles) the triangle set with the pose to test it in.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorityDObjCollision {
    pub bones: Vec<AuthorityDObjCollisionBone>,
    pub coll: Option<AuthorityDObjCollTrace>,
}

/// An animated model's triangles plus everything needed to trace a line through them.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorityDObjCollTrace {
    pub model: Arc<DObjColl>,
    pub world_from_model: glam::Mat4,
    pub bones: Vec<DObjCollBone>,
    pub hide_part_bits: [u32; xmodel_runtime::HidePartBits::WORDS],
}

/// A rigid transform: three axis rows and an origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidXform {
    pub axis: [[f32; 3]; 3],
    pub trans: [f32; 3],
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl RigidXform {
    pub const IDENTITY: Self = Self {
        axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        trans: [0.0, 0.0, 0.0],
    };

    /// Undoes this transform: the axes transpose, and the origin is run back through them.
    fn inverse(&self) -> Self {
        let rows = &self.axis;
        let axis = [0, 1, 2].map(|i| [rows[0][i], rows[1][i], rows[2][i]]);
        let trans = axis.map(|row| -dot3(row, self.trans));
        Self { axis, trans }
    }

    /// The transform that does `self` first and `next` second.
    fn followed_by(&self, next: &Self) -> Self {
        let column = |j: usize| [next.axis[0][j], next.axis[1][j], next.axis[2][j]];
        let axis = self.axis.map(|row| [0, 1, 2].map(|j| dot3(row, column(j))));
        let trans = [0, 1, 2].map(|j| dot3(self.trans, column(j)) + next.trans[j]);
        Self { axis, trans }
    }

    /// Where `point` was before this transform moved it.
    fn undo_point(&self, point: [f32; 3]) -> [f32; 3] {
        let offset = [point[0] - self.trans[0], point[1] - self.trans[1], point[2] - self.trans[2]];
        self.axis.map(|row| dot3(offset, row))
    }
}

/// One bone of an animated model's collision: where it stands now, and where it stood in the bind pose the
/// triangles were authored in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DObjCollBone {
    pub posed: RigidXform,
    pub bind: RigidXform,
}

/// One surface of an animated model: a run of its triangles, skinned to a single bone.
#[derive(Clone, Debug, PartialEq)]
pub struct DObjCollSurf {
    pub first_tri: usize,
    pub tri_count: usize,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub bone_idx: i32,
    pub contents: u32,
    pub surf_flags: u32,
}

/// An animated model's collision triangles, owned outright (the game's own table borrows from the model).
#[derive(Clone, Debug, PartialEq)]
pub struct DObjColl {
    pub coll_lod: i32,
    pub contents: u32,
    pub tris: Vec<clipmap_t6::XModelCollTri>,
    pub surfs: Vec<DObjCollSurf>,
}

/// Whether the bit for `bone` is set in a hide-part word array. Bone 0 is the top bit of word 0.
pub(super) fn part_is_hidden(words: &[u32], bone: usize) -> bool {
    words.get(bone / 32).is_some_and(|word| word & (0x8000_0000 >> (bone % 32)) != 0)
}

impl DObjColl {
    /// Traces the line `start..end` through the posed model. Every visible surface whose contents match is tested
    /// in its bone's bind space (the line is carried there from the pose). Returns the bone of the last surface
    /// the line struck, or -1 if it struck none.
    pub(super) fn trace_line_animated(
        &self,
        trace: &mut trace_t6::Trace,
        start: [f32; 3],
        end: [f32; 3],
        contentmask: u32,
        bones: &[DObjCollBone],
        hidden: &[u32],
    ) -> i32 {
        if self.coll_lod < 0 {
            return -1;
        }
        let mut struck = -1;
        for surf in self.surfs.iter().filter(|surf| surf.contents & contentmask != 0) {
            let bone = usize::try_from(surf.bone_idx).unwrap_or(0);
            if part_is_hidden(hidden, bone) {
                continue;
            }
            let (from, to) = match bones.get(bone) {
                Some(b) if b.posed != b.bind => {
                    let into_bind = b.bind.inverse().followed_by(&b.posed);
                    (into_bind.undo_point(start), into_bind.undo_point(end))
                }
                _ => (start, end),
            };
            let tris = self.tris.get(surf.first_tri..surf.first_tri + surf.tri_count).unwrap_or(&[]);
            let one_surf = [clipmap_t6::XModelCollSurf {
                tris,
                mins: surf.mins,
                maxs: surf.maxs,
                bone_idx: surf.bone_idx,
                contents: surf.contents,
                surf_flags: surf.surf_flags,
            }];
            let model = clipmap_t6::XModelColl { coll_lod: self.coll_lod, surfs: &one_surf };
            if model.trace_line(trace, from, to, contentmask) >= 0 {
                struck = surf.bone_idx;
            }
        }
        struck
    }
}

fn rigid_from_matrix(m: glam::Mat4) -> RigidXform {
    let (_scale, rotation, trans) = m.to_scale_rotation_translation();
    let r = glam::Mat4::from_quat(rotation);
    RigidXform {
        axis: [
            [r.x_axis.x, r.y_axis.x, r.z_axis.x],
            [r.x_axis.y, r.y_axis.y, r.z_axis.y],
            [r.x_axis.z, r.y_axis.z, r.z_axis.z],
        ],
        trans: trans.to_array(),
    }
}

/// Flattens a retained model's collision surfaces into one triangle list with a run per surface.
fn collision_from_capability(capability: &xmodel_runtime::RetainedModelCapability) -> DObjColl {
    let mut tris = Vec::new();
    let mut surfs = Vec::with_capacity(capability.coll_surfs.len());
    for surf in &capability.coll_surfs {
        let first_tri = tris.len();
        tris.extend(surf.tris.iter().map(|tri| clipmap_t6::XModelCollTri {
            plane: tri.plane,
            svec: tri.svec,
            tvec: tri.tvec,
        }));
        surfs.push(DObjCollSurf {
            first_tri,
            tri_count: surf.tris.len(),
            mins: [0, 1, 2].map(|i| surf.midpoint[i] - surf.half_size[i]),
            maxs: [0, 1, 2].map(|i| surf.midpoint[i] + surf.half_size[i]),
            bone_idx: i32::from(surf.bone),
            contents: surf.contents,
            surf_flags: surf.surf_flags,
        });
    }
    DObjColl {
        coll_lod: i32::from(capability.coll_lod),
        contents: capability.contents.unwrap_or(0),
        tris,
        surfs,
    }
}

/// The triangle trace for a model in `request`'s pose, or `None` when it has no triangles a bullet can strike.
pub(super) fn build_coll_trace(
    capability: &xmodel_runtime::RetainedModelCapability,
    request: &xmodel_runtime::DObjPoseRequest,
    world_from_model: glam::Mat4,
) -> Result<Option<AuthorityDObjCollTrace>, xmodel_runtime::MaterializeError> {
    let strikable = capability.coll_lod >= 0
        && capability.coll_surfs.iter().any(|surf| surf.contents & MASK_BULLET_WORLD != 0);
    if !strikable {
        return Ok(None);
    }
    let posed = capability.pose(request, glam::Mat4::IDENTITY)?;
    let bind = capability.pose(&xmodel_runtime::DObjPoseRequest::bind_pose(), glam::Mat4::IDENTITY)?;
    let bone_at = |poses: &[glam::Mat4], i: usize| {
        poses.get(i).copied().map_or(RigidXform::IDENTITY, rigid_from_matrix)
    };
    let bones = (0..posed.len().max(bind.len()))
        .map(|i| DObjCollBone { posed: bone_at(&posed, i), bind: bone_at(&bind, i) })
        .collect();
    Ok(Some(AuthorityDObjCollTrace {
        model: Arc::new(collision_from_capability(capability)),
        world_from_model,
        bones,
        hide_part_bits: *request.hide_part_bits.words(),
    }))
}

/// A model matrix from an origin and game angles (pitch, yaw, roll in degrees).
pub(super) fn matrix_from_angles(origin: glam::Vec3, angles: [f32; 3]) -> glam::Mat4 {
    let [pitch, yaw, roll] = angles.map(f32::to_radians);
    let rotation = glam::Quat::from_euler(glam::EulerRot::ZYX, yaw, pitch, roll);
    glam::Mat4::from_rotation_translation(rotation, origin)
}

/// A world point in the model's own space, or `None` if the matrix cannot be inverted into a finite point.
pub(super) fn world_point_to_entity(world_from_model: glam::Mat4, point: [f32; 3]) -> Option<[f32; 3]> {
    let local = world_from_model.inverse().transform_point3(glam::Vec3::from_array(point));
    local.is_finite().then_some(local.to_array())
}

/// A model-space direction turned into world space (rotation only). A zero result leaves the input as it was.
pub(super) fn entity_normal_to_world(world_from_model: glam::Mat4, normal: [f32; 3]) -> [f32; 3] {
    let (_, rotation, _) = world_from_model.to_scale_rotation_translation();
    let world = rotation * glam::Vec3::from_array(normal);
    let len = world.length();
    if len <= 0.0 {
        return normal;
    }
    (world / len).to_array()
}
