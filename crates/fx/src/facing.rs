//! A part's world origin from its effect's frames and run mode.

use fx_t6::runtime::{
    EffectFacingBasis, FacingBirthKnobs, get_orientation, orientation_pos_to_world,
};

/// `elem_origin` is stored relative to the frame the run mode picks: the
/// effect's current frame, the one it was played with, or a spawn-time
/// placement inside it.
pub fn elem_world_origin(
    part_pos: [f32; 3],
    flags: i32,
    now_frame: &EffectFacingBasis,
    effect_played: &EffectFacingBasis,
    spawn: Option<FacingBirthKnobs>,
) -> [f32; 3] {
    let frame = get_orientation(flags, now_frame, effect_played, spawn);
    orientation_pos_to_world(frame.origin, frame.axis, part_pos)
}
