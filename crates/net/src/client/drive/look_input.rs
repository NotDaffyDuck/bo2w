//! What the player's raw input goes through before it is built into a command: the freezes that
//! swallow it, the mouse that belongs to the map or to a guided missile, the shellshock
//! slowdown, the turn itself, the stance latch, the gun's kick and the scope's zoom key.

use input_t6::{ANGLE2SHORT, ClientInput, Kbutton};
use playerstate_t6::game::PlayerState;
use sim::{ClientId, ClientSnapshotMeta, Snapshot};

use crate::client::input::{ClientActionInput, LookState, accumulate_look};
use crate::client::presentation::presented::PresentedSnapshot;

/// Set in the player state's movement flags while the match holds the player still.
const PM_FROZEN: u32 = 0x800;
/// The raw weapon state of a riot shield being set down.
const SHIELD_DEPLOYING: i32 = 44;
/// The script dvar that names the vehicle the player flies, and how a drone's name starts.
const VEHICLE_DVAR: &str = "bo2mp_vehicle";
const DRONE_NAME_START: &str = "qrdrone";

/// Whether the player can neither move nor turn: the movement flag is set, or the match row
/// says so. A player the match has no row for yet counts as frozen.
pub(super) fn is_frozen(ps: Option<&PlayerState>, row: Option<&ClientSnapshotMeta>) -> bool {
    ps.is_some_and(|ps| ps.pm_flags & PM_FROZEN != 0) || row.is_none_or(|row| row.controls.frozen)
}

pub(super) fn is_setting_down_shield(offhand: Option<(i32, u32)>) -> bool {
    offhand.is_some_and(|(state, _)| state == SHIELD_DEPLOYING)
}

/// Whether a missile he guides is still linked to him.
pub(super) fn is_guiding_missile(row: Option<&ClientSnapshotMeta>) -> bool {
    row.and_then(|row| row.remote_missile)
        .is_some_and(|link| link.unlink_at_ms.is_none())
}

/// Takes away the input the player's state does not allow. A guided missile keeps the look but
/// not the walk; a freeze or a shield being set down takes the mouse and both sticks.
pub(super) fn gate_raw_input(
    actions: &mut ClientActionInput,
    guiding: bool,
    setting_down: bool,
    frozen: bool,
) {
    if guiding && !setting_down {
        actions.pad_move = [0.0; 2];
    } else if frozen || setting_down {
        actions.mouse_x = 0.0;
        actions.mouse_y = 0.0;
        actions.pad_move = [0.0; 2];
        actions.pad_look = [0.0; 2];
    }
}

/// While the map overlay is up the mouse belongs to its cursor: this frame's movement is
/// handed back and the view gets none of it.
pub(super) fn take_cursor_mouse(
    actions: &mut ClientActionInput,
    overlay_open: bool,
) -> Option<(f32, f32)> {
    if !overlay_open {
        return None;
    }
    let moved = (actions.mouse_x, actions.mouse_y);
    actions.mouse_x = 0.0;
    actions.mouse_y = 0.0;
    Some(moved)
}

/// While a guided missile is linked the mouse and the right stick steer it: they are handed
/// back and the view gets none of them.
pub(super) fn take_missile_steering(
    actions: &mut ClientActionInput,
    guiding: bool,
) -> Option<(f32, f32, [f32; 2])> {
    if !guiding {
        return None;
    }
    let steering = (actions.mouse_x, actions.mouse_y, actions.pad_look);
    actions.mouse_x = 0.0;
    actions.mouse_y = 0.0;
    actions.pad_look = [0.0; 2];
    actions.pad_look_delta = [0.0; 2];
    Some(steering)
}

