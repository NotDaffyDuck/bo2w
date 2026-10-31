//! Whose eyes the screen is looking through.

use bevy::prelude::*;
use frame::EyeSubject;
use net::{ClientStep, HostViewClient, ShownWorldFrame};
use playerstate_t6::game::AvatarState;
use frame::{ShownStatePosted, LifeStatePosted};

/// The view for the local client's presented state. A state that is not a
/// live frame is a spectator seat, watching whoever the kill cam names (if
/// anyone); a live one is the player's own view.
pub(crate) fn subject_of(local: u32, seen: Option<&AvatarState>) -> EyeSubject {
    match seen {
        None => EyeSubject::None,
        Some(seen) if seen.is_live_frame() => EyeSubject::Own { client: local },
        Some(seen) => EyeSubject::ChairSlot {
            viewer: local,
            focus: u32::try_from(seen.death_replay_player).ok(),
        },
    }
}

fn announce_viewed_target(
    shown: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    mut subject: ResMut<EyeSubject>,
) {
    *subject = subject_of(local.0.0, shown.player(local.0));
}

pub(crate) fn install(host: &mut App) {
    const PRESENT: ClientStep = ClientStep::Present;
    host.init_resource::<EyeSubject>()
        .add_systems(bevy::app::Update, announce_viewed_target.in_set(ShownStatePosted))
        .configure_sets(
            bevy::app::Update,
            LifeStatePosted
                .in_set(PRESENT)
                .after(announce_viewed_target),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(time_slice: i32, death_replay_player: i32) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.time_slice = time_slice;
        state.death_replay_player = death_replay_player;
        state
    }

    #[test]
    fn nobody_presented_means_no_view() {
        assert_eq!(subject_of(3, None), EyeSubject::None);
    }

    #[test]
    fn a_live_frame_is_the_players_own_view() {
        assert_eq!(
            subject_of(3, Some(&state(0, 9))),
            EyeSubject::Own { client: 3 }
        );
    }

    #[test]
    fn a_frame_that_is_not_live_is_a_seat_watching_the_kill_cam_target() {
        assert_eq!(
            subject_of(3, Some(&state(50, 9))),
            EyeSubject::ChairSlot {
                viewer: 3,
                focus: Some(9)
            }
        );
    }

    #[test]
    fn a_seat_with_no_kill_cam_target_watches_nobody() {
        assert_eq!(
            subject_of(3, Some(&state(50, -1))),
            EyeSubject::ChairSlot {
                viewer: 3,
                focus: None
            }
        );
    }
}
