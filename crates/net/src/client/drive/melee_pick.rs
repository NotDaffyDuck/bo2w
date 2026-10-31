//! Choosing the player a melee press should rush at, so the command can carry him along with it.

use playerstate_t6::game::{AvatarState, InputCommand, buttons, movement_state_bits};
use sim::{ClientId, PlayerSessionStage, ClientSnapshotMeta, Snapshot};

use crate::client::presentation::shown_frame::ShownWorldFrame;

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

fn dot(u: [f32; 3], vb: [f32; 3]) -> f32 {
    u[0] * vb[0] + u[1] * vb[1] + u[2] * vb[2]
}

fn is_opposing(row: &ClientSnapshotMeta, my_team: i32) -> bool {
    row.lifecycle == PlayerSessionStage::Alive
        && row.client_state_team != entity_t6::team::SPECTATOR
        && (my_team == 0 || row.client_state_team != my_team)
}

/// The living players on the other side, standing or crouched, as the snapshot shows them.
/// Nothing at all comes back until the snapshot has a row for `local`, because without it
/// there is no side to be on the other of.
fn opposing_players(
    frame_snap: &Snapshot,
    own_id: ClientId,
) -> Option<impl Iterator<Item = (ClientId, &AvatarState)>> {
    let my_team = frame_snap.meta.for_client(own_id)?.client_state_team;
    Some(frame_snap.players.iter().filter_map(move |(id, state)| {
        let row = frame_snap.meta.for_client(*id)?;
        let wanted = *id != own_id
            && state.movement_mode <= LAST_ON_FOOT_PM_TYPE
            && is_opposing(row, my_team);
        wanted.then_some((*id, state))
    }))
}

/// The opponents as lunge candidates. `seen_at` says where the local player sees one drawn,
/// which can differ from where the snapshot has him; the snapshot's own spot is the fallback.
fn lunge_candidates<'a>(
    frame_snap: &'a Snapshot,
    own_id: ClientId,
    seen_at: impl Fn(ClientId) -> Option<[f32; 3]> + 'a,
) -> impl Iterator<Item = sim::knife_lunge::LungeCandidate> + 'a {
    opposing_players(frame_snap, own_id)
        .into_iter()
        .flatten()
        .filter_map(move |(id, state)| {
            Some(sim::knife_lunge::LungeCandidate {
                ent: u16::try_from(id.0).ok()?,
                origin: seen_at(id).unwrap_or(state.origin),
            })
        })
}

/// The living zombies the server lists near this client (entity number, feet), nearest first.
/// Empty when the match lists none.
pub(super) fn zombies_near(frame_snap: &Snapshot, own_id: ClientId) -> Vec<(u32, [f32; 3])> {
    let Some(list) = frame_snap.meta.script_dvars(own_id).string("bo2zm_zombies") else {
        return Vec::new();
    };
    list.split(';')
        .filter_map(|spot| {
            let v: Vec<f32> = spot.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            match v[..] {
                [n, x, y, z] => Some((n as u32, [x, y, z])),
                _ => None,
            }
        })
        .collect()
}

/// The target of a Black Ops II lunge among the opponents in `snapshot`, if one is in reach.
/// The zombies the server lists count as opponents too, at the zombies' reach.
pub(super) fn lunge_pick(
    ps: &AvatarState,
    own_id: ClientId,
    frame_snap: &Snapshot,
    shown: &ShownWorldFrame,
    sim_world: &sim::SimWorld,
) -> Option<sim::knife_lunge::LungePick> {
    let zombies = zombies_near(frame_snap, own_id);
    let in_zombies = frame_snap.meta.script_dvars(own_id).string("bo2zm_zombies").is_some();
    let contenders = lunge_candidates(frame_snap, own_id, |id| shown.player(id).map(|p| p.origin)).chain(
        zombies.into_iter().filter_map(|(n, spot)| {
            Some(sim::knife_lunge::LungeCandidate { ent: u16::try_from(n).ok()?, origin: spot })
        }),
    );
    let range = sim::knife_lunge::reach(sim_world.gun_facts(), ps.weapon, in_zombies);
    sim::knife_lunge::pick(sim::knife_lunge::picker(ps, range), contenders, |eye, centre| {
        sim_world.trace_static_world(eye, centre, [0.0; 3], [0.0; 3], sim::MASK_SHOT).fraction >= 1.0
    })
}

