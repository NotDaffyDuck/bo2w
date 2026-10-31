//! The frame's read of the player's input. The keys, the mouse and the pad are turned into the
//! command template that the prediction step sends on, after the state the player is in has
//! taken away whatever it does not allow.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use playerstate_t6::game::{buttons, last_input};
use sim::PlayerIntent;

use super::location_cursor::{LocationCursor, steer_command};
use super::look_input::{
    apply_shellshock, gate_raw_input, is_flying_drone, is_frozen, is_guiding_missile, is_setting_down_shield,
    kick_view, latch_stance, melee_key_to_zoom, read_key_context, take_cursor_mouse, take_missile_steering, turn_view,
};
use super::melee_pick::aim_melee_charge;
use super::pad_assist::steer_pad_look;
use super::stores::{PlayerMoveBlueprint, ReceiverStageLog, ClientGuessState, note_phase};
use super::gun_pick::{WeaponSelect, follow_held_weapon_select, out_of_ammo_change};
use super::weapon_keys::{
    ActionRequests, Held, breath_key_switches_optic, hybrid_optic, inventory_key, offhand_choice, press_slots,
    run_cycles, track_held_mapping,
};
use crate::host_tick::tick_systems::HostInputValve;
use crate::client::frame_clock::FrameClock;
use crate::client::local_input::{
    PlayerControlSnapshot, ViewAimAccumulator, assemble_input_record, frame_time_msec, pressed_frame_millis, remote_control_axes,
};
use crate::client::presentation::shown_frame::{HostViewClient, ShownWorldFrame};
use crate::client::realtime::ClientRealtime;

/// What the frame already knows, none of it changed here.
#[derive(SystemParam)]
pub struct FrameFacts<'w> {
    time: Res<'w, Time>,
    forecast: Res<'w, ClientGuessState>,
    world_view: Res<'w, ShownWorldFrame>,
    own_id: Res<'w, HostViewClient>,
    cls: Res<'w, ClientRealtime>,
    clock: Res<'w, FrameClock>,
    view: Option<Res<'w, frame::EyeSubject>>,
}

/// The input state the sampler keeps and rewrites.
#[derive(SystemParam)]
pub struct InputState<'w> {
    ctl: ResMut<'w, PlayerControlSnapshot>,
    look: ResMut<'w, ViewAimAccumulator>,
    template: ResMut<'w, PlayerMoveBlueprint>,
    select: ResMut<'w, WeaponSelect>,
    cursor: ResMut<'w, LocationCursor>,
}

