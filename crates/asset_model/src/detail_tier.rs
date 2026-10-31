//! Choosing how detailed a model is drawn, from how far away it is.
//!
//! TODO(fact): these numbers come from the older game; Black Ops II's own are
//! not read yet.

/// A model has at most this many detail tiers.
pub const MAX_DETAIL_TIERS: usize = 4;

/// A placed model's distance is never divided by less than this when it is
/// shrunk for the model's size.
pub const SMALLEST_PLACED_SIZE: f32 = 1.0;

/// Times the tangent of half the vertical view angle, this gives the factor
/// distances are scaled by.
pub const VIEW_ANGLE_TO_FACTOR: f32 = 2.118_673_1;

pub const DEFAULT_STATIC_TIER_STRETCH: f32 = 1.0;
pub const DEFAULT_STATIC_TIER_SHIFT: f32 = 0.0;
pub const DEFAULT_ANIMATED_TIER_STRETCH: f32 = 1.0;
pub const DEFAULT_ANIMATED_TIER_SHIFT: f32 = 0.0;

/// How much wider, as a ratio, the view may get before the distance factor
/// is worked out again.
pub const DEFAULT_VIEW_ANGLE_CUTOFF: f32 = 2.4;

/// The tier for a model at scaled distance `scaled`, falling back to its
/// unscaled distance `unscaled` when no tier reaches that far. When neither
/// fits, a model that must stay drawn gets the next-to-last tier and any
/// other model is skipped.
#[must_use]
pub fn pick_detail_tier_by_range(
    tiers: i16,
    reach: [f32; MAX_DETAIL_TIERS],
    scaled: f32,
    unscaled: f32,
    keep_drawn: bool,
) -> Option<u8> {
    let usable = &reach[..usize::try_from(tiers).unwrap_or(0).min(MAX_DETAIL_TIERS)];
    let tier_reaching = |distance: f32| usable.iter().position(|&furthest| distance < furthest);
    let mut tier = tier_reaching(scaled);
    if tier.is_none() && unscaled != scaled {
        tier = tier_reaching(unscaled);
    }
    match (tier, keep_drawn) {
        (Some(found), _) => Some(found as u8),
        (None, true) => Some((i32::from(tiers) - 2).clamp(0, 3) as u8),
        (None, false) => None,
    }
}

/// The distance factor and offset for one group of models; both follow the
/// view angle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DetailParamsDirection {
    /// The factor the view angle last called for. It is kept until the view
    /// changes enough to need a new one.
    pub used_reciprocal_factor: f32,
    pub scale: f32,
    pub bias: f32,
}

impl DetailParamsDirection {
    /// Catches up with the current view angle and the player's distance
    /// settings.
    pub fn follow_view(&mut self, half_view_tan: f32, player_scale: f32, player_bias: f32, max_widening: f32) {
        let called_for = half_view_tan * VIEW_ANGLE_TO_FACTOR;
        let kept = self.used_reciprocal_factor;
        if kept <= called_for || kept / called_for > max_widening {
            self.used_reciprocal_factor = called_for;
        }
        let factor = self.used_reciprocal_factor;
        self.bias = player_bias * factor;
        self.scale = player_scale * factor;
    }

    /// A placed model's scaled and unscaled distance, both shrunk when the
    /// model was placed bigger than normal.
    #[must_use]
    pub fn placed_model_distances(self, distance: f32, placed_size: f32) -> (f32, f32) {
        let scaled = distance * self.scale + self.bias;
        let shrink = SMALLEST_PLACED_SIZE / placed_size.max(SMALLEST_PLACED_SIZE);
        (shrink * scaled, shrink * distance)
    }
}

/// A model's detail-tier data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelDetailSelector {
    T5 { detail_tier_total: i16, tier_reach: [f32; MAX_DETAIL_TIERS] },
}

impl ModelDetailSelector {
    #[must_use]
    pub fn detail_tier_total(self) -> i16 {
        let Self::T5 { detail_tier_total: tiers, .. } = self;
        tiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REACH: [f32; MAX_DETAIL_TIERS] = [100.0, 300.0, 900.0, 0.0];

    fn choose(tiers: i16, scaled: f32, unscaled: f32, keep_drawn: bool) -> Option<u8> {
        pick_detail_tier_by_range(tiers, REACH, scaled, unscaled, keep_drawn)
    }

    #[test]
    fn the_first_tier_reaching_the_distance_wins() {
        assert_eq!(choose(3, 50.0, 50.0, false), Some(0));
        assert_eq!(choose(3, 200.0, 200.0, false), Some(1));
        assert_eq!(choose(3, 899.0, 899.0, false), Some(2));
    }

    #[test]
    fn the_unscaled_distance_is_the_fallback() {
        assert_eq!(choose(3, 1000.0, 250.0, false), Some(1));
        assert_eq!(choose(3, 1000.0, 1000.0, false), None);
    }

    #[test]
    fn a_model_kept_drawn_falls_back_to_the_next_to_last_tier() {
        assert_eq!(choose(3, 1000.0, 1000.0, true), Some(1));
        assert_eq!(choose(1, 1000.0, 1000.0, true), Some(0));
        assert_eq!(choose(0, 1.0, 1.0, false), None);
    }

    #[test]
    fn the_factor_holds_until_the_view_changes_enough() {
        let cutoff = DEFAULT_VIEW_ANGLE_CUTOFF;
        let mut group = DetailParamsDirection::default();
        group.follow_view(1.0, 1.0, 0.0, cutoff);
        assert_eq!(group.used_reciprocal_factor, VIEW_ANGLE_TO_FACTOR);
        group.follow_view(0.8, 2.0, 5.0, cutoff);
        assert_eq!(group.used_reciprocal_factor, VIEW_ANGLE_TO_FACTOR);
        assert_eq!(group.scale, VIEW_ANGLE_TO_FACTOR * 2.0);
        assert_eq!(group.bias, VIEW_ANGLE_TO_FACTOR * 5.0);
        group.follow_view(0.1, 1.0, 0.0, cutoff);
        assert_eq!(group.used_reciprocal_factor, 0.1 * VIEW_ANGLE_TO_FACTOR);
    }

    #[test]
    fn a_model_placed_large_has_both_distances_shrunk() {
        let group = DetailParamsDirection { used_reciprocal_factor: 1.0, scale: 2.0, bias: 10.0 };
        assert_eq!(group.placed_model_distances(100.0, 1.0), (210.0, 100.0));
        assert_eq!(group.placed_model_distances(100.0, 2.0), (105.0, 50.0));
        assert_eq!(group.placed_model_distances(100.0, 0.5), (210.0, 100.0));
    }
}
