//! Wires the client's per-frame systems into the schedule.

use bevy::prelude::*;

use super::backlog::enforce_client_work_limits;
use super::frame_time::{advance_cg_frame_clock, advance_cls_realtime};
use super::gun_pick::WeaponSelect;
use super::intake::{receive_ticks, reconcile_prediction};
use super::local_step::predict_local_move;
use super::location_cursor::LocationCursor;
use super::outbox::PendingClientSends;
use super::present::publish_presented;
use super::reliable::apply_weapon_switch_requests;
use super::sample::sample_client_input;
use super::stores::{
    ClientCmdTemplate, ClientPredictionState, ClientReliableAck, LastAdoptedSnapshot,
    PendingDebris, PendingPelletFx, PendingPresentedEntityEvents, ReceivedTicks, RemoteProxyState,
    ReliableControlEvent,
};
use super::teardown::reset_cgame_on_match_torn_down;
use super::ticker::ClientClock;
use super::uplink::{UplinkPacer, flush_bootstrap_applied, send_pending_commands};
use crate::authority::runtime::ClientShotSamples;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::presentation::presented::PresentLocalCensus;
use crate::client::realtime::ClientRealtime;
use crate::schedule::ClientSet;

/// Everything the client keeps between frames, each store empty.
fn add_stores(app: &mut App) {
    app.init_resource::<crate::SignonState>()
        .init_resource::<crate::ClientAdmission>()
        .init_resource::<crate::Scoreboard>()
        .init_resource::<ClientPredictionState>()
        .init_resource::<ClientClock>()
        .init_resource::<FrameClock>()
        .init_resource::<GameActive>()
        .init_resource::<GameJoinCensus>()
        .init_resource::<ClientRealtime>()
        .init_resource::<ReceivedTicks>()
        .init_resource::<LastAdoptedSnapshot>()
        .init_resource::<PresentLocalCensus>()
        .init_resource::<PendingPresentedEntityEvents>()
        .init_resource::<PendingPelletFx>()
        .init_resource::<PendingDebris>()
        .init_resource::<PendingClientSends>()
        .init_resource::<ClientCmdTemplate>()
        .init_resource::<WeaponSelect>()
        .init_resource::<LocationCursor>()
        .init_resource::<ClientReliableAck>()
        .init_resource::<UplinkPacer>()
        .init_resource::<RemoteProxyState>()
        .init_resource::<ClientShotSamples>();
}

/// The messages the client reads and writes within a frame.
fn add_messages(app: &mut App) {
    app.add_message::<ReliableControlEvent>()
        .add_message::<frame::MatchInstalled>()
        .add_message::<frame::MatchTornDown>()
        .add_message::<crate::SvcLocalSound>()
        .add_message::<crate::SvcCardSlotCmd>()
        .add_message::<crate::SvcOpenMenuCmd>()
        .add_message::<crate::SvcHudSplash>()
        .add_message::<crate::SvcGameNotify>();
}

pub fn register_client_runtime(app: &mut App) {
    add_stores(app);
    add_messages(app);
    crate::client::frame_census::register_update_phase_census(app);

    app.add_systems(
        Update,
        enforce_client_work_limits
            .before(predict_local_move)
            .after(reconcile_prediction),
    );
    app.configure_sets(
        Update,
        (
            crate::PresentedPublished
                .in_set(ClientSet::Present)
                .after(publish_presented),
            frame::ClassEquipResolved
                .in_set(ClientSet::Present)
                .after(publish_presented),
        ),
    );
    app.add_systems(
        Update,
        (
            advance_cls_realtime.in_set(ClientSet::Load),
            reset_cgame_on_match_torn_down
                .in_set(ClientSet::Load)
                .after(frame::SessionSwapApplied),
            receive_ticks.in_set(ClientSet::Receive),
            crate::signon::drive_client_admission_facts
                .in_set(ClientSet::Receive)
                .after(receive_ticks),
            crate::signon::drive_signon
                .in_set(ClientSet::Receive)
                .after(crate::signon::drive_client_admission_facts),
            crate::signon::drive_map_loaded
                .in_set(ClientSet::Receive)
                .after(crate::signon::drive_client_admission_facts),
            reconcile_prediction.in_set(ClientSet::Reconcile),
            advance_cg_frame_clock
                .in_set(ClientSet::Reconcile)
                .after(reconcile_prediction),
            flush_bootstrap_applied
                .in_set(ClientSet::Reconcile)
                .after(reconcile_prediction),
        ),
    );
    app.add_systems(
        Update,
        (
            apply_weapon_switch_requests
                .in_set(ClientSet::Input)
                .before(sample_client_input),
            sample_client_input.in_set(ClientSet::Input),
            predict_local_move.in_set(ClientSet::Predict),
            send_pending_commands.in_set(ClientSet::Send),
            publish_presented.in_set(ClientSet::Present),
        ),
    );
}
