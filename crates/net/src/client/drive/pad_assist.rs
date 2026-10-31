//! The gamepad's aim assist: which players it may pull the view toward this frame, and the
//! per-frame call that applies it to the stick input.

use playerstate_t6::game::{AvatarState, get_viewmodel_weapon_index};
use sim::{ClientId, Snapshot};

use crate::client::local_input::PlayerControlSnapshot;
use crate::client::pad_aim::{AimTarget, AimView, pad_look_frame};
use crate::client::presentation::shown_frame::ShownWorldFrame;

/// Half the width of the box a player is aimed at.
const BODY_RADIUS: f32 = 10.0;
/// How far above the eye the box reaches, and how far up it the aim point sits.
const BOX_ABOVE_EYE: f32 = 8.0;
const AIM_POINT_SHARE: f32 = 0.75;
/// A zombie is about this tall, and the assist leans to this height on it.
const ZOMBIE_TOP: f32 = 70.0;
const ZOMBIE_CHEST: f32 = 48.0;
/// Zombie targets are keyed apart from players' client numbers.
const ZOMBIE_KEY_BASE: u64 = 0x1_0000_0000;
/// A fully zoomed-out view would collapse the sensitivity maths, so the scale stops here.
const LEAST_FOV_SCALE: f32 = 0.01;

/// Pitch and yaw in degrees from the view angles the look state keeps as 16-bit counts.
pub(super) fn facing_degrees(ang3: [i32; 3]) -> [f32; 2] {
    [
        ang3[0] as f32 / input_t6::DEGREES_TO_WORD16,
        ang3[1] as f32 / input_t6::DEGREES_TO_WORD16,
    ]
}

/// The players the assist can aim at: not me, not a teammate in a team mode, standing normally,
/// at least nearly in front of me, and with a clear line from my eye to the top of his head.
/// `shown` is where each one is drawn; the snapshot only says who is in the match.
pub(super) fn assist_targets<'a>(
    me: &AvatarState,
    own_id: ClientId,
    heading: [f32; 2],
    frame_snap: Option<&Snapshot>,
    shown: impl Fn(ClientId) -> Option<&'a AvatarState>,
    visible: impl Fn([f32; 3]) -> bool,
) -> Vec<AimTarget> {
    let Some(frame_snap) = frame_snap else {
        return Vec::new();
    };
    let (sin_pitch, cos_pitch) = heading[0].to_radians().sin_cos();
    let (sn, cs) = heading[1].to_radians().sin_cos();
    let forward = [cos_pitch * cs, cos_pitch * sn, -sin_pitch];
    let reaches_forward = |pos: [f32; 3]| {
        let away = [pos[0] - me.origin[0], pos[1] - me.origin[1], pos[2] - me.origin[2]];
        away[0] * forward[0] + away[1] * forward[1] + away[2] * forward[2] + BODY_RADIUS >= 0.0
    };

    let team_of = |id: ClientId| frame_snap.meta.for_client(id).map(|row| row.client_state_team);
    let team_mode = frame_snap.meta.kind.has_teams();
    let my_team = team_of(own_id);

    let mut found = Vec::new();
    for (id, _) in &frame_snap.players {
        let id = *id;
        if id == own_id || (team_mode && team_of(id) == my_team) {
            continue;
        }
        let Some(other) = shown(id).filter(|other| other.movement_mode == 0) else {
            continue;
        };
        let at = other.origin;
        let head = [at[0], at[1], at[2] + other.eye_height_now];
        if !reaches_forward(at) || !visible(head) {
            continue;
        }
        let top = other.eye_height_now + BOX_ABOVE_EYE;
        found.push(AimTarget {
            key: u64::from(id.0),
            box_lo: [at[0] - BODY_RADIUS, at[1] - BODY_RADIUS, at[2]],
            box_hi: [at[0] + BODY_RADIUS, at[1] + BODY_RADIUS, at[2] + top],
            aim: [at[0], at[1], at[2] + top * AIM_POINT_SHARE],
            drift: other.velocity,
        });
    }
    // Black Ops II's zombies take the pad's aim assist too (a zombie stands about 70 tall; the
    // assist leans to its chest).
    for (n, at) in super::melee_pick::zombies_near(frame_snap, own_id) {
        let chest = [at[0], at[1], at[2] + ZOMBIE_CHEST];
        if !reaches_forward(at) || !visible(chest) {
            continue;
        }
        found.push(AimTarget {
            key: ZOMBIE_KEY_BASE | u64::from(n),
            box_lo: [at[0] - BODY_RADIUS, at[1] - BODY_RADIUS, at[2]],
            box_hi: [at[0] + BODY_RADIUS, at[1] + BODY_RADIUS, at[2] + ZOMBIE_TOP],
            aim: chest,
            drift: [0.0; 3],
        });
    }
    found
}

