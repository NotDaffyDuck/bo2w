//! Tallying the corners a lookup kept: how much weight agrees with the chosen
//! light, how much could be traced, and the weight each colour set got.

use crate::corner_pick::sun_band_light;

/// Turns the light index the pick settled on into the one a lookup reports.
/// Below the sun band a light the header says is out of range falls back; at
/// or above it the index counts from the sun's first.
pub fn grid_remap_main_light(
    picked: u8,
    fallback_light: u8,
    carries_light_volumes: bool,
    sun_leads_primaries: u32,
) -> u8 {
    let in_sun_band = u32::from(picked) >= sun_band_light(sun_leads_primaries);
    if in_sun_band {
        // The band's top index (0xff) is the sun's first light; byte arithmetic wraps.
        let past_top = picked.wrapping_add(1);
        return past_top.wrapping_add(sun_leads_primaries as u8);
    }
    let unusable = picked == 0 || u32::from(picked) > sun_leads_primaries;
    match (carries_light_volumes, unusable) {
        (true, true) => fallback_light,
        _ => picked,
    }
}

/// Weight totals over the corners kept.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridProbeBlendFactors {
    /// Weight of corners whose light is the chosen one.
    pub same_band_ratio: f32,
    /// Weight of other corners a trace could still count.
    pub ray_checked_weight: f32,
    pub sum: f32,
}

/// A corner that survived the pick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridProbeCornerSample {
    pub vertex_color_slot: u16,
    pub dominant_light: u8,
    pub share: f32,
    pub ray_check_permits: bool,
}

impl GridProbeCornerSample {
    /// Whether this corner, lit by another light than `selected`, could still
    /// be traced to it: it has no light, or it is a sun light and some light
    /// was chosen. The corner's own trace bits must allow it too.
    fn traceable_toward(&self, selected: u8, sun_band: u32) -> bool {
        let unlit = self.dominant_light == 0;
        let sun_with_choice = selected != 0 && u32::from(self.dominant_light) >= sun_band;
        self.ray_check_permits && (unlit || sun_with_choice)
    }
}

/// The distinct colour sets among the corners and the weight each carries.
#[derive(Clone, Debug, PartialEq)]
pub struct GridLookupTintAccumulator {
    pub picks: [u16; 8],
    pub shares: [f32; 8],
    pub count: u32,
}

impl GridLookupTintAccumulator {
    pub const fn new() -> Self {
        GridLookupTintAccumulator { count: 0, shares: [0.0; 8], picks: [0; 8] }
    }

    /// Adds `weight` to a colour set, starting it if it is new. With eight
    /// sets already started a new one is dropped.
    pub fn blend_in(&mut self, vertex_color_slot: u16, share: f32) {
        let started = self.count as usize;
        let slot = (0..started).find(|&n| self.picks[n] == vertex_color_slot);
        match slot {
            Some(n) => self.shares[n] += share,
            None if started < 8 => {
                (self.picks[started], self.shares[started]) = (vertex_color_slot, share);
                self.count += 1;
            }
            None => {}
        }
    }
}

impl Default for GridLookupTintAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Totals the corners' weights against the chosen light and gathers their
/// colour sets.
pub fn grid_sum_corner_contributions(
    corners: &[GridProbeCornerSample],
    chosen_sun_band: u8,
    sun_floor: u32,
) -> (GridProbeBlendFactors, GridLookupTintAccumulator) {
    let mut sums = GridProbeBlendFactors::default();
    let mut sets = GridLookupTintAccumulator::default();
    for c in corners {
        let bucket = if c.dominant_light == chosen_sun_band {
            Some(&mut sums.same_band_ratio)
        } else if c.traceable_toward(chosen_sun_band, sun_floor) {
            Some(&mut sums.ray_checked_weight)
        } else {
            None
        };
        if let Some(sum) = bucket {
            *sum += c.share;
        }
        sums.sum += c.share;
        sets.blend_in(c.vertex_color_slot, c.share);
    }
    (sums, sets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(vertex_color_slot: u16, dominant_light: u8, part: f32, ray_check_permits: bool) -> GridProbeCornerSample {
        GridProbeCornerSample { vertex_color_slot, dominant_light, share: part, ray_check_permits }
    }

    fn remap(picked: u8, regions: bool) -> u8 {
        grid_remap_main_light(picked, 9, regions, 2)
    }

    #[test]
    fn remap_keeps_an_in_range_light() {
        assert_eq!((remap(1, true), remap(3, false)), (1, 3));
    }

    #[test]
    fn remap_falls_back_for_an_unset_or_out_of_range_light() {
        assert_eq!((remap(0, true), remap(3, true)), (9, 9));
    }

    #[test]
    fn remap_counts_the_sun_band_from_the_first_sun_light() {
        assert_eq!((remap(255, true), remap(254, true)), (2, 1));
    }

    #[test]
    fn matching_and_traceable_weights_are_kept_apart() {
        let corners = [corner(7, 3, 0.5, true), corner(7, 4, 0.25, true), corner(9, 0, 0.125, false)];
        let (sums, sets) = grid_sum_corner_contributions(&corners, 3, 254);
        assert_eq!((sums.same_band_ratio, sums.ray_checked_weight, sums.sum), (0.5, 0.0, 0.875));
        assert_eq!(sets.count, 2);
        assert_eq!((&sets.picks[..2], &sets.shares[..2]), (&[7, 9][..], &[0.75, 0.125][..]));
    }

    #[test]
    fn an_unset_corner_counts_as_traceable_when_allowed() {
        let corners = [corner(1, 3, 0.5, true), corner(2, 0, 0.25, true)];
        assert_eq!(grid_sum_corner_contributions(&corners, 3, 254).0.ray_checked_weight, 0.25);
    }

    #[test]
    fn a_sun_corner_is_traceable_only_when_a_light_was_chosen() {
        let corners = [corner(1, 255, 0.25, true)];
        let traced = |selected| grid_sum_corner_contributions(&corners, selected, 254).0.ray_checked_weight;
        assert_eq!((traced(3), traced(0)), (0.25, 0.0));
    }

    #[test]
    fn a_ninth_colour_set_is_ignored() {
        let mut accum = GridLookupTintAccumulator::new();
        (0..9u16).for_each(|i| accum.blend_in(i, 1.0));
        assert_eq!(accum.count, 8);
        accum.blend_in(3, 1.0);
        assert_eq!(accum.shares[3], 2.0);
    }
}
