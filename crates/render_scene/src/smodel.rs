//! Static-model draw inputs: the material a pass draws with, the LOD choice by
//! camera distance, and the maps a runtime material supplies.

use bevy::prelude::*;
use bevy::render::render_resource::Face;
use render_material::RuntimeMaterialCatalog;

/// Everything a static-model pass needs of one material.
#[derive(Clone, Debug, PartialEq)]
pub struct SmodelPassMaterial {
    pub model_lighting_required: bool,
    pub color: Option<Handle<Image>>,
    pub specular: Option<Handle<Image>>,
    pub probe: Option<Handle<Image>>,
    pub atlas: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub draw_mode: Option<asset_material::MaterialDrawMode>,
    pub cull_mode: Option<Face>,
    pub env_map_parms: [f32; 4],
    pub lighting_lookup_scale: [f32; 4],
    pub atlas_lookup: [f32; 4],
    pub sort_key: u8,
    pub material_sorted_index: Option<u32>,
    /// The material's index in the runtime catalog, for materials that carry no
    /// technique set and so no sorted ordinal; the fallback draw finds them by it.
    pub t6_material: Option<u32>,
}

/// The distance ramp that stretches or shrinks LOD ranges.
#[derive(Clone, Copy, Debug, Default)]
pub struct LodRampArgs {
    pub scale_mid: Option<f32>,
    pub bias_mid: Option<f32>,
    pub scale_last: Option<f32>,
    pub world_unit: Option<f32>,
    pub t5: asset_model::LodParmsAxis,
    pub t5_no_lod_cull_out: bool,
}

/// The LOD level to draw a model at from the eye, or `None` when it is beyond its
/// last level and culled. Without an eye or LOD data there is nothing to choose
/// by, so the first level is drawn.
pub fn smodel_camera_lod(
    lod: Option<asset_model::ModelLodSelector>,
    origin: [f32; 3],
    scale: f32,
    eye: Option<Vec3>,
    ramp: LodRampArgs,
) -> Option<u8> {
    let (Some(eye), Some(lod)) = (eye, lod) else {
        return Some(0);
    };
    let asset_model::ModelLodSelector::T5 { num_lods, lod_dist } = lod;
    let distance = eye.distance(Vec3::from_array(origin));
    let (ramped, base) = ramp.t5.smodel_call_dists(distance, scale);
    asset_model::xmodel_get_lod_for_dist(num_lods, lod_dist, ramped, base, ramp.t5_no_lod_cull_out)
}

/// Which faces a material culls: 0 drops back faces, 1 front faces, anything
/// else (or nothing) draws both.
pub fn runtime_cull_face(cull_mode: Option<u8>) -> Option<Face> {
    match cull_mode {
        Some(0) => Some(Face::Back),
        Some(1) => Some(Face::Front),
        _ => None,
    }
}

/// The maps and draw state a runtime material gives a model.
#[derive(Clone, Debug)]
pub struct AuthoredMaps {
    pub color: Option<Handle<Image>>,
    pub specular: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub draw_mode: Option<asset_material::MaterialDrawMode>,
    pub cull_mode: Option<Face>,
    pub env_map_parms: [f32; 4],
}

/// Looks a material up in the catalog and picks out its colour and specular
/// maps from the loaded images. A material the catalog lacks gets no maps.
pub fn runtime_maps(
    material: Option<assets::MaterialIndex>,
    catalog: &RuntimeMaterialCatalog,
    exact_handles: &[Option<Handle<Image>>],
) -> AuthoredMaps {
    let runtime = material.and_then(|id| catalog.derived(id));
    let map_for = |semantic: u8| {
        runtime
            .and_then(|m| m.texture_semantic(semantic))
            .and_then(|image| exact_handles.get(image.0 as usize))
            .and_then(Clone::clone)
    };
    AuthoredMaps {
        color: map_for(asset_material::TS_COLOR_MAP),
        specular: map_for(asset_material::TS_SPECULAR_MAP),
        alpha_mode: AlphaMode::Opaque,
        draw_mode: None,
        cull_mode: runtime.and_then(|m| runtime_cull_face(m.cull_mode)),
        env_map_parms: runtime.map_or([0.0; 4], |m| m.env_map_parms()),
    }
}

/// The skinned-model LOD ramp as the player's settings give it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LodRampSkinnedDvar {
    pub scale_mid: Option<f32>,
    pub bias_mid: Option<f32>,
    pub scale_last: Option<f32>,
    pub world_unit: Option<f32>,
    pub t5_scale: f32,
    pub t5_bias: f32,
    pub t5_fov_threshold: f32,
    pub t5: asset_model::LodParmsAxis,
}

impl Default for LodRampSkinnedDvar {
    fn default() -> Self {
        Self {
            scale_mid: None,
            bias_mid: None,
            scale_last: None,
            world_unit: None,
            t5_scale: asset_model::R_LOD_SCALE_SKINNED_DEFAULT,
            t5_bias: asset_model::R_LOD_BIAS_SKINNED_DEFAULT,
            t5_fov_threshold: asset_model::R_FOV_SCALE_THRESHOLD_DEFAULT,
            t5: asset_model::LodParmsAxis::default(),
        }
    }
}

impl LodRampSkinnedDvar {
    /// The ramp to hand to [`smodel_camera_lod`]: skinned models are culled
    /// past their last level, never kept at it.
    #[must_use]
    pub fn args(self) -> LodRampArgs {
        LodRampArgs {
            scale_mid: self.scale_mid,
            bias_mid: self.bias_mid,
            scale_last: self.scale_last,
            world_unit: self.world_unit,
            t5: self.t5,
            t5_no_lod_cull_out: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cull_mode_zero_and_one_pick_a_face() {
        assert_eq!(runtime_cull_face(Some(0)), Some(Face::Back));
        assert_eq!(runtime_cull_face(Some(1)), Some(Face::Front));
        assert_eq!(runtime_cull_face(Some(2)), None);
        assert_eq!(runtime_cull_face(None), None);
    }

    #[test]
    fn no_eye_or_no_lod_data_draws_the_first_level() {
        let lod = asset_model::ModelLodSelector::T5 { num_lods: 3, lod_dist: [1.0; 4] };
        let ramp = LodRampArgs::default();
        assert_eq!(smodel_camera_lod(Some(lod), [0.0; 3], 1.0, None, ramp), Some(0));
        assert_eq!(smodel_camera_lod(None, [0.0; 3], 1.0, Some(Vec3::X), ramp), Some(0));
    }

    #[test]
    fn a_model_with_no_levels_is_culled_with_an_eye() {
        let lod = asset_model::ModelLodSelector::T5 { num_lods: 0, lod_dist: [0.0; 4] };
        let ramp = LodRampArgs::default();
        assert_eq!(smodel_camera_lod(Some(lod), [0.0; 3], 1.0, Some(Vec3::X), ramp), None);
    }

    #[test]
    fn the_skinned_ramp_never_keeps_a_model_past_its_last_level() {
        let dvar = LodRampSkinnedDvar::default();
        assert!(!dvar.args().t5_no_lod_cull_out);
        assert_eq!(dvar.t5_scale, asset_model::R_LOD_SCALE_SKINNED_DEFAULT);
    }

    #[test]
    fn a_missing_material_gets_no_maps() {
        let maps = runtime_maps(None, &RuntimeMaterialCatalog::default(), &[]);
        assert!(maps.color.is_none() && maps.specular.is_none());
        assert_eq!(maps.cull_mode, None);
        assert_eq!(maps.env_map_parms, [0.0; 4]);
    }
}
