//! What a player carries: handing weapons out and taking them back, the held weapon, and the lists scripts ask for.

use crate::step_view::StepWorldView;
use crate::frame_step::{arm_held_weapon, seed_ps_ammo_tables};
use crate::sim_parts::{SeatNo, script_gun_wants_two_hands, pack_weapon};
use playerstate_t6::game::AvatarState;
use weapon_t6::ammo_table::{ammo_table_key, clip_table_key};
use weapon_t6::combat_facts::{GunBattleProfile, spawn_clip_stock};

/// Bytes in one row of the player state's ammo table (key, then the counts) and of its clip table.
const AMMO_ROW: usize = 8;
const CLIP_ROW: usize = 12;

/// The weapon with this script name, `none` standing for the empty hands (index 0).
pub(crate) fn weapon_named(world: &StepWorldView, name: &str) -> Result<u32, String> {
    match (name, world.gun_for_script_label(name)) {
        ("none", _) => Ok(0),
        (_, Some(index)) if index != 0 => Ok(index),
        _ => Err(format!("unknown weapon '{name}'")),
    }
}

/// The script name of a weapon index; index 0 is `none`.
pub(crate) fn weapon_name(world: &StepWorldView, gun: u32) -> String {
    if gun == 0 {
        String::from("none")
    } else {
        world.weapon_script_label(gun).to_owned()
    }
}

/// Weapons need a spawned player to hold them.
pub(super) fn require_spawned(world: &StepWorldView, id: SeatNo) -> Result<(), String> {
    if world.avatar(id).is_none() {
        return Err(String::from("player has not spawned"));
    }
    Ok(())
}

/// Adds `weapon` to the carried slots, failing when no slot was free for it.
fn stow_weapon(body: &mut AvatarState, gun: u32, dual: bool) -> Result<(), String> {
    pack_weapon(body, gun, dual);
    if body.weapons.contains(&(gun as i32)) {
        Ok(())
    } else {
        Err(String::from("the player's inventory is full"))
    }
}

/// A melee-inventory weapon with no magazine (knife_mp): held, never fired.
fn is_held_knife(world: &StepWorldView, gun: u32) -> bool {
    let core = world.gun_facts().get(gun).core;
    gun != 0 && core.carry_slot_kind == weapon_t6::facts::inventory::CLOSE_COMBAT && core.magazine_capacity <= 0
}

/// The ammo and clip table keys a weapon's rows are filed under; guns that share a key share their rows.
pub(super) fn table_keys(facts: &GunBattleProfile, gun: u32) -> (i32, i32) {
    (
        ammo_table_key(facts.ammo_type_slot, gun),
        clip_table_key(facts.motion_clip_slot, gun),
    )
}

/// Gives a player a weapon (`akimbo` for both hands; a dual-wield weapon is always both guns). Handing over one he
/// already holds changes nothing. A new gun starts with its spawn ammo, and so does its alternate weapon (a launcher
/// attachment's grenades) unless the two share their ammo.
pub(crate) fn give_weapon(world: &mut StepWorldView, seat: SeatNo, gun: u32, akimbo: bool) -> Result<(), String> {
    require_spawned(world, seat)?;
    if world.weapon_is_melee_only(gun) {
        return Ok(());
    }
    let Some(facts) = world.damage_profile_of(gun) else {
        // Class scripts give the knife_mp every class carries. It has no gun row, but it takes a weapon slot of its own
        // and the gun brain takes the held one as his knife.
        if is_held_knife(world, gun) {
            let body = world.occupant_mut(seat).expect("checked above");
            return stow_weapon(body, gun, false);
        }
        return Err(format!("weapon {} has no combat data", weapon_name(world, gun)));
    };
    let akimbo = akimbo || script_gun_wants_two_hands(world.weapon_script_label(gun)) || facts.dual_wield_weapon != 0;
    let offhand = world.gadget_stats_lookup(gun).filter(|eq| eq.is_secondary_gear());
    let already_held = world
        .avatar(seat)
        .is_some_and(|pstate| pstate.weapons.contains(&(gun as i32)));
    let pstate = world.occupant_mut(seat).expect("checked above");
    stow_weapon(pstate, gun, akimbo)?;
    if already_held {
        return Ok(());
    }
    let (clip, alt_magazine, stock) = match &offhand {
        Some(eq) => (eq.starting_magazine_count(), 0, 0),
        None => spawn_clip_stock(&facts, i32::from(akimbo)),
    };
    seed_ps_ammo_tables(pstate, gun, &facts, clip, alt_magazine, akimbo, stock);
    let owner = world.client_meta_mut(seat);
    owner.store_ammo_counts(gun, clip, stock);
    // G_InitializeAmmo also starts the gun's alternate weapon.
    let alt = facts.alternate_weapon;
    let has_alt = offhand.is_none() && facts.dual_wield_weapon == 0 && alt != 0 && alt != gun;
    if has_alt && let Some(alt_facts) = world.damage_profile_of(alt) {
        if table_keys(&alt_facts, alt) != table_keys(&facts, gun) {
            let (alt_clip, _, alt_stock) = spawn_clip_stock(&alt_facts, 0);
            let pstate = world.occupant_mut(seat).expect("checked above");
            seed_ps_ammo_tables(pstate, alt, &alt_facts, alt_clip, 0, false, alt_stock);
            world.client_meta_mut(seat).store_ammo_counts(alt, alt_clip, alt_stock);
        }
    }
    Ok(())
}

