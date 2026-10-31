//! What clients ask for between movement commands: joining, picking a class, being given a
//! weapon, answering a menu, plus the debug requests that only a host with debug actions on
//! may honour.

use movement_t6::bridge::{DEGREES_TO_WORD16, WORD16_TO_DEGREES};
use playerstate_t6::game::entity_flag_bits;

use super::classes::{answer_custom_class, apply_select_class, choose_bot_class};
use super::give::apply_give_weapon;
use super::is_alive;
use super::weapon_config::apply_configuration_change;
use crate::SpawnPick;
use crate::step_view::StepWorldView;
use crate::match_ids::GameRoundStage;
use crate::tick_feed::{PlayerIntent, MENU_RESPONSE_BYTES};
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::{SeatNo, StepNo};

/// Whether `action` only makes sense for a client that already has a player row. Everything
/// that happens before a client has spawned (joining, naming, class choice, menus) does not.
pub(super) fn needs_player_row(action: &PlayerIntent) -> bool {
    !matches!(
        action,
        PlayerIntent::JoinMatch { .. }
            | PlayerIntent::LeaveMatch { .. }
            | PlayerIntent::SetName { .. }
            | PlayerIntent::UseCopycat { .. }
            | PlayerIntent::ActionSlot { .. }
            | PlayerIntent::ChooseDefaultClass { .. }
            | PlayerIntent::MenuResponse { .. }
    )
}

/// The requests that cheat: they are ignored unless the host was started with debug actions.
fn is_debug_request(action: &PlayerIntent) -> bool {
    matches!(
        action,
        PlayerIntent::ForceSpawn { .. }
            | PlayerIntent::GiveKillstreak { .. }
            | PlayerIntent::ForceDeath { .. }
            | PlayerIntent::StageMatch { .. }
            | PlayerIntent::Move { .. }
            | PlayerIntent::SpinMover { .. }
            | PlayerIntent::TestHarm { .. }
    )
}

/// Carries out the requests of one frame in the order they arrived.
pub(super) fn apply_actions(world: &mut StepWorldView, tick: StepNo, actions: &[(SeatNo, PlayerIntent)]) {
    let debug_allowed = world.bootstrap_ref().permit_cheat_requests;
    for (who, request) in actions {
        let blocked = is_debug_request(request) && !debug_allowed;
        if !blocked {
            apply_action(world, tick, *who, request);
        }
    }
}

fn apply_action(level: &mut StepWorldView, step: StepNo, who_no: SeatNo, request: &PlayerIntent) {
    match *request {
        PlayerIntent::JoinMatch { .. } => {
            let record = level.client_meta_mut(who_no);
            if record.lifecycle == PlayerSessionStage::Connecting {
                record.lifecycle = PlayerSessionStage::PickingLoadout;
            }
        }
        PlayerIntent::LeaveMatch { .. } => level.drop_player_record(who_no),
        PlayerIntent::SetName { name, .. } => level.client_meta_mut(who_no).name = name,
        PlayerIntent::UseCopycat { .. } | PlayerIntent::SpawnClient { .. } => {}
        PlayerIntent::ChooseDefaultClass { index, .. } => choose_bot_class(level, who_no, index),
        PlayerIntent::MenuResponse { menu, response, .. } => answer_menu(level, who_no, &menu, &response),
        PlayerIntent::ActionSlot { slot, .. } => crate::zombie_rt::action_slot_pressed(level.ecs(), who_no.0, slot),
        PlayerIntent::PickLoadout { request_id: ticket, class_id, revision, loadout } => {
            apply_select_class(level, step, who_no, ticket, class_id, revision, loadout);
        }
        PlayerIntent::HandOutGun { request_id: req, gun: weapon } => {
            apply_give_weapon(level, step, who_no, req, weapon);
        }
        PlayerIntent::ChangeWeaponConfiguration { request_id: ticket, from, to } => {
            apply_configuration_change(level, step, who_no, ticket, from, to);
        }
        PlayerIntent::SpawnIntermission { .. } => spawn_intermission(level, who_no),
        PlayerIntent::ForceSpawn { pick, .. } => force_spawn(level, who_no, pick),
        PlayerIntent::GiveKillstreak { name, .. } => {
            let name = crate::menu_response_text(&name);
            crate::zombie_rt::give_killstreak(level.ecs(), who_no.0, name);
        }
        PlayerIntent::ForceDeath { .. } => crate::gsc_host::force_death(level.ecs(), step, who_no.0),
        PlayerIntent::StageMatch { phase, .. } => set_match_phase(level, phase),
        PlayerIntent::Move { origin, angles, .. } => debug_move(level, who_no, origin, angles),
        PlayerIntent::SpinMover { speed, .. } => {
            level.start_rate_spins(speed, crate::bodies::level_time_ms(step));
        }
        PlayerIntent::TestHarm { amount, .. } => debug_damage(level, step, who_no, amount),
    }
}

