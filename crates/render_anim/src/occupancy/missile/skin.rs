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
    pub fn on_cpu(&self, skel: &asset_model::ModelSkel) -> Cow<'_, PosedVerts> {
        if !self.on_gpu() {
            return Cow::Borrowed(self);
        }
        let bone = |b: usize| self.bones.get(b).copied().unwrap_or(Mat4::IDENTITY);
        Cow::Owned(skin_verts(skel, bone))
    }
}

/// CPU skinning: each vertex moved by its weighted bones (a model without
/// per-vertex weights by its first bone).
pub fn skin_verts(skel: &asset_model::ModelSkel, bone: impl Fn(usize) -> Mat4) -> PosedVerts {
    let count = skel.positions.len();
    let weighted = skel.vert_skin.len() == count;
    let first_bone = bone(0);
    let mut out = PosedVerts {
        positions: Vec::with_capacity(count),
        normals: Vec::with_capacity(count),
        bones: Vec::new(),
    };
    for v in 0..count {
        let point = Vec3::from_array(skel.positions[v]);
        let normal = Vec3::from_array(skel.normals.get(v).copied().unwrap_or([0.0, 0.0, 1.0]));
        let mut moved = Vec3::ZERO;
        let mut turned = Vec3::ZERO;
        let mut weight_sum = 0.0;
        if weighted {
            let skin = &skel.vert_skin[v];
            for (&weight, &index) in skin.weights.iter().zip(&skin.bones).take(4) {
                if weight <= 0.0 {
                    continue;
                }
                let m = bone(usize::from(index));
                moved += m.transform_point3(point) * weight;
                turned += m.transform_vector3(normal) * weight;
                weight_sum += weight;
            }
        }
        if weight_sum <= 0.0 {
            moved = first_bone.transform_point3(point);
            turned = first_bone.transform_vector3(normal);
        }
        out.positions.push(moved.to_array());
        out.normals.push(turned.normalize_or_zero().to_array());
    }
    out
}

/// A model's vertices skinned by the matrices of its own bones, which start
/// at `base` in the composed skeleton.
pub(super) fn skin_model(
    skel: &asset_model::ModelSkel,
    skin: &[Mat4],
    base: usize,
) -> Arc<PosedVerts> {
    Arc::new(skin_verts(skel, |b| {
        skin.get(base + b).copied().unwrap_or(Mat4::IDENTITY)
    }))
}
