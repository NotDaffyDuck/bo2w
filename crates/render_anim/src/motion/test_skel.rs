#![cfg(test)]
//! A skeleton with every stream empty, for tests that fill only what they read.

use asset_model::MeshRig;

pub(crate) fn blank_skel(name: &str) -> MeshRig {
    MeshRig {
        detail_face_extent: [(0, 0); 4],
        detail_prop_cache: None,
        lod: None,
        collmap: None,
        collision_faces: vec![],
        collision_detail_tier: 0,
        face_bits: 0,
        contents: None,
        bounds: None,
        light_offset: None,
        radius: None,
        compact_vertices: vec![],
        vert_skin: vec![],
        vert_lists: vec![],
        bent: vec![],
        face_parts: vec![],
        facet_index_ranges: vec![],
        facet_vertex_ranges: vec![],
        facet_materials: vec![],
        indices: vec![],
        texcoord_set: vec![],
        colors: vec![],
        surface_directions: vec![],
        positions: vec![],
        pose: None,
        gun_joint: None,
        eye_joint: None,
        joint_labels: vec![],
        joint_hit_volume: vec![],
        bones: vec![],
        name: String::from(name),
    }
}

/// As [`blank_skel`], with `bones` identity-bound bones.
pub(crate) fn skel_with_bones(name: &str, bones: usize) -> MeshRig {
    let identity = asset_model::JointAttachment {
        shift_offset: [0.0; 3],
        spin: [0.0, 0.0, 0.0, 1.0],
    };
    MeshRig {
        bones: std::iter::repeat_n(identity, bones).collect(),
        ..blank_skel(name)
    }
}
