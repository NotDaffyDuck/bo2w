//! The resources the client half of a frame passes along: what came off the wire, what was
//! adopted from it, and what the presentation still has to spawn.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::prelude::*;
use playerstate_t6::game::InputCommand;
use sim::Snapshot;

use crate::client::run_ahead::LocalForecast;
use crate::client::remote_buffer::OtherPlayerSmoother;
use crate::net_links::listen_link::ArrivedHostStep;

/// The local player's predicted world, owned by the client side of the frame.
#[derive(Resource)]
pub struct ClientGuessState(pub LocalForecast);

impl Default for ClientGuessState {
    fn default() -> Self {
        Self(LocalForecast::new(sim::ClientId(0)))
    }
}

/// Entity events that arrived with snapshots and have not been played yet. While the local
/// player is watching a kill replay the events go to `archived` instead of `live`.
#[derive(Resource, Default)]
pub struct StagedShownEntityEvents {
    pub live: Vec<sim::EntityEventRecord>,

    pub archived: Vec<sim::EntityEventRecord>,

    pub replaying_final_death: bool,
}

impl StagedShownEntityEvents {
    /// Takes the entity events out of a snapshot, unless the snapshot is not newer than
    /// `newest_known`; then nothing is touched and `false` comes back. Which list they join
    /// depends on whether the snapshot shows `local` watching a kill replay.
    pub fn take_from(
        &mut self,
        newest_known: Option<sim::Tick>,
        own_id: sim::ClientId,
        frame_snap: &mut Snapshot,
    ) -> bool {
        if crate::place_of_tick(newest_known, frame_snap.tick) != crate::FramePosition::New {
            return false;
        }
        self.replaying_final_death = frame_snap
            .meta
            .for_client(own_id)
            .is_some_and(|row| row.killcam_hud.is_some());
        let target = if self.replaying_final_death { &mut self.archived } else { &mut self.live };
        target.append(&mut frame_snap.meta.entity_events);
        true
    }
}

#[derive(Resource, Default)]
pub struct QueuedPelletEffect(pub Vec<sim::PelletFxRecord>);

/// Broken-off pieces the host sent that this PC has not spawned yet.
#[derive(Resource, Default)]
pub struct PendingDebris(pub Vec<sim::DebrisRecord>);

/// Ticks read off the link and waiting for the reconcile step, oldest first.
#[derive(Resource, Default)]
pub struct ArrivedStepQueue(pub VecDeque<ArrivedHostStep>);

/// The two newest snapshots taken in: `upcoming_world_state` is the latest, `snap` the one before it.
#[derive(Resource, Default, Debug)]
pub struct NewestTakenSnapshot {
    pub frame_state: Option<Arc<Snapshot>>,
    pub upcoming_world_state: Option<Arc<Snapshot>>,

    pub taken_in_current_step: bool,
}

impl NewestTakenSnapshot {
    pub fn next(&self) -> Option<&Snapshot> {
        self.upcoming_world_state.as_deref()
    }

    /// Takes `fresh` in as the newest snapshot; the one that was newest becomes the previous one.
    pub fn take_in(&mut self, fresh: Arc<Snapshot>) {
        if let Some(until_now) = self.upcoming_world_state.replace(fresh) {
            self.frame_state = Some(until_now);
        }
    }

    /// The sound alias registered under `index`, from the latest snapshot that names it.
    pub fn sound_cue_name(&self, slot_no: u8) -> Option<&str> {
        self.upcoming_world_state
            .as_deref()
            .and_then(|snap| alias_in(snap, slot_no))
            .or_else(|| self.frame_state.as_deref().and_then(|snap| alias_in(snap, slot_no)))
    }

    /// The effect registered under `index`; only the latest snapshot is asked.
    pub fn effect_label(&self, slot_no: u8) -> Option<&str> {
        let snap = self.next()?;
        let (_, name) = snap.meta.effect_names.iter().find(|(at, _)| *at == slot_no)?;
        Some(name.as_str())
    }
}

