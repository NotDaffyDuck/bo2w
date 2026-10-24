//! A skeleton with every stream empty, for tests that fill only what they read.

use asset_model::ModelSkel;

pub(crate) fn blank_skel(name: &str) -> ModelSkel {
    ModelSkel {
        name: name.to_owned(),
        bones: Vec::new(),
        bone_collision: Vec::new(),
        bone_names: Vec::new(),
        tag_view: None,
        tag_weapon: None,
        pose: None,
        positions: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        surface_materials: Vec::new(),
        surface_vertex_ranges: Vec::new(),
        surface_index_ranges: Vec::new(),
        surface_part_bits: Vec::new(),
        surface_deformed: Vec::new(),
        surface_vert_list_count: Vec::new(),
        vert_skin: Vec::new(),
        packed_vertices: Vec::new(),
        radius: None,
        bounds: None,
        contents: None,
        coll_lod: 0,
        coll_surfs: Vec::new(),
        collmap: None,
        lod: None,
        lod_smc: None,
        lod_surf_span: [(0, 0); 4],
    }
}

/// As [`blank_skel`], with `bones` identity-bound bones.
pub(crate) fn skel_with_bones(name: &str, bones: usize) -> ModelSkel {
    let mut skel = blank_skel(name);
    skel.bones = vec![
        asset_model::BoneBind {
            quat: [0.0, 0.0, 0.0, 1.0],
            trans: [0.0; 3],
        };
        bones
    ];
    skel
}
