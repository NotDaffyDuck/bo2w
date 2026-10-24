//! Choosing the player a melee press should rush at, so the command can carry him along with it.

use playerstate_t6::game::{PlayerState, UserCmd, buttons, pm_flags};
use sim::{ClientId, ClientLifecycle, ClientSnapshotMeta, Snapshot};

use crate::client::presentation::presented::PresentedSnapshot;

/// Players farther than this are never rushed at by the plain auto-melee.
const RUSH_REACH: f32 = 128.0;
/// How far off the crosshair a body may be, as the tangent of the view angle, and still count.
const RUSH_TAN_X: f32 = 0.849 * 0.5;
const RUSH_TAN_Y: f32 = 0.478 * 0.5;
/// Where on a player the line of sight is aimed, and how much of him it has to be able to fit.
const BODY_CENTRE_HEIGHT: f32 = 36.0;
const BODY_HALF_WIDTH: f32 = 15.0;
const BODY_HALF_HEIGHT: f32 = 36.0;
/// Player movement types up to this one are on their feet; higher ones are dead or riding.
const LAST_ON_FOOT_PM_TYPE: i32 = 1;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn is_opposing(row: &ClientSnapshotMeta, my_team: i32) -> bool {
    row.lifecycle == ClientLifecycle::Alive
        && row.client_state_team != entity_t6::team::SPECTATOR
        && (my_team == 0 || row.client_state_team != my_team)
}

/// The living players on the other side, standing or crouched, as the snapshot shows them.
/// Nothing at all comes back until the snapshot has a row for `local`, because without it
/// there is no side to be on the other of.
fn opposing_players(
    snapshot: &Snapshot,
    local: ClientId,
) -> Option<impl Iterator<Item = (ClientId, &PlayerState)>> {
    let my_team = snapshot.meta.for_client(local)?.client_state_team;
    Some(snapshot.players.iter().filter_map(move |(id, state)| {
        let row = snapshot.meta.for_client(*id)?;
        let wanted = *id != local
            && state.pm_type <= LAST_ON_FOOT_PM_TYPE
            && is_opposing(row, my_team);
        wanted.then_some((*id, state))
    }))
}

/// The opponents as lunge candidates. `seen_at` says where the local player sees one drawn,
/// which can differ from where the snapshot has him; the snapshot's own spot is the fallback.
fn lunge_candidates<'a>(
    snapshot: &'a Snapshot,
    local: ClientId,
    seen_at: impl Fn(ClientId) -> Option<[f32; 3]> + 'a,
) -> impl Iterator<Item = sim::t6_lunge::LungeCandidate> + 'a {
    opposing_players(snapshot, local)
        .into_iter()
        .flatten()
        .filter_map(move |(id, state)| {
            Some(sim::t6_lunge::LungeCandidate {
                ent: u16::try_from(id.0).ok()?,
                origin: seen_at(id).unwrap_or(state.origin),
            })
        })
}

/// The target of a Black Ops II lunge among the opponents in `snapshot`, if one is in reach.
pub(super) fn lunge_pick(
    ps: &PlayerState,
    local: ClientId,
    snapshot: &Snapshot,
    presented: &PresentedSnapshot,
    world: &sim::SimWorld,
) -> Option<sim::t6_lunge::LungePick> {
    let candidates = lunge_candidates(snapshot, local, |id| presented.player(id).map(|p| p.origin));
    let range = sim::t6_lunge::reach(world.gun_facts(), ps.weapon, false);
    sim::t6_lunge::pick(sim::t6_lunge::picker(ps, range), candidates, |eye, centre| {
        world.trace_static_world(eye, centre, [0.0; 3], [0.0; 3], sim::MASK_SHOT).fraction >= 1.0
    })
}

