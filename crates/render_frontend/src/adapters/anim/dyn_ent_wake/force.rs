//! The arithmetic behind a loose piece being knocked about: how far a blast
//! reaches, how hard it shoves, how much damage a breakable piece takes, and
//! the closest-approach tests a bullet, a knife or a walking player uses.
//! Nothing here touches the world, so every rule can be checked on its own.

use bevy::prelude::*;

/// Shove of a bullet or a knife swipe before the piece's own scale.
pub const DYNENT_BULLET_FORCE: f32 = 1000.0;

/// Shove of a blast at its centre before the piece's own scale.
pub const DYNENT_EXPLODE_FORCE: f32 = 12500.0;

/// Added to the upward part of a blast's heading so pieces lift as well as
/// slide.
const BLAST_LIFT: f32 = 0.5;

/// A blast weaker than this leaves a piece where it lies (unless the event
/// carries a heading of its own).
const BLAST_MIN_SHOVE: f32 = 40.0;

/// Inner and outer reach of a blast from the weapon's two radii. `None` when
/// the weapon has no blast; the inner radius never exceeds the outer one.
pub(super) fn blast_radii(radius: i32, radius_min: i32) -> Option<(f32, f32)> {
    let outer = radius.max(0) as f32;
    if outer <= 0.0 {
        return None;
    }
    Some(((radius_min.max(0) as f32).min(outer), outer))
}

/// How strongly a blast reaches a point `dist` from its centre: full inside
/// `inner`, fading in a straight line to nothing at `outer`, and `None` from
/// `outer` on.
pub(super) fn blast_reach(dist: f32, inner: f32, outer: f32) -> Option<f32> {
    if outer <= 0.0 || dist >= outer {
        return None;
    }
    if dist > inner && inner < outer {
        Some(((outer - dist) / (outer - inner)).max(0.0))
    } else {
        Some(1.0)
    }
}

/// Damage a blast does at `reach`: the weapon's outer damage plus the share
/// of the gap up to its inner damage that `reach` earns. Truncates like the
/// weapon tables do.
pub(super) fn blast_damage(inner_damage: i32, outer_damage: i32, reach: f32) -> i32 {
    (outer_damage as f32 + (inner_damage - outer_damage) as f32 * reach) as i32
}

/// Takes `damage` off `health` (never below zero). True only for the hit
/// that brings it to zero; a piece already at zero, or a harmless hit,
/// reports false.
pub(super) fn take_damage(health: &mut i32, damage: i32) -> bool {
    if damage <= 0 || *health <= 0 {
        return false;
    }
    *health = (*health - damage).max(0);
    *health == 0
}

/// The shove a blast gives a piece sitting `offset` from its centre.
/// `authored` is a heading the event carries; when it is non-zero it wins
/// and is applied as it stands. Otherwise the piece is pushed away from the
/// centre, tipped upward, and only if the blast is strong enough to move it.
pub(super) fn blast_shove(
    offset: Vec3,
    reach: f32,
    piece_scale: f32,
    authored: Vec3,
) -> Option<Vec3> {
    let strength = reach * piece_scale * DYNENT_EXPLODE_FORCE;
    let heading = if authored.length_squared() > 0.0 {
        authored
    } else {
        if strength < BLAST_MIN_SHOVE {
            return None;
        }
        let away = offset.try_normalize()?;
        Vec3::new(away.x, away.y, away.z + BLAST_LIFT).try_normalize()?
    };
    if strength <= 0.0 {
        return None;
    }
    Some(heading * strength)
}

/// The shove of a bullet or a knife swipe travelling along `heading`.
pub(super) fn strike_shove(heading: Vec3, piece_scale: f32) -> Option<Vec3> {
    Some(heading.try_normalize()? * DYNENT_BULLET_FORCE * piece_scale)
}

/// Fraction (0 to 1) along `a`..`b` of the point closest to `p`.
fn nearest_along(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let span = b - a;
    let len_sq = span.length_squared();
    if len_sq < 1e-12 {
        return 0.0;
    }
    ((p - a).dot(span) / len_sq).clamp(0.0, 1.0)
}

/// Where along the segment `a`..`b` it passes within `radius` of `center`:
/// the fraction (0 to 1) of its closest approach, or `None` if it stays
/// farther away than that.
pub(super) fn segment_graze(a: Vec3, b: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let t = nearest_along(a, b, center);
    let closest = a + (b - a) * t;
    (closest.distance_squared(center) <= radius * radius).then_some(t)
}

