//! What the map's scripts show on the HUD and the scene: compass markers, vehicles, scores and the other match
//! information that goes to every client.

use gamemode_t6::Team;

/// Everything of the objective kind the match publishes to its clients.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ObjectiveMatch {
    /// Team scores indexed by team number: free, allies, axis.
    pub scores: [i32; 3],
    /// Named pieces of server information, looked up without regard to case.
    pub server_info: Vec<(String, String)>,
    /// When the game ends, in match milliseconds.
    pub game_end_time: i32,
    pub compass: Vec<CompassObjective>,
    pub vehicles: Vec<CompassVehicle>,
    /// Whether the scripts have taken over the map's own effects.
    pub scripted_effects: bool,
    pub effects: Vec<ScriptEffect>,
    pub fog: Option<crate::ScriptFog>,
    pub earthquakes: Vec<crate::ScriptEarthquake>,
    pub naked_vision: Option<crate::VisionChange>,
    pub thermal_vision: Option<crate::VisionChange>,
    pub night_vision: Option<crate::VisionChange>,
    pub missile_vision: Option<crate::VisionChange>,
    pub pain_vision: Option<crate::VisionChange>,
}

impl ObjectiveMatch {
    /// The server information called `name` (upper or lower case), if the scripts set it.
    pub fn server_info(&self, name: &str) -> Option<&str> {
        self.server_info
            .iter()
            .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()))
    }
}

/// An effect a script placed in the world: which one, where and which way it faces, and when and how often it plays.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScriptEffect {
    pub id: u32,
    /// Which effect plays.
    pub effect: u8,
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    /// When it starts; none until the scripts trigger it.
    pub start_ms: Option<i32>,
    /// Milliseconds between plays.
    pub repeat_ms: i32,
    /// Beyond this distance from the viewer it is not drawn.
    pub cull_distance: f32,
}

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

impl ObjectiveState {
    /// Every state, in wire order: the state with number `n` is `ALL[n]`.
    const ALL: [Self; 6] = [Self::Empty, Self::Active, Self::Invisible, Self::Done, Self::Current, Self::Failed];

    /// The state a script calls `name`; the names are lower case.
    pub fn from_script(name: &str) -> Option<Self> {
        Some(match name {
            "empty" => Self::Empty,
            "active" => Self::Active,
            "invisible" => Self::Invisible,
            "done" => Self::Done,
            "current" => Self::Current,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    /// The state with wire number `raw`.
    pub fn from_u8(raw: u8) -> Option<Self> {
        Self::ALL.get(usize::from(raw)).copied()
    }

    /// Whether a marker in this state is drawn on the compass.
    pub fn drawn(self) -> bool {
        matches!(self, Self::Active | Self::Current)
    }
}

/// One objective marker on the compass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CompassObjective {
    pub index: u8,
    pub state: ObjectiveState,
    /// The team the marker is for; free means everybody.
    pub team: Team,
    pub origin: [f32; 3],
    pub icon: String,
}

impl CompassObjective {
    /// Whether a player on `team` sees the marker: it must be a drawn state and belong to everybody or to his team.
    pub fn shows_to(&self, team: Team) -> bool {
        let belongs = self.team == Team::Free || self.team == team;
        self.state.drawn() && belongs
    }
}

/// A vehicle shown on the compass: where it is and faces, whose it is, and the icons and sizes to draw it with.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CompassVehicle {
    pub owner: u32,
    pub team: i32,
    pub origin: [f32; 3],
    pub yaw: f32,
    pub icons: [String; 2],
    pub size: [i32; 2],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(state: ObjectiveState, team: Team) -> CompassObjective {
        CompassObjective { state, team, ..CompassObjective::default() }
    }

    #[test]
    fn a_wire_number_gives_the_state_numbered_so() {
        for (number, state) in ObjectiveState::ALL.into_iter().enumerate() {
            assert_eq!(state as u8 as usize, number);
            assert_eq!(ObjectiveState::from_u8(number as u8), Some(state));
        }
    }

    #[test]
    fn a_wire_number_past_the_last_state_is_nothing() {
        assert_eq!(ObjectiveState::from_u8(6), None);
        assert_eq!(ObjectiveState::from_u8(255), None);
    }

    #[test]
    fn a_script_names_each_state_in_lower_case() {
        assert_eq!(ObjectiveState::from_script("empty"), Some(ObjectiveState::Empty));
        assert_eq!(ObjectiveState::from_script("active"), Some(ObjectiveState::Active));
        assert_eq!(ObjectiveState::from_script("invisible"), Some(ObjectiveState::Invisible));
        assert_eq!(ObjectiveState::from_script("done"), Some(ObjectiveState::Done));
        assert_eq!(ObjectiveState::from_script("current"), Some(ObjectiveState::Current));
        assert_eq!(ObjectiveState::from_script("failed"), Some(ObjectiveState::Failed));
    }

    #[test]
    fn a_state_name_has_to_match_exactly() {
        assert_eq!(ObjectiveState::from_script("Active"), None);
        assert_eq!(ObjectiveState::from_script("activ"), None);
        assert_eq!(ObjectiveState::from_script(""), None);
    }

    #[test]
    fn only_active_and_current_markers_are_drawn() {
        let drawn: Vec<_> = ObjectiveState::ALL.into_iter().filter(|state| state.drawn()).collect();
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

    #[test]
    fn server_information_is_found_whatever_the_case() {
        let info = ObjectiveMatch {
            server_info: vec![("MapName".into(), "nuketown".into()), ("mode".into(), "tdm".into())],
            ..ObjectiveMatch::default()
        };
        assert_eq!(info.server_info("mapname"), Some("nuketown"));
        assert_eq!(info.server_info("MODE"), Some("tdm"));
        assert_eq!(info.server_info("missing"), None);
    }

    #[test]
    fn the_first_of_two_entries_with_the_same_name_wins() {
        let info = ObjectiveMatch {
            server_info: vec![("a".into(), "one".into()), ("A".into(), "two".into())],
            ..ObjectiveMatch::default()
        };
        assert_eq!(info.server_info("a"), Some("one"));
    }
}
