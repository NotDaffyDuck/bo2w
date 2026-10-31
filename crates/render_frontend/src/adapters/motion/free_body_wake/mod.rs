//! Loose pieces of the map (crates, glass, fruit): what shoves them and what
//! breaks them. Gunfire, knife swipes and blasts push the ones in reach and
//! wound the breakable ones; a player running into a resting piece wakes it.
//! The physics step that then moves a woken piece lives in `render_anim`;
//! this module only decides who gets shoved, how hard, and what breaks.

mod force;
mod grid;
mod hits;
mod player_push;

pub use force::{DEBRIS_BULLET_FORCE, DEBRIS_EXPLODE_FORCE};

use asset_world::{MovableObjectKind, SelfContainedPhysicsPreset};

use crate::prepare::scene::world::LevelDebrisInstance;

/// The physics preset of a piece that can be shoved: it still stands, its
/// kind takes part in physics at all, and it has mass to move.
fn wakeable(piece: &LevelDebrisInstance) -> Option<&SelfContainedPhysicsPreset> {
    if piece.gone {
        return None;
    }
    if !asset_world::dyn_ent_props(piece.ty)?.is_simulated {
        return None;
    }
    piece.physics_template.as_ref().filter(|preset| preset.mass > 0.0)
}

/// Whether pieces of this kind can be broken by damage.
fn is_breakable(kind: MovableObjectKind) -> bool {
    asset_world::dyn_ent_props(kind).is_some_and(|props| props.can_be_wrecked)
}

pub fn register_debris_wake(app: &mut bevy::prelude::App) {
    use bevy::prelude::*;

    app.init_resource::<grid::WakeGrid>()
        .add_observer(hits::on_blast)
        .add_observer(hits::on_bullet)
        .add_observer(hits::on_knife)
        .add_systems(
            Update,
            grid::refresh_wake_grid.in_set(net::ClientStep::Receive),
        )
        .add_systems(
            Update,
            player_push::shove_resting_pieces
                .in_set(frame::JobQueueStage::Physics)
                .before(render_anim::occupancy::debris_phys::step_phys_first),
        );
}

/// Pieces to feed the tests in this module and its children.
#[cfg(test)]
mod sample {
    use asset_world::{MovableObjectKind, LevelModelAssetId, SelfContainedPhysicsPreset};
    use bevy::prelude::*;

    use crate::prepare::scene::world::LevelDebrisInstance;

    /// A standing piece of the given kind whose preset has the given mass and
    /// pushes back with the plain scales.
    pub(super) fn piece(kind: MovableObjectKind, mass: f32) -> LevelDebrisInstance {
        let dead = false;
        LevelDebrisInstance {
            index: 0,
            ty: kind,
            active_mesh: LevelModelAssetId("props/crate".to_string()),
            place: Transform::default(),
            illum_origin: [0.0; 3],
            physics_template: Some(SelfContainedPhysicsPreset {
                name: "crate".to_string(),
                template_category: 0,
                mass,
                bounce: 0.2,
                friction: 0.5,
                shot_push_gain: 1.0,
                blast_push_gain: 1.0,
                audio_entry_stem: String::new(),
                shard_scatter_ratio: 0.0,
                shard_rise_speed: 0.0,
                falls_back_to_tube: false,
                material_footstep_entry: false,
            }),
            hp: 100,
            destroy_effect: None,
            gone: dead,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sample::piece;
    use super::*;

    #[test]
    fn clutter_and_destructibles_can_be_woken() {
        assert!(wakeable(&piece(MovableObjectKind::Clutter, 2.0)).is_some());
        assert!(wakeable(&piece(MovableObjectKind::Destruct, 2.0)).is_some());
    }

    #[test]
    fn kinds_without_physics_stay_put() {
        assert!(wakeable(&piece(MovableObjectKind::Invalid, 2.0)).is_none());
    }

    #[test]
    fn a_broken_or_massless_piece_is_not_woken() {
        let mut broken = piece(MovableObjectKind::Clutter, 2.0);
        broken.gone = true;
        assert!(wakeable(&broken).is_none());
        assert!(wakeable(&piece(MovableObjectKind::Clutter, 0.0)).is_none());
        let mut bare = piece(MovableObjectKind::Clutter, 2.0);
        bare.physics_template = None;
        assert!(wakeable(&bare).is_none());
    }

    #[test]
    fn only_destructibles_break() {
        assert!(is_breakable(MovableObjectKind::Destruct));
        assert!(!is_breakable(MovableObjectKind::Clutter));
        assert!(!is_breakable(MovableObjectKind::Invalid));
    }
}
