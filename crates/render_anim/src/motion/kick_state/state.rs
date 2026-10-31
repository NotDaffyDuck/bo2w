//! The running kick values, and the random stream a shot draws its roll from.

/// Where the random stream starts. It is fixed so a replayed session rolls the
/// same kicks; `reset` deliberately does not rewind it.
const STREAM_START: u32 = 0xA341_316C;

/// How far the view and the gun are currently kicked, and how fast they move.
///
/// Four arrays move together:
/// - the view is pushed by `punch_spin_rate` (degrees per second) and so builds up
///   `recoil_tilt`, which the camera adds to its aim;
/// - the gun swings by `gun_swing_rate` and so builds up `weapon_aim_angles`.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewRecoilState {
    /// How fast the view is being pushed: pitch, yaw, roll.
    pub punch_spin_rate: [f32; 3],

    /// How far the view is currently kicked: pitch, yaw, roll.
    pub recoil_tilt: [f32; 3],

    /// How fast the gun is swinging: pitch, yaw.
    pub gun_swing_rate: [f32; 2],

    /// How far the gun is currently swung: pitch, yaw.
    pub weapon_aim_angles: [f32; 2],

    /// The state of the random stream. Visible to the sibling files that roll
    /// shots, and to the tests that pin the stream.
    pub(super) rng: u32,
}

impl Default for ViewRecoilState {
    fn default() -> Self {
        Self {
            rng: STREAM_START,
            weapon_aim_angles: Default::default(),
            gun_swing_rate: Default::default(),
            recoil_tilt: Default::default(),
            punch_spin_rate: Default::default(),
        }
    }
}

impl ViewRecoilState {
    /// Stop every motion and put the view and gun back at rest.
    ///
    /// The random stream carries on from where it was, so two lives in a row do
    /// not replay the same recoil pattern.
    pub fn restart(&mut self) {
        let resume_from = self.rng;
        *self = Self::default();
        self.rng = resume_from;
    }

    /// Draw the next number in [0, 1] from the stream.
    pub(super) fn next_unit(&mut self) -> f32 {
        let drawn = scramble(self.rng);
        // The scramble can never reach zero from a non-zero start, but a zero
        // would trap the stream forever, so it is nudged off just in case.
        self.rng = if drawn == 0 { 1 } else { drawn };
        drawn as f32 / u32::MAX as f32
    }
}

/// One step of a 32-bit xorshift generator.
fn scramble(mut bits: u32) -> u32 {
    bits ^= bits << 13;
    bits ^= bits >> 17;
    bits ^= bits << 5;
    bits
}
