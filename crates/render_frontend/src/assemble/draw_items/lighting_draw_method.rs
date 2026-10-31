//! Choosing the lit technique for a draw: which variant of the lit shader a
//! surface needs depends on the light that falls on it (sun, spot, or none),
//! whether that light casts a shadow, and whether the fog is the per-pixel
//! kind.

use bevy::prelude::Resource;

use super::material_setup::MethodType;

/// Light type of every primary light the map defines, by light index.
#[derive(Resource, Clone, Debug, Default)]
pub struct MapKeyLightTypes {
    pub types: Vec<u8>,
}

/// Whether the map's fog runs in the per-pixel draw method.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawMethodDepthHaze(pub bool);

/// Whether a sun shadow map exists this frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DaylightShadowMapPresent(pub bool);

/// Indices of the spot lights that have a shadow map this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct ConeShadowMapLights(pub Vec<u8>);

/// Put a primary light index into a packed draw-surface key, leaving every
/// other field alone.
pub fn scene_light_index_of(packed: u64, stage_light_index: u8) -> u64 {
    let key = dpvs_t6::SortedDraw { packed };
    dpvs_t6::with_key_light(key, stage_light_index).packed
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
pub fn colour_lit_method(
    first_index: MethodType,
    compact_drawsurf: u64,
    lamp_types: &[u8],
    depth_haze: bool,
    daylight_shadow_map: bool,
    spot_has_shadow: &[u8],
) -> MethodType {
    if first_index.0 != lighting_t6::LIT_DRAW_PATH_FIRST {
        return first_index;
    }
    let key = dpvs_t6::unpack(dpvs_t6::SortedDraw {
        packed: compact_drawsurf,
    });
    let light = key.key_light_index;
    let lamp_type = if every_light_is_the_sun() {
        lighting_t6::LAMP_KIND_PARALLEL
    } else {
        lamp_types.get(usize::from(light)).map_or(0, |kind| *kind)
    };
    let shadowed = match lamp_type {
        lighting_t6::LAMP_KIND_PARALLEL => daylight_shadow_map,
        lighting_t6::CONE_LAMP => spot_has_shadow.contains(&light),
        _ => false,
    };
    MethodType(lighting_t6::lit_technique_kind(
        first_index.0,
        key.kind,
        lamp_type,
        depth_haze,
        shadowed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_with_light(light: u8) -> u64 {
        scene_light_index_of(0, light)
    }

    fn lit(key: u64, lamp_type: u8, depth_haze: bool, shadowed: bool) -> MethodType {
        let field_list = dpvs_t6::unpack(dpvs_t6::SortedDraw { packed: key });
        MethodType(lighting_t6::lit_technique_kind(
            lighting_t6::LIT_DRAW_PATH_FIRST,
            field_list.kind,
            lamp_type,
            depth_haze,
            shadowed,
        ))
    }

    const BASE: MethodType = MethodType(lighting_t6::LIT_DRAW_PATH_FIRST);

    #[test]
    fn the_light_index_lands_in_the_key() {
        let key = key_with_light(37);
        let fields = dpvs_t6::unpack(dpvs_t6::SortedDraw { packed: key });
        assert_eq!(fields.key_light_index, 37);
    }

    #[test]
    fn setting_the_light_twice_keeps_only_the_last() {
        let key = scene_light_index_of(key_with_light(200), 5);
        assert_eq!(key, key_with_light(5));
    }

    #[test]
    fn techniques_outside_the_lit_range_pass_through() {
        let unlit = MethodType(lighting_t6::LIT_DRAW_PATH_FIRST.wrapping_add(1));
        assert_eq!(colour_lit_method(unlit, 0, &[], true, true, &[]), unlit);
    }

    #[test]
    fn a_sun_lit_surface_is_shadowed_by_the_sun_map() {
        let key = key_with_light(0);
        let types = [lighting_t6::LAMP_KIND_PARALLEL];
        assert_eq!(
            colour_lit_method(BASE, key, &types, false, true, &[]),
            lit(key, lighting_t6::LAMP_KIND_PARALLEL, false, true)
        );
        assert_eq!(
            colour_lit_method(BASE, key, &types, false, false, &[]),
            lit(key, lighting_t6::LAMP_KIND_PARALLEL, false, false)
        );
    }

    #[test]
    fn a_spot_lit_surface_is_shadowed_only_if_its_own_light_is() {
        let key = key_with_light(2);
        let types = [0, 0, lighting_t6::CONE_LAMP];
        assert_eq!(
            colour_lit_method(BASE, key, &types, true, true, &[2]),
            lit(key, lighting_t6::CONE_LAMP, true, true)
        );
        assert_eq!(
            colour_lit_method(BASE, key, &types, true, true, &[1, 3]),
            lit(key, lighting_t6::CONE_LAMP, true, false)
        );
    }

    #[test]
    fn a_light_the_map_never_defined_counts_as_no_light() {
        let key = key_with_light(9);
        assert_eq!(
            colour_lit_method(
                BASE,
                key,
                &[lighting_t6::LAMP_KIND_PARALLEL],
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
        let types = [lighting_t6::POINT_LAMP];
        assert_eq!(
            colour_lit_method(BASE, key, &types, false, true, &[0]),
            lit(key, lighting_t6::POINT_LAMP, false, false)
        );
    }
}
