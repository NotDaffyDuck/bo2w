//! The joining side's view of one match: which world is loaded for it, whether it was admitted,
//! and whether the player may pick a class yet.

use frame::{EntryPassToken, OwnLevelLoadTicket, GameSessionStamp, GameSessionClosed};

use super::deadlines::SessionAbortRecord;
use super::world_rules::{loadout_pick_permitted, teardown_hits_key};
use super::transition::describe_session_change;

#[derive(Clone, Debug, Default)]
pub struct PlayerRoundCore {
    shut: bool,
    live: GameSessionStamp,
    /// The last match that was torn down; late news about it is ignored.
    retired: Option<GameSessionStamp>,
    world: Option<OwnLevelLoadTicket>,
    shown: bool,
    host_authority: bool,
    admitted: Option<EntryPassToken>,
    entered: Option<EntryPassToken>,
    first_failure: Option<SessionAbortRecord>,
}

impl PlayerRoundCore {
    /// The match was torn down. Only the live match, and only the load that is installed, can be
    /// torn down; the rest starts over and remembers which match is now retired.
    pub fn carry_out_shutdown(&mut self, fact: GameSessionClosed) {
        let names_installed_load = match self.world {
            None => true,
            Some(load) => fact.level_generation.0 == Some(load.own_level_ticket_no),
        };
        let on_live_match = teardown_hits_key(fact.game_session_key, self.live);
        if !(on_live_match && names_installed_load) {
            return;
        }
        let retired = (!fact.game_session_key.is_none())
            .then_some(fact.game_session_key)
            .or(self.retired);
        let shut = self.shut;
        *self = Self {
            retired,
            shut,
            ..Self::default()
        };
    }

    /// A world finished loading locally. A load that names no match belongs to the one we have.
    pub fn note_world_loaded(&mut self, finished: OwnLevelLoadTicket) {
        if self.shut || self.retired == Some(finished.game_session_key) {
            return;
        }
        let mut load = finished;
        if load.game_session_key.is_none() {
            load.game_session_key = self.live;
        }
        let foreign = !self.live.is_none() && load.game_session_key != self.live;
        if !foreign {
            self.world = Some(load);
        }
    }

    pub fn take_shown_level(&mut self, presented: OwnLevelLoadTicket) {
        self.shown |= self.world == Some(presented);
    }

    /// The host announced a match. The first one is simply adopted; a different one afterwards
    /// throws away everything that belonged to the old match.
    pub fn note_match_announced(&mut self, announced: GameSessionStamp) {
        if self.shut
            || announced.is_none()
            || self.retired == Some(announced)
            || self.live == announced
        {
            return;
        }
        if self.live.is_none() {
            if let Some(load) = self.world.as_mut().filter(|load| load.game_session_key.is_none()) {
                load.game_session_key = announced;
            }
        } else {
            self.world = None;
            self.shown = false;
            self.host_authority = false;
            self.admitted = None;
            self.entered = None;
            self.first_failure = None;
        }
        self.live = announced;
    }

    pub fn apply_local_authority_ready(
        &mut self,
        up: bool,
    ) {
        self.host_authority = up;
    }

    pub fn installed(&self) -> Option<OwnLevelLoadTicket> {
        self.world
    }

    pub fn game_session_key(&self) -> GameSessionStamp {
        self.live
    }

    /// The host accepted us. Returns the transition line when this changed anything.
    pub fn note_accepted(&mut self, admission: EntryPassToken) -> Option<String> {
        let fresh = admission.game_session_key == self.live && self.admitted != Some(admission);
        fresh.then(|| {
            let prior = self.stage_label();
            self.admitted = Some(admission);
            self.describe_line(&prior, "client accepted", Some(admission))
        })
    }

    /// We entered the match we were adopted into. Only the adopted admission may enter.
    pub fn note_match_entered(&mut self, admission: EntryPassToken) -> Option<String> {
        let allowed = self.admitted == Some(admission) && self.entered != Some(admission);
        allowed.then(|| {
            let prior = self.stage_label();
            self.entered = Some(admission);
            self.describe_line(&prior, "client entered", Some(admission))
        })
    }

    /// Only the first failure is kept.
    pub fn keep_first_failure(&mut self, failure: SessionAbortRecord) {
        self.first_failure.get_or_insert(failure);
    }

    pub fn finish_shutdown(&mut self) {
        self.shut = true;
    }

    pub fn loadout_pick_permitted(&self) -> bool {
        let world_is_ours = self.world.is_some_and(|load| load.issued_for(self.live));
        let seated = self.entered.is_some();
        loadout_pick_permitted(!self.shut, world_is_ours, self.shown, seated, self.failed().is_some())
    }

    /// The same question for the local host's own player, who needs no admission.
    pub fn self_loadout_pick_permitted(&self) -> bool {
        let level_is_up = self.world.is_some();
        let seated = level_is_up && self.host_authority;
        loadout_pick_permitted(!self.shut, level_is_up, self.shown, seated, self.failed().is_some())
    }

    pub fn failed(&self) -> Option<&SessionAbortRecord> {
        self.first_failure.as_ref()
    }

