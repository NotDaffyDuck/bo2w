//! The lights a world places, the definitions they point at, and the images
//! those definitions resolve to.

use asset_material::MaterialDefinitions;

use crate::{AssetCategoryScope, AssetPointer};

/// The sun as the fallback draw lights with.
#[derive(Clone, Copy, Debug)]
pub struct LevelSunlamp {
    pub direction: [f32; 3],
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
}

/// A light the world places on its own.
#[derive(Clone, Debug)]
pub struct LevelDominantLight {
    pub is_daylight: bool,
    pub light_kind: u8,
    pub falloff_power: u8,
    pub rgba: [f32; 3],
    pub heading: [f32; 3],
    pub spot: [f32; 3],
    pub reach: f32,
    pub outer_cone_cosine: f32,
    pub inner_cone_cosine: f32,
    pub widened_half_view_cosine: f32,
    /// The definition this light was authored with.
    pub definition_name: Option<String>,
    pub falloff_map_width: Option<u16>,
    pub lightmap_first_texel: i32,
    pub falloff_texture: Option<usize>,
    pub falloff_lookup: u8,
    pub classic_attenuation: Option<[f32; 4]>,
    pub classic_falloff: Option<[f32; 4]>,
    pub classic_a_ab_b: Option<[f32; 4]>,
    pub classic_angle: Option<[f32; 4]>,
    pub classic_cookie0: Option<[f32; 4]>,
    pub classic_cookie1: Option<[f32; 4]>,
    pub classic_cookie2: Option<[f32; 4]>,
    pub classic_diffuse_color: Option<[f32; 4]>,
    pub classic_specular_color: Option<[f32; 4]>,
}

impl LevelDominantLight {
    /// What the light culler needs to know about this light.
    pub fn cull_input(&self) -> lighting_t6::LampReach {
        let Self { light_kind: kind, spot: at, heading: heading, reach: reach, widened_half_view_cosine: cone, .. } = *self;
        lighting_t6::LampReach {
            widened_half_view_cosine: cone,
            radius: reach,
            direction: heading,
            origin: at,
            kind,
        }
    }

    /// The light as the shader constants are packed.
    pub fn gfx_light_pack(&self) -> lighting_t6::LampRecord {
        let Self {
            rgba: rgb,
            heading: heading,
            light_kind: kind,
            spot: at,
            reach: reach,
            falloff_power: power,
            inner_cone_cosine: inner,
            outer_cone_cosine: outer,
            falloff_map_width: width,
            lightmap_first_texel: start,
            ..
        } = *self;
        lighting_t6::LampRecord {
            lightmap_first: start,
            falloff_map_width: width,
            falloff: power,
            inner_cone_cosine: inner,
            outer_cone_cosine: outer,
            radius: reach,
            origin: at,
            direction: heading,
            color: rgb,
            kind,
        }
    }

    /// Points this light at what its definition resolves to.
    fn soak_up(&mut self, resolved: ResolvedLightDef) {
        let ResolvedLightDef {
            falloff_texture: image,
            falloff_lookup: sampler,
            falloff_map_width: width,
            lightmap_first_texel: start,
        } = resolved;
        (self.falloff_texture, self.falloff_lookup) = (image, sampler);
        (self.falloff_map_width, self.lightmap_first_texel) = (width, start);
    }
}

/// A light definition as the zone names it: its falloff image by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotLightDef {
    pub namespace: AssetCategoryScope,
    pub name: AssetPointer,
    pub falloff_image: Option<String>,
    pub recorded_width: Option<u16>,
    pub falloff_lookup: u8,
    pub lightmap_first_texel: i32,
}

/// A light definition once its falloff image has been looked up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedLightDef {
    pub falloff_texture: Option<usize>,
    pub falloff_lookup: u8,
    pub falloff_map_width: Option<u16>,
    pub lightmap_first_texel: i32,
}

impl ResolvedLightDef {
    /// Looks the definition's falloff image up. Its width is the decoded
    /// image's, else the image row's, else the width the zone recorded; a
    /// width of zero counts as none.
    pub fn of(def: &SnapshotLightDef, catalog: &MaterialDefinitions) -> Self {
        let image_row = match def.falloff_image.as_deref() {
            Some(key) => catalog.image_index_by_key(def.namespace, key),
            None => None,
        };
        let from_image = image_row
            .and_then(|slot| catalog.images.get(slot))
            .and_then(|entry| {
                match entry
                    .decoded
                    .as_ref()
                    .and_then(|pixels| u16::try_from(pixels.texture_descriptor.size.width).ok())
                {
                    Some(w) => Some(w),
                    None => Some(entry.width),
                }
            });
        let usable = |w: &u16| *w != 0;
        let falloff = from_image
            .filter(usable)
            .or_else(|| def.recorded_width.filter(usable));
        let (sampler, start) = (def.falloff_lookup, def.lightmap_first_texel);
        Self {
            lightmap_first_texel: start,
            falloff_map_width: falloff,
            falloff_lookup: sampler,
            falloff_texture: image_row,
        }
    }
}

