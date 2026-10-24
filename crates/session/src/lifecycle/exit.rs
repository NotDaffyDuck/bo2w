//! What the running game asks of the session: end the level, slow down, or
//! change the map.

use std::time::Duration;

use bevy::prelude::*;
use frame::{EndBoardShown, ExitLevelCalled, LaunchIdentity, MatchTornDown};

use super::SessionSwapRequest;
use crate::Bo2mpMatchSetup;

/// The end scoreboard stays up this long after the match ends, then the
/// match goes back to the front end (the game script waits five seconds
/// between showing the board and exiting the level).
const END_BOARD_HOLD: Duration = Duration::from_millis(5000);

/// A change in game speed smaller than this is not worth announcing.
const SPEED_EPSILON: f32 = 1e-3;

/// What to do about an exit-level call.
#[derive(Debug, PartialEq, Eq)]
enum ExitStep {
    /// The scoreboard has not been up long enough yet.
    Hold,
    /// Back to the menu.
    Menu,
    /// Start a new game on the same map.
    NewGame(String),
}

/// A match started from the front end goes back to it once the board has
/// been up for the hold. Any other match ends in a new game on the same map
/// when it has one (there is no menu to go back to for it), else the menu.
fn exit_step(front_end_match: bool, board_up: Option<Duration>, zone: Option<&str>) -> ExitStep {
    if front_end_match {
        return match board_up {
            Some(up) if up < END_BOARD_HOLD => ExitStep::Hold,
            _ => ExitStep::Menu,
        };
    }
    match zone.filter(|zone| zone.starts_with("t6:") || zone.starts_with("zm_")) {
        Some(zone) => ExitStep::NewGame(zone.to_owned()),
        None => ExitStep::Menu,
    }
}

pub(super) fn run_exit_level(
    mut called: MessageReader<ExitLevelCalled>,
    mut swap: ResMut<SessionSwapRequest>,
    identity: Option<Res<LaunchIdentity>>,
    setup: Option<Res<Bo2mpMatchSetup>>,
    mut board: Option<ResMut<EndBoardShown>>,
    mut waiting: Local<bool>,
) {
    if called.read().next().is_some() {
        *waiting = true;
    }
    if !*waiting {
        return;
    }
    let shown_since = board.as_deref().and_then(|board| board.0);
    let step = exit_step(
        setup.is_some(),
        shown_since.map(|since| since.elapsed()),
        identity.as_deref().map(|identity| identity.zone.as_str()),
    );
    if step == ExitStep::Hold {
        return;
    }
    *waiting = false;
    if setup.is_some()
        && let Some(since) = board.as_deref_mut().and_then(|board| board.0.take())
    {
        diag::info!(
            Sim,
            "exitLevel: the end scoreboard was up {} ms",
            since.elapsed().as_millis()
        );
    }
    let (result, what) = match step {
        ExitStep::NewGame(zone) => (
            swap.request_zone(zone.clone()),
            format!("a new game on `{zone}`"),
        ),
        _ => (swap.request_menu(), "the front end".to_owned()),
    };
    match result {
        Ok(id) => diag::info!(Sim, "session: exitLevel -> {what} (swap #{id})"),
        Err(err) => diag::info!(Sim, "session: exitLevel refused - {err}"),
    }
}

/// The setup a script's `map()` call asks for.
fn setup_for(change: sim::MenuMapChange) -> Bo2mpMatchSetup {
    let (bot_friends, bot_enemies, bot_difficulty) = change.bots;
    Bo2mpMatchSetup {
        map: change.map,
        gametype: change.gametype,
        bot_friends,
        bot_enemies,
        bot_difficulty,
    }
}

fn speed_changed(current: f32, target: f32) -> bool {
    (current - target).abs() > SPEED_EPSILON
}