    fn stage_label(&self) -> String {
        let flags = [
            self.world.is_some(),
            self.shown,
            self.admitted.is_some(),
            self.entered.is_some(),
        ];
        format!(
            "open={},epoch={},installed={},presentation={},adopted={},enter={}",
            !self.shut, self.live.game_generation, flags[0], flags[1], flags[2], flags[3]
        )
    }

    fn describe_line(
        &self,
        prior: &str,
        what: &'static str,
        subject: Option<EntryPassToken>,
    ) -> String {
        let who = subject.or(self.admitted).or(self.entered);
        let failure = match &self.first_failure {
            Some(fail) => fail.source.as_str(),
            None => "-",
        };
        let after = self.stage_label();
        let (inc_no, request) = match self.world {
            Some(load) => (Some(load.incarnation), Some(load.own_level_ticket_no)),
            None => (None, None),
        };
        describe_session_change(
            self.live.stamp_bytes,
            inc_no,
            self.live.game_generation,
            request,
            who.map(|a| a.seat_bytes),
            who.map(|a| a.link_number),
            who.map(|a| a.opening_number),
            prior,
            what,
            &after,
            None,
            failure,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_flow::AbortStep;
    use frame::{ShutdownCause, LevelGeneration};

    const SESSION: [u8; 16] = [9; 16];

    fn key(epoch: u32) -> GameSessionStamp {
        GameSessionStamp::new(SESSION, epoch)
    }

    fn load(request: u64, game_session_key: GameSessionStamp) -> OwnLevelLoadTicket {
        OwnLevelLoadTicket::from_request(request, game_session_key, 2)
    }

    fn admission(game_session_key: GameSessionStamp, boot: u32) -> EntryPassToken {
        EntryPassToken {
            game_session_key,
            seat_bytes: [3; 16],
            link_number: 44,
            opening_number: boot,
        }
    }

    fn torn(
        game_session_key: GameSessionStamp,
        gen_no: Option<u64>,
    ) -> GameSessionClosed {
        GameSessionClosed {
            reason: ShutdownCause::RoundOver,
            level_generation: LevelGeneration(gen_no),
            game_session_key,
            game_generation: game_session_key.game_generation,
        }
    }

    fn fail(source: &str) -> SessionAbortRecord {
        SessionAbortRecord {
            stage: AbortStep::Load,
            source: source.into(),
            game_session_key: key(1),
        }
    }

    /// A client that has the match, the loaded world, the presentation and the admission.
    fn entered(epoch: u32) -> (PlayerRoundCore, EntryPassToken, OwnLevelLoadTicket) {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(epoch));
        let load = load(5, key(epoch));
        core.note_world_loaded(load);
        core.take_shown_level(load);
        let admitted = admission(key(epoch), 1);
        core.note_accepted(admitted);
        core.note_match_entered(admitted);
        (core, admitted, load)
    }

    #[test]
    fn a_fresh_client_may_not_pick_a_class() {
        let core = PlayerRoundCore::default();
        assert!(!core.loadout_pick_permitted());
        assert!(!core.self_loadout_pick_permitted());
        assert!(core.failed().is_none());
        assert!(core.installed().is_none());
        assert!(core.game_session_key().is_none());
    }

    #[test]
    fn a_client_that_went_through_every_step_may_pick() {
        let (core, _, _) = entered(4);
        assert!(core.loadout_pick_permitted());
    }

    #[test]
    fn each_missing_step_blocks_class_selection() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        let world = load(5, key(4));
        core.note_world_loaded(world);
        assert!(!core.loadout_pick_permitted(), "no presentation yet");
        core.take_shown_level(world);
        assert!(!core.loadout_pick_permitted(), "not admitted yet");
        let admitted = admission(key(4), 1);
        core.note_accepted(admitted);
        assert!(!core.loadout_pick_permitted(), "adopted but not entered");
        core.note_match_entered(admitted);
        assert!(core.loadout_pick_permitted());
        core.keep_first_failure(fail("boom"));
        assert!(!core.loadout_pick_permitted(), "a failure blocks");
    }