/// Sets the look limits from the shellshock the player is under, or the unshocked ones.
pub(super) fn apply_shellshock(
    actions: &mut ClientActionInput,
    ps: Option<&PlayerState>,
    local: ClientId,
    presented: &PresentedSnapshot,
    cg_time: i32,
) {
    let limits = match (ps, presented.shellshock(local)) {
        (Some(ps), Some(shock)) => {
            let left = hud_t6::flashbang::shellshock_remaining_ms(
                cg_time,
                ps.shellshock_time,
                ps.shellshock_duration,
            );
            hud_t6::flashbang::shellshock_look_control(left, &shock.look_control)
        }
        _ => hud_t6::flashbang::SHELLSHOCK_LOOK_ENDED,
    };
    actions.shellshock_look_scale = limits.sensitivity;
    actions.cgame_max_pitch_speed = limits.max_pitch_speed;
    actions.cgame_max_yaw_speed = limits.max_yaw_speed;
}

/// Turns the view by this frame's keys, then honours a pressed "center view".
pub(super) fn turn_view(
    actions: &mut ClientActionInput,
    look: &mut LookState,
    ps: Option<&PlayerState>,
    frametime: f32,
    frozen: bool,
) {
    let (now, frame_msec, yaw_speed) = (actions.now_msec, actions.frame_msec, actions.cl_yawspeed);
    let (max_pitch, max_yaw) = (actions.cgame_max_pitch_speed, actions.cgame_max_yaw_speed);
    accumulate_look(
        frametime,
        now,
        frame_msec,
        &mut actions.client,
        look,
        yaw_speed,
        frozen,
        max_pitch,
        max_yaw,
    );
    let recenter = std::mem::take(&mut actions.client.center_view);
    if let (true, Some(ps)) = (recenter, ps) {
        look.angles[0] = (-ps.delta_angles[0] * ANGLE2SHORT) as i32;
    }
}

/// Follows the stance events the player state carries. A new life forgets the latch and starts
/// reading the events from where the state is now.
pub(super) fn latch_stance(
    actions: &mut ClientActionInput,
    ps: Option<&PlayerState>,
    life_sequence: u32,
) {
    if actions.stance_life == Some(life_sequence) {
        if let Some(ps) = ps {
            actions.read_stance_events(i32::from(ps.predictable_event_sequence), ps.predictable_events);
        }
        return;
    }
    actions.stance_life = Some(life_sequence);
    actions.client.stance_latch = 0;
    actions.stance_event_sequence = ps.map_or(0, |ps| i32::from(ps.predictable_event_sequence));
    actions.stance_events = ps.map_or([0; 4], |ps| ps.predictable_events);
}

/// Moves the aim by the gun's kick in whole 16-bit steps; what is left over waits for the next
/// command. A frozen player's kick is dropped.
pub(super) fn kick_view(look: &mut LookState, frozen: bool) {
    for axis in 0..2 {
        if frozen {
            look.kick_pending[axis] = 0.0;
            continue;
        }
        let whole = (look.kick_pending[axis] * ANGLE2SHORT).round() as i32;
        if whole == 0 {
            continue;
        }
        look.angles[axis] = look.angles[axis].wrapping_add(whole);
        look.kick_pending[axis] -= whole as f32 / ANGLE2SHORT;
    }
}

/// Through a variable zoom scope the melee key changes the zoom instead of swinging the knife:
/// a fresh press is counted for the camera and the key is cleared either way.
pub(super) fn melee_key_to_zoom(actions: &mut ClientActionInput, look: &mut LookState) {
    if !look.melee_is_zoom {
        return;
    }
    let melee: &mut Kbutton = &mut actions.client.kb.melee;
    if !(melee.was_pressed || melee.active) {
        return;
    }
    if melee.was_pressed {
        look.zoom_presses = look.zoom_presses.wrapping_add(1);
    }
    *melee = Kbutton::default();
}

/// Whether the vehicle dvar names a drone; only its first word counts, in any letter case.
pub(super) fn names_a_drone(vehicle: Option<&str>) -> bool {
    let first_word = vehicle.and_then(|text| text.split(' ').next());
    first_word
        .and_then(|word| word.as_bytes().get(..DRONE_NAME_START.len()))
        .is_some_and(|head| head.eq_ignore_ascii_case(DRONE_NAME_START.as_bytes()))
}

