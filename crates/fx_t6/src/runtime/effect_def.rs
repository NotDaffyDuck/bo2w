//! An effect's definition header.

/// The counts at the head of an effect def, and where its element defs start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxEffectDef {
    /// How long a looping effect runs, in milliseconds (zero if it does not
    /// loop).
    pub msec_looping_life: i32,
    /// How many element defs loop.
    pub looping_count: i32,
    /// How many element defs spawn once.
    pub one_shot_count: i32,
    /// How many element defs are emitted by other elements.
    pub emission_count: i32,
    /// Address of the first element def.
    pub elem_defs: u32,
}
