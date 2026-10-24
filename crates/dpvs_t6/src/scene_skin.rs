//! BO2's scene DObj skin pass (R_SkinSceneDObj, R_SkinSceneDObjModels): one entry per
//! surface of each model's LOD, the surfs buffer size (R_GetSkinSurfaceBufferSize), the
//! skinned flag on the cull state (R_FlagXModelAsSkinned) and the weighted vertex blend.

use crate::scene_dobj::{SCENE_DOBJ_CULL_BOUNDED, SCENE_DOBJ_CULL_DONE, SCENE_DOBJ_CULL_SKIN_PENDING};

/// cull.lods: one LOD per model of the DObj. BO2 unknown: the array length was not read;
/// this is MW2's 32.
pub const SCENE_DOBJ_CULL_LOD_COUNT: usize = 32;

/// R_GetSkinSurfaceBufferSize: bytes per XSurface.
pub const SKIN_SURFACE_BYTES: u32 = 0x18;

/// Plus this per vertList of a surface without flag 0x80 (not deformed).
pub const SKIN_VERT_LIST_BYTES: u32 = 0x28;

/// The marker written for a hidden surface.
pub const SKIN_SURFACE_SKIP_TAG: i32 = -2;

/// The frame's surfs buffer. BO2 unknown: its size was not read; this is MW2's 0x20000.
pub const SKIN_FRAME_BYTES: u32 = 0x2_0000;

/// One blend weight unit of a raw u16 weight. BO2 unknown: the weight path's scale was not
/// read; this is MW2's 2^-16.
pub const SKIN_BLEND_WEIGHT_SCALE: f32 = 1.0 / 65536.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreSkinSurface {
    /// R_GetPartBitsForSurfaceAndBoneIndex before the shift: BO2's 5 words.
    pub part_bits: [u32; 5],

    pub vert_count: u16,

    /// XSurface flag 0x80.
    pub deformed: bool,

    pub vert_list_count: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PreSkinModel<'a> {
    /// cull.lods[model]; a model below 0 is not skinned.
    pub lod: i8,

    pub bone_count: u8,

    pub surfaces: &'a [PreSkinSurface],
}

/// One surface of the skin pass. `tag` is the skip marker, the first vertex in the
/// skinned cache (deformed), or below the marker by the vertList count (rigid). BO2
/// unknown: R_PreSkinXSurface's own entry was not read; the deformed and rigid encodings
/// are MW2's, moved under BO2's marker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneEntSkinEntry {
    pub model: u16,

    pub surface: u16,

    pub tag: i32,

    pub bone_base: u8,

    pub bone_count: u8,

    pub bytes: u32,
}

impl SceneEntSkinEntry {
    #[must_use]
    pub fn hidden(self) -> bool {
        self.tag == SKIN_SURFACE_SKIP_TAG
    }

    /// Whether the surface draws: its entry is not hidden, or the ent has no entries.
    #[must_use]
    pub fn stream_draws(entries: &[Self], model: u16, surface: u16) -> bool {
        match entries
            .iter()
            .copied()
            .find(|entry| entry.model == model && entry.surface == surface)
        {
            Some(entry) => !entry.hidden(),
            None => entries.is_empty(),
        }
    }

