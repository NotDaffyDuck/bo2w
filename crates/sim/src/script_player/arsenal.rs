//! What a player carries: handing weapons out and taking them back, the held weapon, and the lists scripts ask for.

use crate::frame::FrameWorld;
use crate::step::{arm_held_weapon, seed_ps_ammo_tables};
use crate::world::{ClientId, gsc_give_weapon_is_akimbo, inventory_add_weapon};
use weapon_t6::ammo_table::{ammo_table_key, clip_table_key};
use weapon_t6::combat_facts::{WeaponCombatFacts, spawn_clip_stock};

/// Bytes in one row of the player state's ammo table (key, then the counts) and of its clip table.
const AMMO_ROW: usize = 8;
const CLIP_ROW: usize = 12;

/// The weapon with this script name, `none` standing for the empty hands (index 0).
pub(crate) fn weapon_named(world: &FrameWorld, name: &str) -> Result<u32, String> {
    if name == "none" {
        return Ok(0);
    }
    match world.weapon_index_by_script_name(name) {
        Some(index) if index != 0 => Ok(index),
        _ => Err(format!("unknown weapon '{name}'")),
    }
}

/// The script name of a weapon index; index 0 is `none`.
pub(crate) fn weapon_name(world: &FrameWorld, weapon: u32) -> String {
    if weapon == 0 {
        String::from("none")
    } else {
        world.weapon_script_name(weapon).to_owned()
    }
}

/// Weapons need a spawned player to hold them.
pub(super) fn require_spawned(world: &FrameWorld, id: ClientId) -> Result<(), String> {
    match world.player(id) {
        Some(_) => Ok(()),
        None => Err(String::from("player has not spawned")),
    }
}

/// A melee-inventory weapon with no magazine (knife_mp): held, never fired.
fn is_held_knife(world: &FrameWorld, weapon: u32) -> bool {
    let core = world.gun_facts().get(weapon).core;
    weapon != 0 && core.inventory_type == weapon_t6::facts::inventory::MELEE && core.clip_size <= 0
}

/// The ammo and clip table keys a weapon's rows are filed under; guns that share a key share their rows.
pub(super) fn table_keys(facts: &WeaponCombatFacts, weapon: u32) -> (i32, i32) {
    (
        ammo_table_key(facts.ammo_index, weapon),
        clip_table_key(facts.clip_index, weapon),
    )
}

/// Gives a player a weapon (`akimbo` for both hands; a dual-wield weapon is always both guns). Handing over one he
/// already holds changes nothing. A new gun starts with its spawn ammo, and so does its alternate weapon (a launcher
/// attachment's grenades) unless the two share their ammo.
pub(crate) fn give_weapon(world: &mut FrameWorld, id: ClientId, weapon: u32, akimbo: bool) -> Result<(), String> {
    require_spawned(world, id)?;
    if world.weapon_is_melee_only(weapon) {
        return Ok(());
    }
    let Some(facts) = world.combat_facts_for(weapon) else {
        // Class scripts give the knife_mp every class carries. It has no gun row, but it takes a weapon slot of its own
        // and the gun brain takes the held one as his knife.
        if is_held_knife(world, weapon) {
            let ps = world.player_mut(id).expect("checked above");
            inventory_add_weapon(ps, weapon, false);
            if !ps.weapons.contains(&(weapon as i32)) {
                return Err(String::from("the player's inventory is full"));
            }
            return Ok(());
        }
        return Err(format!("weapon {} has no combat data", weapon_name(world, weapon)));
    };
    let akimbo = akimbo || gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon)) || facts.dual_wield_weapon != 0;
    let offhand = world.equipment_facts_for(weapon).filter(|eq| eq.is_offhand());
    let already_held = world
        .player(id)
        .is_some_and(|ps| ps.weapons.contains(&(weapon as i32)));
    let ps = world.player_mut(id).expect("checked above");
    inventory_add_weapon(ps, weapon, akimbo);
    if !ps.weapons.contains(&(weapon as i32)) {
        return Err(String::from("the player's inventory is full"));
    }
    if already_held {
        return Ok(());
    }
    let (clip, clip_alt, stock) = match &offhand {
        Some(eq) => (eq.spawn_clip_count(), 0, 0),
        None => spawn_clip_stock(&facts, i32::from(akimbo)),
    };
    seed_ps_ammo_tables(ps, weapon, &facts, clip, clip_alt, akimbo, stock);
    world.client_meta_mut(id).set_ammo(weapon, clip, stock);
    // G_InitializeAmmo also starts the gun's alternate weapon.
    let alt = facts.alternate_weapon;
    let has_alt = offhand.is_none() && facts.dual_wield_weapon == 0 && alt != 0 && alt != weapon;
    if has_alt && let Some(alt_facts) = world.combat_facts_for(alt) {
        if table_keys(&alt_facts, alt) != table_keys(&facts, weapon) {
            let (alt_clip, _, alt_stock) = spawn_clip_stock(&alt_facts, 0);
            let ps = world.player_mut(id).expect("checked above");
            seed_ps_ammo_tables(ps, alt, &alt_facts, alt_clip, 0, false, alt_stock);
            world.client_meta_mut(id).set_ammo(alt, alt_clip, alt_stock);
        }
    }
    Ok(())
}

