//! What clients ask for between movement commands: joining, picking a class, being given a
//! weapon, answering a menu, plus the debug requests that only a host with debug actions on
//! may honour.

use movement_t6::bridge::{ANGLE2SHORT, SHORT2ANGLE};
use playerstate_t6::game::eflags;

use super::classes::{answer_custom_class, apply_select_class, choose_bot_class};
use super::give::apply_give_weapon;
use super::is_alive;
use super::weapon_config::apply_configuration_change;
use crate::SpawnPick;
use crate::frame::FrameWorld;
use crate::identities::MatchPhase;
use crate::input::{ClientAction, MENU_RESPONSE_BYTES};
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};

/// Whether `action` only makes sense for a client that already has a player row. Everything
/// that happens before a client has spawned (joining, naming, class choice, menus) does not.
pub(super) fn needs_player_row(action: &ClientAction) -> bool {
    !matches!(
        action,
        ClientAction::JoinMatch { .. }
            | ClientAction::LeaveMatch { .. }
            | ClientAction::SetName { .. }
            | ClientAction::UseCopycat { .. }
            | ClientAction::ActionSlot { .. }
            | ClientAction::ChooseDefaultClass { .. }
            | ClientAction::MenuResponse { .. }
    )
}

/// The requests that cheat: they are ignored unless the host was started with debug actions.
fn is_debug_request(action: &ClientAction) -> bool {
    matches!(
        action,
        ClientAction::ForceSpawn { .. }
            | ClientAction::GiveKillstreak { .. }
            | ClientAction::ForceDeath { .. }
            | ClientAction::SetMatchPhase { .. }
            | ClientAction::Move { .. }
            | ClientAction::BeginScriptMoverRotateVelocity { .. }
            | ClientAction::DebugDamage { .. }
    )
}

/// Carries out the requests of one frame in the order they arrived.
pub(super) fn apply_actions(world: &mut FrameWorld, tick: Tick, actions: &[(ClientId, ClientAction)]) {
    let debug_allowed = world.bootstrap_ref().allow_debug_actions;
    for (id, action) in actions {
        if is_debug_request(action) && !debug_allowed {
            continue;
        }
        apply_action(world, tick, *id, action);
    }
}

fn apply_action(world: &mut FrameWorld, tick: Tick, id: ClientId, action: &ClientAction) {
    match *action {
        ClientAction::JoinMatch { .. } => {
            let meta = world.client_meta_mut(id);
            if meta.lifecycle == ClientLifecycle::Connecting {
                meta.lifecycle = ClientLifecycle::ChoosingClass;
            }
        }
        ClientAction::LeaveMatch { .. } => world.retire_client(id),
        ClientAction::SetName { name, .. } => world.client_meta_mut(id).name = name,
        ClientAction::UseCopycat { .. } | ClientAction::SpawnClient { .. } => {}
        ClientAction::ChooseDefaultClass { index, .. } => {
            crate::script::answer_join(world.ecs(), id.0);
            choose_bot_class(world, id, index);
        }
        ClientAction::MenuResponse { menu, response, .. } => answer_menu(world, id, &menu, &response),
        ClientAction::ActionSlot { slot, .. } => {
            crate::script::action_slot_command(world.ecs(), id.0, slot);
            crate::t6::action_slot_pressed(world.ecs(), id.0, slot);
        }
        ClientAction::SelectClass { request_id, class_id, revision, loadout } => {
            apply_select_class(world, tick, id, request_id, class_id, revision, loadout);
        }
        ClientAction::GiveWeapon { request_id, weapon } => {
            apply_give_weapon(world, tick, id, request_id, weapon);
        }
        ClientAction::ChangeWeaponConfiguration { request_id, from, to } => {
            apply_configuration_change(world, tick, id, request_id, from, to);
        }
        ClientAction::SpawnIntermission { .. } => spawn_intermission(world, id),
        ClientAction::ForceSpawn { pick, .. } => force_spawn(world, id, pick),
        ClientAction::GiveKillstreak { name, .. } => {
            let name = crate::menu_response_text(&name);
            if !crate::t6::give_killstreak(world.ecs(), id.0, name) {
                crate::script::give_killstreak(world.ecs(), id.0, name);
            }
        }
        ClientAction::ForceDeath { .. } => crate::script::force_death(world.ecs(), tick, id.0),
        ClientAction::SetMatchPhase { phase, .. } => set_match_phase(world, phase),
        ClientAction::Move { origin, angles, .. } => debug_move(world, id, origin, angles),
        ClientAction::BeginScriptMoverRotateVelocity { speed, .. } => {
            world.begin_script_movers_rotate_velocity_supplied(speed, crate::corpse::level_time_ms(tick));
        }
        ClientAction::DebugDamage { amount, .. } => debug_damage(world, tick, id, amount),
    }
}

