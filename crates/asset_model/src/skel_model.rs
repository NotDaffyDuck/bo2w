//! A model's skeleton and the meshes bound to it.

use asset_t6::T6_PACKED_VERTEX;

use crate::skel_lod::detail_facets;
use crate::skel_parts::BoneHitShape;
use crate::skel_parts::CornerWeights;
use crate::skel_parts::JointAttachment;
use xmodel_runtime::PoseDonorModel;

/// Everything the game keeps of one model: its bones, the vertices of all its
/// surfaces laid end to end, and how those surfaces split up by detail level,
/// material and part.
#[derive(Clone, Debug, Default)]
pub struct MeshRig {
    pub name: String,

    /// Bind pose, one entry per bone.
    pub bones: Vec<JointAttachment>,
    /// The collision shape each bone carries, if any.
    pub joint_hit_volume: Vec<Option<BoneHitShape>>,
    pub joint_labels: Vec<String>,
    /// Bone index of `tag_view`, when the model has one.
    pub eye_joint: Option<usize>,
    /// Bone index of `tag_weapon`, when the model has one.
    pub gun_joint: Option<usize>,
    /// The pose source animation reads; models with no skeleton have none.
    pub pose: Option<PoseDonorModel>,

    // Vertex streams of every surface, one after the other.
    pub positions: Vec<[f32; 3]>,
    pub surface_directions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub texcoord_set: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// The vertices as the model stores them, for drawing without a repack.
    pub compact_vertices: Vec<[u8; T6_PACKED_VERTEX]>,
    pub vert_skin: Vec<CornerWeights>,

    // One entry per surface from here on.
    /// The surface's material, as a row of the model's own material list.
    pub facet_materials: Vec<Option<asset_core::TraversalMaterialSlot>>,
    /// (first vertex, vertex count) of the surface in the vertex streams.
    pub facet_vertex_ranges: Vec<(usize, usize)>,
    /// (first index, index count) of the surface in `indices`.
    pub facet_index_ranges: Vec<(usize, usize)>,
    /// The body parts the surface belongs to, one bit each; hiding a part
    /// hides the surfaces that carry its bit.
    pub face_parts: Vec<[u32; 5]>,
    /// Whether the surface is flagged to be deformed when drawn, when known.
    pub bent: Vec<Option<bool>>,
    /// How many vertex lists the surface is split into, when known.
    pub vert_lists: Vec<Option<u32>>,

    pub radius: Option<f32>,
    /// (mid point, half size) of the model's box.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    /// T6: where the model's light is read, from its origin in its own axes
    /// (`lightingOriginOffset`).
    pub light_offset: Option<[f32; 3]>,
    pub contents: Option<u32>,
    pub collision_detail_tier: i16,
    pub collision_faces: Vec<xmodel_runtime::SurfaceHitMesh>,
    /// What a hit on a bone box shows when the model has no collision surface
    /// there (see `KeptModelAbility`).
    pub face_bits: u32,
    /// The model's physics collision map.
    pub collmap: Option<xmodel_runtime::ModelCollmap>,

    /// How the detail level is picked by distance.
    pub lod: Option<crate::ModelDetailSelector>,
    /// The static model's material-count per detail level, when it has one.
    pub detail_prop_cache: Option<[[u8; 4]; 4]>,
    /// (first surface, surface count) of each detail level.
    pub detail_face_extent: [(u16, u16); 4],
}

impl MeshRig {
    /// What a game-side model needs of this skeleton to pose and trace it, or
    /// nothing when the model has no pose source.
    pub fn retained_capability(&self) -> Option<xmodel_runtime::KeptModelAbility> {
        let Some(src) = &self.pose else {
            return None;
        };
        let (lod_of_coll, surfs) = (self.collision_detail_tier, self.collision_faces.to_vec());
        let (radius, contents) = (self.radius, self.contents);
        let capability = xmodel_runtime::KeptModelAbility {
            pose: src.clone(),
            key: String::from(&self.name),
            radius,
            bounds: self.bounds,
            collmap: self.collmap.clone(),
            collision_faces: surfs,
            face_flags: self.face_bits,
            collision_detail_tier: lod_of_coll,
            contents,
            joint_hit_volume: Vec::clone(&self.joint_hit_volume),
        };
        Some(capability)
    }

    /// The bone `bone` hangs from. Root bones, bones past the parent list and
    /// bones that point at themselves have none.
    pub fn hanging_from(&self, joint: usize) -> Option<usize> {
        let Some(src) = self.pose.as_ref() else {
            return None;
        };
        let slot = joint.checked_sub(src.top_joint_total)?;
        let back = usize::from(*src.ancestor_chain.get(slot)?);
        match back {
            0 => None,
            back => joint.checked_sub(back),
        }
    }

    pub fn camera_joint(&self) -> Option<usize> {
        self.joint_labels.iter().position(|label| label.eq_ignore_ascii_case("tag_camera"))
    }

    /// The surfaces the detail level `tier` draws.
    pub fn facets_at_tier(&self, tier: u8) -> core::ops::Range<usize> {
        let surface_total = self.facet_vertex_ranges.len();
        detail_facets(self.detail_face_extent, surface_total, tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(labels: &[&str]) -> MeshRig {
        let mut bone_rig = MeshRig::default();
        bone_rig.joint_labels.extend(labels.iter().map(|s| s.to_string()));
        bone_rig
    }

    #[test]
    fn the_camera_tag_is_found_whatever_its_case() {
        let with_tag = named(&["j_gun", "TAG_Camera"]);
        let without = named(&["j_gun"]);
        assert_eq!(with_tag.camera_joint(), Some(1));
        assert_eq!(without.camera_joint(), None);
    }

    #[test]
    fn a_model_without_a_pose_has_no_parents_and_no_capability() {
        let bare = named(&["a", "b"]);
        assert_eq!(bare.hanging_from(1), None);
        assert!(bare.retained_capability().is_none());
    }

    #[test]
    fn surfaces_follow_the_recorded_spans() {
        let bone_rig = MeshRig {
            facet_vertex_ranges: vec![(0, 3); 4],
            detail_face_extent: [(0, 3), (3, 1), (0, 0), (0, 0)],
            ..MeshRig::default()
        };
        let drawn: Vec<_> = (0..2).map(|l| bone_rig.facets_at_tier(l)).collect();
        assert_eq!(drawn, [0..3, 3..4]);
    }
}