/// Whether the sphere reaches into the axis-aligned box.
pub(super) fn sphere_in_box(lo: Vec3, hi: Vec3, center: Vec3, radius: f32) -> bool {
    center.clamp(lo, hi).distance_squared(center) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weapon_without_a_blast_has_no_radii() {
        assert_eq!(blast_radii(0, 0), None);
        assert_eq!(blast_radii(-5, 10), None);
    }

    #[test]
    fn the_inner_radius_is_clamped_into_the_outer_one() {
        assert_eq!(blast_radii(300, 100), Some((100.0, 300.0)));
        assert_eq!(blast_radii(300, 900), Some((300.0, 300.0)));
        assert_eq!(blast_radii(300, -4), Some((0.0, 300.0)));
    }

    #[test]
    fn reach_is_full_inside_and_fades_to_nothing_outside() {
        assert_eq!(blast_reach(50.0, 100.0, 300.0), Some(1.0));
        assert_eq!(blast_reach(100.0, 100.0, 300.0), Some(1.0));
        assert_eq!(blast_reach(200.0, 100.0, 300.0), Some(0.5));
        assert_eq!(blast_reach(300.0, 100.0, 300.0), None);
        assert_eq!(blast_reach(999.0, 100.0, 300.0), None);
        assert_eq!(blast_reach(0.0, 100.0, 0.0), None);
    }

    #[test]
    fn a_blast_with_equal_radii_does_not_fade() {
        assert_eq!(blast_reach(299.0, 300.0, 300.0), Some(1.0));
    }

    #[test]
    fn damage_runs_from_the_outer_to_the_inner_value() {
        assert_eq!(blast_damage(200, 50, 1.0), 200);
        assert_eq!(blast_damage(200, 50, 0.0), 50);
        assert_eq!(blast_damage(200, 50, 0.5), 125);
        // Truncated, not rounded.
        assert_eq!(blast_damage(11, 0, 0.5), 5);
    }

    #[test]
    fn only_the_killing_hit_reports_a_break() {
        let mut health = 100;
        assert!(!take_damage(&mut health, 40));
        assert_eq!(health, 60);
        assert!(take_damage(&mut health, 80));
        assert_eq!(health, 0);
        // Already gone: nothing happens and nothing is reported again.
        assert!(!take_damage(&mut health, 10));
        assert_eq!(health, 0);
    }

    #[test]
    fn a_harmless_hit_changes_nothing() {
        let mut health = 30;
        assert!(!take_damage(&mut health, 0));
        assert!(!take_damage(&mut health, -9));
        assert_eq!(health, 30);
    }

    #[test]
    fn an_authored_heading_is_used_as_it_stands() {
        let heading = Vec3::new(0.0, 0.0, 2.0);
        // Even a blast too weak to move a piece on its own obeys it.
        let got = blast_shove(Vec3::X, 0.001, 0.001, heading).unwrap();
        let strength = 0.001 * 0.001 * DYNENT_EXPLODE_FORCE;
        assert!((got - heading * strength).length() < 1e-6);
    }

    #[test]
    fn a_blast_pushes_away_and_up() {
        let got = blast_shove(Vec3::new(10.0, 0.0, 0.0), 1.0, 1.0, Vec3::ZERO).unwrap();
        assert!(got.x > 0.0 && got.z > 0.0);
        assert!(got.y.abs() < 1e-3);
        // The heading is unit length, so the push has the full strength.
        assert!((got.length() - DYNENT_EXPLODE_FORCE).abs() < 1.0);
        // Straight out along x, the lift leaves a heading of (1, 0, 0.5).
        let lifted = BLAST_LIFT / (1.0 + BLAST_LIFT * BLAST_LIFT).sqrt();
        assert!((got.z / got.length() - lifted).abs() < 1e-4);
    }

    #[test]
    fn a_weak_blast_leaves_the_piece_alone() {
        // Strength 0.003 * 12500 = 37.5, under the 40 floor.
        assert_eq!(blast_shove(Vec3::X, 0.003, 1.0, Vec3::ZERO), None);
        assert!(blast_shove(Vec3::X, 0.004, 1.0, Vec3::ZERO).is_some());
    }

    #[test]
    fn a_piece_at_the_centre_has_no_direction_to_go() {
        assert_eq!(blast_shove(Vec3::ZERO, 1.0, 1.0, Vec3::ZERO), None);
    }

    #[test]
    fn a_zero_scale_piece_is_not_shoved() {
        assert_eq!(blast_shove(Vec3::X, 1.0, 0.0, Vec3::new(1.0, 0.0, 0.0)), None);
    }

    #[test]
    fn a_strike_is_a_fixed_force_along_the_path() {
        let got = strike_shove(Vec3::new(0.0, 5.0, 0.0), 2.0).unwrap();
        assert_eq!(got, Vec3::new(0.0, 2.0 * DYNENT_BULLET_FORCE, 0.0));
        assert_eq!(strike_shove(Vec3::ZERO, 1.0), None);
    }

    #[test]
    fn a_segment_grazes_a_sphere_it_passes_close_to() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(100.0, 0.0, 0.0);
        let t = segment_graze(a, b, Vec3::new(40.0, 5.0, 0.0), 6.0).unwrap();
        assert!((t - 0.4).abs() < 1e-6);
        assert_eq!(segment_graze(a, b, Vec3::new(40.0, 7.0, 0.0), 6.0), None);
    }

    #[test]
    fn a_segment_stops_at_its_ends() {
        let a = Vec3::ZERO;
        let b = Vec3::new(10.0, 0.0, 0.0);
        // Beyond the far end the closest point is the end itself.
        assert_eq!(segment_graze(a, b, Vec3::new(14.0, 0.0, 0.0), 5.0), Some(1.0));
        assert_eq!(segment_graze(a, b, Vec3::new(16.0, 0.0, 0.0), 5.0), None);
        assert_eq!(segment_graze(a, b, Vec3::new(-3.0, 0.0, 0.0), 5.0), Some(0.0));
    }

    #[test]
    fn a_zero_length_segment_is_a_point() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(segment_graze(p, p, p + Vec3::X, 2.0), Some(0.0));
        assert_eq!(segment_graze(p, p, p + Vec3::X * 3.0, 2.0), None);
    }

    #[test]
    fn a_sphere_touches_a_box_by_its_nearest_point() {
        let lo = Vec3::new(-15.0, -15.0, 0.0);
        let hi = Vec3::new(15.0, 15.0, 70.0);
        assert!(sphere_in_box(lo, hi, Vec3::new(20.0, 0.0, 30.0), 5.0));
        assert!(!sphere_in_box(lo, hi, Vec3::new(21.0, 0.0, 30.0), 5.0));
        // A centre inside the box always counts.
        assert!(sphere_in_box(lo, hi, Vec3::new(0.0, 0.0, 10.0), 0.0));
    }
}
