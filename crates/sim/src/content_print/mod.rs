//! Fingerprints of the content a match runs on: weapons, equipment, classes, the map's spawns
//! and collision, and the script models in it.
//!
//! The two ends of a connection (and a replay and the game that plays it back) must run the
//! same content, so each side hashes what it loaded and they compare the results.
//! [`content_fingerprint`] is one number for everything; [`content_part_fingerprints`] keeps four parts
//! apart so a mismatch can say which part differs.
//!
//! The byte layout of the hash input is in [`digest`] and the order each part feeds its rows is
//! in [`weapons`], [`map`] and [`models`]. Neither may change without a new
//! [`CONTENT_PRINT_REVISION`].

mod digest;
mod map;
mod models;
mod weapons;

#[cfg(test)]
mod tests;

use crate::spawn_pick::MatchStartSetup;
use crate::sim_parts::SolidBrush;
use crate::{EntityHitAbilities, GadgetLiveStats};
use digest::Fnv1a;
use weapon_t6::combat_facts::GunBattleProfile;

/// Version of the digest layout, hashed into every digest so two builds with different layouts
/// can never agree by accident.
pub const CONTENT_PRINT_REVISION: u64 = 17;

/// The class-creation catalog the match accepts: how many perks, then each perk's stats table
/// number and slot.
fn class_catalog(h: &mut Fnv1a, boot: &MatchStartSetup) {
    h.put_len(boot.class_perk_slots.len());
    for &(id, slot) in &boot.class_perk_slots {
        h.put(id);
        h.put(slot);
    }
}

/// Everything the digests cover, borrowed for the length of one call.
struct ContentInputs<'a> {
    combat_rows: &'a [GunBattleProfile],
    runnable: &'a [bool],
    transition_groups: &'a [u32],
    equipment: &'a [GadgetLiveStats],
    boot: &'a MatchStartSetup,
    world_brushes: &'a [SolidBrush],
    model_caps: &'a [EntityHitAbilities],
}

impl ContentInputs<'_> {
    fn feed_weapons(&self, h: &mut Fnv1a) {
        weapons::combat_rows(h, self.combat_rows);
        weapons::admission(h, self.runnable, self.transition_groups);
        weapons::equipment_rows(h, self.equipment);
    }

    fn feed_map(&self, h: &mut Fnv1a) {
        map::spawn_points(h, &self.boot.spawns);
        map::collision_solids(h, self.world_brushes);
    }
}

pub fn content_fingerprint(
    combat_rows: &[GunBattleProfile],
    runnable: &[bool],
    transition_groups: &[u32],
    equipment: &[GadgetLiveStats],
    boot: &MatchStartSetup,
    world_brushes: &[SolidBrush],
    model_caps: &[EntityHitAbilities],
) -> u64 {
    let inputs = ContentInputs { combat_rows, runnable, transition_groups, equipment, boot, world_brushes, model_caps };

    let mut hash_box = Fnv1a::new();
    hash_box.put(CONTENT_PRINT_REVISION);
    // Weapons, then classes, then the map, then script models: the order is part of the format.
    inputs.feed_weapons(&mut hash_box);
    class_catalog(&mut hash_box, inputs.boot);
    inputs.feed_map(&mut hash_box);
    models::script_models(&mut hash_box, inputs.model_caps);
    hash_box.finish()
}

/// [`content_fingerprint`] taken apart: one hash per kind of content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentPartDigests {
    /// Spawn points and collision brushes.
    pub map: u64,
    /// Script models: their hit boxes and linked brushes.
    pub meshes: u64,
    /// Combat rows, weapon admission and equipment.
    pub guns: u64,
    /// The class-creation catalog.
    pub kits: u64,
}

pub fn content_part_fingerprints(
    combat_rows: &[GunBattleProfile],
    runnable: &[bool],
    transition_groups: &[u32],
    equipment: &[GadgetLiveStats],
    boot: &MatchStartSetup,
    world_brushes: &[SolidBrush],
    model_caps: &[EntityHitAbilities],
) -> ContentPartDigests {
    let inputs = ContentInputs { combat_rows, runnable, transition_groups, equipment, boot, world_brushes, model_caps };

    // Every component starts from the scheme and its own letter, so a map hash can never
    // equal a weapons hash over the same bytes.
    let start = |letter: u8| {
        let mut h = Fnv1a::new();
        h.put(CONTENT_PRINT_REVISION);
        h.put(letter);
        h
    };

    let mut map_hash = start(b'M');
    inputs.feed_map(&mut map_hash);

    let mut model_hash = start(b'D');
    models::script_models(&mut model_hash, inputs.model_caps);

    let mut weapon_hash = start(b'W');
    inputs.feed_weapons(&mut weapon_hash);

    let mut class_hash = start(b'C');
    class_catalog(&mut class_hash, inputs.boot);

    let parts = [map_hash, model_hash, weapon_hash, class_hash].map(Fnv1a::finish);
    ContentPartDigests { map: parts[0], meshes: parts[1], guns: parts[2], kits: parts[3] }
}
