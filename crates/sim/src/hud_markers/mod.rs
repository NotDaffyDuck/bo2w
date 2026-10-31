//! What the map's scripts show on the HUD and the scene: compass markers, vehicles, scores and the other match
//! information that goes to every client.

mod compass_marks;
mod effects;

pub use self::compass_marks::{CompassMark, CompassRide, ObjectiveState};
pub use self::effects::PlacedEffect;

/// Everything of the objective kind the match publishes to its clients.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MatchBoard {
    /// Team scores indexed by team number: free, allies, axis.
    pub scores: [i32; 3],
    /// Named pieces of server information, looked up without regard to case.
    pub server_info: Vec<(String, String)>,
    /// When the game ends, in match milliseconds.
    pub game_end_time: i32,
    pub compass: Vec<CompassMark>,
    pub vehicles: Vec<CompassRide>,
    /// Whether the scripts have taken over the map's own effects.
    pub scripted_effects: bool,
    pub effects: Vec<PlacedEffect>,
    pub fog: Option<crate::ScriptFog>,
    pub earthquakes: Vec<crate::ScriptEarthquake>,
    pub naked_vision: Option<crate::VisionChange>,
    pub thermal_vision: Option<crate::VisionChange>,
    pub night_vision: Option<crate::VisionChange>,
    pub missile_vision: Option<crate::VisionChange>,
    pub pain_vision: Option<crate::VisionChange>,
}

impl MatchBoard {
    /// The server information called `wanted` (in any case), if the scripts set it. When two entries share a name the
    /// first one wins.
    pub fn server_info(&self, wanted: &str) -> Option<&str> {
        let (_, value) = self.server_info.iter().find(|(key, _)| key.eq_ignore_ascii_case(wanted))?;
        Some(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_info(entries: &[(&str, &str)]) -> MatchBoard {
        let mut board = MatchBoard::default();
        board.server_info = entries.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect();
        board
    }

    #[test]
    fn server_information_is_found_whatever_the_case() {
        let info = with_info(&[("MapName", "nuketown"), ("mode", "tdm")]);
        assert_eq!(info.server_info("mapname"), Some("nuketown"));
        assert_eq!(info.server_info("MODE"), Some("tdm"));
        assert_eq!(info.server_info("missing"), None);
    }

    #[test]
    fn the_first_of_two_entries_with_the_same_name_wins() {
        let info = with_info(&[("a", "one"), ("A", "two")]);
        assert_eq!(info.server_info("a"), Some("one"));
    }
}
