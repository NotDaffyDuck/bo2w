//! Turns gameplay events (a stride finishing, a weapon firing) into sound messages.

use asset_core::AssetNamespace;
use bevy::prelude::*;
use movement_t6::bob_cycle_wrapped;

use crate::{
    aliases::{StepGait, footstep_aliases, gear_rattle_alias, select_fire_alias},
    messages::{Footstep, WeaponSound},
};

/// Watches a player's bob cycle byte and reports each time it wraps, which is
/// when a foot comes down.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct BobCycleTracker {
    pub last: u8,
    primed: bool,
}

impl BobCycleTracker {
    /// True when `bob` has wrapped since the previous call. The first call
    /// only records the starting value.
    pub fn observe(&mut self, bob: u8) -> bool {
        let previous = std::mem::replace(&mut self.last, bob);
        if !std::mem::replace(&mut self.primed, true) {
            return false;
        }
        bob_cycle_wrapped(previous, bob)
    }
}

/// Sends the footstep and gear-rattle sounds when the bob cycle wraps.
/// Returns whether a step was emitted.
pub fn emit_footstep_on_bob_wrap(
    tracker: &mut BobCycleTracker,
    bob_cycle: u8,
    gait: StepGait,
    surface_flags: u32,
    local_player: bool,
    quieter: bool,
    origin_inches: Option<[f32; 3]>,
    snd_ent: Option<u32>,
    footsteps: &mut MessageWriter<Footstep>,
    weapon_sounds: &mut MessageWriter<WeaponSound>,
) -> bool {
    if !tracker.observe(bob_cycle) {
        return false;
    }

    let (alias, fallback) = footstep_aliases(gait, surface_flags, local_player, quieter);
    footsteps.write(Footstep {
        alias,
        fallback,
        origin_inches,
        snd_ent,
    });

    let rattle = gear_rattle_alias(gait, local_player);
    weapon_sounds.write(WeaponSound {
        namespace: AssetNamespace::T6,
        alias: rattle.to_owned(),
        origin_inches,
        snd_ent,
    });
    true
}

/// Sends the shot sound for a weapon, if it has one for this view.
/// Returns whether a sound was sent.
pub fn emit_weapon_fire(
    player_view: bool,
    fire: Option<&str>,
    fire_player: Option<&str>,
    origin_inches: Option<[f32; 3]>,
    snd_ent: Option<u32>,
    namespace: AssetNamespace,
    weapon_sounds: &mut MessageWriter<WeaponSound>,
) -> bool {
    match select_fire_alias(player_view, fire, fire_player) {
        Some(alias) => {
            weapon_sounds.write(WeaponSound {
                namespace,
                alias: alias.to_owned(),
                origin_inches,
                snd_ent,
            });
            true
        }
        None => false,
    }
}
