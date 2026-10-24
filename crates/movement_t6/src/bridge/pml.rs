//! The per-frame movement scratch that every step of a move reads and writes.

/// What one move frame carries between its steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pml {
    /// The view's forward, right and up axes for this frame.
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    /// The frame length in seconds and in whole milliseconds.
    pub frametime: f32,
    pub msec: i32,
    /// Non-zero while the player is walking on the ground.
    pub walking: u32,
    /// Non-zero when the ground trace found a plane to stand on.
    pub ground_plane: u32,
    /// Non-zero when the ground is close enough to count as standing.
    pub almost_ground_plane: u32,
    /// The last ground trace, as raw words: the normal sits in words 1 to 3
    /// as float bits, the surface flags in word 4.
    pub ground_trace: [u32; 11],
    /// Where and how fast the player was when the frame began.
    pub previous_origin: [f32; 3],
    pub previous_velocity: [f32; 3],
    /// The frame's random seed.
    pub holdrand: i32,
}
