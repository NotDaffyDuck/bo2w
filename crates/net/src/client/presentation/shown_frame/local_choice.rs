//! Which state stands for the local player on screen: the snapshot's, the predicted one, or the
//! one shown last frame.

use playerstate_t6::game::AvatarState;

use super::ShownWorldFrame;

/// Where the local player's shown state came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Archived,
    Predicted,
    Previous,
    Snapshot,
    Zero,
}

impl Source {
    fn name_word(self) -> &'static str {
        match self {
            Source::Archived => "archived",
            Source::Predicted => "predicted",
            Source::Previous => "previous",
            Source::Snapshot => "snapshot",
            Source::Zero => "zero",
        }
    }

    /// An archived (killcam) state is shown as it is, whatever was predicted. After that the
    /// order of preference is prediction, last frame's, the snapshot's, and last of all zero.
    fn choose(snapshot: Option<&AvatarState>, have_guess: bool, previous: bool) -> Source {
        let candidates = [
            (
                snapshot.is_some_and(|state| !state.is_live_frame()),
                Source::Archived,
            ),
            (have_guess, Source::Predicted),
            (previous, Source::Previous),
            (snapshot.is_some(), Source::Snapshot),
        ];
        candidates
            .into_iter()
            .find_map(|(usable, source)| usable.then_some(source))
            .unwrap_or(Source::Zero)
    }
}

impl ShownWorldFrame {
    pub fn display_self_participant_state(
        world_state_of_self: Option<&AvatarState>,
        predicted: Option<AvatarState>,
        previous_presented: Option<AvatarState>,
    ) -> AvatarState {
        let source = Source::choose(
            world_state_of_self,
            predicted.is_some(),
            previous_presented.is_some(),
        );
        let from_world_state = world_state_of_self.copied();
        let chosen = match source {
            Source::Predicted => predicted,
            Source::Previous => previous_presented,
            Source::Archived | Source::Snapshot => from_world_state,
            Source::Zero => None,
        };
        chosen.unwrap_or(AvatarState::ZERO)
    }

    pub fn display_self_pick(
        world_state_of_self: Option<&AvatarState>,
        predicted: Option<&AvatarState>,
        previous_presented: Option<&AvatarState>,
    ) -> &'static str {
        Source::choose(
            world_state_of_self,
            predicted.is_some(),
            previous_presented.is_some(),
        )
        .name_word()
    }

    /// The same choice when prediction is not running.
    pub fn display_self_participant_state_bare(
        world_state_of_self: Option<&AvatarState>,
    ) -> AvatarState {
        Self::display_self_participant_state(world_state_of_self, None, None)
    }

    pub fn presentation_local_choice_unarmed(world_state_of_self: Option<&AvatarState>) -> &'static str {
        Self::display_self_pick(world_state_of_self, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin[0] = x;
        state
    }

    #[test]
    fn prediction_wins_over_the_previous_frame_over_the_snapshot() {
        let snapshot = at(1.0);
        let predicted = at(2.0);
        let previous = at(3.0);
        let pick = |p: Option<AvatarState>, q: Option<AvatarState>| {
            ShownWorldFrame::display_self_participant_state(Some(&snapshot), p, q).origin[0]
        };
        assert_eq!(pick(Some(predicted), Some(previous)), 2.0);
        assert_eq!(pick(None, Some(previous)), 3.0);
        assert_eq!(pick(None, None), 1.0);
    }

    #[test]
    fn with_nothing_at_all_the_zero_state_stands_in() {
        let state = ShownWorldFrame::display_self_participant_state(None, None, None);
        assert_eq!(state.origin, AvatarState::ZERO.origin);
        assert_eq!(
            ShownWorldFrame::display_self_pick(None, None, None),
            "zero"
        );
        assert_eq!(
            ShownWorldFrame::presentation_local_choice_unarmed(None),
            "zero"
        );
    }

    #[test]
    fn the_choice_names_what_was_used() {
        let state = at(0.0);
        let choice = ShownWorldFrame::display_self_pick;
        assert_eq!(
            choice(Some(&state), Some(&state), Some(&state)),
            "predicted"
        );
        assert_eq!(choice(Some(&state), None, Some(&state)), "previous");
        assert_eq!(choice(Some(&state), None, None), "snapshot");
        assert_eq!(
            ShownWorldFrame::presentation_local_choice_unarmed(Some(&state)),
            "snapshot"
        );
    }
}
