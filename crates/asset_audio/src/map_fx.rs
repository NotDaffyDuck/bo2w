//! What a map's compiled scripts place for sound and effects: emitters at
//! points, and sounds played now and then.

use bevy::prelude::Resource;

/// A sound that loops where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnEffectAudioLoop {
    pub cue_alias: String,
    pub position_in_inches: [f32; 3],
    /// bo2zm: a line emitter's other end (a Black Ops II map's wind and
    /// power lines): the loop is heard from the line's point nearest the
    /// listener.
    pub line_end_inches: Option<[f32; 3]>,
}

/// An effect started where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnSingleShotEffect {
    pub effect_id: String,
    pub position_in_inches: [f32; 3],
    pub rotation_degrees: [f32; 3],
    /// Seconds after the match starts.
    pub start_after: f32,
}

/// The effect emitters a session has placed.
#[derive(Clone, Debug, Default, Resource)]
pub struct CreateFxOneshotEmitters(pub Vec<SpawnSingleShotEffect>);

/// A sound played now and then at one of a few places.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomPointSound {
    pub cue_alias: String,
    /// Seconds between plays: [min, max].
    pub wait_secs: [f32; 2],
    pub places_inches: Vec<[f32; 3]>,
}

/// Everything a map's compiled scripts place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptedMapFx {
    pub oneshots: Vec<SpawnSingleShotEffect>,
    pub loop_sounds: Vec<SpawnEffectAudioLoop>,
    pub random_sounds: Vec<RandomPointSound>,
    /// bo2zm: the map's ambient rooms and their echoes
    /// (`setambientroomreverb(room, preset, dry, wet)`): room, preset, dry,
    /// wet. The default room (`declareambientroom(room, true)`) is first.
    pub rooms: Vec<(String, String, f32, f32)>,
}