/// Zeroes every row of a table whose first four bytes are `key`.
fn clear_rows(table: &mut [u8], row_len: usize, key: i32) {
    for row in table.chunks_exact_mut(row_len) {
        let row_key = i32::from_le_bytes([row[0], row[1], row[2], row[3]]);
        if row_key == key {
            row.fill(0);
        }
    }
}

/// Takes a weapon away. The gun's ammo rows go with it unless another gun he still holds shares them (the tables hold
/// only 15 rows each).
pub(crate) fn take_weapon(world: &mut FrameWorld, id: ClientId, weapon: u32) {
    let leaving = world.combat_facts_for(weapon).map(|f| table_keys(&f, weapon));
    let held_on: Vec<u32> = world
        .player(id)
        .map(|ps| ps.weapons.iter().map(|&w| w as u32).collect())
        .unwrap_or_default();
    let still_used: Vec<(i32, i32)> = held_on
        .into_iter()
        .filter(|&w| w != 0 && w != weapon)
        .filter_map(|w| world.combat_facts_for(w).map(|f| table_keys(&f, w)))
        .collect();
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    if let Some(slot) = ps.weapons.iter().position(|&w| w == weapon as i32) {
        ps.weapons[slot] = 0;
        ps.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    }
    if let Some((ammo_key, clip_key)) = leaving {
        if still_used.iter().all(|&(ammo, _)| ammo != ammo_key) {
            clear_rows(&mut ps.ammo[..], AMMO_ROW, ammo_key);
        }
        if still_used.iter().all(|&(_, clip)| clip != clip_key) {
            clear_rows(&mut ps.ammoclip[..], CLIP_ROW, clip_key);
        }
    }
    if ps.weapon == weapon {
        ps.weapon = 0;
    }
    if ps.weapon_primary == weapon {
        ps.weapon_primary = 0;
    }
    let meta = world.client_meta_mut(id);
    meta.ammo_by_weapon.retain(|row| row.0 != weapon);
    if meta.controls.switch_to == weapon {
        meta.controls.switch_to = 0;
    }
}

/// Empties the player's hands and weapon slots.
pub(crate) fn take_all_weapons(world: &mut FrameWorld, id: ClientId) {
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    ps.weapons = [0; 15];
    ps.weapon_data.fill(0);
    ps.ammo.fill(0);
    ps.ammoclip.fill(0);
    ps.weapon = 0;
    ps.weapon_primary = 0;
    ps.offhand_primary = 0;
    ps.offhand_secondary = 0;
    let meta = world.client_meta_mut(id);
    meta.clear_ammo_inventory();
    meta.controls.switch_to = 0;
}

/// Raises `weapon` as the held weapon at once, keeping every ammo count he had.
pub(crate) fn set_spawn_weapon(world: &mut FrameWorld, id: ClientId, weapon: u32) -> Result<(), String> {
    require_spawned(world, id)?;
    let facts = world
        .combat_facts_for(weapon)
        .unwrap_or_else(WeaponCombatFacts::none);
    let remembered = world.client_meta(id).map(|m| m.ammo_for(weapon));
    let ps = world.player_mut(id).expect("checked above");
    let (ammo, clips) = (ps.ammo, ps.ammoclip);
    ps.weapon = weapon;
    arm_held_weapon(ps, weapon, &facts);
    ps.ammo = ammo;
    ps.ammoclip = clips;
    let meta = world.client_meta_mut(id);
    if let Some((clip, stock)) = remembered {
        meta.set_ammo(weapon, clip, stock);
    }
    meta.mirror_held_ammo(weapon);
    meta.controls.switch_to = 0;
    Ok(())
}