    #[test]
    fn presentation_for_another_load_is_ignored() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.note_world_loaded(load(5, key(4)));
        core.take_shown_level(load(6, key(4)));
        assert!(!core.self_loadout_pick_permitted());
    }

    #[test]
    fn the_local_player_needs_the_authority_instead_of_an_admission() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        let world = load(5, key(4));
        core.note_world_loaded(world);
        core.take_shown_level(world);
        assert!(!core.self_loadout_pick_permitted());
        core.apply_local_authority_ready(true);
        assert!(core.self_loadout_pick_permitted());
        core.apply_local_authority_ready(false);
        assert!(!core.self_loadout_pick_permitted());
    }

    #[test]
    fn an_install_without_a_match_waits_for_the_start_and_takes_its_key() {
        let mut core = PlayerRoundCore::default();
        core.note_world_loaded(load(5, GameSessionStamp::NONE));
        assert!(core.installed().unwrap().game_session_key.is_none());
        core.note_match_announced(key(4));
        assert_eq!(core.installed().unwrap().game_session_key, key(4));
        assert_eq!(core.game_session_key(), key(4));
    }

    #[test]
    fn an_install_without_a_match_joins_the_one_we_have() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.note_world_loaded(load(5, GameSessionStamp::NONE));
        assert_eq!(core.installed().unwrap().game_session_key, key(4));
    }

    #[test]
    fn an_install_for_a_different_match_is_ignored() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.note_world_loaded(load(5, key(3)));
        assert!(core.installed().is_none());
    }

    #[test]
    fn a_second_match_discards_what_belonged_to_the_first() {
        let (mut core, _, _) = entered(4);
        core.keep_first_failure(fail("old"));
        core.note_match_announced(key(5));
        assert_eq!(core.game_session_key(), key(5));
        assert!(core.installed().is_none());
        assert!(core.failed().is_none());
        assert!(!core.loadout_pick_permitted());
    }

    #[test]
    fn a_repeated_or_empty_start_changes_nothing() {
        let (mut core, _, _) = entered(4);
        core.note_match_announced(key(4));
        core.note_match_announced(GameSessionStamp::NONE);
        assert!(core.loadout_pick_permitted());
        assert_eq!(core.game_session_key(), key(4));
    }

    #[test]
    fn teardown_of_the_live_match_resets_and_retires_it() {
        let (mut core, _, world) = entered(4);
        core.carry_out_shutdown(torn(key(4), Some(world.own_level_ticket_no)));
        assert!(core.game_session_key().is_none());
        assert!(core.installed().is_none());
        core.note_match_announced(key(4));
        assert!(core.game_session_key().is_none(), "a retired match cannot restart");
        core.note_world_loaded(load(9, key(4)));
        assert!(core.installed().is_none(), "nor install");
        core.note_match_announced(key(5));
        assert_eq!(core.game_session_key(), key(5));
    }

    #[test]
    fn teardown_of_another_match_or_another_load_is_ignored() {
        let (mut core, _, world) = entered(4);
        core.carry_out_shutdown(torn(key(3), Some(world.own_level_ticket_no)));
        assert_eq!(core.game_session_key(), key(4));
        core.carry_out_shutdown(torn(key(4), Some(world.own_level_ticket_no + 1)));
        assert_eq!(core.game_session_key(), key(4));
        core.carry_out_shutdown(torn(key(4), None));
        assert_eq!(core.game_session_key(), key(4));
    }

    #[test]
    fn teardown_with_nothing_installed_needs_no_generation() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.carry_out_shutdown(torn(key(4), None));
        assert!(core.game_session_key().is_none());
    }

    #[test]
    fn teardown_keeps_the_session_closed_if_it_was() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.finish_shutdown();
        core.carry_out_shutdown(torn(key(4), None));
        core.note_match_announced(key(6));
        assert!(core.game_session_key().is_none());
    }

    #[test]
    fn a_closed_session_takes_no_new_match_or_install() {
        let mut core = PlayerRoundCore::default();
        core.finish_shutdown();
        core.note_match_announced(key(4));
        core.note_world_loaded(load(5, key(4)));
        assert!(core.game_session_key().is_none());
        assert!(core.installed().is_none());
    }

    #[test]
    fn adoption_needs_the_live_match_and_reports_only_once() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        assert!(core.note_accepted(admission(key(3), 1)).is_none());
        let first = core.note_accepted(admission(key(4), 1)).unwrap();
        assert!(first.contains("event=client accepted"));
        assert!(first.contains("member_id=03030303030303030303030303030303"));
        assert!(first.contains("connection_id=44"));
        assert!(first.contains("bootstrap_id=1"));
        assert!(first.contains("stage_before=open=true,epoch=4,installed=false,presentation=false,adopted=false,enter=false"));
        assert!(first.contains("stage_after=open=true,epoch=4,installed=false,presentation=false,adopted=true,enter=false"));
        assert!(core.note_accepted(admission(key(4), 1)).is_none());
    }

    #[test]
    fn entering_needs_the_adopted_admission_and_reports_only_once() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        let admitted = admission(key(4), 1);
        assert!(core.note_match_entered(admitted).is_none(), "not adopted yet");
        core.note_accepted(admitted);
        assert!(core.note_match_entered(admission(key(4), 2)).is_none(), "another bootstrap");
        let line = core.note_match_entered(admitted).unwrap();
        assert!(line.contains("event=client entered"));
        assert!(core.note_match_entered(admitted).is_none());
    }

    #[test]
    fn the_transition_line_carries_the_installed_load_and_the_failure() {
        let mut core = PlayerRoundCore::default();
        core.note_match_announced(key(4));
        core.note_world_loaded(load(77, key(4)));
        core.keep_first_failure(fail("first"));
        core.keep_first_failure(fail("second"));
        let line = core.note_accepted(admission(key(4), 1)).unwrap();
        assert!(line.contains("incarnation=2"));
        assert!(line.contains("local_load_id=77"));
        assert!(line.contains("failure_source=first"));
        assert_eq!(core.failed().unwrap().source, "first");
    }
}
