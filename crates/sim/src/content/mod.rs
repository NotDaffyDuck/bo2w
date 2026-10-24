//! Fingerprints of the content a match runs on: weapons, equipment, classes, the map's spawns
//! and collision, and the script models in it.
//!
//! The two ends of a connection (and a replay and the game that plays it back) must run the
//! same content, so each side hashes what it loaded and they compare the results.
//! [`content_digest`] is one number for everything; [`content_components`] keeps four parts
//! apart so a mismatch can say which part differs.
//!
//! The byte layout of the hash input is in [`digest`] and the order each part feeds its rows is
//! in [`weapons`], [`map`] and [`models`]. Neither may change without a new
//! [`CONTENT_DIGEST_SCHEME`].

mod digest;
mod map;
mod models;
mod weapons;

#[cfg(test)]
mod tests;

use crate::spawn::MatchBootstrap;
use crate::world::SimBrush;
use crate::{EntityCollisionCapabilities, EquipmentRuntimeFacts};
use digest::Fnv1a;
use weapon_t6::combat_facts::WeaponCombatFacts;

/// Version of the digest layout, hashed into every digest so two builds with different layouts
/// can never agree by accident.
pub const CONTENT_DIGEST_SCHEME: u64 = 16;

/// The class-creation catalog the match accepts: every perk name, then every deathstreak name.
fn class_catalog(h: &mut Fnv1a) {
    for name in crate::match_state::CLASS_CATALOG_PERKS {
        h.put_text(name);
    }
    for name in crate::match_state::CLASS_CATALOG_DEATHSTREAKS {
        h.put_text(name);
    }
}

/// Everything the digests cover, borrowed for the length of one call.
struct ContentInputs<'a> {
    combat: &'a [WeaponCombatFacts],
    runnable: &'a [bool],
    transition_groups: &'a [u32],
    equipment: &'a [EquipmentRuntimeFacts],
    bootstrap: &'a MatchBootstrap,
    clip_brushes: &'a [SimBrush],
    script_models: &'a [EntityCollisionCapabilities],
}

impl ContentInputs<'_> {
    fn feed_weapons(&self, h: &mut Fnv1a) {
        weapons::combat_rows(h, self.combat);
        weapons::admission(h, self.runnable, self.transition_groups);
        weapons::equipment_rows(h, self.equipment);
    }

    fn feed_map(&self, h: &mut Fnv1a) {
        map::spawn_points(h, &self.bootstrap.spawns);
        map::clip_brushes(h, self.clip_brushes);
    }
}

pub fn content_digest(
    combat: &[WeaponCombatFacts],
    runnable: &[bool],
    transition_groups: &[u32],
    equipment: &[EquipmentRuntimeFacts],
    bootstrap: &MatchBootstrap,
    clip_brushes: &[SimBrush],
    script_models: &[EntityCollisionCapabilities],
) -> u64 {
    let inputs = ContentInputs {
        combat,
        runnable,
        transition_groups,
        equipment,
        bootstrap,
        clip_brushes,
        script_models,
    };

    let mut h = Fnv1a::new();
    h.put(CONTENT_DIGEST_SCHEME);
    // Weapons, then classes, then the map, then script models: the order is part of the format.
    inputs.feed_weapons(&mut h);
    class_catalog(&mut h);
    inputs.feed_map(&mut h);
    models::script_models(&mut h, inputs.script_models);
    h.finish()
}

/// [`content_digest`] taken apart: one hash per kind of content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentComponents {
    /// Spawn points and collision brushes.
    pub map: u64,
    /// Script models: their hit boxes and linked brushes.
    pub models: u64,
    /// Combat rows, weapon admission and equipment.
    pub weapons: u64,
    /// The class-creation catalog.
    pub classes: u64,
}

pub fn content_components(
    combat: &[WeaponCombatFacts],
    runnable: &[bool],
    transition_groups: &[u32],
    equipment: &[EquipmentRuntimeFacts],
    bootstrap: &MatchBootstrap,
    clip_brushes: &[SimBrush],
    script_models: &[EntityCollisionCapabilities],
) -> ContentComponents {
    let inputs = ContentInputs {
        combat,
        runnable,
        transition_groups,
        equipment,
        bootstrap,
        clip_brushes,
        script_models,
    };

    // Every component starts from the scheme and its own letter, so a map hash can never
    // equal a weapons hash over the same bytes.
    let start = |letter: u8| {
        let mut h = Fnv1a::new();
        h.put(CONTENT_DIGEST_SCHEME);
        h.put(letter);
        h
    };

    let mut map = start(b'M');
    inputs.feed_map(&mut map);

    let mut models = start(b'D');
    models::script_models(&mut models, inputs.script_models);

    let mut weapons = start(b'W');
    inputs.feed_weapons(&mut weapons);

    let mut classes = start(b'C');
    class_catalog(&mut classes);

    ContentComponents {
        map: map.finish(),
        models: models.finish(),
        weapons: weapons.finish(),
        classes: classes.finish(),
    }
}