/// A menu answer goes to the map's own scripts first; otherwise it may be a class pick, and
/// only then the generic menu handler.
fn answer_menu(
    world: &mut FrameWorld,
    id: ClientId,
    menu: &[u8; MENU_RESPONSE_BYTES],
    response: &[u8; MENU_RESPONSE_BYTES],
) {
    let menu = crate::menu_response_text(menu);
    let response = crate::menu_response_text(response);
    if crate::t6::menu_response(world.ecs(), id.0, menu, response) {
        return;
    }
    crate::script::note_team_answer(world.ecs(), id.0, menu);
    if !answer_custom_class(world, id, menu, response) {
        crate::script::answer_menu(world.ecs(), id.0, menu, response);
    }
}

fn set_match_phase(world: &mut FrameWorld, phase: MatchPhase) {
    if phase == MatchPhase::Playing && world.phase() == MatchPhase::Warmup {
        crate::score::finish_prematch(world);
    } else {
        world.set_phase(phase);
    }
}

/// A living player is moved to the spawn at once; anyone else gets it for the next spawn.
fn force_spawn(world: &mut FrameWorld, id: ClientId, pick: SpawnPick) {
    if is_alive(world, id) {
        crate::script_player::move_to_forced_spawn(world, id, pick);
    } else {
        world.client_meta_mut(id).forced_spawn = Some(pick);
    }
}

/// Puts the player at the camera of the end-of-match scene.
fn spawn_intermission(world: &mut FrameWorld, id: ClientId) {
    let already = world
        .client_meta(id)
        .is_none_or(|meta| meta.lifecycle == ClientLifecycle::Intermission);
    if already {
        return;
    }
    let view = world.bootstrap_ref().intermission_view.clone();
    let meta = world.client_meta_mut(id);
    meta.lifecycle = ClientLifecycle::Intermission;
    meta.dead_since_tick = None;
    world.unlink_player_area(id);

    let Some(view) = view else {
        return;
    };
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    ps.origin = view.origin;
    ps.velocity = [0.0; 3];
    ps.viewangles = view.angles;
    ps.delta_angles = packed_look_delta(view.angles);
    ps.e_flags ^= eflags::TELEPORT;
}

/// Teleports a living player.
fn debug_move(world: &mut FrameWorld, id: ClientId, origin: [f32; 3], angles: [f32; 3]) {
    let finite = origin.iter().chain(&angles).all(|v| v.is_finite());
    if !finite || !is_alive(world, id) {
        return;
    }
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    let old_origin = std::mem::replace(&mut ps.origin, origin);
    ps.velocity = [0.0; 3];
    ps.viewangles = angles;
    ps.e_flags ^= eflags::TELEPORT;
    ps.delta_angles = packed_look_delta(angles);

    let shift = [0, 1, 2].map(|axis| origin[axis] - old_origin[axis]);
    world.translate_player_area(id, shift);
}

fn debug_damage(world: &mut FrameWorld, tick: Tick, id: ClientId, amount: i32) {
    if amount > 0 && is_alive(world, id) {
        crate::script_player::debug_damage(world, tick, id, amount);
    }
}

/// The offset between exact angles and the 16-bit angles the wire carries, which the client
/// subtracts again when it reads the view.
fn packed_look_delta(viewangles: [f32; 3]) -> [f32; 3] {
    viewangles.map(|angle| angle - (angle * ANGLE2SHORT) as i32 as f32 * SHORT2ANGLE)
}

/// After the commands have run, a debug move's view direction is put back exactly: the
/// commands of that frame would otherwise have turned it.
pub(super) fn restamp_debug_move_look(world: &mut FrameWorld, actions: &[(ClientId, ClientAction)]) {
    if !world.bootstrap_ref().allow_debug_actions {
        return;
    }
    for (id, action) in actions {
        let ClientAction::Move { angles, .. } = *action else {
            continue;
        };
        if !is_alive(world, *id) {
            continue;
        }
        if let Some(ps) = world.player_mut(*id) {
            ps.viewangles = angles;
            ps.delta_angles = packed_look_delta(angles);
        }
    }
}
