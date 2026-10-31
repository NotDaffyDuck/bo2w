//! BO2's bullet damage: the six (damage, range) falloff points, what kind of shot a weapon
//! fires, and the identity location table. Facts: weapon-hits-pen-t6.md sections 2 and 4.

use crate::hitloc::HITLOC_COUNT;

/// The weapon types and classes as `asset_game`'s catalog writes them: it maps BO2's
/// `weapType_t` / `weapClass_t` onto the older games' numbering (`remap_t6_weap_type`,
/// `remap_t6_weap_class`), so these are those numbers, not [`crate::facts::weap_class`]'s.
/// TODO(fact): read BO2's numbers straight from the def and drop the remap.
pub mod catalog {
    /// Bullet, grenade and projectile keep BO2's numbers (weapon-hits-pen-t6.md 2.8).
    pub const GUN_FAMILY_HITSCAN: i32 = 0;
    pub const GUN_FAMILY_THROWN: i32 = 1;
    pub const GUN_FAMILY_ROCKET: i32 = 2;
    /// BO2 `WEAPTYPE_RIOTSHIELD` (8), which `remap_t6_weap_type` writes as 3.
    pub const WEAPTYPE_RIOTSHIELD: i32 = 3;

    /// BO2 `IMPACT_TYPE_BLADE`: the raw `impactType` of a ballistic or throwing knife.
    pub const IMPACT_TYPE_BLADE: i32 = 15;

    /// BO2 spread and pistol spread.
    pub const WEAPCLASS_SPREAD: i32 = 4;
    /// BO2 grenade.
    pub const GUN_GROUP_THROWABLE: i32 = 6;
}

use catalog::{GUN_GROUP_THROWABLE, GUN_FAMILY_HITSCAN, GUN_FAMILY_THROWN, GUN_FAMILY_ROCKET};

/// dvar bulletrange's default: how far a bullet goes (weapon-hits-pen-t6.md 3.13).
pub const BULLET_MAX_RANGE: f32 = 8192.0;

/// TODO(fact): the half-width of a rocket's spread plane, kept from the older code.
pub const ROCKET_SPREAD_PLANE: f32 = 16.0;

/// A location table that leaves damage as it is.
pub const LOCATION_DAMAGE_IDENTITY: [f32; HITLOC_COUNT] = [1.0; HITLOC_COUNT];

/// Every multiplier is a number and none is below zero.
#[must_use]
pub fn location_damage_is_valid(table: &[f32; HITLOC_COUNT]) -> bool {
    table.iter().all(|value| value.is_finite() && *value >= 0.0)
}

/// Black Ops II: a bullet's damage falls over up to six (damage, range)
/// points. A range under 0.001 is unused. Ranges are scaled at use by the
/// product of the attachments' damage range scales.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DamageSteps {
    pub damage: [i32; 6],
    pub range: [f32; 6],
    pub range_scale: f32,
}

impl DamageSteps {
    /// The file's points with BO2's load-time fill-ins: an unset first range
    /// is 999999, an unset last one 999999.1, a negative middle one unused.
    /// bo2mp: a middle range that went backwards (BO2 stops loading there) is
    /// dropped as unused; the loader logs it.
    pub fn from_file(damage: [i32; 6], mut range: [f32; 6]) -> Self {
        let backwards = Self::backwards_ranges(&range);
        for (r, backwards) in range.iter_mut().zip(backwards) {
            if backwards {
                *r = 0.0;
            }
        }
        if range[0] <= 0.0 {
            range[0] = 999_999.0;
        }
        if range[5] <= 0.0 {
            range[5] = 999_999.1;
        }
        for r in &mut range[1..5] {
            if *r < 0.0 {
                *r = 0.0;
            }
        }
        Self { damage, range, range_scale: 1.0 }
    }

