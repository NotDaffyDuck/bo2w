//! Player-state helpers: a fresh player, and putting a weapon in his hands.

use playerstate_t6::game::{PlayerState, other_flags};

/// Whether a weapon's script name is its dual-wield (akimbo) variant.
pub(crate) fn gsc_give_weapon_is_akimbo(script_name: &str) -> bool {
    script_name.contains("_akimbo")
}

/// Puts `weapon` in the first free slot of the inventory (if he does not carry it) and notes whether it is held in
/// two hands. Weapon 0 is nothing.
pub(crate) fn inventory_add_weapon(ps: &mut PlayerState, weapon: u32, akimbo: bool) {
    if weapon == 0 {
        return;
    }
    let id = weapon as i32;
    if !ps.weapons.contains(&id) {
        if let Some(free) = ps.weapons.iter_mut().find(|slot| **slot == 0) {
            *free = id;
        }
    }
    weapon_t6::ammo_table::latch_weapon_dual_wield(&ps.weapons, &mut ps.weapon_data, weapon, akimbo);
}

/// Gives `weapon` and takes it up. When the inventory is full it replaces the first slot. Weapon 0 empties his
/// hands.
pub(crate) fn give_weapon_to_ps_akimbo(ps: &mut PlayerState, weapon: u32, akimbo: bool) {
    if weapon == 0 {
        ps.weapon = 0;
        ps.weapon_primary = 0;
        ps.last_weapon_hand = 0;
        return;
    }
    inventory_add_weapon(ps, weapon, akimbo);
    if !ps.weapons.contains(&(weapon as i32)) {
        if let Some(first) = ps.weapons.first_mut() {
            *first = weapon as i32;
            weapon_t6::ammo_table::latch_weapon_dual_wield(&ps.weapons, &mut ps.weapon_data, weapon, akimbo);
        }
    }
    ps.weapon = weapon;
    ps.weapon_primary = weapon;
    ps.last_weapon_hand = weapon_t6::ammo_table::num_hands_for_held(&ps.weapons, &ps.weapon_data, weapon);
}

/// A player state with everything zero.
pub fn blank_player_state() -> PlayerState {
    PlayerState::ZERO
}

/// A living player standing at `origin`, looking along `viewangles`, with the starting health and speed.
pub(crate) fn spawn_player_state(origin: [f32; 3], viewangles: [f32; 3]) -> PlayerState {
    PlayerState {
        origin,
        viewangles,
        speed: 190,
        health: 100,
        max_health: 100,
        move_speed_scale_multiplier: 1.0,
        view_height_target: 60,
        view_height_current: 60.0,
        gravity: 800,
        ground_entity_num: entity_t6::ENTITYNUM_WORLD,
        other_flags: other_flags::PLAYER,
        corpse_index: -1,
        ..blank_player_state()
    }
}
