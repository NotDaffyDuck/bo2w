//! BO2's draw-surf key (GfxDrawSurf): one u64 whose ascending order is the draw order.
//! Low dword: objectId 0-15, customIndex 16-24, reflectionProbeIndex 25-29, dlightMask 30-31.
//! High dword: materialSortedIndex 32-43, primaryLightIndex 44-51, surfType 52-55,
//! prepass 56-57, primarySortKey 58-63. No hero-light bit and no gfxEnt bit.

pub const SF_TRIANGLES: u8 = 0;
pub const SF_TRIANGLES_NO_SUN_SHADOW: u8 = 1;
pub const SF_STATICMODEL_RIGID_NO_SUN_SHADOW: u8 = 2;
pub const SF_STATICMODEL_RIGID: u8 = 3;
pub const SF_STATICMODEL_SKINNED: u8 = 4;
pub const SF_BMODEL: u8 = 5;
pub const SF_XMODEL_RIGID: u8 = 6;
pub const SF_XMODEL_RIGID_NO_SUN_SHADOW: u8 = 7;
pub const SF_XMODEL_RIGID_SKINNED: u8 = 8;
pub const SF_XMODEL_SKINNED: u8 = 9;
pub const SF_XMODEL_SKINNED_NO_SUN_SHADOW: u8 = 10;
pub const SF_CODE_MESH: u8 = 11;
pub const SF_MARK_MESH: u8 = 12;
pub const SF_PARTICLE_CLOUD: u8 = 13;
pub const SF_ROPE_MESH: u8 = 14;
pub const SF_GLASS_MESH: u8 = 15;

/// GfxScene keeps one list per draw-surf stage (the DRAW_SURF_* enum), not one global list.
pub const DRAW_SURF_STAGE_COUNT: usize = 50;

pub const DRAW_SURF_BSP_CAMERA_LIT: usize = 0;
pub const DRAW_SURF_BSP_CAMERA_LIGHT_MAP: usize = 3;
pub const DRAW_SURF_BSP_CAMERA_LIT_TRANS: usize = 7;
pub const DRAW_SURF_BSP_CAMERA_EMISSIVE_OPAQUE: usize = 16;
pub const DRAW_SURF_BSP_CAMERA_EMISSIVE_TRANS: usize = 19;