/// The definition called `name`: the map's before the common set's, and a
/// real entry before a placeholder, the latest first.
fn find_light_def<'d>(
    name: &str,
    map_defs: &'d [SnapshotLightDef],
    common_defs: &'d [SnapshotLightDef],
) -> Option<&'d SnapshotLightDef> {
    let bare = Some(AssetPointer::bare_name(name)).filter(|b| !b.is_empty())?;
    let scan = |pool: &'d [SnapshotLightDef]| {
        let spells_same = || pool.iter().rev().filter(|d| d.name.as_str() == bare);
        match spells_same().find(|d| d.name.is_genuine()) {
            Some(hit) => Some(hit),
            None => spells_same().next(),
        }
    };
    scan(map_defs).or_else(|| scan(common_defs))
}

/// Gives every primary light the falloff of the definition it names.
pub fn resolve_primary_light_attenuation(
    draw: &mut crate::LevelDrawCall,
    materials: &MaterialDefinitions,
    common_defs: &[SnapshotLightDef],
) {
    let own_defs = &draw.light_defs;
    for light in draw.key_lights.iter_mut() {
        let Some(key) = light.definition_name.as_deref() else {
            continue;
        };
        if let Some(def) = find_light_def(key, own_defs, common_defs) {
            light.soak_up(ResolvedLightDef::of(def, materials));
        }
    }
}

/// The convex region a light reaches in one cell.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelLightZoneHull {
    pub bounds_center_point: [f32; 9],
    pub bounds_half_extent: [f32; 9],
    pub axes: Vec<lighting_t6::LightVolumeSpan>,
}

/// What a shadow-casting light touches: surface and static-model indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelShadowMesh {
    pub surface_total: Vec<u16>,
    pub fixed_props: Vec<u16>,
}

/// A reflection probe: its image row and where it sits.
#[derive(Clone, Debug)]
pub struct LevelEnvReflectionCapture {
    pub image: Option<usize>,
    pub spot: [f32; 3],
}

/// The image the outdoor-light lookup uses, and where it was found: the
/// world's own pointer first, then the shared `$outdoor` image.
pub fn locate_exterior_texture(
    own_image_key: Option<&str>,
    level_scope: AssetCategoryScope,
    catalog: &MaterialDefinitions,
) -> (Option<usize>, &'static str) {
    let by_key = |key: &str| catalog.image_index_by_key(level_scope, key);
    let world_image = own_image_key.and_then(by_key);
    if world_image.is_some() {
        return (world_image, "gfxworld+412");
    }
    let shared = by_key("$outdoor");
    let origin = if shared.is_some() { "$outdoor" } else { "missing" };
    (shared, origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, sampler: u8) -> SnapshotLightDef {
        SnapshotLightDef {
            name: AssetPointer::decode(name),
            falloff_lookup: sampler,
            ..SnapshotLightDef::default()
        }
    }

    #[test]
    fn the_map_is_searched_before_the_common_set() {
        let map = [def("lamp", 1)];
        let common = [def("lamp", 2), def("torch", 3)];
        assert_eq!(find_light_def("lamp", &map, &common).unwrap().falloff_lookup, 1);
        assert_eq!(find_light_def("torch", &map, &common).unwrap().falloff_lookup, 3);
        assert!(find_light_def("missing", &map, &common).is_none());
        assert!(find_light_def("", &map, &common).is_none());
    }

    #[test]
    fn the_latest_definition_of_a_name_wins() {
        let map = [def("lamp", 1), def("lamp", 5)];
        assert_eq!(find_light_def("lamp", &map, &[]).unwrap().falloff_lookup, 5);
    }

    #[test]
    fn a_width_of_zero_falls_back_to_the_recorded_one() {
        let catalog = MaterialDefinitions::default();
        let mut recorded = def("lamp", 4);
        recorded.recorded_width = Some(64);
        recorded.lightmap_first_texel = 9;
        let resolved = ResolvedLightDef::of(&recorded, &catalog);
        assert_eq!(resolved.falloff_texture, None);
        assert_eq!(resolved.falloff_map_width, Some(64));
        assert_eq!(resolved.falloff_lookup, 4);
        assert_eq!(resolved.lightmap_first_texel, 9);

        recorded.recorded_width = Some(0);
        assert_eq!(ResolvedLightDef::of(&recorded, &catalog).falloff_map_width, None);
    }

    #[test]
    fn no_outdoor_image_is_reported_as_missing() {
        let catalog = MaterialDefinitions::default();
        let found = locate_exterior_texture(Some("outdoor_img"), AssetCategoryScope::T6, &catalog);
        assert_eq!(found, (None, "missing"));
    }
}
