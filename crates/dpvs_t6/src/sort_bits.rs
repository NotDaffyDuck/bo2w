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
pub const SURFACE_KIND_XMODEL_RIGID: u8 = 6;
pub const SF_XMODEL_RIGID_NO_SUN_SHADOW: u8 = 7;
pub const SURFACE_KIND_MODEL_RIGID_BONED: u8 = 8;
pub const SF_XMODEL_SKINNED: u8 = 9;
pub const SF_XMODEL_SKINNED_NO_SUN_SHADOW: u8 = 10;
pub const KIND_CODE_MESH: u8 = 11;
pub const SURFACE_KIND_DECAL_MESH: u8 = 12;
pub const SURFACE_KIND_PARTICLE_CLOUD: u8 = 13;
pub const SF_ROPE_MESH: u8 = 14;
pub const SURFACE_KIND_GLASS_MESH: u8 = 15;

/// VisualStage keeps one list per draw-surf stage (the DRAW_SURF_* enum), not one global list.
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
    /// Per stage, its list size; 0 marks a stage whose size is not known.
    const SIZES: [usize; DRAW_SURF_STAGE_COUNT] = {
        let mut sizes = [0usize; DRAW_SURF_STAGE_COUNT];
        // (first stage, one past the last, size); every stage from 37 on holds 1024.
        let spans: [(usize, usize, usize); 15] = [
            (0, 3, 4608),
            (3, 6, 3584),
            (6, 9, 512),
            (9, 10, 1024),
            (10, 11, 2048),
            (11, 12, 512),
            (12, 15, 8192),
            (15, 17, 1024),
            (19, 20, 1024),
            (24, 27, 8192),
            (29, 30, 4096),
            (32, 33, 512),
            (33, 34, 8192),
            (36, 37, 512),
            (37, DRAW_SURF_STAGE_COUNT, 1024),
        ];
        let mut s = 0;
        while s < spans.len() {
            let (from, to, size) = spans[s];
            let mut stage = from;
            while stage < to {
                sizes[stage] = size;
                stage += 1;
            }
            s += 1;
        }
        sizes
    };
    if stage >= DRAW_SURF_STAGE_COUNT || SIZES[stage] == 0 {
        return None;
    }
    Some(SIZES[stage])
}


/// Where one field sits in the key: its lowest bit and how many bits it has.
#[derive(Clone, Copy)]
struct Field {
    shift: u32,
    width: u32,
}

impl Field {
    const fn ones(self) -> u64 {
        u64::MAX >> (64 - self.width)
    }

    const fn read(self, key: u64) -> u64 {
        (key >> self.shift) & self.ones()
    }

    /// `key` with this field replaced by `value`, cut to the field's width.
    const fn write(self, key: u64, value: u64) -> u64 {
        let cleared = key & !(self.ones() << self.shift);
        cleared | ((value & self.ones()) << self.shift)
    }
}

const OBJECT_ID: Field = Field { shift: 0, width: 16 };
const CUSTOM_INDEX: Field = Field { shift: 16, width: 9 };
const PROBE: Field = Field { shift: 25, width: 5 };
const DLIGHTS: Field = Field { shift: 30, width: 2 };
const MATERIAL: Field = Field { shift: 32, width: 12 };
const LIGHT: Field = Field { shift: 44, width: 8 };
const SURF_TYPE: Field = Field { shift: 52, width: 4 };
const PREPASS: Field = Field { shift: 56, width: 2 };
const SORT_KEY: Field = Field { shift: 58, width: 6 };

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SortedDraw {
    pub packed: u64,
}

impl SortedDraw {
    pub const fn unpack_from_bits(packed: u64) -> Self {
        SortedDraw { packed }
    }

    const fn with(self, field: Field, value: u64) -> Self {
        Self::unpack_from_bits(field.write(self.packed, value))
    }

    pub const fn object_tag(self) -> u16 {
        OBJECT_ID.read(self.packed) as u16
    }

    pub const fn extra_ix(self) -> u16 {
        CUSTOM_INDEX.read(self.packed) as u16
    }

    pub const fn env_probe_index(self) -> u8 {
        PROBE.read(self.packed) as u8
    }

    pub const fn dlight_mask(self) -> u8 {
        DLIGHTS.read(self.packed) as u8
    }

    pub const fn material_draw_rank_slot(self) -> u16 {
        MATERIAL.read(self.packed) as u16
    }

