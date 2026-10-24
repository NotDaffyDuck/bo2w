//! The joining side's view of one match: which world is loaded for it, whether it was admitted,
//! and whether the player may pick a class yet.

use frame::{AdmissionKey, LocalLoadKey, MatchKey, MatchTornDown};

use super::progress::SessionFail;
use super::rules::{class_select_allowed, match_key_boundary_applies};
use super::transition::format_session_transition;

#[derive(Clone, Debug)]
pub struct ClientMatchCore {
    session_open: bool,
    match_key: MatchKey,
    /// The last match that was torn down; late news about it is ignored.
    retired_match: Option<MatchKey>,
    installed: Option<LocalLoadKey>,
    presentation_ready: bool,
    local_authority_ready: bool,
    adopted: Option<AdmissionKey>,
    enter: Option<AdmissionKey>,
    failed: Option<SessionFail>,
}

impl Default for ClientMatchCore {
    fn default() -> Self {
        Self {
            session_open: true,
            match_key: MatchKey::NONE,
            retired_match: None,
            installed: None,
            presentation_ready: false,
            local_authority_ready: false,
            adopted: None,
            enter: None,
            failed: None,
        }
    }
}

impl ClientMatchCore {
    /// The match was torn down. Only the live match, and only the load that is installed, can be
    /// torn down; the rest starts over and remembers which match is now retired.
    pub fn apply_teardown(&mut self, fact: MatchTornDown) {
        let hits_live_match = match_key_boundary_applies(fact.match_key, self.match_key);
        let hits_installed_load = self
            .installed
            .is_none_or(|load| fact.world_generation.0 == Some(load.local_load_request_id));
        if !hits_live_match || !hits_installed_load {
            return;
        }
        let retired = (!fact.match_key.is_none())
            .then_some(fact.match_key)
            .or(self.retired_match);
        *self = Self {
            session_open: self.session_open,
            retired_match: retired,
            ..Self::default()
        };
    }

    /// A world finished loading locally. A load that names no match belongs to the one we have.
    pub fn apply_install(&mut self, load_key: LocalLoadKey) {
        if !self.session_open || self.retired_match == Some(load_key.match_key) {
            return;
        }
        let mut load = load_key;
        if load.match_key.is_none() {
            load.match_key = self.match_key;
        }
        let known = !self.match_key.is_none();
        if known && load.match_key != self.match_key {
            return;
        }
        self.installed = Some(load);
    }

    pub fn apply_presentation(&mut self, load_key: LocalLoadKey) {
        if self.installed == Some(load_key) {
            self.presentation_ready = true;
        }
    }

    /// The host announced a match. The first one is simply adopted; a different one afterwards
    /// throws away everything that belonged to the old match.
    pub fn apply_start(&mut self, match_key: MatchKey) {
        let refused = !self.session_open
            || match_key.is_none()
            || self.retired_match == Some(match_key)
            || self.match_key == match_key;
        if refused {
            return;
        }
        if self.match_key.is_none() {
            self.match_key = match_key;
            if let Some(load) = self.installed.as_mut()
                && load.match_key.is_none()
            {
                load.match_key = match_key;
            }
            return;
        }
        self.installed = None;
        self.presentation_ready = false;
        self.local_authority_ready = false;
        self.adopted = None;
        self.enter = None;
        self.failed = None;
        self.match_key = match_key;
    }

    pub fn apply_local_authority_ready(&mut self, ready: bool) {
        self.local_authority_ready = ready;
    }

    pub fn installed(&self) -> Option<LocalLoadKey> {
        self.installed
    }

    pub fn match_key(&self) -> MatchKey {
        self.match_key
    }

    /// The host accepted us. Returns the transition line when this changed anything.
    pub fn apply_adopted(&mut self, key: AdmissionKey) -> Option<String> {
        if key.match_key != self.match_key || self.adopted == Some(key) {
            return None;
        }
        let before = self.stage_label();
        self.adopted = Some(key);
        Some(self.transition_line(&before, "client adopted", Some(key)))
    }

    /// We entered the match we were adopted into. Only the adopted admission may enter.
    pub fn apply_enter(&mut self, key: AdmissionKey) -> Option<String> {
        if self.adopted != Some(key) || self.enter == Some(key) {
            return None;
        }
        let before = self.stage_label();
        self.enter = Some(key);
        Some(self.transition_line(&before, "client enter", Some(key)))
    }

    /// Only the first failure is kept.
    pub fn apply_fail(&mut self, fail: SessionFail) {
        self.failed.get_or_insert(fail);
    }

    pub fn apply_close(&mut self) {
        self.session_open = false;
    }

    pub fn class_select_allowed(&self) -> bool {
        let installed_for_match = self
            .installed
            .is_some_and(|load| load.belongs_to(self.match_key));
        class_select_allowed(
            self.session_open,
            installed_for_match,
            self.presentation_ready,
            self.enter.is_some(),
            self.failed.is_some(),
        )
    }

