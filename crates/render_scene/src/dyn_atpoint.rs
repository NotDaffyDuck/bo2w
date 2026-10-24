//! Which primary light a dynamic object is lit by, from the map's light culling data.

use asset_world::WorldLightRegionHull;
use bevy::prelude::*;
use lighting_t6::{
    DYN_ENT_PRIMARY_LIGHT_LINK_DIST2_INIT, LIGHT_GRID_ATPOINT_EMPTY_PRIMARY, LightRegionHull,
    PrimaryLightCull, cull_box_from_primary_light, dyn_ent_primary_light_link_dist2,
    light_region_culls_box,
};

/// The map's primary lights as the dynamic-object lookups see them: each light's
/// cull volume, how many of the leading lights are suns, and each light's region.
#[derive(Resource, Clone, Debug, Default)]
pub struct DynAtPointLookup {
    primary_light_cull: Vec<PrimaryLightCull>,
    sun_primary_light_count: u32,
    light_region_hulls: Option<Vec<Vec<WorldLightRegionHull>>>,
}

impl DynAtPointLookup {
    pub fn replace(
        &mut self,
        primary_light_cull: Vec<PrimaryLightCull>,
        sun_primary_light_count: u32,
        light_region_hulls: Option<Vec<Vec<WorldLightRegionHull>>>,
    ) {
        *self = Self { primary_light_cull, sun_primary_light_count, light_region_hulls };
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The first light after the sun that reaches the box, as a light index, or 0
    /// when none does. Without a box size or region data there is nothing to
    /// test against, and the grid's "empty" primary is returned.
    pub fn fallback(&self, mid: [f32; 3], box_half: Option<[f32; 3]>) -> u8 {
        let (Some(half), Some(regions)) = (box_half, &self.light_region_hulls) else {
            return LIGHT_GRID_ATPOINT_EMPTY_PRIMARY;
        };
        let first = self.first_non_sun();
        self.primary_light_cull
            .iter()
            .zip(regions)
            .enumerate()
            .skip(first)
            .find(|(_, (light, hulls))| {
                !cull_box_from_primary_light(light, mid, half)
                    && !region_culls_box(hulls, light.origin, mid, half)
            })
            .map_or(0, |(index, _)| index as u8)
    }

    #[must_use]
    pub fn sun_primary(&self) -> u32 {
        self.sun_primary_light_count
    }

    #[must_use]
    pub fn primary_count(&self) -> u32 {
        self.primary_light_cull.len() as u32
    }

    /// Reports, for every light after the sun, whether it reaches the box
    /// (`on_bit(index, reaches)`), and returns the nearest one that does (0 when
    /// none). A light with no region data never reaches.
    pub fn link_dyn_ent_primary(
        &self,
        mid: [f32; 3],
        half: [f32; 3],
        mut on_bit: impl FnMut(u32, bool),
    ) -> u8 {
        let mut nearest = 0;
        let mut nearest_dist2 = DYN_ENT_PRIMARY_LIGHT_LINK_DIST2_INIT;
        for (index, light) in self.primary_light_cull.iter().enumerate().skip(self.first_non_sun()) {
            let reaches = !cull_box_from_primary_light(light, mid, half)
                && self
                    .light_region_hulls
                    .as_ref()
                    .and_then(|regions| regions.get(index))
                    .is_some_and(|hulls| !region_culls_box(hulls, light.origin, mid, half));
            on_bit(index as u32, reaches);
            if reaches {
                let dist2 = dyn_ent_primary_light_link_dist2(light.origin, mid);
                if dist2 < nearest_dist2 {
                    nearest_dist2 = dist2;
                    nearest = index as u8;
                }
            }
        }
        nearest
    }

    /// Lights are ordered suns first; the sun count plus one is where the walk starts.
    fn first_non_sun(&self) -> usize {
        self.sun_primary_light_count.saturating_add(1) as usize
    }
}

fn region_culls_box(
    hulls: &[WorldLightRegionHull],
    light_origin: [f32; 3],
    mid: [f32; 3],
    half: [f32; 3],
) -> bool {
    let hulls = hulls
        .iter()
        .map(|h| LightRegionHull { kdop_mid: h.kdop_mid, kdop_half: h.kdop_half, axes: &h.axes });
    light_region_culls_box(hulls, light_origin, mid, half)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: [f32; 3] = [1.0; 3];

    fn omni(origin: [f32; 3], radius: f32) -> PrimaryLightCull {
        PrimaryLightCull {
            light_type: 0,
            origin,
            direction: [0.0, 0.0, 1.0],
            radius,
            cos_half_fov_expanded: -1.0,
        }
    }

    /// With no suns the walk starts at light 1: light 0 is never tested. Lights 1
    /// and 2 sit at x = 0 and x = 100.
    fn lookup() -> DynAtPointLookup {
        let mut lookup = DynAtPointLookup::default();
        lookup.replace(
            vec![omni([0.0; 3], 1.0), omni([0.0; 3], 50.0), omni([100.0, 0.0, 0.0], 50.0)],
            0,
            Some(vec![vec![]; 3]),
        );
        lookup
    }

    #[test]
    fn without_a_box_or_regions_the_grid_primary_is_empty() {
        assert_eq!(lookup().fallback([0.0; 3], None), LIGHT_GRID_ATPOINT_EMPTY_PRIMARY);
        let mut no_regions = lookup();
        no_regions.replace(vec![omni([0.0; 3], 50.0)], 0, None);
        assert_eq!(no_regions.fallback([0.0; 3], Some(HALF)), LIGHT_GRID_ATPOINT_EMPTY_PRIMARY);
    }

    #[test]
    fn the_fallback_skips_the_leading_lights_and_takes_the_first_that_reaches() {
        let lookup = lookup();
        assert_eq!(lookup.fallback([0.0; 3], Some(HALF)), 1, "light 0 would reach but is not tested");
        assert_eq!(lookup.fallback([100.0, 0.0, 0.0], Some(HALF)), 2);
        assert_eq!(lookup.fallback([1000.0, 0.0, 0.0], Some(HALF)), 0);
    }

    #[test]
    fn each_sun_pushes_the_start_of_the_walk_back_one_light() {
        let mut lookup = lookup();
        lookup.replace(
            vec![omni([0.0; 3], 50.0), omni([0.0; 3], 50.0), omni([100.0, 0.0, 0.0], 50.0)],
            1,
            Some(vec![vec![]; 3]),
        );
        assert_eq!(lookup.fallback([0.0; 3], Some(HALF)), 0, "only light 2 is tested, and it is too far");
        assert_eq!(lookup.fallback([100.0, 0.0, 0.0], Some(HALF)), 2);
    }

    #[test]
    fn linking_reports_every_walked_light_and_returns_the_nearest_reaching_one() {
        let lookup = lookup();
        let mut seen = Vec::new();
        let nearest = lookup.link_dyn_ent_primary([60.0, 0.0, 0.0], HALF, |i, set| seen.push((i, set)));
        assert_eq!(seen, vec![(1, false), (2, true)]);
        assert_eq!(nearest, 2, "light 1 is 60 away, past its radius of 50");
        seen.clear();
        assert_eq!(lookup.link_dyn_ent_primary([1000.0, 0.0, 0.0], HALF, |i, s| seen.push((i, s))), 0);
        assert_eq!(seen, vec![(1, false), (2, false)]);
    }

    #[test]
    fn a_light_without_region_data_never_links() {
        let mut lookup = DynAtPointLookup::default();
        lookup.replace(vec![omni([0.0; 3], 1.0), omni([0.0; 3], 50.0)], 0, None);
        let mut seen = Vec::new();
        assert_eq!(lookup.link_dyn_ent_primary([0.0; 3], HALF, |i, s| seen.push((i, s))), 0);
        assert_eq!(seen, vec![(1, false)]);
    }

    #[test]
    fn counts_come_from_the_replaced_data_and_clear_empties_them() {
        let mut lookup = lookup();
        assert_eq!((lookup.sun_primary(), lookup.primary_count()), (0, 3));
        lookup.clear();
        assert_eq!((lookup.sun_primary(), lookup.primary_count()), (0, 0));
    }
}
