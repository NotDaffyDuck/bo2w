//! The per-frame movement scratch that every step of a move reads and writes.

/// What one move frame carries between its steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickLocals {
    /// The view's forward, right and up axes for this frame.
    pub fwd_axis: [f32; 3],
    pub right_axis: [f32; 3],
    pub up_axis: [f32; 3],
    /// The frame length in seconds and in whole milliseconds.
    pub step_millis_elapsed: f32,
    pub frame_ms: i32,
    /// Non-zero while the player is walking on the ground.
    pub walking: u32,
    /// Non-zero when the ground trace found a plane to stand on.
    pub standing_on_floor: u32,
    /// Non-zero when the ground is close enough to count as standing.
    pub near_floor_contact: u32,
    /// The last ground trace, as raw words: the normal sits in words 1 to 3
    /// as float bits, the surface flags in word 4.
    pub floor_probe: [u32; 11],
    /// Where and how fast the player was when the frame began.
    pub prior_tick_position: [f32; 3],
    pub prior_tick_speed_vec: [f32; 3],
    /// The frame's random seed.
    pub frame_seed: i32,
}

impl TickLocals {
    /// The player is off the ground for the rest of the frame: not walking,
    /// with no plane, near or under them, to stand on.
    pub fn leave_ground(&mut self) {
        [self.walking, self.standing_on_floor, self.near_floor_contact] = [0; 3];
    }
}
