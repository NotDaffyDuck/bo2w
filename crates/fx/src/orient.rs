//! A part's world origin from its effect's frames and run mode.

use fx_t6::runtime::{
    FxOrientFrame, FxOrientSpawnParams, get_orientation, orientation_pos_to_world,
};

/// `elem_origin` is stored relative to the frame the run mode picks: the
/// effect's current frame, the one it was played with, or a spawn-time
/// placement inside it.
pub fn elem_world_origin(
    elem_origin: [f32; 3],
    flags: i32,
    effect_now: &FxOrientFrame,
    effect_played: &FxOrientFrame,
    spawn: Option<FxOrientSpawnParams>,
) -> [f32; 3] {
    let frame = get_orientation(flags, effect_now, effect_played, spawn);
    orientation_pos_to_world(frame.origin, frame.axis, elem_origin)
}
