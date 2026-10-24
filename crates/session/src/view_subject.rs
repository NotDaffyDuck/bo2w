//! Whose eyes the screen is looking through.

use bevy::prelude::*;
use frame::{LifeFrontPublished, PresentedPublished, ViewSubject};
use net::{ClientSet, LocalPresentClient, PresentedSnapshot};
use playerstate_t6::game::PlayerState;

/// The view for the local client's presented state. A state that is not a
/// live frame is a spectator seat, watching whoever the kill cam names (if
/// anyone); a live one is the player's own view.
pub(crate) fn subject_of(local: u32, state: Option<&PlayerState>) -> ViewSubject {
    match state {
        None => ViewSubject::None,
        Some(state) if state.is_live_frame() => ViewSubject::Own { client: local },
        Some(state) => ViewSubject::Seat {
            viewer: local,
            focus: u32::try_from(state.kill_cam_client_num).ok(),
        },
    }
}

fn publish_view_subject(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut subject: ResMut<ViewSubject>,
) {
    *subject = subject_of(local.0.0, presented.player(local.0));
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<ViewSubject>()
        .add_systems(Update, publish_view_subject.in_set(PresentedPublished))
        .configure_sets(
            Update,
            LifeFrontPublished
                .in_set(ClientSet::Present)
                .after(publish_view_subject),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(delta_time: i32, kill_cam_client_num: i32) -> PlayerState {
        let mut state = PlayerState::ZERO;
        state.delta_time = delta_time;
        state.kill_cam_client_num = kill_cam_client_num;
        state
    }

    #[test]
    fn nobody_presented_means_no_view() {
        assert_eq!(subject_of(3, None), ViewSubject::None);
    }

    #[test]
    fn a_live_frame_is_the_players_own_view() {
        assert_eq!(
            subject_of(3, Some(&state(0, 9))),
            ViewSubject::Own { client: 3 }
        );
    }

    #[test]
    fn a_frame_that_is_not_live_is_a_seat_watching_the_kill_cam_target() {
        assert_eq!(
            subject_of(3, Some(&state(50, 9))),
            ViewSubject::Seat {
                viewer: 3,
                focus: Some(9)
            }
        );
    }

    #[test]
    fn a_seat_with_no_kill_cam_target_watches_nobody() {
        assert_eq!(
            subject_of(3, Some(&state(50, -1))),
            ViewSubject::Seat {
                viewer: 3,
                focus: None
            }
        );
    }
}
