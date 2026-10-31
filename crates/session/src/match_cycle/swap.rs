//! A swap changes what this process is showing: a map, a recorded match, or the
//! front-end menu. Only one swap runs at a time. It brings the old world down,
//! decides which role the process takes on afterwards, then waits for the new
//! world to be installed (or to fail).

use bevy::prelude::*;
use frame::{
    LevelPresent, StartupPersona, OwnLevelLoadTicket, LevelOpenGranted, LevelOpenRejected, GameSessionArmed,
    GameSessionStamp, GameSessionClosed, ReturnedToMenu, ProcessDuty, ShutdownCause,
};

use super::ShutdownOrder;

/// What a swap is heading for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionReplacementGoal {
    /// Load the named map and play on it.
    Scene(String),
    /// Replay a recorded match on the map it was played on.
    Demo {
        demo: String,
        map: String,
        exit_when_done: bool,
    },
    /// Return to the front end. With `drop_lobby` the lobby is left as well.
    Menu { drop_lobby: bool },
}

impl SessionReplacementGoal {
    /// Short tag used for this swap in the perf log.
    fn label(&self) -> String {
        match self {
            Self::Scene(map) => format!("zone:{map}"),
            Self::Demo { demo, .. } => format!("demo:{demo}"),
            Self::Menu { drop_lobby: true } => "leave".into(),
            Self::Menu { drop_lobby: false } => "menu".into(),
        }
    }

    /// The reason the current world is taken down on the way to this target.
    fn teardown_reason(&self) -> ShutdownCause {
        match self {
            Self::Menu { drop_lobby: true } => ShutdownCause::Disconnect,
            Self::Menu { drop_lobby: false } => ShutdownCause::RoundOver,
            Self::Scene(_) | Self::Demo { .. } => ShutdownCause::Replaced,
        }
    }