    #[must_use]
    pub fn rigid_placements(self) -> Option<u32> {
        if self.tag < SKIN_SURFACE_SKIP_TAG {
            u32::try_from(SKIN_SURFACE_SKIP_TAG - self.tag).ok()
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreSkinSummary {
    pub surface_count: u32,

    pub skinned_vert_count: u32,

    /// The surfs buffer size, rounded up to 16.
    pub bytes: u32,

    pub frame_overflow: bool,
}

/// R_GetSkinSurfaceBufferSize for one surface.
#[must_use]
pub fn skin_surface_bytes(surface: &PreSkinSurface) -> u32 {
    if surface.deformed {
        SKIN_SURFACE_BYTES
    } else {
        SKIN_SURFACE_BYTES
            .saturating_add(surface.vert_list_count.saturating_mul(SKIN_VERT_LIST_BYTES))
    }
}

/// The model's part bits moved to its first bone in the DObj (MSB first).
#[must_use]
pub fn shift_part_bits(src: [u32; 5], bone_base: u32) -> [u32; 5] {
    let word_shift = (bone_base >> 5) as usize;
    let bit_shift = bone_base & 31;
    let mut out = [0u32; 5];
    for (i, word) in out.iter_mut().enumerate().skip(word_shift) {
        let low = src[i - word_shift];
        *word = if bit_shift == 0 {
            low
        } else {
            let high = if i > word_shift { src[i - word_shift - 1] << (32 - bit_shift) } else { 0 };
            (low >> bit_shift) | high
        };
    }
    out
}

/// A surface is dropped when its part bits less the hidden ones are all zero, so a
/// surface with one bone left showing still draws. A surface with no part bits at all
/// (none loaded) draws.
#[must_use]
pub fn scene_ent_surface_hidden(part_bits: &[u32; 5], hide: &[u32; 5], bone_base: u32) -> bool {
    let bits = shift_part_bits(*part_bits, bone_base);
    bits.iter().any(|&b| b != 0) && bits.iter().zip(hide).all(|(&b, &h)| b & !h == 0)
}

/// R_SkinSceneDObjModels: one entry per surface of each model's LOD; the bytes are the
/// whole buffer, and a buffer past the frame's surfs leaves nothing skinned.
pub fn pre_skin_scene_ent(
    models: &[PreSkinModel<'_>],
    hide: &[u32; 5],
    frame_bytes_used: u32,
    mut emit: impl FnMut(SceneEntSkinEntry),
) -> PreSkinSummary {
    let mut out = PreSkinSummary::default();
    let mut bone_base = 0u32;
    for (model_index, model) in models.iter().enumerate() {
        if model.lod >= 0 {
            out.surface_count = out.surface_count.saturating_add(model.surfaces.len() as u32);
            for (surface_index, surface) in model.surfaces.iter().enumerate() {
                let tag = if scene_ent_surface_hidden(&surface.part_bits, hide, bone_base) {
                    SKIN_SURFACE_SKIP_TAG
                } else if surface.deformed {
                    let base = out.skinned_vert_count;
                    out.skinned_vert_count =
                        out.skinned_vert_count.saturating_add(u32::from(surface.vert_count));
                    i32::try_from(base).unwrap_or(i32::MAX)
                } else {
                    let lists = i32::try_from(surface.vert_list_count).unwrap_or(i32::MAX);
                    SKIN_SURFACE_SKIP_TAG.saturating_sub(lists)
                };
                let bytes = skin_surface_bytes(surface);
                out.bytes = out.bytes.saturating_add(bytes);
                emit(SceneEntSkinEntry {
                    model: model_index as u16,
                    surface: surface_index as u16,
                    tag,
                    bone_base: bone_base as u8,
                    bone_count: model.bone_count,
                    bytes,
                });
            }
        }
        bone_base = bone_base.saturating_add(u32::from(model.bone_count));
    }
    out.bytes = out.bytes.saturating_add(15) & !15;
    if frame_bytes_used.saturating_add(out.bytes) > SKIN_FRAME_BYTES {
        out.frame_overflow = true;
        out.surface_count = 0;
    }
    out
}

/// R_SkinSceneDObj's claim: bounds built (2) moves to skin pending (3).
#[must_use]
pub fn scene_dobj_skin_begin(state: &mut u32) -> bool {
    if *state == SCENE_DOBJ_CULL_BOUNDED {
        *state = SCENE_DOBJ_CULL_SKIN_PENDING;
        true
    } else {
        false
    }
}

/// R_FlagXModelAsSkinned: only from skin pending; the state becomes surfaceCount + 4.
pub fn scene_dobj_flag_skinned(state: &mut u32, surface_count: u32) {
    if *state == SCENE_DOBJ_CULL_SKIN_PENDING {
        *state = surface_count.saturating_add(SCENE_DOBJ_CULL_DONE);
    }
}

#[must_use]
pub fn scene_dobj_skinned_surface_count(state: u32) -> Option<u32> {
    state.checked_sub(SCENE_DOBJ_CULL_DONE)
}

/// The bone matrix (column-major 4x4) applied to a point.
#[must_use]
pub fn skin_transform_point(p: [f32; 3], m: &[f32; 16]) -> [f32; 3] {
    [
        p[0] * m[0] + p[1] * m[4] + p[2] * m[8] + m[12],
        p[0] * m[1] + p[1] * m[5] + p[2] * m[9] + m[13],
        p[0] * m[2] + p[1] * m[6] + p[2] * m[10] + m[14],
    ]
}

/// A blended vertex: the primary bone takes what the extra weights leave.
#[must_use]
pub fn skin_weighted_point(p: [f32; 3], primary: &[f32; 16], extras: &[(&[f32; 16], u16)]) -> [f32; 3] {
    let base = skin_transform_point(p, primary);
    let mut wsum = 0.0f32;
    let mut extra = [0.0f32; 3];
    for &(m, raw) in extras {
        let w = f32::from(raw) * SKIN_BLEND_WEIGHT_SCALE;
        wsum += w;
        let t = skin_transform_point(p, m);
        extra[0] += w * t[0];
        extra[1] += w * t[1];
        extra[2] += w * t[2];
    }
    let rest = 1.0 - wsum;
    [rest * base[0] + extra[0], rest * base[1] + extra[1], rest * base[2] + extra[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(part0: u32, deformed: bool, lists: u32) -> PreSkinSurface {
        let mut part_bits = [0u32; 5];
        part_bits[0] = part0;
        PreSkinSurface { part_bits, vert_count: 10, deformed, vert_list_count: lists }
    }

    #[test]
    fn hidden_only_when_every_bone_is() {
        let part = [0xc000_0000, 0, 0, 0, 0];
        let mut hide = [0u32; 5];
        hide[0] = 0x8000_0000;
        assert!(!scene_ent_surface_hidden(&part, &hide, 0));
        hide[0] = 0xc000_0000;
        assert!(scene_ent_surface_hidden(&part, &hide, 0));
        // Bone base 31: the second bone lands in word 1.
        let mut hide = [0u32; 5];
        hide[0] = 1;
        assert!(!scene_ent_surface_hidden(&part, &hide, 31));
        hide[1] = 0x8000_0000;
        assert!(scene_ent_surface_hidden(&part, &hide, 31));
        assert!(!scene_ent_surface_hidden(&[0; 5], &[!0; 5], 0));
    }

    #[test]
    fn entries_and_buffer_size() {
        let a = [surface(0x8000_0000, true, 0), surface(0x4000_0000, false, 2)];
        let b = [surface(0x8000_0000, true, 0)];
        let models = [
            PreSkinModel { lod: 0, bone_count: 2, surfaces: &a },
            PreSkinModel { lod: -1, bone_count: 1, surfaces: &b },
            PreSkinModel { lod: 1, bone_count: 1, surfaces: &b },
        ];
        // Hide bone 3: the last model's only bone.
        let mut hide = [0u32; 5];
        hide[0] = 0x1000_0000;
        let mut entries = alloc::vec::Vec::new();
        let sum = pre_skin_scene_ent(&models, &hide, 0, |e| entries.push(e));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].tag, 0);
        assert_eq!(entries[1].rigid_placements(), Some(2));
        assert!(entries[2].hidden() && entries[2].bone_base == 3);
        assert_eq!(sum.surface_count, 3);
        assert_eq!(sum.skinned_vert_count, 10);
        // 0x18 + (0x18 + 2 * 0x28) + 0x18 = 0x98, rounded to 0xa0.
        assert_eq!(sum.bytes, 0xa0);
        assert!(SceneEntSkinEntry::stream_draws(&entries, 0, 0));
        assert!(!SceneEntSkinEntry::stream_draws(&entries, 2, 0));
        let over = pre_skin_scene_ent(&models, &hide, SKIN_FRAME_BYTES - 0x10, |_| {});
        assert!(over.frame_overflow && over.surface_count == 0);
    }

    #[test]
    fn skinned_state() {
        let mut state = SCENE_DOBJ_CULL_BOUNDED;
        scene_dobj_flag_skinned(&mut state, 5);
        assert_eq!(state, SCENE_DOBJ_CULL_BOUNDED);
        assert!(scene_dobj_skin_begin(&mut state));
        assert!(!scene_dobj_skin_begin(&mut state));
        scene_dobj_flag_skinned(&mut state, 5);
        assert_eq!(scene_dobj_skinned_surface_count(state), Some(5));
        assert_eq!(scene_dobj_skinned_surface_count(SCENE_DOBJ_CULL_SKIN_PENDING), None);
    }

    #[test]
    fn weighted_blend() {
        let mut a = [0.0f32; 16];
        a[0] = 1.0;
        a[5] = 1.0;
        a[10] = 1.0;
        a[15] = 1.0;
        let mut b = a;
        b[12] = 4.0;
        let p = skin_weighted_point([1.0, 2.0, 3.0], &a, &[(&b, 0x4000)]);
        assert_eq!(p, [2.0, 2.0, 3.0]);
    }
}
