//! Result of one effects frame.

use bevy::prelude::Transform;
use fx::DecalMeshTally;
use fx::CornerBuildOutput;
use fx_t6::AfterglowSprite;

use crate::{effect_play::EffectVertexHoles, tracer::StreakLevel};

/// Where the eye stood and what it could see when the frame was built.
pub struct FxFrameView {
    pub camera: Transform,
    pub frustum: Vec<[f32; 4]>,
    pub view_proj: Option<[f32; 16]>,
    pub fov_half_tan: Option<(f32, f32)>,
    pub level_present: bool,
}

/// What a frame produced when effect generation ran to completion.
pub struct GeneratedFxFrame {
    pub products: CornerBuildOutput,
    pub gaps: EffectVertexHoles,
    pub marks: Option<DecalMeshTally>,
    pub after_lights: Vec<AfterglowSprite>,
    pub tracers: StreakLevel,
    pub view: FxFrameView,
}

/// Why effect generation could not run for a frame.
#[derive(Clone, Copy, Debug)]
pub enum FxSkipCause {
    NoCatalog,
}

/// Either a generated frame or the reason there is none.
pub enum FxFrameResult {
    Ready(GeneratedFxFrame),
    Skipped(FxSkipCause),
}
