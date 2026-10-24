//! Perks and stance: the perk bits a script turns on and off, and the stance the player's flags report.

use crate::frame::FrameWorld;
use crate::world::ClientId;
use playerstate_t6::game::eflags;

/// The perk bit (in the first word of the player state's perks) each perk name sets.
const PERK_BITS: &[(&str, u32)] = &[
    ("specialty_fastreload", weapon_t6::combat_facts::PERK_FASTRELOAD),
    ("specialty_coldblooded", playerstate_t6::game::PERK_COLDBLOODED),
    ("specialty_lightweight", weapon_t6::view_bob::PERK_LIGHTWEIGHT_VIEW_BOB_BIT),
    ("specialty_scavenger", playerstate_t6::game::PERK_SCAVENGER),
    ("specialty_quieter", playerstate_t6::game::PERK_QUIETER),
    ("specialty_heartbreaker", playerstate_t6::game::PERK_HEARTBREAKER),
    ("specialty_marathon", playerstate_t6::perks::PERK_MARATHON),
    ("specialty_bulletaccuracy", weapon_t6::spread::PERK_BULLETACCURACY),
    ("specialty_pistoldeath", playerstate_t6::game::PERK_PISTOLDEATH),
    ("specialty_fallheight", playerstate_t6::perks::PERK_FALLHEIGHT),
    ("specialty_flakjacket", playerstate_t6::perks::PERK_FLAKJACKET),
    ("specialty_fastads", playerstate_t6::perks::PERK_FASTADS),
    ("specialty_fastmantle", movement_t6::bridge::PERK_FASTMANTLE),
    ("specialty_fastladderclimb", movement_t6::PERK_FASTLADDERCLIMB),
    ("specialty_longersprint", playerstate_t6::perks::PERK_LONGERSPRINT),
    ("specialty_bulletflinch", weapon_t6::view_bob::PERK_BULLETFLINCH),
    ("specialty_movefaster", playerstate_t6::perks::PERK_MOVEFASTER),
    ("specialty_stalker", playerstate_t6::perks::PERK_STALKER),
    ("specialty_sprintrecovery", weapon_t6::combat_facts::PERK_SPRINTRECOVERY),
    // The unlimited sprint a script grants is the marathon bit.
    ("specialty_unlimitedsprint", playerstate_t6::perks::PERK_MARATHON),
];

/// The entity flag a perk sets on top of (or instead of) its perk bit.
const PERK_ENTITY_FLAGS: &[(&str, u32)] = &[("specialty_localjammer", eflags::RADAR_JAM)];

/// The player-state perk bit and entity flags that go with a perk name; zero for a perk with neither.
fn perk_bits(name: &str) -> (u32, u32) {
    let lookup = |table: &[(&str, u32)]| {
        table
            .iter()
            .find(|(perk, _)| *perk == name)
            .map_or(0, |&(_, bits)| bits)
    };
    (lookup(PERK_BITS), lookup(PERK_ENTITY_FLAGS))
}

/// Turns a perk on or off: its place in the perk record the scripts and weapon code read, and its bits in the
/// player state.
pub(crate) fn set_perk(world: &mut FrameWorld, id: ClientId, name: &str, on: bool) {
    let (bit, flags) = perk_bits(name);
    if let Some(number) = weapon_t6::env::perk::by_name(name) {
        let record = &mut world.client_meta_mut(id).t6_perks;
        let mask = 1u64 << number;
        if on {
            *record |= mask;
        } else {
            *record &= !mask;
        }
    }
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    if on {
        ps.perks[0] |= bit;
        ps.e_flags |= flags;
    } else {
        ps.perks[0] &= !bit;
        ps.e_flags &= !flags;
    }
}

/// Takes every perk away.
pub(crate) fn clear_perks(world: &mut FrameWorld, id: ClientId) {
    world.client_meta_mut(id).t6_perks = 0;
    if let Some(ps) = world.player_mut(id) {
        ps.perks = [0; 2];
        ps.perk_slots = [0; 8];
        ps.e_flags &= !eflags::RADAR_JAM;
    }
}

/// The stance word scripts compare against.
pub(crate) fn stance(world: &FrameWorld, id: ClientId) -> &'static str {
    let flags = world.player(id).map_or(0, |ps| ps.e_flags);
    if flags & eflags::PRONE != 0 {
        "prone"
    } else if flags & eflags::DUCK != 0 {
        "crouch"
    } else {
        "stand"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newer_perks_have_bits_of_their_own() {
        assert_eq!(perk_bits("specialty_movefaster").0, 1 << 9);
        assert_eq!(perk_bits("specialty_sprintrecovery").0, 1 << 10);
        assert_eq!(perk_bits("specialty_unlimitedsprint").0, playerstate_t6::perks::PERK_MARATHON);
    }

    #[test]
    fn the_radar_jammer_sets_a_flag_and_no_perk_bit() {
        assert_eq!(perk_bits("specialty_localjammer"), (0, eflags::RADAR_JAM));
        assert_eq!(perk_bits("specialty_no_such_perk"), (0, 0));
    }

    #[test]
    fn a_perk_name_appears_once() {
        for (at, (name, _)) in PERK_BITS.iter().enumerate() {
            assert!(PERK_BITS[at + 1..].iter().all(|(other, _)| other != name), "{name}");
        }
    }
}
