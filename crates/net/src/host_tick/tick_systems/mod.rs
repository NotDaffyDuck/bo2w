//! The authority loop: advance the clock, take in the network, gather input, step the simulation,
//! publish the tick, fan it out to every player and keep the books.

mod census;
mod dumps;
mod fanout;
mod intake;
mod run_gate;
mod plugin;
mod loop_resources;
mod step;

pub use census::{HostStageTally, StepUpdateTally, LocalHostFanoutTally};
pub use dumps::{DumpConfigurationChangeLog, DeathLogDumper, DeathDumpLine, GrantLogDumper};
pub use fanout::host_stage_upkeep;
pub use run_gate::{ActionBatchScope, host_due_for_step};
pub use plugin::enroll_hosted_systems;
pub use loop_resources::{
    HostInputValve, HostLoadingPause, HostStageTrail, HostSimulationWorld, PlayerShotOrigins,
    PreviousHostRoster, LinkDiagnosticsReport, StagedReceipts, StagedHostTickInput,
    StagedLinkFailures, StagedStepOutput, HostStepNumber, HostStepOutput,
};
