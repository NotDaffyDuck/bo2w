//! The pieces the netcode plugin adds to an app, one function per area.

use bevy::prelude::{App, Fixed, IntoScheduleConfigs, Time, Update};
use frame::{ProcessDuty, MatchSwapDone};

use crate::host_tick::peer_mail::{HOST_TICK_FREQUENCY, HostTimeKeeper, PlayerActionIntake, PlayerMoveIntake};
use crate::host_tick::tick_systems::{HostInputValve, HostLoadingPause};
use crate::client::drive::enroll_receiver_systems;
use crate::client::local_input::{PlayerControlSnapshot, ViewAimAccumulator};
use crate::client::presentation::entity_slots::enroll_receiver_objects;
use crate::client::presentation::event_raise::enroll_object_occurrence_routing;
use crate::client::presentation::shown_frame::{HostViewClient, ShownWorldFrame, ViewweaponAim};
use crate::net_links::listen_link::LocalHostTickPipe;
use crate::{
    ActionRequestCounter, PlayerActionBook, ClientStep, DirectoryMatchBegin, GuaranteedEventRouter, tick_sets, join_walk,
    net_links,
};

/// What every role has: the role itself and the resources the presentation reads.
pub(super) fn shared(app: &mut App, role: ProcessDuty) {
    app.insert_resource(role)
        .init_resource::<ShownWorldFrame>()
        .init_resource::<HostViewClient>()
        .init_resource::<DirectoryMatchBegin>()
        .init_resource::<ViewweaponAim>();
}

/// The authority's system sets, inboxes and fixed clock. A role without a host carries them too and never feeds them.
pub(super) fn authority_state(app: &mut App) {
    tick_sets::order_server_side_stages(app);
    app.init_resource::<HostTimeKeeper>()
        .init_resource::<PlayerMoveIntake>()
        .init_resource::<PlayerActionIntake>()
        .init_resource::<PlayerActionBook>()
        .init_resource::<ActionRequestCounter>()
        .init_resource::<GuaranteedEventRouter>()
        .insert_resource(Time::<Fixed>::from_hz(HOST_TICK_FREQUENCY));
}

/// What only a role with a client needs: input state, the loopback to a local host, and the client's systems.
pub(super) fn client_side(app: &mut App, role: ProcessDuty) {
    tick_sets::order_player_side_stages(app);
    app.init_resource::<ViewAimAccumulator>()
        .init_resource::<PlayerControlSnapshot>()
        .init_resource::<LocalHostTickPipe>()
        .init_resource::<HostInputValve>();

    if matches!(role, ProcessDuty::Client) {
        app.init_resource::<HostLoadingPause>().init_resource::<HostTimeKeeper>();
    }
    if !role.hosts_simulation() {
        app.init_resource::<PlayerMoveIntake>().init_resource::<PlayerActionIntake>();
    }

    enroll_receiver_systems(app);
    enroll_receiver_objects(app);
    enroll_object_occurrence_routing(app);
}

/// Through a master server when one is configured; otherwise a driver that walks the match boundaries itself.
pub(super) fn match_flow(app: &mut App, via_master: bool) {
    if via_master {
        net_links::lobby_link::register(app);
    } else {
        app.add_systems(Update, join_walk::step_match_edge_phase.in_set(ClientStep::Load).after(MatchSwapDone));
    }
}