    /// The same question for the local host's own player, who needs no admission.
    pub fn local_class_select_allowed(&self) -> bool {
        let installed = self.installed.is_some();
        class_select_allowed(
            self.session_open,
            installed,
            self.presentation_ready,
            self.local_authority_ready && installed,
            self.failed.is_some(),
        )
    }

    pub fn failed(&self) -> Option<&SessionFail> {
        self.failed.as_ref()
    }

    fn stage_label(&self) -> String {
        format!(
            "open={},epoch={},installed={},presentation={},adopted={},enter={}",
            self.session_open,
            self.match_key.match_epoch,
            self.installed.is_some(),
            self.presentation_ready,
            self.adopted.is_some(),
            self.enter.is_some(),
        )
    }

    fn transition_line(
        &self,
        before: &str,
        event: &'static str,
        key: Option<AdmissionKey>,
    ) -> String {
        let key = key.or(self.adopted).or(self.enter);
        let failure = self
            .failed
            .as_ref()
            .map_or("-", |fail| fail.source.as_str());
        format_session_transition(
            self.match_key.session_id,
            self.installed.map(|load| load.incarnation),
            self.match_key.match_epoch,
            self.installed.map(|load| load.local_load_request_id),
            key.map(|key| key.member_id),
            key.map(|key| key.connection_id),
            key.map(|key| key.bootstrap_id),
            before,
            event,
            &self.stage_label(),
            None,
            failure,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_core::FailStage;
    use frame::{TeardownReason, WorldGeneration};

    const SESSION: [u8; 16] = [9; 16];

    fn key(epoch: u32) -> MatchKey {
        MatchKey::new(SESSION, epoch)
    }

    fn load(request: u64, match_key: MatchKey) -> LocalLoadKey {
        LocalLoadKey::from_request(request, match_key, 2)
    }

    fn admission(match_key: MatchKey, bootstrap_id: u32) -> AdmissionKey {
        AdmissionKey {
            match_key,
            member_id: [3; 16],
            connection_id: 44,
            bootstrap_id,
        }
    }

    fn torn(match_key: MatchKey, generation: Option<u64>) -> MatchTornDown {
        MatchTornDown {
            reason: TeardownReason::MatchEnded,
            world_generation: WorldGeneration(generation),
            match_key,
            match_epoch: match_key.match_epoch,
        }
    }

    fn fail(source: &str) -> SessionFail {
        SessionFail {
            stage: FailStage::Load,
            source: source.into(),
            match_key: key(1),
        }
    }

    /// A client that has the match, the loaded world, the presentation and the admission.
    fn entered(epoch: u32) -> (ClientMatchCore, AdmissionKey, LocalLoadKey) {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(epoch));
        let load = load(5, key(epoch));
        core.apply_install(load);
        core.apply_presentation(load);
        let admitted = admission(key(epoch), 1);
        core.apply_adopted(admitted);
        core.apply_enter(admitted);
        (core, admitted, load)
    }

    #[test]
    fn a_fresh_client_may_not_pick_a_class() {
        let core = ClientMatchCore::default();
        assert!(!core.class_select_allowed());
        assert!(!core.local_class_select_allowed());
        assert!(core.failed().is_none());
        assert!(core.installed().is_none());
        assert!(core.match_key().is_none());
    }

    #[test]
    fn a_client_that_went_through_every_step_may_pick() {
        let (core, _, _) = entered(4);
        assert!(core.class_select_allowed());
    }

    #[test]
    fn each_missing_step_blocks_class_selection() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        let world = load(5, key(4));
        core.apply_install(world);
        assert!(!core.class_select_allowed(), "no presentation yet");
        core.apply_presentation(world);
        assert!(!core.class_select_allowed(), "not admitted yet");
        let admitted = admission(key(4), 1);
        core.apply_adopted(admitted);
        assert!(!core.class_select_allowed(), "adopted but not entered");
        core.apply_enter(admitted);
        assert!(core.class_select_allowed());
        core.apply_fail(fail("boom"));
        assert!(!core.class_select_allowed(), "a failure blocks");
    }