    pub const fn key_light_index(self) -> u8 {
        LIGHT.read(self.packed) as u8
    }

    pub const fn kind(self) -> u8 {
        SURF_TYPE.read(self.packed) as u8
    }

    pub const fn depth_prepass(self) -> u8 {
        PREPASS.read(self.packed) as u8
    }

    pub const fn leading_sort_bucket(self) -> u8 {
        SORT_KEY.read(self.packed) as u8
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SortedDrawParts {
    pub object_tag: u16,
    pub extra_ix: u16,
    pub env_probe_index: u8,
    pub dlight_mask: u8,
    pub material_draw_rank_slot: u16,
    pub key_light_index: u8,
    pub kind: u8,
    pub depth_prepass: u8,
    pub leading_sort_bucket: u8,
}

/// Packs the fields, each masked to its width.
#[must_use]
pub fn pack(pieces: SortedDrawParts) -> SortedDraw {
    let f = pieces;
    let pieces = [
        (OBJECT_ID, u64::from(f.object_tag)),
        (CUSTOM_INDEX, u64::from(f.extra_ix)),
        (PROBE, u64::from(f.env_probe_index)),
        (DLIGHTS, u64::from(f.dlight_mask)),
        (MATERIAL, u64::from(f.material_draw_rank_slot)),
        (LIGHT, u64::from(f.key_light_index)),
        (SURF_TYPE, u64::from(f.kind)),
        (PREPASS, u64::from(f.depth_prepass)),
        (SORT_KEY, u64::from(f.leading_sort_bucket)),
    ];
    pieces.into_iter().fold(SortedDraw::default(), |key, (field, value)| key.with(field, value))
}

#[must_use]
pub fn unpack(face: SortedDraw) -> SortedDrawParts {
    let s = face;
    let (object_tag, custom_slot, dlight_mask) = (s.object_tag(), s.extra_ix(), s.dlight_mask());
    let (env_probe_index, key_light_index) = (s.env_probe_index(), s.key_light_index());
    let (material_draw_rank_slot, surf_type) = (s.material_draw_rank_slot(), s.kind());
    let (prepass, leading_sort_bucket) = (s.depth_prepass(), s.leading_sort_bucket());
    SortedDrawParts { object_tag, extra_ix: custom_slot, env_probe_index, dlight_mask, material_draw_rank_slot, key_light_index, kind: surf_type, depth_prepass: prepass, leading_sort_bucket }
}

#[must_use]
pub const fn set_object_tag(base: SortedDraw, object_tag: u16) -> SortedDraw {
    base.with(OBJECT_ID, object_tag as u64)
}

/// R_GetWorldDrawSurf's light write: clears high-dword bits 12-19 (mask 0xfff00fff) and ORs
/// in the light.
#[must_use]
pub const fn with_key_light(base: SortedDraw, key_light_index: u8) -> SortedDraw {
    base.with(LIGHT, key_light_index as u64)
}

#[must_use]
pub const fn with_env_probe_index(
    base: SortedDraw,
    env_probe_index: u8,
) -> SortedDraw {
    base.with(PROBE, env_probe_index as u64)
}

/// The camera BSP add path's per-frame dlight bits: replaces low-dword bits 30-31 and keeps
/// the other 30 baked bits.
#[must_use]
pub const fn with_dlight_mask(base: SortedDraw, dlight_mask: u8) -> SortedDraw {
    base.with(DLIGHTS, dlight_mask as u64)
}

#[must_use]
pub const fn with_surf_type(base: SortedDraw, surf_type: u8) -> SortedDraw {
    base.with(SURF_TYPE, surf_type as u64)
}

#[must_use]
pub const fn with_prepass(base: SortedDraw, prepass: u8) -> SortedDraw {
    base.with(PREPASS, prepass as u64)
}

// BO2 unknown: how the entity and xmodel adders fill objectId and surfType was not traced.
// MW2's rule is kept: the material's baked key with objectId and surfType replaced.

#[must_use]
pub fn pack_rigid_model_surface(material_sort_bits: SortedDraw, object_tag: u16) -> SortedDraw {
    material_sort_bits.with(OBJECT_ID, u64::from(object_tag)).with(SURF_TYPE, u64::from(SURFACE_KIND_XMODEL_RIGID))
}

#[must_use]
pub fn encode_model_bone_driven_sort_key(
    material_sort_bits: SortedDraw,
    object_tag: u16,
) -> SortedDraw {
    let keyed = material_sort_bits.with(OBJECT_ID, u64::from(object_tag));
    keyed.with(SURF_TYPE, u64::from(SURFACE_KIND_MODEL_RIGID_BONED))
}

/// A mesh key: the material's sort row and index, the mesh as the object, and the type.
/// Everything else starts at zero.
fn mesh_key(sort_key: u8, material: u16, mesh: u16, surf_type: u8) -> SortedDraw {
    let row = crate::material_sort_group(sort_key);
    SortedDraw::default()
        .with(SORT_KEY, u64::from(row))
        .with(MATERIAL, u64::from(material))
        .with(SURF_TYPE, u64::from(surf_type))
        .with(OBJECT_ID, u64::from(mesh))
}

#[must_use]
pub fn encode_engine_mesh_sort_key(
    key_byte: u8,
    material_draw_rank_slot: u16,
    mesh_slot: u16,
) -> SortedDraw {
    mesh_key(key_byte, material_draw_rank_slot, mesh_slot, KIND_CODE_MESH)
}

/// The rope code-mesh adder forces surfType ROPE_MESH and prepass 2.
#[must_use]
pub fn pack_rope_mesh_draw_surf(
    key_b: u8,
    material_draw_rank_slot: u16,
    mesh_slot: u16,
) -> SortedDraw {
    mesh_key(key_b, material_draw_rank_slot, mesh_slot, SF_ROPE_MESH).with(PREPASS, 2)
}

/// BO2 unknown: the cloud adder was not traced; MW2's fields are kept.
#[must_use]
pub fn encode_particle_cloud_sort_key(
    key_v: u8,
    material_draw_rank_slot: u16,
    cloud_slot: u16,
) -> SortedDraw {
    mesh_key(key_v, material_draw_rank_slot, cloud_slot, SURFACE_KIND_PARTICLE_CLOUD)
}

/// BO2 unknown: the mark adder was not traced; MW2's fields are kept, with the lightmap in
/// the (9-bit) custom index and the probe cut to BO2's 5 bits.
#[must_use]
pub fn pack_stain_mesh_facet_draw(
    key_u8: u8,
    material_draw_rank_slot: u16,
    decal_slot: u16,
    baked_light_slot_number: u8,
    key_light_index: u8,
    env_probe_index: u8,
) -> SortedDraw {
    mesh_key(key_u8, material_draw_rank_slot, decal_slot, SURFACE_KIND_DECAL_MESH)
        .with(CUSTOM_INDEX, u64::from(baked_light_slot_number))
        .with(LIGHT, u64::from(key_light_index))
        .with(PROBE, u64::from(env_probe_index))
}

/// BO2 unknown: the glass adder was not traced; MW2's fields are kept.
#[must_use]
pub fn encode_window_pane_sort_key(
    key_in: u8,
    material_draw_rank_slot: u16,
    pane_slot: u16,
    env_probe_index: u8,
) -> SortedDraw {
    let mesh = mesh_key(key_in, material_draw_rank_slot, pane_slot, SURFACE_KIND_GLASS_MESH);
    mesh.with(PROBE, u64::from(env_probe_index))
}

/// R_SortDrawSurfs: high dword then low dword, ascending, not stable; the same as an
/// unstable sort on the whole u64.
pub fn order_draws(keys: &mut [SortedDraw]) {
    keys.sort_unstable();
}

/// Material changes along a sorted list: the first key binds one, and every key whose
/// material differs from the key before binds another.
#[must_use]
pub fn material_switch_count(in_order: &[SortedDraw]) -> usize {
    let first = usize::from(!in_order.is_empty());
    let changes = in_order
        .windows(2)
        .filter(|pair| pair[0].material_draw_rank_slot() != pair[1].material_draw_rank_slot())
        .count();
    first + changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_land_in_bo2_bits() {
        let full = SortedDraw::unpack_from_bits(u64::MAX);
        let widths = (
            (full.object_tag(), full.extra_ix(), full.env_probe_index()),
            (full.dlight_mask(), full.material_draw_rank_slot(), full.key_light_index()),
            (full.kind(), full.depth_prepass(), full.leading_sort_bucket()),
        );
        assert_eq!(widths, ((0xffff, 0x1ff, 0x1f), (3, 0xfff, 0xff), (0xf, 3, 0x3f)));
        assert_eq!(pack(unpack(full)), full);

        let lowest_bits: [(fn(&mut SortedDrawParts), u32); 8] = [
            (|f| f.extra_ix = 1, 16),
            (|f| f.env_probe_index = 1, 25),
            (|f| f.dlight_mask = 1, 30),
            (|f| f.material_draw_rank_slot = 1, 32),
            (|f| f.key_light_index = 1, 44),
            (|f| f.kind = 1, 52),
            (|f| f.depth_prepass = 1, 56),
            (|f| f.leading_sort_bucket = 1, 58),
        ];
        for (set, bit) in lowest_bits {
            let mut fields = SortedDrawParts::default();
            set(&mut fields);
            assert_eq!(pack(fields).packed, 1 << bit, "bit {bit}");
        }
    }

    #[test]
    fn unpack_round_trips() {
        let mut f = SortedDrawParts::default();
        (f.object_tag, f.extra_ix, f.env_probe_index) = (0x1234, 0x155, 0x11);
        (f.dlight_mask, f.material_draw_rank_slot, f.key_light_index) = (2, 0xabc, 0x5a);
        (f.kind, f.depth_prepass, f.leading_sort_bucket) = (SURFACE_KIND_DECAL_MESH, 1, 0x2a);
        assert_eq!(unpack(pack(f)), f);
    }

    #[test]
    fn prepass_outranks_surf_type_and_material() {
        let sorted = SortedDraw::default().with(SORT_KEY, 5);
        let lo = sorted.with(SURF_TYPE, 15).with(MATERIAL, 0xfff);
        let hi = sorted.with(PREPASS, 1);
        let mut keys = [hi, lo];
        order_draws(&mut keys);
        assert_eq!(keys, [lo, hi]);
    }

    #[test]
    fn setters_touch_only_their_bits() {
        let base = SortedDraw::unpack_from_bits(u64::MAX);
        assert_eq!(with_key_light(base, 0).packed, !(0xffu64 << 44));
        assert_eq!(with_dlight_mask(base, 0).packed, !(3u64 << 30));
        assert_eq!(with_env_probe_index(base, 0).packed, !(0x1fu64 << 25));
        assert_eq!(set_object_tag(base, 0).packed, !0xffff);
        assert_eq!(with_dlight_mask(SortedDraw::default(), 7).dlight_mask(), 3);
    }

    #[test]
    fn xmodel_keeps_the_baked_material_bits() {
        let baked = SortedDraw::default().with(MATERIAL, 0x321).with(PREPASS, 2).with(SORT_KEY, 9).with(PROBE, 4);
        let f = unpack(encode_model_bone_driven_sort_key(baked, 77));
        assert_eq!((f.object_tag, f.kind), (77, SURFACE_KIND_MODEL_RIGID_BONED));
        assert_eq!((f.material_draw_rank_slot, f.depth_prepass, f.leading_sort_bucket), (0x321, 2, 9));
        assert_eq!(f.env_probe_index, 4);
    }

    #[test]
    fn rope_forces_prepass_two() {
        let f = unpack(pack_rope_mesh_draw_surf(0x45, 0x1abc, 3));
        assert_eq!((f.kind, f.depth_prepass), (SF_ROPE_MESH, 2));
        assert_eq!((f.leading_sort_bucket, f.material_draw_rank_slot), (5, 0xabc));
    }

    #[test]
    fn stage_capacities() {
        let cases = [
            (DRAW_SURF_BSP_CAMERA_LIT, Some(4608)),
            (DRAW_SURF_BSP_CAMERA_LIGHT_MAP, Some(3584)),
            (DRAW_SURF_BSP_CAMERA_LIT_TRANS, Some(512)),
            (DRAW_SURF_BSP_CAMERA_EMISSIVE_TRANS, Some(1024)),
            (30, None),
            (49, Some(1024)),
            (DRAW_SURF_STAGE_COUNT, None),
        ];
        for (stage, want) in cases {
            assert_eq!(stage_capacity(stage), want, "stage {stage}");
        }
    }

    #[test]
    fn rebinds_count_material_changes() {
        let keys = [1, 1, 2, 1].map(|m| SortedDraw::default().with(MATERIAL, m));
        assert_eq!(material_switch_count(&keys), 3);
        assert_eq!(material_switch_count(&[]), 0);
    }
}
