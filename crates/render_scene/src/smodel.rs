//! Static-model draw inputs: the material a pass draws with, the LOD choice by
//! camera distance, and the maps a runtime material supplies.

use bevy::prelude::*;
use bevy::render::render_resource;
use render_material::LiveMaterialLedger;

/// Everything a static-model pass needs of one material.
#[derive(Clone, Debug, PartialEq)]
pub struct PropPassMaterial {
    pub model_illum_required: bool,
    pub color: Option<Handle<Image>>,
    pub specular: Option<Handle<Image>>,
    pub probe: Option<Handle<Image>>,
    pub atlas: Option<Handle<Image>>,
    pub see_through: AlphaMode,
    pub paint_mode: Option<asset_material::ShaderDrawStyle>,
    pub face_cull: Option<render_resource::Face>,
    pub env_map_params: [f32; 4],
    pub illum_lookup_scale: [f32; 4],
    pub atlas_find: [f32; 4],
    pub order_key: u8,
    pub material_draw_rank_slot: Option<u32>,
    /// The material's index in the runtime catalog, for materials that carry no
    /// technique set and so no sorted ordinal; the fallback draw finds them by it.
    pub t6_material: Option<u32>,
}

/// The distance ramp that stretches or shrinks LOD ranges.
#[derive(Clone, Copy, Debug, Default)]
pub struct DetailRampArgs {
    pub mid_scale: Option<f32>,
    pub mid_bias: Option<f32>,
    pub prior_size: Option<f32>,
    pub map_unit: Option<f32>,
    pub t5: asset_model::DetailParamsDirection,
    pub classic_no_detail_hide_out: bool,
}

/// The LOD level to draw a model at from the eye, or `None` when it is beyond its
/// last level and culled. Without an eye or LOD data there is nothing to choose
/// by, so the first level is drawn.
pub fn prop_camera_detail(
    lod: Option<asset_model::ModelDetailSelector>,
    at: [f32; 3],
    scale: f32,
    viewer: Option<Vec3>,
    ramp: DetailRampArgs,
) -> Option<u8> {
    let Some((seen_from, lod)) = viewer.zip(lod) else { return Some(0) };
    let asset_model::ModelDetailSelector::T5 { detail_tier_total, tier_reach: lod_dist } = lod;
    let distance = seen_from.distance(Vec3::from_array(at));
    let (stretched, plain) = ramp.t5.placed_model_distances(distance, scale);
    asset_model::pick_detail_tier_by_range(detail_tier_total, lod_dist, stretched, plain, ramp.classic_no_detail_hide_out)
}

/// Which faces a material culls: 0 drops back faces, 1 front faces, anything
/// else (or nothing) draws both.
pub fn culled_face_from_mode(mode: Option<u8>) -> Option<render_resource::Face> {
    match mode? {
        0 => Some(render_resource::Face::Back),
        1 => Some(render_resource::Face::Front),
        _ => None,
    }
}

/// The maps and draw state a runtime material gives a model.
#[derive(Clone, Debug)]
pub struct DesignerTextureSet {
    pub color: Option<Handle<Image>>,
    pub specular: Option<Handle<Image>>,
    pub see_through: AlphaMode,
    pub paint_mode: Option<asset_material::ShaderDrawStyle>,
    pub cull_mode: Option<render_resource::Face>,
    pub env_map_params: [f32; 4],
}

/// Looks a material up in the catalog and picks out its colour and specular
/// maps from the loaded images. A material the catalog lacks gets no maps.
pub fn live_model_tables(
    material_id: Option<assets::SurfaceMaterialId>,
    catalog: &LiveMaterialLedger,
    precise_handles: &[Option<Handle<Image>>],
) -> DesignerTextureSet {
    let live = material_id.and_then(|id| catalog.derived(id));
    let map_for = |semantic: u8| {
        let image = live?.semantic_for_image(semantic)?;
        precise_handles.get(image.0 as usize)?.clone()
    };
    DesignerTextureSet {
        color: map_for(asset_material::SEMANTIC_COLOR),
        specular: map_for(asset_material::SEMANTIC_SPECULAR),
        see_through: AlphaMode::default(),
        paint_mode: None,
        cull_mode: live.and_then(|m| culled_face_from_mode(m.cull_mode)),
        env_map_params: live.map_or([0.0; 4], |m| m.env_map_params()),
    }
}

