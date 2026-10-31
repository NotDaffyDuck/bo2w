//! Effects a script places in the world.

/// An effect a script placed in the world: which one, where and which way it faces, and when and how often it plays.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PlacedEffect {
    pub id: u32,
    /// Which effect plays.
    pub effect: u8,
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub up_dir: [f32; 3],
    /// When it starts; none until the scripts trigger it.
    pub start_ms: Option<i32>,
    /// Milliseconds between plays.
    pub repeat_ms: i32,
    /// Beyond this distance from the viewer it is not drawn.
    pub cull_distance: f32,
}
