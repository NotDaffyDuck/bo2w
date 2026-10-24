//! The frame's read of the player's input. The keys, the mouse and the pad are turned into the
//! command template that the prediction step sends on, after the state the player is in has
//! taken away whatever it does not allow.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use playerstate_t6::game::{buttons, last_input};
use sim::ClientAction;

use super::location_cursor::{LocationCursor, steer_command};
use super::look_input::{
    apply_shellshock, gate_raw_input, is_flying_drone, is_frozen, is_guiding_missile, is_setting_down_shield,
    kick_view, latch_stance, melee_key_to_zoom, read_key_context, take_cursor_mouse, take_missile_steering, turn_view,
};
use super::melee_pick::aim_melee_charge;
use super::pad_assist::steer_pad_look;
use super::stores::{ClientCmdTemplate, ClientPhaseTrace, ClientPredictionState, note_phase};
use super::gun_pick::{WeaponSelect, follow_held_weapon_select};
use super::weapon_keys::{
    ActionRequests, Held, breath_key_switches_optic, hybrid_optic, inventory_key, offhand_choice, press_slots,
    run_cycles, track_held_mapping,
};
use crate::authority::runtime::AuthorityInputGate;
use crate::client::frame_clock::FrameClock;
use crate::client::input::{
    ClientActionInput, LookState, build_usercmd, frame_time_msec, key_frame_msec, remote_control_axes,
};
use crate::client::presentation::presented::{LocalPresentClient, PresentedSnapshot};
use crate::client::realtime::ClientRealtime;

/// What the frame already knows, none of it changed here.
#[derive(SystemParam)]
pub struct FrameFacts<'w> {
    time: Res<'w, Time>,
    prediction: Res<'w, ClientPredictionState>,
    presented: Res<'w, PresentedSnapshot>,
    local: Res<'w, LocalPresentClient>,
    cls: Res<'w, ClientRealtime>,
    clock: Res<'w, FrameClock>,
    view: Option<Res<'w, frame::ViewSubject>>,
}

/// The input state the sampler keeps and rewrites.
#[derive(SystemParam)]
pub struct InputState<'w> {
    actions: ResMut<'w, ClientActionInput>,
    look: ResMut<'w, LookState>,
    template: ResMut<'w, ClientCmdTemplate>,
    select: ResMut<'w, WeaponSelect>,
    cursor: ResMut<'w, LocationCursor>,
}