/// A stage's list capacity, or `None` where BO2's value was not recorded (emissive smodel
/// and entity stages 17-18 and 20-23, 27-28, and the sun-shadow smodel/entity stages
/// 30-31 and 34-35: BO2 unknown).
#[must_use]
pub const fn stage_capacity(stage: usize) -> Option<usize> {
    match stage {
        0..=2 => Some(4608),
        3..=5 => Some(3584),
        6..=8 => Some(512),
        9 => Some(1024),
        10 => Some(2048),
        11 => Some(512),
        12..=14 | 24..=26 => Some(8192),
        15 | 16 | 19 => Some(1024),
        29 => Some(4096),
        32 | 36 => Some(512),
        33 => Some(8192),
        37..DRAW_SURF_STAGE_COUNT => Some(1024),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GfxDrawSurf {
    pub packed: u64,
}

impl GfxDrawSurf {
    pub const fn from_packed(packed: u64) -> Self {
        Self { packed }
    }

    pub const fn object_id(self) -> u16 {
        (self.packed & 0xffff) as u16
    }

    pub const fn custom_index(self) -> u16 {
        ((self.packed >> 16) & 0x1ff) as u16
    }

    pub const fn reflection_probe_index(self) -> u8 {
        ((self.packed >> 25) & 0x1f) as u8
    }

    pub const fn dlight_mask(self) -> u8 {
        ((self.packed >> 30) & 0x3) as u8
    }

    pub const fn material_sorted_index(self) -> u16 {
        ((self.packed >> 32) & 0xfff) as u16
    }

    pub const fn primary_light_index(self) -> u8 {
        ((self.packed >> 44) & 0xff) as u8
    }

    pub const fn surf_type(self) -> u8 {
        ((self.packed >> 52) & 0xf) as u8
    }

    pub const fn prepass(self) -> u8 {
        ((self.packed >> 56) & 0x3) as u8
    }

    pub const fn primary_sort_key(self) -> u8 {
        ((self.packed >> 58) & 0x3f) as u8
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxDrawSurfFields {
    pub object_id: u16,
    pub custom_index: u16,
    pub reflection_probe_index: u8,
    pub dlight_mask: u8,
    pub material_sorted_index: u16,
    pub primary_light_index: u8,
    pub surf_type: u8,
    pub prepass: u8,
    pub primary_sort_key: u8,
}

/// Packs the fields, each masked to its width.
#[must_use]
pub fn pack(fields: GfxDrawSurfFields) -> GfxDrawSurf {
    let mut p = u64::from(fields.object_id);
    p |= u64::from(fields.custom_index & 0x1ff) << 16;
    p |= u64::from(fields.reflection_probe_index & 0x1f) << 25;
    p |= u64::from(fields.dlight_mask & 0x3) << 30;
    p |= u64::from(fields.material_sorted_index & 0xfff) << 32;
    p |= u64::from(fields.primary_light_index) << 44;
    p |= u64::from(fields.surf_type & 0xf) << 52;
    p |= u64::from(fields.prepass & 0x3) << 56;
    p |= u64::from(fields.primary_sort_key & 0x3f) << 58;
    GfxDrawSurf { packed: p }
}

#[must_use]
pub fn unpack(surf: GfxDrawSurf) -> GfxDrawSurfFields {
    GfxDrawSurfFields {
        object_id: surf.object_id(),
        custom_index: surf.custom_index(),
        reflection_probe_index: surf.reflection_probe_index(),
        dlight_mask: surf.dlight_mask(),
        material_sorted_index: surf.material_sorted_index(),
        primary_light_index: surf.primary_light_index(),
        surf_type: surf.surf_type(),
        prepass: surf.prepass(),
        primary_sort_key: surf.primary_sort_key(),
    }
}

const fn with_bits(base: GfxDrawSurf, shift: u32, mask: u64, value: u64) -> GfxDrawSurf {
    GfxDrawSurf {
        packed: (base.packed & !(mask << shift)) | ((value & mask) << shift),
    }
}

#[must_use]
pub const fn with_object_id(base: GfxDrawSurf, object_id: u16) -> GfxDrawSurf {
    with_bits(base, 0, 0xffff, object_id as u64)
}

/// R_GetWorldDrawSurf's light write: clears high-dword bits 12-19 (mask 0xfff00fff) and ORs
/// in the light.
#[must_use]
pub const fn with_primary_light_index(base: GfxDrawSurf, primary_light_index: u8) -> GfxDrawSurf {
    with_bits(base, 44, 0xff, primary_light_index as u64)
}

#[must_use]
pub const fn with_reflection_probe_index(
    base: GfxDrawSurf,
    reflection_probe_index: u8,
) -> GfxDrawSurf {
    with_bits(base, 25, 0x1f, reflection_probe_index as u64)
}

/// The camera BSP add path's per-frame dlight bits: replaces low-dword bits 30-31 and keeps
/// the other 30 baked bits.
#[must_use]
pub const fn with_dlight_mask(base: GfxDrawSurf, dlight_mask: u8) -> GfxDrawSurf {
    with_bits(base, 30, 0x3, dlight_mask as u64)
}

#[must_use]
pub const fn with_surf_type(base: GfxDrawSurf, surf_type: u8) -> GfxDrawSurf {
    with_bits(base, 52, 0xf, surf_type as u64)
}

#[must_use]
pub const fn with_prepass(base: GfxDrawSurf, prepass: u8) -> GfxDrawSurf {
    with_bits(base, 56, 0x3, prepass as u64)
}

// BO2 unknown: how the entity and xmodel adders fill objectId and surfType was not traced.
// MW2's rule is kept: the material's baked key with objectId and surfType replaced.

#[must_use]
pub fn pack_xmodel_rigid_draw_surf(material_baked: GfxDrawSurf, object_id: u16) -> GfxDrawSurf {
    with_surf_type(with_object_id(material_baked, object_id), SF_XMODEL_RIGID)
}

#[must_use]
pub fn pack_xmodel_rigid_skinned_draw_surf(
    material_baked: GfxDrawSurf,
    object_id: u16,
) -> GfxDrawSurf {
    with_surf_type(with_object_id(material_baked, object_id), SF_XMODEL_RIGID_SKINNED)
}

fn pack_mesh(
    sort_key: u8,
    material_sorted_index: u16,
    object_id: u16,
    surf_type: u8,
) -> GfxDrawSurfFields {
    GfxDrawSurfFields {
        object_id,
        material_sorted_index: material_sorted_index & 0xfff,
        primary_sort_key: crate::material_sort_key_row(sort_key),
        surf_type,
        ..Default::default()
    }
}

#[must_use]
pub fn pack_code_mesh_draw_surf(
    sort_key: u8,
    material_sorted_index: u16,
    code_mesh_index: u16,
) -> GfxDrawSurf {
    pack(pack_mesh(sort_key, material_sorted_index, code_mesh_index, SF_CODE_MESH))
}

/// The rope code-mesh adder forces surfType ROPE_MESH and prepass 2.
#[must_use]
pub fn pack_rope_mesh_draw_surf(
    sort_key: u8,
    material_sorted_index: u16,
    code_mesh_index: u16,
) -> GfxDrawSurf {
    pack(GfxDrawSurfFields {
        prepass: 2,
        ..pack_mesh(sort_key, material_sorted_index, code_mesh_index, SF_ROPE_MESH)
    })
}

/// BO2 unknown: the cloud adder was not traced; MW2's fields are kept.
#[must_use]
pub fn pack_particle_cloud_draw_surf(
    sort_key: u8,
    material_sorted_index: u16,
    cloud_index: u16,
) -> GfxDrawSurf {
    pack(pack_mesh(sort_key, material_sorted_index, cloud_index, SF_PARTICLE_CLOUD))
}

/// BO2 unknown: the mark adder was not traced; MW2's fields are kept, with the lightmap in
/// the (9-bit) custom index and the probe cut to BO2's 5 bits.
#[must_use]
pub fn pack_mark_mesh_draw_surf(
    sort_key: u8,
    material_sorted_index: u16,
    mark_mesh_index: u16,
    lmap_index: u8,
    primary_light_index: u8,
    reflection_probe_index: u8,
) -> GfxDrawSurf {
    pack(GfxDrawSurfFields {
        custom_index: u16::from(lmap_index),
        primary_light_index,
        reflection_probe_index,
        ..pack_mesh(sort_key, material_sorted_index, mark_mesh_index, SF_MARK_MESH)
    })
}

/// BO2 unknown: the glass adder was not traced; MW2's fields are kept.
#[must_use]
pub fn pack_glass_mesh_draw_surf(
    sort_key: u8,
    material_sorted_index: u16,
    glass_surf_index: u16,
    reflection_probe_index: u8,
) -> GfxDrawSurf {
    pack(GfxDrawSurfFields {
        reflection_probe_index,
        ..pack_mesh(sort_key, material_sorted_index, glass_surf_index, SF_GLASS_MESH)
    })
}

/// R_SortDrawSurfs: high dword then low dword, ascending, not stable; the same as an
/// unstable sort on the whole u64.
pub fn sort_keys(keys: &mut [GfxDrawSurf]) {
    keys.sort_unstable_by_key(|k| k.packed);
}

/// Material changes along a sorted list.
#[must_use]
pub fn material_rebind_count(sorted: &[GfxDrawSurf]) -> usize {
    let mut count = 0usize;
    let mut prev: Option<u16> = None;
    for k in sorted {
        let m = k.material_sorted_index();
        if prev != Some(m) {
            count += 1;
            prev = Some(m);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_land_in_bo2_bits() {
        let all = pack(GfxDrawSurfFields {
            object_id: 0xffff,
            custom_index: 0x1ff,
            reflection_probe_index: 0x1f,
            dlight_mask: 3,
            material_sorted_index: 0xfff,
            primary_light_index: 0xff,
            surf_type: 0xf,
            prepass: 3,
            primary_sort_key: 0x3f,
        });
        assert_eq!(all.packed, u64::MAX);
        let one = |f: GfxDrawSurfFields| pack(f).packed;
        assert_eq!(one(GfxDrawSurfFields { custom_index: 1, ..Default::default() }), 1 << 16);
        assert_eq!(
            one(GfxDrawSurfFields { reflection_probe_index: 1, ..Default::default() }),
            1 << 25
        );
        assert_eq!(one(GfxDrawSurfFields { dlight_mask: 1, ..Default::default() }), 1 << 30);
        assert_eq!(
            one(GfxDrawSurfFields { material_sorted_index: 1, ..Default::default() }),
            1 << 32
        );
        assert_eq!(
            one(GfxDrawSurfFields { primary_light_index: 1, ..Default::default() }),
            1 << 44
        );
        assert_eq!(one(GfxDrawSurfFields { surf_type: 1, ..Default::default() }), 1 << 52);
        assert_eq!(one(GfxDrawSurfFields { prepass: 1, ..Default::default() }), 1 << 56);
        assert_eq!(one(GfxDrawSurfFields { primary_sort_key: 1, ..Default::default() }), 1 << 58);
    }

    #[test]
    fn unpack_round_trips() {
        let f = GfxDrawSurfFields {
            object_id: 0x1234,
            custom_index: 0x155,
            reflection_probe_index: 0x11,
            dlight_mask: 2,
            material_sorted_index: 0xabc,
            primary_light_index: 0x5a,
            surf_type: SF_MARK_MESH,
            prepass: 1,
            primary_sort_key: 0x2a,
        };
        assert_eq!(unpack(pack(f)), f);
    }

    #[test]
    fn prepass_outranks_surf_type_and_material() {
        let lo = pack(GfxDrawSurfFields {
            prepass: 0,
            surf_type: 15,
            material_sorted_index: 0xfff,
            primary_sort_key: 5,
            ..Default::default()
        });
        let hi = pack(GfxDrawSurfFields {
            prepass: 1,
            primary_sort_key: 5,
            ..Default::default()
        });
        let mut keys = [hi, lo];
        sort_keys(&mut keys);
        assert_eq!(keys, [lo, hi]);
    }

    #[test]
    fn setters_touch_only_their_bits() {
        let base = GfxDrawSurf::from_packed(u64::MAX);
        assert_eq!(with_primary_light_index(base, 0).packed, !(0xffu64 << 44));
        assert_eq!(with_dlight_mask(base, 0).packed, !(3u64 << 30));
        assert_eq!(with_reflection_probe_index(base, 0).packed, !(0x1fu64 << 25));
        assert_eq!(with_object_id(base, 0).packed, !0xffff);
        assert_eq!(with_dlight_mask(GfxDrawSurf::default(), 7).dlight_mask(), 3);
    }

    #[test]
    fn xmodel_keeps_the_baked_material_bits() {
        let baked = pack(GfxDrawSurfFields {
            material_sorted_index: 0x321,
            prepass: 2,
            primary_sort_key: 9,
            reflection_probe_index: 4,
            ..Default::default()
        });
        let f = unpack(pack_xmodel_rigid_skinned_draw_surf(baked, 77));
        assert_eq!(f.object_id, 77);
        assert_eq!(f.surf_type, SF_XMODEL_RIGID_SKINNED);
        assert_eq!((f.material_sorted_index, f.prepass, f.primary_sort_key), (0x321, 2, 9));
        assert_eq!(f.reflection_probe_index, 4);
    }

    #[test]
    fn rope_forces_prepass_two() {
        let f = unpack(pack_rope_mesh_draw_surf(0x45, 0x1abc, 3));
        assert_eq!((f.surf_type, f.prepass), (SF_ROPE_MESH, 2));
        assert_eq!((f.primary_sort_key, f.material_sorted_index), (5, 0xabc));
    }

    #[test]
    fn stage_capacities() {
        assert_eq!(stage_capacity(DRAW_SURF_BSP_CAMERA_LIT), Some(4608));
        assert_eq!(stage_capacity(DRAW_SURF_BSP_CAMERA_LIGHT_MAP), Some(3584));
        assert_eq!(stage_capacity(DRAW_SURF_BSP_CAMERA_LIT_TRANS), Some(512));
        assert_eq!(stage_capacity(DRAW_SURF_BSP_CAMERA_EMISSIVE_TRANS), Some(1024));
        assert_eq!(stage_capacity(30), None);
        assert_eq!(stage_capacity(49), Some(1024));
        assert_eq!(stage_capacity(DRAW_SURF_STAGE_COUNT), None);
    }

    #[test]
    fn rebinds_count_material_changes() {
        let k = |m| pack(GfxDrawSurfFields { material_sorted_index: m, ..Default::default() });
        assert_eq!(material_rebind_count(&[k(1), k(1), k(2), k(1)]), 3);
    }
}
