//! Wires the client's per-frame systems into the schedule.

use bevy::prelude::*;

use super::backlog::cap_receiver_workload;
use super::frame_time::{step_presentation_clock, advance_receiver_wall_clock};
use super::gun_pick::WeaponSelect;
use super::intake::{ingest_host_steps, correct_forecast};
use super::local_step::forecast_own_move;
use super::location_cursor::LocationCursor;
use super::outbox::QueuedOutgoingPlayerMessages;
use super::present::expose_shown_frame;
use super::reliable::apply_weapon_switch_requests;
use super::sample::poll_receiver_controls;
use super::stores::{
    PlayerMoveBlueprint, ClientGuessState, PlayerGuaranteedReceipt, NewestTakenSnapshot,
    PendingDebris, QueuedPelletEffect, StagedShownEntityEvents, ArrivedStepQueue, OtherPlayerSmootherHolder,
    GuaranteedSimEvent,
};
use super::match_forget::wipe_presentation_after_match_end;
use super::ticker::PlayerTimeStepper;
use super::uplink::{UplinkPacer, settle_setup_applied, transmit_queued_inputs};
use crate::host_tick::tick_systems::PlayerShotOrigins;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::presentation::shown_frame::OwnViewerTally;
use crate::client::realtime::ClientRealtime;
use crate::tick_sets::ClientStep;
use crate::join_walk::{process_entry_ticket_events, step_map_ready_phase, step_join_handshake};

/// Inserts an empty (default) copy of each listed resource, in list order.
macro_rules! empty_stores {
    ($app:ident: $($store:ty),+ $(,)?) => {
        $( $app.init_resource::<$store>(); )+
    };
}

/// Registers each listed message type.
macro_rules! frame_messages {
    ($app:ident: $($msg:ty),+ $(,)?) => {
        $( $app.add_message::<$msg>(); )+
    };
}

/// Everything the client keeps between frames, plus the messages it trades within one.
fn add_stores_and_messages(app: &mut App) {
    empty_stores!(app:
        crate::JoinProgressRecord,
        crate::PlayerEntryTicket,
        crate::Scoreboard,
        ClientGuessState,
        PlayerTimeStepper,
        FrameClock,
        GameActive,
        GameJoinCensus,
        ClientRealtime,
        ArrivedStepQueue,
        NewestTakenSnapshot,
        OwnViewerTally,
        StagedShownEntityEvents,
        QueuedPelletEffect,
        PendingDebris,
        QueuedOutgoingPlayerMessages,
        PlayerMoveBlueprint,
        WeaponSelect,
        LocationCursor,
        PlayerGuaranteedReceipt,
        UplinkPacer,
        OtherPlayerSmootherHolder,
        PlayerShotOrigins,
    );
    frame_messages!(app:
        GuaranteedSimEvent,
        frame::GameSessionArmed,
        frame::GameSessionClosed,
        crate::HostLocalSoundMsg,
        crate::CardSlotCommand,
        crate::MenuOpenCommand,
        crate::HostScreenFlash,
        crate::HostGameAnnouncement,
    );
}

/// Load and receive: clocks, teardown, snapshot intake and the signon drivers fed by it.
fn add_receive_side(app: &mut App) {
    let load = (
        advance_receiver_wall_clock,
        wipe_presentation_after_match_end.after(frame::MatchSwapDone),
    )
        .in_set(ClientStep::Load);
    let receive = (
        ingest_host_steps,
        process_entry_ticket_events.after(ingest_host_steps),
        (step_join_handshake, step_map_ready_phase).after(process_entry_ticket_events),
    )
        .in_set(ClientStep::Receive);
    let reconcile = (
        correct_forecast,
        (step_presentation_clock, settle_setup_applied).after(correct_forecast),
    )
        .in_set(ClientStep::Reconcile);
    app.add_systems(Update, (load, receive, reconcile));
}

/// Input through presentation: weapon switches, sampling, forecast, sending and publishing.
fn add_send_side(app: &mut App) {
    let input = (apply_weapon_switch_requests.before(poll_receiver_controls), poll_receiver_controls)
        .in_set(ClientStep::Input);
    app.add_systems(
        Update,
        (
            input,
            forecast_own_move.in_set(ClientStep::Predict),
            transmit_queued_inputs.in_set(ClientStep::Send),
            expose_shown_frame.in_set(ClientStep::Present),
        ),
    );
    // The work-limit guard sits between reconciliation and the next local prediction.
    app.add_systems(Update, cap_receiver_workload.after(correct_forecast).before(forecast_own_move));
    // Anything waiting on "presented" or "class equipped" runs once the presented view is out.
    app.configure_sets(
        Update,
        (crate::ShownStatePosted, frame::KitEquipDone)
            .in_set(ClientStep::Present)
            .after(expose_shown_frame),
    );
}

pub fn enroll_receiver_systems(app: &mut App) {
    add_stores_and_messages(app);
    crate::client::frame_tally::enroll_update_stage_tally(app);
    add_receive_side(app);
    add_send_side(app);
}
