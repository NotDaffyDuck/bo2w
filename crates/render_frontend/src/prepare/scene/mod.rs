//! What the frontend prepares from the loaded world each frame: the camera,
//! visibility, model lighting, and the world's GPU data.

pub mod camera;
pub mod cell_cone_jobs;
pub mod cull;
pub mod view_planes;
pub mod visual_stage;
pub mod model_illum_atlas;
pub mod model_illum_cache;
pub mod prop_geom_memo;
pub mod prop_light_slots;
pub mod spawn;
pub mod eye_parms;
pub mod world;
pub mod level_gpu;
pub mod level_pictures;
pub mod level_presence;
pub mod level_plan;
