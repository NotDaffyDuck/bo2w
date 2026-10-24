//! What a map's compiled scripts place for sound and effects: emitters at
//! points, and sounds played now and then.

use bevy::prelude::Resource;

/// A sound that loops where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateFxLoopSound {
    pub soundalias: String,
    pub origin_inches: [f32; 3],
}

/// An effect started where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateFxOneshot {
    pub fxid: String,
    pub origin_inches: [f32; 3],
    pub angles_deg: [f32; 3],
    /// Seconds after the match starts.
    pub delay: f32,
}

/// The effect emitters a session has placed.
#[derive(Clone, Debug, Default, Resource)]
pub struct CreateFxOneshotEmitters(pub Vec<CreateFxOneshot>);

/// A sound played now and then at one of a few places.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomPointSound {
    pub soundalias: String,
    /// Seconds between plays: [min, max].
    pub wait_secs: [f32; 2],
    pub places_inches: Vec<[f32; 3]>,
}

/// Everything a map's compiled scripts place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptedMapFx {
    pub oneshots: Vec<CreateFxOneshot>,
    pub loop_sounds: Vec<CreateFxLoopSound>,
    pub random_sounds: Vec<RandomPointSound>,
}