pub fn sample_client_input(
    facts: FrameFacts,
    input: InputState,
    gate: Res<AuthorityInputGate>,
    mut requests: ActionRequests,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    note_phase(trace, "Input");
    let FrameFacts { time, prediction, presented, local, cls, clock, view } = facts;
    let InputState { mut actions, mut look, mut template, mut select, mut cursor } = input;

    if !gate.local_cmds_enabled {
        if !actions.client.weapon_cycles.is_empty() {
            diag::info!(Net, "bo2mp cycle: dropped, local commands are off");
        }
        actions.client.weapon_cycles.clear();
        actions.client.action_slots.clear();
        return;
    }
    actions.frame_msec = key_frame_msec(time.delta_secs());
    actions.now_msec = frame_time_msec(time.elapsed_secs());

    let me = local.0;
    let world = prediction.0.world();
    let cg_time = clock.time();
    let ps = prediction.0.predicted_local().or_else(|| presented.player(me));
    let my_row = presented.snapshot().and_then(|snapshot| snapshot.meta.for_client(me));
    read_key_context(&mut actions.client, ps, presented.player(me), world);

    // What the gun brain runs on: the predicted raw weapon state and grenade, or the last
    // snapshot's state with no grenade before prediction has started the brain.
    let frozen = is_frozen(ps, my_row);
    let offhand = world
        .client_meta(me)
        .and_then(|meta| meta.t6_offhand())
        .or_else(|| my_row.and_then(|row| row.t6_view).map(|shown| (shown.weaponstate, 0)));
    let guiding = is_guiding_missile(my_row);
    gate_raw_input(&mut actions, guiding, is_setting_down_shield(offhand), frozen);

    match ps {
        Some(ps) => steer_pad_look(&mut actions, ps, me, look.angles, cls.frametime_secs(), &presented, world),
        None => actions.pad_look_delta = [0.0; 2],
    }
    if frozen {
        actions.pad_look_delta = [0.0; 2];
    }

    let choose_direction = my_row
        .and_then(|row| row.location_selection.as_ref())
        .map(|selection| selection.choose_direction);
    let location_mouse = take_cursor_mouse(&mut actions, choose_direction.is_some());
    let remote_mouse = take_missile_steering(&mut actions, guiding);

    apply_shellshock(&mut actions, ps, me, &presented, cg_time);
    turn_view(&mut actions, &mut look, ps, cls.frametime_secs(), frozen);
    let life_sequence = my_row.map_or(0, |row| row.life_sequence.0);
    latch_stance(&mut actions, ps, life_sequence);
    follow_held_weapon_select(&mut select, ps.map_or(0, |ps| ps.weapon), life_sequence, cg_time);

    let held = ps.map(|ps| Held { ps, world, now: cg_time, offhand, frozen });
    if let Some(held) = &held {
        track_held_mapping(&mut select, held);
    }

    // A slot press is always passed on to the host; picking the gun it names is only for a
    // player who can move.
    let slots = std::mem::take(&mut actions.client.action_slots);
    for &slot in &slots {
        requests.push(me, "action_slot", |request_id| ClientAction::ActionSlot { request_id, slot: slot as u8 });
    }
    if let Some(held) = held.as_ref().filter(|held| !held.frozen) {
        press_slots(&mut select, &mut actions.client, held, &slots);
    }
    if let Some(held) = &held
        && breath_key_switches_optic(&actions.client, &look, frozen)
    {
        hybrid_optic(&mut select, held);
    }

    let cycles = std::mem::take(&mut actions.client.weapon_cycles);
    // The switch-weapon key is Black Ops II's change-seat button too, on the press.
    let change_seat = std::mem::take(&mut actions.client.change_seat_press);
    let in_killcam = view.is_some_and(|view| view.in_killcam());
    match &held {
        Some(held) => run_cycles(&mut select, &mut actions.client, held, &cycles, in_killcam, &mut requests, me),
        None if !cycles.is_empty() => diag::info!(Net, "bo2mp cycle: dropped, no player state yet"),
        None => {}
    }

    kick_view(&mut look, frozen);
    melee_key_to_zoom(&mut actions, &mut look);

    // Flying a drone, Ctrl is "down" while held, so its toggle is dropped and the held key goes
    // as crouch.
    let drone = is_flying_drone(presented.snapshot(), me);
    if drone {
        actions.client.stance_latch = 0;
    }
    let mut cmd = build_usercmd(&mut actions, &look, 0);
    if drone && input_t6::lower_key_held(&actions.client) {
        cmd.buttons |= buttons::CROUCH;
    }
    cmd.last_input = if actions.last_input_pad { last_input::GAMEPAD } else { last_input::MOUSECLICK };
    if cmd.buttons & buttons::USE_RELOAD != 0 && ps.is_some_and(|ps| ps.cursor_hint == 0) {
        cmd.buttons |= buttons::RELOAD;
    }
    look.angles = cmd.angles;
    if let Some((mouse_x, mouse_y, pad_look)) = remote_mouse {
        cmd.remote_control = remote_control_axes(&actions, mouse_x, mouse_y, pad_look);
        cmd.buttons |= buttons::REMOTE_CONTROL;
    }
    if change_seat {
        cmd.buttons |= buttons::CHANGE_SEAT;
    }

    let inventory_request = std::mem::take(&mut actions.client.inventory_request);
    if let Some(held) = &held {
        // The snapshot's own state of him, not the predicted one the rest reads.
        let server_state = presented
            .snapshot()
            .and_then(|snapshot| snapshot.players.iter().find(|(id, _)| *id == me))
            .map(|(_, state)| state);
        if inventory_key(&mut select, &mut actions.client, held, inventory_request, server_state) {
            cmd.buttons |= buttons::INVENTORY;
        }
    }
    cmd.weapon = select.index as u16;
    cmd.weapon_mapped = select.mapped_index as u16;
    if let Some(loadout) = my_row.and_then(|row| row.loadout.as_ref())
        && let Some(grenade) = offhand_choice(
            &actions.client.kb.frag,
            &actions.client.kb.smoke,
            loadout.lethal as u16,
            loadout.tactical as u16,
        )
    {
        cmd.off_hand_index = grenade;
    }

    let already_picking = template.cmd.buttons & buttons::LOCATION_SELECT != 0;
    steer_command(&mut cursor, &mut cmd, actions.sensitivity, location_mouse, choose_direction, already_picking);
    aim_melee_charge(&mut cmd, ps, me, presented.snapshot(), &presented, world);

    template.cmd = cmd;
    template.ready = true;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sim::ClientId;

    use super::*;
    use crate::{ActionRequestIds, authority::inbox::ClientActionInbox};

    const ME: ClientId = ClientId(1);

    fn app(commands_on: bool) -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<ClientActionInput>()
            .init_resource::<LookState>()
            .init_resource::<ClientCmdTemplate>()
            .init_resource::<WeaponSelect>()
            .init_resource::<LocationCursor>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<PresentedSnapshot>()
            .insert_resource(LocalPresentClient(ME))
            .insert_resource(AuthorityInputGate { local_cmds_enabled: commands_on })
            .init_resource::<ClientRealtime>()
            .init_resource::<FrameClock>()
            .add_systems(Update, sample_client_input);
        app
    }

    fn frame(app: &mut App) {
        app.world_mut().resource_mut::<Time>().advance_by(Duration::from_millis(16));
        app.update();
    }

    fn actions(app: &mut App) -> Mut<'_, ClientActionInput> {
        app.world_mut().resource_mut::<ClientActionInput>()
    }

    fn template(app: &App) -> &ClientCmdTemplate {
        app.world().resource::<ClientCmdTemplate>()
    }

    #[test]
    fn with_local_commands_off_the_queued_presses_are_dropped_and_no_command_is_built() {
        let mut app = app(false);
        actions(&mut app).client.weapon_cycles.push(true);
        actions(&mut app).client.action_slots.push(2);
        frame(&mut app);
        assert!(actions(&mut app).client.weapon_cycles.is_empty());
        assert!(actions(&mut app).client.action_slots.is_empty());
        assert!(!template(&app).ready);
    }

    #[test]
    fn the_phase_is_traced_whether_or_not_commands_are_on() {
        for commands_on in [false, true] {
            let mut app = app(commands_on);
            app.insert_resource(ClientPhaseTrace::default());
            frame(&mut app);
            assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Input"]);
        }
    }

    #[test]
    fn a_frame_before_any_player_state_still_leaves_a_ready_command() {
        let mut app = app(true);
        frame(&mut app);
        assert!(template(&app).ready);
        assert_eq!(actions(&mut app).frame_msec, key_frame_msec(0.016));
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
        assert_eq!((template(&app).cmd.weapon, template(&app).cmd.weapon_mapped), (7, 9));
    }

    #[test]
    fn a_player_the_match_has_no_row_for_gets_no_mouse() {
        let mut app = app(true);
        actions(&mut app).mouse_x = 9.0;
        actions(&mut app).mouse_y = -3.0;
        frame(&mut app);
        assert_eq!((actions(&mut app).mouse_x, actions(&mut app).mouse_y), (0.0, 0.0));
    }

    #[test]
    fn the_command_says_which_device_was_used_last() {
        let mut app = app(true);
        actions(&mut app).last_input_pad = true;
        frame(&mut app);
        assert_eq!(template(&app).cmd.last_input, last_input::GAMEPAD);
        actions(&mut app).last_input_pad = false;
        frame(&mut app);
        assert_eq!(template(&app).cmd.last_input, last_input::MOUSECLICK);
    }

    #[test]
    fn the_seat_change_press_rides_one_command_only() {
        let mut app = app(true);
        actions(&mut app).client.change_seat_press = true;
        frame(&mut app);
        assert_ne!(template(&app).cmd.buttons & buttons::CHANGE_SEAT, 0);
        frame(&mut app);
        assert_eq!(template(&app).cmd.buttons & buttons::CHANGE_SEAT, 0);
    }

    #[test]
    fn cycles_with_no_player_state_are_dropped_and_leave_the_gun_alone() {
        let mut app = app(true);
        actions(&mut app).client.weapon_cycles.extend([true, false]);
        frame(&mut app);
        assert!(actions(&mut app).client.weapon_cycles.is_empty());
        assert_eq!(app.world().resource::<WeaponSelect>().index, 0);
    }

    #[test]
    fn an_inventory_press_is_used_up_even_with_nobody_to_switch() {
        let mut app = app(true);
        actions(&mut app).client.inventory_request = true;
        frame(&mut app);
        assert!(!actions(&mut app).client.inventory_request);
        assert_eq!(template(&app).cmd.buttons & buttons::INVENTORY, 0);
    }

    #[test]
    fn a_slot_press_is_queued_for_the_host_with_or_without_a_player_state() {
        let mut app = app(true);
        app.init_resource::<ClientActionInbox>().init_resource::<ActionRequestIds>();
        actions(&mut app).client.action_slots.extend([3, 1]);
        frame(&mut app);
        let queued = app.world().resource::<ClientActionInbox>().pending().to_vec();
        assert_eq!(queued.len(), 2);
        assert!(matches!(queued[0], (ME, ClientAction::ActionSlot { slot: 3, .. })));
        assert!(matches!(queued[1], (ME, ClientAction::ActionSlot { slot: 1, .. })));
        assert!(actions(&mut app).client.action_slots.is_empty());
    }
}
