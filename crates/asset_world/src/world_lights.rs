//! The lights a world places, the definitions they point at, and the images
//! those definitions resolve to.

use asset_material::MaterialDefinitions;

use crate::{AssetNamespace, AssetRef};

/// The sun as the fallback draw lights with.
#[derive(Clone, Copy, Debug)]
pub struct WorldSunLight {
    pub direction: [f32; 3],
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
}

/// A light the world places on its own.
#[derive(Clone, Debug)]
pub struct WorldPrimaryLight {
    pub is_sun: bool,
    pub light_type: u8,
    pub can_cast_shadow: bool,
    pub exponent: u8,
    pub color: [f32; 3],
    pub direction: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub cos_half_fov_expanded: f32,
    /// The definition this light was authored with.
    pub def_name: Option<String>,
    pub falloff_image_width: Option<u16>,
    pub lmap_lookup_start: i32,
    pub attenuation_image: Option<usize>,
    pub attenuation_sampler: u8,
    pub t5_attenuation: Option<[f32; 4]>,
    pub t5_falloff: Option<[f32; 4]>,
    pub t5_a_ab_b: Option<[f32; 4]>,
    pub t5_angle: Option<[f32; 4]>,
    pub t5_cookie0: Option<[f32; 4]>,
    pub t5_cookie1: Option<[f32; 4]>,
    pub t5_cookie2: Option<[f32; 4]>,
    pub t5_diffuse_color: Option<[f32; 4]>,
    pub t5_specular_color: Option<[f32; 4]>,
}

impl WorldPrimaryLight {
    /// What the light culler needs to know about this light.
    pub fn cull_input(&self) -> lighting_t6::PrimaryLightCull {
        lighting_t6::PrimaryLightCull {
            light_type: self.light_type,
            origin: self.origin,
            direction: self.direction,
            radius: self.radius,
            cos_half_fov_expanded: self.cos_half_fov_expanded,
        }
    }

    /// The light as the shader constants are packed.
    pub fn gfx_light_pack(&self) -> lighting_t6::GfxLightPack {
        lighting_t6::GfxLightPack {
            light_type: self.light_type,
            color: self.color,
            direction: self.direction,
            origin: self.origin,
            radius: self.radius,
            cos_outer: self.cos_outer,
            cos_inner: self.cos_inner,
            exponent: self.exponent,
            falloff_image_width: self.falloff_image_width,
            lmap_lookup_start: self.lmap_lookup_start,
        }
    }

    /// Points this light at what its definition resolves to.
    fn take(&mut self, resolved: ResolvedLightDef) {
        self.attenuation_image = resolved.attenuation_image;
        self.attenuation_sampler = resolved.attenuation_sampler;
        self.falloff_image_width = resolved.falloff_image_width;
        self.lmap_lookup_start = resolved.lmap_lookup_start;
    }
}

/// A light definition as the zone names it: its falloff image by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapturedLightDef {
    pub namespace: AssetNamespace,
    pub name: AssetRef,
    pub attenuation_image_name: Option<String>,
    pub attenuation_width: Option<u16>,
    pub attenuation_sampler: u8,
    pub lmap_lookup_start: i32,
}

/// A light definition once its falloff image has been looked up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedLightDef {
    pub attenuation_image: Option<usize>,
    pub attenuation_sampler: u8,
    pub falloff_image_width: Option<u16>,
    pub lmap_lookup_start: i32,
}

impl ResolvedLightDef {
    /// Looks the definition's falloff image up. Its width is the decoded
    /// image's, else the image row's, else the width the zone recorded; a
    /// width of zero counts as none.
    pub fn of(def: &CapturedLightDef, catalog: &MaterialDefinitions) -> Self {
        let image = def
            .attenuation_image_name
            .as_deref()
            .and_then(|name| catalog.image_index_by_key(def.namespace, name));
        let measured = image.and_then(|index| catalog.images.get(index)).and_then(|row| {
            let decoded = row
                .decoded
                .as_ref()
                .and_then(|image| u16::try_from(image.texture_descriptor.size.width).ok());
            decoded.or(Some(row.width))
        });
        let non_zero = |width: &u16| *width != 0;
        Self {
            attenuation_image: image,
            attenuation_sampler: def.attenuation_sampler,
            falloff_image_width: measured
                .filter(non_zero)
                .or(def.attenuation_width.filter(non_zero)),
            lmap_lookup_start: def.lmap_lookup_start,
        }
    }
}

