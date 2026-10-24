//! A material's baked draw-surf key (Material_SortInternal) and the world surfaces' keys and
//! shadow-caster bytes (R_SortWorldSurfaces).

use crate::drawsurf::{GfxDrawSurf, GfxDrawSurfFields, pack, with_primary_light_index};

/// Technique flag 0x400: prepass 3.
pub const TECHNIQUE_FLAG_PREPASS_3: u16 = 0x400;

/// Material game flag 0x40: the material's surfaces cast shadows.
pub const MATERIAL_GAME_FLAG_CASTS_SHADOW: u32 = 0x40;

pub const SURF_FLAG_CASTS_SUN_SHADOW: u32 = 0x1;
pub const SURF_FLAG_PRIMARY_LIGHT_CONFLICT: u32 = 0x2;
pub const SURF_FLAG_SKY: u32 = 0x4;
pub const SURF_FLAG_NO_DRAW: u32 = 0x8;
pub const SURF_FLAG_CASTS_SHADOW: u32 = 0x10;
pub const SURF_FLAG_QUANTIZED: u32 = 0x20;
pub const SURF_FLAG_NO_COLOR: u32 = 0x40;

/// What R_DrawSurfStandardPrepassSortKey reads from a material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialPrepassInput {
    pub technique0_present: bool,
    pub technique1_present: bool,
    pub technique0_flags: u16,
    pub state_flags: u8,
    /// The technique-set name contains "flag_".
    pub techset_name_has_flag: bool,
}

/// R_DrawSurfStandardPrepassSortKey, in order: no technique 0 gives 2, or 3 with
/// technique 1; technique 0 flag 0x400 gives 3; state flag bit 4 gives 2; otherwise 1,
/// or 0 when the set name lacks "flag_" and technique 0 flag bit 4 and state flag bit 1
/// are both set.
#[must_use]
pub fn material_prepass(input: MaterialPrepassInput) -> u8 {
    if !input.technique0_present {
        2 + u8::from(input.technique1_present)
    } else if input.technique0_flags & TECHNIQUE_FLAG_PREPASS_3 != 0 {
        3
    } else if input.state_flags & 0x10 != 0 {
        2
    } else if !input.techset_name_has_flag
        && input.technique0_flags & 0x10 != 0
        && input.state_flags & 0x2 != 0
    {
        0
    } else {
        1
    }
}

#[inline]
#[must_use]
pub const fn material_sort_key_row(sort_key: u8) -> u8 {
    sort_key & 0x3f
}

/// A material's key, built from zero at material sort time: sort key, prepass and sorted
/// position. The custom index is forced to zero; surfType, light and dlight bits are set
/// later.
#[must_use]
pub fn bake_material_draw_surf_key(
    sort_key: u8,
    prepass: u8,
    material_sorted_index: u16,
) -> GfxDrawSurf {
    pack(GfxDrawSurfFields {
        material_sorted_index: material_sorted_index & 0xfff,
        prepass: prepass & 3,
        primary_sort_key: material_sort_key_row(sort_key),
        ..GfxDrawSurfFields::default()
    })
}

/// R_GetWorldDrawSurf: the material's key with the surface's primary light.
#[must_use]
pub fn world_surface_material(
    material_draw_surf: GfxDrawSurf,
    primary_light_index: u8,
) -> GfxDrawSurf {
    with_primary_light_index(material_draw_surf, primary_light_index)
}

/// The key R_SortWorldSurfaces stores per surface: low dword masked 0xfe00ffff, which drops
/// the custom index bits again.
#[must_use]
pub const fn stored_world_surface_key(key: GfxDrawSurf) -> GfxDrawSurf {
    GfxDrawSurf {
        packed: key.packed & !0x01ff_0000,
    }
}

