//! What the match's scripts hand the controls and the HUD each tick: the
//! weapon locks the engine keeps up to date, location picks (airstrike-style
//! streaks), the server info the client reads, and the start of the tick's
//! settling pass.

use bevy_ecs::prelude::{Resource, World};
use playerstate_t6::game::{InputCommand, buttons};

use super::runtime::Runtime;
use crate::step_view::StepWorldView;
use crate::{SeatNo, PlayerSessionStage, MatchBoard, WeaponLock};

/// Server-info dvars the client shows next to the ones the post-fx rows need.
const SERVER_INFO: [&str; 3] = ["ui_bomb_timer", "mapname", "g_gametype"];

/// Marks the match as one the scripts settle collision for at the start of a
/// tick (so whatever runs next already sees each body in its final pose).
#[derive(Resource, Clone, Default)]
pub(crate) struct Mechanics;

pub(crate) fn advance_mechanics(world: &mut World) {
    if !world
        .resource::<crate::frame_step::FrameTicket>()
        .reason
        .moves_server_world_forward()
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
        .filter_map(|&called| Some((called.to_owned(), runtime.dvars.get(called)?.clone())))
        .chain(
            runtime
                .dvars
                .iter()
                .filter(|(called, _)| crate::is_postfx_dvar(called))
                .map(|(called, value)| (called.clone(), value.clone())),
        )
        .collect();
    let objectives = MatchBoard {
        server_info: info,
        scripted_effects: runtime.program.is_some(),
        ..MatchBoard::default()
    };
    StepWorldView::from_world(world).objectives = objectives;
}

/// Keep each client's weapon lock honest: it holds only while the client lives
/// and still holds the weapon it was taken with, and its target follows what
/// the lock is aimed at.
pub(crate) fn sync_script_locks(world: &mut World) {
    let mut frame = StepWorldView::from_world(world);
    for id in frame.ordered_player_ids() {
        let Some(ps) = frame.avatar(id).copied() else {
            continue;
        };
        let Some(meta) = frame.client_meta(id) else {
            continue;
        };
        let lock = meta.weapon_lock;
        let valid = lock.gun == ps.weapon
            && lock.life_no == meta.life_sequence.0
            && meta.lifecycle == PlayerSessionStage::Alive;
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
pub(crate) fn select_location(world: &mut World, who: u32, cmd: &mut InputCommand, last_tick_buttons: u32) {
    let selecting = StepWorldView::from_world(world)
        .client_meta(SeatNo(who))
        .is_some_and(|meta| meta.location_selection.is_some());
    if !selecting {
        return;
    }
    let pressed = cmd.buttons & !last_tick_buttons;
    let on_host = world
        .resource::<crate::frame_step::FrameTicket>()
        .reason
        .moves_server_world_forward();
    let picked = pressed & (buttons::MAP_TARGET_PICK | buttons::LOCATION_CANCEL);
    if on_host
        && picked != 0
        && crate::zombie_rt::location_pick(
            world,
            who,
            cmd.chosen_map_spot,
            pressed & buttons::MAP_TARGET_PICK != 0,
        )
    {
        diag::info!(
            Sim,
            "bo2mp location: client {who} {} at {:?}",
            if picked & buttons::MAP_TARGET_PICK != 0 { "picked" } else { "backed out" },
            cmd.chosen_map_spot
        );
    }
    // The pick's own bits stay, so the next command of a held click (several
    // reach one server frame) is not a new pick: BO2's Lightning Strike takes
    // three picks, one a click.
    cmd.buttons &=
        buttons::DUCK_BUTTON_BIT | buttons::LYING_FLAT | buttons::MAP_TARGET_PICK | buttons::LOCATION_CANCEL;
}
