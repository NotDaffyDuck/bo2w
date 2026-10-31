//! Post-fx frame data: the frame values the extract step fills and the
//! post-fx, DoF, bloom and sun-effect passes read. Moved out of the SM3 film code.

use super::dof_bloom::BokehFrame;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct ExtractedPostEffect {
    pub frame: BokehFrame,
    pub vision: Option<asset_world::ScreenGradeLook>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PostEffectSet;
