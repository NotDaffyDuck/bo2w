//! Client behaviour that follows from certain dispatched events.

use bevy::prelude::*;
use entity_t6::EventCode;
use sim::{ClientId, SimWorld};

use super::raised_events::{SoundCueAt, EntitySightsLowered};
use crate::client::drive::{ClientGuessState, WeaponSelect};
use crate::client::frame_clock::FrameClock;
use crate::client::local_input::PlayerControlSnapshot;
use crate::client::presentation::shown_frame::HostViewClient;

/// Whether an event's entity number is the local player's.
fn is_local_number(number: i32, local: &HostViewClient) -> bool {
    number == local.0.0 as i32
}

/// A reset-aim event on the local player takes him out of aiming down sights.
pub(super) fn set_ads_from_reset(
    reset: On<EntitySightsLowered>,
    local: Res<HostViewClient>,
    mut actions: ResMut<PlayerControlSnapshot>,
) {
    if is_local_number(reset.event.payload.number, &local) {
        input_t6::set_ads(&mut actions.client, false);
    }
}

/// Dry fire moves to the next gun, like the game's out-of-ammo event does
/// (`sim::out_of_ammo_pick`). A killcam never switches.
pub(super) fn switch_when_out_of_ammo(
    trigger: On<SoundCueAt>,
    local: Res<HostViewClient>,
    mut actions: ResMut<PlayerControlSnapshot>,
    selection: Option<ResMut<WeaponSelect>>,
    clock: Option<Res<FrameClock>>,
    subject: Option<Res<frame::EyeSubject>>,
    prediction: Option<Res<ClientGuessState>>,
) {
    let dispatched = &trigger.event;
    let dry_fire = dispatched.event == EventCode::NOAMMO;
    let watching_killcam = subject.is_some_and(|subject| subject.replaying_final_death());
    if !dry_fire || watching_killcam || !is_local_number(dispatched.payload.number, &local) {
        return;
    }
    let (Some(prediction), Some(fake_clock), Some(mut selection)) = (prediction, clock, selection) else {
        return;
    };
    let world = prediction.0.world();
    let off_hand = dispatched.payload.event_parm & sim::NOAMMO_PARM_LEFT != 0;
    let (held, remembered) = (selection.index, selection.previous_main_weapon);
    let next = sim::out_of_ammo_pick(
        world,
        local.0,
        held,
        remembered,
        &selection.last_mode,
        off_hand,
    );
    let Some(next) = next.filter(|&next| next != selection.index) else {
        return;
    };

    apply_choice(&mut selection, world, local.0, next, fake_clock.time());
    input_t6::set_ads(&mut actions.client, false);
}

/// Point the selection at `pick`, keeping what the cycle needs to find its way back.
fn apply_choice(select: &mut WeaponSelect, world: &SimWorld, viewer: ClientId, pick: u32, now: i32) {
    let from = select.index;
    if from != 0 && matches!(world.weapon_stats_entry(from), Some(row) if row.carry_slot_kind == 0) {
        select.previous_main_weapon = from;
    }
    select.index = pick;
    select.mapped_index = match world.weapon_stats_entry(pick) {
        Some(row) if row.carry_slot_kind == 3 => row.alternate_weapon,
        _ => pick,
    };
    select.time = now;
    if let Some(player) = world.player(viewer) {
        sim::remember_mode(world, &player.weapons, &mut select.last_mode, pick);
    }
}