/// The definition called `name`: the map's before the common set's, and a
/// real entry before a placeholder, the latest first.
fn find_light_def<'a>(
    name: &str,
    map_defs: &'a [CapturedLightDef],
    common_defs: &'a [CapturedLightDef],
) -> Option<&'a CapturedLightDef> {
    let wanted = AssetRef::bare_name(name);
    if wanted.is_empty() {
        return None;
    }
    let search = |defs: &'a [CapturedLightDef]| {
        let called = || defs.iter().rev().filter(|def| def.name.as_str() == wanted);
        called().find(|def| def.name.is_real()).or_else(|| called().next())
    };
    search(map_defs).or_else(|| search(common_defs))
}

/// Gives every primary light the falloff of the definition it names.
pub fn resolve_primary_light_attenuation(
    draw: &mut crate::WorldDraw,
    materials: &MaterialDefinitions,
    common_defs: &[CapturedLightDef],
) {
    let map_defs = &draw.light_defs;
    for light in &mut draw.primary_lights {
        let found = light
            .def_name
            .as_deref()
            .and_then(|name| find_light_def(name, map_defs, common_defs));
        if let Some(def) = found {
            light.take(ResolvedLightDef::of(def, materials));
        }
    }
}

/// The convex region a light reaches in one cell.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldLightRegionHull {
    pub kdop_mid: [f32; 9],
    pub kdop_half: [f32; 9],
    pub axes: Vec<lighting_t6::LightRegionAxis>,
}

/// What a shadow-casting light touches: surface and static-model indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldShadowGeometry {
    pub surfaces: Vec<u16>,
    pub smodels: Vec<u16>,
}

/// A reflection probe: its image row and where it sits.
#[derive(Clone, Debug)]
pub struct WorldReflectionProbe {
    pub image: Option<usize>,
    pub origin: [f32; 3],
}

/// The image the outdoor-light lookup uses, and where it was found: the
/// world's own pointer first, then the shared `$outdoor` image.
pub fn resolve_outdoor_image(
    world_ptr_name: Option<&str>,
    map_namespace: AssetNamespace,
    catalog: &MaterialDefinitions,
) -> (Option<usize>, &'static str) {
    let lookup = |name: &str| catalog.image_index_by_key(map_namespace, name);
    if let Some(index) = world_ptr_name.and_then(lookup) {
        return (Some(index), "gfxworld+412");
    }
    match lookup("$outdoor") {
        Some(index) => (Some(index), "$outdoor"),
        None => (None, "missing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, sampler: u8) -> CapturedLightDef {
        CapturedLightDef {
            name: AssetRef::decode(name),
            attenuation_sampler: sampler,
            ..CapturedLightDef::default()
        }
    }

    #[test]
    fn the_map_is_searched_before_the_common_set() {
        let map = [def("lamp", 1)];
        let common = [def("lamp", 2), def("torch", 3)];
        assert_eq!(find_light_def("lamp", &map, &common).unwrap().attenuation_sampler, 1);
        assert_eq!(find_light_def("torch", &map, &common).unwrap().attenuation_sampler, 3);
        assert!(find_light_def("missing", &map, &common).is_none());
        assert!(find_light_def("", &map, &common).is_none());
    }

    #[test]
    fn the_latest_definition_of_a_name_wins() {
        let map = [def("lamp", 1), def("lamp", 5)];
        assert_eq!(find_light_def("lamp", &map, &[]).unwrap().attenuation_sampler, 5);
    }

    #[test]
    fn a_width_of_zero_falls_back_to_the_recorded_one() {
        let catalog = MaterialDefinitions::default();
        let mut recorded = def("lamp", 4);
        recorded.attenuation_width = Some(64);
        recorded.lmap_lookup_start = 9;
        let resolved = ResolvedLightDef::of(&recorded, &catalog);
        assert_eq!(resolved.attenuation_image, None);
        assert_eq!(resolved.falloff_image_width, Some(64));
        assert_eq!(resolved.attenuation_sampler, 4);
        assert_eq!(resolved.lmap_lookup_start, 9);

        recorded.attenuation_width = Some(0);
        assert_eq!(ResolvedLightDef::of(&recorded, &catalog).falloff_image_width, None);
    }

    #[test]
    fn no_outdoor_image_is_reported_as_missing() {
        let catalog = MaterialDefinitions::default();
        let found = resolve_outdoor_image(Some("outdoor_img"), AssetNamespace::T6, &catalog);
        assert_eq!(found, (None, "missing"));
    }
}
