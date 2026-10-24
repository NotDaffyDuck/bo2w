//! Changing what this process is showing: a map, a demo, or the menu. One
//! swap is in flight at a time; it tears the old world down, says which role
//! this process plays next, and waits for the new world to install (or fail).

use bevy::prelude::*;
use frame::{
    HasWorld, LaunchIdentity, LocalLoadKey, MapLoadApproved, MapLoadFailed, MatchInstalled,
    MatchKey, MatchTornDown, ReturnedToMenu, RuntimeRole, TeardownReason,
};

use super::TeardownRequest;

/// Where a swap is headed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSwapTarget {
    /// Load a map and play it.
    Zone(String),
    /// Play a recorded match back on its map.
    Demo {
        name: String,
        zone: String,
        quit_on_end: bool,
    },
    /// Go back to the front end; `leave_session` also leaves the lobby.
    Menu { leave_session: bool },
}

impl SessionSwapTarget {
    /// The short name the swap goes by in the perf log.
    fn label(&self) -> String {
        match self {
            Self::Zone(zone) => format!("zone:{zone}"),
            Self::Demo { name, .. } => format!("demo:{name}"),
            Self::Menu {
                leave_session: true,
            } => "leave".into(),
            Self::Menu { .. } => "menu".into(),
        }
    }

    /// Why the old world is being torn down for this target.
    fn teardown_reason(&self) -> TeardownReason {
        match self {
            Self::Menu {
                leave_session: true,
            } => TeardownReason::Disconnect,
            Self::Menu { .. } => TeardownReason::MatchEnded,
            Self::Zone(_) | Self::Demo { .. } => TeardownReason::Replaced,
        }
    }

