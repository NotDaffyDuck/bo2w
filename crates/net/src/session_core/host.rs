//! The host's view of one match: loading, running, and the admission of each peer.
//!
//! Callers feed events in and carry out the effects that come back; nothing here touches the
//! network or the clock.

use std::collections::HashMap;

use frame::MatchKey;
use master_protocol::{AdmissionFailure, MemberId};

use super::progress::{
    FailStage, PEER_ADMISSION_DEADLINE_MS, ProgressPhase, ProgressWatch, SessionFail,
};
use super::rules::{HostWorldReady, match_key_boundary_applies};
use super::transition::format_session_transition;
use crate::transport::bootstrap::epoch_applies;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchPhase {
    None,
    Loading,
    Running,
    Ending,
}

impl MatchPhase {
    /// A match exists and has not finished.
    fn is_live(self) -> bool {
        matches!(self, Self::Loading | Self::Running)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerPhase {
    Waiting,
    Syncing { bootstrap_id: u32 },
    Admitted { bootstrap_id: u32 },
    Failed,
    Cancelled,
}

impl PeerPhase {
    /// Nothing more can happen to this peer's admission.
    fn is_over(self) -> bool {
        matches!(self, Self::Admitted { .. } | Self::Failed | Self::Cancelled)
    }

    /// The peer is still being brought in.
    fn is_pending(self) -> bool {
        matches!(self, Self::Waiting | Self::Syncing { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerAdmission {
    pub phase: PeerPhase,
    pub map_loaded: Option<HostWorldReady>,
    pub started_at_ms: u64,
    pub connection_id: Option<u64>,
}

impl Default for PeerAdmission {
    fn default() -> Self {
        Self {
            phase: PeerPhase::Waiting,
            map_loaded: None,
            started_at_ms: 0,
            connection_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostMatchEvent {
    Start {
        match_key: MatchKey,
        now_ms: u64,
    },
    AuthorityReady {
        ready: HostWorldReady,
        now_ms: u64,
    },
    AuthorityProgress {
        now_ms: u64,
    },
    MapLoaded {
        member: MemberId,
        loaded: HostWorldReady,
        now_ms: u64,
    },
    BootstrapPrepared {
        match_key: MatchKey,
        member: MemberId,
        bootstrap_id: u32,
        connection_id: Option<u64>,
    },
    Applied {
        match_key: MatchKey,
        member: MemberId,
        bootstrap_id: u32,
        connection_id: Option<u64>,
    },
    MatchEnded {
        match_key: MatchKey,
    },
    PeerCancelled {
        member: MemberId,
    },
    Close,
    Tick {
        now_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostMatchEffect {
    PublishReady {
        match_key: MatchKey,
        ready: HostWorldReady,
    },
    PublishEnd {
        match_key: MatchKey,
    },
    FlushBootstraps {
        member: MemberId,
    },
    Enter {
        member: MemberId,
        bootstrap_id: u32,
        match_key: MatchKey,
    },
    CancelPeer {
        member: MemberId,
        reason: AdmissionFailure,
        match_key: MatchKey,
    },
    Fail {
        fail: SessionFail,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMatchApply {
    pub accepted: bool,
    pub effects: Vec<HostMatchEffect>,
    pub transition: String,
}

/// What happened in a handler: why it was accepted with what effects, or why it was refused.
type Verdict = Result<(&'static str, Vec<HostMatchEffect>), &'static str>;

/// The identifiers an event itself carries, for the transition line.
struct EventFacts {
    member: Option<MemberId>,
    bootstrap_id: Option<u32>,
    connection_id: Option<u64>,
    now_ms: Option<u64>,
    /// When the phase being left began; the elapsed time is counted from it.
    phase_began_ms: u64,
}

impl EventFacts {
    fn of(event: &HostMatchEvent, phase_began_ms: u64) -> Self {
        use HostMatchEvent as E;
        let (member, bootstrap_id, connection_id, now_ms) = match event {
            E::Start { now_ms, .. }
            | E::AuthorityReady { now_ms, .. }
            | E::AuthorityProgress { now_ms }
            | E::Tick { now_ms } => (None, None, None, Some(*now_ms)),
            E::MapLoaded { member, now_ms, .. } => (Some(*member), None, None, Some(*now_ms)),
            E::BootstrapPrepared {
                member,
                bootstrap_id,
                connection_id,
                ..
            }
            | E::Applied {
                member,
                bootstrap_id,
                connection_id,
                ..
            } => (Some(*member), Some(*bootstrap_id), *connection_id, None),
            E::PeerCancelled { member } => (Some(*member), None, None, None),
            E::MatchEnded { .. } | E::Close => (None, None, None, None),
        };
        Self {
            member,
            bootstrap_id,
            connection_id,
            now_ms,
            phase_began_ms,
        }
    }
}

/// Checks a peer's loaded world against the host's once both are known.
///
/// Returns nothing while the peer is past caring or has not loaded the right epoch. A mismatch
/// fails the peer and asks for it to be cancelled; a match asks for its bootstraps.
fn judge_peer(
    match_key: MatchKey,
    host: HostWorldReady,
    member: MemberId,
    peer: &mut PeerAdmission,
) -> Option<HostMatchEffect> {
    if peer.phase.is_over() {
        return None;
    }
    let loaded = peer.map_loaded?;
    if !epoch_applies(loaded.epoch, match_key.match_epoch) {
        return None;
    }
    let mismatch = if host.map != loaded.map {
        Some(AdmissionFailure::MapContent {
            host: host.map,
            peer: loaded.map,
        })
    } else if host.weapons != loaded.weapons {
        Some(AdmissionFailure::WeaponRegistry {
            host: host.weapons,
            peer: loaded.weapons,
        })
    } else if host.classes != loaded.classes {
        Some(AdmissionFailure::ClassRegistry {
            host: host.classes,
            peer: loaded.classes,
        })
    } else {
        None
    };
    match mismatch {
        Some(reason) => {
            peer.phase = PeerPhase::Failed;
            Some(HostMatchEffect::CancelPeer {
                member,
                reason,
                match_key,
            })
        }
        None => Some(HostMatchEffect::FlushBootstraps { member }),
    }
}

#[derive(Clone, Debug)]
pub struct HostMatchCore {
    session_open: bool,
    match_key: MatchKey,
    phase: MatchPhase,
    host_world: Option<HostWorldReady>,
    authority_ready: bool,
    peers: HashMap<MemberId, PeerAdmission>,
    progress: ProgressWatch,
    incarnation: u64,
    local_load_id: Option<u64>,
}

impl Default for HostMatchCore {
    fn default() -> Self {
        Self {
            session_open: true,
            match_key: MatchKey::NONE,
            phase: MatchPhase::None,
            host_world: None,
            authority_ready: false,
            peers: HashMap::new(),
            progress: ProgressWatch::default(),
            incarnation: 0,
            local_load_id: None,
        }
    }
}

impl HostMatchCore {
    pub fn match_key(&self) -> MatchKey {
        self.match_key
    }

    pub fn phase(&self) -> MatchPhase {
        self.phase
    }

    pub fn host_world(&self) -> Option<HostWorldReady> {
        self.host_world
    }

    pub fn authority_ready(&self) -> bool {
        self.authority_ready
    }

    pub fn peer(&self, member: MemberId) -> Option<&PeerAdmission> {
        self.peers.get(&member)
    }

    pub fn set_incarnation(&mut self, incarnation: u64) {
        self.incarnation = incarnation;
    }

    pub fn set_local_load_id(&mut self, local_load_id: Option<u64>) {
        self.local_load_id = local_load_id;
    }

    pub fn local_load_id(&self) -> Option<u64> {
        self.local_load_id
    }

    /// Takes one event. The reply says whether it was accepted, what to do about it, and carries
    /// the transition line for the log.
    pub fn apply(&mut self, event: HostMatchEvent) -> HostMatchApply {
        let before = self.stage_label();
        let facts = EventFacts::of(&event, self.progress.phase_started_at());
        let verdict = if self.session_open {
            self.dispatch(event)
        } else {
            Err("session closed")
        };
        match verdict {
            Ok((reason, effects)) => self.conclude(&facts, before, reason, effects, true),
            Err(reason) => self.conclude(&facts, before, reason, Vec::new(), false),
        }
    }

    fn dispatch(&mut self, event: HostMatchEvent) -> Verdict {
        match event {
            HostMatchEvent::Start { match_key, now_ms } => self.on_start(match_key, now_ms),
            HostMatchEvent::AuthorityReady { ready, now_ms } => {
                self.on_authority_ready(ready, now_ms)
            }
            HostMatchEvent::AuthorityProgress { now_ms } => {
                if self.phase == MatchPhase::Running {
                    self.progress.beat(now_ms);
                }
                Ok(("authority progress", Vec::new()))
            }
            HostMatchEvent::MapLoaded {
                member,
                loaded,
                now_ms,
            } => self.on_map_loaded(member, loaded, now_ms),
            HostMatchEvent::BootstrapPrepared {
                match_key,
                member,
                bootstrap_id,
                connection_id,
            } => self.on_bootstrap_prepared(match_key, member, bootstrap_id, connection_id),
            HostMatchEvent::Applied {
                match_key,
                member,
                bootstrap_id,
                connection_id,
            } => self.on_applied(match_key, member, bootstrap_id, connection_id),
            HostMatchEvent::MatchEnded { match_key } => self.on_match_ended(match_key),
            HostMatchEvent::PeerCancelled { member } => {
                if let Some(peer) = self.peers.get_mut(&member) {
                    peer.phase = PeerPhase::Cancelled;
                }
                let cancel = HostMatchEffect::CancelPeer {
                    member,
                    reason: AdmissionFailure::Cancelled,
                    match_key: self.match_key,
                };
                Ok(("peer cancelled", vec![cancel]))
            }
            HostMatchEvent::Close => {
                self.session_open = false;
                self.phase = MatchPhase::None;
                Ok(("close session", Vec::new()))
            }
            HostMatchEvent::Tick { now_ms } => Ok(("tick", self.on_tick(now_ms))),
        }
    }

    fn on_start(&mut self, match_key: MatchKey, now_ms: u64) -> Verdict {
        if match_key.is_none() {
            return Err("start without match key");
        }
        let same_key = self.match_key == match_key;
        if same_key && self.phase == MatchPhase::Loading {
            return Ok(("start already loading", Vec::new()));
        }
        if same_key && self.phase == MatchPhase::Running {
            return Ok(("start already running", Vec::new()));
        }
        self.match_key = match_key;
        self.phase = MatchPhase::Loading;
        self.host_world = None;
        self.authority_ready = false;
        self.peers.clear();
        self.progress.set_phase(ProgressPhase::Loading, now_ms);
        Ok(("start loading", Vec::new()))
    }

    fn on_authority_ready(&mut self, ready: HostWorldReady, now_ms: u64) -> Verdict {
        if !epoch_applies(ready.epoch, self.match_key.match_epoch) {
            return Err("stale host world");
        }
        if !self.phase.is_live() {
            return Err("host world without match");
        }
        self.host_world = Some(ready);
        self.authority_ready = true;
        if self.phase == MatchPhase::Loading {
            self.phase = MatchPhase::Running;
            self.progress.set_phase(ProgressPhase::Running, now_ms);
            for peer in self.peers.values_mut() {
                peer.started_at_ms = now_ms;
            }
        } else {
            self.progress.beat(now_ms);
        }
        let key = self.match_key;
        let effects = self
            .peers
            .iter_mut()
            .filter_map(|(member, peer)| judge_peer(key, ready, *member, peer))
            .collect();
        Ok(("authority ready", effects))
    }

    fn on_map_loaded(&mut self, member: MemberId, loaded: HostWorldReady, now_ms: u64) -> Verdict {
        if !epoch_applies(loaded.epoch, self.match_key.match_epoch) || !self.phase.is_live() {
            return Err("stale map loaded");
        }
        let running = self.phase == MatchPhase::Running;
        let key = self.match_key;
        let host = self.host_world;
        let peer = self.peers.entry(member).or_default();
        if matches!(peer.phase, PeerPhase::Failed | PeerPhase::Cancelled) {
            return Err("map loaded after admission ended");
        }
        if running && peer.started_at_ms == 0 {
            peer.started_at_ms = now_ms;
        }
        peer.map_loaded = Some(loaded);
        let effects = host
            .and_then(|host| judge_peer(key, host, member, peer))
            .into_iter()
            .collect();
        Ok(("client map loaded", effects))
    }

    fn on_bootstrap_prepared(
        &mut self,
        match_key: MatchKey,
        member: MemberId,
        bootstrap_id: u32,
        connection_id: Option<u64>,
    ) -> Verdict {
        if match_key != self.match_key || match_key.is_none() || !self.phase.is_live() {
            return Err("stale bootstrap completion");
        }
        let Some(peer) = self.peers.get_mut(&member) else {
            return Err("bootstrap for unknown peer");
        };
        if peer.phase.is_over() {
            return Err("bootstrap after admission ended");
        }
        if connection_id.is_some() {
            peer.connection_id = connection_id;
        }
        peer.phase = PeerPhase::Syncing { bootstrap_id };
        Ok(("bootstrap prepared", Vec::new()))
    }

    fn on_applied(
        &mut self,
        match_key: MatchKey,
        member: MemberId,
        bootstrap_id: u32,
        connection_id: Option<u64>,
    ) -> Verdict {
        if match_key != self.match_key || match_key.is_none() {
            return Err("stale applied");
        }
        if !self.phase.is_live() {
            return Err("applied without match");
        }
        let Some(peer) = self.peers.get_mut(&member) else {
            return Err("applied unknown peer");
        };
        let repeat = match peer.phase {
            PeerPhase::Syncing { bootstrap_id: due } if due == bootstrap_id => false,
            PeerPhase::Admitted { bootstrap_id: done } if done == bootstrap_id => true,
            _ => return Err("applied mismatch"),
        };
        if connection_id.is_some() {
            peer.connection_id = connection_id;
        }
        peer.phase = PeerPhase::Admitted { bootstrap_id };
        let reason = if repeat {
            "duplicate applied repeats enter"
        } else {
            "enter committed"
        };
        let enter = HostMatchEffect::Enter {
            member,
            bootstrap_id,
            match_key,
        };
        Ok((reason, vec![enter]))
    }

    fn on_match_ended(&mut self, match_key: MatchKey) -> Verdict {
        if !match_key_boundary_applies(match_key, self.match_key) || !self.phase.is_live() {
            return Err("stale match end");
        }
        self.phase = MatchPhase::None;
        self.authority_ready = false;
        self.host_world = None;
        self.peers.clear();
        self.progress.set_phase(ProgressPhase::Idle, 0);
        Ok(("end match", Vec::new()))
    }

    fn on_tick(&mut self, now_ms: u64) -> Vec<HostMatchEffect> {
        let mut effects = Vec::new();
        if self.progress.expired(now_ms) {
            let (stage, source) = if self.phase == MatchPhase::Running {
                (FailStage::Authority, "authority progress silence")
            } else {
                (FailStage::Load, "host load deadline")
            };
            effects.push(HostMatchEffect::Fail {
                fail: SessionFail {
                    stage,
                    source: source.into(),
                    match_key: self.match_key,
                },
            });
        }
        if self.phase == MatchPhase::Running {
            let key = self.match_key;
            for (member, peer) in &mut self.peers {
                let waited = now_ms.saturating_sub(peer.started_at_ms);
                if peer.phase.is_pending()
                    && peer.started_at_ms != 0
                    && waited >= PEER_ADMISSION_DEADLINE_MS
                {
                    peer.phase = PeerPhase::Failed;
                    effects.push(HostMatchEffect::CancelPeer {
                        member: *member,
                        reason: AdmissionFailure::Deadline,
                        match_key: key,
                    });
                }
            }
        }
        effects
    }

    fn stage_label(&self) -> String {
        format!(
            "open={},epoch={},phase={:?},authority={}",
            self.session_open, self.match_key.match_epoch, self.phase, self.authority_ready
        )
    }

    /// Builds the reply, filling the identifiers the event lacked from the effects and the peer.
    fn conclude(
        &self,
        facts: &EventFacts,
        before: String,
        reason: &'static str,
        effects: Vec<HostMatchEffect>,
        accepted: bool,
    ) -> HostMatchApply {
        let member = facts.member.or_else(|| {
            effects.iter().find_map(|effect| match effect {
                HostMatchEffect::FlushBootstraps { member }
                | HostMatchEffect::Enter { member, .. }
                | HostMatchEffect::CancelPeer { member, .. } => Some(*member),
                HostMatchEffect::Fail { .. }
                | HostMatchEffect::PublishReady { .. }
                | HostMatchEffect::PublishEnd { .. } => None,
            })
        });
        let bootstrap_id = facts.bootstrap_id.or_else(|| {
            effects.iter().find_map(|effect| match effect {
                HostMatchEffect::Enter { bootstrap_id, .. } => Some(*bootstrap_id),
                _ => None,
            })
        });
        let connection_id = facts.connection_id.or_else(|| {
            let peer = self.peers.get(&member?)?;
            peer.connection_id
        });
        let elapsed_ms = facts
            .now_ms
            .map(|now| now.saturating_sub(facts.phase_began_ms));
        let failure_source = effects
            .iter()
            .find_map(|effect| match effect {
                HostMatchEffect::Fail { fail } => Some(fail.source.as_str()),
                _ => None,
            })
            .unwrap_or("-");
        let transition = format_session_transition(
            self.match_key.session_id,
            (self.incarnation != 0).then_some(self.incarnation),
            self.match_key.match_epoch,
            self.local_load_id,
            member.map(|id| id.0),
            connection_id,
            bootstrap_id,
            &before,
            reason,
            &self.stage_label(),
            elapsed_ms,
            failure_source,
        );
        HostMatchApply {
            accepted,
            effects,
            transition,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: [u8; 16] = [7; 16];
    const ALICE: MemberId = MemberId([1; 16]);
    const BOB: MemberId = MemberId([2; 16]);

    fn key(epoch: u32) -> MatchKey {
        MatchKey::new(SESSION, epoch)
    }

    fn world(epoch: u32) -> HostWorldReady {
        HostWorldReady {
            epoch,
            map: 10,
            weapons: 20,
            classes: 30,
        }
    }

    fn started(epoch: u32, at: u64) -> HostMatchCore {
        let mut core = HostMatchCore::default();
        let applied = core.apply(HostMatchEvent::Start {
            match_key: key(epoch),
            now_ms: at,
        });
        assert!(applied.accepted);
        core
    }

    fn running(epoch: u32) -> HostMatchCore {
        let mut core = started(epoch, 100);
        core.apply(HostMatchEvent::AuthorityReady {
            ready: world(epoch),
            now_ms: 200,
        });
        core
    }

    fn loaded(core: &mut HostMatchCore, member: MemberId, loaded: HostWorldReady, now_ms: u64) -> HostMatchApply {
        core.apply(HostMatchEvent::MapLoaded {
            member,
            loaded,
            now_ms,
        })
    }

    fn admit(core: &mut HostMatchCore, member: MemberId, epoch: u32, bootstrap_id: u32) {
        loaded(core, member, world(epoch), 300);
        core.apply(HostMatchEvent::BootstrapPrepared {
            match_key: key(epoch),
            member,
            bootstrap_id,
            connection_id: Some(55),
        });
    }

    #[test]
    fn a_start_moves_from_none_to_loading() {
        let core = started(3, 100);
        assert_eq!(core.phase(), MatchPhase::Loading);
        assert_eq!(core.match_key(), key(3));
        assert!(!core.authority_ready());
        assert_eq!(core.host_world(), None);
    }

    #[test]
    fn a_start_without_a_key_is_refused() {
        let mut core = HostMatchCore::default();
        let applied = core.apply(HostMatchEvent::Start {
            match_key: MatchKey::NONE,
            now_ms: 5,
        });
        assert!(!applied.accepted);
        assert!(applied.transition.contains("event=start without match key"));
        assert_eq!(core.phase(), MatchPhase::None);
    }

    #[test]
    fn starting_the_same_match_twice_changes_nothing() {
        let mut core = started(3, 100);
        let again = core.apply(HostMatchEvent::Start {
            match_key: key(3),
            now_ms: 999,
        });
        assert!(again.accepted);
        assert!(again.transition.contains("event=start already loading"));
        let mut core = running(3);
        let again = core.apply(HostMatchEvent::Start {
            match_key: key(3),
            now_ms: 999,
        });
        assert!(again.transition.contains("event=start already running"));
        assert_eq!(core.phase(), MatchPhase::Running);
    }

    #[test]
    fn a_new_match_replaces_the_old_one_and_drops_its_peers() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        core.apply(HostMatchEvent::Start {
            match_key: key(4),
            now_ms: 400,
        });
        assert_eq!(core.phase(), MatchPhase::Loading);
        assert!(core.peer(ALICE).is_none());
        assert!(!core.authority_ready());
    }

    #[test]
    fn the_authority_being_ready_starts_the_run() {
        let mut core = started(3, 100);
        let applied = core.apply(HostMatchEvent::AuthorityReady {
            ready: world(3),
            now_ms: 250,
        });
        assert!(applied.accepted);
        assert_eq!(core.phase(), MatchPhase::Running);
        assert!(core.authority_ready());
        assert_eq!(core.host_world(), Some(world(3)));
    }

    #[test]
    fn a_world_for_another_epoch_or_no_match_is_refused() {
        let mut core = started(3, 100);
        let stale = core.apply(HostMatchEvent::AuthorityReady {
            ready: world(2),
            now_ms: 1,
        });
        assert!(!stale.accepted);
        assert!(stale.transition.contains("event=stale host world"));
        let mut idle = HostMatchCore::default();
        let none = idle.apply(HostMatchEvent::AuthorityReady {
            ready: world(0),
            now_ms: 1,
        });
        assert!(none.transition.contains("event=stale host world"));
    }

    #[test]
    fn a_peer_that_loaded_before_the_host_is_flushed_when_the_host_is_ready() {
        let mut core = started(3, 100);
        let early = loaded(&mut core, ALICE, world(3), 120);
        assert!(early.accepted);
        assert!(early.effects.is_empty());
        let ready = core.apply(HostMatchEvent::AuthorityReady {
            ready: world(3),
            now_ms: 200,
        });
        assert_eq!(
            ready.effects,
            vec![HostMatchEffect::FlushBootstraps { member: ALICE }]
        );
        assert_eq!(core.peer(ALICE).unwrap().started_at_ms, 200);
    }

    #[test]
    fn a_peer_loading_after_the_host_is_flushed_at_once() {
        let mut core = running(3);
        let applied = loaded(&mut core, ALICE, world(3), 300);
        assert_eq!(
            applied.effects,
            vec![HostMatchEffect::FlushBootstraps { member: ALICE }]
        );
        assert_eq!(core.peer(ALICE).unwrap().started_at_ms, 300);
    }

    #[test]
    fn a_peer_with_other_content_is_cancelled_for_the_first_difference() {
        let cases: [(HostWorldReady, AdmissionFailure); 3] = [
            (
                HostWorldReady { map: 11, ..world(3) },
                AdmissionFailure::MapContent { host: 10, peer: 11 },
            ),
            (
                HostWorldReady { weapons: 21, ..world(3) },
                AdmissionFailure::WeaponRegistry { host: 20, peer: 21 },
            ),
            (
                HostWorldReady { classes: 31, ..world(3) },
                AdmissionFailure::ClassRegistry { host: 30, peer: 31 },
            ),
        ];
        for (peer_world, reason) in cases {
            let mut core = running(3);
            let applied = loaded(&mut core, ALICE, peer_world, 300);
            assert_eq!(
                applied.effects,
                vec![HostMatchEffect::CancelPeer {
                    member: ALICE,
                    reason,
                    match_key: key(3),
                }]
            );
            assert_eq!(core.peer(ALICE).unwrap().phase, PeerPhase::Failed);
        }
    }

    #[test]
    fn a_map_loaded_for_the_wrong_epoch_is_refused() {
        let mut core = running(3);
        let applied = loaded(&mut core, ALICE, world(2), 300);
        assert!(!applied.accepted);
        assert!(applied.transition.contains("event=stale map loaded"));
        assert!(core.peer(ALICE).is_none());
    }

    #[test]
    fn a_failed_peer_cannot_load_again() {
        let mut core = running(3);
        loaded(&mut core, ALICE, HostWorldReady { map: 99, ..world(3) }, 300);
        let again = loaded(&mut core, ALICE, world(3), 310);
        assert!(!again.accepted);
        assert!(again.transition.contains("event=map loaded after admission ended"));
    }

    #[test]
    fn the_bootstrap_and_apply_steps_admit_a_peer_once_and_repeat_harmlessly() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        assert_eq!(core.peer(ALICE).unwrap().phase, PeerPhase::Syncing { bootstrap_id: 8 });
        assert_eq!(core.peer(ALICE).unwrap().connection_id, Some(55));

        let applied = core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 8,
            connection_id: None,
        });
        let enter = HostMatchEffect::Enter {
            member: ALICE,
            bootstrap_id: 8,
            match_key: key(3),
        };
        assert_eq!(applied.effects, vec![enter.clone()]);
        assert!(applied.transition.contains("event=enter committed"));
        assert_eq!(core.peer(ALICE).unwrap().phase, PeerPhase::Admitted { bootstrap_id: 8 });

        let repeat = core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 8,
            connection_id: None,
        });
        assert_eq!(repeat.effects, vec![enter]);
        assert!(repeat.transition.contains("event=duplicate applied repeats enter"));
    }

    #[test]
    fn an_apply_for_another_bootstrap_is_a_mismatch() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        let applied = core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 9,
            connection_id: None,
        });
        assert!(!applied.accepted);
        assert!(applied.transition.contains("event=applied mismatch"));
    }

    #[test]
    fn bootstrap_and_apply_for_strangers_or_stale_matches_are_refused() {
        let mut core = running(3);
        let cases = [
            (
                core.clone().apply(HostMatchEvent::BootstrapPrepared {
                    match_key: key(2),
                    member: ALICE,
                    bootstrap_id: 1,
                    connection_id: None,
                }),
                "stale bootstrap completion",
            ),
            (
                core.clone().apply(HostMatchEvent::BootstrapPrepared {
                    match_key: key(3),
                    member: BOB,
                    bootstrap_id: 1,
                    connection_id: None,
                }),
                "bootstrap for unknown peer",
            ),
            (
                core.clone().apply(HostMatchEvent::Applied {
                    match_key: key(2),
                    member: ALICE,
                    bootstrap_id: 1,
                    connection_id: None,
                }),
                "stale applied",
            ),
            (
                core.clone().apply(HostMatchEvent::Applied {
                    match_key: key(3),
                    member: BOB,
                    bootstrap_id: 1,
                    connection_id: None,
                }),
                "applied unknown peer",
            ),
        ];
        for (applied, reason) in cases {
            assert!(!applied.accepted, "{reason}");
            assert!(applied.transition.contains(&format!("event={reason} ")), "{reason}");
        }
        core.apply(HostMatchEvent::MatchEnded { match_key: key(3) });
        let late = core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 1,
            connection_id: None,
        });
        assert!(late.transition.contains("event=applied without match"));
    }

    #[test]
    fn an_admitted_peer_gets_no_second_bootstrap() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 8);
        core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 8,
            connection_id: None,
        });
        let again = core.apply(HostMatchEvent::BootstrapPrepared {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 9,
            connection_id: None,
        });
        assert!(!again.accepted);
        assert!(again.transition.contains("event=bootstrap after admission ended"));
    }

    #[test]
    fn ending_the_match_clears_everything_and_a_stale_end_is_refused() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        let stale = core.apply(HostMatchEvent::MatchEnded { match_key: key(2) });
        assert!(!stale.accepted);
        assert!(stale.transition.contains("event=stale match end"));
        let ended = core.apply(HostMatchEvent::MatchEnded { match_key: key(3) });
        assert!(ended.accepted);
        assert_eq!(core.phase(), MatchPhase::None);
        assert!(!core.authority_ready());
        assert!(core.peer(ALICE).is_none());
        let twice = core.apply(HostMatchEvent::MatchEnded { match_key: key(3) });
        assert!(!twice.accepted);
    }

