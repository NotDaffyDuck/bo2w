//! Host-side bookkeeping for a single match: its load, its run, and how each peer is let in.
//!
//! Events go in and the caller carries out the effects that come out. This module never reads
//! the clock and never touches the network.

use std::collections::HashMap;

use frame::GameSessionStamp;
use master_protocol::{EntryRefusalReason, RoomOccupantId};

use super::deadlines::{
    AbortStep, PEER_ENTRY_TIMEOUT_MILLIS, LoadStageKind, LoadStageTimer, SessionAbortRecord,
};
use super::world_rules::{HostMapLoaded, teardown_hits_key};
use super::transition::describe_session_change;
use crate::net_links::match_handoff::same_generation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GameRoundStage {
    #[default]
    None,
    Loading,
    Running,
    Ending,
}

impl GameRoundStage {
    /// True while a match is loading or running.
    fn still_counting(self) -> bool {
        self == Self::Loading || self == Self::Running
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerJoinStage {
    #[default]
    Waiting,
    CatchingUp { bootstrap_id: u32 },
    WelcomedIn { bootstrap_id: u32 },
    Failed,
    Cancelled,
}

impl PlayerJoinStage {
    /// The peer's admission has reached an end state.
    fn is_over(self) -> bool {
        !self.is_pending()
    }

    /// The peer has not been let in yet and can still be.
    fn is_pending(self) -> bool {
        match self {
            Self::Waiting | Self::CatchingUp { .. } => true,
            Self::WelcomedIn { .. } | Self::Failed | Self::Cancelled => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerEntryGate {
    pub phase: PlayerJoinStage,
    /// The world the peer reported once its map finished loading.
    pub loaded_world: Option<HostMapLoaded>,
    /// Host clock at the moment the peer's admission clock started; zero means not started.
    pub waiting_since_ms: u64,
    pub link_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostRoundOccurrence {
    Start { game_session_key: GameSessionStamp, clock_millis: u64 },
    WorldUp { ready: HostMapLoaded, clock_millis: u64 },
    HostAdvance { clock_millis: u64 },
    MapLoaded { member: RoomOccupantId, loaded: HostMapLoaded, clock_millis: u64 },
    SetupReady {
        game_session_key: GameSessionStamp,
        member: RoomOccupantId,
        transfer_ticket: u32,
        link_no: Option<u64>,
    },
    TakenIntoEffect {
        game_session_key: GameSessionStamp,
        member: RoomOccupantId,
        transfer_ticket: u32,
        link_no: Option<u64>,
    },
    MatchEnded { game_session_key: GameSessionStamp },
    PeerCancelled { member: RoomOccupantId },
    Close,
    Tick { clock_millis: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostRoundConsequence {
    ReadyToAnnounce { game_session_key: GameSessionStamp, ready: HostMapLoaded },
    PublishEnd { game_session_key: GameSessionStamp },
    FlushBootstraps { member: RoomOccupantId },
    Enter {
        member: RoomOccupantId,
        transfer_ticket: u32,
        game_session_key: GameSessionStamp,
    },
    DropRemotePeer {
        member: RoomOccupantId,
        reason: EntryRefusalReason,
        game_session_key: GameSessionStamp,
    },
    Fail { fail: SessionAbortRecord },
}

impl HostRoundConsequence {
    fn cancel(member: RoomOccupantId, reason: EntryRefusalReason, game_session_key: GameSessionStamp) -> Self {
        Self::DropRemotePeer { member, reason, game_session_key }
    }

    /// The peer an effect is aimed at, when it is aimed at one.
    fn target_member(&self) -> Option<RoomOccupantId> {
        match self {
            Self::FlushBootstraps { member: seat }
            | Self::Enter { member: seat, .. }
            | Self::DropRemotePeer { member: seat, .. } => Some(*seat),
            Self::ReadyToAnnounce { .. } | Self::PublishEnd { .. } | Self::Fail { .. } => None,
        }
    }

    fn joined_setup_stage(&self) -> Option<u32> {
        match self {
            Self::Enter { transfer_ticket: setup_no, .. } => Some(*setup_no),
            _ => None,
        }
    }

    fn failure_source(&self) -> Option<&str> {
        match self {
            Self::Fail { fail } => Some(fail.source.as_str()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRoundInstall {
    pub was_taken: bool,
    pub effects: Vec<HostRoundConsequence>,
    pub change_note: String,
}

/// Identifiers that an event carries by itself; they feed the transition line.
#[derive(Default)]
struct EventIds {
    seat: Option<RoomOccupantId>,
    bootstrap: Option<u32>,
    link: Option<u64>,
    clock_ms: Option<u64>,
    /// When the phase being left started; elapsed time is measured from here.
    phase_start_ms: u64,
}

impl EventIds {
    fn from_event(
        event: &HostRoundOccurrence,
        phase_start_ms: u64,
    ) -> Self {
        let mut ids = Self {
            phase_start_ms,
            ..Self::default()
        };
        match event {
            HostRoundOccurrence::MatchEnded { .. } | HostRoundOccurrence::Close => {}
            HostRoundOccurrence::PeerCancelled { member } => ids.seat = Some(*member),
            HostRoundOccurrence::MapLoaded { member, clock_millis, .. } => {
                ids.seat = Some(*member);
                ids.clock_ms = Some(*clock_millis);
            }
            HostRoundOccurrence::Start { clock_millis, .. }
            | HostRoundOccurrence::WorldUp { clock_millis, .. }
            | HostRoundOccurrence::HostAdvance { clock_millis }
            | HostRoundOccurrence::Tick { clock_millis } => ids.clock_ms = Some(*clock_millis),
            HostRoundOccurrence::SetupReady {
                member,
                transfer_ticket: setup_no,
                link_no: wire_link,
                ..
            }
            | HostRoundOccurrence::TakenIntoEffect {
                member,
                transfer_ticket: setup_no,
                link_no: wire_link,
                ..
            } => {
                ids.seat = Some(*member);
                ids.bootstrap = Some(*setup_no);
                ids.link = *wire_link;
            }
        }
        ids
    }
}

/// The first way a peer's loaded world differs from the host's, checked in a fixed order.
fn first_difference(
    ours: HostMapLoaded,
    theirs: HostMapLoaded,
) -> Option<EntryRefusalReason> {
    if ours.map != theirs.map {
        return Some(EntryRefusalReason::LevelAssetHashes {
            host_digest: ours.map,
            peer_digest: theirs.map,
        });
    }
    if ours.weapon_digest != theirs.weapon_digest {
        return Some(EntryRefusalReason::WeaponLedger {
            host_digest: ours.weapon_digest,
            peer_digest: theirs.weapon_digest,
        });
    }
    if ours.class_digest != theirs.class_digest {
        return Some(EntryRefusalReason::LoadoutTableHashes {
            host_digest: ours.class_digest,
            peer_digest: theirs.class_digest,
        });
    }
    None
}

/// Compares a peer's loaded world with the host's once both are known.
///
/// Yields nothing when the peer's admission is already over or it loaded another epoch. A
/// difference fails the peer and requests its cancellation; no difference requests its bootstraps.
fn vet_peer(
    key: GameSessionStamp,
    ours: HostMapLoaded,
    member: RoomOccupantId,
    admission: &mut PeerEntryGate,
) -> Option<HostRoundConsequence> {
    if admission.phase.is_over() {
        return None;
    }
    let theirs = admission.loaded_world?;
    if !same_generation(theirs.round_era, key.game_generation) {
        return None;
    }
    let Some(reason) = first_difference(ours, theirs) else {
        return Some(HostRoundConsequence::FlushBootstraps { member });
    };
    admission.phase = PlayerJoinStage::Failed;
    Some(HostRoundConsequence::cancel(member, reason, key))
}

#[derive(Clone, Debug, Default)]
pub struct HostRoundCore {
    closed: bool,
    key: GameSessionStamp,
    round_stage: GameRoundStage,
    world: Option<HostMapLoaded>,
    authority_up: bool,
    admissions: HashMap<RoomOccupantId, PeerEntryGate>,
    watch: LoadStageTimer,
    incarnation_no: u64,
    load_request: Option<u64>,
}

impl HostRoundCore {
    pub fn game_session_key(&self) -> GameSessionStamp {
        self.key
    }

    pub fn phase(&self) -> GameRoundStage {
        self.round_stage
    }

    pub fn host_world(&self) -> Option<HostMapLoaded> {
        self.world
    }

    pub fn host_prepared(&self) -> bool {
        self.authority_up
    }

    pub fn gate_for(&self, who: RoomOccupantId) -> Option<&PeerEntryGate> {
        self.admissions.get(&who)
    }

    pub fn remember_incarnation(&mut self, number: u64) {
        self.incarnation_no = number;
    }

    pub fn remember_load_ticket(&mut self, request: Option<u64>) {
        self.load_request = request;
    }

    pub fn load_ticket(&self) -> Option<u64> {
        self.load_request
    }

    /// Handles one event. The reply tells whether it was accepted and what to do next, and holds
    /// the transition line for the log.
    pub fn take_event(
        &mut self,
        event: HostRoundOccurrence,
    ) -> HostRoundInstall {
        let stage_before = self.stage_label();
        let ids = EventIds::from_event(&event, self.watch.stage_entered_stamp());
        let handled = match self.closed {
            true => Err("session closed"),
            false => self.route_event(event),
        };
        self.conclude(&ids, stage_before, handled)
    }

    fn route_event(&mut self, event: HostRoundOccurrence) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        match event {
            HostRoundOccurrence::Tick { clock_millis } => Ok(("tick", self.on_tick(clock_millis))),
            HostRoundOccurrence::Close => {
                self.closed = true;
                self.round_stage = GameRoundStage::None;
                Ok(("close session", Vec::default()))
            }
            HostRoundOccurrence::PeerCancelled { member } => {
                if let Some(admission) = self.admissions.get_mut(&member) {
                    admission.phase = PlayerJoinStage::Cancelled;
                }
                let cancel = HostRoundConsequence::cancel(member, EntryRefusalReason::Cancelled, self.key);
                Ok(("peer cancelled", vec![cancel]))
            }
            HostRoundOccurrence::MatchEnded { game_session_key } => self.close_match(game_session_key),
            HostRoundOccurrence::TakenIntoEffect {
                game_session_key,
                member,
                transfer_ticket: setup_no,
                link_no: wire_link,
            } => self.on_applied(game_session_key, member, setup_no, wire_link),
            HostRoundOccurrence::SetupReady {
                game_session_key,
                member,
                transfer_ticket: setup_no,
                link_no: wire_link,
            } => self.on_bootstrap_prepared(game_session_key, member, setup_no, wire_link),
            HostRoundOccurrence::MapLoaded {
                member,
                loaded,
                clock_millis,
            } => self.on_map_loaded(member, loaded, clock_millis),
            HostRoundOccurrence::HostAdvance { clock_millis } => {
                if self.round_stage == GameRoundStage::Running {
                    self.watch.mark_alive(clock_millis);
                }
                Ok(("authority progress", Vec::default()))
            }
            HostRoundOccurrence::WorldUp { ready, clock_millis } => self.on_authority_ready(ready, clock_millis),
            HostRoundOccurrence::Start { game_session_key, clock_millis } => self.on_start(game_session_key, clock_millis),
        }
    }

    fn on_start(
        &mut self,
        game_session_key: GameSessionStamp,
        clock_millis: u64,
    ) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if game_session_key.game_generation == 0 {
            return Err("start without match key");
        }
        if self.key == game_session_key {
            match self.round_stage {
                GameRoundStage::Loading => return Ok(("start already loading", Vec::default())),
                GameRoundStage::Running => return Ok(("start already running", Vec::default())),
                GameRoundStage::None | GameRoundStage::Ending => {}
            }
        }
        self.key = game_session_key;
        self.round_stage = GameRoundStage::Loading;
        self.world = None;
        self.authority_up = false;
        self.admissions.clear();
        self.watch.enter_phase(LoadStageKind::Loading, clock_millis);
        Ok(("start loading", Vec::default()))
    }

    fn on_authority_ready(
        &mut self,
        ready: HostMapLoaded,
        clock_millis: u64,
    ) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if !same_generation(ready.round_era, self.key.game_generation) {
            return Err("stale host world");
        }
        if !self.round_stage.still_counting() {
            return Err("host world without match");
        }
        self.world = Some(ready);
        self.authority_up = true;
        match self.round_stage {
            GameRoundStage::Loading => {
                self.round_stage = GameRoundStage::Running;
                self.watch.enter_phase(LoadStageKind::Running, clock_millis);
                self.admissions
                    .values_mut()
                    .for_each(|admission| admission.waiting_since_ms = clock_millis);
            }
            _ => self.watch.mark_alive(clock_millis),
        }
        Ok(("authority ready", self.vet_all_peers(ready)))
    }

    /// Runs the world comparison for every known peer and gathers what it asks for.
    fn vet_all_peers(
        &mut self,
        ours: HostMapLoaded,
    ) -> Vec<HostRoundConsequence> {
        let key = self.key;
        let mut asked = Vec::new();
        for (seat, gate) in self.admissions.iter_mut() {
            asked.extend(vet_peer(key, ours, *seat, gate));
        }
        asked
    }

    fn on_map_loaded(
        &mut self,
        member: RoomOccupantId,
        loaded: HostMapLoaded,
        clock_millis: u64,
    ) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if !self.round_stage.still_counting() || !same_generation(loaded.round_era, self.key.game_generation) {
            return Err("stale map loaded");
        }
        let key = self.key;
        let ours = self.world;
        let is_running = self.round_stage == GameRoundStage::Running;
        let admission = self.admissions.entry(member).or_default();
        if matches!(admission.phase, PlayerJoinStage::Failed | PlayerJoinStage::Cancelled) {
            return Err("map loaded after admission ended");
        }
        if is_running && admission.waiting_since_ms == 0 {
            admission.waiting_since_ms = clock_millis;
        }
        admission.loaded_world = Some(loaded);
        let effects = ours
            .and_then(|world| vet_peer(key, world, member, admission))
            .into_iter()
            .collect();
        Ok(("client map loaded", effects))
    }

    fn on_bootstrap_prepared(
        &mut self,
        game_session_key: GameSessionStamp,
        member: RoomOccupantId,
        setup_no: u32,
        wire_link: Option<u64>,
    ) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if game_session_key.is_none() || game_session_key != self.key || !self.round_stage.still_counting() {
            return Err("stale bootstrap completion");
        }
        let Some(admission) = self.admissions.get_mut(&member) else {
            return Err("bootstrap for unknown peer");
        };
        if admission.phase.is_over() {
            return Err("bootstrap after admission ended");
        }
        admission.link_id = wire_link.or(admission.link_id);
        admission.phase = PlayerJoinStage::CatchingUp { bootstrap_id: setup_no };
        Ok(("bootstrap prepared", Vec::default()))
    }

    fn on_applied(
        &mut self,
        game_session_key: GameSessionStamp,
        member: RoomOccupantId,
        setup_no: u32,
        wire_link: Option<u64>,
    ) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if game_session_key.is_none() || game_session_key != self.key {
            return Err("stale applied");
        }
        if !self.round_stage.still_counting() {
            return Err("applied without match");
        }
        let Some(admission) = self.admissions.get_mut(&member) else {
            return Err("applied unknown peer");
        };
        let reason = match admission.phase {
            PlayerJoinStage::CatchingUp { bootstrap_id: due } if due == setup_no => "enter committed",
            PlayerJoinStage::WelcomedIn { bootstrap_id: done } if done == setup_no => {
                "repeat applied repeats enter"
            }
            _ => return Err("applied mismatch"),
        };
        admission.link_id = wire_link.or(admission.link_id);
        admission.phase = PlayerJoinStage::WelcomedIn { bootstrap_id: setup_no };
        let enter = HostRoundConsequence::Enter {
            member,
            transfer_ticket: setup_no,
            game_session_key,
        };
        Ok((reason, vec![enter]))
    }

    fn close_match(&mut self, game_session_key: GameSessionStamp) -> Result<(&'static str, Vec<HostRoundConsequence>), &'static str> {
        if !self.round_stage.still_counting() || !teardown_hits_key(game_session_key, self.key) {
            return Err("stale match end");
        }
        self.round_stage = GameRoundStage::None;
        self.world = None;
        self.authority_up = false;
        self.admissions.clear();
        self.watch.enter_phase(LoadStageKind::Idle, 0);
        Ok(("end match", Vec::default()))
    }

    fn on_tick(
        &mut self,
        clock_millis: u64,
    ) -> Vec<HostRoundConsequence> {
        let running = self.round_stage == GameRoundStage::Running;
        let mut effects: Vec<HostRoundConsequence> = vec![];
        if self.watch.timed_out(clock_millis) {
            let (stage, source) = match running {
                true => (AbortStep::Authority, "authority progress silence"),
                false => (AbortStep::Load, "host load deadline"),
            };
            let fail = SessionAbortRecord {
                stage,
                source: source.to_owned(),
                game_session_key: self.key,
            };
            effects.push(HostRoundConsequence::Fail { fail });
        }
        if !running {
            return effects;
        }
        let key = self.key;
        for (seat, gate) in self.admissions.iter_mut() {
            let timed_out = gate.phase.is_pending()
                && gate.waiting_since_ms != 0
                && clock_millis.saturating_sub(gate.waiting_since_ms) >= PEER_ENTRY_TIMEOUT_MILLIS;
            if timed_out {
                gate.phase = PlayerJoinStage::Failed;
                effects.push(HostRoundConsequence::cancel(*seat, EntryRefusalReason::CutoffInstant, key));
            }
        }
        effects
    }

    fn stage_label(&self) -> String {
        format!(
            "open={},epoch={},phase={:?},authority={}",
            !self.closed, self.key.game_generation, self.round_stage, self.authority_up
        )
    }

    /// Assembles the reply. Identifiers the event did not carry are taken from the effects and
    /// from the recorded peer.
    fn conclude(&self, ids: &EventIds, stage_before: String, outcome: Result<(&'static str, Vec<HostRoundConsequence>), &'static str>) -> HostRoundInstall {
        let accepted = outcome.is_ok();
        let (reason, effects) = outcome.unwrap_or_else(|refusal| (refusal, Vec::default()));
        let seat = ids
            .seat
            .or_else(|| effects.iter().find_map(HostRoundConsequence::target_member));
        let bootstrap = ids
            .bootstrap
            .or_else(|| effects.iter().find_map(HostRoundConsequence::joined_setup_stage));
        let link = ids
            .link
            .or_else(|| self.admissions.get(&seat?)?.link_id);
        let spent_ms = ids
            .clock_ms
            .map(|clock| clock.saturating_sub(ids.phase_start_ms));
        let fail_note = effects.iter().find_map(HostRoundConsequence::failure_source).unwrap_or("-");
        let seat_raw = seat.map(|id| id.0);
        let incarn = Some(self.incarnation_no).filter(|number| *number != 0);
        let transition = describe_session_change(
            self.key.stamp_bytes,
            incarn,
            self.key.game_generation,
            self.load_request,
            seat_raw,
            link,
            bootstrap,
            &stage_before,
            reason,
            &self.stage_label(),
            spent_ms,
            fail_note,
        );
        HostRoundInstall { was_taken: accepted, effects, change_note: transition }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: [u8; 16] = [7; 16];
    const ALICE: RoomOccupantId = RoomOccupantId([1; 16]);
    const BOB: RoomOccupantId = RoomOccupantId([2; 16]);

    fn send(
        core: &mut HostRoundCore,
        event: HostRoundOccurrence,
    ) -> HostRoundInstall {
        core.take_event(event)
    }

    fn key(epoch: u32) -> GameSessionStamp {
        GameSessionStamp::new(SESSION, epoch)
    }

    fn world(epoch: u32) -> HostMapLoaded {
        HostMapLoaded {
            round_era: epoch,
            map: 10,
            weapon_digest: 20,
            class_digest: 30,
        }
    }

    fn started(epoch: u32, at: u64) -> HostRoundCore {
        let mut core = HostRoundCore::default();
        let outcome = send(&mut core, HostRoundOccurrence::Start {
            game_session_key: key(epoch),
            clock_millis: at,
        });
        assert!(outcome.was_taken);
        core
    }

    fn running(epoch: u32) -> HostRoundCore {
        let mut core = started(epoch, 100);
        send(&mut core, HostRoundOccurrence::WorldUp {
            ready: world(epoch),
            clock_millis: 200,
        });
        core
    }

    fn loaded(
        core: &mut HostRoundCore,
        member: RoomOccupantId,
        loaded: HostMapLoaded,
        clock_millis: u64,
    ) -> HostRoundInstall {
        send(core, HostRoundOccurrence::MapLoaded {
            member,
            loaded,
            clock_millis,
        })
    }

    fn admit(
        core: &mut HostRoundCore,
        member: RoomOccupantId,
        epoch: u32,
        boot: u32,
    ) {
        loaded(core, member, world(epoch), 300);
        send(core, HostRoundOccurrence::SetupReady {
            game_session_key: key(epoch),
            member,
            transfer_ticket: boot,
            link_no: Some(55),
        });
    }

    #[test]
    fn a_start_moves_from_none_to_loading() {
        let core = started(3, 100);
        assert_eq!(core.phase(), GameRoundStage::Loading);
        assert_eq!(core.game_session_key(), key(3));
        assert!(!core.host_prepared());
        assert_eq!(core.host_world(), None);
    }

    #[test]
    fn a_start_without_a_key_is_refused() {
        let mut core = HostRoundCore::default();
        let outcome = send(&mut core, HostRoundOccurrence::Start { game_session_key: GameSessionStamp::NONE, clock_millis: 5 });
        assert!(!outcome.was_taken);
        assert!(outcome.change_note.contains("event=start without match key"));
        assert_eq!(core.phase(), GameRoundStage::None);
    }

    #[test]
    fn starting_the_same_match_twice_changes_nothing() {
        let mut core = started(3, 100);
        let again = send(&mut core, HostRoundOccurrence::Start {
            game_session_key: key(3),
            clock_millis: 999,
        });
        assert!(again.was_taken);
        assert!(again.change_note.contains("event=start already loading"));
        let mut core = running(3);
        let again = send(&mut core, HostRoundOccurrence::Start {
            game_session_key: key(3),
            clock_millis: 999,
        });
        assert!(again.change_note.contains("event=start already running"));
        assert_eq!(core.phase(), GameRoundStage::Running);
    }

    #[test]
    fn a_new_match_replaces_the_old_one_and_drops_its_peers() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        send(&mut core, HostRoundOccurrence::Start {
            game_session_key: key(4),
            clock_millis: 400,
        });
        assert_eq!(core.phase(), GameRoundStage::Loading);
        assert!(core.gate_for(ALICE).is_none());
        assert!(!core.host_prepared());
    }

    #[test]
    fn the_authority_being_ready_starts_the_run() {
        let mut core = started(3, 100);
        let outcome = send(&mut core, HostRoundOccurrence::WorldUp {
            ready: world(3),
            clock_millis: 250,
        });
        assert!(outcome.was_taken);
        assert_eq!(core.phase(), GameRoundStage::Running);
        assert!(core.host_prepared());
        assert_eq!(core.host_world(), Some(world(3)));
    }

    #[test]
    fn a_world_for_another_epoch_or_no_match_is_refused() {
        let mut core = started(3, 100);
        let stale = send(&mut core, HostRoundOccurrence::WorldUp {
            ready: world(2),
            clock_millis: 1,
        });
        assert!(!stale.was_taken);
        assert!(stale.change_note.contains("event=stale host world"));
        let mut idle = HostRoundCore::default();
        let none = send(&mut idle, HostRoundOccurrence::WorldUp {
            ready: world(0),
            clock_millis: 1,
        });
        assert!(none.change_note.contains("event=stale host world"));
    }

    #[test]
    fn a_peer_that_loaded_before_the_host_is_flushed_when_the_host_is_ready() {
        let mut core = started(3, 100);
        let early = loaded(&mut core, ALICE, world(3), 120);
        assert!(early.was_taken);
        assert!(early.effects.is_empty());
        let ready = send(&mut core, HostRoundOccurrence::WorldUp {
            ready: world(3),
            clock_millis: 200,
        });
        assert_eq!(
            ready.effects,
            vec![HostRoundConsequence::FlushBootstraps { member: ALICE }]
        );
        assert_eq!(core.gate_for(ALICE).unwrap().waiting_since_ms, 200);
    }

    #[test]
    fn a_peer_loading_after_the_host_is_flushed_at_once() {
        let mut core = running(3);
        let outcome = loaded(&mut core, ALICE, world(3), 300);
        assert_eq!(
            outcome.effects,
            vec![HostRoundConsequence::FlushBootstraps { member: ALICE }]
        );
        assert_eq!(core.gate_for(ALICE).unwrap().waiting_since_ms, 300);
    }

    #[test]
    fn a_peer_with_other_content_is_cancelled_for_the_first_difference() {
        let cases: [(HostMapLoaded, EntryRefusalReason); 3] = [
            (
                HostMapLoaded { map: 11, ..world(3) },
                EntryRefusalReason::LevelAssetHashes { host_digest: 10, peer_digest: 11 },
            ),
            (
                HostMapLoaded { weapon_digest: 21, ..world(3) },
                EntryRefusalReason::WeaponLedger { host_digest: 20, peer_digest: 21 },
            ),
            (
                HostMapLoaded { class_digest: 31, ..world(3) },
                EntryRefusalReason::LoadoutTableHashes { host_digest: 30, peer_digest: 31 },
            ),
        ];
        for (peer_world, reason) in cases {
            let mut core = running(3);
            let outcome = loaded(&mut core, ALICE, peer_world, 300);
            assert_eq!(
                outcome.effects,
                vec![HostRoundConsequence::cancel(ALICE, reason, key(3))]
            );
            assert_eq!(core.gate_for(ALICE).unwrap().phase, PlayerJoinStage::Failed);
        }
    }

    #[test]
    fn a_map_loaded_for_the_wrong_epoch_is_refused() {
        let mut core = running(3);
        let outcome = loaded(&mut core, ALICE, world(2), 300);
        assert!(!outcome.was_taken);
        assert!(outcome.change_note.contains("event=stale map loaded"));
        assert!(core.gate_for(ALICE).is_none());
    }

    #[test]
    fn a_failed_peer_cannot_load_again() {
        let mut core = running(3);
        loaded(&mut core, ALICE, HostMapLoaded { map: 99, ..world(3) }, 300);
        let again = loaded(&mut core, ALICE, world(3), 310);
        assert!(!again.was_taken);
        assert!(again.change_note.contains("event=map loaded after admission ended"));
    }

    #[test]
    fn the_bootstrap_and_apply_steps_admit_a_peer_once_and_repeat_harmlessly() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        assert_eq!(core.gate_for(ALICE).unwrap().phase, PlayerJoinStage::CatchingUp { bootstrap_id: 8 });
        assert_eq!(core.gate_for(ALICE).unwrap().link_id, Some(55));

        let outcome = send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 8,
            link_no: None,
        });
        let enter = HostRoundConsequence::Enter {
            member: ALICE,
            transfer_ticket: 8,
            game_session_key: key(3),
        };
        assert_eq!(outcome.effects, vec![enter.clone()]);
        assert!(outcome.change_note.contains("event=enter committed"));
        assert_eq!(core.gate_for(ALICE).unwrap().phase, PlayerJoinStage::WelcomedIn { bootstrap_id: 8 });

        let repeat = send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 8,
            link_no: None,
        });
        assert_eq!(repeat.effects, vec![enter]);
        assert!(repeat.change_note.contains("event=repeat applied repeats enter"));
    }

    #[test]
    fn an_apply_for_another_bootstrap_is_a_mismatch() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        let outcome = send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 9,
            link_no: None,
        });
        assert!(!outcome.was_taken);
        assert!(outcome.change_note.contains("event=applied mismatch"));
    }

