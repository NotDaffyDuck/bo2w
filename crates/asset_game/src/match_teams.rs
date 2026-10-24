//! The announcer voice and music each team of a match is given.

use bevy::prelude::Resource;

/// The music a team hears at each turn of a match. A moment with no music
/// stays `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamMusic {
    pub spawn: Option<String>,
    pub victory: Option<String>,
    pub defeat: Option<String>,
    pub winning: Option<String>,
    pub losing: Option<String>,
}

/// What the map says about its teams' voices and music. A map that says
/// nothing leaves every field empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapTeamSettings {
    pub allies_voice: Option<String>,
    pub axis_voice: Option<String>,
    pub allies_music: TeamMusic,
    pub axis_music: TeamMusic,
}

/// The settings of the map in play.
#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct SessionTeamSettings(pub MapTeamSettings);