/// Slow motion (the final killcam, the mod menu) runs the whole game at the
/// speed it has slid to, moved on by this frame's real time, until the match
/// ends. A script's `map()` call loads the new match as the front end would,
/// keeping the bots.
pub(super) fn run_menu_requests(
    mut swap: ResMut<SessionSwapRequest>,
    setup: Option<ResMut<Bo2mpMatchSetup>>,
    mut commands: Commands,
    mut time: ResMut<Time<Virtual>>,
    real: Res<Time<Real>>,
    mut torn: MessageReader<MatchTornDown>,
) {
    if torn.read().count() > 0 {
        sim::reset_menu_timescale();
    }
    let speed = sim::advance_menu_timescale(real.delta_secs() * 1000.0);
    if speed_changed(time.relative_speed(), speed) {
        time.set_relative_speed(speed);
        diag::info!(Sim, "session: game speed {speed:.2}");
    }
    let Some(change) = sim::take_menu_map_change() else {
        return;
    };
    let next = setup_for(change);
    let zone = next.zone();
    let gametype = next.gametype.clone();
    match setup {
        Some(mut current) => *current = next,
        None => commands.insert_resource(next),
    }
    sim::reset_menu_timescale();
    match swap.request_zone(zone.clone()) {
        Ok(id) => diag::info!(Sim, "session: menu -> `{zone}` {gametype} (swap #{id})"),
        Err(err) => diag::info!(Sim, "session: menu map change refused - {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG_ENOUGH: Duration = Duration::from_millis(5000);

    #[test]
    fn a_front_end_match_waits_for_the_board() {
        let held = Duration::from_millis(1200);
        assert_eq!(exit_step(true, Some(held), None), ExitStep::Hold);
        assert_eq!(
            exit_step(true, Some(END_BOARD_HOLD - Duration::from_millis(1)), None),
            ExitStep::Hold
        );
    }

    #[test]
    fn a_front_end_match_leaves_once_the_board_has_been_up_long_enough() {
        assert_eq!(exit_step(true, Some(LONG_ENOUGH), None), ExitStep::Menu);
        assert_eq!(
            exit_step(true, Some(LONG_ENOUGH * 2), Some("t6:mp_x")),
            ExitStep::Menu
        );
    }

    #[test]
    fn a_front_end_match_that_never_showed_a_board_does_not_wait() {
        assert_eq!(exit_step(true, None, None), ExitStep::Menu);
    }

    #[test]
    fn other_matches_start_a_new_game_on_a_map_that_has_no_menu() {
        assert_eq!(
            exit_step(false, None, Some("t6:mp_x")),
            ExitStep::NewGame("t6:mp_x".into())
        );
        assert_eq!(
            exit_step(false, Some(Duration::ZERO), Some("zm_town")),
            ExitStep::NewGame("zm_town".into())
        );
    }

    #[test]
    fn other_matches_with_no_such_map_go_to_the_menu() {
        assert_eq!(exit_step(false, None, Some("mp_other")), ExitStep::Menu);
        assert_eq!(exit_step(false, None, Some("")), ExitStep::Menu);
        assert_eq!(exit_step(false, None, None), ExitStep::Menu);
    }

    #[test]
    fn a_map_change_keeps_the_bots_it_asked_for() {
        let setup = setup_for(sim::MenuMapChange {
            map: "mp_x".into(),
            gametype: "tdm".into(),
            bots: (3, 4, 2),
        });
        assert_eq!(
            setup,
            Bo2mpMatchSetup {
                map: "mp_x".into(),
                gametype: "tdm".into(),
                bot_friends: 3,
                bot_enemies: 4,
                bot_difficulty: 2,
            }
        );
        assert_eq!(setup.zone(), "t6:mp_x");
    }

    #[test]
    fn tiny_speed_differences_are_not_changes() {
        assert!(!speed_changed(1.0, 1.0005));
        assert!(speed_changed(1.0, 0.25));
        assert!(speed_changed(0.25, 1.0));
    }
}
