//! BO2's scene DObj skin pass (R_SkinSceneDObj, R_SkinSceneDObjModels): one entry per
//! surface of each model's LOD, the surfs buffer size (R_GetSkinSurfaceBufferSize), the
//! skinned flag on the cull state (R_FlagXModelAsSkinned) and the weighted vertex blend.

use crate::stage_puppet::{SCENE_DOBJ_CULL_BOUNDED, SCENE_DOBJ_CULL_DONE, SCENE_DOBJ_CULL_SKIN_PENDING};

/// cull.detail_tiers: one LOD per model of the DObj. BO2 unknown: the array length was not read;
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
pub const SKIN_WEIGHT_UNIT: f32 = 1.0 / 65536.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreposedSurfaceInput {
    /// R_GetPartBitsForSurfaceAndBoneIndex before the shift: BO2's 5 words.
    pub hide_bits: [u32; 5],
    pub vertex_tally: u16,
    /// XSurface flag 0x80.
    pub deformed: bool,
    pub vert_lists: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PreposedModelInput<'a> {
    /// cull.detail_tiers[model]; a model below 0 is not skinned.
    pub lod: i8,
    pub joint_total: u8,
    pub surfaces: &'a [PreposedSurfaceInput],
}

/// One surface of the skin pass. `tag` is the skip marker, the first vertex in the
/// skinned cache (deformed), or below the marker by the vertList count (rigid). BO2
/// unknown: R_PreSkinXSurface's own entry was not read; the deformed and rigid encodings
/// are MW2's, moved under BO2's marker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySkinSlotRecord {
    pub mdl: u16,
    pub surface_no: u16,
    pub bone_tag: i32,
    pub joint_start_offset: u8,
    pub joint_total: u8,
    pub bytes: u32,
}

impl EntitySkinSlotRecord {
    #[must_use]
    pub fn is_hidden(self) -> bool {
        self.bone_tag == SKIN_SURFACE_SKIP_TAG
    }

    /// Whether the surface draws: its entry is not hidden, or the ent has no entries.
    #[must_use]
    pub fn draws_streamed(entries: &[Self], model_ix: u16, surf_ix: u16) -> bool {
        if entries.is_empty() {
            return true;
        }
        let mut same = entries.iter().filter(|e| (e.mdl, e.surface_no) == (model_ix, surf_ix));
        same.next().is_some_and(|e| !e.is_hidden())
    }

