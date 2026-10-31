//! The per-frame draw products: the colour, light, emissive and distortion
//! lists seen from the camera, and the sun and spot shadow caster lists.
//!
//! A frame goes through these steps (the systems are ordered in
//! `assemble/mod.rs`):
//!
//! 1. [`open_frame_outputs`] gathers the frame's inputs;
//! 2. the shadow bakes plan what casts into the sun and spot shadow maps;
//! 3. [`execute_camera_outputs`], [`execute_daylight_output`] and
//!    [`execute_cone_output`] build the lists, reusing last frame's work
//!    whenever its stamp still matches;
//! 4. [`publish_frame_outputs`] swaps the finished products into a
//!    [`RenderFrameProducts`] bank, which the renderer reads.
//!
//! Module map:
//!
//! * `bank`: the ring of published snapshots;
//! * `lanes`: reading the three draw lanes as one ordered list;
//! * `tech`: which technique a row is drawn with;
//! * `memo`: collapsing repeated draws, and replaying that collapse;
//! * `stamps`: what each list depends on, as comparable values and digests;
//! * `slot`: a product with its memo and the three ways to fill it;
//! * `inputs`, `dynamic_lights`: the frame inputs and the dynamic-light list;
//! * `camera`, `sun`, `spot`: the three product builders;
//! * `publish`, `census`: handing the frame over, and a probe diagnostic.

use std::sync::Arc;

use bevy::prelude::*;

use super::material_setup::MethodType;
use super::material_live::{StagedMaterialTable, LiveEngineSources, LiveMaterialLedger};

mod bank;
mod cam_xf;
mod tally;
mod dynamic_lights;
mod in_set;
mod lane_set;
mod memo;
mod publish;
mod seat;
mod spot;
mod stamps;
mod daylight;
mod technique_ix;

pub use bank::FrameRenderOutputs;
pub use cam_xf::CameraOutputs;
pub use in_set::FrameBuildFeed;
pub use render_anim::DrawFocus;
pub use render_frame::{
    FrameOutputItem, FrameOutputKind, FrameOutputState, FrameOutputsSnapshot, AbsentOutputReason,
    OutputTarget, FocusedFrameSnapshot, ConeShadowFrameSlot,
};
pub use spot::ConeOutput;
pub use daylight::DaylightOutput;

pub(crate) use cam_xf::execute_camera_outputs;
pub(crate) use tally::log_env_probe_index_census;
pub(crate) use in_set::open_frame_outputs;
pub(crate) use publish::publish_frame_outputs;
pub(crate) use spot::{bake_cone_shadow_casters, execute_cone_output};
pub(crate) use daylight::{
    StaticSunCasters, bake_static_sun_shadow_casters, bake_daylight_shadow_casters, execute_daylight_output,
};

/// Whether refraction/distortion draws are produced at all.
#[derive(Resource)]
pub struct WarpSettings {
    pub enabled: bool,
}

impl Default for WarpSettings {
    fn default() -> Self {
        let enabled = true;
        Self { enabled }
    }
}

/// Which technique family the camera lists are drawn with.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColourPaintMethod {
    #[default]
    Standard,
    Unshaded,
    MaterialProbe,
}

impl ColourPaintMethod {
    /// Technique of the lit (colour) pass.
    pub const fn shading_technique_kind(self) -> MethodType {
        match self {
            Self::Standard => MethodType(lighting_t6::LIT_DRAW_PATH_FIRST),
            Self::Unshaded => MethodType(lighting_t6::TECHNIQUE_UNLIT),
            Self::MaterialProbe => MethodType(lighting_t6::TECHNIQUE_DEBUG),
        }
    }

    /// Technique of the emissive pass.
    pub const fn emissive_tech_type(self) -> MethodType {
        match self {
            Self::Standard => MethodType(lighting_t6::TECHNIQUE_EMISSIVE),
            Self::Unshaded | Self::MaterialProbe => self.shading_technique_kind(),
        }
    }
}

/// The material catalog together with its prepared form.
#[derive(Resource, Clone, Debug, Default)]
pub struct MaterialEpoch {
    pub catalog: Arc<LiveMaterialLedger>,
    pub prepared: Arc<StagedMaterialTable>,
}

/// Prepares `catalog` for drawing. BO2 draws every pass through the native
/// T6 path, so no pass carries a shader-program port.
pub(crate) fn admit_material_generation(catalog: Arc<LiveMaterialLedger>) -> MaterialEpoch {
    let prepared = Arc::new(StagedMaterialTable::build_from_index(&catalog, |_, _| None));
    MaterialEpoch { catalog, prepared }
}

/// Per-frame values the material system reads while drawing.
#[derive(Resource, Clone, Debug, Default)]
pub struct MaterialFrameFeed {
    pub engine_sources: LiveEngineSources,
    pub eye_origin: Vec3,
    pub time_float: f32,
    pub world_to_clip: Option<Mat4>,
    pub world_to_view: Option<Mat4>,
    pub view_to_clip: Option<Mat4>,
    pub outdoor: Option<super::MapOpenair>,
    pub hand_model_clip_from_level: Option<Mat4>,
    pub hand_near_plane: Option<f32>,
    pub sun_shadow: Option<super::DaylightShadowForcedFrame>,
    pub cone_receivers: Vec<Option<render_frame::ConeShadowSink>>,
}

/// The camera list a draw with this camera region belongs to; `None` for a
/// draw that no camera list takes.
pub(crate) fn frame_product_kind_for_camera_region(area: Option<u8>) -> Option<FrameOutputKind> {
    match area {
        Some(render_material::VIEW_STAGE_ABSENT) => None,
        Some(render_material::VIEW_STAGE_SELF_LIT) => Some(FrameOutputKind::Emissive),
        _ => Some(FrameOutputKind::Colour),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_draws_lit_and_emissive_with_their_own_techniques() {
        let method = ColourPaintMethod::Standard;
        assert_eq!(method.shading_technique_kind().0, lighting_t6::LIT_DRAW_PATH_FIRST);
        assert_eq!(method.emissive_tech_type().0, lighting_t6::TECHNIQUE_EMISSIVE);
    }

    #[test]
    fn the_debug_methods_use_one_technique_for_both_passes() {
        for method in [ColourPaintMethod::Unshaded, ColourPaintMethod::MaterialProbe] {
            assert_eq!(method.shading_technique_kind(), method.emissive_tech_type());
        }
        assert_eq!(ColourPaintMethod::Unshaded.shading_technique_kind().0, lighting_t6::TECHNIQUE_UNLIT);
        assert_eq!(ColourPaintMethod::MaterialProbe.shading_technique_kind().0, lighting_t6::TECHNIQUE_DEBUG);
    }

    #[test]
    fn camera_regions_pick_their_list() {
        use render_material::*;
        assert_eq!(frame_product_kind_for_camera_region(None), Some(FrameOutputKind::Colour));
        assert_eq!(frame_product_kind_for_camera_region(Some(VIEW_STAGE_ABSENT)), None);
        assert_eq!(
            frame_product_kind_for_camera_region(Some(VIEW_STAGE_SELF_LIT)),
            Some(FrameOutputKind::Emissive)
        );
        for region in [VIEW_STAGE_SOLID_LIT, VIEW_STAGE_BLENDED_LIT, CAMERA_REGION_NEAR_DEPTH_TRICK] {
            assert_eq!(
                frame_product_kind_for_camera_region(Some(region)),
                Some(FrameOutputKind::Colour)
            );
        }
    }

    #[test]
    fn distortion_is_on_by_default() {
        assert!(WarpSettings::default().enabled);
    }
}
