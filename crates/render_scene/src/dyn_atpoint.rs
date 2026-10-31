//! Which primary light a dynamic object is lit by, from the map's light culling data.

use asset_world::LevelLightZoneHull;
use bevy::prelude::*;
use lighting_t6::GRID_SAMPLE_VACANT_SUN_BAND;
use lighting_t6::LightVolumeHull;
use lighting_t6::PROP_KEY_LIGHT_LINK_FARTHEST_SQUARED;
use lighting_t6::LampReach;
use lighting_t6::cull_volume_for_main_light;
use lighting_t6::hulls_reject_box;
use lighting_t6::mover_relink_dist_sq;

/// The map's primary lights as the dynamic-object lookups see them: each light's
/// cull volume, how many of the leading lights are suns, and each light's region.
#[derive(Resource, Clone, Debug, Default)]
pub struct LooseAtSpotLookup {
    key_light_hide: Vec<LampReach>,
    daylight_key_light_tally: u32,
    light_zone_hulls: Option<Vec<Vec<LevelLightZoneHull>>>,
}

impl LooseAtSpotLookup {
    pub fn load(
        &mut self,
        key_light_hide: Vec<LampReach>,
        daylight_key_light_tally: u32,
        light_zone_hulls: Option<Vec<Vec<LevelLightZoneHull>>>,
    ) {
        self.key_light_hide = key_light_hide;
        self.daylight_key_light_tally = daylight_key_light_tally;
        self.light_zone_hulls = light_zone_hulls;
    }

    pub fn clear(&mut self) {
        self.load(Vec::new(), 0, None);
    }

    /// The first light after the sun that reaches the box, as a light index, or 0
    /// when none does. Without a box size or region data there is nothing to
    /// test against, and the grid's "empty" primary is returned.
    pub fn light_for_box(&self, centre: [f32; 3], extent: Option<[f32; 3]>) -> u8 {
        let Some(half) = extent else { return GRID_SAMPLE_VACANT_SUN_BAND };
        let Some(regions) = self.light_zone_hulls.as_deref() else { return GRID_SAMPLE_VACANT_SUN_BAND };
        let walked = self.key_light_hide.iter().zip(regions).enumerate().skip(self.first_non_sun());
        for (row, (source, volumes)) in walked {
            if !cull_volume_for_main_light(source, centre, half) && !region_culls_box(volumes, source.origin, centre, half) {
                return row as u8;
            }
        }
        0
    }

    #[must_use]
    pub fn sunlight_primary_flag_index(&self) -> u32 {
        self.daylight_key_light_tally
    }

    #[must_use]
    pub fn main_light_total(&self) -> u32 {
        self.key_light_hide.len() as u32
    }

    /// Reports, for every light after the sun, whether it reaches the box
    /// (`report(index, reaches)`), and returns the nearest one that does (0 when
    /// none). A light with no region data never reaches.
    pub fn bind_moving_entity_main_light(
        &self,
        mid: [f32; 3],
        half: [f32; 3],
        mut report: impl FnMut(u32, bool),
    ) -> u8 {
        let regions = self.light_zone_hulls.as_deref().unwrap_or_default();
        let mut best = None::<(f32, usize)>;
        for (row, source) in self.key_light_hide.iter().enumerate().skip(self.first_non_sun()) {
            let reaches = !cull_volume_for_main_light(source, mid, half)
                && regions.get(row).is_some_and(|volumes| !region_culls_box(volumes, source.origin, mid, half));
            report(row as u32, reaches);
            if !reaches {
                continue;
            }
            let gap = mover_relink_dist_sq(source.origin, mid);
            let closest_so_far = best.map_or(PROP_KEY_LIGHT_LINK_FARTHEST_SQUARED, |(kept, _)| kept);
            if gap < closest_so_far {
                best = Some((gap, row));
            }
        }
        best.map_or(0, |(_, row)| row as u8)
    }

    /// Lights are ordered suns first; the sun count plus one is where the walk starts.
    fn first_non_sun(&self) -> usize {
        self.daylight_key_light_tally.saturating_add(1) as usize
    }
}

/// Older spellings other crates still call; the rename sweep moves them over.
impl LooseAtSpotLookup {

    pub fn fallback(&self, mid: [f32; 3], box_half: Option<[f32; 3]>) -> u8 {
        self.light_for_box(mid, box_half)
    }
}

