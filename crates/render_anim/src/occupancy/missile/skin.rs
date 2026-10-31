use super::*;

impl PosedVerts {
    /// A pose the graphics card skins: `count` bones, each read through
    /// `bone`.
    pub fn from_bones(count: usize, bone: impl Fn(usize) -> Mat4) -> Self {
        Self {
            bones: (0..count.max(1)).map(bone).collect(),
            ..Default::default()
        }
    }

    /// True when only bones are held: the vertices are skinned on the
    /// graphics card.
    pub fn on_gpu(&self) -> bool {
        self.positions.is_empty() && !self.bones.is_empty()
    }

    /// This pose's vertices on the CPU; a bones-only pose is skinned here
    /// (for a model held still).
    pub fn on_cpu(&self, bone_rig: &asset_model::MeshRig) -> Cow<'_, PosedVerts> {
        if !self.on_gpu() {
            return Cow::Borrowed(self);
        }
        let bone = |b: usize| self.bones.get(b).copied().unwrap_or(Mat4::IDENTITY);
        Cow::Owned(skin_verts(bone_rig, bone))
    }
}

/// CPU skinning: each vertex moved by its weighted bones (a model without
/// per-vertex weights by its first bone).
pub fn skin_verts(bone_rig: &asset_model::MeshRig, bone: impl Fn(usize) -> Mat4) -> PosedVerts {
    let count = bone_rig.positions.len();
    let weighted = bone_rig.vert_skin.len() == count;
    let first_bone = bone(0);
    let mut out = PosedVerts {
        positions: Vec::with_capacity(count),
        surface_directions: Vec::with_capacity(count),
        bones: Vec::new(),
    };
    for v in 0..count {
        let point = Vec3::from_array(bone_rig.positions[v]);
        let normal = Vec3::from_array(bone_rig.surface_directions.get(v).copied().unwrap_or([0.0, 0.0, 1.0]));
        let mut moved = Vec3::ZERO;
        let mut turned = Vec3::ZERO;
        let mut weight_sum = 0.0;
        if weighted {
            let skin = &bone_rig.vert_skin[v];
            for (&wt, &index) in skin.weights.iter().zip(&skin.joints).take(4) {
                if wt <= 0.0 {
                    continue;
                }
                let m = bone(usize::from(index));
                moved += m.transform_point3(point) * wt;
                turned += m.transform_vector3(normal) * wt;
                weight_sum += wt;
            }
        }
        if weight_sum <= 0.0 {
            moved = first_bone.transform_point3(point);
            turned = first_bone.transform_vector3(normal);
        }
        out.positions.push(moved.to_array());
        out.surface_directions.push(turned.normalize_or_zero().to_array());
    }
    out
}

/// A model's vertices skinned by the joint_transforms of its own bones, which start
/// at `base` in the composed skeleton.
pub(super) fn skin_model(
    bone_rig: &asset_model::MeshRig,
    skin_in: &[Mat4],
    base_in: usize,
) -> Arc<PosedVerts> {
    Arc::new(skin_verts(bone_rig, |b| {
        skin_in.get(base_in + b).copied().unwrap_or(Mat4::IDENTITY)
    }))
}
