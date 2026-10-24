//! Post-fx frame data: the frame values the extract step fills and the
//! post-fx, DoF, bloom and sun-effect passes read. Moved out of the SM3 film code.

use super::postfx_dof::DofFrame;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct ExtractedPostFx {
    pub frame: DofFrame,
    pub vision: Option<asset_world::FilmVision>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PostFxSet;