pub fn poll_receiver_controls(
    facts: FrameFacts,
    input: InputState,
    gate: Res<HostInputValve>,
    mut requests: ActionRequests,
    trace: Option<ResMut<ReceiverStageLog>>,
) {
    note_phase(trace, "Input");
    let FrameFacts { time, forecast, world_view, own_id, cls, clock, view } = facts;
    let InputState { mut ctl, look: mut aim_state, template: mut blueprint, select: mut gun_choice, mut cursor } = input;

    if !gate.local_cmds_enabled {
        if !ctl.client.gun_swap_steps.is_empty() {
            diag::info!(WireLink, "bo2mp cycle: dropped, local commands are off");
        }
        ctl.client.gun_swap_steps.clear();
        ctl.client.action_slots.clear();
        return;
    }
    ctl.tick_span_ms = pressed_frame_millis(time.delta_secs());
    ctl.present_tick_ms = frame_time_msec(time.elapsed_secs());

    let me = own_id.0;
    let world = forecast.0.world();
    let scene_clock_ms = clock.time();
    let own_state = forecast.0.predicted_local().or_else(|| world_view.player(me));
    let my_row = world_view.snapshot().and_then(|snapshot| snapshot.meta.for_client(me));
    read_key_context(&mut ctl.client, own_state, world_view.player(me), world);

    // What the gun brain runs on: the predicted raw weapon state and grenade, or the last
    // snapshot's state with no grenade before prediction has started the brain.
    let frozen = is_frozen(own_state, my_row);
    let offhand = world
        .client_meta(me)
        .and_then(|meta| meta.t6_offhand())
        .or_else(|| my_row.and_then(|row| row.t6_view).map(|shown| (shown.weaponstate, 0)));
    let guiding = is_guiding_missile(my_row);
    gate_raw_input(&mut ctl, guiding, is_setting_down_shield(offhand), frozen);

    match own_state {
        Some(ps) => steer_pad_look(&mut ctl, ps, me, aim_state.angles, cls.step_duration_seconds(), &world_view, world),
        None => ctl.pad_look_delta = [0.0; 2],
    }
    if frozen {
        ctl.pad_look_delta = [0.0; 2];
    }

    let choose_direction = my_row
        .and_then(|row| row.location_selection.as_ref())
        .map(|selection| selection.choose_direction);
    let location_mouse = take_cursor_mouse(&mut ctl, choose_direction.is_some());
    let remote_mouse = take_missile_steering(&mut ctl, guiding);

    apply_shellshock(&mut ctl, own_state, me, &world_view, scene_clock_ms);
    turn_view(&mut ctl, &mut aim_state, own_state, cls.step_duration_seconds(), frozen);
    let life_no = my_row.map_or(0, |row| row.life_sequence.0);
    latch_stance(&mut ctl, own_state, life_no);
    follow_held_weapon_select(&mut gun_choice, own_state.map_or(0, |ps| ps.weapon), life_no, scene_clock_ms);
    if let Some(own_state) = own_state {
        out_of_ammo_change(&mut gun_choice, own_state, world, scene_clock_ms);
    }

    let held = own_state.map(|ps| Held { ps, sim_world: world, now: scene_clock_ms, offhand, scripted_hold: frozen });
    if let Some(held) = &held {
        track_held_mapping(&mut gun_choice, held);
    }

    // A slot press is always passed on to the host; picking the gun it names is only for a
    // player who can move.
    let slots = std::mem::take(&mut ctl.client.action_slots);
    for &slot in &slots {
        requests.push(me, "action_slot", |request_id| PlayerIntent::ActionSlot { request_id, slot: slot as u8 });
    }
    if let Some(held) = held.as_ref().filter(|held| !held.scripted_hold) {
        press_slots(&mut gun_choice, &mut ctl.client, held, &slots);
    }
    if let Some(held) = &held
        && breath_key_switches_optic(&ctl.client, &aim_state, frozen)
    {
        hybrid_optic(&mut gun_choice, held);
    }

    let cycles = std::mem::take(&mut ctl.client.gun_swap_steps);
    // The switch-weapon key is Black Ops II's change-seat button too, on the press.
    let change_seat = std::mem::take(&mut ctl.client.change_seat_press);
    let replaying_final_death = view.is_some_and(|view| view.replaying_final_death());
    match &held {
        Some(held) => run_cycles(&mut gun_choice, &mut ctl.client, held, &cycles, replaying_final_death, &mut requests, me),
        None if !cycles.is_empty() => diag::info!(WireLink, "bo2mp cycle: dropped, no player state yet"),
        None => {}
    }

    kick_view(&mut aim_state, frozen);
    melee_key_to_zoom(&mut ctl, &mut aim_state);

    // Flying a drone, Ctrl is "down" while held, so its toggle is dropped and the held key goes
    // as crouch.
    let drone = is_flying_drone(world_view.snapshot(), me);
    if drone {
        ctl.client.posture_hold = 0;
    }
    let mut cmd = assemble_input_record(&mut ctl, &aim_state, 0);
    if drone && input_t6::lower_key_held(&ctl.client) {
        cmd.buttons |= buttons::DUCK_BUTTON_BIT;
    }
    cmd.last_input = if ctl.last_input_pad { last_input::GAMEPAD } else { last_input::MOUSECLICK };
    if cmd.buttons & buttons::INTERACT_RELOAD_KEY != 0 && own_state.is_some_and(|ps| ps.hint_kind == 0) {
        cmd.buttons |= buttons::RELOAD;
    }
    aim_state.angles = cmd.angles;
    if let Some((pointer_delta_horizontal, pointer_delta_vertical, pad_look)) = remote_mouse {
        cmd.remote_operated = remote_control_axes(&ctl, pointer_delta_horizontal, pointer_delta_vertical, pad_look);
        cmd.buttons |= buttons::REMOTE_CONTROL;
    }
    if change_seat {
        cmd.buttons |= buttons::CHANGE_SEAT;
    }

    let inventory_request = std::mem::take(&mut ctl.client.inventory_request);
    if let Some(held) = &held {
        // The snapshot's own state of him, not the predicted one the rest reads.
        let server_state = world_view
            .snapshot()
            .and_then(|snapshot| snapshot.players.iter().find(|(id, _)| *id == me))
            .map(|(_, sv)| sv);
        if inventory_key(&mut gun_choice, &mut ctl.client, held, inventory_request, server_state) {
            cmd.buttons |= buttons::INVENTORY;
        }
    }
    cmd.weapon = gun_choice.index as u16;
    cmd.weapon_slot_code = gun_choice.mapped_index as u16;
    if let Some(loadout) = my_row.and_then(|row| row.loadout.as_ref())
        && let Some(grenade) = offhand_choice(
            &ctl.client.pressed_key_latch.frag,
            &ctl.client.pressed_key_latch.smoke,
            loadout.lethal as u16,
            loadout.tactical as u16,
        )
    {
        cmd.secondary_hand_slot = grenade;
    }

    let already_picking = blueprint.cmd.buttons & buttons::MAP_TARGET_PICK != 0;
    steer_command(&mut cursor, &mut cmd, ctl.sensitivity, location_mouse, choose_direction, already_picking);
    aim_melee_charge(&mut cmd, own_state, me, world_view.snapshot(), &world_view, world);

    blueprint.cmd = cmd;
    blueprint.prefilled = true;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sim::ClientId;

    use super::*;
    use crate::{ActionRequestCounter, host_tick::peer_mail::PlayerActionIntake};

    const ME: ClientId = ClientId(1);

    fn app(commands_on: bool) -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<PlayerControlSnapshot>()
            .init_resource::<ViewAimAccumulator>()
            .init_resource::<PlayerMoveBlueprint>()
            .init_resource::<WeaponSelect>()
            .init_resource::<LocationCursor>()
            .init_resource::<ClientGuessState>()
            .init_resource::<ShownWorldFrame>()
            .insert_resource(HostViewClient(ME))
            .insert_resource(HostInputValve { local_cmds_enabled: commands_on })
            .init_resource::<ClientRealtime>()
            .init_resource::<FrameClock>()
            .add_systems(Update, poll_receiver_controls);
        app
    }

    fn frame(app: &mut App) {
        app.world_mut().resource_mut::<Time>().advance_by(Duration::from_millis(16));
        app.update();
    }

    fn ctl(app: &mut App) -> Mut<'_, PlayerControlSnapshot> {
        app.world_mut().resource_mut::<PlayerControlSnapshot>()
    }

    fn template(app: &App) -> &PlayerMoveBlueprint {
        app.world().resource::<PlayerMoveBlueprint>()
    }

    #[test]
    fn with_local_commands_off_the_queued_presses_are_dropped_and_no_command_is_built() {
        let mut app = app(false);
        ctl(&mut app).client.gun_swap_steps.push(true);
        ctl(&mut app).client.action_slots.push(2);
        frame(&mut app);
        assert!(ctl(&mut app).client.gun_swap_steps.is_empty());
        assert!(ctl(&mut app).client.action_slots.is_empty());
        assert!(!template(&app).prefilled);
    }

    #[test]
    fn the_phase_is_traced_whether_or_not_commands_are_on() {
        for commands_on in [false, true] {
            let mut app = app(commands_on);
            app.insert_resource(ReceiverStageLog::default());
            frame(&mut app);
            assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["Input"]);
        }
    }

    #[test]
    fn a_frame_before_any_player_state_still_leaves_a_ready_command() {
        let mut app = app(true);
        frame(&mut app);
        assert!(template(&app).prefilled);
        assert_eq!(ctl(&mut app).tick_span_ms, pressed_frame_millis(0.016));
    }

    #[test]
    fn the_command_names_the_gun_that_is_selected() {
        let mut app = app(true);
        {
            let mut select = app.world_mut().resource_mut::<WeaponSelect>();
            select.index = 7;
            select.mapped_index = 9;
        }
        frame(&mut app);
        assert_eq!((template(&app).cmd.weapon, template(&app).cmd.weapon_slot_code), (7, 9));
    }

    #[test]
    fn a_player_the_match_has_no_row_for_gets_no_mouse() {
        let mut app = app(true);
        ctl(&mut app).pointer_delta_horizontal = 9.0;
        ctl(&mut app).pointer_delta_vertical = -3.0;
        frame(&mut app);
        assert_eq!((ctl(&mut app).pointer_delta_horizontal, ctl(&mut app).pointer_delta_vertical), (0.0, 0.0));
    }

    #[test]
    fn the_command_says_which_device_was_used_last() {
        let mut app = app(true);
        ctl(&mut app).last_input_pad = true;
        frame(&mut app);
        assert_eq!(template(&app).cmd.last_input, last_input::GAMEPAD);
        ctl(&mut app).last_input_pad = false;
        frame(&mut app);
        assert_eq!(template(&app).cmd.last_input, last_input::MOUSECLICK);
    }

    #[test]
    fn the_seat_change_press_rides_one_command_only() {
        let mut app = app(true);
        ctl(&mut app).client.change_seat_press = true;
        frame(&mut app);
        assert_ne!(template(&app).cmd.buttons & buttons::CHANGE_SEAT, 0);
        frame(&mut app);
        assert_eq!(template(&app).cmd.buttons & buttons::CHANGE_SEAT, 0);
    }

    #[test]
    fn cycles_with_no_player_state_are_dropped_and_leave_the_gun_alone() {
        let mut app = app(true);
        ctl(&mut app).client.gun_swap_steps.extend([true, false]);
        frame(&mut app);
        assert!(ctl(&mut app).client.gun_swap_steps.is_empty());
        assert_eq!(app.world().resource::<WeaponSelect>().index, 0);
    }

    #[test]
    fn an_inventory_press_is_used_up_even_with_nobody_to_switch() {
        let mut app = app(true);
        ctl(&mut app).client.inventory_request = true;
        frame(&mut app);
        assert!(!ctl(&mut app).client.inventory_request);
        assert_eq!(template(&app).cmd.buttons & buttons::INVENTORY, 0);
    }

    #[test]
    fn a_slot_press_is_queued_for_the_host_with_or_without_a_player_state() {
        let mut app = app(true);
        app.init_resource::<PlayerActionIntake>().init_resource::<ActionRequestCounter>();
        ctl(&mut app).client.action_slots.extend([3, 1]);
        frame(&mut app);
        let queued = app.world().resource::<PlayerActionIntake>().pending().to_vec();
        assert_eq!(queued.len(), 2);
        assert!(matches!(queued[0], (ME, PlayerIntent::ActionSlot { slot: 3, .. })));
        assert!(matches!(queued[1], (ME, PlayerIntent::ActionSlot { slot: 1, .. })));
        assert!(ctl(&mut app).client.action_slots.is_empty());
    }
}
