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
mod match_forget;
mod ticker;
mod uplink;
mod weapon_keys;

pub use frame_time::{step_presentation_clock, advance_receiver_wall_clock};
pub use gun_pick::{WeaponSelect, cycle_weapon_select, follow_held_weapon_select};
pub use intake::{ingest_host_steps, correct_forecast};
pub use listen_arm::{
    enable_local_host_forecast, forecast_needs_rearm, enroll_hosted_forecast_hookup,
};
pub use local_step::forecast_own_move;
pub use location_cursor::LocationCursor;
pub use outbox::QueuedOutgoingPlayerMessages;
pub use present::expose_shown_frame;
pub use register::enroll_receiver_systems;
pub use sample::poll_receiver_controls;
pub use stores::{
    PlayerMoveBlueprint, ReceiverStageLog, ClientGuessState, PlayerGuaranteedReceipt,
    NewestTakenSnapshot, PendingDebris, QueuedPelletEffect, StagedShownEntityEvents,
    ArrivedStepQueue, GuaranteedSimEvent, OtherPlayerSmootherHolder,
};
pub use ticker::{PlayerTimeStepper, TimerStepResult};
pub use uplink::transmit_queued_inputs;
