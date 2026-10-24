//! Choosing the lit technique for a draw: which variant of the lit shader a
//! surface needs depends on the light that falls on it (sun, spot, or none),
//! whether that light casts a shadow, and whether the fog is the per-pixel
//! kind.

use bevy::prelude::Resource;

use super::setup::TechType;

/// Light type of every primary light the map defines, by light index.
#[derive(Resource, Clone, Debug, Default)]
pub struct MapPrimaryLightTypes {
    pub types: Vec<u8>,
}

/// Whether the map's fog runs in the per-pixel draw method.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawMethodDfog(pub bool);

/// Whether a sun shadow map exists this frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SunShadowMapPresent(pub bool);

/// Indices of the spot lights that have a shadow map this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct SpotShadowMapLights(pub Vec<u8>);

/// Put a primary light index into a packed draw-surface key, leaving every
/// other field alone.
pub fn with_scene_light_index(packed: u64, scene_light_index: u8) -> u64 {
    let key = dpvs_t6::GfxDrawSurf { packed };
    dpvs_t6::with_primary_light_index(key, scene_light_index).packed
}

/// Debug switch: treat every lit surface as sun-lit.
fn every_light_is_the_sun() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_LIT_FORCE_DIR").is_some())
}

/// The technique a surface is drawn with. Only techniques in the lit range
/// are adjusted; everything else comes back as it came in. For a lit one the
/// key says which primary light lights the surface and what kind of surface
/// it is, and `light_types` says what kind of light that is.
pub fn colour_lit_technique(
    base: TechType,
    packed_drawsurf: u64,
    light_types: &[u8],
    dfog: bool,
    sun_shadow_map: bool,
    spot_shadowed: &[u8],
) -> TechType {
    if base.0 != lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN {
        return base;
    }
    let key = dpvs_t6::unpack(dpvs_t6::GfxDrawSurf {
        packed: packed_drawsurf,
    });
    let light = key.primary_light_index;
    let light_type = if every_light_is_the_sun() {
        lighting_t6::GFX_LIGHT_TYPE_DIR
    } else {
        light_types.get(usize::from(light)).map_or(0, |kind| *kind)
    };
    let shadowed = match light_type {
        lighting_t6::GFX_LIGHT_TYPE_DIR => sun_shadow_map,
        lighting_t6::GFX_LIGHT_TYPE_SPOT => spot_shadowed.contains(&light),
        _ => false,
    };
    TechType(lighting_t6::lit_tech_type(
        base.0,
        key.surf_type,
        light_type,
        dfog,
        shadowed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_with_light(light: u8) -> u64 {
        with_scene_light_index(0, light)
    }

    fn lit(key: u64, light_type: u8, dfog: bool, shadowed: bool) -> TechType {
        let fields = dpvs_t6::unpack(dpvs_t6::GfxDrawSurf { packed: key });
        TechType(lighting_t6::lit_tech_type(
            lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN,
            fields.surf_type,
            light_type,
            dfog,
            shadowed,
        ))
    }

    const BASE: TechType = TechType(lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN);

    #[test]
    fn the_light_index_lands_in_the_key() {
        let key = key_with_light(37);
        let fields = dpvs_t6::unpack(dpvs_t6::GfxDrawSurf { packed: key });
        assert_eq!(fields.primary_light_index, 37);
    }

    #[test]
    fn setting_the_light_twice_keeps_only_the_last() {
        let key = with_scene_light_index(key_with_light(200), 5);
        assert_eq!(key, key_with_light(5));
    }

    #[test]
    fn techniques_outside_the_lit_range_pass_through() {
        let unlit = TechType(lighting_t6::GFX_DRAW_METHOD_LIT_BEGIN.wrapping_add(1));
        assert_eq!(colour_lit_technique(unlit, 0, &[], true, true, &[]), unlit);
    }

    #[test]
    fn a_sun_lit_surface_is_shadowed_by_the_sun_map() {
        let key = key_with_light(0);
        let types = [lighting_t6::GFX_LIGHT_TYPE_DIR];
        assert_eq!(
            colour_lit_technique(BASE, key, &types, false, true, &[]),
            lit(key, lighting_t6::GFX_LIGHT_TYPE_DIR, false, true)
        );
        assert_eq!(
            colour_lit_technique(BASE, key, &types, false, false, &[]),
            lit(key, lighting_t6::GFX_LIGHT_TYPE_DIR, false, false)
        );
    }

    #[test]
    fn a_spot_lit_surface_is_shadowed_only_if_its_own_light_is() {
        let key = key_with_light(2);
        let types = [0, 0, lighting_t6::GFX_LIGHT_TYPE_SPOT];
        assert_eq!(
            colour_lit_technique(BASE, key, &types, true, true, &[2]),
            lit(key, lighting_t6::GFX_LIGHT_TYPE_SPOT, true, true)
        );
        assert_eq!(
            colour_lit_technique(BASE, key, &types, true, true, &[1, 3]),
            lit(key, lighting_t6::GFX_LIGHT_TYPE_SPOT, true, false)
        );
    }

    #[test]
    fn a_light_the_map_never_defined_counts_as_no_light() {
        let key = key_with_light(9);
        assert_eq!(
            colour_lit_technique(
                BASE,
                key,
                &[lighting_t6::GFX_LIGHT_TYPE_DIR],
                false,
                true,
                &[9]
            ),
            lit(key, 0, false, false)
        );
    }

    #[test]
    fn other_light_kinds_never_cast_shadows_here() {
        let key = key_with_light(0);
        let types = [lighting_t6::GFX_LIGHT_TYPE_OMNI];
        assert_eq!(
            colour_lit_technique(BASE, key, &types, false, true, &[0]),
            lit(key, lighting_t6::GFX_LIGHT_TYPE_OMNI, false, false)
        );
    }
}