    #[test]
    fn bootstrap_and_apply_for_strangers_or_stale_matches_are_refused() {
        let mut core = running(3);
        let cases = [
            (
                core.clone().take_event(HostRoundOccurrence::SetupReady {
                    game_session_key: key(2),
                    member: ALICE,
                    transfer_ticket: 1,
                    link_no: None,
                }),
                "stale bootstrap completion",
            ),
            (
                core.clone().take_event(HostRoundOccurrence::SetupReady {
                    game_session_key: key(3),
                    member: BOB,
                    transfer_ticket: 1,
                    link_no: None,
                }),
                "bootstrap for unknown peer",
            ),
            (
                core.clone().take_event(HostRoundOccurrence::TakenIntoEffect {
                    game_session_key: key(2),
                    member: ALICE,
                    transfer_ticket: 1,
                    link_no: None,
                }),
                "stale applied",
            ),
            (
                core.clone().take_event(HostRoundOccurrence::TakenIntoEffect {
                    game_session_key: key(3),
                    member: BOB,
                    transfer_ticket: 1,
                    link_no: None,
                }),
                "applied unknown peer",
            ),
        ];
        for (outcome, reason) in cases {
            assert!(!outcome.was_taken, "{reason}");
            assert!(outcome.change_note.contains(&format!("event={reason} ")), "{reason}");
        }
        send(&mut core, HostRoundOccurrence::MatchEnded { game_session_key: key(3) });
        let late = send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 1,
            link_no: None,
        });
        assert!(late.change_note.contains("event=applied without match"));
    }

    #[test]
    fn an_admitted_peer_gets_no_second_bootstrap() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 8,
            link_no: None,
        });
        let again = send(&mut core, HostRoundOccurrence::SetupReady {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 9,
            link_no: None,
        });
        assert!(!again.was_taken);
        assert!(again.change_note.contains("event=bootstrap after admission ended"));
    }

    #[test]
    fn ending_the_match_clears_everything_and_a_stale_end_is_refused() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        let stale = send(&mut core, HostRoundOccurrence::MatchEnded { game_session_key: key(2) });
        assert!(!stale.was_taken);
        assert!(stale.change_note.contains("event=stale match end"));
        let ended = send(&mut core, HostRoundOccurrence::MatchEnded { game_session_key: key(3) });
        assert!(ended.was_taken);
        assert_eq!(core.phase(), GameRoundStage::None);
        assert!(!core.host_prepared());
        assert!(core.gate_for(ALICE).is_none());
        let twice = send(&mut core, HostRoundOccurrence::MatchEnded { game_session_key: key(3) });
        assert!(!twice.was_taken);
    }

    #[test]
    fn cancelling_a_peer_marks_it_and_asks_for_the_cancel() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        let outcome = send(&mut core, HostRoundOccurrence::PeerCancelled { member: ALICE });
        assert_eq!(
            outcome.effects,
            vec![HostRoundConsequence::cancel(ALICE, EntryRefusalReason::Cancelled, key(3))]
        );
        assert_eq!(core.gate_for(ALICE).unwrap().phase, PlayerJoinStage::Cancelled);
        let stranger = send(&mut core, HostRoundOccurrence::PeerCancelled { member: BOB });
        assert_eq!(stranger.effects.len(), 1);
        assert!(core.gate_for(BOB).is_none());
    }

    #[test]
    fn a_closed_session_refuses_everything() {
        let mut core = running(3);
        let closed = send(&mut core, HostRoundOccurrence::Close);
        assert!(closed.was_taken);
        assert_eq!(core.phase(), GameRoundStage::None);
        let after = send(&mut core, HostRoundOccurrence::Start {
            game_session_key: key(5),
            clock_millis: 1,
        });
        assert!(!after.was_taken);
        assert!(after.change_note.contains("event=session closed"));
        assert!(after.change_note.contains("open=false"));
    }

    #[test]
    fn loading_past_its_deadline_fails_the_load() {
        let mut core = started(3, 1_000);
        let early = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 1_000 + 119_999 });
        assert!(early.effects.is_empty());
        let late = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 1_000 + 120_000 });
        let HostRoundConsequence::Fail { fail } = &late.effects[0] else {
            panic!("expected a failure, got {:?}", late.effects);
        };
        assert_eq!(fail.stage, AbortStep::Load);
        assert_eq!(fail.source, "host load deadline");
        assert_eq!(fail.game_session_key, key(3));
        assert!(late.change_note.contains("failure_source=host load deadline"));
        assert!(late.change_note.contains("elapsed_ms=120000"));
    }

    #[test]
    fn a_silent_authority_fails_the_run_until_it_reports_progress() {
        let mut core = running(3);
        send(&mut core, HostRoundOccurrence::HostAdvance { clock_millis: 30_000 });
        let fine = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 89_999 });
        assert!(fine.effects.is_empty());
        let silent = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 90_000 });
        let HostRoundConsequence::Fail { fail } = &silent.effects[0] else {
            panic!("expected a failure");
        };
        assert_eq!(fail.stage, AbortStep::Authority);
        assert_eq!(fail.source, "authority progress silence");
    }

    #[test]
    fn progress_before_the_run_does_not_count() {
        let mut core = started(3, 1_000);
        send(&mut core, HostRoundOccurrence::HostAdvance { clock_millis: 100_000 });
        let outcome = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 1_000 + 120_000 });
        assert!(matches!(outcome.effects[0], HostRoundConsequence::Fail { .. }));
    }

    #[test]
    fn a_peer_not_admitted_in_time_is_failed_with_the_deadline() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 1_000);
        send(&mut core, HostRoundOccurrence::HostAdvance { clock_millis: 20_000 });
        let early = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 1_000 + 29_999 });
        assert!(early.effects.is_empty());
        send(&mut core, HostRoundOccurrence::HostAdvance { clock_millis: 31_000 });
        let late = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 1_000 + 30_000 });
        assert_eq!(
            late.effects,
            vec![HostRoundConsequence::cancel(ALICE, EntryRefusalReason::CutoffInstant, key(3))]
        );
        assert_eq!(core.gate_for(ALICE).unwrap().phase, PlayerJoinStage::Failed);
    }

    #[test]
    fn an_admitted_peer_never_times_out() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 4);
        send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 4,
            link_no: None,
        });
        send(&mut core, HostRoundOccurrence::HostAdvance { clock_millis: 100_000 });
        let outcome = send(&mut core, HostRoundOccurrence::Tick { clock_millis: 100_001 });
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn the_transition_line_names_the_member_connection_and_bootstrap() {
        let mut core = running(3);
        core.remember_incarnation(6);
        core.remember_load_ticket(Some(12));
        assert_eq!(core.load_ticket(), Some(12));
        admit(&mut core, ALICE, 3, 8);
        let outcome = send(&mut core, HostRoundOccurrence::TakenIntoEffect {
            game_session_key: key(3),
            member: ALICE,
            transfer_ticket: 8,
            link_no: None,
        });
        let line = outcome.change_note;
        assert!(line.contains("session_id=07070707070707070707070707070707"));
        assert!(line.contains("incarnation=6"));
        assert!(line.contains("epoch=3"));
        assert!(line.contains("local_load_id=12"));
        assert!(line.contains("member_id=01010101010101010101010101010101"));
        assert!(line.contains("connection_id=55"));
        assert!(line.contains("bootstrap_id=8"));
        assert!(line.contains("stage_before=open=true,epoch=3,phase=Running,authority=true"));
        assert!(line.contains("elapsed_ms=-"));
    }
}
