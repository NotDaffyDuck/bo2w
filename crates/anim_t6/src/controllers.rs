//! The player's DObj controllers (`clientControllers_t`) and an anim's turn
//! speed, from `facts/xanim-dobj-t6.md` sections 9 and 10.

/// The six angle slots, in the order the setup code fills them (Neck and
/// Head take the two places left: inferred). They are data only: MP turns
/// no model bone by them (facts 10b), so no slot has a bone name.
pub const CONTROLLER_BACK_LOW: usize = 0;
pub const CONTROLLER_BACK_MID: usize = 1;
pub const CONTROLLER_BACK_UP: usize = 2;
pub const CONTROLLER_NECK: usize = 3;
pub const CONTROLLER_HEAD: usize = 4;
pub const CONTROLLER_PELVIS: usize = 5;
pub const CONTROLLER_SLOTS: usize = 6;

/// One player's controller angles: six slots plus the root tag's own
/// angles and offset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerControllers {
    pub angles: [[f32; 3]; CONTROLLER_SLOTS],
    pub tag_origin_angles: [f32; 3],
    pub tag_origin_offset: [f32; 3],
}

impl PlayerControllers {
    /// `BG_Player_DoControllers`' only DObj write: bone 0 (the root
    /// `tag_origin`) gets the origin offset and angles, and only when any of
    /// those six floats is non-zero. The six slot angles are computed but no
    /// code applying them to a bone was found.
    pub fn root_write(&self) -> Option<([f32; 3], [f32; 3])> {
        let any = self
            .tag_origin_offset
            .iter()
            .chain(&self.tag_origin_angles)
            .any(|v| *v != 0.0);
        any.then_some((self.tag_origin_offset, self.tag_origin_angles))
    }
}

/// An anim's turn speed in degrees per second, set once when the player
/// anims are finalised: (1 / length in seconds) times the anim's XAnim
/// parameter "turn" (0 without one). Not taken from the clip's yaw delta.
/// `BG_RunLerpFrameRate` reads it only for an anim with no move speed
/// ([`crate::t6_turn_rate`]).
///
/// TODO(rotspeed-wiring): the asset parser does not read the XAnim "turn"
/// parameter yet (the assets lane is Clean-out 2's), and the event and
/// mantle types the script parser zeroes it for are in
/// `anim-dpvs-t6.md` section 6. Until then nothing calls this and a turn
/// in place plays at 1.0.
pub fn t6_rot_speed(turn: Option<f32>, length_s: f32) -> f32 {
    match turn {
        Some(turn) if length_s > 0.0 => turn / length_s,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_nonzero_root_is_written() {
        let mut c = PlayerControllers::default();
        c.angles[CONTROLLER_HEAD] = [10.0, 0.0, 0.0];
        assert_eq!(c.root_write(), None);
        c.tag_origin_angles[1] = 5.0;
        assert_eq!(c.root_write(), Some(([0.0; 3], [0.0, 5.0, 0.0])));
    }

    #[test]
    fn rot_speed_is_turn_over_length() {
        assert_eq!(t6_rot_speed(Some(90.0), 2.0), 45.0);
        assert_eq!(t6_rot_speed(None, 2.0), 0.0);
        assert_eq!(t6_rot_speed(Some(90.0), 0.0), 0.0);
    }
}
