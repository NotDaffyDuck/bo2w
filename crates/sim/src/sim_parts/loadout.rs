//! Player-state helpers: a fresh player, and putting a weapon in his hands.

use playerstate_t6::game::{AvatarState, secondary_state_bits};

/// Marker that script names of dual-wield weapons carry.
const AKIMBO_MARK: &str = "_akimbo";

/// A player state with every field zeroed.
const BLANK_STATE: AvatarState = AvatarState::ZERO;

/// Whether a weapon's script name is its dual-wield (akimbo) variant.
pub(crate) fn script_gun_wants_two_hands(script_label: &str) -> bool {
    script_label.contains(AKIMBO_MARK)
}

/// Puts `weapon` in the first free slot of the inventory (if he does not carry it) and notes whether it is held in
/// two hands. Weapon 0 is nothing.
pub(crate) fn pack_weapon(
    player: &mut AvatarState,
    gun: u32,
    akimbo: bool,
) {
    if gun == 0 {
        return;
    }
    let wanted = gun as i32;
    let missing = !player.weapons.contains(&wanted);
    if let Some(empty) = player.weapons.iter_mut().find(|held| **held == 0).filter(|_| missing) {
        *empty = wanted;
    }
    weapon_t6::ammo_table::latch_weapon_dual_wield(&player.weapons, &mut player.weapon_data, gun, akimbo);
}

/// Gives `weapon` and takes it up. When the inventory is full it replaces the first slot. Weapon 0 empties his
/// hands.
pub(crate) fn grant_dual_wield_weapon(
    player: &mut AvatarState,
    gun: u32,
    akimbo: bool,
) {
    if gun == 0 {
        (player.weapon, player.weapon_primary, player.previous_gun_hand) = (0, 0, 0);
        return;
    }
    pack_weapon(player, gun, akimbo);
    let wanted = gun as i32;
    if !player.weapons.contains(&wanted) {
        if let Some(first) = player.weapons.first_mut() {
            *first = wanted;
            weapon_t6::ammo_table::latch_weapon_dual_wield(&player.weapons, &mut player.weapon_data, gun, akimbo);
        }
    }
    (player.weapon, player.weapon_primary) = (gun, gun);
    player.previous_gun_hand = weapon_t6::ammo_table::num_hands_for_held(&player.weapons, &player.weapon_data, gun);
}

/// A player state with everything zero.
pub const fn empty_avatar() -> AvatarState {
    BLANK_STATE
}

/// A living player standing at `origin`, looking along `viewangles`, with the starting health and speed.
pub(crate) fn fresh_avatar_at(at: [f32; 3], looking: [f32; 3]) -> AvatarState {
    AvatarState {
        origin: at,
        look_angles: looking,
        speed: 190,
        health: 100,
        hit_point_ceiling: 100,
        run_speed_modifier: 1.0,
        eye_height_goal: 60,
        eye_height_now: 60.0,
        gravity: 800,
        standing_on: entity_t6::ENTITYNUM_WORLD,
        secondary_state_bits: secondary_state_bits::PLAYER,
        body_index: -1,
        ..empty_avatar()
    }
}