    #[test]
    fn presentation_for_another_load_is_ignored() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_install(load(5, key(4)));
        core.apply_presentation(load(6, key(4)));
        assert!(!core.local_class_select_allowed());
    }

    #[test]
    fn the_local_player_needs_the_authority_instead_of_an_admission() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        let world = load(5, key(4));
        core.apply_install(world);
        core.apply_presentation(world);
        assert!(!core.local_class_select_allowed());
        core.apply_local_authority_ready(true);
        assert!(core.local_class_select_allowed());
        core.apply_local_authority_ready(false);
        assert!(!core.local_class_select_allowed());
    }

    #[test]
    fn an_install_without_a_match_waits_for_the_start_and_takes_its_key() {
        let mut core = ClientMatchCore::default();
        core.apply_install(load(5, MatchKey::NONE));
        assert!(core.installed().unwrap().match_key.is_none());
        core.apply_start(key(4));
        assert_eq!(core.installed().unwrap().match_key, key(4));
        assert_eq!(core.match_key(), key(4));
    }

    #[test]
    fn an_install_without_a_match_joins_the_one_we_have() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_install(load(5, MatchKey::NONE));
        assert_eq!(core.installed().unwrap().match_key, key(4));
    }

    #[test]
    fn an_install_for_a_different_match_is_ignored() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_install(load(5, key(3)));
        assert!(core.installed().is_none());
    }

    #[test]
    fn a_second_match_discards_what_belonged_to_the_first() {
        let (mut core, _, _) = entered(4);
        core.apply_fail(fail("old"));
        core.apply_start(key(5));
        assert_eq!(core.match_key(), key(5));
        assert!(core.installed().is_none());
        assert!(core.failed().is_none());
        assert!(!core.class_select_allowed());
    }

    #[test]
    fn a_repeated_or_empty_start_changes_nothing() {
        let (mut core, _, _) = entered(4);
        core.apply_start(key(4));
        core.apply_start(MatchKey::NONE);
        assert!(core.class_select_allowed());
        assert_eq!(core.match_key(), key(4));
    }

    #[test]
    fn teardown_of_the_live_match_resets_and_retires_it() {
        let (mut core, _, world) = entered(4);
        core.apply_teardown(torn(key(4), Some(world.local_load_request_id)));
        assert!(core.match_key().is_none());
        assert!(core.installed().is_none());
        core.apply_start(key(4));
        assert!(core.match_key().is_none(), "a retired match cannot restart");
        core.apply_install(load(9, key(4)));
        assert!(core.installed().is_none(), "nor install");
        core.apply_start(key(5));
        assert_eq!(core.match_key(), key(5));
    }

    #[test]
    fn teardown_of_another_match_or_another_load_is_ignored() {
        let (mut core, _, world) = entered(4);
        core.apply_teardown(torn(key(3), Some(world.local_load_request_id)));
        assert_eq!(core.match_key(), key(4));
        core.apply_teardown(torn(key(4), Some(world.local_load_request_id + 1)));
        assert_eq!(core.match_key(), key(4));
        core.apply_teardown(torn(key(4), None));
        assert_eq!(core.match_key(), key(4));
    }

    #[test]
    fn teardown_with_nothing_installed_needs_no_generation() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_teardown(torn(key(4), None));
        assert!(core.match_key().is_none());
    }

    #[test]
    fn teardown_keeps_the_session_closed_if_it_was() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_close();
        core.apply_teardown(torn(key(4), None));
        core.apply_start(key(6));
        assert!(core.match_key().is_none());
    }

    #[test]
    fn a_closed_session_takes_no_new_match_or_install() {
        let mut core = ClientMatchCore::default();
        core.apply_close();
        core.apply_start(key(4));
        core.apply_install(load(5, key(4)));
        assert!(core.match_key().is_none());
        assert!(core.installed().is_none());
    }

    #[test]
    fn adoption_needs_the_live_match_and_reports_only_once() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        assert!(core.apply_adopted(admission(key(3), 1)).is_none());
        let first = core.apply_adopted(admission(key(4), 1)).unwrap();
        assert!(first.contains("event=client adopted"));
        assert!(first.contains("member_id=03030303030303030303030303030303"));
        assert!(first.contains("connection_id=44"));
        assert!(first.contains("bootstrap_id=1"));
        assert!(first.contains("stage_before=open=true,epoch=4,installed=false,presentation=false,adopted=false,enter=false"));
        assert!(first.contains("stage_after=open=true,epoch=4,installed=false,presentation=false,adopted=true,enter=false"));
        assert!(core.apply_adopted(admission(key(4), 1)).is_none());
    }

    #[test]
    fn entering_needs_the_adopted_admission_and_reports_only_once() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        let admitted = admission(key(4), 1);
        assert!(core.apply_enter(admitted).is_none(), "not adopted yet");
        core.apply_adopted(admitted);
        assert!(core.apply_enter(admission(key(4), 2)).is_none(), "another bootstrap");
        let line = core.apply_enter(admitted).unwrap();
        assert!(line.contains("event=client enter"));
        assert!(core.apply_enter(admitted).is_none());
    }

    #[test]
    fn the_transition_line_carries_the_installed_load_and_the_failure() {
        let mut core = ClientMatchCore::default();
        core.apply_start(key(4));
        core.apply_install(load(77, key(4)));
        core.apply_fail(fail("first"));
        core.apply_fail(fail("second"));
        let line = core.apply_adopted(admission(key(4), 1)).unwrap();
        assert!(line.contains("incarnation=2"));
        assert!(line.contains("local_load_id=77"));
        assert!(line.contains("failure_source=first"));
        assert_eq!(core.failed().unwrap().source, "first");
    }
}
