//! An effect's definition header.

/// The counts at the head of an effect def, and where its element defs start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectBlueprint {
    /// How long a looping effect runs, in milliseconds (zero if it does not
    /// loop).
    pub loop_lifetime_millis: i32,
    /// How many element defs loop.
    pub repeat_total: i32,
    /// How many element defs spawn once.
    pub single_play_total: i32,
    /// How many element defs are emitted by other elements.
    pub spawn_total: i32,
    /// Address of the first element def.
    pub particle_recipes: u32,
}
