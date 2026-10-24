//! Tallying the corners a lookup kept: how much weight agrees with the chosen
//! light, how much could be traced, and the weight each colour set got.

use crate::pick::light_grid_primary_light_sun_band;

/// Turns the light index the pick settled on into the one a lookup reports.
/// Below the sun band a light the header says is out of range falls back; at
/// or above it the index counts from the sun's first.
pub fn light_grid_lookup_remap_primary(
    picked: u8,
    fallback_primary: u8,
    has_light_regions: bool,
    sun_primary_first: u32,
) -> u8 {
    let sun_band = light_grid_primary_light_sun_band(sun_primary_first);
    if u32::from(picked) >= sun_band {
        return picked
            .wrapping_sub(0xff)
            .wrapping_add(sun_primary_first as u8);
    }
    let out_of_range = picked == 0 || sun_primary_first < u32::from(picked);
    if has_light_regions && out_of_range {
        fallback_primary
    } else {
        picked
    }
}

/// Whether a corner whose light differs from the chosen one could still be
/// traced to it.
fn corner_wants_trace(corner_primary: u8, selected_primary: u8, sun_band: u32) -> bool {
    corner_primary == 0 || (u32::from(corner_primary) >= sun_band && selected_primary != 0)
}

/// Weight totals over the corners kept.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightGridLookupWeights {
    /// Weight of corners whose light is the chosen one.
    pub matching_primary: f32,
    /// Weight of other corners a trace could still count.
    pub traced_influence: f32,
    pub total: f32,
}

/// A corner that survived the pick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGridLookupCorner {
    pub colors_index: u16,
    pub primary_light: u8,
    pub weight: f32,
    pub trace_allows: bool,
}

/// The distinct colour sets among the corners and the weight each carries.
#[derive(Clone, Debug, PartialEq)]
pub struct LightGridLookupColorAccum {
    pub indices: [u16; 8],
    pub weights: [f32; 8],
    pub count: u32,
}

impl LightGridLookupColorAccum {
    pub const fn new() -> Self {
        Self {
            indices: [0; 8],
            weights: [0.0; 8],
            count: 0,
        }
    }

    /// Adds `weight` to a colour set, starting it if it is new.
    pub fn add(&mut self, colors_index: u16, weight: f32) {
        let used = self.count as usize;
        if let Some(slot) = self.indices[..used].iter().position(|&i| i == colors_index) {
            self.weights[slot] += weight;
        } else if used < self.indices.len() {
            self.indices[used] = colors_index;
            self.weights[used] = weight;
            self.count += 1;
        }
    }
}

impl Default for LightGridLookupColorAccum {
    fn default() -> Self {
        Self::new()
    }
}

/// Totals the corners' weights against the chosen light and gathers their
/// colour sets.
pub fn light_grid_lookup_accumulate_corners(
    corners: &[LightGridLookupCorner],
    selected_primary: u8,
    sun_band: u32,
) -> (LightGridLookupWeights, LightGridLookupColorAccum) {
    let mut weights = LightGridLookupWeights::default();
    let mut colors = LightGridLookupColorAccum::new();
    for corner in corners {
        if corner.primary_light == selected_primary {
            weights.matching_primary += corner.weight;
        } else if corner.trace_allows
            && corner_wants_trace(corner.primary_light, selected_primary, sun_band)
        {
            weights.traced_influence += corner.weight;
        }
        weights.total += corner.weight;
        colors.add(corner.colors_index, corner.weight);
    }
    (weights, colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(
        colors_index: u16,
        primary_light: u8,
        weight: f32,
        trace_allows: bool,
    ) -> LightGridLookupCorner {
        LightGridLookupCorner {
            colors_index,
            primary_light,
            weight,
            trace_allows,
        }
    }

    #[test]
    fn remap_keeps_an_in_range_light() {
        assert_eq!(light_grid_lookup_remap_primary(1, 9, true, 2), 1);
        assert_eq!(light_grid_lookup_remap_primary(3, 9, false, 2), 3);
    }

    #[test]
    fn remap_falls_back_for_an_unset_or_out_of_range_light() {
        assert_eq!(light_grid_lookup_remap_primary(0, 9, true, 2), 9);
        assert_eq!(light_grid_lookup_remap_primary(3, 9, true, 2), 9);
    }

    #[test]
    fn remap_counts_the_sun_band_from_the_first_sun_light() {
        assert_eq!(light_grid_lookup_remap_primary(255, 9, true, 2), 2);
        assert_eq!(light_grid_lookup_remap_primary(254, 9, true, 2), 1);
    }

    #[test]
    fn matching_and_traceable_weights_are_kept_apart() {
        let corners = [
            corner(7, 3, 0.5, true),
            corner(7, 4, 0.25, true),
            corner(9, 0, 0.125, false),
        ];
        let (weights, colors) = light_grid_lookup_accumulate_corners(&corners, 3, 254);
        assert_eq!(weights.matching_primary, 0.5);
        assert_eq!(weights.traced_influence, 0.0);
        assert_eq!(weights.total, 0.875);
        assert_eq!(colors.count, 2);
        assert_eq!(&colors.indices[..2], &[7, 9]);
        assert_eq!(&colors.weights[..2], &[0.75, 0.125]);
    }

    #[test]
    fn an_unset_corner_counts_as_traceable_when_allowed() {
        let corners = [corner(1, 3, 0.5, true), corner(2, 0, 0.25, true)];
        let (weights, _) = light_grid_lookup_accumulate_corners(&corners, 3, 254);
        assert_eq!(weights.traced_influence, 0.25);
    }

    #[test]
    fn a_sun_corner_is_traceable_only_when_a_light_was_chosen() {
        let corners = [corner(1, 255, 0.25, true)];
        let (with_light, _) = light_grid_lookup_accumulate_corners(&corners, 3, 254);
        assert_eq!(with_light.traced_influence, 0.25);
        let (without, _) = light_grid_lookup_accumulate_corners(&corners, 0, 254);
        assert_eq!(without.traced_influence, 0.0);
    }

    #[test]
    fn a_ninth_colour_set_is_ignored() {
        let mut accum = LightGridLookupColorAccum::new();
        for i in 0..9u16 {
            accum.add(i, 1.0);
        }
        assert_eq!(accum.count, 8);
        accum.add(3, 1.0);
        assert_eq!(accum.weights[3], 2.0);
    }
}
