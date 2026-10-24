//! Whose eyes the screen looks through, and so which bodies are "mine" when
//! a handler chooses between a first-person and a third-person effect.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::ViewSubject;
use net::{CEntity, LocalPresentClient, PlayerDrawGate, PresentedSnapshot};

/// The entity number the camera sits on: whoever a spectating seat is
/// watching, otherwise the local client itself.
pub(super) fn eyes_entity_num(view: &ViewSubject, local: u32) -> i32 {
    let number = match view {
        ViewSubject::Seat {
            focus: Some(focus), ..
        } => *focus,
        _ => local,
    };
    i32::try_from(number).unwrap_or(0)
}

fn draw_gate(view: &ViewSubject, local: u32, other_flags: u32) -> PlayerDrawGate {
    PlayerDrawGate {
        eyes_entity_num: eyes_entity_num(view, local),
        other_flags,
        rendering_third_person: false,
    }
}

#[derive(SystemParam)]
pub(super) struct Viewpoint<'w> {
    local: Res<'w, LocalPresentClient>,
    presented: Res<'w, PresentedSnapshot>,
    view: Res<'w, ViewSubject>,
}

impl Viewpoint<'_> {
    /// Whether the entity belongs to the client playing on this machine.
    pub(super) fn is_local(&self, identity: &CEntity) -> bool {
        identity.client() == Some(self.local.0)
    }

    /// Whether the entity is the body the camera is looking out of.
    pub(super) fn is_viewed_body(&self, identity: &CEntity) -> bool {
        let flags = self
            .presented
            .player(self.local.0)
            .map_or(0, |state| state.other_flags);
        draw_gate(&self.view, self.local.0.0, flags).is_player_view(identity.number())
    }

    /// Whether the player sits in the scorestreak vehicle with this entity
    /// number (the VTOL Warship's gun, the Dragonfire's): its weapons count as
    /// his own for flash and sound.
    pub(super) fn rides(&self, number: i32) -> bool {
        render_anim::occupancy::third_person::vehicle_view(&self.presented, self.local.0)
            .is_some_and(|vehicle| vehicle.number == number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_is_on_the_local_client_by_default() {
        assert_eq!(eyes_entity_num(&ViewSubject::None, 3), 3);
        assert_eq!(eyes_entity_num(&ViewSubject::Own { client: 3 }, 3), 3);
    }

    #[test]
    fn a_seat_with_nobody_in_focus_stays_on_the_local_client() {
        let seat = ViewSubject::Seat {
            viewer: 1,
            focus: None,
        };
        assert_eq!(eyes_entity_num(&seat, 1), 1);
    }

    #[test]
    fn a_seat_follows_the_player_it_watches() {
        let seat = ViewSubject::Seat {
            viewer: 1,
            focus: Some(7),
        };
        assert_eq!(eyes_entity_num(&seat, 1), 7);
    }

    #[test]
    fn a_number_too_big_for_the_engine_reads_as_zero() {
        assert_eq!(eyes_entity_num(&ViewSubject::None, u32::MAX), 0);
    }

    #[test]
    fn only_the_watched_body_counts_as_the_viewers() {
        let seat = ViewSubject::Seat {
            viewer: 1,
            focus: Some(7),
        };
        let gate = draw_gate(&seat, 1, PlayerDrawGate::SELF_ENTITY_MASK);
        assert!(gate.is_player_view(7));
        assert!(!gate.is_player_view(1));
        assert!(!gate.is_player_view(8));
    }

    #[test]
    fn without_the_self_flags_no_body_is_the_viewers() {
        let gate = draw_gate(&ViewSubject::Own { client: 2 }, 2, 0);
        assert!(!gate.is_player_view(2));
    }
}