/// Zeroes every row of a table whose first four bytes are `key`.
fn clear_rows(table: &mut [u8], row_len: usize, key: i32) {
    for row in table.chunks_exact_mut(row_len) {
        let row_key = i32::from_le_bytes([row[0], row[1], row[2], row[3]]);
        if row_key == key {
            row.iter_mut().for_each(|byte| *byte = 0);
        }
    }
}

/// Takes a weapon away. The gun's ammo rows go with it unless another gun he still holds shares them (the tables hold
/// only 15 rows each).
pub(crate) fn take_weapon(world: &mut StepWorldView, seat: SeatNo, gun: u32) {
    let leaving = world.damage_profile_of(gun).map(|f| table_keys(&f, gun));
    let held_on: Vec<u32> = world
        .avatar(seat)
        .map(|ps| ps.weapons.iter().map(|&w| w as u32).collect())
        .unwrap_or_default();
    let still_used: Vec<(i32, i32)> = held_on
        .into_iter()
        .filter(|&w| w != 0 && w != gun)
        .filter_map(|w| world.damage_profile_of(w).map(|f| table_keys(&f, w)))
        .collect();
    let Some(body) = world.occupant_mut(seat) else {
        return;
    };
    if let Some(slot) = body.weapons.iter().position(|&w| w == gun as i32) {
        body.weapons[slot] = 0;
        body.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    }
    if let Some((ammo_key, clip_key)) = leaving {
        if still_used.iter().all(|&(ammo, _)| ammo != ammo_key) {
            clear_rows(&mut body.ammo[..], AMMO_ROW, ammo_key);
        }
        if still_used.iter().all(|&(_, clip)| clip != clip_key) {
            clear_rows(&mut body.magazine_rows[..], CLIP_ROW, clip_key);
        }
    }
    if body.weapon == gun {
        body.weapon = 0;
    }
    if body.weapon_primary == gun {
        body.weapon_primary = 0;
    }
    let record = world.client_meta_mut(seat);
    record.rounds_per_gun.retain(|row| row.0 != gun);
    if record.controls.switch_to == gun {
        record.controls.switch_to = 0;
    }
}

/// Empties the player's hands and weapon slots.
pub(crate) fn take_all_weapons(world: &mut StepWorldView, id: SeatNo) {
    let Some(body) = world.occupant_mut(id) else {
        return;
    };
    body.weapons = [0; 15];
    body.weapon_data.fill(0);
    body.ammo.fill(0);
    body.magazine_rows.fill(0);
    body.weapon = 0;
    body.weapon_primary = 0;
    body.secondary_gear_main = 0;
    body.secondary_gear_alt = 0;
    let record = world.client_meta_mut(id);
    record.wipe_ammo_stock();
    record.controls.switch_to = 0;
}

/// Raises `weapon` as the held weapon at once, keeping every ammo count he had.
pub(crate) fn set_spawn_weapon(world: &mut StepWorldView, id: SeatNo, gun: u32) -> Result<(), String> {
    require_spawned(world, id)?;
    let facts = world.damage_profile_of(gun).unwrap_or_else(GunBattleProfile::none);
    let remembered = world.client_meta(id).map(|m| m.ammo_counts_of(gun));
    let body = world.occupant_mut(id).expect("checked above");
    let (ammo, clips) = (body.ammo, body.magazine_rows);
    body.weapon = gun;
    arm_held_weapon(body, gun, &facts);
    body.ammo = ammo;
    body.magazine_rows = clips;
    let record = world.client_meta_mut(id);
    if let Some((clip, stock)) = remembered {
        record.store_ammo_counts(gun, clip, stock);
    }
    record.copy_carried_ammo(gun);
    record.controls.switch_to = 0;
    Ok(())
}

/// Asks the player's client to switch to a weapon he holds; one he does not hold is ignored.
pub(crate) fn switch_to_weapon(world: &mut StepWorldView, seat: SeatNo, gun: u32) {
    let holds = world
        .avatar(seat)
        .is_some_and(|ps| ps.weapons.contains(&(gun as i32)));
    if holds {
        world.client_meta_mut(seat).controls.switch_to = gun;
    }
}

/// Which of a player's weapons a script asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WeaponList {
    All,
    Primaries,
    Offhands,
    Items,
    Exclusives,
}

impl WeaponList {
    /// Whether a weapon of inventory kind `kind` is in this list. The held knife (kind -1) is in `All` only.
    fn takes(self, kind: i32) -> bool {
        match self {
            WeaponList::All => true,
            WeaponList::Primaries => kind == 0,
            WeaponList::Offhands => kind == 1,
            WeaponList::Items => kind == 2,
            WeaponList::Exclusives => kind == 4,
        }
    }
}

