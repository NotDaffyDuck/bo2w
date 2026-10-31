//! Probe systems that note when their set runs, so a test can read back the order.

use bevy::prelude::*;
use frame::{GameSessionClosed, server_stage_name, client_stage_name};

use crate::tick_sets::{SERVER_STAGES, ServerStage, CLIENT_STAGES, ClientStep};

/// Labels in the order the probes ran.
#[derive(Resource, Default)]
pub(super) struct Trace(pub Vec<String>);

impl Trace {
    pub fn has_prefix(&self, head: &str) -> bool {
        self.0.iter().any(|label| label.starts_with(head))
    }

    pub fn position(&self, label: &str) -> Result<usize, String> {
        let found = self.0.iter().position(|seen| seen.as_str() == label);
        found.ok_or_else(|| format!("{label} did not run: {:?}", self.0))
    }
}

/// A system that adds `label` to the trace each time it runs.
pub(super) fn note(label: impl Into<String>) -> impl FnMut(ResMut<Trace>) + 'static {
    let label = label.into();
    move |mut trace: ResMut<Trace>| trace.0.push(label.clone())
}

/// A server-tick set's label: `A:` and the set's name.
pub(super) fn authority_label(set: &ServerStage) -> String {
    format!("A:{}", server_stage_name(set))
}

/// A client-frame set's label: `C:` and the set's name.
pub(super) fn client_label(set: &ClientStep) -> String {
    format!("C:{}", client_stage_name(set))
}

/// The labels the tables say should come out, in table order.
pub(super) fn authority_expected() -> Vec<String> {
    SERVER_STAGES.iter().map(authority_label).collect()
}

pub(super) fn client_expected() -> Vec<String> {
    CLIENT_STAGES.iter().map(client_label).collect()
}

/// One probe per server-tick set, in `FixedUpdate`.
pub(super) fn probe_authority_sets(harness: &mut App) {
    for (&set, label) in SERVER_STAGES.iter().zip(authority_expected()) {
        harness.add_systems(FixedUpdate, note(label).in_set(set));
    }
}

/// One probe per client-frame set, in `Update`.
pub(super) fn probe_client_sets(harness: &mut App) {
    for (&set, label) in CLIENT_STAGES.iter().zip(client_expected()) {
        harness.add_systems(Update, note(label).in_set(set));
    }
}

/// An app with just enough in it to run the schedules, and an empty trace.
pub(super) fn bare_app() -> App {
    let mut harness = App::new();
    harness.add_plugins(bevy::time::TimePlugin)
        .add_message::<GameSessionClosed>()
        .init_resource::<Trace>();
    harness
}

pub(super) fn trace_of(harness: &App) -> Trace {
    Trace(harness.world().resource::<Trace>().0.clone())
}