    fn is_menu(&self) -> bool {
        matches!(*self, Self::Menu { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Fresh,
    TearingDown,
    Installing,
}

#[derive(Debug)]
struct Inflight {
    seq: u64,
    goal: SessionReplacementGoal,
    step: Step,
    /// A load was already begun for this swap and must be cancelled.
    cancel_load: bool,
    world_was_torn: bool,
}

/// The way a finished swap came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverVerdict {
    Installed {
        zone: String,
    },
    Menu,
    Failed {
        zone: String,
        error: String,
    },
}

impl HandoverVerdict {
    fn label(&self) -> String {
        match self {
            Self::Menu => "menu".into(),
            Self::Installed { zone } => format!("installed:{zone}"),
            Self::Failed { zone: region, .. } => format!("failed:{region}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoverFinish {
    pub id: u64,
    pub result: HandoverVerdict,
}

/// Where swaps are requested; it also holds the outcome of the latest one.
#[derive(Resource, Default, Debug)]
pub struct SessionHandoverOrder {
    last_seq: u64,
    current: Option<Inflight>,
    outcome: Option<HandoverFinish>,
}

impl SessionHandoverOrder {
    /// Is an install tagged `request_id` the one the running swap is waiting
    /// on? When nothing is running, only the very first load (id 0, with no
    /// swap ever requested) is allowed through.
    pub(crate) fn takes_install(&self, wanted_id: u64) -> bool {
        let Some(active) = &self.current else {
            return wanted_id == 0 && self.last_seq == 0;
        };
        active.seq == wanted_id && active.step == Step::Installing && !active.goal.is_menu()
    }

    pub fn ask_for_region(&mut self, region: String) -> Result<u64, String> {
        self.start(SessionReplacementGoal::Scene(region))
    }

    pub fn ask_for_recording(
        &mut self,
        name: String,
        zone: String,
        stop_after: bool,
    ) -> Result<u64, String> {
        self.start(SessionReplacementGoal::Demo {
            demo: name,
            map: zone,
            exit_when_done: stop_after,
        })
    }

    /// Return to the menu but stay in the lobby.
    pub fn ask_for_menu(&mut self) -> Result<u64, String> {
        self.return_to_menu(false)
    }

    /// Return to the menu and leave the lobby too.
    pub fn request_leave(&mut self) -> Result<u64, String> {
        self.return_to_menu(true)
    }

    /// A trip back to the menu beats any load that is still running: the
    /// swap is retargeted and the load is cancelled rather than queued behind.
    fn return_to_menu(&mut self, leave_session: bool) -> Result<u64, String> {
        let Some(active) = self.current.as_mut() else {
            return self.start(SessionReplacementGoal::Menu {
                drop_lobby: leave_session,
            });
        };
        let cause = if leave_session {
            "disconnect"
        } else {
            "match end"
        };
        let replaced = match &active.goal {
            SessionReplacementGoal::Menu { .. } => None,
            SessionReplacementGoal::Scene(map) => Some(format!("map `{map}`")),
            SessionReplacementGoal::Demo { demo, .. } => Some(format!("demo `{demo}`")),
        };
        match (&mut active.goal, replaced) {
            (SessionReplacementGoal::Menu { drop_lobby }, _) => {
                *drop_lobby |= leave_session;
            }
            (goal, Some(what)) => {
                diag::info!(
                    Sim,
                    "session: {cause} superseded in-flight {what} (swap #{}) -> menu",
                    active.seq
                );
                *goal = SessionReplacementGoal::Menu {
                    drop_lobby: leave_session,
                };
                active.cancel_load = true;
            }
            (_, None) => {}
        }
        Ok(active.seq)
    }

    fn start(&mut self, aim: SessionReplacementGoal) -> Result<u64, String> {
        if self.current.is_some() {
            return Err("session swap already in progress".into());
        }
        self.last_seq = self.last_seq.saturating_add(1);
        let seq = self.last_seq;
        perf::swap(seq, "requested", &aim.label());
        self.current = Some(Inflight {
            seq,
            goal: aim,
            step: Step::Fresh,
            cancel_load: false,
            world_was_torn: false,
        });
        Ok(seq)
    }

    /// Hands out the outcome of the swap that ended since the last call.
    pub fn take_completed(&mut self) -> Option<HandoverFinish> {
        self.outcome.take()
    }

    /// Id of the swap that is running right now.
    pub fn trace_dump_number(&self) -> Option<u64> {
        self.current.as_ref().map(|active| active.seq)
    }

    pub(super) fn in_flight(&self) -> bool {
        self.current.is_some()
    }

    fn conclude(&mut self, done: HandoverFinish) {
        perf::swap(done.id, "done", &done.result.label());
        self.current.take();
        self.outcome = Some(done);
    }
}

/// What the process turns into after the old world is gone, and what comes
/// after that.
#[derive(Debug, PartialEq, Eq)]
struct Handover {
    new_role: ProcessDuty,
    /// Also leave the lobby (a menu swap that asked for it, while in one).
    exit_lobby: bool,
    then: Followup,
}

#[derive(Debug, PartialEq, Eq)]
enum Followup {
    /// Nothing more to do; the swap is complete.
    Finished,
    /// Approve loading this map, then wait for the install.
    LoadMap { zone: String },
}

fn plan_handover(
    goal: &SessionReplacementGoal,
    current_role: Option<ProcessDuty>,
    in_lobby: bool,
) -> Handover {
    match goal {
        SessionReplacementGoal::Menu { drop_lobby } => Handover {
            new_role: ProcessDuty::Listen,
            exit_lobby: in_lobby && *drop_lobby,
            then: Followup::Finished,
        },
        SessionReplacementGoal::Scene(map) => Handover {
            // Remote clients stay clients; everyone else hosts the map locally.
            new_role: match current_role {
                Some(ProcessDuty::Client) => ProcessDuty::Client,
                _ => ProcessDuty::Listen,
            },
            exit_lobby: false,
            then: Followup::LoadMap { zone: map.clone() },
        },
        SessionReplacementGoal::Demo { map, .. } => Handover {
            new_role: ProcessDuty::Replay,
            exit_lobby: false,
            then: Followup::LoadMap { zone: map.clone() },
        },
    }
}

/// Identifies a load: in a hosted or joined match it follows the lobby's key,
/// otherwise it is only this process's own request.
fn swap_load_key(request_id: u64, master: Option<&net::DirectoryLink>) -> OwnLevelLoadTicket {
    let standalone = OwnLevelLoadTicket::from_request(request_id, GameSessionStamp::NONE, 0);
    let Some(master) = master else {
        return standalone;
    };
    let state = master.state();
    if matches!(state, net::DirectoryLinkStatus::Hosting { .. }) {
        return OwnLevelLoadTicket::from_request(request_id, GameSessionStamp::NONE, master.incarnation());
    }
    if let net::DirectoryLinkStatus::Joined {
        identity: lobby,
        playing: true,
        ..
    } = state
        && lobby.epoch != 0
    {
        let game_session_key = GameSessionStamp::new(lobby.lobby_number.0, lobby.epoch);
        return OwnLevelLoadTicket::from_request(request_id, game_session_key, master.incarnation());
    }
    standalone
}

/// Does a reported teardown satisfy a swap that waits for `wanted`? A menu
/// swap also accepts a plain replacement, since the world is gone either way.
fn teardown_matches(
    got: ShutdownCause,
    wanted: ShutdownCause,
    goal: &SessionReplacementGoal,
) -> bool {
    got == wanted || (got == ShutdownCause::Replaced && goal.is_menu())
}

pub(super) fn assign_role(
    runtime_role: &mut Option<ResMut<ProcessDuty>>,
    launch: &mut Option<ResMut<StartupPersona>>,
    new_role: ProcessDuty,
) {
    if let Some(current) = runtime_role.as_mut() {
        **current = new_role;
    }
    if let Some(launch) = launch.as_mut() {
        launch.duty_caption = format!("{new_role:?}");
    }
}

/// The step after the old world is gone: adopt the new role, then either
/// finish (menu) or approve the load and wait for the install.
fn take_over(
    active: &mut Inflight,
    runtime_role: &mut Option<ResMut<ProcessDuty>>,
    launch: &mut Option<ResMut<StartupPersona>>,
    approvals: &mut MessageWriter<LevelOpenGranted>,
    menu_returns: &mut MessageWriter<ReturnedToMenu>,
    lobby_exit: &mut Option<ResMut<net::QueuedMenuChoice>>,
    master: Option<&net::DirectoryLink>,
) -> Option<HandoverFinish> {
    let current_role = runtime_role.as_deref().copied();
    let handover = plan_handover(&active.goal, current_role, master.is_some());
    assign_role(runtime_role, launch, handover.new_role);
    if handover.exit_lobby
        && let Some(slot) = lobby_exit.as_mut()
        && slot.0.is_none()
    {
        slot.0 = Some(net::DirectoryMenuChoice::ExitRoom);
    }
    match handover.then {
        Followup::Finished => {
            menu_returns.write(ReturnedToMenu {
                swap_id: active.seq,
                had_world: active.world_was_torn,
            });
            Some(HandoverFinish {
                id: active.seq,
                result: HandoverVerdict::Menu,
            })
        }
        Followup::LoadMap { zone } => {
            approvals.write(LevelOpenGranted {
                ask_number: active.seq,
                decrypt_secret: swap_load_key(active.seq, master),
                zone,
            });
            active.step = Step::Installing;
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_session_swap(
    mut commands: Commands,
    mut swaps: ResMut<SessionHandoverOrder>,
    world_flag: Option<Res<LevelPresent>>,
    mut teardown_req: ResMut<ShutdownOrder>,
    mut runtime_role: Option<ResMut<ProcessDuty>>,
    mut launch: Option<ResMut<StartupPersona>>,
    mut approvals: MessageWriter<LevelOpenGranted>,
    mut torn_down: MessageReader<GameSessionClosed>,
    mut load_failures: MessageReader<LevelOpenRejected>,
    mut installs: MessageReader<GameSessionArmed>,
    mut menu_returns: MessageWriter<ReturnedToMenu>,
    mut lobby_exit: Option<ResMut<net::QueuedMenuChoice>>,
    master: Option<Res<net::DirectoryLink>>,
) {
    let Some(active) = swaps.current.as_mut() else {
        return;
    };
    if active.cancel_load {
        commands.insert_resource(assets::SessionLoadCancel(active.seq));
        active.cancel_load = false;
    }
    let wanted = active.goal.teardown_reason();
    let outcome = match active.step {
        Step::Fresh if world_flag.is_some_and(|flag| flag.0) => {
            teardown_req.0 = Some(wanted);
            active.world_was_torn = true;
            active.step = Step::TearingDown;
            None
        }
        Step::Fresh => take_over(
            active,
            &mut runtime_role,
            &mut launch,
            &mut approvals,
            &mut menu_returns,
            &mut lobby_exit,
            master.as_deref(),
        ),
        Step::TearingDown => {
            let goal = &active.goal;
            let satisfied = torn_down
                .read()
                .any(|event| teardown_matches(event.reason, wanted, goal));
            if satisfied {
                take_over(
                    active,
                    &mut runtime_role,
                    &mut launch,
                    &mut approvals,
                    &mut menu_returns,
                    &mut lobby_exit,
                    master.as_deref(),
                )
            } else {
                None
            }
        }
        // A menu swap has no load to wait for, so it completes right away.
        Step::Installing if active.goal.is_menu() => take_over(
            active,
            &mut runtime_role,
            &mut launch,
            &mut approvals,
            &mut menu_returns,
            &mut lobby_exit,
            master.as_deref(),
        ),
        Step::Installing => {
            if let Some(failure) = load_failures
                .read()
                .find(|failure| failure.ask_number == active.seq)
            {
                assign_role(&mut runtime_role, &mut launch, ProcessDuty::Listen);
                if let Some(launch) = launch.as_mut() {
                    launch.zone.clear();
                }
                menu_returns.write(ReturnedToMenu {
                    swap_id: active.seq,
                    had_world: active.world_was_torn,
                });
                diag::warn!(
                    Sim,
                    "session: map `{}` failed (swap #{}): {} - returned to menu",
                    failure.zone,
                    active.seq,
                    failure.error
                );
                Some(HandoverFinish {
                    id: active.seq,
                    result: HandoverVerdict::Failed {
                        zone: failure.zone.clone(),
                        error: failure.error.clone(),
                    },
                })
            } else {
                installs
                    .read()
                    .find(|event| event.ask_number == active.seq)
                    .map(|event| HandoverFinish {
                        id: active.seq,
                        result: HandoverVerdict::Installed {
                            zone: event.zone.clone(),
                        },
                    })
            }
        }
    };
    if let Some(done) = outcome {
        swaps.conclude(done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> SessionReplacementGoal {
        SessionReplacementGoal::Scene(name.into())
    }

    fn waiting_for_install(swap: &mut SessionHandoverOrder) {
        swap.current.as_mut().unwrap().step = Step::Installing;
    }

    #[test]
    fn swap_ids_count_up_from_one() {
        let mut swap = SessionHandoverOrder::default();
        assert_eq!(swap.ask_for_region("a".into()), Ok(1));
        swap.current = None;
        assert_eq!(swap.ask_for_region("b".into()), Ok(2));
    }

    #[test]
    fn a_second_swap_waits_for_the_first() {
        let mut swap = SessionHandoverOrder::default();
        swap.ask_for_region("a".into()).unwrap();
        assert!(swap.ask_for_region("b".into()).is_err());
        assert!(swap.ask_for_recording("d".into(), "z".into(), false).is_err());
        assert_eq!(swap.trace_dump_number(), Some(1));
    }

    #[test]
    fn the_first_load_is_let_through_before_any_swap() {
        let swap = SessionHandoverOrder::default();
        assert!(swap.takes_install(0));
        assert!(!swap.takes_install(1));
    }

    #[test]
    fn after_a_swap_ran_a_stray_load_is_refused() {
        let mut swap = SessionHandoverOrder::default();
        swap.ask_for_region("a".into()).unwrap();
        swap.current = None;
        assert!(!swap.takes_install(0));
        assert!(!swap.takes_install(1));
    }

    #[test]
    fn a_load_is_taken_only_once_the_swap_waits_for_it() {
        let mut swap = SessionHandoverOrder::default();
        let id = swap.ask_for_region("a".into()).unwrap();
        assert!(!swap.takes_install(id));
        waiting_for_install(&mut swap);
        assert!(swap.takes_install(id));
        assert!(!swap.takes_install(id + 1));
    }

    #[test]
    fn a_menu_swap_takes_no_load() {
        let mut swap = SessionHandoverOrder::default();
        let id = swap.ask_for_menu().unwrap();
        waiting_for_install(&mut swap);
        assert!(!swap.takes_install(id));
    }

    #[test]
    fn asking_for_the_menu_twice_keeps_the_swap() {
        let mut swap = SessionHandoverOrder::default();
        let first = swap.ask_for_menu().unwrap();
        assert_eq!(swap.ask_for_menu(), Ok(first));
        assert!(!swap.current.as_ref().unwrap().cancel_load);
    }

    #[test]
    fn leaving_upgrades_a_menu_swap_and_never_downgrades_it() {
        let mut swap = SessionHandoverOrder::default();
        swap.ask_for_menu().unwrap();
        swap.request_leave().unwrap();
        let target = |swap: &SessionHandoverOrder| swap.current.as_ref().unwrap().goal.clone();
        assert_eq!(
            target(&swap),
            SessionReplacementGoal::Menu {
                drop_lobby: true
            }
        );
        swap.ask_for_menu().unwrap();
        assert_eq!(
            target(&swap),
            SessionReplacementGoal::Menu {
                drop_lobby: true
            }
        );
    }

    #[test]
    fn the_menu_overrides_a_load_in_flight_and_stops_it() {
        let mut swap = SessionHandoverOrder::default();
        let id = swap.ask_for_region("a".into()).unwrap();
        assert_eq!(swap.request_leave(), Ok(id));
        let pending = swap.current.as_ref().unwrap();
        assert_eq!(
            pending.goal,
            SessionReplacementGoal::Menu {
                drop_lobby: true
            }
        );
        assert!(pending.cancel_load);
    }

    #[test]
    fn the_menu_overrides_a_demo_in_flight_too() {
        let mut swap = SessionHandoverOrder::default();
        swap.ask_for_recording("d".into(), "z".into(), true).unwrap();
        swap.ask_for_menu().unwrap();
        let pending = swap.current.as_ref().unwrap();
        assert!(pending.goal.is_menu());
        assert!(pending.cancel_load);
    }

    #[test]
    fn finishing_a_swap_frees_the_slot_and_keeps_the_outcome_once() {
        let mut swap = SessionHandoverOrder::default();
        let id = swap.ask_for_region("a".into()).unwrap();
        swap.conclude(HandoverFinish {
            id,
            result: HandoverVerdict::Installed { zone: "a".into() },
        });
        assert_eq!(swap.trace_dump_number(), None);
        assert!(swap.ask_for_menu().is_ok());
        let done = swap.take_completed().unwrap();
        assert_eq!(done.id, id);
        assert_eq!(swap.take_completed(), None);
    }

    #[test]
    fn each_target_has_its_own_teardown_reason() {
        assert_eq!(zone("a").teardown_reason(), ShutdownCause::Replaced);
        let demo = SessionReplacementGoal::Demo {
            demo: "d".into(),
            map: "z".into(),
            exit_when_done: false,
        };
        assert_eq!(demo.teardown_reason(), ShutdownCause::Replaced);
        let menu = |drop_lobby| SessionReplacementGoal::Menu { drop_lobby };
        assert_eq!(menu(false).teardown_reason(), ShutdownCause::RoundOver);
        assert_eq!(menu(true).teardown_reason(), ShutdownCause::Disconnect);
    }

    #[test]
    fn a_menu_swap_accepts_a_replacement_teardown_but_a_load_does_not_accept_others() {
        let menu = SessionReplacementGoal::Menu {
            drop_lobby: false,
        };
        assert!(teardown_matches(
            ShutdownCause::Replaced,
            ShutdownCause::RoundOver,
            &menu
        ));
        assert!(!teardown_matches(
            ShutdownCause::Disconnect,
            ShutdownCause::RoundOver,
            &menu
        ));
        assert!(!teardown_matches(
            ShutdownCause::RoundOver,
            ShutdownCause::Replaced,
            &zone("a")
        ));
        assert!(teardown_matches(
            ShutdownCause::Replaced,
            ShutdownCause::Replaced,
            &zone("a")
        ));
    }

    #[test]
    fn a_menu_swap_hands_the_process_back_to_its_own_listen_role() {
        let menu = SessionReplacementGoal::Menu {
            drop_lobby: false,
        };
        let plan = plan_handover(&menu, Some(ProcessDuty::Client), true);
        assert_eq!(plan.new_role, ProcessDuty::Listen);
        assert!(!plan.exit_lobby);
        assert_eq!(plan.then, Followup::Finished);
    }

    #[test]
    fn only_a_swap_that_leaves_and_has_a_lobby_leaves_it() {
        let leave = SessionReplacementGoal::Menu {
            drop_lobby: true,
        };
        assert!(plan_handover(&leave, None, true).exit_lobby);
        assert!(!plan_handover(&leave, None, false).exit_lobby);
    }

    #[test]
    fn a_remote_client_stays_a_client_across_maps() {
        let plan = plan_handover(&zone("a"), Some(ProcessDuty::Client), true);
        assert_eq!(plan.new_role, ProcessDuty::Client);
        assert_eq!(plan.then, Followup::LoadMap { zone: "a".into() });
        for current in [None, Some(ProcessDuty::Listen), Some(ProcessDuty::Replay)] {
            let handover = plan_handover(&zone("a"), current, false);
            assert_eq!(handover.new_role, ProcessDuty::Listen);
        }
    }

    #[test]
    fn a_demo_plays_as_a_replay() {
        let demo = SessionReplacementGoal::Demo {
            demo: "d".into(),
            map: "z".into(),
            exit_when_done: true,
        };
        let plan = plan_handover(&demo, Some(ProcessDuty::Client), true);
        assert_eq!(plan.new_role, ProcessDuty::Replay);
        assert_eq!(plan.then, Followup::LoadMap { zone: "z".into() });
    }

    #[test]
    fn without_a_lobby_a_load_is_known_by_its_request_alone() {
        assert_eq!(
            swap_load_key(7, None),
            OwnLevelLoadTicket::from_request(7, GameSessionStamp::NONE, 0)
        );
    }

    #[test]
    fn labels_name_the_target_and_the_outcome() {
        assert_eq!(zone("a").label(), "zone:a");
        let menu = |drop_lobby| SessionReplacementGoal::Menu { drop_lobby };
        assert_eq!(menu(true).label(), "leave");
        assert_eq!(menu(false).label(), "menu");
        assert_eq!(HandoverVerdict::Menu.label(), "menu");
        assert_eq!(
            HandoverVerdict::Failed {
                zone: "a".into(),
                error: "x".into()
            }
            .label(),
            "failed:a"
        );
    }
}
