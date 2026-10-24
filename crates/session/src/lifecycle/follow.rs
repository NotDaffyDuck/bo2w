//! Following the lobby into a match: when the master says a match has
//! started, load its map, after clearing out whatever this process is running.

use bevy::prelude::*;
use frame::{HasWorld, LaunchIdentity, MatchKey, RuntimeRole};
use net::{MasterBridge, MasterBridgeState, MasterMatchOffer, MasterMatchStart};

use super::{LiveWorldIdentity, SessionSwapRequest};

/// What the lobby says about the match the offer is for.
struct Lobby {
    in_match: bool,
    match_key: MatchKey,
    hosting: bool,
}

/// Everything a decision about one offer depends on.
struct Facts<'a> {
    offer_key: MatchKey,
    offer_map: &'a str,
    lobby: Option<Lobby>,
    has_world: bool,
    live_key: MatchKey,
    launch_zone: &'a str,
    /// A match load is under way or ready to install.
    loading: bool,
    /// A session swap is in flight.
    swap_pending: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// The offer is stale or already satisfied.
    Forget,
    /// Something else is running: stop it and go to the menu first.
    Retire,
    /// The menu is on its way; ask again next frame.
    Wait,
    /// Nothing is in the way: start the load.
    Load { hosting: bool },
}

