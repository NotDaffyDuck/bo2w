//! What the map's main script says about its sound.

use bevy::prelude::Resource;

/// The ambience the map starts and the team voice sets it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapScriptSoundFacts {
    /// The alias the script plays as the map's ambience.
    pub ambient_alias: Option<String>,
    /// The voice set the attacking team uses.
    pub attackers: Option<String>,
    /// The voice set the defending team uses.
    pub defenders: Option<String>,
    /// The script the facts were read from.
    pub script: Option<String>,
}

/// The facts for the session's map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct SessionMapScriptSound(pub MapScriptSoundFacts);