/// The yaw and distance of the opponent a melee press should rush, for weapons that do not use
/// the lunge rules: the one nearest the crosshair among those within reach and in view, with a
/// clear line to his chest. None when prone, when nobody qualifies, or when he is already
/// close enough to hit without moving.
pub(super) fn rush_pick(
    own_state: &AvatarState,
    own_id: ClientId,
    frame_snap: &Snapshot,
    sim_world: &sim::SimWorld,
) -> Option<(f32, u8)> {
    if own_state.movement_state_bits & movement_state_bits::LYING_FLAT != 0 {
        return None;
    }
    let opponents = opposing_players(frame_snap, own_id)?;
    let eye = [own_state.origin[0], own_state.origin[1], own_state.origin[2] + own_state.eye_height_now];
    let (forward, right, up) = game_math::euler_basis(own_state.look_angles);

    // A target standing at `origin` (feet): in reach, inside the melee region round the
    // crosshair, nothing solid between; the one nearest the crosshair wins.
    let mut nearest: Option<(f32, [f32; 3], bool)> = None;
    let mut consider = |pos: [f32; 3], zombie: bool| {
        let delta = [pos[0] - own_state.origin[0], pos[1] - own_state.origin[1], pos[2] - own_state.origin[2]];
        if dot(delta, delta) > RUSH_REACH * RUSH_REACH {
            return;
        }
        let centre = [pos[0], pos[1], pos[2] + BODY_CENTRE_HEIGHT];
        let to_centre = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let depth = dot(to_centre, forward);
        if depth <= 1.0 {
            return;
        }
        let across = dot(to_centre, right) / depth;
        let above = dot(to_centre, up) / depth;
        if across.abs() - BODY_HALF_WIDTH / depth > RUSH_TAN_X || above.abs() - BODY_HALF_HEIGHT / depth > RUSH_TAN_Y {
            return;
        }
        let off_crosshair = across * across + above * above;
        if nearest.is_some_and(|(best, _, _)| best <= off_crosshair) {
            return;
        }
        let sight = sim_world.trace_static_world(eye, centre, [0.0; 3], [0.0; 3], sim::MASK_SHOT);
        if sight.fraction < 1.0 {
            return;
        }
        nearest = Some((off_crosshair, delta, zombie));
    };
    // The zombies the server lists near him.
    for (_, spot) in zombies_near(frame_snap, own_id) {
        consider(spot, true);
    }
    for (_, other) in opponents {
        consider(other.origin, false);
    }

    let (_, delta, zombie) = nearest?;
    let mut distance = dot(delta, delta).sqrt();
    let arm_reach = movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT;
    if distance <= arm_reach {
        if !zombie {
            return None;
        }
        // A zombie in reach always gets the stab (BO2 slashes only at nothing): one already
        // inside melee range is lunged at just past it, a step its own body stops.
        distance = arm_reach + 1.0;
    }
    Some((game_math::heading_of(delta[0], delta[1]), distance.min(255.0) as u8))
}