/// The yaw and distance of the opponent a melee press should rush, for weapons that do not use
/// the lunge rules: the one nearest the crosshair among those within reach and in view, with a
/// clear line to his chest. None when prone, when nobody qualifies, or when he is already
/// close enough to hit without moving.
pub(super) fn rush_pick(
    ps: &PlayerState,
    local: ClientId,
    snapshot: &Snapshot,
    world: &sim::SimWorld,
) -> Option<(f32, u8)> {
    if ps.pm_flags & pm_flags::PRONE != 0 {
        return None;
    }
    let opponents = opposing_players(snapshot, local)?;
    let eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
    let (forward, right, up) = game_math::angle_vectors(ps.viewangles);

    let mut nearest: Option<(f32, [f32; 3])> = None;
    for (_, other) in opponents {
        let delta = [
            other.origin[0] - ps.origin[0],
            other.origin[1] - ps.origin[1],
            other.origin[2] - ps.origin[2],
        ];
        if dot(delta, delta) > RUSH_REACH * RUSH_REACH {
            continue;
        }
        let centre = [other.origin[0], other.origin[1], other.origin[2] + BODY_CENTRE_HEIGHT];
        let to_centre = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let depth = dot(to_centre, forward);
        if depth <= 1.0 {
            continue;
        }
        let across = dot(to_centre, right) / depth;
        let above = dot(to_centre, up) / depth;
        if across.abs() - BODY_HALF_WIDTH / depth > RUSH_TAN_X
            || above.abs() - BODY_HALF_HEIGHT / depth > RUSH_TAN_Y
        {
            continue;
        }
        let off_crosshair = across * across + above * above;
        if nearest.is_some_and(|(best, _)| best <= off_crosshair) {
            continue;
        }
        let sight = world.trace_static_world(eye, centre, [0.0; 3], [0.0; 3], sim::MASK_SHOT);
        if sight.fraction < 1.0 {
            continue;
        }
        nearest = Some((off_crosshair, delta));
    }

    let (_, delta) = nearest?;
    let distance = dot(delta, delta).sqrt();
    if distance <= movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT {
        return None;
    }
    Some((game_math::vec_to_yaw(delta[0], delta[1]), distance as u8))
}

