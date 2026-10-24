//! A flashbang going off: how strongly each live player in sight of it is blinded.

use super::{dot, length, minus, open_to_blast, reachable_clients};
use crate::frame::FrameWorld;
use crate::match_state::ClientLifecycle;
use crate::world::ClientId;

/// The distances a flash is full strength inside and fades out beyond: the smaller radius is held to at least 1 unit,
/// and the larger one is never smaller than it.
fn flash_radii(radius_min: f32, radius_max: f32) -> (f32, f32) {
    let inner = radius_min.max(1.0);
    (inner, if radius_max < inner { inner } else { radius_max })
}

/// How much of the flash reaches a player `distance` away: all of it inside `inner`, none at `outer`, and a straight
/// fade between. A flash with no fade zone is all or nothing.
fn distance_fade(distance: f32, inner: f32, outer: f32) -> f32 {
    if distance <= inner || outer <= inner {
        1.0
    } else {
        1.0 - (distance - inner) / (outer - inner)
    }
}

/// How much of the flash a player sees: all of it looking straight at it, none looking directly away. `to_blast` is the
/// way from his eye to the flash and `forward` the way he looks. A flash at his eye counts as straight ahead.
fn facing_fade(to_blast: [f32; 3], forward: [f32; 3]) -> f32 {
    let reach = length(to_blast);
    let cosine = if reach > 0.0 { dot(to_blast, forward) / reach } else { 1.0 };
    (cosine + 1.0) * 0.5
}

/// A flashbang at `origin` thrown by `attacker`. A map that scripts its own flashbangs handles it itself; otherwise
/// every live player the blast can see is flashed, by how far he is and how squarely he faces it.
pub(crate) fn apply_flashbang_blast(
    world: &mut FrameWorld,
    origin: [f32; 3],
    radius_max: f32,
    radius_min: f32,
    attacker: ClientId,
) {
    if crate::t6::flashbang::blast(world.ecs(), origin, radius_max, radius_min, attacker) {
        return;
    }
    let (inner, outer) = flash_radii(radius_min, radius_max);
    let mut flashed = Vec::new();
    for target in reachable_clients(world, origin, outer) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        if meta.lifecycle != ClientLifecycle::Alive {
            continue;
        }
        let Some(ps) = world.player(target) else {
            continue;
        };
        let distance = length(minus(ps.origin, origin));
        if distance > outer || open_to_blast(world, origin, target) <= 0.0 {
            continue;
        }
        let eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
        let (forward, _, _) = game_math::angle_vectors(ps.viewangles);
        flashed.push((target, distance_fade(distance, inner, outer), facing_fade(minus(origin, eye), forward)));
    }
    for (target, by_distance, by_angle) in flashed {
        crate::script::flashbang(world.ecs(), target, origin, by_distance, by_angle, attacker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inner_radius_is_at_least_one_and_the_outer_never_less_than_it() {
        assert_eq!(flash_radii(0.0, 10.0), (1.0, 10.0));
        assert_eq!(flash_radii(5.0, 20.0), (5.0, 20.0));
        assert_eq!(flash_radii(5.0, 3.0), (5.0, 5.0));
        assert_eq!(flash_radii(-4.0, 0.5), (1.0, 1.0));
    }

    #[test]
    fn the_flash_is_full_inside_the_inner_radius_and_gone_at_the_outer() {
        assert_eq!(distance_fade(0.0, 5.0, 15.0), 1.0);
        assert_eq!(distance_fade(5.0, 5.0, 15.0), 1.0);
        assert_eq!(distance_fade(15.0, 5.0, 15.0), 0.0);
    }

    #[test]
    fn the_flash_fades_in_a_straight_line_between_the_radii() {
        assert_eq!(distance_fade(10.0, 5.0, 15.0), 0.5);
        assert_eq!(distance_fade(7.5, 5.0, 15.0), 0.75);
    }

    #[test]
    fn a_flash_with_no_fade_zone_is_all_or_nothing_at_its_radius() {
        assert_eq!(distance_fade(3.0, 5.0, 5.0), 1.0);
        assert_eq!(distance_fade(5.0, 5.0, 5.0), 1.0);
    }

    #[test]
    fn looking_at_the_flash_is_full_and_looking_away_is_none() {
        let forward = [1.0, 0.0, 0.0];
        assert_eq!(facing_fade([10.0, 0.0, 0.0], forward), 1.0);
        assert_eq!(facing_fade([-10.0, 0.0, 0.0], forward), 0.0);
    }

    #[test]
    fn a_flash_off_to_the_side_is_half_and_one_at_the_eye_counts_as_ahead() {
        let forward = [1.0, 0.0, 0.0];
        assert_eq!(facing_fade([0.0, 10.0, 0.0], forward), 0.5);
        assert_eq!(facing_fade([0.0, 0.0, 7.0], forward), 0.5);
        assert_eq!(facing_fade([0.0; 3], forward), 1.0);
    }
}
