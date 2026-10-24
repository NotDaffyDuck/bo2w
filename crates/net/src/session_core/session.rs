//! The lobby session as a client sees it: the host's authoritative state, newest wins, and the
//! match phase that follows from it.

use master_protocol::MemberId;

use super::host::MatchPhase;
use super::rules::match_boundary_applies;
use super::transition::format_session_transition;
use crate::transport::bootstrap::epoch_applies;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    AuthoritativeState {
        session_id: [u8; 16],
        member: MemberId,
        revision: u64,
        match_epoch: u32,
        in_match: bool,
        start_nonce: u32,
        map: String,
        mode: String,
        members: Vec<MemberId>,
        skip_votes: u8,
    },
    EnterMatch {
        epoch: u32,
        bootstrap_id: u32,
    },
    MatchEnded {
        epoch: u32,
    },
    CloseSession,
}

impl SessionEvent {
    /// The name the event goes by in the transition line.
    fn label(&self) -> &'static str {
        match self {
            Self::AuthoritativeState { .. } => "state",
            Self::EnterMatch { .. } => "enter committed",
            Self::MatchEnded { .. } => "end-match",
            Self::CloseSession => "close-session",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionView {
    pub map: String,
    pub mode: String,
    pub members: Vec<MemberId>,
    pub skip_votes: u8,
    pub start_nonce: u32,
    pub in_match: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionApply {
    pub view: SessionView,
    pub changed: bool,
    pub enter_bootstrap: Option<u32>,
    pub closed: bool,
    pub transition: String,
}

/// How an event ended: what it changed, and for a refusal why.
struct Outcome {
    changed: bool,
    enter_bootstrap: Option<u32>,
    failure_source: &'static str,
}

impl Outcome {
    fn changed() -> Self {
        Self {
            changed: true,
            enter_bootstrap: None,
            failure_source: "-",
        }
    }

    fn unchanged() -> Self {
        Self {
            changed: false,
            enter_bootstrap: None,
            failure_source: "-",
        }
    }

    fn refused(why: &'static str) -> Self {
        Self {
            failure_source: why,
            ..Self::unchanged()
        }
    }
}

const NO_ID: [u8; 16] = [0; 16];

#[derive(Clone, Debug)]
pub struct SessionCore {
    session_open: bool,
    /// Epoch and revision of the newest state taken.
    applied_state: Option<(u32, u64)>,
    view: SessionView,
    match_phase: MatchPhase,
    session_id: [u8; 16],
    incarnation: u64,
    local_member: Option<MemberId>,
}

impl Default for SessionCore {
    fn default() -> Self {
        Self {
            session_open: true,
            applied_state: None,
            view: SessionView::default(),
            match_phase: MatchPhase::None,
            session_id: NO_ID,
            incarnation: 0,
            local_member: None,
        }
    }
}

impl SessionCore {
    /// A core for a connection whose session is already known.
    pub fn for_connection(session_id: [u8; 16], incarnation: u64) -> Self {
        Self {
            session_id,
            incarnation,
            ..Self::default()
        }
    }

    pub fn view(&self) -> &SessionView {
        &self.view
    }

    pub fn match_phase(&self) -> MatchPhase {
        self.match_phase
    }

    pub fn apply(&mut self, event: SessionEvent) -> SessionApply {
        let before = self.stage_label();
        let label = event.label();
        let outcome = if self.session_open {
            match event {
                SessionEvent::AuthoritativeState {
                    session_id,
                    member,
                    revision,
                    match_epoch,
                    in_match,
                    start_nonce,
                    map,
                    mode,
                    members,
                    skip_votes,
                } => {
                    let identity = StateIdentity {
                        session_id,
                        member,
                        match_epoch,
                        in_match,
                        start_nonce,
                    };
                    let view = SessionView {
                        map,
                        mode,
                        members,
                        skip_votes,
                        start_nonce,
                        in_match,
                    };
                    self.take_state(identity, revision, view)
                }
                SessionEvent::EnterMatch {
                    epoch,
                    bootstrap_id,
                } => self.enter(epoch, bootstrap_id),
                SessionEvent::MatchEnded { epoch } => self.end(epoch),
                SessionEvent::CloseSession => self.close(),
            }
        } else {
            Outcome::refused("session closed")
        };
        self.reply(before, label, outcome)
    }

    fn take_state(&mut self, identity: StateIdentity, revision: u64, view: SessionView) -> Outcome {
        if !self.identity_fits(&identity) {
            return Outcome::refused("state identity mismatch");
        }
        if !state_is_newer(self.applied_state, revision, identity.match_epoch) {
            return Outcome::refused("stale state");
        }
        let same_match = self.view.in_match && self.view.start_nonce == identity.match_epoch;
        self.session_id = identity.session_id;
        self.local_member = Some(identity.member);
        self.applied_state = Some((identity.match_epoch, revision));
        self.match_phase = match (view.in_match, same_match, self.match_phase) {
            (false, _, _) => MatchPhase::None,
            (true, true, MatchPhase::Running) => MatchPhase::Running,
            (true, _, _) => MatchPhase::Loading,
        };
        self.view = view;
        Outcome::changed()
    }

    /// The state names a real session and member, agrees with what we already know, and its
    /// epoch is consistent with whether a match is on.
    fn identity_fits(&self, state: &StateIdentity) -> bool {
        let session_ok = state.session_id != NO_ID
            && (self.session_id == NO_ID || self.session_id == state.session_id);
        let member_ok = state.member.0 != NO_ID
            && self.local_member.is_none_or(|local| local == state.member);
        let epoch_ok = state.start_nonce == state.match_epoch
            && !(state.in_match && state.match_epoch == 0);
        session_ok && member_ok && epoch_ok
    }

    fn enter(&mut self, epoch: u32, bootstrap_id: u32) -> Outcome {
        if !(epoch_applies(epoch, self.view.start_nonce) && self.view.in_match) {
            return Outcome::refused("enter epoch mismatch");
        }
        self.match_phase = MatchPhase::Running;
        Outcome {
            enter_bootstrap: Some(bootstrap_id),
            ..Outcome::unchanged()
        }
    }

    fn end(&mut self, epoch: u32) -> Outcome {
        if !match_boundary_applies(epoch, self.view.start_nonce) {
            return Outcome::refused("stale match end");
        }
        if !self.view.in_match {
            return Outcome::unchanged();
        }
        self.view.in_match = false;
        self.match_phase = MatchPhase::None;
        Outcome::changed()
    }

    fn close(&mut self) -> Outcome {
        self.session_open = false;
        self.view.in_match = false;
        self.match_phase = MatchPhase::None;
        Outcome::changed()
    }

    fn stage_label(&self) -> String {
        format!(
            "open={},epoch={},in_match={},phase={:?},rev={:?}",
            self.session_open,
            self.view.start_nonce,
            self.view.in_match,
            self.match_phase,
            self.applied_state
        )
    }

    fn reply(&self, before: String, event: &'static str, outcome: Outcome) -> SessionApply {
        let transition = format_session_transition(
            self.session_id,
            (self.incarnation != 0).then_some(self.incarnation),
            self.view.start_nonce,
            None,
            self.local_member.map(|member| member.0),
            None,
            outcome.enter_bootstrap,
            &before,
            event,
            &self.stage_label(),
            None,
            outcome.failure_source,
        );
        SessionApply {
            view: self.view.clone(),
            changed: outcome.changed,
            enter_bootstrap: outcome.enter_bootstrap,
            closed: !self.session_open,
            transition,
        }
    }
}

/// The parts of an authoritative state that say whose it is and which match it speaks of.
struct StateIdentity {
    session_id: [u8; 16],
    member: MemberId,
    match_epoch: u32,
    in_match: bool,
    start_nonce: u32,
}

/// Whether a state at `revision` in `epoch` supersedes the one applied. A different epoch wins
/// when it lies in the half-circle ahead; within an epoch the revision must differ and lie in the
/// half-circle ahead. Both counters wrap.
pub fn state_is_newer(applied: Option<(u32, u64)>, revision: u64, epoch: u32) -> bool {
    let Some((prev_epoch, prev_revision)) = applied else {
        return true;
    };
    if epoch != prev_epoch {
        return epoch.wrapping_sub(prev_epoch) < 1 << 31;
    }
    revision != prev_revision && revision.wrapping_sub(prev_revision) < 1 << 63
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: [u8; 16] = [4; 16];
    const ME: MemberId = MemberId([5; 16]);
    const OTHER: MemberId = MemberId([6; 16]);

    fn state(revision: u64, epoch: u32, in_match: bool) -> SessionEvent {
        SessionEvent::AuthoritativeState {
            session_id: SESSION,
            member: ME,
            revision,
            match_epoch: epoch,
            in_match,
            start_nonce: epoch,
            map: "mp_nuketown".into(),
            mode: "tdm".into(),
            members: vec![ME, OTHER],
            skip_votes: 1,
        }
    }

    fn core_in_match(epoch: u32) -> SessionCore {
        let mut core = SessionCore::default();
        assert!(core.apply(state(1, epoch, true)).changed);
        core
    }

    #[test]
    fn newer_than_nothing_is_always_true() {
        assert!(state_is_newer(None, 0, 0));
        assert!(state_is_newer(None, u64::MAX, u32::MAX));
    }

    #[test]
    fn within_an_epoch_only_a_later_revision_wins() {
        assert!(state_is_newer(Some((3, 10)), 11, 3));
        assert!(!state_is_newer(Some((3, 10)), 10, 3));
        assert!(!state_is_newer(Some((3, 10)), 9, 3));
    }

    #[test]
    fn revisions_wrap_around() {
        assert!(state_is_newer(Some((3, u64::MAX)), 0, 3));
        assert!(!state_is_newer(Some((3, 0)), u64::MAX, 3));
        assert!(state_is_newer(Some((3, 0)), (1 << 63) - 1, 3));
        assert!(!state_is_newer(Some((3, 0)), 1 << 63, 3));
    }

    #[test]
    fn a_later_epoch_wins_whatever_the_revision() {
        assert!(state_is_newer(Some((3, 500)), 0, 4));
        assert!(!state_is_newer(Some((4, 0)), 500, 3));
        assert!(state_is_newer(Some((u32::MAX, 0)), 0, 0));
        assert!(!state_is_newer(Some((0, 0)), 0, u32::MAX));
    }

    #[test]
    fn the_first_state_is_taken_and_shown() {
        let mut core = SessionCore::default();
        let applied = core.apply(state(1, 0, false));
        assert!(applied.changed);
        assert!(!applied.closed);
        assert_eq!(applied.view.map, "mp_nuketown");
        assert_eq!(applied.view.mode, "tdm");
        assert_eq!(applied.view.members, vec![ME, OTHER]);
        assert_eq!(applied.view.skip_votes, 1);
        assert!(!applied.view.in_match);
        assert_eq!(core.match_phase(), MatchPhase::None);
        assert_eq!(core.view(), &applied.view);
    }

    #[test]
    fn a_state_in_a_match_starts_loading_and_stays_running_once_entered() {
        let mut core = core_in_match(2);
        assert_eq!(core.match_phase(), MatchPhase::Loading);
        let entered = core.apply(SessionEvent::EnterMatch {
            epoch: 2,
            bootstrap_id: 9,
        });
        assert!(!entered.changed);
        assert_eq!(entered.enter_bootstrap, Some(9));
        assert_eq!(core.match_phase(), MatchPhase::Running);
        core.apply(state(2, 2, true));
        assert_eq!(core.match_phase(), MatchPhase::Running, "same match stays running");
        core.apply(state(1, 3, true));
        assert_eq!(core.match_phase(), MatchPhase::Loading, "a new match loads again");
    }

    #[test]
    fn a_state_out_of_a_match_clears_the_phase() {
        let mut core = core_in_match(2);
        core.apply(SessionEvent::EnterMatch {
            epoch: 2,
            bootstrap_id: 1,
        });
        core.apply(state(2, 2, false));
        assert_eq!(core.match_phase(), MatchPhase::None);
    }

    #[test]
    fn stale_and_repeated_states_are_refused() {
        let mut core = core_in_match(2);
        for revision in [0, 1] {
            let applied = core.apply(state(revision, 2, true));
            assert!(!applied.changed);
            assert!(applied.transition.contains("failure_source=stale state"));
        }
        assert!(core.apply(state(2, 2, true)).changed);
    }

    #[test]
    fn identity_that_does_not_fit_is_refused() {
        let mut core = SessionCore::default();
        core.apply(state(1, 2, true));
        let wrong_session = SessionEvent::AuthoritativeState {
            session_id: [8; 16],
            member: ME,
            revision: 2,
            match_epoch: 2,
            in_match: true,
            start_nonce: 2,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        let wrong_member = SessionEvent::AuthoritativeState {
            session_id: SESSION,
            member: OTHER,
            revision: 2,
            match_epoch: 2,
            in_match: true,
            start_nonce: 2,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        let nonce_differs = SessionEvent::AuthoritativeState {
            session_id: SESSION,
            member: ME,
            revision: 2,
            match_epoch: 2,
            in_match: true,
            start_nonce: 3,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        let match_without_epoch = SessionEvent::AuthoritativeState {
            session_id: SESSION,
            member: ME,
            revision: 2,
            match_epoch: 0,
            in_match: true,
            start_nonce: 0,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        for event in [wrong_session, wrong_member, nonce_differs, match_without_epoch] {
            let applied = core.apply(event);
            assert!(!applied.changed);
            assert!(applied.transition.contains("failure_source=state identity mismatch"));
        }
        assert_eq!(core.view().map, "mp_nuketown", "the good state is untouched");
    }

    #[test]
    fn empty_ids_are_refused() {
        let mut core = SessionCore::default();
        let no_session = SessionEvent::AuthoritativeState {
            session_id: NO_ID,
            member: ME,
            revision: 1,
            match_epoch: 0,
            in_match: false,
            start_nonce: 0,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        let no_member = SessionEvent::AuthoritativeState {
            session_id: SESSION,
            member: MemberId(NO_ID),
            revision: 1,
            match_epoch: 0,
            in_match: false,
            start_nonce: 0,
            map: String::new(),
            mode: String::new(),
            members: vec![],
            skip_votes: 0,
        };
        assert!(!core.apply(no_session).changed);
        assert!(!core.apply(no_member).changed);
    }

    #[test]
    fn a_connection_knows_its_session_up_front() {
        let mut core = SessionCore::for_connection([8; 16], 3);
        let applied = core.apply(state(1, 0, false));
        assert!(!applied.changed, "the state is for another session");
        assert!(applied.transition.contains("session_id=08080808080808080808080808080808"));
        assert!(applied.transition.contains("incarnation=3"));
        let mut right = SessionCore::for_connection(SESSION, 3);
        assert!(right.apply(state(1, 0, false)).changed);
    }

    #[test]
    fn entering_needs_a_match_and_its_epoch() {
        let mut idle = SessionCore::default();
        let early = idle.apply(SessionEvent::EnterMatch {
            epoch: 2,
            bootstrap_id: 1,
        });
        assert_eq!(early.enter_bootstrap, None);
        assert!(early.transition.contains("failure_source=enter epoch mismatch"));
        let mut core = core_in_match(2);
        let wrong = core.apply(SessionEvent::EnterMatch {
            epoch: 3,
            bootstrap_id: 1,
        });
        assert_eq!(wrong.enter_bootstrap, None);
        assert_eq!(core.match_phase(), MatchPhase::Loading);
        let right = core.apply(SessionEvent::EnterMatch {
            epoch: 2,
            bootstrap_id: 1,
        });
        assert!(right.transition.contains("bootstrap_id=1"));
        assert!(right.transition.contains("event=enter committed"));
    }

    #[test]
    fn ending_the_match_needs_the_right_epoch_and_only_changes_once() {
        let mut core = core_in_match(2);
        let stale = core.apply(SessionEvent::MatchEnded { epoch: 1 });
        assert!(!stale.changed);
        assert!(stale.transition.contains("failure_source=stale match end"));
        let ended = core.apply(SessionEvent::MatchEnded { epoch: 2 });
        assert!(ended.changed);
        assert!(!ended.view.in_match);
        assert_eq!(core.match_phase(), MatchPhase::None);
        let twice = core.apply(SessionEvent::MatchEnded { epoch: 2 });
        assert!(!twice.changed);
        assert!(twice.transition.contains("failure_source=-"));
        assert!(twice.transition.contains("event=end-match"));
    }

    #[test]
    fn closing_ends_everything_and_refuses_what_follows() {
        let mut core = core_in_match(2);
        let closed = core.apply(SessionEvent::CloseSession);
        assert!(closed.changed);
        assert!(closed.closed);
        assert!(!closed.view.in_match);
        assert_eq!(core.match_phase(), MatchPhase::None);
        let after = core.apply(state(9, 3, true));
        assert!(!after.changed);
        assert!(after.closed);
        assert!(after.transition.contains("failure_source=session closed"));
        assert!(after.transition.contains("event=state"));
    }

    #[test]
    fn the_transition_line_names_the_stage_before_and_after() {
        let mut core = SessionCore::default();
        let applied = core.apply(state(7, 2, true));
        assert!(applied.transition.contains(
            "stage_before=open=true,epoch=0,in_match=false,phase=None,rev=None"
        ));
        assert!(applied.transition.contains(
            "stage_after=open=true,epoch=2,in_match=true,phase=Loading,rev=Some((2, 7))"
        ));
        assert!(applied.transition.contains("member_id=05050505050505050505050505050505"));
    }
}