/// Runs one frame of the pad's look: finds who the assist may lock onto (only when the player
/// has the assist on), then lets the pad code turn the stick into a view change.
pub(super) fn steer_pad_look(
    ctl: &mut PlayerControlSnapshot,
    me: &AvatarState,
    own_id: ClientId,
    ang3: [i32; 3],
    step_millis_elapsed: f32,
    world_view: &ShownWorldFrame,
    sim_world: &sim::SimWorld,
) {
    let eye = [me.origin[0], me.origin[1], me.origin[2] + me.eye_height_now];
    let heading = facing_degrees(ang3);
    let targets = if ctl.pad_aim_assist > 0 {
        let visible = |point: [f32; 3]| {
            let hit = sim_world.trace_world(
                eye,
                point,
                [0.0; 3],
                [0.0; 3],
                hud_t6::overhead_names::NAMEPLATE_SIGHT_FILTER,
            );
            hit.fraction >= 1.0 && !hit.starts_inside
        };
        assist_targets(me, own_id, heading, world_view.snapshot(), |id| world_view.player(id), visible)
    } else {
        Vec::new()
    };
    let ranges = sim_world
        .weapon_stats_entry(get_viewmodel_weapon_index(me))
        .map_or(weapon_t6::combat_facts::AimAssistRanges::OFF, |facts| facts.aim_assist);
    let view = AimView {
        eye,
        pitch_yaw: heading,
        drift: me.velocity,
        ads_lerp: me.weapon_raise_fraction,
        view_angle_multiplier: ctl.view_angle_multiplier.max(LEAST_FOV_SCALE),
        ranges,
        dt: step_millis_elapsed,
    };
    let ads = ctl.client.scoped_in || ctl.client.pressed_key_latch.speed.active;
    pad_look_frame(ctl, &view, &targets, ads);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamemode_t6::RulesetKind;
    use sim::ClientSnapshotMeta;

    const ME: ClientId = ClientId(1);
    const AHEAD: [f32; 2] = [0.0, 0.0];

    fn player(pos: [f32; 3]) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin = pos;
        state.eye_height_now = 60.0;
        state
    }

    fn match_of(kind: RulesetKind, others: &[(u32, [f32; 3], i32)]) -> Snapshot {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        snap.meta.kind = kind;
        let mut add = |id: u32, pos: [f32; 3], team: i32| {
            snap.players.push((ClientId(id), player(pos)));
            snap.meta.clients.push((
                ClientId(id),
                ClientSnapshotMeta { client_state_team: team, ..Default::default() },
            ));
        };
        add(ME.0, [0.0; 3], 1);
        for (id, pos, team) in others {
            add(*id, *pos, *team);
        }
        snap
    }

    fn targets_in(snap: &Snapshot, visible: impl Fn([f32; 3]) -> bool) -> Vec<u64> {
        let me = player([0.0; 3]);
        let shown = |id: ClientId| snap.players.iter().find(|(at, _)| *at == id).map(|(_, p)| p);
        assist_targets(&me, ME, AHEAD, Some(snap), shown, visible)
            .iter()
            .map(|target| target.key)
            .collect()
    }

    #[test]
    fn view_angles_are_read_from_sixteen_bit_counts() {
        let quarter = input_t6::DEGREES_TO_WORD16 * 90.0;
        assert_eq!(facing_degrees([quarter as i32, 0, 7]), [90.0, 0.0]);
        assert_eq!(facing_degrees([0, -(quarter as i32), 0]), [0.0, -90.0]);
    }

    #[test]
    fn everyone_else_in_front_and_in_sight_is_a_target_in_free_for_all() {
        let snap = match_of(
            RulesetKind::FreeForAll,
            &[(2, [100.0, 0.0, 0.0], 1), (3, [100.0, 50.0, 0.0], 2)],
        );
        assert_eq!(targets_in(&snap, |_| true), [2, 3]);
    }

    #[test]
    fn a_team_mode_leaves_out_the_players_own_team() {
        let snap = match_of(
            RulesetKind::TeamDeathmatch,
            &[(2, [100.0, 0.0, 0.0], 1), (3, [100.0, 50.0, 0.0], 2)],
        );
        assert_eq!(targets_in(&snap, |_| true), [3]);
    }

    #[test]
    fn a_team_mode_without_any_team_rows_targets_nobody() {
        let mut snap = match_of(RulesetKind::TeamDeathmatch, &[(2, [100.0, 0.0, 0.0], 2)]);
        snap.meta.clients.clear();
        assert!(targets_in(&snap, |_| true).is_empty());
    }

    #[test]
    fn no_snapshot_means_no_targets() {
        let me = player([0.0; 3]);
        assert!(assist_targets(&me, ME, AHEAD, None, |_| None, |_| true).is_empty());
    }

    #[test]
    fn a_player_behind_is_left_out_unless_he_is_within_a_body_width() {
        let snap = match_of(
            RulesetKind::FreeForAll,
            &[(2, [-50.0, 0.0, 0.0], 2), (3, [-5.0, 0.0, 0.0], 2), (4, [-10.0, 0.0, 0.0], 2)],
        );
        assert_eq!(targets_in(&snap, |_| true), [3, 4]);
    }

    #[test]
    fn the_view_direction_decides_what_is_in_front() {
        let snap = match_of(RulesetKind::FreeForAll, &[(2, [-100.0, 0.0, 0.0], 2)]);
        let me = player([0.0; 3]);
        let shown = |id: ClientId| snap.players.iter().find(|(at, _)| *at == id).map(|(_, p)| p);
        let turned = assist_targets(&me, ME, [0.0, 180.0], Some(&snap), shown, |_| true);
        assert_eq!(turned.len(), 1);
    }

    #[test]
    fn sight_is_checked_at_the_top_of_his_head() {
        let snap = match_of(RulesetKind::FreeForAll, &[(2, [100.0, 0.0, 0.0], 2)]);
        let asked = std::cell::RefCell::new(Vec::new());
        let found = targets_in(&snap, |point| {
            asked.borrow_mut().push(point);
            false
        });
        assert!(found.is_empty());
        assert_eq!(*asked.borrow(), [[100.0, 0.0, 60.0]]);
    }

    #[test]
    fn a_player_who_is_not_walking_about_is_left_out() {
        let mut snap = match_of(RulesetKind::FreeForAll, &[(2, [100.0, 0.0, 0.0], 2)]);
        snap.players[1].1.movement_mode = 1;
        assert!(targets_in(&snap, |_| true).is_empty());
    }

    #[test]
    fn a_target_is_a_box_around_him_with_the_aim_point_in_his_upper_body() {
        let mut snap = match_of(RulesetKind::FreeForAll, &[(2, [100.0, 20.0, 5.0], 2)]);
        snap.players[1].1.velocity = [1.0, 2.0, 3.0];
        let me = player([0.0; 3]);
        let shown = |id: ClientId| snap.players.iter().find(|(at, _)| *at == id).map(|(_, p)| p);
        let found = assist_targets(&me, ME, AHEAD, Some(&snap), shown, |_| true);
        let target = found[0];
        assert_eq!(target.key, 2);
        assert_eq!(target.box_lo, [90.0, 10.0, 5.0]);
        assert_eq!(target.box_hi, [110.0, 30.0, 73.0]);
        assert_eq!(target.aim, [100.0, 20.0, 5.0 + 68.0 * 0.75]);
        assert_eq!(target.drift, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_player_the_view_does_not_show_is_skipped() {
        let snap = match_of(RulesetKind::FreeForAll, &[(2, [100.0, 0.0, 0.0], 2)]);
        let me = player([0.0; 3]);
        let found = assist_targets(&me, ME, AHEAD, Some(&snap), |_| None, |_| true);
        assert!(found.is_empty());
    }
}