    fn is_menu(&self) -> bool {
        matches!(self, Self::Menu { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Requested,
    WaitingForTeardown,
    WaitingForInstall,
}

#[derive(Debug)]
struct Pending {
    id: u64,
    target: SessionSwapTarget,
    phase: Phase,
    /// The load already started for this swap has to be stopped.
    abort_install: bool,
    tore_down_world: bool,
}

/// How a finished swap ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSwapResult {
    Installed { zone: String },
    Menu,
    Failed { zone: String, error: String },
}

impl SessionSwapResult {
    fn label(&self) -> String {
        match self {
            Self::Installed { zone } => format!("installed:{zone}"),
            Self::Menu => "menu".into(),
            Self::Failed { zone, .. } => format!("failed:{zone}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSwapCompletion {
    pub id: u64,
    pub result: SessionSwapResult,
}

/// Asks for a swap and hands back how the last one ended.
#[derive(Resource, Default, Debug)]
pub struct SessionSwapRequest {
    next_id: u64,
    pending: Option<Pending>,
    completed: Option<SessionSwapCompletion>,
}

impl SessionSwapRequest {
    /// Whether an install carrying `request_id` is the one this swap waits
    /// for. With no swap running, only the very first load (id 0, before any
    /// swap was ever asked for) is let through.
    pub(crate) fn accepts_install(&self, request_id: u64) -> bool {
        match &self.pending {
            Some(pending) => {
                pending.id == request_id
                    && pending.phase == Phase::WaitingForInstall
                    && !pending.target.is_menu()
            }
            None => request_id == 0 && self.next_id == 0,
        }
    }

    pub fn request_zone(&mut self, zone: String) -> Result<u64, String> {
        self.begin(SessionSwapTarget::Zone(zone))
    }

    pub fn request_demo(
        &mut self,
        name: String,
        zone: String,
        quit_on_end: bool,
    ) -> Result<u64, String> {
        self.begin(SessionSwapTarget::Demo {
            name,
            zone,
            quit_on_end,
        })
    }

    /// Back to the menu, staying in the lobby.
    pub fn request_menu(&mut self) -> Result<u64, String> {
        self.request_return(false)
    }

    /// Back to the menu and out of the lobby.
    pub fn request_leave(&mut self) -> Result<u64, String> {
        self.request_return(true)
    }

    /// Going back to the menu wins over a load still in flight: the swap
    /// changes its target and stops the load instead of queueing behind it.
    fn request_return(&mut self, leave_session: bool) -> Result<u64, String> {
        let Some(pending) = self.pending.as_mut() else {
            return self.begin(SessionSwapTarget::Menu { leave_session });
        };
        let verb = if leave_session {
            "disconnect"
        } else {
            "match end"
        };
        let superseded = match &pending.target {
            SessionSwapTarget::Menu { .. } => None,
            SessionSwapTarget::Zone(zone) => Some(format!("map `{zone}`")),
            SessionSwapTarget::Demo { name, .. } => Some(format!("demo `{name}`")),
        };
        match (&mut pending.target, superseded) {
            (
                SessionSwapTarget::Menu {
                    leave_session: held,
                },
                _,
            ) => {
                *held = *held || leave_session;
            }
            (target, Some(what)) => {
                diag::info!(
                    Sim,
                    "session: {verb} superseded in-flight {what} (swap #{}) -> menu",
                    pending.id
                );
                *target = SessionSwapTarget::Menu { leave_session };
                pending.abort_install = true;
            }
            (_, None) => {}
        }
        Ok(pending.id)
    }

    fn begin(&mut self, target: SessionSwapTarget) -> Result<u64, String> {
        if self.pending.is_some() {
            return Err("session swap already in progress".into());
        }
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        perf::swap(id, "requested", &target.label());
        self.pending = Some(Pending {
            id,
            target,
            phase: Phase::Requested,
            abort_install: false,
            tore_down_world: false,
        });
        Ok(id)
    }

    /// The swap that ended since the last call, if any.
    pub fn take_completed(&mut self) -> Option<SessionSwapCompletion> {
        self.completed.take()
    }

    /// The id of the swap in flight.
    pub fn dump_id(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.id)
    }

    pub(super) fn in_flight(&self) -> bool {
        self.pending.is_some()
    }

    fn finish(&mut self, completed: SessionSwapCompletion) {
        perf::swap(completed.id, "done", &completed.result.label());
        self.pending = None;
        self.completed = Some(completed);
    }
}

/// What this process becomes once the old world is gone, and what happens
/// next.
#[derive(Debug, PartialEq, Eq)]
struct Occupation {
    role: RuntimeRole,
    /// Leave the lobby too (a menu swap that asked for it, with a lobby to leave).
    leave_lobby: bool,
    next: Next,
}

#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// The swap is over.
    Done,
    /// Approve the load of this map and wait for it.
    Load { zone: String },
}

fn occupation(
    target: &SessionSwapTarget,
    current: Option<RuntimeRole>,
    in_lobby: bool,
) -> Occupation {
    match target {
        SessionSwapTarget::Menu { leave_session } => Occupation {
            role: RuntimeRole::Listen,
            leave_lobby: *leave_session && in_lobby,
            next: Next::Done,
        },
        SessionSwapTarget::Zone(zone) => Occupation {
            // A remote client stays one; anyone else hosts their own map.
            role: if current == Some(RuntimeRole::Client) {
                RuntimeRole::Client
            } else {
                RuntimeRole::Listen
            },
            leave_lobby: false,
            next: Next::Load { zone: zone.clone() },
        },
        SessionSwapTarget::Demo { zone, .. } => Occupation {
            role: RuntimeRole::Replay,
            leave_lobby: false,
            next: Next::Load { zone: zone.clone() },
        },
    }
}

/// The key a load is known by: a hosted or joined match is named after the
/// lobby's, anything else is just this process's own request.
fn load_key_for_swap(request_id: u64, bridge: Option<&net::MasterBridge>) -> LocalLoadKey {
    let own = LocalLoadKey::from_request(request_id, MatchKey::NONE, 0);
    let Some(bridge) = bridge else {
        return own;
    };
    match bridge.state() {
        net::MasterBridgeState::Hosting { .. } => {
            LocalLoadKey::from_request(request_id, MatchKey::NONE, bridge.incarnation())
        }
        net::MasterBridgeState::Joined {
            identity,
            in_match: true,
            ..
        } if identity.epoch != 0 => LocalLoadKey::from_request(
            request_id,
            MatchKey::new(identity.room_id.0, identity.epoch),
            bridge.incarnation(),
        ),
        _ => own,
    }
}

/// A teardown report that satisfies a swap waiting for `expected`. A menu
/// swap also takes a plain replacement: the world went away either way.
fn teardown_satisfies(
    reported: TeardownReason,
    expected: TeardownReason,
    target: &SessionSwapTarget,
) -> bool {
    reported == expected || (target.is_menu() && reported == TeardownReason::Replaced)
}

pub(super) fn set_role(
    role: &mut Option<ResMut<RuntimeRole>>,
    identity: &mut Option<ResMut<LaunchIdentity>>,
    next: RuntimeRole,
) {
    if let Some(role) = role.as_mut() {
        **role = next;
    }
    if let Some(identity) = identity.as_mut() {
        identity.role_label = format!("{next:?}");
    }
}

/// The part of a swap after the old world is gone: take the new role, then
/// either finish (menu) or approve the load and wait for it.
fn occupy(
    pending: &mut Pending,
    role: &mut Option<ResMut<RuntimeRole>>,
    identity: &mut Option<ResMut<LaunchIdentity>>,
    approved: &mut MessageWriter<MapLoadApproved>,
    menu: &mut MessageWriter<ReturnedToMenu>,
    leave: &mut Option<ResMut<net::PendingMasterMenuAction>>,
    bridge: Option<&net::MasterBridge>,
) -> Option<SessionSwapCompletion> {
    let current = role.as_deref().copied();
    let plan = occupation(&pending.target, current, bridge.is_some());
    set_role(role, identity, plan.role);
    if plan.leave_lobby
        && let Some(action) = leave.as_mut()
        && action.0.is_none()
    {
        action.0 = Some(net::MasterMenuAction::LeaveLobby);
    }
    match plan.next {
        Next::Done => {
            menu.write(ReturnedToMenu {
                swap_id: pending.id,
                had_world: pending.tore_down_world,
            });
            Some(SessionSwapCompletion {
                id: pending.id,
                result: SessionSwapResult::Menu,
            })
        }
        Next::Load { zone } => {
            approved.write(MapLoadApproved {
                request_id: pending.id,
                load_key: load_key_for_swap(pending.id, bridge),
                zone,
            });
            pending.phase = Phase::WaitingForInstall;
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_session_swap(
    mut commands: Commands,
    mut swap: ResMut<SessionSwapRequest>,
    has_world: Option<Res<HasWorld>>,
    mut teardown: ResMut<TeardownRequest>,
    mut role: Option<ResMut<RuntimeRole>>,
    mut identity: Option<ResMut<LaunchIdentity>>,
    mut approved: MessageWriter<MapLoadApproved>,
    mut torn: MessageReader<MatchTornDown>,
    mut failed: MessageReader<MapLoadFailed>,
    mut installed: MessageReader<MatchInstalled>,
    mut menu: MessageWriter<ReturnedToMenu>,
    mut leave: Option<ResMut<net::PendingMasterMenuAction>>,
    bridge: Option<Res<net::MasterBridge>>,
) {
    let Some(pending) = swap.pending.as_mut() else {
        return;
    };
    if pending.abort_install {
        commands.insert_resource(assets::MatchLoadAbort(pending.id));
        pending.abort_install = false;
    }
    let expected = pending.target.teardown_reason();
    let finished = match pending.phase {
        Phase::Requested if has_world.is_some_and(|world| world.0) => {
            teardown.0 = Some(expected);
            pending.tore_down_world = true;
            pending.phase = Phase::WaitingForTeardown;
            None
        }
        Phase::Requested => occupy(
            pending,
            &mut role,
            &mut identity,
            &mut approved,
            &mut menu,
            &mut leave,
            bridge.as_deref(),
        ),
        Phase::WaitingForTeardown => {
            let target = &pending.target;
            if torn
                .read()
                .any(|fact| teardown_satisfies(fact.reason, expected, target))
            {
                occupy(
                    pending,
                    &mut role,
                    &mut identity,
                    &mut approved,
                    &mut menu,
                    &mut leave,
                    bridge.as_deref(),
                )
            } else {
                None
            }
        }
        // Nothing is loading for a menu swap; it finishes on the spot.
        Phase::WaitingForInstall if pending.target.is_menu() => occupy(
            pending,
            &mut role,
            &mut identity,
            &mut approved,
            &mut menu,
            &mut leave,
            bridge.as_deref(),
        ),
        Phase::WaitingForInstall => {
            if let Some(fact) = failed.read().find(|fact| fact.request_id == pending.id) {
                set_role(&mut role, &mut identity, RuntimeRole::Listen);
                if let Some(identity) = identity.as_mut() {
                    identity.zone.clear();
                }
                menu.write(ReturnedToMenu {
                    swap_id: pending.id,
                    had_world: pending.tore_down_world,
                });
                diag::warn!(
                    Sim,
                    "session: map `{}` failed (swap #{}): {} - returned to menu",
                    fact.zone,
                    pending.id,
                    fact.error
                );
                Some(SessionSwapCompletion {
                    id: pending.id,
                    result: SessionSwapResult::Failed {
                        zone: fact.zone.clone(),
                        error: fact.error.clone(),
                    },
                })
            } else {
                installed
                    .read()
                    .find(|fact| fact.request_id == pending.id)
                    .map(|fact| SessionSwapCompletion {
                        id: pending.id,
                        result: SessionSwapResult::Installed {
                            zone: fact.zone.clone(),
                        },
                    })
            }
        }
    };
    if let Some(completed) = finished {
        swap.finish(completed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> SessionSwapTarget {
        SessionSwapTarget::Zone(name.into())
    }

    fn waiting_for_install(swap: &mut SessionSwapRequest) {
        swap.pending.as_mut().unwrap().phase = Phase::WaitingForInstall;
    }

    #[test]
    fn swap_ids_count_up_from_one() {
        let mut swap = SessionSwapRequest::default();
        assert_eq!(swap.request_zone("a".into()), Ok(1));
        swap.pending = None;
        assert_eq!(swap.request_zone("b".into()), Ok(2));
    }

    #[test]
    fn a_second_swap_waits_for_the_first() {
        let mut swap = SessionSwapRequest::default();
        swap.request_zone("a".into()).unwrap();
        assert!(swap.request_zone("b".into()).is_err());
        assert!(swap.request_demo("d".into(), "z".into(), false).is_err());
        assert_eq!(swap.dump_id(), Some(1));
    }

    #[test]
    fn the_first_load_is_let_through_before_any_swap() {
        let swap = SessionSwapRequest::default();
        assert!(swap.accepts_install(0));
        assert!(!swap.accepts_install(1));
    }

    #[test]
    fn after_a_swap_ran_a_stray_load_is_refused() {
        let mut swap = SessionSwapRequest::default();
        swap.request_zone("a".into()).unwrap();
        swap.pending = None;
        assert!(!swap.accepts_install(0));
        assert!(!swap.accepts_install(1));
    }

    #[test]
    fn a_load_is_taken_only_once_the_swap_waits_for_it() {
        let mut swap = SessionSwapRequest::default();
        let id = swap.request_zone("a".into()).unwrap();
        assert!(!swap.accepts_install(id));
        waiting_for_install(&mut swap);
        assert!(swap.accepts_install(id));
        assert!(!swap.accepts_install(id + 1));
    }

    #[test]
    fn a_menu_swap_takes_no_load() {
        let mut swap = SessionSwapRequest::default();
        let id = swap.request_menu().unwrap();
        waiting_for_install(&mut swap);
        assert!(!swap.accepts_install(id));
    }

    #[test]
    fn asking_for_the_menu_twice_keeps_the_swap() {
        let mut swap = SessionSwapRequest::default();
        let first = swap.request_menu().unwrap();
        assert_eq!(swap.request_menu(), Ok(first));
        assert!(!swap.pending.as_ref().unwrap().abort_install);
    }

    #[test]
    fn leaving_upgrades_a_menu_swap_and_never_downgrades_it() {
        let mut swap = SessionSwapRequest::default();
        swap.request_menu().unwrap();
        swap.request_leave().unwrap();
        let target = |swap: &SessionSwapRequest| swap.pending.as_ref().unwrap().target.clone();
        assert_eq!(
            target(&swap),
            SessionSwapTarget::Menu {
                leave_session: true
            }
        );
        swap.request_menu().unwrap();
        assert_eq!(
            target(&swap),
            SessionSwapTarget::Menu {
                leave_session: true
            }
        );
    }

    #[test]
    fn the_menu_overrides_a_load_in_flight_and_stops_it() {
        let mut swap = SessionSwapRequest::default();
        let id = swap.request_zone("a".into()).unwrap();
        assert_eq!(swap.request_leave(), Ok(id));
        let pending = swap.pending.as_ref().unwrap();
        assert_eq!(
            pending.target,
            SessionSwapTarget::Menu {
                leave_session: true
            }
        );
        assert!(pending.abort_install);
    }

    #[test]
    fn the_menu_overrides_a_demo_in_flight_too() {
        let mut swap = SessionSwapRequest::default();
        swap.request_demo("d".into(), "z".into(), true).unwrap();
        swap.request_menu().unwrap();
        let pending = swap.pending.as_ref().unwrap();
        assert!(pending.target.is_menu());
        assert!(pending.abort_install);
    }

    #[test]
    fn finishing_a_swap_frees_the_slot_and_keeps_the_outcome_once() {
        let mut swap = SessionSwapRequest::default();
        let id = swap.request_zone("a".into()).unwrap();
        swap.finish(SessionSwapCompletion {
            id,
            result: SessionSwapResult::Installed { zone: "a".into() },
        });
        assert_eq!(swap.dump_id(), None);
        assert!(swap.request_menu().is_ok());
        let done = swap.take_completed().unwrap();
        assert_eq!(done.id, id);
        assert_eq!(swap.take_completed(), None);
    }

    #[test]
    fn each_target_has_its_own_teardown_reason() {
        assert_eq!(zone("a").teardown_reason(), TeardownReason::Replaced);
        let demo = SessionSwapTarget::Demo {
            name: "d".into(),
            zone: "z".into(),
            quit_on_end: false,
        };
        assert_eq!(demo.teardown_reason(), TeardownReason::Replaced);
        let menu = |leave_session| SessionSwapTarget::Menu { leave_session };
        assert_eq!(menu(false).teardown_reason(), TeardownReason::MatchEnded);
        assert_eq!(menu(true).teardown_reason(), TeardownReason::Disconnect);
    }

    #[test]
    fn a_menu_swap_accepts_a_replacement_teardown_but_a_load_does_not_accept_others() {
        let menu = SessionSwapTarget::Menu {
            leave_session: false,
        };
        assert!(teardown_satisfies(
            TeardownReason::Replaced,
            TeardownReason::MatchEnded,
            &menu
        ));
        assert!(!teardown_satisfies(
            TeardownReason::Disconnect,
            TeardownReason::MatchEnded,
            &menu
        ));
        assert!(!teardown_satisfies(
            TeardownReason::MatchEnded,
            TeardownReason::Replaced,
            &zone("a")
        ));
        assert!(teardown_satisfies(
            TeardownReason::Replaced,
            TeardownReason::Replaced,
            &zone("a")
        ));
    }

    #[test]
    fn a_menu_swap_hands_the_process_back_to_its_own_listen_role() {
        let menu = SessionSwapTarget::Menu {
            leave_session: false,
        };
        let plan = occupation(&menu, Some(RuntimeRole::Client), true);
        assert_eq!(plan.role, RuntimeRole::Listen);
        assert!(!plan.leave_lobby);
        assert_eq!(plan.next, Next::Done);
    }

    #[test]
    fn only_a_swap_that_leaves_and_has_a_lobby_leaves_it() {
        let leave = SessionSwapTarget::Menu {
            leave_session: true,
        };
        assert!(occupation(&leave, None, true).leave_lobby);
        assert!(!occupation(&leave, None, false).leave_lobby);
    }

    #[test]
    fn a_remote_client_stays_a_client_across_maps() {
        let plan = occupation(&zone("a"), Some(RuntimeRole::Client), true);
        assert_eq!(plan.role, RuntimeRole::Client);
        assert_eq!(plan.next, Next::Load { zone: "a".into() });
        for current in [None, Some(RuntimeRole::Listen), Some(RuntimeRole::Replay)] {
            assert_eq!(
                occupation(&zone("a"), current, false).role,
                RuntimeRole::Listen
            );
        }
    }

    #[test]
    fn a_demo_plays_as_a_replay() {
        let demo = SessionSwapTarget::Demo {
            name: "d".into(),
            zone: "z".into(),
            quit_on_end: true,
        };
        let plan = occupation(&demo, Some(RuntimeRole::Client), true);
        assert_eq!(plan.role, RuntimeRole::Replay);
        assert_eq!(plan.next, Next::Load { zone: "z".into() });
    }

    #[test]
    fn without_a_lobby_a_load_is_known_by_its_request_alone() {
        assert_eq!(
            load_key_for_swap(7, None),
            LocalLoadKey::from_request(7, MatchKey::NONE, 0)
        );
    }

    #[test]
    fn labels_name_the_target_and_the_outcome() {
        assert_eq!(zone("a").label(), "zone:a");
        let menu = |leave_session| SessionSwapTarget::Menu { leave_session };
        assert_eq!(menu(true).label(), "leave");
        assert_eq!(menu(false).label(), "menu");
        assert_eq!(SessionSwapResult::Menu.label(), "menu");
        assert_eq!(
            SessionSwapResult::Failed {
                zone: "a".into(),
                error: "x".into()
            }
            .label(),
            "failed:a"
        );
    }
}
