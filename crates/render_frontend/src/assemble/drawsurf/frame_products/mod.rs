//! The per-frame draw products: the colour, light, emissive and distortion
//! lists seen from the camera, and the sun and spot shadow caster lists.
//!
//! A frame goes through these steps (the systems are ordered in
//! `assemble/mod.rs`):
//!
//! 1. [`open_frame_products`] gathers the frame's inputs;
//! 2. the shadow bakes plan what casts into the sun and spot shadow maps;
//! 3. [`execute_camera_products`], [`execute_sun_product`] and
//!    [`execute_spot_product`] build the lists, reusing last frame's work
//!    whenever its stamp still matches;
//! 4. [`publish_frame_products`] swaps the finished products into a
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

use super::setup::TechType;
use super::material_runtime::{PreparedMaterialTable, RuntimeCodeSources, RuntimeMaterialCatalog};

mod bank;
mod camera;
mod census;
mod dynamic_lights;
mod inputs;
mod lanes;
mod memo;
mod publish;
mod slot;
mod spot;
mod stamps;
mod sun;
mod tech;

pub use bank::RenderFrameProducts;
pub use camera::CameraProducts;
pub use inputs::FrameAssemblyInputs;
pub use render_anim::RenderFocus;
pub use render_frame::{
    FrameProduct, FrameProductKind, FrameProductStatus, FrameProductsSnapshot, MissingProductCause,
    ProductTarget, RenderFocusFrame, SpotShadowFrameSlot,
};
pub use spot::SpotProduct;
pub use sun::SunProduct;

pub(crate) use camera::execute_camera_products;
pub(crate) use census::log_probe_index_census;
pub(crate) use inputs::open_frame_products;
pub(crate) use publish::publish_frame_products;
pub(crate) use spot::{bake_spot_shadow_casters, execute_spot_product};
pub(crate) use sun::{
    StaticSunCasters, bake_static_sun_shadow_casters, bake_sun_shadow_casters, execute_sun_product,
};

/// Whether refraction/distortion draws are produced at all.
#[derive(Resource)]
pub struct DistortionSettings {
    pub enabled: bool,
}

impl Default for DistortionSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Which technique family the camera lists are drawn with.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColourDrawMethod {
    #[default]
    Standard,
    Fullbright,
    DebugMaterial,
}

impl ColourDrawMethod {
    /// Technique of the lit (colour) pass.
    pub const fn tech_type(self) -> TechType {
        match self {
            Self::Standard => TechType(lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN),
            Self::Fullbright => TechType(lighting_t6::TECHNIQUE_UNLIT),
            Self::DebugMaterial => TechType(lighting_t6::TECHNIQUE_DEBUG),
        }
    }

    /// Technique of the emissive pass.
    pub const fn emissive_tech_type(self) -> TechType {
        match self {
            Self::Standard => TechType(lighting_t6::TECHNIQUE_EMISSIVE),
            Self::Fullbright | Self::DebugMaterial => self.tech_type(),
        }
    }
}

/// The material catalog together with its prepared form.
#[derive(Resource, Clone, Debug, Default)]
pub struct MaterialGeneration {
    pub catalog: Arc<RuntimeMaterialCatalog>,
    pub prepared: Arc<PreparedMaterialTable>,
}

/// Prepares `catalog` for drawing. BO2 draws every pass through the native
/// T6 path, so no pass carries a shader-program port.
pub(crate) fn admit_material_generation(catalog: Arc<RuntimeMaterialCatalog>) -> MaterialGeneration {
    let prepared = Arc::new(PreparedMaterialTable::from_catalog(&catalog, |_, _| None));
    MaterialGeneration { catalog, prepared }
}

/// Per-frame values the material system reads while drawing.
#[derive(Resource, Clone, Debug, Default)]
pub struct MaterialFrameInputs {
    pub code_sources: RuntimeCodeSources,
    pub view_origin: Vec3,
    pub float_time: f32,
    pub clip_from_world: Option<Mat4>,
    pub view_from_world: Option<Mat4>,
    pub clip_from_view: Option<Mat4>,
    pub outdoor: Option<super::MapOutdoor>,
    pub viewmodel_clip_from_world: Option<Mat4>,
    pub viewmodel_near: Option<f32>,
    pub sun_shadow: Option<super::SunShadowForcedFrame>,
    pub spot_receivers: Vec<Option<render_frame::SpotShadowReceiver>>,
}

/// The camera list a draw with this camera region belongs to; `None` for a
/// draw that no camera list takes.
pub(crate) fn frame_product_kind_for_camera_region(region: Option<u8>) -> Option<FrameProductKind> {
    match region {
        Some(render_material::CAMERA_REGION_NONE) => None,
        Some(render_material::CAMERA_REGION_EMISSIVE) => Some(FrameProductKind::Emissive),
        _ => Some(FrameProductKind::Colour),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_draws_lit_and_emissive_with_their_own_techniques() {
        let method = ColourDrawMethod::Standard;
        assert_eq!(method.tech_type().0, lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN);
        assert_eq!(method.emissive_tech_type().0, lighting_t6::TECHNIQUE_EMISSIVE);
    }

    #[test]
    fn the_debug_methods_use_one_technique_for_both_passes() {
        for method in [ColourDrawMethod::Fullbright, ColourDrawMethod::DebugMaterial] {
            assert_eq!(method.tech_type(), method.emissive_tech_type());
        }
        assert_eq!(ColourDrawMethod::Fullbright.tech_type().0, lighting_t6::TECHNIQUE_UNLIT);
        assert_eq!(ColourDrawMethod::DebugMaterial.tech_type().0, lighting_t6::TECHNIQUE_DEBUG);
    }

    #[test]
    fn camera_regions_pick_their_list() {
        use render_material::*;
        assert_eq!(frame_product_kind_for_camera_region(None), Some(FrameProductKind::Colour));
        assert_eq!(frame_product_kind_for_camera_region(Some(CAMERA_REGION_NONE)), None);
        assert_eq!(
            frame_product_kind_for_camera_region(Some(CAMERA_REGION_EMISSIVE)),
            Some(FrameProductKind::Emissive)
        );
        for region in [CAMERA_REGION_LIT_OPAQUE, CAMERA_REGION_LIT_TRANS, CAMERA_REGION_DEPTH_HACK] {
            assert_eq!(
                frame_product_kind_for_camera_region(Some(region)),
                Some(FrameProductKind::Colour)
            );
        }
    }

    #[test]
    fn distortion_is_on_by_default() {
        assert!(DistortionSettings::default().enabled);
    }
}