    #[test]
    fn cancelling_a_peer_marks_it_and_asks_for_the_cancel() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 300);
        let applied = core.apply(HostMatchEvent::PeerCancelled { member: ALICE });
        assert_eq!(
            applied.effects,
            vec![HostMatchEffect::CancelPeer {
                member: ALICE,
                reason: AdmissionFailure::Cancelled,
                match_key: key(3),
            }]
        );
        assert_eq!(core.peer(ALICE).unwrap().phase, PeerPhase::Cancelled);
        let stranger = core.apply(HostMatchEvent::PeerCancelled { member: BOB });
        assert_eq!(stranger.effects.len(), 1);
        assert!(core.peer(BOB).is_none());
    }

    #[test]
    fn a_closed_session_refuses_everything() {
        let mut core = running(3);
        let closed = core.apply(HostMatchEvent::Close);
        assert!(closed.accepted);
        assert_eq!(core.phase(), MatchPhase::None);
        let after = core.apply(HostMatchEvent::Start {
            match_key: key(5),
            now_ms: 1,
        });
        assert!(!after.accepted);
        assert!(after.transition.contains("event=session closed"));
        assert!(after.transition.contains("open=false"));
    }

    #[test]
    fn loading_past_its_deadline_fails_the_load() {
        let mut core = started(3, 1_000);
        let early = core.apply(HostMatchEvent::Tick { now_ms: 1_000 + 119_999 });
        assert!(early.effects.is_empty());
        let late = core.apply(HostMatchEvent::Tick { now_ms: 1_000 + 120_000 });
        let HostMatchEffect::Fail { fail } = &late.effects[0] else {
            panic!("expected a failure, got {:?}", late.effects);
        };
        assert_eq!(fail.stage, FailStage::Load);
        assert_eq!(fail.source, "host load deadline");
        assert_eq!(fail.match_key, key(3));
        assert!(late.transition.contains("failure_source=host load deadline"));
        assert!(late.transition.contains("elapsed_ms=120000"));
    }

    #[test]
    fn a_silent_authority_fails_the_run_until_it_reports_progress() {
        let mut core = running(3);
        core.apply(HostMatchEvent::AuthorityProgress { now_ms: 30_000 });
        let fine = core.apply(HostMatchEvent::Tick { now_ms: 89_999 });
        assert!(fine.effects.is_empty());
        let silent = core.apply(HostMatchEvent::Tick { now_ms: 90_000 });
        let HostMatchEffect::Fail { fail } = &silent.effects[0] else {
            panic!("expected a failure");
        };
        assert_eq!(fail.stage, FailStage::Authority);
        assert_eq!(fail.source, "authority progress silence");
    }

    #[test]
    fn progress_before_the_run_does_not_count() {
        let mut core = started(3, 1_000);
        core.apply(HostMatchEvent::AuthorityProgress { now_ms: 100_000 });
        let applied = core.apply(HostMatchEvent::Tick { now_ms: 1_000 + 120_000 });
        assert!(matches!(applied.effects[0], HostMatchEffect::Fail { .. }));
    }

    #[test]
    fn a_peer_not_admitted_in_time_is_failed_with_the_deadline() {
        let mut core = running(3);
        loaded(&mut core, ALICE, world(3), 1_000);
        core.apply(HostMatchEvent::AuthorityProgress { now_ms: 20_000 });
        let early = core.apply(HostMatchEvent::Tick { now_ms: 1_000 + 29_999 });
        assert!(early.effects.is_empty());
        core.apply(HostMatchEvent::AuthorityProgress { now_ms: 31_000 });
        let late = core.apply(HostMatchEvent::Tick { now_ms: 1_000 + 30_000 });
        assert_eq!(
            late.effects,
            vec![HostMatchEffect::CancelPeer {
                member: ALICE,
                reason: AdmissionFailure::Deadline,
                match_key: key(3),
            }]
        );
        assert_eq!(core.peer(ALICE).unwrap().phase, PeerPhase::Failed);
    }

    #[test]
    fn an_admitted_peer_never_times_out() {
        let mut core = running(3);
        admit(&mut core, ALICE, 3, 4);
        core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 4,
            connection_id: None,
        });
        core.apply(HostMatchEvent::AuthorityProgress { now_ms: 100_000 });
        let applied = core.apply(HostMatchEvent::Tick { now_ms: 100_001 });
        assert!(applied.effects.is_empty());
    }

    #[test]
    fn the_transition_line_names_the_member_connection_and_bootstrap() {
        let mut core = running(3);
        core.set_incarnation(6);
        core.set_local_load_id(Some(12));
        assert_eq!(core.local_load_id(), Some(12));
        admit(&mut core, ALICE, 3, 8);
        let applied = core.apply(HostMatchEvent::Applied {
            match_key: key(3),
            member: ALICE,
            bootstrap_id: 8,
            connection_id: None,
        });
        let line = applied.transition;
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
