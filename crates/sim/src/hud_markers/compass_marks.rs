//! The compass: objective markers and the vehicles drawn on it.

use gamemode_t6::Team;

/// Where an objective stands. The numbers are the ones sent over the wire.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectiveState {
    #[default]
    Empty = 0,
    Active = 1,
    Invisible = 2,
    Done = 3,
    Current = 4,
    Failed = 5,
}

/// Every state in wire order, so the state with number `n` sits at index `n`.
const WIRE_ORDER: [ObjectiveState; 6] = {
    use ObjectiveState::*;
    [Empty, Active, Invisible, Done, Current, Failed]
};

impl ObjectiveState {
    /// The states in wire order.
    const IN_WIRE_ORDER: [Self; 6] = WIRE_ORDER;

    /// The state a script calls `name`; the names are lower case.
    pub fn from_script(name: &str) -> Option<Self> {
        match name {
            "empty" => Some(Self::Empty),
            "active" => Some(Self::Active),
            "invisible" => Some(Self::Invisible),
            "done" => Some(Self::Done),
            "current" => Some(Self::Current),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// The state with wire number `number`.
    pub fn from_u8(number: u8) -> Option<Self> {
        Self::IN_WIRE_ORDER.get(usize::from(number)).copied()
    }

    /// Whether a marker in this state is drawn on the compass.
    pub fn drawn(self) -> bool {
        match self {
            Self::Active | Self::Current => true,
            _ => false,
        }
    }
}

/// One objective marker on the compass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CompassMark {
    pub index: u8,
    pub state: ObjectiveState,
    /// The team the marker is for; free means everybody.
    pub team: Team,
    pub origin: [f32; 3],
    pub icon: String,
}

impl CompassMark {
    /// Whether a player on `viewer` sees the marker: it has to be in a drawn state and belong to everybody or to the
    /// viewer's team.
    pub fn shows_to(&self, viewer: Team) -> bool {
        let everyone_or_viewer = self.team == Team::Free || self.team == viewer;
        self.state.drawn() && everyone_or_viewer
    }
}

/// A vehicle shown on the compass: where it is and faces, whose it is, and the icons and sizes to draw it with.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CompassRide {
    pub owner: u32,
    pub team: i32,
    pub origin: [f32; 3],
    pub turn_deg: f32,
    pub icons: [String; 2],
    pub size: [i32; 2],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(state: ObjectiveState, team: Team) -> CompassMark {
        CompassMark { state, team, ..CompassMark::default() }
    }

    #[test]
    fn a_wire_number_gives_the_state_numbered_so() {
        for (wire, state) in ObjectiveState::IN_WIRE_ORDER.into_iter().enumerate() {
            assert_eq!(state as usize, wire);
            assert_eq!(ObjectiveState::from_u8(wire as u8), Some(state));
        }
    }

    #[test]
    fn a_wire_number_past_the_last_state_is_nothing() {
        assert_eq!(ObjectiveState::from_u8(6), None);
        assert_eq!(ObjectiveState::from_u8(255), None);
    }

    #[test]
    fn a_script_names_each_state_in_lower_case() {
        let names = ["empty", "active", "invisible", "done", "current", "failed"];
        for (name, state) in names.into_iter().zip(ObjectiveState::IN_WIRE_ORDER) {
            assert_eq!(ObjectiveState::from_script(name), Some(state));
        }
    }

    #[test]
    fn a_state_name_has_to_match_exactly() {
        assert_eq!(ObjectiveState::from_script("Active"), None);
        assert_eq!(ObjectiveState::from_script("activ"), None);
        assert_eq!(ObjectiveState::from_script(""), None);
    }

    #[test]
    fn only_active_and_current_markers_are_drawn() {
        let drawn: Vec<_> = ObjectiveState::IN_WIRE_ORDER.into_iter().filter(|s| s.drawn()).collect();
        assert_eq!(drawn, [ObjectiveState::Active, ObjectiveState::Current]);
    }

    #[test]
    fn a_marker_for_everyone_shows_to_every_team_while_drawn() {
        let free = marker(ObjectiveState::Active, Team::Free);
        assert!(free.shows_to(Team::Allies) && free.shows_to(Team::Axis) && free.shows_to(Team::Free));
        assert!(!marker(ObjectiveState::Done, Team::Free).shows_to(Team::Allies));
    }

    #[test]
    fn a_teams_marker_shows_only_to_that_team() {
        let allied = marker(ObjectiveState::Current, Team::Allies);
        assert!(allied.shows_to(Team::Allies));
        assert!(!allied.shows_to(Team::Axis));
    }
}
