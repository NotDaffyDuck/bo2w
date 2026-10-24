//! The authority loop: advance the clock, take in the network, gather input, step the simulation,
//! publish the tick, fan it out to every player and keep the books.

mod census;
mod dumps;
mod fanout;
mod intake;
mod lifecycle;
mod plugin;
mod state;
mod step;

pub use census::{AuthorityPhaseCensus, FixedUpdateCensus, ListenFanoutCensus};
pub use dumps::{DumpConfigurationChangeLog, DumpDeathLog, DumpDeathRow, DumpGiveLog};
pub use fanout::authority_bookkeeping;
pub use lifecycle::{ActionTransactionScope, authority_should_tick};
pub use plugin::register_listen_runtime;
pub use state::{
    AuthorityInputGate, AuthorityLoadHold, AuthorityPhaseTrace, AuthorityWorld, ClientShotSamples,
    LastAuthorityRoster, NetDiagnostics, PendingAcks, PendingAuthorityInput,
    PendingConnectionFaults, PendingStepResult, ServerTick, ServerTickData,
};