    /// BO2's load check: the middle ranges (1 to 4) whose used value is under the last
    /// used one before it ("Damage range distances went backwards", fatal in BO2), marked true.
    pub fn backwards_ranges(range: &[f32; 6]) -> [bool; 6] {
        let mut out = [false; 6];
        let mut last = if range[0] > 0.0 { range[0] } else { 0.0 };
        for (i, &r) in range.iter().enumerate().take(5).skip(1) {
            if r <= 0.0 {
                continue;
            }
            if r < last {
                out[i] = true;
            } else {
                last = r;
            }
        }
        out
    }

    /// Damage at `dist` from where the bullet left: the first point's damage
    /// up to its range, the last point's past its range, and in between a
    /// straight line from the last used point at or under `dist` to the
    /// next used point past it, cut to a whole number.
    pub fn damage_at(&self, range: f32) -> i32 {
        let s = self.range_scale;
        let dd = range * range;
        let first = self.range[0] * s;
        if dd <= first * first {
            return self.damage[0];
        }
        let last = self.range[5] * s;
        if last * last < dd {
            return self.damage[5];
        }
        let mut below = 0;
        for next in 1..6 {
            let r = self.range[next] * s;
            if r < 0.001 {
                continue;
            }
            if r * r <= dd {
                below = next;
                continue;
            }
            let t = (range - self.range[below] * s) / ((self.range[next] - self.range[below]) * s);
            return (self.damage[below] as f32 * (1.0 - t) + self.damage[next] as f32 * t) as i32;
        }
        self.damage[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotOriginKind {
    Bullet,
    HandTossedExplosive,
    TubeLauncher,
    Missile,
}

/// What a weapon of this type and (catalog) class fires.
#[must_use]
pub const fn fire_weapon_kind(gun_family_code: i32, weap_class: i32) -> Option<ShotOriginKind> {
    match gun_family_code {
        GUN_FAMILY_HITSCAN => Some(ShotOriginKind::Bullet),
        GUN_FAMILY_THROWN => Some(ShotOriginKind::HandTossedExplosive),
        GUN_FAMILY_ROCKET => {
            if weap_class == GUN_GROUP_THROWABLE {
                Some(ShotOriginKind::TubeLauncher)
            } else {
                Some(ShotOriginKind::Missile)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::DamageSteps;

    #[test]
    fn damage_steps_follow_bo2_points() {
        // An SMG's points: 33 to 750, 22 from 751 to 1050, 18 past 1051 (two unused).
        let smg = DamageSteps::from_file([33, 22, 22, 0, 0, 18], [750.0, 751.0, 1050.0, 0.0, 0.0, 1051.0]);
        assert_eq!(smg.damage_at(500.0), 33);
        assert_eq!(smg.damage_at(750.0), 33);
        assert_eq!(smg.damage_at(750.5), 27);
        assert_eq!(smg.damage_at(900.0), 22);
        assert_eq!(smg.damage_at(1050.5), 20);
        assert_eq!(smg.damage_at(1300.0), 18);
        // One point only: the first range is the whole lerp up to the last.
        let one = DamageSteps::from_file([33, 0, 0, 0, 0, 24], [1500.0, 0.0, 0.0, 0.0, 0.0, 1501.0]);
        assert_eq!(one.damage_at(1500.25), 30);
        // Unset ranges: full damage everywhere.
        let flat = DamageSteps::from_file([50, 0, 0, 0, 0, 0], [0.0; 6]);
        assert_eq!(flat.damage_at(5000.0), 50);
        // An attachment that stretches range by 1.5 moves every point.
        let mut long = smg;
        long.range_scale = 1.5;
        assert_eq!(long.damage_at(1100.0), 33);
        assert_eq!(long.damage_at(1400.0), 22);
    }

    #[test]
    fn a_backwards_middle_range_is_dropped() {
        let r = [750.0, 1050.0, 900.0, 0.0, 1200.0, 1500.0];
        assert_eq!(DamageSteps::backwards_ranges(&r), [false, false, true, false, false, false]);
        assert_eq!(DamageSteps::from_file([33, 30, 28, 0, 22, 18], r).range[2], 0.0);
        assert_eq!(DamageSteps::backwards_ranges(&[750.0, 751.0, 1050.0, 0.0, 0.0, 1051.0]), [false; 6]);
    }
}
