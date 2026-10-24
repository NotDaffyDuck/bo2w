//! Wiring: the resources and fixed-update phases of the authority loop.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use frame::{RuntimeRole, register_script_notify};

use super::census::{
    AuthorityPhaseCensus, FixedUpdateCensus, ListenFanoutCensus, begin_fixed_census,
    end_fixed_census, install_phase_seams, publish_fixed_census, publish_phase_census,
};
use super::dumps::{DumpConfigurationChangeLog, DumpDeathLog, DumpGiveLog};
use super::fanout::{authority_bookkeeping, fanout};
use super::intake::{advance_clock, gather_input, ingress};
use super::lifecycle::{
    apply_connection_faults, authority_should_tick, reset_on_match_torn_down,
    retire_departed_peers,
};
use super::state::{
    AuthorityInputGate, AuthorityLoadHold, AuthorityPhaseTrace, AuthorityWorld, ClientShotSamples,
    LastAuthorityRoster, NetDiagnostics, PendingAcks, PendingAuthorityInput,
    PendingConnectionFaults, PendingStepResult, ServerTick,
};
use super::step::{publish_tick, step_world};
use crate::gaps::NetIdentityGaps;
use crate::policy::seat::ActiveKillcams;
use crate::schedule::{AuthoritySet, ClientSet};
use crate::transport::archive::FrameArchive;
use crate::{PendingGameNotify, PendingPlayerCard, PendingScoreboard, PendingSvcSounds};

pub fn register_listen_runtime(app: &mut App) {
    register_script_notify(app);
    let role = *app.world().resource::<RuntimeRole>();
    if role != RuntimeRole::Client {
        app.init_resource::<AuthorityWorld>();
    }
    app.init_resource::<AuthorityInputGate>()
        .init_resource::<AuthorityLoadHold>()
        .init_resource::<AuthorityPhaseTrace>()
        .init_resource::<PendingAuthorityInput>()
        .init_resource::<PendingAcks>()
        .init_resource::<PendingStepResult>()
        .init_resource::<ServerTick>()
        .init_resource::<FrameArchive>()
        .init_resource::<ActiveKillcams>()
        .init_resource::<NetDiagnostics>()
        .init_resource::<ClientShotSamples>()
        .init_resource::<NetIdentityGaps>()
        .init_resource::<PendingSvcSounds>()
        .init_resource::<PendingPlayerCard>()
        .init_resource::<PendingGameNotify>()
        .init_resource::<PendingScoreboard>()
        .init_resource::<FixedUpdateCensus>()
        .init_resource::<ListenFanoutCensus>()
        .init_resource::<DumpDeathLog>()
        .init_resource::<DumpGiveLog>()
        .init_resource::<DumpConfigurationChangeLog>()
        .init_resource::<LastAuthorityRoster>()
        .init_resource::<PendingConnectionFaults>();

    // A dedicated server fans out only after its entity mirror has caught up with the world.
    let dedicated = role == RuntimeRole::Dedicated;
    let mut fanout_system = fanout.in_set(AuthoritySet::Fanout);
    if dedicated {
        fanout_system =
            fanout_system.after(crate::client::presentation::entities::sync_authority_entities);
    }
    app.add_systems(
        FixedUpdate,
        (
            begin_fixed_census
                .before(AuthoritySet::Advance)
                .before(frame::AuthorityEdge(0)),
            advance_clock.in_set(AuthoritySet::Advance),
            ingress.in_set(AuthoritySet::Ingress),
            gather_input.in_set(AuthoritySet::Gather),
            step_world.in_set(AuthoritySet::Step),
            publish_tick.in_set(AuthoritySet::Snapshot),
            fanout_system,
            apply_connection_faults
                .in_set(AuthoritySet::Bookkeeping)
                .before(retire_departed_peers),
            retire_departed_peers.in_set(AuthoritySet::Bookkeeping),
            authority_bookkeeping.in_set(frame::AuthorityBookkeeping),
            end_fixed_census.after(AuthoritySet::Bookkeeping),
        )
            .run_if(authority_should_tick),
    );
    if dedicated {
        crate::client::presentation::entities::register_authority_entities(app);
    }

    app.init_resource::<AuthorityPhaseCensus>().add_systems(
        RunFixedMainLoop,
        (publish_fixed_census, publish_phase_census)
            .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
    );
    app.add_systems(
        Update,
        reset_on_match_torn_down
            .in_set(ClientSet::Load)
            .after(frame::SessionSwapApplied),
    );
    install_phase_seams(app);
}
