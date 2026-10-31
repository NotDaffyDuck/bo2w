//! Wiring: the resources and fixed-update phases of the authority loop.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use frame::{ProcessDuty, hook_script_signal};

use super::census::{
    HostStageTally, StepUpdateTally, LocalHostFanoutTally, begin_fixed_census,
    finish_steady_tally, install_phase_seams, post_step_census, publish_phase_census,
};
use super::dumps::{DumpConfigurationChangeLog, DeathLogDumper, GrantLogDumper};
use super::fanout::{host_stage_upkeep, fanout};
use super::intake::{advance_clock, gather_input, ingress};
use super::run_gate::{
    inject_link_impairments, host_due_for_step, reset_on_match_torn_down,
    discard_gone_peers,
};
use super::loop_resources::{
    HostInputValve, HostLoadingPause, HostStageTrail, HostSimulationWorld, PlayerShotOrigins,
    PreviousHostRoster, LinkDiagnosticsReport, StagedReceipts, StagedHostTickInput,
    StagedLinkFailures, StagedStepOutput, HostStepNumber,
};
use super::step::{publish_tick, step_world};
use crate::identity_slips::LinkIdentityDeficits;
use crate::viewer_policy::cam_view::LiveDeathReplays;
use crate::tick_sets::{ServerStage, ClientStep};
use crate::net_links::snapshot_history::SnapshotVault;
use crate::{StagedAnnouncements, StagedCardUpdate, StagedScoreboardSend, StagedHostSounds};

pub fn enroll_hosted_systems(app: &mut App) {
    hook_script_signal(app);
    let role = *app.world().resource::<ProcessDuty>();
    if role != ProcessDuty::Client {
        app.init_resource::<HostSimulationWorld>();
    }
    app.init_resource::<HostInputValve>()
        .init_resource::<HostLoadingPause>()
        .init_resource::<HostStageTrail>()
        .init_resource::<StagedHostTickInput>()
        .init_resource::<StagedReceipts>()
        .init_resource::<StagedStepOutput>()
        .init_resource::<HostStepNumber>()
        .init_resource::<SnapshotVault>()
        .init_resource::<LiveDeathReplays>()
        .init_resource::<LinkDiagnosticsReport>()
        .init_resource::<PlayerShotOrigins>()
        .init_resource::<LinkIdentityDeficits>()
        .init_resource::<StagedHostSounds>()
        .init_resource::<StagedCardUpdate>()
        .init_resource::<StagedAnnouncements>()
        .init_resource::<StagedScoreboardSend>()
        .init_resource::<StepUpdateTally>()
        .init_resource::<LocalHostFanoutTally>()
        .init_resource::<DeathLogDumper>()
        .init_resource::<GrantLogDumper>()
        .init_resource::<DumpConfigurationChangeLog>()
        .init_resource::<PreviousHostRoster>()
        .init_resource::<StagedLinkFailures>();

    // A dedicated server fans out only after its entity mirror has caught up with the world.
    let dedicated = role == ProcessDuty::Dedicated;
    let mut fanout_system = fanout.in_set(ServerStage::Fanout);
    if dedicated {
        fanout_system =
            fanout_system.after(crate::client::presentation::entity_slots::sync_authority_entities);
    }
    app.add_systems(
        FixedUpdate,
        (
            begin_fixed_census
                .before(ServerStage::Advance)
                .before(frame::ServerHandoffLink(0)),
            advance_clock.in_set(ServerStage::Advance),
            ingress.in_set(ServerStage::Ingress),
            gather_input.in_set(ServerStage::Gather),
            step_world.in_set(ServerStage::Step),
            publish_tick.in_set(ServerStage::Snapshot),
            fanout_system,
            inject_link_impairments
                .in_set(ServerStage::Bookkeeping)
                .before(discard_gone_peers),
            discard_gone_peers.in_set(ServerStage::Bookkeeping),
            host_stage_upkeep.in_set(frame::ServerLedgerMark),
            finish_steady_tally.after(ServerStage::Bookkeeping),
        )
            .run_if(host_due_for_step),
    );
    if dedicated {
        crate::client::presentation::entity_slots::enroll_host_objects(app);
    }

    app.init_resource::<HostStageTally>().add_systems(
        RunFixedMainLoop,
        (post_step_census, publish_phase_census)
            .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
    );
    app.add_systems(
        Update,
        reset_on_match_torn_down
            .in_set(ClientStep::Load)
            .after(frame::MatchSwapDone),
    );
    install_phase_seams(app);
}