/// Fills dpvs.surfaceMaterials; a surface with no baked material gets key 0.
pub fn fill_surface_materials(
    surface_material_slots: &[Option<usize>],
    surface_primary_lights: &[u8],
    baked_by_material_slot: &[Option<GfxDrawSurf>],
    out: &mut [GfxDrawSurf],
) {
    let n = out
        .len()
        .min(surface_material_slots.len())
        .min(surface_primary_lights.len());
    for i in 0..n {
        out[i] = surface_material_slots[i]
            .and_then(|slot| baked_by_material_slot.get(slot).copied().flatten())
            .map(|base| stored_world_surface_key(world_surface_material(base, surface_primary_lights[i])))
            .unwrap_or(GfxDrawSurf::from_packed(0));
    }
}

/// A world surface's two caster bytes (surfaceCastsShadow, surfaceCastsSunShadow).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCasters {
    pub shadow: bool,
    pub sun_shadow: bool,
}

/// Only a material with game flag 0x40 casts; its surfaces then cast shadow (BO2 asserts
/// surface flag 0x10) and cast sun shadow when surface flag bit 0 is set.
#[must_use]
pub const fn surface_casters(material_game_flags: u32, surface_flags: u32) -> SurfaceCasters {
    let shadow = material_game_flags & MATERIAL_GAME_FLAG_CASTS_SHADOW != 0;
    SurfaceCasters {
        shadow,
        sun_shadow: shadow && surface_flags & SURF_FLAG_CASTS_SUN_SHADOW != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepass(f: impl FnOnce(&mut MaterialPrepassInput)) -> u8 {
        let mut input = MaterialPrepassInput {
            technique0_present: true,
            techset_name_has_flag: true,
            ..Default::default()
        };
        f(&mut input);
        material_prepass(input)
    }

    #[test]
    fn prepass_rules_in_order() {
        assert_eq!(prepass(|i| i.technique0_present = false), 2);
        assert_eq!(
            prepass(|i| {
                i.technique0_present = false;
                i.technique1_present = true;
            }),
            3
        );
        assert_eq!(
            prepass(|i| {
                i.technique0_flags = 0x400;
                i.state_flags = 0x10;
            }),
            3
        );
        assert_eq!(prepass(|i| i.state_flags = 0x12), 2);
        assert_eq!(prepass(|_| {}), 1);
        let zero = |i: &mut MaterialPrepassInput| {
            i.techset_name_has_flag = false;
            i.technique0_flags = 0x10;
            i.state_flags = 0x2;
        };
        assert_eq!(prepass(zero), 0);
        assert_eq!(
            prepass(|i| {
                zero(i);
                i.techset_name_has_flag = true;
            }),
            1
        );
    }

    #[test]
    fn bake_has_no_custom_index() {
        let key = bake_material_draw_surf_key(0x47, 2, 0x1234);
        let f = crate::unpack(key);
        assert_eq!((f.primary_sort_key, f.prepass, f.material_sorted_index), (7, 2, 0x234));
        assert_eq!(key.packed as u32, 0);
    }

    #[test]
    fn world_key_takes_the_light_and_drops_custom_bits() {
        let base = GfxDrawSurf::from_packed(0x0000_0000_01ff_0000 | (0x55u64 << 44));
        let mut out = [GfxDrawSurf::default(); 3];
        fill_surface_materials(&[Some(0), None, Some(1)], &[9, 9, 9], &[Some(base)], &mut out);
        assert_eq!(out[0].primary_light_index(), 9);
        assert_eq!(out[0].custom_index(), 0);
        assert_eq!(out[1].packed, 0);
        assert_eq!(out[2].packed, 0);
    }

    #[test]
    fn casters_need_game_flag_0x40() {
        assert_eq!(surface_casters(0, 0x11), SurfaceCasters::default());
        assert_eq!(
            surface_casters(0x40, 0x10),
            SurfaceCasters { shadow: true, sun_shadow: false }
        );
        assert_eq!(
            surface_casters(0x40, 0x11),
            SurfaceCasters { shadow: true, sun_shadow: true }
        );
    }
}
