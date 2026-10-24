//! A model's skeleton and the meshes bound to it.

use asset_t6::T6_PACKED_VERTEX;

use crate::{
    dobj::ModelPoseSrc,
    skel_lod::lod_surfaces,
    skel_parts::{BoneBind, BoneCollision, VertSkin},
};

/// Everything the game keeps of one model: its bones, the vertices of all its
/// surfaces laid end to end, and how those surfaces split up by detail level,
/// material and part.
#[derive(Clone, Debug, Default)]
pub struct ModelSkel {
    pub name: String,

    /// Bind pose, one entry per bone.
    pub bones: Vec<BoneBind>,
    /// The collision shape each bone carries, if any.
    pub bone_collision: Vec<Option<BoneCollision>>,
    pub bone_names: Vec<String>,
    /// Bone index of `tag_view`, when the model has one.
    pub tag_view: Option<usize>,
    /// Bone index of `tag_weapon`, when the model has one.
    pub tag_weapon: Option<usize>,
    /// The pose source animation reads; models with no skeleton have none.
    pub pose: Option<ModelPoseSrc>,

    // Vertex streams of every surface, one after the other.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// The vertices as the model stores them, for drawing without a repack.
    pub packed_vertices: Vec<[u8; T6_PACKED_VERTEX]>,
    pub vert_skin: Vec<VertSkin>,

    // One entry per surface from here on.
    /// The surface's material, as a row of the model's own material list.
    pub surface_materials: Vec<Option<crate::WalkLocalMaterialIndex>>,
    /// (first vertex, vertex count) of the surface in the vertex streams.
    pub surface_vertex_ranges: Vec<(usize, usize)>,
    /// (first index, index count) of the surface in `indices`.
    pub surface_index_ranges: Vec<(usize, usize)>,
    /// The body parts the surface belongs to, one bit each; hiding a part
    /// hides the surfaces that carry its bit.
    pub surface_part_bits: Vec<[u32; 5]>,
    /// Whether the surface is flagged to be deformed when drawn, when known.
    pub surface_deformed: Vec<Option<bool>>,
    /// How many vertex lists the surface is split into, when known.
    pub surface_vert_list_count: Vec<Option<u32>>,

    pub radius: Option<f32>,
    /// (mid point, half size) of the model's box.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub contents: Option<u32>,
    pub coll_lod: i16,
    pub coll_surfs: Vec<xmodel_runtime::CollSurfCollision>,
    /// The model's physics collision map.
    pub collmap: Option<xmodel_runtime::ModelCollmap>,

    /// How the detail level is picked by distance.
    pub lod: Option<crate::ModelLodSelector>,
    /// The static model's material-count per detail level, when it has one.
    pub lod_smc: Option<[[u8; 4]; 4]>,
    /// (first surface, surface count) of each detail level.
    pub lod_surf_span: [(u16, u16); 4],
}

impl ModelSkel {
    /// What a game-side model needs of this skeleton to pose and trace it, or
    /// nothing when the model has no pose source.
    pub fn retained_capability(&self) -> Option<xmodel_runtime::RetainedModelCapability> {
        let pose = self.pose.clone()?;
        Some(xmodel_runtime::RetainedModelCapability {
            key: self.name.clone(),
            pose,
            bone_collision: self.bone_collision.clone(),
            contents: self.contents,
            coll_lod: self.coll_lod,
            coll_surfs: self.coll_surfs.clone(),
            collmap: self.collmap.clone(),
            bounds: self.bounds,
            radius: self.radius,
        })
    }

    /// The bone `bone` hangs from. Root bones, bones past the parent list and
    /// bones that point at themselves have none.
    pub fn parent_of(&self, bone: usize) -> Option<usize> {
        let pose = self.pose.as_ref()?;
        let local = bone.checked_sub(pose.num_root_bones)?;
        let step = *pose.parent_list.get(local)? as usize;
        if step == 0 {
            return None;
        }
        bone.checked_sub(step)
    }

    pub fn tag_camera(&self) -> Option<usize> {
        self.bone_names
            .iter()
            .position(|name| name.eq_ignore_ascii_case("tag_camera"))
    }

    /// The surfaces the detail level `lod` draws.
    pub fn surfaces_for_lod(&self, lod: u8) -> core::ops::Range<usize> {
        lod_surfaces(self.lod_surf_span, self.surface_vertex_ranges.len(), lod)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(bones: &[&str]) -> ModelSkel {
        ModelSkel {
            bone_names: bones.iter().map(|name| (*name).to_owned()).collect(),
            ..ModelSkel::default()
        }
    }

    #[test]
    fn the_camera_tag_is_found_whatever_its_case() {
        assert_eq!(named(&["j_gun", "TAG_Camera"]).tag_camera(), Some(1));
        assert_eq!(named(&["j_gun"]).tag_camera(), None);
    }

    #[test]
    fn a_model_without_a_pose_has_no_parents_and_no_capability() {
        let skel = named(&["a", "b"]);
        assert_eq!(skel.parent_of(1), None);
        assert!(skel.retained_capability().is_none());
    }

    #[test]
    fn surfaces_follow_the_recorded_spans() {
        let mut skel = ModelSkel::default();
        skel.surface_vertex_ranges = vec![(0, 3); 4];
        skel.lod_surf_span = [(0, 3), (3, 1), (0, 0), (0, 0)];
        assert_eq!(skel.surfaces_for_lod(0), 0..3);
        assert_eq!(skel.surfaces_for_lod(1), 3..4);
    }
}