pub(super) fn is_flying_drone(snapshot: Option<&Snapshot>, local: ClientId) -> bool {
    names_a_drone(snapshot.and_then(|snapshot| snapshot.meta.script_dvars(local).string(VEHICLE_DVAR)))
}

/// What the keys read about the player: whether he sprints at full height (the stance key then
/// dives), and the gun he is shown holding (its jam window for the fire key, and whether it
/// blocks going prone). With no shown state there is no jam roll.
pub(super) fn read_key_context(
    client: &mut ClientInput,
    ps: Option<&PlayerState>,
    shown: Option<&PlayerState>,
    world: &sim::SimWorld,
) {
    client.sprinting = ps.is_some_and(|ps| {
        ps.pm_flags & playerstate_t6::game::pm_flags::SPRINTING != 0
            && ps.view_height_current >= movement_t6::bridge::view_height::STAND as f32
    });
    let gun = shown.map(|shown| world.gun_facts().get(shown.weapon));
    client.jam_fire_time = gun.as_ref().map(|facts| facts.jam_window());
    client.weapon_blocks_prone = gun.is_some_and(|facts| facts.moves.blocks_prone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::ScriptControls;

    const ME: ClientId = ClientId(3);

    fn row(frozen: bool) -> ClientSnapshotMeta {
        ClientSnapshotMeta {
            controls: ScriptControls { frozen, ..Default::default() },
            ..Default::default()
        }
    }

    fn busy_input() -> ClientActionInput {
        let mut actions = ClientActionInput::default();
        actions.mouse_x = 4.0;
        actions.mouse_y = -2.0;
        actions.pad_move = [1.0, 0.5];
        actions.pad_look = [0.25, -0.75];
        actions.pad_look_delta = [3.0, 4.0];
        actions
    }

    #[test]
    fn a_player_with_no_match_row_counts_as_frozen() {
        assert!(is_frozen(None, None));
        assert!(is_frozen(Some(&PlayerState::ZERO), None));
    }

    #[test]
    fn the_match_row_or_the_movement_flag_freezes() {
        assert!(!is_frozen(Some(&PlayerState::ZERO), Some(&row(false))));
        assert!(is_frozen(Some(&PlayerState::ZERO), Some(&row(true))));
        let mut held = PlayerState::ZERO;
        held.pm_flags = PM_FROZEN | 0x1;
        assert!(is_frozen(Some(&held), Some(&row(false))));
        assert!(!is_frozen(None, Some(&row(false))));
    }

    #[test]
    fn only_the_deploying_state_means_a_shield_is_going_down() {
        assert!(is_setting_down_shield(Some((44, 0))));
        assert!(is_setting_down_shield(Some((44, 9))));
        assert!(!is_setting_down_shield(Some((43, 0))));
        assert!(!is_setting_down_shield(None));
    }

    #[test]
    fn a_missile_counts_until_it_is_unlinked() {
        assert!(!is_guiding_missile(None));
        assert!(!is_guiding_missile(Some(&row(false))));
        let mut guiding = row(false);
        guiding.remote_missile = Some(Default::default());
        assert!(is_guiding_missile(Some(&guiding)));
        guiding.remote_missile.as_mut().unwrap().unlink_at_ms = Some(Default::default());
        assert!(!is_guiding_missile(Some(&guiding)));
    }

    #[test]
    fn a_freeze_takes_the_mouse_and_both_sticks() {
        let mut actions = busy_input();
        gate_raw_input(&mut actions, false, false, true);
        assert_eq!((actions.mouse_x, actions.mouse_y), (0.0, 0.0));
        assert_eq!((actions.pad_move, actions.pad_look), ([0.0; 2], [0.0; 2]));
        assert_eq!(actions.pad_look_delta, [3.0, 4.0]);
    }

    #[test]
    fn a_shield_being_set_down_takes_them_too() {
        let mut actions = busy_input();
        gate_raw_input(&mut actions, false, true, false);
        assert_eq!((actions.mouse_x, actions.pad_move, actions.pad_look), (0.0, [0.0; 2], [0.0; 2]));
    }

    #[test]
    fn a_guided_missile_only_takes_the_walk() {
        let mut actions = busy_input();
        gate_raw_input(&mut actions, true, false, true);
        assert_eq!(actions.pad_move, [0.0; 2]);
        assert_eq!((actions.mouse_x, actions.mouse_y), (4.0, -2.0));
        assert_eq!(actions.pad_look, [0.25, -0.75]);
    }

    #[test]
    fn a_shield_going_down_beats_the_missile() {
        let mut actions = busy_input();
        gate_raw_input(&mut actions, true, true, false);
        assert_eq!((actions.mouse_x, actions.pad_look), (0.0, [0.0; 2]));
    }

    #[test]
    fn nothing_is_taken_from_a_free_player() {
        let mut actions = busy_input();
        gate_raw_input(&mut actions, false, false, false);
        assert_eq!((actions.mouse_x, actions.pad_move, actions.pad_look), (4.0, [1.0, 0.5], [0.25, -0.75]));
    }

    #[test]
    fn the_map_cursor_takes_the_mouse_when_the_overlay_is_up() {
        let mut actions = busy_input();
        assert_eq!(take_cursor_mouse(&mut actions, false), None);
        assert_eq!(actions.mouse_x, 4.0);
        assert_eq!(take_cursor_mouse(&mut actions, true), Some((4.0, -2.0)));
        assert_eq!((actions.mouse_x, actions.mouse_y), (0.0, 0.0));
        assert_eq!(actions.pad_look, [0.25, -0.75]);
    }

    #[test]
    fn steering_a_missile_takes_the_mouse_the_stick_and_the_pad_turn() {
        let mut actions = busy_input();
        assert_eq!(take_missile_steering(&mut actions, false), None);
        assert_eq!(actions.pad_look_delta, [3.0, 4.0]);
        assert_eq!(take_missile_steering(&mut actions, true), Some((4.0, -2.0, [0.25, -0.75])));
        assert_eq!((actions.mouse_x, actions.mouse_y), (0.0, 0.0));
        assert_eq!((actions.pad_look, actions.pad_look_delta), ([0.0; 2], [0.0; 2]));
    }

    #[test]
    fn the_cursor_is_served_before_the_missile() {
        let mut actions = busy_input();
        assert_eq!(take_cursor_mouse(&mut actions, true), Some((4.0, -2.0)));
        assert_eq!(take_missile_steering(&mut actions, true), Some((0.0, 0.0, [0.25, -0.75])));
    }

    #[test]
    fn without_shellshock_the_look_limits_are_the_unshocked_ones() {
        let mut actions = ClientActionInput::default();
        actions.shellshock_look_scale = 9.0;
        actions.cgame_max_pitch_speed = 9.0;
        actions.cgame_max_yaw_speed = 9.0;
        let ended = hud_t6::flashbang::SHELLSHOCK_LOOK_ENDED;
        let presented = PresentedSnapshot::default();
        apply_shellshock(&mut actions, Some(&PlayerState::ZERO), ME, &presented, 1000);
        assert_eq!(actions.shellshock_look_scale, ended.sensitivity);
        assert_eq!(actions.cgame_max_pitch_speed, ended.max_pitch_speed);
        assert_eq!(actions.cgame_max_yaw_speed, ended.max_yaw_speed);
        actions.shellshock_look_scale = 9.0;
        apply_shellshock(&mut actions, None, ME, &presented, 1000);
        assert_eq!(actions.shellshock_look_scale, ended.sensitivity);
    }

    #[test]
    fn centering_the_view_points_the_pitch_back_at_the_spawn_angle() {
        let mut actions = ClientActionInput::default();
        actions.client.center_view = true;
        let mut look = LookState { angles: [500, 700, 0], ..Default::default() };
        let mut ps = PlayerState::ZERO;
        ps.delta_angles = [10.0, 0.0, 0.0];
        turn_view(&mut actions, &mut look, Some(&ps), 0.016, false);
        assert_eq!(look.angles[0], (-10.0 * ANGLE2SHORT) as i32);
        assert_eq!(look.angles[1], 700);
        assert!(!actions.client.center_view);
    }

    #[test]
    fn centering_without_a_player_state_still_uses_up_the_press() {
        let mut actions = ClientActionInput::default();
        actions.client.center_view = true;
        let mut look = LookState { angles: [500, 700, 0], ..Default::default() };
        turn_view(&mut actions, &mut look, None, 0.016, false);
        assert_eq!(look.angles, [500, 700, 0]);
        assert!(!actions.client.center_view);
    }

    #[test]
    fn a_still_player_does_not_turn() {
        let mut actions = ClientActionInput::default();
        let mut look = LookState { angles: [100, 200, 300], ..Default::default() };
        turn_view(&mut actions, &mut look, None, 0.016, true);
        assert_eq!(look.angles, [100, 200, 300]);
    }

    fn crouch_event_at(sequence: u8) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.predictable_event_sequence = sequence.into();
        ps.predictable_events = [i32::from(entity_t6::event::STANCE_FORCE_CROUCH), 0, 0, 0];
        ps
    }

    #[test]
    fn a_new_life_clears_the_stance_latch_and_starts_reading_from_here() {
        let mut actions = ClientActionInput::default();
        actions.client.stance_latch = 0x200;
        latch_stance(&mut actions, Some(&crouch_event_at(1)), 4);
        assert_eq!(actions.client.stance_latch, 0);
        assert_eq!(actions.stance_life, Some(4));
        assert_eq!(actions.stance_event_sequence, 1);
        assert_eq!(actions.stance_events, [i32::from(entity_t6::event::STANCE_FORCE_CROUCH), 0, 0, 0]);
    }

    #[test]
    fn a_crouch_event_inside_a_life_sets_the_latch() {
        let mut actions = ClientActionInput::default();
        latch_stance(&mut actions, None, 4);
        assert_eq!(actions.stance_event_sequence, 0);
        latch_stance(&mut actions, Some(&crouch_event_at(1)), 4);
        assert_eq!(actions.client.stance_latch, playerstate_t6::button::CROUCH as i32);
        assert_eq!(actions.stance_event_sequence, 1);
    }

    #[test]
    fn without_a_player_state_the_same_life_changes_nothing() {
        let mut actions = ClientActionInput::default();
        latch_stance(&mut actions, Some(&crouch_event_at(2)), 4);
        let before = (actions.stance_event_sequence, actions.stance_events);
        latch_stance(&mut actions, None, 4);
        assert_eq!((actions.stance_event_sequence, actions.stance_events), before);
    }

    #[test]
    fn kick_moves_the_aim_in_whole_steps_and_keeps_the_remainder() {
        let mut look = LookState::default();
        look.kick_pending = [1.0, -0.5];
        kick_view(&mut look, false);
        assert_eq!(look.angles[0], ANGLE2SHORT.round() as i32);
        assert_eq!(look.angles[1], (-0.5 * ANGLE2SHORT).round() as i32);
        assert!(look.kick_pending[0].abs() < 1.0 / ANGLE2SHORT);
        assert_eq!(look.angles[2], 0);

        look.kick_pending = [0.001, 0.0];
        let kept = look.angles;
        kick_view(&mut look, false);
        assert_eq!(look.angles, kept);
        assert_eq!(look.kick_pending[0], 0.001);
    }

    #[test]
    fn a_frozen_player_loses_the_kick() {
        let mut look = LookState::default();
        look.kick_pending = [2.0, 3.0];
        kick_view(&mut look, true);
        assert_eq!((look.kick_pending, look.angles), ([0.0; 2], [0; 3]));
    }

    #[test]
    fn a_zoom_scope_turns_a_melee_press_into_a_zoom_change() {
        let mut actions = ClientActionInput::default();
        let mut look = LookState { melee_is_zoom: true, ..Default::default() };
        actions.client.kb.melee.was_pressed = true;
        actions.client.kb.melee.active = true;
        melee_key_to_zoom(&mut actions, &mut look);
        assert_eq!(look.zoom_presses, 1);
        assert!(!actions.client.kb.melee.was_pressed && !actions.client.kb.melee.active);
    }

    #[test]
    fn a_held_melee_key_on_a_zoom_scope_is_cleared_without_a_count() {
        let mut actions = ClientActionInput::default();
        let mut look = LookState { melee_is_zoom: true, zoom_presses: 7, ..Default::default() };
        actions.client.kb.melee.active = true;
        melee_key_to_zoom(&mut actions, &mut look);
        assert_eq!(look.zoom_presses, 7);
        assert!(!actions.client.kb.melee.active);
    }

    #[test]
    fn without_a_zoom_scope_the_melee_key_is_left_alone() {
        let mut actions = ClientActionInput::default();
        let mut look = LookState::default();
        actions.client.kb.melee.was_pressed = true;
        melee_key_to_zoom(&mut actions, &mut look);
        assert!(actions.client.kb.melee.was_pressed);
        assert_eq!(look.zoom_presses, 0);
    }

    #[test]
    fn a_drone_is_named_by_the_first_word_in_any_case() {
        assert!(names_a_drone(Some("qrdrone")));
        assert!(names_a_drone(Some("QRDrone_mp 1 2")));
        assert!(names_a_drone(Some("qrdrone_turret extra")));
        assert!(!names_a_drone(Some("heli_guard qrdrone")));
        assert!(!names_a_drone(Some("qrdron")));
        assert!(!names_a_drone(Some("")));
        assert!(!names_a_drone(None));
    }

    #[test]
    fn a_snapshot_without_the_dvar_is_not_a_drone() {
        assert!(!is_flying_drone(None, ME));
        assert!(!is_flying_drone(Some(&Snapshot::unpublished(sim::Tick(0))), ME));
    }

    #[test]
    fn the_vehicle_dvar_on_the_players_row_marks_a_drone() {
        let mut flying = Snapshot::unpublished(sim::Tick(0));
        let mut mine = ClientSnapshotMeta::default();
        mine.client_dvars.push(("BO2MP_Vehicle".to_owned(), "qrdrone_mp".to_owned()));
        flying.meta.clients.push((ME, mine));
        assert!(is_flying_drone(Some(&flying), ME));
        assert!(!is_flying_drone(Some(&flying), ClientId(4)));
    }

    #[test]
    fn sprinting_at_full_height_is_what_dives() {
        let world = sim::SimWorld::new();
        let mut client = ClientInput::default();
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = playerstate_t6::game::pm_flags::SPRINTING;
        ps.view_height_current = movement_t6::bridge::view_height::STAND as f32;
        read_key_context(&mut client, Some(&ps), None, &world);
        assert!(client.sprinting);

        ps.view_height_current -= 1.0;
        read_key_context(&mut client, Some(&ps), None, &world);
        assert!(!client.sprinting);

        read_key_context(&mut client, None, None, &world);
        assert!(!client.sprinting);
    }

    #[test]
    fn with_no_shown_state_there_is_no_jam_roll_and_prone_is_allowed() {
        let world = sim::SimWorld::new();
        let mut client = ClientInput::default();
        client.jam_fire_time = Some(50);
        client.weapon_blocks_prone = true;
        read_key_context(&mut client, None, None, &world);
        assert_eq!(client.jam_fire_time, None);
        assert!(!client.weapon_blocks_prone);
    }

    #[test]
    fn a_shown_gun_with_unknown_facts_has_a_zero_jam_window() {
        let world = sim::SimWorld::new();
        let mut client = ClientInput::default();
        read_key_context(&mut client, None, Some(&PlayerState::ZERO), &world);
        assert_eq!(client.jam_fire_time, Some(0));
        assert!(!client.weapon_blocks_prone);
    }
}
