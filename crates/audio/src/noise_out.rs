//! Turns gameplay events (a stride finishing, a weapon firing) into sound messages.

use asset_core::AssetCategoryScope;
use bevy::prelude::*;
use movement_t6::sway_phase_looped;

use crate::cue_names::{FootfallPace, footfall_cue_names, gear_jingle_cue, pick_gunshot_cue};
use crate::alerts::{FootfallMsg, GunNoiseMsg};

/// Watches a player's bob cycle byte and reports each time it wraps, which is
/// when a foot comes down.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct StrideRhythmWatch {
    has_prior: bool,
    pub last: u8,
}

impl StrideRhythmWatch {
    /// True when `bob` has wrapped since the previous call. The first call
    /// only records the starting value.
    pub fn note_phase(&mut self, bob_phase: u8) -> bool {
        let before = std::mem::replace(&mut self.last, bob_phase);
        let primed = std::mem::replace(&mut self.has_prior, true);
        primed && sway_phase_looped(before, bob_phase)
    }
}

/// Queues one weapon-channel sound at an entity and/or a point.
fn send_weapon_sound(
    out: &mut MessageWriter<GunNoiseMsg>,
    alias: String,
    namespace: AssetCategoryScope,
    (source_entity_no, position_in_inches): (Option<u32>, Option<[f32; 3]>),
) {
    out.write(GunNoiseMsg { alias, cue_ns: namespace, source_entity_no, position_in_inches });
}

/// Sends the footstep and gear-rattle sounds when the bob cycle wraps.
/// Returns whether a step was emitted.
pub fn send_footfall_at_stride_end(
    tracker: &mut StrideRhythmWatch,
    sway_phase: u8,
    gait: FootfallPace,
    surf_bits: u32,
    is_own_player: bool,
    hushed: bool,
    position_in_inches: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
    footsteps: &mut MessageWriter<FootfallMsg>,
    weapon_sounds: &mut MessageWriter<GunNoiseMsg>,
) -> bool {
    if !tracker.note_phase(sway_phase) {
        return false;
    }
    let names = footfall_cue_names(gait, surf_bits, is_own_player, hushed);
    footsteps.write(FootfallMsg { alias: names.0, fallback: names.1, source_entity_no, position_in_inches });
    let rattle = gear_jingle_cue(gait, is_own_player).to_string();
    send_weapon_sound(weapon_sounds, rattle, AssetCategoryScope::T6, (source_entity_no, position_in_inches));
    true
}

/// Sends the shot sound for a weapon, if it has one for this view.
/// Returns whether a sound was sent.
pub fn send_gunshot_noise(
    first_person: bool,
    fire_cue: Option<&str>,
    trigger_sound_name: Option<&str>,
    position_in_inches: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
    namespace: AssetCategoryScope,
    weapon_sounds: &mut MessageWriter<GunNoiseMsg>,
) -> bool {
    let Some(shot) = pick_gunshot_cue(first_person, fire_cue, trigger_sound_name) else {
        return false;
    };
    send_weapon_sound(weapon_sounds, shot.to_owned(), namespace, (source_entity_no, position_in_inches));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_reading_only_primes_the_tracker() {
        let mut tracker = StrideRhythmWatch::default();
        assert!(!tracker.note_phase(200));
        assert_eq!(tracker.last, 200);
    }

    #[test]
    fn crossing_a_half_turn_mark_between_readings_is_a_footfall() {
        let mut tracker = StrideRhythmWatch::default();
        tracker.note_phase(0x30);
        assert!(tracker.note_phase(0x50));
        assert!(!tracker.note_phase(0x60));
        assert_eq!(tracker.last, 0x60);
    }
}
