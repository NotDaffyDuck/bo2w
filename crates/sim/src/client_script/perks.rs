//! Perks and stance: the perk bits a script turns on and off, and the stance the player's flags report.

use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use playerstate_t6::game::entity_flag_bits;

/// The entity flag a perk sets on top of (or instead of) its perk bit.
const PERK_ENTITY_FLAGS: &[(&str, u32)] = &[("specialty_localjammer", entity_flag_bits::RADAR_JAM)];

/// The entity flags that go with a perk name; zero for a perk with none.
fn perk_entity_flags(name: &str) -> u32 {
    PERK_ENTITY_FLAGS
        .iter()
        .find(|(perk, _)| *perk == name)
        .map_or(0, |&(_, flags)| flags)
}

/// Turns a perk on or off: its place in the perk record the scripts and weapon code read, its BO2 perk number in
/// the player state's perk words, and its entity flags.
pub(crate) fn set_perk(world: &mut StepWorldView, id: SeatNo, name: &str, on: bool) {
    let flags = perk_entity_flags(name);
    if let Some(number) = weapon_t6::env::perk::by_name(name) {
        let record = &mut world.client_meta_mut(id).t6_perks;
        let mask = 1u64 << number;
        if on {
            *record |= mask;
        } else {
            *record &= !mask;
        }
    }
    let Some(ps) = world.occupant_mut(id) else {
        return;
    };
    if let Some(number) = playerstate_t6::perks::by_name(name) {
        ps.set_perk(number, on);
    }
    if on {
        ps.e_flags |= flags;
    } else {
        ps.e_flags &= !flags;
    }
}

/// Takes every perk away.
pub(crate) fn clear_perks(world: &mut StepWorldView, id: SeatNo) {
    world.client_meta_mut(id).t6_perks = 0;
    if let Some(body) = world.occupant_mut(id) {
        body.perks = [0; 2];
        body.loadout_perks = [0; 8];
        body.e_flags &= !entity_flag_bits::RADAR_JAM;
    }
}

/// The stance word scripts compare against.
pub(crate) fn stance(world: &StepWorldView, id: SeatNo) -> &'static str {
    let flags = world.avatar(id).map_or(0, |ps| ps.e_flags);
    if flags & entity_flag_bits::LYING_FLAT != 0 {
        "prone"
    } else if flags & entity_flag_bits::CROUCH_BIT != 0 {
        "crouch"
    } else {
        "stand"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_perk_name_lands_on_its_bo2_perk_number() {
        let number = |name| playerstate_t6::perks::by_name(name).expect(name);
        assert_eq!(number("specialty_movefaster"), 40);
        assert_eq!(number("specialty_sprintrecovery"), 57);
        assert_eq!(number("specialty_unlimitedsprint"), 62);
    }

    #[test]
    fn the_radar_jammer_sets_a_flag_and_no_perk_bit() {
        assert_eq!(perk_entity_flags("specialty_localjammer"), entity_flag_bits::RADAR_JAM);
        assert!(playerstate_t6::perks::by_name("specialty_localjammer").is_none());
        assert_eq!(perk_entity_flags("specialty_no_such_perk"), 0);
    }

    #[test]
    fn every_perk_the_weapon_code_tests_has_the_same_number_in_both_tables() {
        for (name, number) in playerstate_t6::perks::PERK_NAMES {
            if let Some(weapon_number) = weapon_t6::env::perk::by_name(name) {
                assert_eq!(weapon_number, *number, "{name}");
            }
        }
    }
}
