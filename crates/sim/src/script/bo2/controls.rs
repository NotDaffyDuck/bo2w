//! What the match's scripts hand the controls and the HUD each tick: the
//! weapon locks the engine keeps up to date, location picks (airstrike-style
//! streaks), the server info the client reads, and the start of the tick's
//! settling pass.

use bevy_ecs::prelude::{Resource, World};
use playerstate_t6::game::{UserCmd, buttons};

use super::runtime::Runtime;
use crate::frame::FrameWorld;
use crate::{ClientId, ClientLifecycle, ObjectiveMatch, WeaponLock};

/// Server-info dvars the client shows next to the ones the post-fx rows need.
const SERVER_INFO: [&str; 3] = ["ui_bomb_timer", "mapname", "g_gametype"];

/// Marks the match as one the scripts settle collision for at the start of a
/// tick (so whatever runs next already sees each body in its final pose).
#[derive(Resource, Clone, Default)]
pub(crate) struct Mechanics;

pub(crate) fn advance_mechanics(world: &mut World) {
    if !world
        .resource::<crate::step::StepRequest>()
        .reason
        .advances_authority_world()
    {
        return;
    }
    let runtime = world.resource::<Runtime>();
    if runtime.fault.is_some() || runtime.program.is_none() {
        return;
    }
    super::presence::settle_collision(world);
}

/// The match rows the scripts own: the server-info dvars the client reads, and
/// whether a script program is running.
pub(crate) fn publish_objectives(world: &mut World) {
    let runtime = world.resource::<Runtime>();
    let info = SERVER_INFO
        .iter()
        .filter_map(|&name| Some((name.to_owned(), runtime.dvars.get(name)?.clone())))
        .chain(
            runtime
                .dvars
                .iter()
                .filter(|(name, _)| crate::is_postfx_dvar(name))
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .collect();
    let objectives = ObjectiveMatch {
        server_info: info,
        scripted_effects: runtime.program.is_some(),
        ..ObjectiveMatch::default()
    };
    FrameWorld::from_world(world).objectives = objectives;
}

/// Keep each client's weapon lock honest: it holds only while the client lives
/// and still holds the weapon it was taken with, and its target follows what
/// the lock is aimed at.
pub(crate) fn sync_script_locks(world: &mut World) {
    let mut frame = FrameWorld::from_world(world);
    for id in frame.client_ids_sorted() {
        let Some(ps) = frame.player(id).copied() else {
            continue;
        };
        let Some(meta) = frame.client_meta(id) else {
            continue;
        };
        let lock = meta.weapon_lock;
        let valid = lock.weapon == ps.weapon
            && lock.life == meta.life_sequence.0
            && meta.lifecycle == ClientLifecycle::Alive;
        let point = lock.aim.filter(|_| valid).and_then(|aim| aim.resolve(&frame));
        if let Some(point) = point {
            frame.client_meta_mut(id).weapon_lock.target = point;
        } else if lock.aim.is_some() {
            frame.client_meta_mut(id).weapon_lock = WeaponLock::default();
        }
    }
}

/// A client choosing a spot on the map (Lightning Strike and the like): a click
/// picks, the cancel button backs out.
pub(crate) fn select_location(world: &mut World, client: u32, cmd: &mut UserCmd, old_buttons: u32) {
    let selecting = FrameWorld::from_world(world)
        .client_meta(ClientId(client))
        .is_some_and(|meta| meta.location_selection.is_some());
    if !selecting {
        return;
    }
    let pressed = cmd.buttons & !old_buttons;
    let authority = world
        .resource::<crate::step::StepRequest>()
        .reason
        .advances_authority_world();
    let picked = pressed & (buttons::LOCATION_SELECT | buttons::LOCATION_CANCEL);
    if authority
        && picked != 0
        && crate::t6::location_pick(
            world,
            client,
            cmd.selected_location,
            pressed & buttons::LOCATION_SELECT != 0,
        )
    {
        diag::info!(
            Sim,
            "bo2mp location: client {client} {} at {:?}",
            if picked & buttons::LOCATION_SELECT != 0 { "picked" } else { "backed out" },
            cmd.selected_location
        );
    }
    // The pick's own bits stay, so the next command of a held click (several
    // reach one server frame) is not a new pick: BO2's Lightning Strike takes
    // three picks, one a click.
    cmd.buttons &=
        buttons::CROUCH | buttons::PRONE | buttons::LOCATION_SELECT | buttons::LOCATION_CANCEL;
}

/// Key and action-slot presses the start-up scripts never bind; the step loop
/// still reports them.
pub(crate) fn player_commands(_: &mut World, _client: u32, _buttons: u32, _old_buttons: u32) {}

pub(crate) fn action_slot_command(_: &mut World, _client: u32, _slot: u8) {}

/// A restart request the start-up scripts never raise.
pub(crate) fn restart_level(_: &mut World, _tick: crate::Tick) {}
