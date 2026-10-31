//! Following the lobby into a match: when the master says a match has
//! started, load its map, after clearing out whatever this process is running.

use bevy::prelude::*;
use frame::{LevelPresent, StartupPersona, GameSessionStamp, ProcessDuty};
use net::{DirectoryLink, DirectoryLinkStatus, DirectoryGameOffer, DirectoryMatchBegin};
use super::{RunningWorldLabel, SessionHandoverOrder};


/// What the lobby says about the match the offer is for.
struct Lobby {
    in_play: bool,
    game_session_key: GameSessionStamp,
    hosting: bool,
}

/// Everything a decision about one offer depends on.
struct Facts<'a> {
    offer_key: GameSessionStamp,
    offer_map: &'a str,
    lobby: Option<Lobby>,
    level_is_up: bool,
    live_key: GameSessionStamp,
    launch_zone: &'a str,
    /// A match load is under way or ready to install.
    loading_now: bool,
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

fn follow_step(facts: &Facts, already_retiring: bool) -> Step {
    let Some(lobby) = facts.lobby.as_ref() else {
        return Step::Forget;
    };
    let lobby_agrees = lobby.in_play && lobby.game_session_key == facts.offer_key;
    if !lobby_agrees {
        return Step::Forget;
    }
    // A host that started its own match already runs it, before the lobby's
    // key has reached the world.
    let same_key = facts.live_key == facts.offer_key;
    let own_unkeyed =
        lobby.hosting && facts.live_key.is_none() && facts.launch_zone == facts.offer_map;
    if facts.level_is_up && (same_key || own_unkeyed) {
        return Step::Forget;
    }
    if !(facts.level_is_up || facts.loading_now || facts.swap_pending) {
        return Step::Load {
            hosting: lobby.hosting,
        };
    }
    if already_retiring {
        Step::Wait
    } else {
        Step::Retire
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn follow_master_match(
    mut commands: Commands,
    mut inbox: ResMut<DirectoryMatchBegin>,
    bridge: Option<Res<DirectoryLink>>,
    launch: Res<StartupPersona>,
    on_screen: Res<RunningWorldLabel>,
    world_flag: Res<LevelPresent>,
    load_busy: Option<Res<assets::SessionLoadOccupied>>,
    load_request: Option<Res<assets::SessionLoadOrder>>,
    load_ready: Option<Res<assets::ReadiedSessionSignal>>,
    mut swap: ResMut<SessionHandoverOrder>,
    mut role: ResMut<ProcessDuty>,
    mut followed: Local<Option<(DirectoryGameOffer, bool)>>,
) {
    if let Some(fresh) = inbox.take() {
        *followed = Some((fresh, false));
    }
    let Some((ask, retiring)) = followed.as_mut() else {
        return;
    };
    let lobby_state = bridge.as_ref().map(|link| link.state());
    let loading_now =
        load_busy.is_some_and(|busy| busy.0) || load_request.is_some() || load_ready.is_some();
    let facts = Facts {
        offer_key: ask.game_session_key,
        offer_map: &ask.map,
        lobby: lobby_state.as_ref().map(|state| Lobby {
            in_play: state.underway(),
            game_session_key: state.identity().game_session_key(),
            hosting: matches!(state, DirectoryLinkStatus::Hosting { .. }),
        }),
        level_is_up: world_flag.0,
        live_key: on_screen.decrypt_secret.game_session_key,
        launch_zone: &launch.zone,
        loading_now,
        swap_pending: swap.in_flight(),
    };
    match follow_step(&facts, *retiring) {
        Step::Forget => *followed = None,
        Step::Wait => {}
        Step::Retire => {
            // Whatever load is under way is for a match that no longer
            // matters.
            let abort_id = load_ready
                .as_ref()
                .map(|loaded| loaded.ticket)
                .or_else(|| load_request.as_ref().map(|asked| asked.ticket))
                .or_else(|| swap.trace_dump_number())
                .unwrap_or(0);
            commands.insert_resource(assets::SessionLoadCancel(abort_id));
            match swap.ask_for_menu() {
                Ok(_) => *retiring = true,
                Err(why) => diag::warn!(WireLink, "master match teardown refused: {why}"),
            }
        }
        Step::Load { hosting } => {
            // bo2zm: a Black Ops II Zombies lobby's mode is `gametype:location` (`zclassic:transit`);
            // everyone in it starts there (free-for-all underneath, as every Zombies map loads). A
            // party's own lobby (Custom Games) adds `:private`, so Public Match never joins it.
            let token = match ask.mode.split_once(':') {
                Some((gametype, rest)) if ask.map.starts_with("t6:") => {
                    let location = rest.split_once(':').map_or(rest, |(location, _)| location);
                    frame::set_bo2_zm_start(&ask.map["t6:".len()..], location, gametype);
                    "dm"
                }
                _ => ask.mode.as_str(),
            };
            let Some(selection) = sim::HostChosenMode::from_short_code(token) else {
                diag::warn!(WireLink, "master match mode refused: {mode}", mode = ask.mode);
                *followed = None;
                return;
            };
            *role = match hosting {
                true => ProcessDuty::Listen,
                false => ProcessDuty::Client,
            };
            commands.insert_resource(selection);
            let load_id = match swap.ask_for_region(ask.map.clone()) {
                Ok(load_id) => load_id,
                Err(why) => {
                    diag::warn!(WireLink, "master match load refused: {why}");
                    return;
                }
            };
            if let Some(link) = bridge.as_ref() {
                let (incarnation, epoch) = (link.incarnation(), ask.game_session_key.game_generation);
                diag::info!(
                    WireLink,
                    "master match {incarnation}:{epoch} -> session load #{load_id} {}",
                    ask.map
                );
            }
            *followed = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(generation: u32) -> GameSessionStamp {
        GameSessionStamp::new([7; 16], generation)
    }

    fn lobby(generation: u32, hosting: bool) -> Option<Lobby> {
        Some(Lobby {
            in_play: true,
            game_session_key: key(generation),
            hosting,
        })
    }

    /// A joined client at the menu with nothing loading, offered match 1.
    fn idle() -> Facts<'static> {
        Facts {
            offer_key: key(1),
            offer_map: "t6:mp_x",
            lobby: lobby(1, false),
            level_is_up: false,
            live_key: GameSessionStamp::NONE,
            launch_zone: "",
            loading_now: false,
            swap_pending: false,
        }
    }

    #[test]
    fn a_free_client_loads_the_offered_match() {
        let step = follow_step(&idle(), false);
        assert_eq!(step, Step::Load { hosting: false });
    }

    #[test]
    fn a_free_host_loads_as_host() {
        let facts = Facts {
            lobby: lobby(1, true),
            ..idle()
        };
        let step = follow_step(&facts, false);
        assert_eq!(step, Step::Load { hosting: true });
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
                in_play: false,
                game_session_key: key(1),
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
            level_is_up: true,
            live_key: key(1),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn a_hosts_own_world_on_the_same_map_satisfies_the_offer_before_it_has_a_key() {
        let facts = Facts {
            lobby: lobby(1, true),
            level_is_up: true,
            launch_zone: "t6:mp_x",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Forget);
    }

    #[test]
    fn a_client_does_not_take_an_unkeyed_world_for_the_match() {
        let facts = Facts {
            level_is_up: true,
            launch_zone: "t6:mp_x",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_hosts_world_on_another_map_is_not_the_match() {
        let facts = Facts {
            lobby: lobby(1, true),
            level_is_up: true,
            launch_zone: "t6:mp_other",
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_world_from_another_match_is_retired_first() {
        let facts = Facts {
            level_is_up: true,
            live_key: key(0),
            ..idle()
        };
        assert_eq!(follow_step(&facts, false), Step::Retire);
    }

    #[test]
    fn a_load_in_progress_is_retired_first() {
        let facts = Facts {
            loading_now: true,
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
            level_is_up: true,
            live_key: key(0),
            swap_pending: true,
            ..idle()
        };
        assert_eq!(follow_step(&facts, true), Step::Wait);
    }

    #[test]
    fn once_the_menu_is_reached_it_loads_even_though_it_was_retiring() {
        let step = follow_step(&idle(), true);
        assert_eq!(step, Step::Load { hosting: false });
    }
}
