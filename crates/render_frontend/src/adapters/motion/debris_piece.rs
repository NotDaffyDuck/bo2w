//! The loose map pieces' front-end wiring: the shared piece types from `render_anim`, and
//! the systems that register for them.

use bevy::prelude::*;
use render_frontend::{TASK_ORDER_ROOM_DEBRIS_MODEL, TaskOrderBusyInput};

use crate::prepare::scene::prop_geom_memo::FrontendTaskOrders;

pub use render_anim::occupancy::shard_piece::{
    DebrisSolidKeyLightVis, DebrisRoomBits, PropMainLightVisibility, PieceModels,
    reserve_main_light_bits, fpv_cull_planes, relink_debris_key_lights, sphere_outside_frustum,
    model_collision_corners, model_bound_radius,
};
pub use render_anim::{PropPhysicsShape, DebrisPhysLevel};

/// Cell dyn-model work handed to the worker queue is finished by the time the frame's
/// animation set runs; take the queue's results so it does not back up.
fn drain_room_debris_model_orders(mut task_orders: ResMut<FrontendTaskOrders>) {
    let _ = task_orders.queues.wait_for_type(
        TASK_ORDER_ROOM_DEBRIS_MODEL,
        TaskOrderBusyInput::default(),
        |_| {},
    );
}

pub fn register_debris_frontend(app: &mut App) {
    super::free_body_wake::register_debris_wake(app);
    super::debris::register_debris(app);
    app.add_systems(
        Update,
        drain_room_debris_model_orders
            .in_set(frame::DrawPassStage::Anim)
            .in_set(frame::JobQueueStage::CellDynModel),
    );
}