/// The skinned-model LOD ramp as the player's settings give it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct DetailRampSkinnedTunable {
    pub mid_scale: Option<f32>,
    pub mid_bias: Option<f32>,
    pub prior_size: Option<f32>,
    pub map_unit: Option<f32>,
    pub classic_scale: f32,
    pub classic_bias: f32,
    pub classic_fov_threshold: f32,
    pub t5: asset_model::DetailParamsDirection,
}

impl Default for DetailRampSkinnedTunable {
    fn default() -> Self {
        let unset = DetailRampArgs::default();
        Self {
            mid_scale: unset.mid_scale,
            mid_bias: unset.mid_bias,
            prior_size: unset.prior_size,
            map_unit: unset.map_unit,
            classic_scale: asset_model::DEFAULT_ANIMATED_TIER_STRETCH,
            classic_bias: asset_model::DEFAULT_ANIMATED_TIER_SHIFT,
            classic_fov_threshold: asset_model::DEFAULT_VIEW_ANGLE_CUTOFF,
            t5: unset.t5,
        }
    }
}

impl DetailRampSkinnedTunable {
    /// The ramp to hand to [`prop_camera_detail`]: skinned models are culled
    /// past their last level, never kept at it.
    #[must_use]
    pub fn args(self) -> DetailRampArgs {
        let Self { mid_scale, mid_bias, prior_size: last_stretch, map_unit: unit, t5, .. } = self;
        let mut ramp = DetailRampArgs { mid_scale, mid_bias, t5, ..DetailRampArgs::default() };
        ramp.prior_size = last_stretch;
        ramp.map_unit = unit;
        ramp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cull_mode_zero_and_one_pick_a_face() {
        assert_eq!(culled_face_from_mode(Some(0)), Some(render_resource::Face::Back));
        assert_eq!(culled_face_from_mode(Some(1)), Some(render_resource::Face::Front));
        assert_eq!(culled_face_from_mode(Some(2)), None);
        assert_eq!(culled_face_from_mode(None), None);
    }

    #[test]
    fn no_eye_or_no_lod_data_draws_the_first_level() {
        let lod = asset_model::ModelDetailSelector::T5 { detail_tier_total: 3, tier_reach: [1.0; 4] };
        let ramp = DetailRampArgs::default();
        assert_eq!(prop_camera_detail(Some(lod), [0.0; 3], 1.0, None, ramp), Some(0));
        assert_eq!(prop_camera_detail(None, [0.0; 3], 1.0, Some(Vec3::X), ramp), Some(0));
    }

    #[test]
    fn a_model_with_no_levels_is_culled_with_an_eye() {
        let lod = asset_model::ModelDetailSelector::T5 { detail_tier_total: 0, tier_reach: [0.0; 4] };
        let ramp = DetailRampArgs::default();
        assert_eq!(prop_camera_detail(Some(lod), [0.0; 3], 1.0, Some(Vec3::X), ramp), None);
    }

    #[test]
    fn the_skinned_ramp_never_keeps_a_model_past_its_last_level() {
        let dvar = DetailRampSkinnedTunable::default();
        assert!(!dvar.args().classic_no_detail_hide_out);
        assert_eq!(dvar.classic_scale, asset_model::DEFAULT_ANIMATED_TIER_STRETCH);
    }

    #[test]
    fn a_missing_material_gets_no_maps() {
        let maps = live_model_tables(None, &LiveMaterialLedger::default(), &[]);
        assert!(maps.color.is_none() && maps.specular.is_none());
        assert_eq!(maps.cull_mode, None);
        assert_eq!(maps.env_map_params, [0.0; 4]);
    }
}
