//! Turning a sorted draw into an executable material: look the material and
//! technique up, check the pass state, and bind code constants and samplers.

mod build;
mod lanes;
mod refusal;
mod resolve;
mod shell;

pub use build::run_material_pipeline;
pub use lanes::{
    CodeConstantChannel, CodeConstantBundle, CodeSamplerChannel, CompactEngineSamplers,
};
pub use refusal::{MaterialExecutionDenial, UnhandledStateFields};
pub use resolve::{
    SurfaceSortKeyPair, surface_add_gains_technique, draw_uses_engine_texture, draw_engine_sampler_bits,
    resolved_draw_technique, select_material_technique, lookup_ordered_material,
};
pub use resolve::smodel_tess_vertex_type;
pub use resolve::tess_vertex_kind;
pub use resolve::level_stitch_vertex_type_designer;
pub use shell::{
    BoundPass, CodeLanes, MaterialRunPlan, MaterialPlan, PassPlan, capture_plan, execute_plan,
};
