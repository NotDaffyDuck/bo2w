use weapon_t6::combat_facts::GunBattleProfile;

/// The damage of one bullet fired from a weapon described by `facts`, hitting at `distance` units.
///
/// A weapon with a damage table reads it. Otherwise the damage slides in a straight line from `damage` at the
/// max-damage range down to `min_damage` at the min-damage range, and holds either value beyond. A weapon without a
/// minimum never falls off.
pub fn shot_damage_at_range(facts: &GunBattleProfile, distance: f32) -> i32 {
    if let Some(steps) = facts.damage_steps {
        return steps.damage_at(distance).max(0);
    }
    let near = facts.damage;
    let far = if facts.hurt_floor > 0 { facts.hurt_floor } else { near };
    let span = facts.minimum_damage_reach - facts.full_damage_reach;
    if near == far || span == 0.0 || distance < facts.full_damage_reach {
        return near.max(0);
    }
    if distance >= facts.minimum_damage_reach {
        return far.max(0);
    }
    let weight = ((distance - facts.full_damage_reach) / span).clamp(0.0, 1.0);
    let blended = near as f32 * (1.0 - weight) + far as f32 * weight;
    (blended as i32).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(damage: i32, floor: i32, near: f32, far: f32) -> GunBattleProfile {
        GunBattleProfile {
            damage,
            hurt_floor: floor,
            full_damage_reach: near,
            minimum_damage_reach: far,
            damage_steps: None,
            ..GunBattleProfile::default()
        }
    }

    #[test]
    fn damage_holds_inside_the_near_range_and_beyond_the_far_range() {
        let gun = facts(100, 40, 200.0, 600.0);
        assert_eq!(shot_damage_at_range(&gun, 0.0), 100);
        assert_eq!(shot_damage_at_range(&gun, 200.0), 100);
        assert_eq!(shot_damage_at_range(&gun, 600.0), 40);
        assert_eq!(shot_damage_at_range(&gun, 5000.0), 40);
    }

    #[test]
    fn damage_slides_in_a_straight_line_between_the_ranges() {
        let gun = facts(100, 40, 200.0, 600.0);
        assert_eq!(shot_damage_at_range(&gun, 400.0), 70);
    }

    #[test]
    fn a_weapon_without_a_minimum_never_falls_off() {
        let gun = facts(55, 0, 100.0, 300.0);
        assert_eq!(shot_damage_at_range(&gun, 10_000.0), 55);
    }

    #[test]
    fn a_degenerate_range_does_not_divide_by_zero() {
        let gun = facts(80, 20, 300.0, 300.0);
        assert_eq!(shot_damage_at_range(&gun, 900.0), 80);
    }
}
