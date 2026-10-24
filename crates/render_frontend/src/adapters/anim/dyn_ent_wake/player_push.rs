//! A player who runs into a loose piece lying still wakes it: the piece is
//! shoved along the player's own velocity, scaled by its mass.

use bevy::prelude::*;
use net::ClientPredictionState;

use super::force::sphere_in_box;
use super::wakeable;
use crate::adapters::anim::dyn_ent::PieceModels;
use crate::adapters::anim::dyn_ent_phys::{DynEntPhysImpulse, DynEntPhysWorld};
use crate::prepare::scene::cull::DynEntModelEntity;
use crate::prepare::scene::world::WorldDynEntInstance;

/// Corners of the player's body relative to its feet.
const BODY_LOW: Vec3 = Vec3::new(-15.0, -15.0, 0.0);
const BODY_HIGH: Vec3 = Vec3::new(15.0, 15.0, 70.0);

/// A player moving slower than this does not wake what they brush against.
const BRUSH_SPEED: f32 = 12.0;

fn is_moving_fast_enough(velocity: Vec3) -> bool {
    velocity.length() >= BRUSH_SPEED
}

/// Whether a piece with the given bounding sphere is touched by the body of
/// a player standing at `feet`.
fn touches_body(feet: Vec3, center: Vec3, reach: f32) -> bool {
    sphere_in_box(feet + BODY_LOW, feet + BODY_HIGH, center, reach)
}

pub(super) fn shove_resting_pieces(
    prediction: Option<Res<ClientPredictionState>>,
    bodies: Res<DynEntPhysWorld>,
    map_models: Option<Res<asset_world::MapXModelSceneCatalog>>,
    t6_models: Option<Res<assets::PreparedFpvMeshes>>,
    pieces: Query<(Entity, &WorldDynEntInstance, &Transform), With<DynEntModelEntity>>,
    mut pushes: MessageWriter<DynEntPhysImpulse>,
) {
    let Some(player) = prediction.as_ref().and_then(|p| p.0.predicted_local()) else {
        return;
    };
    let velocity = Vec3::from_array(player.velocity);
    if !is_moving_fast_enough(velocity) {
        return;
    }
    let feet = Vec3::from_array(player.origin);
    let models = PieceModels {
        map: map_models.as_deref(),
        t6: t6_models.as_deref(),
    };
    for (entity, piece, at) in &pieces {
        if bodies.is_awake(entity) {
            continue;
        }
        let Some(preset) = wakeable(piece) else {
            continue;
        };
        let reach = models.radius(&piece.current_model).max(1.0);
        if touches_body(feet, at.translation, reach) {
            pushes.write(DynEntPhysImpulse {
                entity,
                impulse: velocity * preset.mass,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walker_does_not_wake_things() {
        assert!(!is_moving_fast_enough(Vec3::new(8.0, 8.0, 0.0)));
        assert!(!is_moving_fast_enough(Vec3::ZERO));
    }

    #[test]
    fn a_runner_wakes_them() {
        assert!(is_moving_fast_enough(Vec3::new(12.0, 0.0, 0.0)));
        assert!(is_moving_fast_enough(Vec3::new(0.0, -200.0, 30.0)));
    }

    #[test]
    fn a_piece_beside_the_legs_is_touched() {
        let feet = Vec3::new(100.0, 100.0, 0.0);
        // Four units clear of the body's side (x runs 85 to 115).
        let beside = Vec3::new(119.0, 100.0, 20.0);
        assert!(touches_body(feet, beside, 5.0));
        assert!(!touches_body(feet, beside, 3.0));
    }

    #[test]
    fn a_piece_above_the_head_is_not_touched() {
        let feet = Vec3::new(0.0, 0.0, 0.0);
        assert!(!touches_body(feet, Vec3::new(0.0, 0.0, 90.0), 10.0));
        assert!(touches_body(feet, Vec3::new(0.0, 0.0, 79.0), 10.0));
    }

    #[test]
    fn the_body_follows_the_feet() {
        let center = Vec3::new(5.0, 0.0, 30.0);
        assert!(touches_body(Vec3::ZERO, center, 1.0));
        assert!(!touches_body(Vec3::new(500.0, 0.0, 0.0), center, 1.0));
    }
}
