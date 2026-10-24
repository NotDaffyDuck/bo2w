//! What the client does each frame: take in what the host sent, work out where the local
//! player is, send the next commands, and publish what to draw.
//!
//! The stages run in this order inside `Update` (see [`register`] for the exact wiring):
//! load → receive → reconcile → input → predict → send → present.

mod backlog;
mod frame_time;
mod gun_pick;
mod intake;
mod killcam_log;
mod listen_arm;
mod local_step;
mod location_cursor;
mod look_input;
mod melee_pick;
mod outbox;
mod pad_assist;
mod present;
mod register;
mod reliable;
mod sample;
mod stores;
mod teardown;
mod ticker;
mod uplink;
mod weapon_keys;

pub use frame_time::{advance_cg_frame_clock, advance_cls_realtime};
pub use gun_pick::{WeaponSelect, cycle_weapon_select, follow_held_weapon_select};
pub use intake::{receive_ticks, reconcile_prediction};
pub use listen_arm::{
    arm_listen_prediction, listen_prediction_needs_content, register_listen_prediction_arm,
};
pub use local_step::predict_local_move;
pub use location_cursor::LocationCursor;
pub use outbox::PendingClientSends;
pub use present::publish_presented;
pub use register::register_client_runtime;
pub use sample::sample_client_input;
pub use stores::{
    ClientCmdTemplate, ClientPhaseTrace, ClientPredictionState, ClientReliableAck,
    LastAdoptedSnapshot, PendingDebris, PendingPelletFx, PendingPresentedEntityEvents,
    ReceivedTicks, ReliableControlEvent, RemoteProxyState,
};
pub use ticker::{ClientClock, ClockTick};
pub use uplink::send_pending_commands;
