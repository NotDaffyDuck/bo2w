//! Origins' upgraded staffs charging, what the charging player hears
//! (zm_tomb_amb.csc sndchargeshot, the engine's chargeshotweaponsoundnotify
//! callback): the staff's charge loop while he holds fire and its level's
//! sound each time the level changes, both his own (unplaced); the loop
//! stops 0.5 s after the last call (sndstoploopent). The rumble at each
//! level is zm_tomb.csc's, from the `player_staff_charge` field
//! (clientfields.rs). The loop is named in the client dvar
//! `bo2zm_staffloop` (empty = stopped), and the client fades it in over
//! 1.5 s and out over 0.1 s as playloopsound/stoploopsound do (audio
//! staffloop.rs).
//!
//! Not measured: how often the engine calls back. Ours calls every tick
//! while the staff charges (the 0.5 s stop is a debounce, so repeated calls
//! read as BO2's).

use std::collections::HashMap;

use bevy_ecs::prelude::{Resource, World};

use super::Zm;
use crate::world::ClientId;

/// sndstoploopent's `wait 0.5`.
const STOP_AFTER_MS: i64 = 500;

#[derive(Default)]
struct Charge {
    /// `self.sndlastcharge`: kept after the loop stops, as the script does,
    /// so a charge to the same level as the last one plays no new sound.
    last: Option<i32>,
    /// The loop playing, and when it stops unless called again.
    playing: Option<(String, i64)>,
}

#[derive(Resource, Default)]
struct ChargeSounds(HashMap<u32, Charge>);

/// sndchargeshot's alias for the staff: the fire staff's unless another.
fn prefix(weapon: &str) -> &'static str {
    match weapon {
        "staff_water_upgraded_zm" => "wpn_waterstaff_charge_",
        "staff_lightning_upgraded_zm" => "wpn_lightningstaff_charge_",
        "staff_air_upgraded_zm" => "wpn_airstaff_charge_",
        _ => "wpn_firestaff_charge_",
    }
}

fn local(world: &mut World, n: u32, alias: &str) {
    if std::env::var_os("IW4L_T6_SNDLOG").is_some() {
        diag::info!(Sim, "bo2zm t6 staff charge: client {n} plays {alias}");
    }
    let mut f = super::frame(world);
    let alias_index = f.sound_alias_index(&format!("t6:{alias}"));
    f.push_local_sound(crate::PendingLocalSound {
        recipient: ClientId(n),
        stop: false,
        alias_index,
    });
}

/// The loop the client plays and fades ("" = stop it).
fn set_loop(world: &mut World, n: u32, alias: &str) {
    if std::env::var_os("IW4L_T6_SNDLOG").is_some() {
        diag::info!(Sim, "bo2zm t6 staff charge: client {n} loop `{alias}`");
    }
    super::set_client_dvar(world, n, "bo2zm_staffloop", alias);
}

/// sndchargeshot: the loop on a new level (kept if it is already this
/// staff's), the level's sound, and the stop pushed 0.5 s on.
fn call(world: &mut World, n: u32, s: &mut Charge, weapon: &str, level: i32, now: i64) {
    if let Some((_, until)) = &mut s.playing {
        *until = now + STOP_AFTER_MS;
    }
    if s.last == Some(level) {
        return;
    }
    let p = prefix(weapon);
    let alias = format!("{p}loop");
    if s.playing.as_ref().is_none_or(|(old, _)| *old != alias) {
        set_loop(world, n, &alias);
    }
    s.playing = Some((alias, now + STOP_AFTER_MS));
    local(world, n, &format!("{p}{level}"));
    s.last = Some(level);
}

pub(super) fn advance(world: &mut World, now: i64) {
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut all = world.remove_resource::<ChargeSounds>().unwrap_or_default();
    for n in players {
        let s = all.0.entry(n).or_default();
        if let Some((weapon, level)) = super::charge_shot::charging(world, ClientId(n)) {
            let name = super::weapon_text(world, weapon);
            if name.starts_with("staff_") {
                call(world, n, s, &name, level, now);
                continue;
            }
        }
        // sndstoploopent: 0.5 s after the last call.
        if s.playing.as_ref().is_some_and(|(_, until)| now >= *until) {
            s.playing = None;
            set_loop(world, n, "");
        }
    }
    world.insert_resource(all);
}
