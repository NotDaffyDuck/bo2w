//! What the map's main script says about its sound.

use bevy::prelude::Resource;

/// The ambience the map starts and the team voice sets it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelScriptAudioSummary {
    /// The alias the script plays as the map's ambience.
    pub background_sound_entry: Option<String>,
    /// The voice set the attacking team uses.
    pub attacker_voices: Option<String>,
    /// The voice set the defending team uses.
    pub defender_voices: Option<String>,
    /// The script the facts were read from.
    pub source_script: Option<String>,
    /// bo2zm: sounds the map's scripts name (prepared at load, so a script
    /// sound never arrives late).
    pub prepare: Vec<String>,
}

/// The facts for the session's map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Resource)]
pub struct SessionMapScriptSound(pub LevelScriptAudioSummary);
