//! Bullet damage over distance, and the forward vector of a pair of view angles.

use weapon_t6::combat_facts::WeaponCombatFacts;

/// The damage one bullet of `facts` does at `dist` units: a table of steps when the weapon has one, otherwise a straight
/// slide from full damage (up to its max-damage range) to its minimum (from its min-damage range on).
pub fn bullet_damage_at_distance(facts: &WeaponCombatFacts, dist: f32) -> i32 {
    if let Some(steps) = facts.damage_steps {
        return steps.damage_at(dist).max(0);
    }
    let full = facts.damage;
    let floor = if facts.min_damage > 0 { facts.min_damage } else { full };
    let fall_off = facts.min_damage_range - facts.max_damage_range;
    let slides = full != floor && fall_off != 0.0 && dist >= facts.max_damage_range;
    let damage = if !slides {
        full
    } else if dist >= facts.min_damage_range {
        floor
    } else {
        let t = ((dist - facts.max_damage_range) / fall_off).clamp(0.0, 1.0);
        (full as f32 * (1.0 - t) + floor as f32 * t) as i32
    };
    damage.max(0)
}

/// The unit forward vector of `angles` (pitch, yaw, roll in degrees).
pub fn angles_to_forward(angles: [f32; 3]) -> [f32; 3] {
    let [pitch, yaw, _] = angles.map(f32::to_radians);
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    [cos_pitch * cos_yaw, cos_pitch * sin_yaw, -sin_pitch]
}
