//! A material's baked draw-surf key (Material_SortInternal) and the world surfaces' keys and
//! shadow-caster bytes (R_SortWorldSurfaces).

use crate::sort_bits::{SortedDraw, SortedDrawParts, pack, with_key_light};

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
    pub has_second_pass: bool,
    pub technique0_flags: u16,
    pub condition_bits: u8,
    /// The technique-set name contains "flag_".
    pub techset_name_has_flag: bool,
}

/// R_DrawSurfStandardPrepassSortKey, in order: no technique 0 gives 2, or 3 with
/// technique 1; technique 0 flag 0x400 gives 3; state flag bit 4 gives 2; otherwise 1,
/// or 0 when the set name lacks "flag_" and technique 0 flag bit 4 and state flag bit 1
/// are both set.
#[must_use]
pub fn prepass_flag_of(input: MaterialPrepassInput) -> u8 {
    if !input.technique0_present {
        2 + u8::from(input.has_second_pass)
    } else if input.technique0_flags & TECHNIQUE_FLAG_PREPASS_3 != 0 {
        3
    } else if input.condition_bits & 0x10 != 0 {
        2
    } else if !input.techset_name_has_flag
        && input.technique0_flags & 0x10 != 0
        && input.condition_bits & 0x2 != 0
    {
        0
    } else {
        1
    }
}

#[inline]
#[must_use]
pub const fn material_sort_group(sort_key: u8) -> u8 {
    sort_key % 64
}

/// A material's key, built from zero at material sort time: sort key, prepass and sorted
/// position. The custom index is forced to zero; surfType, light and dlight bits are set
/// later.
#[must_use]
pub fn material_order_key(
    key_byte: u8,
    depth_prepass: u8,
    material_draw_rank_slot: u16,
) -> SortedDraw {
    pack(SortedDrawParts {
        material_draw_rank_slot: material_draw_rank_slot & 0xfff,
        depth_prepass: depth_prepass & 3,
        leading_sort_bucket: material_sort_group(key_byte),
        ..SortedDrawParts::default()
    })
}

/// R_GetWorldDrawSurf: the material's key with the surface's primary light.
#[must_use]
pub fn static_surface_material(
    mat_sort_key: SortedDraw,
    key_light_index: u8,
) -> SortedDraw {
    with_key_light(mat_sort_key, key_light_index)
}

/// The key R_SortWorldSurfaces stores per surface: low dword masked 0xfe00ffff, which drops
/// the custom index bits again.
#[must_use]
pub const fn stored_world_surface_key(key: SortedDraw) -> SortedDraw {
    SortedDraw {
        packed: key.packed & !0x01ff_0000,
    }
}

/// Fills dpvs.surfaceMaterials; a surface with no baked material gets key 0.
pub fn populate_surface_material_table(
    face_mat_slots: &[Option<usize>],
    facet_key_lights: &[u8],
    keys_by_mat_slot: &[Option<SortedDraw>],
    out: &mut [SortedDraw],
) {
    let n = out
        .len()
        .min(face_mat_slots.len())
        .min(facet_key_lights.len());
    for i in 0..n {
        out[i] = face_mat_slots[i]
            .and_then(|slot| keys_by_mat_slot.get(slot).copied().flatten())
            .map(|base| stored_world_surface_key(static_surface_material(base, facet_key_lights[i])))
            .unwrap_or(SortedDraw::unpack_from_bits(0));
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
        prepass_flag_of(input)
    }

    #[test]
    fn prepass_rules_in_order() {
        assert_eq!(prepass(|i| i.technique0_present = false), 2);
        assert_eq!(
            prepass(|i| {
                i.technique0_present = false;
                i.has_second_pass = true;
            }),
            3
        );
        assert_eq!(
            prepass(|i| {
                i.technique0_flags = 0x400;
                i.condition_bits = 0x10;
            }),
            3
        );
        assert_eq!(prepass(|i| i.condition_bits = 0x12), 2);
        assert_eq!(prepass(|_| {}), 1);
        let zero = |i: &mut MaterialPrepassInput| {
            i.techset_name_has_flag = false;
            i.technique0_flags = 0x10;
            i.condition_bits = 0x2;
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
        let key = material_order_key(0x47, 2, 0x1234);
        let f = crate::unpack(key);
        assert_eq!((f.leading_sort_bucket, f.depth_prepass, f.material_draw_rank_slot), (7, 2, 0x234));
        assert_eq!(key.packed as u32, 0);
    }

    #[test]
    fn world_key_takes_the_light_and_drops_custom_bits() {
        let base = SortedDraw::unpack_from_bits(0x0000_0000_01ff_0000 | (0x55u64 << 44));
        let mut out = [SortedDraw::default(); 3];
        populate_surface_material_table(&[Some(0), None, Some(1)], &[9, 9, 9], &[Some(base)], &mut out);
        assert_eq!(out[0].key_light_index(), 9);
        assert_eq!(out[0].extra_ix(), 0);
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