fn alias_in(snap: &Snapshot, slot_no: u8) -> Option<&str> {
    let (_, name) = snap.meta.sound_aliases.iter().find(|(at, _)| *at == slot_no)?;
    Some(name.as_str())
}

#[derive(Resource, Default)]
pub struct OtherPlayerSmootherHolder(pub OtherPlayerSmoother);

/// The command the next frame's input is built from, once the first snapshot has set it up.
#[derive(Resource, Default)]
pub struct PlayerMoveBlueprint {
    pub cmd: InputCommand,
    pub prefilled: bool,
}

/// The newest reliable-message number this client has taken in and acknowledged.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerGuaranteedReceipt(pub u16);

/// A reliable event from the host that the client systems answer (a forced weapon switch, ...).
#[derive(bevy::prelude::Message, Clone, Copy, Debug)]
pub struct GuaranteedSimEvent(pub sim::MatchOccurrence);

/// The names of the client phases in the order they ran this frame; tests turn it on by
/// inserting the resource.
#[derive(Resource, Default, Debug, Clone)]
pub struct ReceiverStageLog(pub Vec<&'static str>);

pub(super) fn note_phase(trace: Option<ResMut<ReceiverStageLog>>, phase: &'static str) {
    if let Some(mut trace) = trace {
        trace.0.push(phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with(aliases: &[(u8, &str)], effects: &[(u8, &str)]) -> Arc<Snapshot> {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        snap.meta.sound_aliases = aliases.iter().map(|(i, n)| (*i, n.to_string())).collect();
        snap.meta.effect_names = effects.iter().map(|(i, n)| (*i, n.to_string())).collect();
        Arc::new(snap)
    }

    #[test]
    fn a_sound_name_is_looked_up_newest_snapshot_first() {
        let held = NewestTakenSnapshot {
            frame_state: Some(snapshot_with(&[(1, "old_one"), (2, "old_two")], &[])),
            upcoming_world_state: Some(snapshot_with(&[(1, "new_one")], &[])),
            taken_in_current_step: false,
        };
        assert_eq!(held.sound_cue_name(1), Some("new_one"));
        assert_eq!(held.sound_cue_name(2), Some("old_two"));
        assert_eq!(held.sound_cue_name(3), None);
    }

    #[test]
    fn an_effect_name_is_only_read_from_the_newest_snapshot() {
        let held = NewestTakenSnapshot {
            frame_state: Some(snapshot_with(&[], &[(4, "only_old")])),
            upcoming_world_state: Some(snapshot_with(&[], &[(5, "fresh")])),
            taken_in_current_step: false,
        };
        assert_eq!(held.effect_label(5), Some("fresh"));
        assert_eq!(held.effect_label(4), None);
        assert_eq!(NewestTakenSnapshot::default().effect_label(5), None);
    }

    #[test]
    fn taking_in_a_snapshot_pushes_the_newest_back_one_place() {
        let mut held = NewestTakenSnapshot::default();
        let first = snapshot_with(&[(1, "a")], &[]);
        let second = snapshot_with(&[(1, "b")], &[]);
        held.take_in(Arc::clone(&first));
        assert!(held.frame_state.is_none());
        assert!(Arc::ptr_eq(held.upcoming_world_state.as_ref().unwrap(), &first));
        held.take_in(Arc::clone(&second));
        assert!(Arc::ptr_eq(held.frame_state.as_ref().unwrap(), &first));
        assert!(Arc::ptr_eq(held.upcoming_world_state.as_ref().unwrap(), &second));
    }

    const ME: sim::ClientId = sim::ClientId(2);

    fn event_record(sequence: u32) -> sim::EntityEventRecord {
        sim::EntityEventRecord {
            sequence: sim::EventSequence(sequence),
            tick: sim::Tick(sequence),
            audience: sim::EventRecipients::All,
            event: entity_t6::EventCode(4),
            payload: sim::EntityEventPayload::default(),
        }
    }

    fn snapshot_at(tick: u32, events: &[u32]) -> Snapshot {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        snap.tick = sim::Tick(tick);
        snap.meta.entity_events = events.iter().map(|n| event_record(*n)).collect();
        snap
    }

    fn watching_replay(mut snap: Snapshot) -> Snapshot {
        let row = sim::ClientSnapshotMeta {
            killcam_hud: Some(sim::KillcamHud { final_kill: false }),
            ..Default::default()
        };
        snap.meta.clients.push((ME, row));
        snap
    }

    fn sequences(list: &[sim::EntityEventRecord]) -> Vec<u32> {
        list.iter().map(|record| record.sequence.0).collect()
    }

    #[test]
    fn the_events_of_a_new_snapshot_join_the_live_list() {
        let mut pending = StagedShownEntityEvents::default();
        let mut snap = snapshot_at(5, &[1, 2]);
        assert!(pending.take_from(Some(sim::Tick(4)), ME, &mut snap));
        assert_eq!(sequences(&pending.live), [1, 2]);
        assert!(pending.archived.is_empty());
        assert!(!pending.replaying_final_death);
        assert!(snap.meta.entity_events.is_empty());
    }

    #[test]
    fn the_first_snapshot_ever_is_new() {
        let mut pending = StagedShownEntityEvents::default();
        let mut snap = snapshot_at(0, &[7]);
        assert!(pending.take_from(None, ME, &mut snap));
        assert_eq!(sequences(&pending.live), [7]);
    }

    #[test]
    fn a_snapshot_that_is_not_newer_is_left_alone() {
        let mut pending = StagedShownEntityEvents::default();
        for (known, incoming) in [(9, 9), (9, 8)] {
            let mut snap = snapshot_at(incoming, &[1]);
            assert!(!pending.take_from(Some(sim::Tick(known)), ME, &mut snap));
            assert_eq!(snap.meta.entity_events.len(), 1);
        }
        assert!(pending.live.is_empty() && pending.archived.is_empty());
    }

    #[test]
    fn events_seen_during_a_kill_replay_are_archived_instead() {
        let mut pending = StagedShownEntityEvents::default();
        let mut snap = watching_replay(snapshot_at(3, &[4, 5]));
        assert!(pending.take_from(None, ME, &mut snap));
        assert!(pending.replaying_final_death);
        assert_eq!(sequences(&pending.archived), [4, 5]);
        assert!(pending.live.is_empty());

        let mut live_again = snapshot_at(4, &[6]);
        assert!(pending.take_from(Some(sim::Tick(3)), ME, &mut live_again));
        assert!(!pending.replaying_final_death);
        assert_eq!(sequences(&pending.live), [6]);
        assert_eq!(sequences(&pending.archived), [4, 5]);
    }

    #[test]
    fn another_players_replay_does_not_archive_my_events() {
        let mut pending = StagedShownEntityEvents::default();
        let mut snap = watching_replay(snapshot_at(3, &[4]));
        assert!(pending.take_from(None, sim::ClientId(9), &mut snap));
        assert_eq!(sequences(&pending.live), [4]);
    }

    #[test]
    fn no_snapshot_means_no_names() {
        let empty = NewestTakenSnapshot::default();
        assert_eq!(empty.sound_cue_name(0), None);
        assert!(empty.next().is_none());
    }

    #[test]
    fn the_phase_trace_collects_in_call_order() {
        let mut app = App::new();
        app.insert_resource(ReceiverStageLog::default());
        app.add_systems(Update, (|t: Option<ResMut<ReceiverStageLog>>| note_phase(t, "first"),
            |t: Option<ResMut<ReceiverStageLog>>| note_phase(t, "second")).chain());
        app.update();
        assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["first", "second"]);
    }
}