/// The weapons a player holds that belong in `list`, in slot order.
pub(crate) fn weapons(world: &StepWorldView, id: SeatNo, list: WeaponList) -> Vec<u32> {
    let Some(body) = world.avatar(id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for weapon in body.weapons.iter().filter(|&&w| w > 0).map(|&w| w as u32) {
        let kind = match world.damage_profile_of(weapon) {
            Some(facts) => facts.carry_slot_kind,
            None if is_held_knife(world, weapon) => -1,
            None => 0,
        };
        if list.takes(kind) {
            out.push(weapon);
        }
    }
    out
}

/// Whether the player holds this weapon in a slot.
pub(crate) fn has_weapon(world: &StepWorldView, seat: SeatNo, gun: u32) -> bool {
    gun != 0
        && world
            .avatar(seat)
            .is_some_and(|ps| ps.weapons.contains(&(gun as i32)))
}

/// The held weapons with each gun's alternate mode after it (the mode is a weapon of its own right after its gun,
/// `t6_gun::alt_mode_held_with`).
pub(crate) fn weapons_with_alt_modes(world: &StepWorldView, id: SeatNo) -> Vec<u32> {
    let held = weapons(world, id, WeaponList::All);
    if world.gun_facts().is_empty() {
        return held;
    }
    let mut out = Vec::with_capacity(held.len() * 2);
    for &weapon in &held {
        out.push(weapon);
        let alt = crate::gun_brain::alt_mode_held_with(world.gun_facts(), &held, weapon);
        if alt != 0 {
            out.push(alt);
        }
    }
    out
}

/// BO2's hasweapon: any held weapon, a gun's alternate mode included.
pub(crate) fn holds_weapon(world: &StepWorldView, id: SeatNo, gun: u32) -> bool {
    if has_weapon(world, id, gun) {
        return true;
    }
    gun != 0 && weapons_with_alt_modes(world, id).contains(&gun)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_ledger::PlayerSessionStage;

    #[test]
    fn the_class_knife_takes_a_weapon_slot_of_its_own() {
        use weapon_t6::facts::{FactsTable, T6WeaponFacts, inventory};
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let mut knife = T6WeaponFacts::default();
        knife.core.carry_slot_kind = inventory::CLOSE_COMBAT;
        let mut facts = FactsTable::new();
        facts.set(3, "knife_mp", knife);
        let mut content = crate::SimContentBuilder::default();
        content.set_gun_facts(facts);
        world.install_content(content.finish());
        let id = SeatNo(0);
        world.client_meta_mut(id).lifecycle = PlayerSessionStage::Alive;
        world.seat_slot_for(id);
        // The class script gives knife_mp, which has no gun row: he holds it all the same.
        assert_eq!(give_weapon(&mut world, id, 3, false), Ok(()));
        assert!(world.avatar(id).unwrap().weapons.contains(&3));
        // It is not a gun: getweaponslistprimaries leaves it out (scripts switch to those).
        assert_eq!(weapons(&world, id, WeaponList::All), vec![3]);
        assert!(weapons(&world, id, WeaponList::Primaries).is_empty());
        // A weapon with neither a gun row nor a knife's facts is still refused.
        assert!(give_weapon(&mut world, id, 4, false).is_err());
    }

    #[test]
    fn weapons_need_a_spawned_player() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let mut world = sim.frame();
        let id = SeatNo(0);
        assert_eq!(
            give_weapon(&mut world, id, 3, false),
            Err(String::from("player has not spawned"))
        );
        assert!(set_spawn_weapon(&mut world, id, 3).is_err());
    }

    #[test]
    fn the_empty_name_and_index_are_none() {
        let mut sim = crate::sim_whole::WholeSim::default();
        let world = sim.frame();
        assert_eq!(weapon_named(&world, "none"), Ok(0));
        assert_eq!(weapon_name(&world, 0), "none");
        assert!(weapon_named(&world, "no_such_gun_mp").is_err());
    }

    #[test]
    fn a_table_row_goes_only_when_its_key_matches() {
        let mut rows = [0u8; 16];
        rows[..4].copy_from_slice(&7i32.to_le_bytes());
        rows[4] = 9;
        rows[8..12].copy_from_slice(&8i32.to_le_bytes());
        rows[12] = 5;
        clear_rows(&mut rows, 8, 7);
        assert_eq!(rows[..8], [0; 8]);
        assert_eq!(rows[12], 5);
    }

    #[test]
    fn the_lists_split_on_inventory_kind() {
        assert!(WeaponList::All.takes(-1));
        assert!(!WeaponList::Primaries.takes(-1));
        assert!(WeaponList::Primaries.takes(0));
        assert!(WeaponList::Offhands.takes(1));
        assert!(WeaponList::Items.takes(2));
        assert!(WeaponList::Exclusives.takes(4));
        assert!(!WeaponList::Exclusives.takes(3));
    }
}
