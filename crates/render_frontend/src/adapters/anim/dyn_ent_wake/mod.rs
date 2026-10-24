//! Loose pieces of the map (crates, glass, fruit): what shoves them and what
//! breaks them. Gunfire, knife swipes and blasts push the ones in reach and
//! wound the breakable ones; a player running into a resting piece wakes it.
//! The physics step that then moves a woken piece lives in `render_anim`;
//! this module only decides who gets shoved, how hard, and what breaks.

mod force;
mod grid;
mod hits;
mod player_push;

pub use force::{DYNENT_BULLET_FORCE, DYNENT_EXPLODE_FORCE};

use asset_world::{DynEntType, OwnedPhysPreset};

use crate::prepare::scene::world::WorldDynEntInstance;

/// The physics preset of a piece that can be shoved: it still stands, its
/// kind takes part in physics at all, and it has mass to move.
fn wakeable(piece: &WorldDynEntInstance) -> Option<&OwnedPhysPreset> {
    if piece.dead {
        return None;
    }
    if !asset_world::dyn_ent_props(piece.ty)?.use_physics {
        return None;
    }
    piece.phys_preset.as_ref().filter(|preset| preset.mass > 0.0)
}

/// Whether pieces of this kind can be broken by damage.
fn is_breakable(kind: DynEntType) -> bool {
    asset_world::dyn_ent_props(kind).is_some_and(|props| props.destroyable)
}

pub fn register_dyn_ent_wake(app: &mut bevy::prelude::App) {
    use bevy::prelude::*;

    app.init_resource::<grid::WakeGrid>()
        .add_observer(hits::on_blast)
        .add_observer(hits::on_bullet)
        .add_observer(hits::on_knife)
        .add_systems(
            Update,
            grid::refresh_wake_grid.in_set(net::ClientSet::Receive),
        )
        .add_systems(
            Update,
            player_push::shove_resting_pieces
                .in_set(frame::WorkerCmdSet::Physics)
                .before(render_anim::occupancy::dyn_ent_phys::step_phys_world0),
        );
}

/// Pieces to feed the tests in this module and its children.
#[cfg(test)]
mod sample {
    use asset_world::{DynEntType, MapXModelAssetKey, OwnedPhysPreset};
    use bevy::prelude::*;

    use crate::prepare::scene::world::WorldDynEntInstance;

    /// A standing piece of the given kind whose preset has the given mass and
    /// pushes back with the plain scales.
    pub(super) fn piece(kind: DynEntType, mass: f32) -> WorldDynEntInstance {
        WorldDynEntInstance {
            index: 0,
            ty: kind,
            current_model: MapXModelAssetKey("props/crate".to_string()),
            transform: Transform::default(),
            lighting_origin: [0.0; 3],
            phys_preset: Some(OwnedPhysPreset {
                name: "crate".to_string(),
                preset_type: 0,
                mass,
                bounce: 0.2,
                friction: 0.5,
                bullet_force_scale: 1.0,
                explosive_force_scale: 1.0,
                snd_alias_prefix: String::new(),
                pieces_spread_fraction: 0.0,
                pieces_upward_velocity: 0.0,
                temp_default_to_cylinder: false,
                per_surface_snd_alias: false,
            }),
            health: 100,
            destroy_fx: None,
            dead: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sample::piece;
    use super::*;

    #[test]
    fn clutter_and_destructibles_can_be_woken() {
        assert!(wakeable(&piece(DynEntType::Clutter, 2.0)).is_some());
        assert!(wakeable(&piece(DynEntType::Destruct, 2.0)).is_some());
    }

    #[test]
    fn kinds_without_physics_stay_put() {
        assert!(wakeable(&piece(DynEntType::Invalid, 2.0)).is_none());
    }

    #[test]
    fn a_broken_or_massless_piece_is_not_woken() {
        let mut broken = piece(DynEntType::Clutter, 2.0);
        broken.dead = true;
        assert!(wakeable(&broken).is_none());
        assert!(wakeable(&piece(DynEntType::Clutter, 0.0)).is_none());
        let mut bare = piece(DynEntType::Clutter, 2.0);
        bare.phys_preset = None;
        assert!(wakeable(&bare).is_none());
    }

    #[test]
    fn only_destructibles_break() {
        assert!(is_breakable(DynEntType::Destruct));
        assert!(!is_breakable(DynEntType::Clutter));
        assert!(!is_breakable(DynEntType::Invalid));
    }
}