/// Asks the player's client to switch to a weapon he holds; one he does not hold is ignored.
pub(crate) fn switch_to_weapon(world: &mut FrameWorld, id: ClientId, weapon: u32) {
    let holds = world
        .player(id)
        .is_some_and(|ps| ps.weapons.contains(&(weapon as i32)));
    if holds {
        world.client_meta_mut(id).controls.switch_to = weapon;
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
pub(crate) fn weapons(world: &FrameWorld, id: ClientId, list: WeaponList) -> Vec<u32> {
    let Some(ps) = world.player(id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for &slot in ps.weapons.iter().filter(|&&w| w > 0) {
        let weapon = slot as u32;
        let kind = match world.combat_facts_for(weapon) {
            Some(facts) => facts.inventory_type,
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
pub(crate) fn has_weapon(world: &FrameWorld, id: ClientId, weapon: u32) -> bool {
    weapon != 0
        && world
            .player(id)
            .is_some_and(|ps| ps.weapons.contains(&(weapon as i32)))
}

/// The held weapons with each gun's alternate mode after it (the mode is a weapon of its own right after its gun,
/// `t6_gun::alt_mode_held_with`).
pub(crate) fn weapons_with_alt_modes(world: &FrameWorld, id: ClientId) -> Vec<u32> {
    let held = weapons(world, id, WeaponList::All);
    if world.gun_facts().is_empty() {
        return held;
    }
    let mut out = Vec::with_capacity(held.len() * 2);
    for &weapon in &held {
        out.push(weapon);
        let alt = crate::t6_gun::alt_mode_held_with(world.gun_facts(), &held, weapon);
        if alt != 0 {
            out.push(alt);
        }
    }
    out
}

/// BO2's hasweapon: any held weapon, a gun's alternate mode included.
pub(crate) fn holds_weapon(world: &FrameWorld, id: ClientId, weapon: u32) -> bool {
    if has_weapon(world, id, weapon) {
        return true;
    }
    weapon != 0 && weapons_with_alt_modes(world, id).contains(&weapon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_state::ClientLifecycle;

    #[test]
    fn the_class_knife_takes_a_weapon_slot_of_its_own() {
        use weapon_t6::facts::{FactsTable, T6WeaponFacts, inventory};
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let mut knife = T6WeaponFacts::default();
        knife.core.inventory_type = inventory::MELEE;
        let mut facts = FactsTable::new();
        facts.set(3, "knife_mp", knife);
        let mut content = crate::SimContentBuilder::default();
        content.set_gun_facts(facts);
        world.install_content(content.finish());
        let id = ClientId(0);
        world.client_meta_mut(id).lifecycle = ClientLifecycle::Alive;
        world.ensure_player(id);
        // The class script gives knife_mp, which has no gun row: he holds it all the same.
        assert_eq!(give_weapon(&mut world, id, 3, false), Ok(()));
        assert!(world.player(id).unwrap().weapons.contains(&3));
        // It is not a gun: getweaponslistprimaries leaves it out (scripts switch to those).
        assert_eq!(weapons(&world, id, WeaponList::All), vec![3]);
        assert!(weapons(&world, id, WeaponList::Primaries).is_empty());
        // A weapon with neither a gun row nor a knife's facts is still refused.
        assert!(give_weapon(&mut world, id, 4, false).is_err());
    }

    #[test]
    fn weapons_need_a_spawned_player() {
        let mut sim = crate::carrier::SimWorld::default();
        let mut world = sim.frame();
        let id = ClientId(0);
        assert_eq!(
            give_weapon(&mut world, id, 3, false),
            Err(String::from("player has not spawned"))
        );
        assert!(set_spawn_weapon(&mut world, id, 3).is_err());
    }

    #[test]
    fn the_empty_name_and_index_are_none() {
        let mut sim = crate::carrier::SimWorld::default();
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