fn region_culls_box(volumes: &[LevelLightZoneHull], lit_from: [f32; 3], centre: [f32; 3], extent: [f32; 3]) -> bool {
    let hulls = volumes.iter().map(|zone| LightVolumeHull {
        bounds_center_point: zone.bounds_center_point,
        bounds_half_extent: zone.bounds_half_extent,
        axes: &zone.axes,
    });
    hulls_reject_box(hulls, lit_from, centre, extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: [f32; 3] = [1.0; 3];

    fn omni(origin: [f32; 3], radius: f32) -> LampReach {
        LampReach {
            kind: 0,
            origin,
            direction: Vec3::Z.to_array(),
            radius,
            widened_half_view_cosine: -1.0,
        }
    }

    fn far(x: f32) -> [f32; 3] {
        Vec3::new(x, 0.0, 0.0).to_array()
    }

    /// With no suns the walk starts at light 1: light 0 is never tested. Lights 1
    /// and 2 sit at x = 0 and x = 100.
    fn three_lights() -> LooseAtSpotLookup {
        let mut lights = LooseAtSpotLookup::default();
        lights.load(vec![omni(far(0.0), 1.0), omni(far(0.0), 50.0), omni(far(100.0), 50.0)], 0, Some(vec![vec![]; 3]));
        lights
    }

    #[test]
    fn without_a_box_or_regions_the_grid_primary_is_empty() {
        assert_eq!(three_lights().light_for_box(far(0.0), None), GRID_SAMPLE_VACANT_SUN_BAND);
        let mut no_regions = three_lights();
        no_regions.load(vec![omni(far(0.0), 50.0)], 0, None);
        assert_eq!(no_regions.light_for_box(far(0.0), Some(HALF)), GRID_SAMPLE_VACANT_SUN_BAND);
    }

    #[test]
    fn the_walk_skips_the_leading_lights_and_takes_the_first_that_reaches() {
        let lights = three_lights();
        assert_eq!(lights.light_for_box(far(0.0), Some(HALF)), 1, "light 0 would reach but is not tested");
        assert_eq!(lights.light_for_box(far(100.0), Some(HALF)), 2);
        assert_eq!(lights.light_for_box(far(1000.0), Some(HALF)), 0);
    }

    #[test]
    fn each_sun_pushes_the_start_of_the_walk_back_one_light() {
        let mut lights = three_lights();
        lights.load(vec![omni(far(0.0), 50.0), omni(far(0.0), 50.0), omni(far(100.0), 50.0)], 1, Some(vec![vec![]; 3]));
        assert_eq!(lights.light_for_box(far(0.0), Some(HALF)), 0, "only light 2 is tested, and it is too far");
        assert_eq!(lights.light_for_box(far(100.0), Some(HALF)), 2);
    }

    #[test]
    fn linking_reports_every_walked_light_and_returns_the_nearest_reaching_one() {
        let lights = three_lights();
        let mut seen = Vec::new();
        let nearest = lights.bind_moving_entity_main_light(far(60.0), HALF, |row, lit| seen.push((row, lit)));
        assert_eq!(seen, vec![(1, false), (2, true)]);
        assert_eq!(nearest, 2, "light 1 is 60 away, past its radius of 50");
        seen.clear();
        assert_eq!(lights.bind_moving_entity_main_light(far(1000.0), HALF, |row, lit| seen.push((row, lit))), 0);
        assert_eq!(seen, vec![(1, false), (2, false)]);
    }

    #[test]
    fn a_light_without_region_data_never_links() {
        let mut lights = LooseAtSpotLookup::default();
        lights.load(vec![omni(far(0.0), 1.0), omni(far(0.0), 50.0)], 0, None);
        let mut seen = Vec::new();
        assert_eq!(lights.bind_moving_entity_main_light(far(0.0), HALF, |row, lit| seen.push((row, lit))), 0);
        assert_eq!(seen, vec![(1, false)]);
    }

    #[test]
    fn counts_come_from_the_loaded_data_and_clear_empties_them() {
        let mut lights = three_lights();
        assert_eq!((lights.sunlight_primary_flag_index(), lights.main_light_total()), (0, 3));
        lights.clear();
        assert_eq!((lights.sunlight_primary_flag_index(), lights.main_light_total()), (0, 0));
    }
}