fn follow_step(facts: &Facts, retiring: bool) -> Step {
    let Some(lobby) = &facts.lobby else {
        return Step::Forget;
    };
    if !lobby.in_match || lobby.match_key != facts.offer_key {
        return Step::Forget;
    }
    // A host that started its own match already runs it, before the lobby's
    // key has reached the world.
    let already_here = facts.has_world
        && (facts.live_key == facts.offer_key
            || (lobby.hosting && facts.live_key.is_none() && facts.launch_zone == facts.offer_map));
    if already_here {
        return Step::Forget;
    }
    let in_the_way = facts.has_world || facts.loading || facts.swap_pending;
    match (in_the_way, retiring) {
        (true, false) => Step::Retire,
        (true, true) => Step::Wait,
        (false, _) => Step::Load {
            hosting: lobby.hosting,
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn follow_master_match(
    mut commands: Commands,
    mut offers: ResMut<MasterMatchStart>,
    bridge: Option<Res<MasterBridge>>,
    identity: Res<LaunchIdentity>,
    live: Res<LiveWorldIdentity>,
    has_world: Res<HasWorld>,
    busy: Option<Res<assets::MatchLoadBusy>>,
    request: Option<Res<assets::MatchLoadRequest>>,
    ready: Option<Res<assets::PreparedMatchReady>>,
    mut swap: ResMut<SessionSwapRequest>,
    mut role: ResMut<RuntimeRole>,
    mut pending: Local<Option<(MasterMatchOffer, bool)>>,
) {
    if let Some(offer) = offers.take() {
        *pending = Some((offer, false));
    }
    let Some((offer, retiring)) = pending.as_mut() else {
        return;
    };
    let state = bridge.as_ref().map(|bridge| bridge.state());
    let facts = Facts {
        offer_key: offer.match_key,
        offer_map: &offer.map,
        lobby: state.as_ref().map(|state| Lobby {
            in_match: state.in_match(),
            match_key: state.identity().match_key(),
            hosting: matches!(state, MasterBridgeState::Hosting { .. }),
        }),
        has_world: has_world.0,
        live_key: live.load_key.match_key,
        launch_zone: &identity.zone,
        loading: busy.is_some_and(|busy| busy.0) || request.is_some() || ready.is_some(),
        swap_pending: swap.in_flight(),
    };
    match follow_step(&facts, *retiring) {
        Step::Forget => *pending = None,
        Step::Wait => {}
        Step::Retire => {
            // Whatever load is under way is for a match that no longer
            // matters.
            let id = ready
                .as_ref()
                .map(|ready| ready.request_id)
                .or_else(|| request.as_ref().map(|request| request.request_id))
                .or_else(|| swap.dump_id())
                .unwrap_or(0);
            commands.insert_resource(assets::MatchLoadAbort(id));
            match swap.request_menu() {
                Ok(_) => *retiring = true,
                Err(error) => diag::warn!(Net, "master match teardown refused: {error}"),
            }
        }
        Step::Load { hosting } => {
            let Some(mode) = sim::HostGameModeSelection::from_token(&offer.mode) else {
                diag::warn!(Net, "master match mode refused: {}", offer.mode);
                *pending = None;
                return;
            };
            *role = if hosting {
                RuntimeRole::Listen
            } else {
                RuntimeRole::Client
            };
            commands.insert_resource(mode);
            match swap.request_zone(offer.map.clone()) {
                Ok(id) => {
                    if let Some(bridge) = bridge.as_ref() {
                        diag::info!(
                            Net,
                            "master match {}:{} -> session load #{id} {}",
                            bridge.incarnation(),
                            offer.match_key.match_epoch,
                            offer.map
                        );
                    }
                    *pending = None;
                }
                Err(error) => diag::warn!(Net, "master match load refused: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(epoch: u32) -> MatchKey {
        MatchKey::new([7; 16], epoch)
    }

    fn lobby(epoch: u32, hosting: bool) -> Option<Lobby> {
        Some(Lobby {
            in_match: true,
            match_key: key(epoch),
            hosting,
        })
    }

    /// A joined client at the menu with nothing loading, offered match 1.
    fn idle() -> Facts<'static> {
        Facts {
            offer_key: key(1),
            offer_map: "t6:mp_x",
            lobby: lobby(1, false),
            has_world: false,
            live_key: MatchKey::NONE,
            launch_zone: "",
            loading: false,
            swap_pending: false,
        }
    }

    #[test]
    fn a_free_client_loads_the_offered_match() {
        assert_eq!(follow_step(&idle(), false), Step::Load { hosting: false });
    }

    #[test]
    fn a_free_host_loads_as_host() {
        let facts = Facts {
            lobby: lobby(1, true),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Load { hosting: true });
    }

    #[test]
    fn an_offer_with_no_lobby_is_forgotten() {
        let facts = Facts {
            lobby: None,
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn an_offer_for_a_match_the_lobby_has_left_is_forgotten() {
        let facts = Facts {
            lobby: Some(Lobby {
                in_match: false,
                match_key: key(1),
                hosting: false,
            }),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn an_offer_for_an_older_match_is_forgotten() {
        let facts = Facts {
            lobby: lobby(2, false),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn the_world_already_from_this_match_satisfies_the_offer() {
        let facts = Facts {
            has_world: true,
            live_key: key(1),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn a_hosts_own_world_on_the_same_map_satisfies_the_offer_before_it_has_a_key() {
        let facts = Facts {
            lobby: lobby(1, true),
            has_world: true,
            launch_zone: "t6:mp_x",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn a_client_does_not_take_an_unkeyed_world_for_the_match() {
        let facts = Facts {
            has_world: true,
            launch_zone: "t6:mp_x",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_hosts_world_on_another_map_is_not_the_match() {
        let facts = Facts {
            lobby: lobby(1, true),
            has_world: true,
            launch_zone: "t6:mp_other",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_world_from_another_match_is_retired_first() {
        let facts = Facts {
            has_world: true,
            live_key: key(0),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_load_in_progress_is_retired_first() {
        let facts = Facts {
            loading: true,
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_swap_in_flight_is_retired_first() {
        let facts = Facts {
            swap_pending: true,
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn once_retiring_it_waits_for_the_menu() {
        let facts = Facts {
            has_world: true,
            live_key: key(0),
            swap_pending: true,
            ..idle()
        };
        assert_eq!(follow_step(&facts, true), Step::Wait);
    }

    #[test]
    fn once_the_menu_is_reached_it_loads_even_though_it_was_retiring() {
        assert_eq!(follow_step(&idle(), true), Step::Load { hosting: false });
    }
}
