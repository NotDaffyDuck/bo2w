//! Which level of detail a model is drawn at, and the distances that decide it.
//!
//! TODO(fact): the numbers here are the ones the older game uses; Black Ops
//! II's own are not read yet.

/// The most levels of detail a model has.
pub const MAX_LODS: usize = 4;

/// A placed model is never counted smaller than this when its distance is
/// scaled down for its size.
pub const PLACEMENT_SCALE_FLOOR: f32 = 1.0;

/// What the tangent of half the vertical field of view is multiplied by to
/// get the distance scale.
pub const FOV_SCALE_CONST: f32 = 2.118_673_1;

pub const R_LOD_SCALE_RIGID_DEFAULT: f32 = 1.0;
pub const R_LOD_BIAS_RIGID_DEFAULT: f32 = 0.0;
pub const R_LOD_SCALE_SKINNED_DEFAULT: f32 = 1.0;
pub const R_LOD_BIAS_SKINNED_DEFAULT: f32 = 0.0;

/// How far the field of view may widen, as a ratio, before the distance scale
/// follows it.
pub const R_FOV_SCALE_THRESHOLD_DEFAULT: f32 = 2.4;

/// The level of detail for a model `dist` away (its distance after the scale
/// and bias), or, when none fits, for the plain distance `base_dist`.
/// `lod_dist` holds how far each level reaches. With no level left, a model
/// that may not be culled falls back to the second-last level, and any other
/// model is not drawn.
#[must_use]
pub fn xmodel_get_lod_for_dist(
    num_lods: i16,
    lod_dist: [f32; MAX_LODS],
    dist: f32,
    base_dist: f32,
    no_lod_cull_out: bool,
) -> Option<u8> {
    let levels = usize::try_from(num_lods).unwrap_or(0).min(MAX_LODS);
    let level_reaching = |at: f32| lod_dist[..levels].iter().position(|&reach| at < reach);
    let level = level_reaching(dist).or_else(|| {
        if base_dist != dist {
            level_reaching(base_dist)
        } else {
            None
        }
    });
    match level {
        Some(level) => Some(level as u8),
        None if no_lod_cull_out => Some((i32::from(num_lods) - 2).clamp(0, 3) as u8),
        None => None,
    }
}

/// The distance scale and bias for one kind of model, which follow the field
/// of view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LodParmsAxis {
    /// The factor the field of view last asked for, which holds until the
    /// view moves far enough to need a new one.
    pub applied_inv_scale: f32,
    pub scale: f32,
    pub bias: f32,
}

impl LodParmsAxis {
    /// Brings the axis up to the current field of view and the player's
    /// scale and bias settings.
    pub fn update(
        &mut self,
        tan_half_fov_y: f32,
        scale_dvar: f32,
        bias_dvar: f32,
        fov_scale_threshold: f32,
    ) {
        let wanted = tan_half_fov_y * FOV_SCALE_CONST;
        let view_narrowed = self.applied_inv_scale <= wanted;
        let view_widened_far = self.applied_inv_scale / wanted > fov_scale_threshold;
        if view_narrowed || view_widened_far {
            self.applied_inv_scale = wanted;
        }
        self.scale = self.applied_inv_scale * scale_dvar;
        self.bias = self.applied_inv_scale * bias_dvar;
    }

    /// The scaled and the plain distance of a placed model: both are shrunk
    /// for a model placed larger than normal.
    #[must_use]
    pub fn smodel_call_dists(self, dist: f32, placement_scale: f32) -> (f32, f32) {
        let shrink = PLACEMENT_SCALE_FLOOR / placement_scale.max(PLACEMENT_SCALE_FLOOR);
        (shrink * (dist * self.scale + self.bias), shrink * dist)
    }
}

/// The level-of-detail data of one model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelLodSelector {
    T5 { num_lods: i16, lod_dist: [f32; 4] },
}

impl ModelLodSelector {
    #[must_use]
    pub fn num_lods(self) -> i16 {
        let Self::T5 { num_lods, .. } = self;
        num_lods
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REACH: [f32; MAX_LODS] = [100.0, 300.0, 900.0, 0.0];

    #[test]
    fn the_first_level_that_reaches_far_enough_is_picked() {
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 50.0, 50.0, false), Some(0));
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 200.0, 200.0, false), Some(1));
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 899.0, 899.0, false), Some(2));
    }

    #[test]
    fn the_plain_distance_is_tried_when_the_scaled_one_is_too_far() {
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 1000.0, 250.0, false), Some(1));
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 1000.0, 1000.0, false), None);
    }

    #[test]
    fn a_model_that_may_not_be_culled_takes_the_second_last_level() {
        assert_eq!(xmodel_get_lod_for_dist(3, REACH, 1000.0, 1000.0, true), Some(1));
        assert_eq!(xmodel_get_lod_for_dist(1, REACH, 1000.0, 1000.0, true), Some(0));
        assert_eq!(xmodel_get_lod_for_dist(0, REACH, 1.0, 1.0, false), None);
    }

    #[test]
    fn the_axis_keeps_its_factor_until_the_view_moves_far() {
        let mut axis = LodParmsAxis::default();
        axis.update(1.0, 1.0, 0.0, R_FOV_SCALE_THRESHOLD_DEFAULT);
        assert_eq!(axis.applied_inv_scale, FOV_SCALE_CONST);
        axis.update(0.8, 2.0, 5.0, R_FOV_SCALE_THRESHOLD_DEFAULT);
        assert_eq!(axis.applied_inv_scale, FOV_SCALE_CONST);
        assert_eq!(axis.scale, FOV_SCALE_CONST * 2.0);
        assert_eq!(axis.bias, FOV_SCALE_CONST * 5.0);
        axis.update(0.1, 1.0, 0.0, R_FOV_SCALE_THRESHOLD_DEFAULT);
        assert_eq!(axis.applied_inv_scale, 0.1 * FOV_SCALE_CONST);
    }

    #[test]
    fn a_large_placement_shrinks_both_distances() {
        let axis = LodParmsAxis { applied_inv_scale: 1.0, scale: 2.0, bias: 10.0 };
        assert_eq!(axis.smodel_call_dists(100.0, 1.0), (210.0, 100.0));
        assert_eq!(axis.smodel_call_dists(100.0, 2.0), (105.0, 50.0));
        assert_eq!(axis.smodel_call_dists(100.0, 0.5), (210.0, 100.0));
    }
}
