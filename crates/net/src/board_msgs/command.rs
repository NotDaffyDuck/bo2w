//! Building the scoreboard command from a snapshot, and reading one back.

use sim::{ClientId, ClientSnapshotMeta, Snapshot};

use crate::parse_scores::{SCOREBOARD_ROW_LIMIT, DecodedScoreboard, read_board_words};

/// Build `b <count> <axis> <allies> <limit>` and, for each client, eight numbers:
/// id, score, ping, deaths, status icon, kills, assists, extra. Ping, icon, assists and
/// extra are not tracked and go out as 0. At most [`SCOREBOARD_ROW_LIMIT`] clients are listed.
pub fn compose_standings_command(
    roster: &[(ClientId, &ClientSnapshotMeta)],
    axis_points: i32,
    allies_points: i32,
    points_cap: i32,
) -> String {
    let listed = roster.len().min(SCOREBOARD_ROW_LIMIT);
    let mut command = format!("b {listed} {axis_points} {allies_points} {points_cap}");
    for (who, line_meta) in &roster[..listed] {
        command.push_str(&format!(
            " {} {} 0 {} 0 {} 0 0",
            who.0, line_meta.score, line_meta.deaths, line_meta.kills
        ));
    }
    command
}

/// The command for a snapshot: clients best score first (fewest deaths breaks a tie), then
/// the two team scores and the score limit from the match state.
pub fn compose_standings_from_state(snap: &Snapshot) -> String {
    let mut standings: Vec<(ClientId, &ClientSnapshotMeta)> =
        snap.meta.clients.iter().map(|(who, line_meta)| (*who, line_meta)).collect();
    standings.sort_by(|left, right| right.1.score.cmp(&left.1.score).then(left.1.deaths.cmp(&right.1.deaths)));
    standings.truncate(SCOREBOARD_ROW_LIMIT);

    let team_score = |team: i32| {
        snap.meta.objectives.scores.get(team as usize).copied().unwrap_or(0)
    };
    compose_standings_command(
        &standings,
        team_score(entity_t6::team::AXIS),
        team_score(entity_t6::team::ALLIES),
        snap.meta.score_limit,
    )
}

/// Read a scoreboard command given as one string.
pub fn read_board_command(command: &str) -> DecodedScoreboard {
    let words: Vec<&str> = command.split_whitespace().collect();
    read_board_words(&words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::Tick;

    fn client_line(points: i32, died: i32, frags: i32) -> ClientSnapshotMeta {
        ClientSnapshotMeta { score: points, deaths: died, kills: frags, ..Default::default() }
    }

    #[test]
    fn no_clients_makes_a_header_only() {
        assert_eq!(compose_standings_command(&[], 3, 4, 5), "b 0 3 4 5");
    }

    #[test]
    fn each_client_adds_eight_numbers_with_the_untracked_ones_zero() {
        let client = client_line(100, 2, 9);
        let command = compose_standings_command(&[(ClientId(7), &client)], 1, 2, 3);
        assert_eq!(command, "b 1 1 2 3 7 100 0 2 0 9 0 0");
    }

    #[test]
    fn no_more_than_the_cap_are_listed() {
        let client = client_line(1, 1, 1);
        let many: Vec<_> = (0..30).map(|id| (ClientId(id), &client)).collect();
        let command = compose_standings_command(&many, 0, 0, 0);
        assert!(command.starts_with("b 18 "), "{command}");
        assert_eq!(read_board_command(&command).num, 18);
        assert_eq!(command.split_whitespace().count(), 5 + 18 * 8);
    }

    #[test]
    fn a_formatted_command_reads_back_to_the_same_numbers() {
        let (ann, bob) = (client_line(50, 3, 12), client_line(80, 1, 20));
        let command = compose_standings_command(&[(ClientId(4), &ann), (ClientId(9), &bob)], 10, 20, 30);
        let parsed = read_board_command(&command);
        assert_eq!(parsed.num, 2);
        assert_eq!((parsed.opposing_side_points, parsed.friendly_side_points, parsed.points_cap), (10, 20, 30));
        assert_eq!(
            (parsed.rows[1].client, parsed.rows[1].tally, parsed.rows[1].times_died, parsed.rows[1].frags),
            (9, 80, 1, 20)
        );
    }

    #[test]
    fn a_snapshot_lists_the_best_score_first_and_fewer_deaths_win_ties() {
        let mut snap = Snapshot::unpublished(Tick(1));
        snap.meta.clients = vec![
            (ClientId(0), client_line(10, 5, 0)),
            (ClientId(1), client_line(30, 9, 0)),
            (ClientId(2), client_line(10, 2, 0)),
        ];
        let parsed = read_board_command(&compose_standings_from_state(&snap));
        let order: Vec<_> = parsed.rows[..parsed.num].iter().map(|row| row.client).collect();
        assert_eq!(order, [1, 2, 0]);
    }

    #[test]
    fn a_snapshot_supplies_the_team_scores_and_the_limit() {
        let mut snap = Snapshot::unpublished(Tick(1));
        snap.meta.objectives.scores = [0, 111, 222];
        snap.meta.score_limit = 75;
        let parsed = read_board_command(&compose_standings_from_state(&snap));
        let by_team = |team: i32| snap.meta.objectives.scores[team as usize];
        assert_eq!(parsed.opposing_side_points, by_team(entity_t6::team::AXIS));
        assert_eq!(parsed.friendly_side_points, by_team(entity_t6::team::ALLIES));
        assert_eq!(parsed.points_cap, 75);
    }

    #[test]
    fn a_snapshot_with_more_than_the_cap_keeps_the_best() {
        let mut snap = Snapshot::unpublished(Tick(1));
        snap.meta.clients = (0..25).map(|id| (ClientId(id), client_line(id as i32, 0, 0))).collect();
        let parsed = read_board_command(&compose_standings_from_state(&snap));
        assert_eq!(parsed.num, 18);
        assert_eq!(parsed.rows[0].client, 24);
        assert_eq!(parsed.rows[17].client, 7);
    }

    #[test]
    fn text_that_is_not_a_scoreboard_parses_empty() {
        assert_eq!(read_board_command("hello world"), DecodedScoreboard::default());
        assert_eq!(read_board_command(""), DecodedScoreboard::default());
    }
}