    /// How many vertLists a rigid surface places, read back from its tag.
    #[must_use]
    pub fn fixed_mesh_instance_total(self) -> Option<u32> {
        let below = i64::from(SKIN_SURFACE_SKIP_TAG) - i64::from(self.bone_tag);
        u32::try_from(below).ok().filter(|&n| n != 0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreSkinningDigest {
    pub facet_tally: u32,
    pub deformed_vertex_total: u32,
    /// The surfs buffer size, rounded up to 16.
    pub bytes: u32,
    pub frame_budget_exceeded: bool,
}

/// R_GetSkinSurfaceBufferSize for one surface: a fixed part, plus room for each
/// vertList when the surface is rigid.
#[must_use]
pub fn skin_surface_bytes(surf: &PreposedSurfaceInput) -> u32 {
    let lists = if surf.deformed { 0 } else { surf.vert_lists };
    lists.saturating_mul(SKIN_VERT_LIST_BYTES).saturating_add(SKIN_SURFACE_BYTES)
}

/// The model's part bits moved to its first bone in the DObj (MSB first): the
/// five words read as one 160-bit number shifted right by `joint_start_offset`.
#[must_use]
pub fn shift_part_bits(src: [u32; 5], joint_start_offset: u32) -> [u32; 5] {
    let words = (joint_start_offset / 32) as usize;
    let bits = joint_start_offset % 32;
    core::array::from_fn(|i| {
        let Some(from) = i.checked_sub(words) else {
            return 0;
        };
        let above = from.checked_sub(1).map_or(0, |j| src[j]);
        let pair = (u64::from(above) << 32) | u64::from(src[from]);
        (pair >> bits) as u32
    })
}

/// A surface is dropped when its part bits less the hidden ones are all zero, so a
/// surface with one bone left showing still draws. A surface with no part bits at all
/// (none loaded) draws.
#[must_use]
pub fn scene_surface_is_hidden(surf_parts: &[u32; 5], hide_mask: &[u32; 5], joint_start_offset: u32) -> bool {
    let moved = shift_part_bits(*surf_parts, joint_start_offset);
    let has_bones = moved.iter().any(|&w| w != 0);
    let showing = moved.iter().zip(hide_mask).any(|(&w, &h)| w & !h != 0);
    has_bones && !showing
}

/// R_SkinSceneDObjModels: one entry per surface of each model's LOD; the bytes are the
/// whole buffer, and a buffer past the frame's surfs leaves nothing skinned.
pub fn bake_scene_entity_bones_early(
    meshes_list: &[PreposedModelInput<'_>],
    masked: &[u32; 5],
    frame_bytes_consumed: u32,
    mut emit: impl FnMut(EntitySkinSlotRecord),
) -> PreSkinningDigest {
    let mut sum = PreSkinningDigest::default();
    let mut first_bone = 0u32;
    for (m, mesh) in meshes_list.iter().enumerate() {
        let next_bone = first_bone.saturating_add(u32::from(mesh.joint_total));
        let joint_start_offset = core::mem::replace(&mut first_bone, next_bone);
        // A negative LOD means the mesh is not drawn, so not skinned; its bones still count.
        if mesh.lod.is_negative() {
            continue;
        }
        sum.facet_tally = sum.facet_tally.saturating_add(mesh.surfaces.len() as u32);
        for (s, surface) in mesh.surfaces.iter().enumerate() {
            let tag = match (scene_surface_is_hidden(&surface.hide_bits, masked, joint_start_offset), surface.deformed) {
                (true, _) => SKIN_SURFACE_SKIP_TAG,
                (false, true) => {
                    let first_vert = sum.deformed_vertex_total;
                    sum.deformed_vertex_total = first_vert.saturating_add(u32::from(surface.vertex_tally));
                    i32::try_from(first_vert).unwrap_or(i32::MAX)
                }
                (false, false) => {
                    let lists = i32::try_from(surface.vert_lists).unwrap_or(i32::MAX);
                    SKIN_SURFACE_SKIP_TAG.saturating_sub(lists)
                }
            };
            let bytes = skin_surface_bytes(surface);
            sum.bytes = sum.bytes.saturating_add(bytes);
            emit(EntitySkinSlotRecord {
                mdl: m as u16,
                surface_no: s as u16,
                bone_tag: tag,
                joint_start_offset: joint_start_offset as u8,
                joint_total: mesh.joint_total,
                bytes,
            });
        }
    }
    sum.bytes = sum.bytes.saturating_add(15) / 16 * 16;
    sum.frame_budget_exceeded = frame_bytes_consumed.saturating_add(sum.bytes) > SKIN_FRAME_BYTES;
    if sum.frame_budget_exceeded {
        sum.facet_tally = 0;
    }
    sum
}

/// R_SkinSceneDObj's claim: bounds built (2) moves to skin pending (3).
#[must_use]
pub fn scene_dobj_skin_begin(state: &mut u32) -> bool {
    let claimed = *state == SCENE_DOBJ_CULL_BOUNDED;
    if claimed {
        *state = SCENE_DOBJ_CULL_SKIN_PENDING;
    }
    claimed
}

/// R_FlagXModelAsSkinned: only from skin pending; the state becomes surfaceCount + 4.
pub fn scene_dobj_flag_skinned(state: &mut u32, facet_tally: u32) {
    if *state != SCENE_DOBJ_CULL_SKIN_PENDING {
        return;
    }
    *state = SCENE_DOBJ_CULL_DONE.saturating_add(facet_tally);
}

#[must_use]
pub fn scene_dobj_skinned_surface_count(state: u32) -> Option<u32> {
    state.checked_sub(SCENE_DOBJ_CULL_DONE)
}

/// The bone matrix (column-major 4x4) applied to a point.
#[must_use]
pub fn skin_transform_point(pt: [f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    let [x, y, z] = pt;
    core::array::from_fn(|row| x * mat[row] + y * mat[4 + row] + z * mat[8 + row] + mat[12 + row])
}

/// A blended vertex: the primary bone takes what the extra weights leave.
#[must_use]
pub fn skin_weighted_point(at_p: [f32; 3], base_mat: &[f32; 16], blend_mats: &[(&[f32; 16], u16)]) -> [f32; 3] {
    let (taken, blended) = blend_mats.iter().fold((0.0f32, [0.0f32; 3]), |(taken, mut acc), &(bone, raw)| {
        let weight = f32::from(raw) * SKIN_WEIGHT_UNIT;
        let moved = skin_transform_point(at_p, bone);
        for (a, v) in acc.iter_mut().zip(moved) {
            *a += weight * v;
        }
        (taken + weight, acc)
    });
    let own = skin_transform_point(at_p, base_mat);
    let left = 1.0 - taken;
    core::array::from_fn(|k| left * own[k] + blended[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surf(part0: u32, deformed: bool, list_n: u32) -> PreposedSurfaceInput {
        let mut got = PreposedSurfaceInput { vertex_tally: 10, deformed, vert_lists: list_n, ..Default::default() };
        got.hide_bits[0] = part0;
        got
    }

    fn words(first: u32, second: u32) -> [u32; 5] {
        [first, second, 0, 0, 0]
    }

    #[test]
    fn hidden_only_when_every_bone_is() {
        let part = words(0xc000_0000, 0);
        assert!(!scene_surface_is_hidden(&part, &words(0x8000_0000, 0), 0));
        assert!(scene_surface_is_hidden(&part, &words(0xc000_0000, 0), 0));
        // Bone base 31: the second bone lands in word 1.
        assert!(!scene_surface_is_hidden(&part, &words(1, 0), 31));
        assert!(scene_surface_is_hidden(&part, &words(1, 0x8000_0000), 31));
        assert!(!scene_surface_is_hidden(&[0; 5], &[!0; 5], 0));
    }

    #[test]
    fn part_bits_shift_across_words_and_off_the_end() {
        assert_eq!(shift_part_bits(words(0x8000_0001, 0), 1), words(0x4000_0000, 0x8000_0000));
        assert_eq!(shift_part_bits(words(7, 0), 32), words(0, 7));
        assert_eq!(shift_part_bits([!0; 5], 160), [0; 5]);
    }

    #[test]
    fn entries_and_buffer_size() {
        let a = [surf(0x8000_0000, true, 0), surf(0x4000_0000, false, 2)];
        let b = [surf(0x8000_0000, true, 0)];
        let lineup = [
            PreposedModelInput { lod: 0, joint_total: 2, surfaces: &a },
            PreposedModelInput { lod: -1, joint_total: 1, surfaces: &b },
            PreposedModelInput { lod: 1, joint_total: 1, surfaces: &b },
        ];
        // Hide bone 3: the last model's only bone.
        let hide = words(0x1000_0000, 0);
        let mut entries = alloc::vec::Vec::new();
        let sum = bake_scene_entity_bones_early(&lineup, &hide, 0, |e| entries.push(e));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].bone_tag, 0);
        assert_eq!(entries[1].fixed_mesh_instance_total(), Some(2));
        assert!(entries[2].is_hidden() && entries[2].joint_start_offset == 3);
        assert_eq!((sum.facet_tally, sum.deformed_vertex_total), (3, 10));
        // 0x18 + (0x18 + 2 * 0x28) + 0x18 = 0x98, rounded to 0xa0.
        assert_eq!(sum.bytes, 0xa0);
        assert!(EntitySkinSlotRecord::draws_streamed(&entries, 0, 0));
        assert!(!EntitySkinSlotRecord::draws_streamed(&entries, 2, 0));
        assert!(EntitySkinSlotRecord::draws_streamed(&[], 2, 0));
        let over = bake_scene_entity_bones_early(&lineup, &hide, SKIN_FRAME_BYTES - 0x10, |_| {});
        assert!(over.frame_budget_exceeded && over.facet_tally == 0);
    }

    #[test]
    fn only_tags_below_the_marker_are_rigid() {
        let with_tag = |tag| EntitySkinSlotRecord { mdl: 0, surface_no: 0, bone_tag: tag, joint_start_offset: 0, joint_total: 0, bytes: 0 };
        assert_eq!(with_tag(SKIN_SURFACE_SKIP_TAG).fixed_mesh_instance_total(), None);
        assert_eq!(with_tag(5).fixed_mesh_instance_total(), None);
        assert_eq!(with_tag(i32::MIN).fixed_mesh_instance_total(), Some((-2i64 - i64::from(i32::MIN)) as u32));
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
        for k in [0, 5, 10, 15] {
            a[k] = 1.0;
        }
        let mut b = a;
        b[12] = 4.0;
        assert_eq!(skin_weighted_point([1.0, 2.0, 3.0], &a, &[(&b, 0x4000)]), [2.0, 2.0, 3.0]);
    }
}
