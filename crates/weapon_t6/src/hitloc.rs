//! BO2's hit locations and the damage multiplier each one gets.

/// BO2 has 21 hit locations: one more than MW2 (torso_mid at 5), and "riotshield" last.
pub const HITLOC_COUNT: usize = 21;

/// G_GetHitLocationString's names, in BO2's order (also its models' bone classification bytes).
pub const HITLOC_NAMES: [&str; HITLOC_COUNT] = [
    "none",
    "helmet",
    "head",
    "neck",
    "torso_upper",
    "torso_mid",
    "torso_lower",
    "right_arm_upper",
    "left_arm_upper",
    "right_arm_lower",
    "left_arm_lower",
    "right_hand",
    "left_hand",
    "right_leg_upper",
    "left_leg_upper",
    "right_leg_lower",
    "left_leg_lower",
    "right_foot",
    "left_foot",
    "gun",
    "riotshield",
];

pub const BODY_ZONE_UNSET: u8 = 0;
pub const STRUCK_HELM: u8 = 1;
pub const STRUCK_SKULL: u8 = 2;
pub const HITLOC_GUN: u8 = 19;
pub const HITLOC_RIOTSHIELD: u8 = 20;

/// The global table before info/mp_lochit_dmgtable fills it: every slot 1.0, the gun 0.0.
pub const GLOBAL_START: [f32; HITLOC_COUNT] = {
    let mut t = [1.0f32; HITLOC_COUNT];
    t[HITLOC_GUN as usize] = 0.0;
    t
};

/// BO2 `weapType_t` values whose own table is used (bullet, grenade, projectile).
const OWN_TABLE_TYPES: [i32; 3] = [
    crate::facts::gun_family_code::BULLET as i32,
    crate::facts::gun_family_code::GRENADE as i32,
    crate::facts::gun_family_code::PROJECTILE as i32,
];

/// G_GetWeaponHitLocationMultiplier's choice, per slot: a bullet, grenade or projectile weapon
/// uses its own table (BO2 replaces, where MW2 multiplied it with the global one); anything
/// else uses the global table. `gun_family_code` is BO2's raw `weapType_t`, not a remapped number.
/// A weapon with no table of its own keeps the global one.
#[must_use]
pub fn location_table(
    gun_family_code: i32,
    global: [f32; HITLOC_COUNT],
    weapon: Option<[f32; HITLOC_COUNT]>,
) -> [f32; HITLOC_COUNT] {
    match weapon {
        Some(own) if OWN_TABLE_TYPES.contains(&gun_family_code) => own,
        _ => global,
    }
}

/// The multiplier at a hit location. BO2 asserts 0..=20; past that it reads slot 0 here.
#[must_use]
pub fn location_scale(table: &[f32; HITLOC_COUNT], hitloc: u8) -> f32 {
    table.get(usize::from(hitloc)).copied().unwrap_or(table[0])
}

/// BO2's headshot: the helmet or the head.
#[must_use]
pub const fn is_headshot(hitloc: u8) -> bool {
    hitloc == STRUCK_HELM || hitloc == STRUCK_SKULL
}

/// The name of a hit location ("none" past the end).
#[must_use]
pub fn hitloc_name(hitloc: u8) -> &'static str {
    HITLOC_NAMES.get(usize::from(hitloc)).copied().unwrap_or("none")
}

/// Damage times the multiplier, cut to a whole number, at least 1 (melee skips this).
#[must_use]
pub fn scaled_damage(damage: i32, factor: f32) -> i32 {
    ((damage as f32 * factor) as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torso_mid_shifts_the_rest_by_one() {
        assert_eq!(HITLOC_NAMES[5], "torso_mid");
        assert_eq!(HITLOC_NAMES[6], "torso_lower");
        assert_eq!(hitloc_name(HITLOC_GUN), "gun");
        assert_eq!(hitloc_name(HITLOC_RIOTSHIELD), "riotshield");
        assert_eq!(hitloc_name(21), "none");
    }

    #[test]
    fn global_starts_at_one_with_the_gun_at_zero() {
        assert_eq!(GLOBAL_START[2], 1.0);
        assert_eq!(GLOBAL_START[19], 0.0);
        assert_eq!(GLOBAL_START[20], 1.0);
    }

    #[test]
    fn weapon_table_replaces_the_global_one() {
        let mut global = GLOBAL_START;
        global[2] = 2.0;
        let mut own = [1.0f32; HITLOC_COUNT];
        own[2] = 1.5;
        assert_eq!(location_table(0, global, Some(own))[2], 1.5);
        assert_eq!(location_table(1, global, Some(own))[2], 1.5);
        assert_eq!(location_table(2, global, Some(own))[2], 1.5);
        // binoculars, gas, bomb, mine, melee, riot shield and a weapon with no table: global
        for raw in [3, 4, 5, 6, 7, 8] {
            assert_eq!(location_table(raw, global, Some(own))[2], 2.0, "raw type {raw}");
        }
        assert_eq!(location_table(0, global, None)[2], 2.0);
    }

    #[test]
    fn damage_truncates_with_a_floor_of_one() {
        assert_eq!(scaled_damage(40, 1.4), 56);
        assert_eq!(scaled_damage(33, 0.9), 29);
        assert_eq!(scaled_damage(10, 0.0), 1);
        assert!(is_headshot(1) && is_headshot(2) && !is_headshot(3));
    }
}