/// Puts the melee target on a command that carries the melee-charge press: for a Black Ops II
/// gun the lunge pick and who it picked, otherwise the plain rush at the nearest opponent in
/// front. Nothing is added without a press, a player state and a snapshot to pick from.
pub(super) fn aim_melee_charge(
    cmd: &mut InputCommand,
    ps: Option<&AvatarState>,
    own_id: ClientId,
    frame_snap: Option<&Snapshot>,
    shown: &ShownWorldFrame,
    sim_world: &sim::SimWorld,
) {
    if cmd.buttons & buttons::MELEE_CHARGE == 0 {
        return;
    }
    let (Some(ps), Some(frame_snap)) = (ps, frame_snap) else {
        return;
    };
    if sim::knife_lunge::brain_on(sim_world.gun_facts()) {
        if let Some(pick) = lunge_pick(ps, own_id, frame_snap, shown, sim_world) {
            cmd.melee_charge_ent = Some(pick.ent);
            cmd.lunge_heading = pick.turn_deg;
            cmd.lunge_reach = pick.dist;
        }
    } else if let Some((yaw, dist)) = rush_pick(ps, own_id, frame_snap, sim_world) {
        cmd.lunge_heading = yaw;
        cmd.lunge_reach = dist;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: ClientId = ClientId(1);

    fn standing_at(pos: [f32; 3]) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin = pos;
        state.eye_height_now = 60.0;
        state
    }

    fn row(lifecycle: PlayerSessionStage, team: i32) -> ClientSnapshotMeta {
        ClientSnapshotMeta {
            lifecycle,
            client_state_team: team,
            ..Default::default()
        }
    }

    /// Me at the origin facing +x on team `my_team`, plus the given other players.
    fn scene(my_team: i32, others: &[(u32, [f32; 3], PlayerSessionStage, i32)]) -> Snapshot {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        snap.players.push((ME, standing_at([0.0; 3])));
        snap.meta.clients.push((ME, row(PlayerSessionStage::Alive, my_team)));
        for (id, pos, lifecycle, team) in others {
            snap.players.push((ClientId(*id), standing_at(*pos)));
            snap.meta.clients.push((ClientId(*id), row(*lifecycle, *team)));
        }
        snap
    }

    fn ids(snap: &Snapshot) -> Vec<u32> {
        opposing_players(snap, ME)
            .map(|found| found.map(|(id, _)| id.0).collect())
            .unwrap_or_default()
    }

    fn me(snap: &Snapshot) -> AvatarState {
        snap.players[0].1.clone()
    }

    const ALIVE: PlayerSessionStage = PlayerSessionStage::Alive;

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
                (4, [10.0, 0.0, 0.0], PlayerSessionStage::Dead, 2),
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
        snap.players[2].1.movement_mode = LAST_ON_FOOT_PM_TYPE + 1;
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
            &[(2, [100.0, 0.0, 0.0], ALIVE, 1), (3, [100.0, 0.0, 0.0], PlayerSessionStage::Dead, 2)],
        );
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn nobody_is_rushed_while_prone() {
        let mut snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        snap.players[0].1.movement_state_bits |= movement_state_bits::LYING_FLAT;
        assert_eq!(rush(&snap), None);
    }

    #[test]
    fn nobody_is_rushed_before_my_row_exists() {
        let mut snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        snap.meta.clients.retain(|(id, _)| *id != ME);
        assert_eq!(rush(&snap), None);
    }

    fn charging() -> InputCommand {
        InputCommand { buttons: buttons::MELEE_CHARGE, ..Default::default() }
    }

    fn aimed(cmd: InputCommand, snap: Option<&Snapshot>) -> InputCommand {
        let mut cmd = cmd;
        let me = snap.map(me);
        aim_melee_charge(
            &mut cmd,
            me.as_ref(),
            ME,
            snap,
            &ShownWorldFrame::default(),
            &sim::SimWorld::new(),
        );
        cmd
    }

    #[test]
    fn a_melee_press_rushes_the_opponent_ahead() {
        let snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(charging(), Some(&snap));
        assert_eq!(cmd.lunge_heading, 0.0);
        assert_eq!(cmd.lunge_reach, 100);
        assert_eq!(cmd.melee_charge_ent, None);
    }

    #[test]
    fn without_the_press_the_command_is_untouched() {
        let snap = scene(1, &[(2, [100.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(InputCommand::default(), Some(&snap));
        assert_eq!(cmd.lunge_reach, 0);
    }

    #[test]
    fn without_a_snapshot_there_is_nobody_to_rush() {
        let cmd = aimed(charging(), None);
        assert_eq!(cmd.lunge_reach, 0);
        assert_eq!(cmd.lunge_heading, 0.0);
    }

    #[test]
    fn nobody_in_reach_leaves_the_press_bare() {
        let snap = scene(1, &[(2, [900.0, 0.0, 0.0], ALIVE, 2)]);
        let cmd = aimed(charging(), Some(&snap));
        assert_eq!(cmd.lunge_reach, 0);
    }
}
