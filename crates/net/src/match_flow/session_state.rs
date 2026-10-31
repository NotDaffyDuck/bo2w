//! The lobby session as seen from a client: the host's authoritative state, where the newest one
//! wins, and the match phase that follows from it.

use master_protocol::RoomOccupantId;

use super::match_host_books::GameRoundStage;
use super::world_rules::teardown_hits_epoch;
use super::transition::describe_session_change;
use crate::net_links::match_handoff::same_generation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameSessionNotice {
    HostDecidedState {
        session_no: [u8; 16],
        member: RoomOccupantId,
        version_no: u64,
        game_generation: u32,
        in_play: bool,
        launch_token: u32,
        map: String,
        game_mode: String,
        members: Vec<RoomOccupantId>,
        pass_vote_count: u8,
    },
    JoinGame { epoch: u32, bootstrap_id: u32 },
    MatchEnded { epoch: u32 },
    EndSession,
}

impl GameSessionNotice {
    /// How the event is named in the transition line.
    fn label(&self) -> &'static str {
        match self {
            Self::EndSession => "close-session",
            Self::MatchEnded { .. } => "end-match",
            Self::JoinGame { .. } => "enter committed",
            Self::HostDecidedState { .. } => "state",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSnapshotView {
    pub map: String,
    pub game_mode: String,
    pub members: Vec<RoomOccupantId>,
    pub pass_vote_count: u8,
    pub launch_token: u32,
    pub in_play: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInstallParams {
    pub shown_view: SessionSnapshotView,
    pub did_change: bool,
    pub bootstrap_to_enter: Option<u32>,
    pub session_closed: bool,
    pub change_note: String,
}

/// What an event amounted to: whether it changed anything, and the refusal reason if it was one.
struct Settled {
    moved: bool,
    entered_boot: Option<u32>,
    refusal: &'static str,
}

impl Settled {
    const NO_REFUSAL: &'static str = "-";

    fn moved() -> Self {
        Self {
            moved: true,
            ..Self::still()
        }
    }

    fn still() -> Self {
        Self {
            moved: false,
            entered_boot: None,
            refusal: Self::NO_REFUSAL,
        }
    }

    fn refused(why: &'static str) -> Self {
        Self {
            refusal: why,
            ..Self::still()
        }
    }
}

const BLANK_ID: [u8; 16] = [0; 16];

/// Who sent an authoritative state, for which session, and which match it speaks of.
struct Sender {
    room: [u8; 16],
    member: RoomOccupantId,
    epoch: u32,
    nonce: u32,
    live: bool,
}

impl Sender {
    /// Epoch and nonce agree, and epoch 0 (no match yet) is never claimed to be live.
    fn epoch_consistent(&self) -> bool {
        if self.nonce != self.epoch {
            return false;
        }
        self.epoch > 0 || !self.live
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionStateCore {
    closed: bool,
    /// Epoch and revision of the newest state taken in.
    newest: Option<(u32, u64)>,
    shown: SessionSnapshotView,
    round_stage: GameRoundStage,
    lobby_number: [u8; 16],
    attempt: u64,
    me: Option<RoomOccupantId>,
}

impl SessionStateCore {
    /// A core for a connection whose session is already known.
    pub fn for_connection(lobby_number: [u8; 16], attempt: u64) -> Self {
        let mut core = Self::default();
        core.lobby_number = lobby_number;
        core.attempt = attempt;
        core
    }

    pub fn view(&self) -> &SessionSnapshotView {
        &self.shown
    }

    pub fn match_phase(&self) -> GameRoundStage {
        self.round_stage
    }

    pub fn take_event(&mut self, event: GameSessionNotice) -> SessionInstallParams {
        let prior = self.stage_label();
        let name = event.label();
        let settled = match self.closed {
            true => Settled::refused("session closed"),
            false => self.settle(event),
        };
        self.reply(&prior, name, settled)
    }

    fn settle(&mut self, event: GameSessionNotice) -> Settled {
        match event {
            GameSessionNotice::EndSession => {
                self.closed = true;
                self.close_match()
            }
            GameSessionNotice::MatchEnded { epoch } => self.end(epoch),
            GameSessionNotice::JoinGame { epoch, bootstrap_id: boot } => self.enter(epoch, boot),
            GameSessionNotice::HostDecidedState { session_no: room, member, version_no: rev, game_generation, in_play: live, launch_token, map, game_mode: kind, members, pass_vote_count } => {
                let sender = Sender { room, member, epoch: game_generation, nonce: launch_token, live };
                let view = SessionSnapshotView { map, game_mode: kind, members, pass_vote_count, launch_token, in_play: live };
                self.take_state(sender, rev, view)
            }
        }
    }

    /// The sender names a real session and member and agrees with what is already known.
    fn knows(&self, sender: &Sender) -> bool {
        if sender.room == BLANK_ID || sender.member.0 == BLANK_ID {
            return false;
        }
        let room_agrees = self.lobby_number == BLANK_ID || self.lobby_number == sender.room;
        let member_agrees = match self.me {
            Some(known) => known == sender.member,
            None => true,
        };
        room_agrees && member_agrees
    }

    fn take_state(&mut self, sender: Sender, rev_no: u64, view: SessionSnapshotView) -> Settled {
        if !(self.knows(&sender) && sender.epoch_consistent()) {
            return Settled::refused("state identity mismatch");
        }
        if !supersedes_applied(self.newest, rev_no, sender.epoch) {
            return Settled::refused("stale state");
        }
        // Only the match already shown, and already entered, carries on running.
        let same_match = self.shown.in_play && self.shown.launch_token == sender.epoch;
        let entered = self.round_stage == GameRoundStage::Running;
        self.round_stage = match (view.in_play, same_match && entered) {
            (false, _) => GameRoundStage::None,
            (true, true) => GameRoundStage::Running,
            (true, false) => GameRoundStage::Loading,
        };
        self.lobby_number = sender.room;
        self.me = Some(sender.member);
        self.newest = Some((sender.epoch, rev_no));
        self.shown = view;
        Settled::moved()
    }

    fn enter(&mut self, epoch: u32, boot: u32) -> Settled {
        if !self.shown.in_play || !same_generation(epoch, self.shown.launch_token) {
            return Settled::refused("enter epoch mismatch");
        }
        self.round_stage = GameRoundStage::Running;
        Settled { entered_boot: Some(boot), ..Settled::still() }
    }

    fn end(&mut self, epoch: u32) -> Settled {
        if !teardown_hits_epoch(epoch, self.shown.launch_token) {
            Settled::refused("stale match end")
        } else if self.shown.in_play {
            self.close_match()
        } else {
            Settled::still()
        }
    }

    /// The match is over: the view says so and the phase goes back to none.
    fn close_match(&mut self) -> Settled {
        self.shown.in_play = false;
        self.round_stage = GameRoundStage::None;
        Settled::moved()
    }

    /// `open=..,epoch=..,in_match=..,phase=..,rev=..` for the transition line.
    fn stage_label(&self) -> String {
        let parts = [
            ("open", (!self.closed).to_string()),
            ("epoch", self.shown.launch_token.to_string()),
            ("in_match", self.shown.in_play.to_string()),
            ("phase", format!("{:?}", self.round_stage)),
            ("rev", format!("{:?}", self.newest)),
        ];
        let pairs: Vec<String> = parts.iter().map(|(key, value)| format!("{key}={value}")).collect();
        pairs.join(",")
    }

    fn reply(&self, prior: &str, label_text: &'static str, settled: Settled) -> SessionInstallParams {
        let inc_no = (self.attempt != 0).then_some(self.attempt);
        let after = self.stage_label();
        let line = describe_session_change(
            self.lobby_number,
            inc_no,
            self.shown.launch_token,
            None,
            self.me.map(|id| id.0),
            None,
            settled.entered_boot,
            prior,
            label_text,
            &after,
            None,
            settled.refusal,
        );
        SessionInstallParams {
            shown_view: self.shown.clone(),
            did_change: settled.moved,
            bootstrap_to_enter: settled.entered_boot,
            session_closed: self.closed,
            change_note: line,
        }
    }
}

/// `new` lies in the half-circle ahead of `old` (and is not `old`), on a wrapping u32 counter.
fn ahead32(new: u32, old: u32) -> bool {
    (new.wrapping_sub(old) as i32) > 0
}

/// `new` lies in the half-circle ahead of `old` (and is not `old`), on a wrapping u64 counter.
fn ahead64(new: u64, old: u64) -> bool {
    (new.wrapping_sub(old) as i64) > 0
}

/// Whether a state at `revision` in `epoch` supersedes the one applied: a different epoch must be
/// ahead of the applied one, the same epoch needs a revision ahead of the applied one.
pub fn supersedes_applied(held: Option<(u32, u64)>, rev_no: u64, new_epoch: u32) -> bool {
    let Some((had_epoch, had_revision)) = held else {
        return true;
    };
    if had_epoch == new_epoch {
        ahead64(rev_no, had_revision)
    } else {
        ahead32(new_epoch, had_epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: [u8; 16] = [4; 16];
    const ME: RoomOccupantId = RoomOccupantId([5; 16]);
    const OTHER: RoomOccupantId = RoomOccupantId([6; 16]);

    /// An authoritative state; `detailed` fills map/mode/members/votes, otherwise they are empty.
    fn claim(room: [u8; 16], from: RoomOccupantId, rev_epoch_nonce: (u64, u32, u32), live: bool, detailed: bool) -> GameSessionNotice {
        let (rev, epoch, nonce) = rev_epoch_nonce;
        let text = |s: &str| if detailed { s.to_owned() } else { String::new() };
        GameSessionNotice::HostDecidedState {
            session_no: room,
            member: from,
            version_no: rev,
            game_generation: epoch,
            in_play: live,
            launch_token: nonce,
            map: text("mp_nuketown"),
            game_mode: text("tdm"),
            members: if detailed { vec![ME, OTHER] } else { Vec::new() },
            pass_vote_count: u8::from(detailed),
        }
    }

    fn state(revision: u64, epoch: u32, in_match: bool) -> GameSessionNotice {
        claim(SESSION, ME, (revision, epoch, epoch), in_match, true)
    }

    fn enter_event(epoch: u32, boot: u32) -> GameSessionNotice {
        GameSessionNotice::JoinGame { epoch, bootstrap_id: boot }
    }

    fn ended(epoch: u32) -> GameSessionNotice {
        GameSessionNotice::MatchEnded { epoch }
    }

    fn core_in_match(epoch: u32) -> SessionStateCore {
        let mut core = SessionStateCore::default();
        assert!(core.take_event(state(1, epoch, true)).did_change);
        core
    }

    fn refused_with(applied: &SessionInstallParams, why: &str) -> bool {
        !applied.did_change && applied.change_note.contains(&format!("failure_source={why}"))
    }

    #[test]
    fn newer_than_nothing_is_always_true() {
        assert!(supersedes_applied(None, 0, 0));
        assert!(supersedes_applied(None, u64::MAX, u32::MAX));
    }

    #[test]
    fn within_an_epoch_only_a_later_revision_wins() {
        let verdicts: Vec<bool> = [11, 10, 9].iter().map(|&rev| supersedes_applied(Some((3, 10)), rev, 3)).collect();
        assert_eq!(verdicts, [true, false, false]);
    }

    #[test]
    fn revisions_wrap_around() {
        let cases = [
            ((3, u64::MAX), 0, true),
            ((3, 0), u64::MAX, false),
            ((3, 0), (1 << 63) - 1, true),
            ((3, 0), 1 << 63, false),
        ];
        for (had, rev, want) in cases {
            assert_eq!(supersedes_applied(Some(had), rev, 3), want, "{had:?} -> {rev}");
        }
    }

    #[test]
    fn a_later_epoch_wins_whatever_the_revision() {
        let cases = [((3, 500), 0, 4, true), ((4, 0), 500, 3, false), ((u32::MAX, 0), 0, 0, true), ((0, 0), 0, u32::MAX, false)];
        for (had, rev, epoch, want) in cases {
            assert_eq!(supersedes_applied(Some(had), rev, epoch), want, "{had:?} -> ({epoch}, {rev})");
        }
    }

    #[test]
    fn the_first_state_is_taken_and_shown() {
        let mut core = SessionStateCore::default();
        let applied = core.take_event(state(1, 0, false));
        assert!(applied.did_change && !applied.session_closed);
        let want = SessionSnapshotView {
            map: "mp_nuketown".into(),
            game_mode: "tdm".into(),
            members: vec![ME, OTHER],
            pass_vote_count: 1,
            launch_token: 0,
            in_play: false,
        };
        assert_eq!(applied.shown_view, want);
        assert_eq!(core.match_phase(), GameRoundStage::None);
        assert_eq!(core.view(), &applied.shown_view);
    }

    #[test]
    fn a_state_in_a_match_starts_loading_and_stays_running_once_entered() {
        let mut core = core_in_match(2);
        assert_eq!(core.match_phase(), GameRoundStage::Loading);
        let entered = core.take_event(enter_event(2, 9));
        assert_eq!((entered.did_change, entered.bootstrap_to_enter), (false, Some(9)));
        assert_eq!(core.match_phase(), GameRoundStage::Running);
        core.take_event(state(2, 2, true));
        assert_eq!(core.match_phase(), GameRoundStage::Running, "same match stays running");
        core.take_event(state(1, 3, true));
        assert_eq!(core.match_phase(), GameRoundStage::Loading, "a new match loads again");
    }

    #[test]
    fn a_state_out_of_a_match_clears_the_phase() {
        let mut core = core_in_match(2);
        core.take_event(enter_event(2, 1));
        core.take_event(state(2, 2, false));
        assert_eq!(core.match_phase(), GameRoundStage::None);
    }

    #[test]
    fn stale_and_repeated_states_are_refused() {
        let mut core = core_in_match(2);
        assert!(refused_with(&core.take_event(state(0, 2, true)), "stale state"));
        assert!(refused_with(&core.take_event(state(1, 2, true)), "stale state"));
        assert!(core.take_event(state(2, 2, true)).did_change);
    }

    #[test]
    fn identity_that_does_not_fit_is_refused() {
        let mut core = SessionStateCore::default();
        core.take_event(state(1, 2, true));
        let misfits = [
            ([8; 16], ME, (2, 2, 2)),
            (SESSION, OTHER, (2, 2, 2)),
            (SESSION, ME, (2, 2, 3)),
            (SESSION, ME, (2, 0, 0)),
        ];
        let all_refused = misfits
            .into_iter()
            .all(|(room, from, numbers)| refused_with(&core.take_event(claim(room, from, numbers, true, false)), "state identity mismatch"));
        assert!(all_refused);
        assert_eq!(core.view().map, "mp_nuketown", "the good state is untouched");
    }

    #[test]
    fn empty_ids_are_refused() {
        let mut core = SessionStateCore::default();
        let blank_room = claim(BLANK_ID, ME, (1, 0, 0), false, false);
        let blank_member = claim(SESSION, RoomOccupantId(BLANK_ID), (1, 0, 0), false, false);
        assert!(!core.take_event(blank_room).did_change);
        assert!(!core.take_event(blank_member).did_change);
    }

    #[test]
    fn a_connection_knows_its_session_up_front() {
        let mut elsewhere = SessionStateCore::for_connection([8; 16], 3);
        let applied = elsewhere.take_event(state(1, 0, false));
        assert!(!applied.did_change, "the state is for another session");
        for needle in ["session_id=08080808080808080808080808080808", "incarnation=3"] {
            assert!(applied.change_note.contains(needle), "{needle}");
        }
        let mut right = SessionStateCore::for_connection(SESSION, 3);
        assert!(right.take_event(state(1, 0, false)).did_change);
    }

    #[test]
    fn entering_needs_a_match_and_its_epoch() {
        let early = SessionStateCore::default().take_event(enter_event(2, 1));
        assert_eq!(early.bootstrap_to_enter, None);
        assert!(early.change_note.contains("failure_source=enter epoch mismatch"));
        let mut core = core_in_match(2);
        assert_eq!(core.take_event(enter_event(3, 1)).bootstrap_to_enter, None);
        assert_eq!(core.match_phase(), GameRoundStage::Loading);
        let right = core.take_event(enter_event(2, 1)).change_note;
        assert!(right.contains("bootstrap_id=1") && right.contains("event=enter committed"));
    }

    #[test]
    fn ending_the_match_needs_the_right_epoch_and_only_changes_once() {
        let mut core = core_in_match(2);
        assert!(refused_with(&core.take_event(ended(1)), "stale match end"));
        let first = core.take_event(ended(2));
        assert!(first.did_change && !first.shown_view.in_play);
        assert_eq!(core.match_phase(), GameRoundStage::None);
        let again = core.take_event(ended(2));
        assert!(refused_with(&again, "-"));
        assert!(again.change_note.contains("event=end-match"));
    }

    #[test]
    fn closing_ends_everything_and_refuses_what_follows() {
        let mut core = core_in_match(2);
        let closing = core.take_event(GameSessionNotice::EndSession);
        assert!(closing.did_change && closing.session_closed && !closing.shown_view.in_play);
        assert_eq!(core.match_phase(), GameRoundStage::None);
        let later = core.take_event(state(9, 3, true));
        assert!(later.session_closed);
        assert!(refused_with(&later, "session closed"));
        assert!(later.change_note.contains("event=state"));
    }

    #[test]
    fn the_transition_line_names_the_stage_before_and_after() {
        let line = SessionStateCore::default().take_event(state(7, 2, true)).change_note;
        let expected = [
            "stage_before=open=true,epoch=0,in_match=false,phase=None,rev=None",
            "stage_after=open=true,epoch=2,in_match=true,phase=Loading,rev=Some((2, 7))",
            "member_id=05050505050505050505050505050505",
        ];
        for piece in expected {
            assert!(line.contains(piece), "missing {piece} in {line}");
        }
    }
}