/// Puts the melee target on a command that carries the melee-charge press: for a Black Ops II
/// gun the lunge pick and who it picked, otherwise the plain rush at the nearest opponent in
/// front. Nothing is added without a press, a player state and a snapshot to pick from.
pub(super) fn aim_melee_charge(
    cmd: &mut UserCmd,
    ps: Option<&PlayerState>,
    local: ClientId,
    snapshot: Option<&Snapshot>,
    presented: &PresentedSnapshot,
    world: &sim::SimWorld,
) {
    if cmd.buttons & buttons::MELEE_CHARGE == 0 {
        return;
    }
    let (Some(ps), Some(snapshot)) = (ps, snapshot) else {
        return;
    };
    if sim::t6_lunge::brain_on(world.gun_facts()) {
        if let Some(pick) = lunge_pick(ps, local, snapshot, presented, world) {
            cmd.melee_charge_ent = Some(pick.ent);
            cmd.melee_charge_yaw = pick.yaw;
            cmd.melee_charge_dist = pick.dist;
        }
    } else if let Some((yaw, dist)) = rush_pick(ps, local, snapshot, world) {
        cmd.melee_charge_yaw = yaw;
        cmd.melee_charge_dist = dist;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: ClientId = ClientId(1);

    fn standing_at(origin: [f32; 3]) -> PlayerState {
        let mut state = PlayerState::ZERO;
        state.origin = origin;
        state.view_height_current = 60.0;
        state
    }

    fn row(lifecycle: ClientLifecycle, team: i32) -> ClientSnapshotMeta {
        ClientSnapshotMeta {
            lifecycle,
            client_state_team: team,
            ..Default::default()
        }
    }

    /// Me at the origin facing +x on team `my_team`, plus the given other players.
    fn scene(my_team: i32, others: &[(u32, [f32; 3], ClientLifecycle, i32)]) -> Snapshot {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        snap.players.push((ME, standing_at([0.0; 3])));
        snap.meta.clients.push((ME, row(ClientLifecycle::Alive, my_team)));
        for (id, origin, lifecycle, team) in others {
            snap.players.push((ClientId(*id), standing_at(*origin)));
            snap.meta.clients.push((ClientId(*id), row(*lifecycle, *team)));
        }
        snap
    }

    fn ids(snap: &Snapshot) -> Vec<u32> {
        opposing_players(snap, ME)
            .map(|found| found.map(|(id, _)| id.0).collect())
            .unwrap_or_default()
    }

    fn me(snap: &Snapshot) -> PlayerState {
        snap.players[0].1.clone()
    }

    const ALIVE: ClientLifecycle = ClientLifecycle::Alive;

    #[test]
    fn nobody_is_an_opponent_before_my_own_row_exists() {
        let mut snap = scene(1, &[(2, [10.0, 0.0, 0.0], ALIVE, 2)]);
        snap.meta.clients.retain(|(id, _)| *id != ME);
        assert!(opposing_players(&snap, ME).is_none());
    }

    #[test]
    fn only_living_players_on_the_other_team_are_opponents() {
        let snap = scene(
            1,
            &[
                (2, [10.0, 0.0, 0.0], ALIVE, 2),
                (3, [10.0, 0.0, 0.0], ALIVE, 1),
                (4, [10.0, 0.0, 0.0], ClientLifecycle::Dead, 2),
                (5, [10.0, 0.0, 0.0], ALIVE, entity_t6::team::SPECTATOR),
                (6, [10.0, 0.0, 0.0], ALIVE, 3),
            ],
        );
        assert_eq!(ids(&snap), [2, 6]);
    }

    #[test]
    fn with_no_team_of_my_own_everyone_else_alive_is_an_opponent() {
        let snap = scene(0, &[(2, [10.0, 0.0, 0.0], ALIVE, 0), (3, [10.0, 0.0, 0.0], ALIVE, 1)]);
        assert_eq!(ids(&snap), [2, 3]);
    }

    #[test]
    fn a_player_who_is_not_on_foot_is_left_out() {
        let mut snap = scene(1, &[(2, [10.0, 0.0, 0.0], ALIVE, 2), (3, [10.0, 0.0, 0.0], ALIVE, 2)]);
        snap.players[2].1.pm_type = LAST_ON_FOOT_PM_TYPE + 1;
        assert_eq!(ids(&snap), [2]);
    }

    #[test]
    fn a_player_without_a_row_is_left_out() {
        let mut snap = scene(1, &[(2, [10.0, 0.0, 0.0], ALIVE, 2), (3, [10.0, 0.0, 0.0], ALIVE, 2)]);
        snap.meta.clients.retain(|(id, _)| id.0 != 3);
        assert_eq!(ids(&snap), [2]);
    }

    #[test]
    fn lunge_candidates_are_drawn_where_the_local_player_sees_them() {
        let snap = scene(1, &[(2, [10.0, 0.0, 0.0], ALIVE, 2), (3, [20.0, 0.0, 0.0], ALIVE, 2)]);
        let seen = |id: ClientId| (id.0 == 3).then_some([99.0, 1.0, 2.0]);
        let found: Vec<_> = lunge_candidates(&snap, ME, seen).map(|c| (c.ent, c.origin)).collect();
        assert_eq!(found, [(2, [10.0, 0.0, 0.0]), (3, [99.0, 1.0, 2.0])]);
    }

    #[test]
    fn a_client_number_past_sixteen_bits_cannot_be_a_lunge_target() {
        let snap = scene(1, &[(2, [10.0, 0.0, 0.0], ALIVE, 2), (70_000, [10.0, 0.0, 0.0], ALIVE, 2)]);
        let found: Vec<u16> = lunge_candidates(&snap, ME, |_| None).map(|c| c.ent).collect();
        assert_eq!(found, [2]);
    }

    fn rush(snap: &Snapshot) -> Option<(f32, u8)> {
        rush_pick(&me(snap), ME, snap, &sim::SimWorld::new())
    }

    #[test]
    fn an_opponent_dead_ahead_and_out_of_arm_reach_is_rushed() {
        let snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        let (yaw, distance) = rush(&snap).expect("a target");
        assert_eq!(yaw, 0.0);
        assert_eq!(distance, 100);
    }

    #[test]
    fn the_rush_turns_toward_the_side_he_stands_on() {
        let snap = scene(1, &[(2, [100.0, 30.0, 0.0], ALIVE, 2)]);
        let (yaw, _) = rush(&snap).expect("a target");
        assert!((yaw - 16.7).abs() < 0.1, "yaw was {yaw}");
    }

    #[test]
    fn an_opponent_already_in_arm_reach_needs_no_rush() {
        let snap = scene(1, &[(2, [50.0, 0.0, 0.0], ALIVE, 2)]);
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn an_opponent_beyond_the_rush_reach_is_ignored() {
        let snap = scene(1, &[(2, [200.0, 0.0, 0.0], ALIVE, 2)]);
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn an_opponent_behind_me_is_ignored() {
        let snap = scene(1, &[(2, [-100.0, 0.0, 0.0], ALIVE, 2)]);
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn an_opponent_far_off_to_the_side_is_outside_the_view_region() {
        let snap = scene(1, &[(2, [60.0, 90.0, 0.0], ALIVE, 2)]);
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn the_opponent_nearest_the_crosshair_wins() {
        let snap = scene(
            1,
            &[(2, [100.0, 30.0, 0.0], ALIVE, 2), (3, [100.0, 0.0, 0.0], ALIVE, 2)],
        );
        let (yaw, distance) = rush(&snap).expect("a target");
        assert_eq!(yaw, 0.0);
        assert_eq!(distance, 100);
    }

    #[test]
    fn a_friend_or_a_corpse_is_not_rushed() {
        let snap = scene(
            1,
            &[(2, [100.0, 0.0, 0.0], ALIVE, 1), (3, [100.0, 0.0, 0.0], ClientLifecycle::Dead, 2)],
        );
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn nobody_is_rushed_while_prone() {
        let mut snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        snap.players[0].1.pm_flags |= pm_flags::PRONE;
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn nobody_is_rushed_before_my_row_exists() {
        let mut snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        snap.meta.clients.retain(|(id, _)| *id != ME);
        assert_eq!(rush(&snap), None);
    }

    fn charging() -> UserCmd {
        UserCmd { buttons: buttons::MELEE_CHARGE, ..Default::default() }
    }

    fn aimed(cmd: UserCmd, snap: Option<&Snapshot>) -> UserCmd {
        let mut cmd = cmd;
        let me = snap.map(me);
        aim_melee_charge(
            &mut cmd,
            me.as_ref(),
            ME,
            snap,
            &PresentedSnapshot::default(),
            &sim::SimWorld::new(),
        );
        cmd
    }

    #[test]
    fn a_melee_press_rushes_the_opponent_ahead() {
        let snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(charging(), Some(&snap));
        assert_eq!(cmd.melee_charge_yaw, 0.0);
        assert_eq!(cmd.melee_charge_dist, 100);
        assert_eq!(cmd.melee_charge_ent, None);
    }

    #[test]
    fn without_the_press_the_command_is_untouched() {
        let snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(UserCmd::default(), Some(&snap));
        assert_eq!(cmd.melee_charge_dist, 0);
    }

    #[test]
    fn without_a_snapshot_there_is_nobody_to_rush() {
        let cmd = aimed(charging(), None);
        assert_eq!(cmd.melee_charge_dist, 0);
        assert_eq!(cmd.melee_charge_yaw, 0.0);
    }

    #[test]
    fn nobody_in_reach_leaves_the_press_bare() {
        let snap = scene(1, &[(2, [900.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(charging(), Some(&snap));
        assert_eq!(cmd.melee_charge_dist, 0);
    }
}
