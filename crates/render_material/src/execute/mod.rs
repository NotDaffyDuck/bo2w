//! Turning a sorted draw into an executable material: look the material and
//! technique up, check the pass state, and bind code constants and samplers.

mod build;
mod lanes;
mod refusal;
mod resolve;
mod shell;

pub use build::{execute_material, execute_material_with_shell};
pub use lanes::{
    PackedCodeConstantLane, PackedCodeConstants, PackedCodeSamplerLane, PackedCodeSamplers,
};
pub use refusal::{MaterialRefusal, UnsupportedStateFields};
pub use resolve::{
    MaterialDrawKey, add_surf_has_technique, draw_binds_code_texture, draw_code_sampler_mask,
    prepared_draw_technique, resolve_material_technique, resolve_sorted_material,
    smodel_tess_vertex_type, world_tess_vertex_type, world_tess_vertex_type_authored,
};
pub use shell::{
    ExecutablePass, ExecutablePassView, MaterialExecution, StableMaterialShell, StablePassShell,
    capture_stable_shell, rebind_stable_material,
};
