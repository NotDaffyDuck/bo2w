//! The loose map pieces' front-end wiring: the shared piece types from `render_anim`, and
//! the systems that register for them.

use bevy::prelude::*;
use render_frontend::{WORKER_CMD_CELL_DYN_MODEL, WorkerCmdBusyInput};

use crate::prepare::scene::smodel_geom_cache::FrontendWorkerCmds;

pub use render_anim::occupancy::dyn_ent::{
    DynEntBrushPrimaryLightVis, DynEntCellBits, DynEntPrimaryLightVis, PieceModels,
    ensure_primary_light_words, fpv_frustum_planes, relink_dyn_ent_primary_lights,
    sphere_behind_frustum, xmodel_phys_hull, xmodel_radius,
};
pub use render_anim::{DynEntPhysClip, DynEntPhysWorld};

/// Cell dyn-model work handed to the worker queue is finished by the time the frame's
/// animation set runs; take the queue's results so it does not back up.
fn drain_cell_dyn_model_cmds(mut worker_cmds: ResMut<FrontendWorkerCmds>) {
    let _ = worker_cmds.queues.wait_of_type(
        WORKER_CMD_CELL_DYN_MODEL,
        WorkerCmdBusyInput::default(),
        |_| {},
    );
}

pub fn register_dyn_ent_frontend(app: &mut App) {
    super::dyn_ent_wake::register_dyn_ent_wake(app);
    super::debris::register_debris(app);
    app.add_systems(
        Update,
        drain_cell_dyn_model_cmds
            .in_set(frame::RenderSet::Anim)
            .in_set(frame::WorkerCmdSet::CellDynModel),
    );
}