/// A menu answer goes to the map's own scripts first; otherwise it may be a class pick, and
/// only then the generic menu handler.
fn answer_menu(
    world: &mut StepWorldView,
    id: SeatNo,
    menu: &[u8; MENU_RESPONSE_BYTES],
    response: &[u8; MENU_RESPONSE_BYTES],
) {
    let menu = crate::menu_response_text(menu);
    let response = crate::menu_response_text(response);
    if crate::zombie_rt::menu_response(world.ecs(), id.0, menu, response) {
        return;
    }
    answer_custom_class(world, id, menu, response);
}

fn set_match_phase(level: &mut StepWorldView, stage: GameRoundStage) {
    let ends_warmup = matches!(
        (stage, level.phase()),
        (GameRoundStage::Playing, GameRoundStage::Warmup)
    );
    if ends_warmup {
        crate::round_clock::finish_prematch(level);
    } else {
        level.set_phase(stage)
    }
}

/// A living player is moved to the spawn at once; anyone else gets it for the next spawn.
fn force_spawn(world: &mut StepWorldView, id: SeatNo, pick: SpawnPick) {
    if is_alive(world, id) {
        crate::client_script::move_to_forced_spawn(world, id, pick);
    } else {
        world.client_meta_mut(id).forced_spawn = Some(pick);
    }
}

/// Puts the player at the camera of the end-of-match scene.
fn spawn_intermission(world: &mut StepWorldView, client: SeatNo) {
    let already = world
        .client_meta(client)
        .is_none_or(|meta| meta.lifecycle == PlayerSessionStage::Intermission);
    if already {
        return;
    }
    let camera = world.bootstrap_ref().scoreboard_eye.clone();
    let record = world.client_meta_mut(client);
    record.lifecycle = PlayerSessionStage::Intermission;
    record.died_at_step = None;
    world.release_seat_area(client);

    let (Some(camera), Some(player)) = (camera, world.occupant_mut(client)) else {
        return;
    };
    player.origin = camera.origin;
    player.velocity = [0.0; 3];
    player.look_angles = camera.turn;
    player.view_offset_angles = packed_look_delta(camera.turn);
    player.e_flags ^= entity_flag_bits::TELEPORT;
}

/// Teleports a living player.
fn debug_move(world: &mut StepWorldView, id: SeatNo, spot: [f32; 3], aim: [f32; 3]) {
    let finite = spot.iter().chain(&aim).all(|v| v.is_finite());
    if !finite || !is_alive(world, id) {
        return;
    }
    let Some(player) = world.occupant_mut(id) else {
        return;
    };
    let old_origin = std::mem::replace(&mut player.origin, spot);
    player.velocity = [0.0; 3];
    player.look_angles = aim;
    player.e_flags ^= entity_flag_bits::TELEPORT;
    player.view_offset_angles = packed_look_delta(aim);

    let shift = [0, 1, 2].map(|axis| spot[axis] - old_origin[axis]);
    world.translate_player_area(id, shift);
}

fn debug_damage(world: &mut StepWorldView, tick: StepNo, id: SeatNo, qty: i32) {
    if qty > 0 && is_alive(world, id) {
        crate::client_script::debug_damage(world, tick, id, qty);
    }
}

/// The offset between exact angles and the 16-bit angles the wire carries, which the client
/// subtracts again when it reads the view.
fn packed_look_delta(look: [f32; 3]) -> [f32; 3] {
    look.map(|angle| angle - (angle * DEGREES_TO_WORD16) as i32 as f32 * WORD16_TO_DEGREES)
}

/// After the commands have run, a debug move's view direction is put back exactly: the
/// commands of that frame would otherwise have turned it.
pub(super) fn retag_cheat_move_aim(world: &mut StepWorldView, actions: &[(SeatNo, PlayerIntent)]) {
    if !world.bootstrap_ref().permit_cheat_requests {
        return;
    }
    for (who, request) in actions {
        let &PlayerIntent::Move { angles: aim, .. } = request else {
            continue;
        };
        if !is_alive(world, *who) {
            continue;
        }
        if let Some(player) = world.occupant_mut(*who) {
            player.look_angles = aim;
            player.view_offset_angles = packed_look_delta(aim);
        }
    }
}
