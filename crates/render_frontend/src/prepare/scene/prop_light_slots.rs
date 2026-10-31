//! Static-model lighting: deciding which in-view prop owns a tile of the
//! model-lighting atlas each frame, and copying freshly granted tiles into it.

use bevy::prelude::*;
use lighting_t6::{
    ILLUM_TILE_LEN, ModelIllumAtlasDims, IllumTileNo,
    STATIC_LIGHT_OVERUSE_WARN_LIMIT, StaleIllumAction, SModelIllumAlloc,
    SModelIllumCounters, StaticModelLightingBlob, stale_illum_action,
    prop_illum_word_count,
};

use crate::prepare::scene::cull::prop_hide_dist_skips_slot;
use crate::prepare::scene::model_illum_atlas::{
    LevelModelIllumAtlas, light_atlas_store_tile,
};
use super::prop_geom_memo::{DetailRampTunable, LevelPropCache};
use crate::prepare::scene::eye_parms::{StagedStageEye, FrozenVisibilitySet};
use super::world::{LevelHide, LevelStage, StaticPropLightSample};


/// Per-prop host flags handed to the staleness check. We track none, so every
/// stale prop is resampled on the spot.
const NO_HOST_FLAGS: u8 = 0;

/// Static-model lighting runs unless `BO2MP_SMODEL_LIGHTING` says otherwise.
fn prop_illum_enabled() -> bool {
    let switch = std::env::var("BO2MP_SMODEL_LIGHTING").unwrap_or_default();
    !["0", "false", "off", "no"].contains(&switch.as_str())
}

/// World resource: authored lighting plus live atlas ownership, indexed by
/// static model slot.
#[derive(Resource)]
pub struct LevelPropIllum {
    entry_cap: u32,
    /// 256 / atlas height, used by the shader lookup.
    pub inv_height_x256: u32,
    /// Authored tile per slot (None where the map shipped no sample).
    pub slot_tiles: Vec<Option<[u8; ILLUM_TILE_LEN]>>,
    /// Authored packed lighting word per slot, same coverage as `tiles`.
    pub compact_illum: Vec<Option<[u8; 4]>>,
    /// Atlas handle currently owned by each slot (0 = owns nothing).
    pub handle_list: Vec<u16>,
    /// Entity that was spawned to draw each slot.
    pub parent_list: Vec<Option<Entity>>,
    alloc_counters: SModelIllumCounters,
    free_entries: Vec<u16>,
    entry_owner: Vec<u16>,
    entry_last_used: Vec<i32>,
    stale_bits: Vec<u32>,
    warned_atlas_full: bool,
    /// How many slots came out lit at world spawn.
    pub spawn_lit_tally: u32,
    /// How many sampled slots had no material technique able to use it.
    pub spawn_sample_without_method: u32,
}

impl LevelPropIllum {
    pub fn new(
        dims: ModelIllumAtlasDims,
        slot_total: usize,
        samples: &[StaticPropLightSample],
    ) -> Self {
        let mut authored_tiles: Vec<Option<[u8; ILLUM_TILE_LEN]>> = vec![None; slot_total];
        let mut authored_words: Vec<Option<[u8; 4]>> = vec![None; slot_total];
        samples
            .iter()
            .filter(|s| s.designer_slot < slot_total)
            .for_each(|s| {
                authored_tiles[s.designer_slot] = Some(s.tile_px);
                authored_words[s.designer_slot] = Some(s.compact_illum);
            });

        let cap = dims.prop_entry_limit;
        let per_entry = cap as usize;
        Self {
            entry_cap: cap,
            inv_height_x256: 256 / dims.rows_tall,
            slot_tiles: authored_tiles,
            compact_illum: authored_words,
            handle_list: vec![0u16; slot_total],
            parent_list: std::iter::repeat_n(None, slot_total).collect(),
            alloc_counters: SModelIllumCounters::POST_LIGHT_TRAVERSAL,
            free_entries: vec![0u16; per_entry],
            entry_owner: vec![0u16; per_entry],
            entry_last_used: vec![0i32; per_entry],
            stale_bits: vec![0u32; prop_illum_word_count(slot_total as u32)],
            warned_atlas_full: false,
            spawn_lit_tally: 0,
            spawn_sample_without_method: 0,
        }
    }

    /// Lends out the allocator view over our tables alongside the slot handle
    /// table. If the table sizes are inconsistent, reports it under `stage`
    /// and gives back None.
    fn borrow_allocator(
        &mut self,
        stage: &str,
    ) -> Option<(StaticModelLightingBlob<'_>, &mut [u16])> {
        let cap = self.entry_cap;
        let seat_count = self.slot_tiles.len() as u32;
        let view = StaticModelLightingBlob::new(
            cap,
            seat_count,
            &mut self.alloc_counters,
            &mut self.free_entries,
            &mut self.entry_owner,
            &mut self.entry_last_used,
            &mut self.stale_bits,
        );
        if view.is_err() {
            diag::error!(
                World,
                "smodel lighting {stage}: glob borrow failed, entryLimit={cap} slots={seat_count}"
            );
        }
        view.ok().map(|v| (v, &mut self.handle_list[..]))
    }

    #[inline]
    pub fn compact_illum_for_slot(&self, seat: usize) -> Option<[u8; 4]> {
        *self.compact_illum.get(seat)?
    }
}

/// Builds the resource, or None if the feature is switched off or the map
/// ships no samples at all.
pub(crate) fn prepare_prop_illum(
    dims: ModelIllumAtlasDims,
    samples: &[StaticPropLightSample],
    slot_total: usize,
) -> Option<LevelPropIllum> {
    (prop_illum_enabled() && !samples.is_empty())
        .then(|| LevelPropIllum::new(dims, slot_total, samples))
}

pub(crate) fn slot_has_lighting_sample(lighting: &LevelPropIllum, tile: usize) -> bool {
    matches!(lighting.slot_tiles.get(tile), Some(Some(_)))
}

/// Whether at least one surface material samples the model-lighting constant.
pub(crate) fn facets_take_model_illum(
    facet_materials: impl IntoIterator<Item = Option<assets::SurfaceMaterialId>>,
    materials: &crate::assemble::draw_items::LiveMaterialLedger,
) -> bool {
    facet_materials
        .into_iter()
        .flatten()
        .filter_map(|id| materials.derived(id))
        .any(|derived| derived.uses_model_illum_const)
}

pub(crate) fn seat_is_lit_candidate(
    lighting: &LevelPropIllum,
    index: usize,
    facets_take_model_illum: bool,
) -> bool {
    if !facets_take_model_illum {
        return false;
    }
    slot_has_lighting_sample(lighting, index)
}

/// Every slot that should own a tile this frame: it has a sample, it was
/// spawned, the cull marked it visible and it is inside its draw distance.
fn slots_needing_tiles(
    lighting: &LevelPropIllum,
    scene: &LevelStage,
    cull: &LevelHide,
    eye_pos: Option<Vec3>,
    detail_scale: Option<f32>,
) -> Vec<usize> {
    let wants_tile = |slot: usize| -> bool {
        let spawned = matches!(lighting.parent_list.get(slot), Some(Some(_)));
        let visible = cull.prop_vis.get(slot).map_or(false, |&v| v != 0);
        if !(slot_has_lighting_sample(lighting, slot) && spawned && visible) {
            return false;
        }
        let at = match scene.prop_instances.get(slot) {
            Some(Some(placed)) => placed.origin,
            _ => [0.0; 3],
        };
        !prop_hide_dist_skips_slot(&cull.prop_hide_dists, slot, at, eye_pos, detail_scale)
    };
    (0..lighting.slot_tiles.len()).filter(|&s| wants_tile(s)).collect()
}

/// Asks the allocator for an atlas handle for each requested slot, stealing
/// from idle props once the atlas is full. Yields the number of slots left
/// without a handle, or None when the allocator view could not be built.
fn assign_handles(
    lighting: &mut LevelPropIllum,
    slots: &[usize],
    mut geom_cache: Option<&mut LevelPropCache>,
) -> Option<u32> {
    let (mut alloc, owned) = lighting.borrow_allocator("allocation")?;
    alloc.flip_frame();
    alloc.reset_changed_flags();

    let mut unserved = 0u32;
    for slot in slots.iter().copied() {
        let granted = match alloc.reserve_static_model_light_slot(slot as u32, owned[slot]) {
            SModelIllumAlloc::Recycled { handle } | SModelIllumAlloc::SlotFilled { handle } => {
                handle
            }
            SModelIllumAlloc::Evicted { handle, evicted_prop_index } => {
                let loser = usize::from(evicted_prop_index);
                owned[loser] = 0;
                if let Some(cached) = geom_cache.as_deref_mut() {
                    cached.release_slot(loser);
                }
                handle
            }
            SModelIllumAlloc::Failed => {
                unserved += 1;
                0
            }
        };
        owned[slot] = granted;
    }
    Some(unserved)
}

/// Pairs of (slot, atlas handle) whose tile has to be uploaded now. Props
/// that would want a deferred patch are tallied and reported instead.
fn dirty_tile_slots(lighting: &mut LevelPropIllum) -> Option<Vec<(usize, u16)>> {
    let mut stale_props: Vec<u32> = Vec::new();
    lighting
        .borrow_allocator("dirty walk")?
        .0
        .update_dirty_smodel_lighting(|prop| stale_props.push(prop));

    let owned = &lighting.handle_list;
    let mut deferred = 0u32;
    let uploads: Vec<(usize, u16)> = stale_props
        .into_iter()
        .filter_map(|prop| {
            let held = owned.get(prop as usize).map_or(0, |&h| h);
            match stale_illum_action(prop, held, NO_HOST_FLAGS)? {
                StaleIllumAction::SampleNow { entry, .. } => {
                    let next = entry.wrapping_add(1);
                    Some((prop as usize, next))
                }
                StaleIllumAction::QueuePatch { .. } => {
                    deferred += 1;
                    None
                }
            }
        })
        .collect();

    if deferred != 0 {
        diag::error!(
            World,
            "smodel lighting: {deferred} dirty smodel(s) want EnqueuePatch, \
             but lighting patch upload is missing"
        );
    }
    Some(uploads)
}

/// Copies the authored tile for every (slot, handle) pair into the atlas.
fn write_tiles(
    gpu_images: &mut Assets<Image>,
    lit_atlas: &LevelModelIllumAtlas,
    tiles: &[Option<[u8; ILLUM_TILE_LEN]>],
    writes: &[(usize, u16)],
) {
    let Some(mut image) = gpu_images.get_mut(&lit_atlas.image) else {
        return;
    };
    let resolved = writes.iter().filter_map(|&(seat, handle)| {
        let tile = tiles.get(seat)?.as_ref()?;
        let place = IllumTileNo::from_handle(handle)?;
        Some((seat, handle, place, tile))
    });
    for (slot, handle, place, tile) in resolved {
        let stored = light_atlas_store_tile(&mut image, lit_atlas.dims, place, tile);
        if !stored {
            diag::error!(
                World,
                "smodel lighting: atlas refused the tile of slot {slot} (handle {handle})"
            );
        }
    }
}

pub(crate) fn update_prop_illum(
    mut lighting: Option<ResMut<LevelPropIllum>>,
    mut geom_cache: Option<ResMut<LevelPropCache>>,
    atlas: Option<Res<LevelModelIllumAtlas>>,
    scene: Res<LevelStage>,
    mut gpu_images: ResMut<Assets<Image>>,
    prepared: Res<StagedStageEye>,
    lock_sight: Res<FrozenVisibilitySet>,
    detail_ramp: Res<DetailRampTunable>,
) {
    let Some(state) = lighting.as_deref_mut() else {
        return;
    };
    let Some(lit_atlas) = atlas else {
        return;
    };
    let Some(cull) = scene.culling.as_ref() else {
        return;
    };

    let eye = if prepared.primed {
        Some(lock_sight.walk_eye(&prepared))
    } else {
        None
    };
    let asked = slots_needing_tiles(state, &scene, cull, eye, detail_ramp.ramp_end_scale);

    let Some(unserved) = assign_handles(state, &asked, geom_cache.as_deref_mut()) else {
        return;
    };
    if unserved != 0 && !state.warned_atlas_full {
        state.warned_atlas_full = true;
        diag::error!(
            World,
            "static model lighting atlas is full (limit 0x{STATIC_LIGHT_OVERUSE_WARN_LIMIT:x}): {unserved} \
             lit static models in view go unlit this frame, entry cap {}",
            state.entry_cap
        );
    }

    if let Some(uploads) = dirty_tile_slots(state) {
        write_tiles(&mut gpu_images, &lit_atlas, &state.slot_tiles, &uploads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(smodel_entries: u32) -> ModelIllumAtlasDims {
        ModelIllumAtlasDims {
            figure_entry_limit: 8,
            prop_entry_limit: smodel_entries,
            rows_tall: 64,
        }
    }

    fn sample(slot: usize, shade: u8) -> StaticPropLightSample {
        StaticPropLightSample {
            designer_slot: slot,
            illum_origin: [0.0; 3],
            tile_px: [shade; 256],
            compact_illum: [shade, 0, 0, 0xff],
        }
    }

    /// Four slots, all with a sample, and room for three tiles: entry 0 of the
    /// atlas is never given out.
    fn lit(slots: usize) -> LevelPropIllum {
        let samples: Vec<_> = (0..slots).map(|slot| sample(slot, slot as u8 + 1)).collect();
        LevelPropIllum::new(dims(4), slots, &samples)
    }

    #[test]
    fn each_slot_holds_the_sample_authored_for_it() {
        let lighting = LevelPropIllum::new(dims(4), 3, &[sample(1, 9), sample(7, 5)]);
        assert_eq!(lighting.slot_tiles.len(), 3);
        assert_eq!(lighting.slot_tiles[1], Some([9; 256]));
        assert_eq!(lighting.compact_illum_for_slot(1), Some([9, 0, 0, 0xff]));
        // Slot 7 does not exist, and the slots nobody sampled have nothing.
        assert!(lighting.slot_tiles[0].is_none() && lighting.slot_tiles[2].is_none());
        assert_eq!(lighting.compact_illum_for_slot(0), None);
        assert_eq!(lighting.compact_illum_for_slot(7), None);
        assert!(lighting.handle_list.iter().all(|&handle| handle == 0));
        assert!(lighting.parent_list.iter().all(Option::is_none));
    }

    #[test]
    fn the_lookup_scale_follows_the_atlas_height() {
        let lighting = LevelPropIllum::new(dims(4), 1, &[]);
        assert_eq!(lighting.inv_height_x256, 4);
    }

    #[test]
    fn a_slot_is_a_lit_candidate_only_with_both_a_sample_and_a_material_that_wants_it() {
        let lighting = LevelPropIllum::new(dims(4), 3, &[sample(1, 2)]);
        assert!(slot_has_lighting_sample(&lighting, 1));
        assert!(!slot_has_lighting_sample(&lighting, 0));
        assert!(!slot_has_lighting_sample(&lighting, 50));
        assert!(seat_is_lit_candidate(&lighting, 1, true));
        assert!(!seat_is_lit_candidate(&lighting, 1, false));
        assert!(!seat_is_lit_candidate(&lighting, 0, true));
    }

    #[test]
    fn no_samples_means_no_static_model_lighting() {
        assert!(prepare_prop_illum(dims(4), &[], 3).is_none());
    }

    #[test]
    fn samples_give_the_world_static_model_lighting() {
        // Unless the environment switches static-model lighting off.
        if !prop_illum_enabled() {
            return;
        }
        let lighting = prepare_prop_illum(dims(4), &[sample(0, 3)], 2).expect("lighting");
        assert_eq!(lighting.slot_tiles.len(), 2);
    }

    #[test]
    fn visible_slots_take_atlas_entries_after_the_reserved_one() {
        let mut lighting = lit(4);
        assert_eq!(assign_handles(&mut lighting, &[0, 1, 2], None), Some(0));
        assert_eq!(&lighting.handle_list[..3], [2, 3, 4]);
        assert_eq!(lighting.handle_list[3], 0);
    }

    #[test]
    fn a_slot_that_already_holds_a_handle_keeps_it() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1], None);
        let before = lighting.handle_list.clone();
        assert_eq!(assign_handles(&mut lighting, &[0, 1], None), Some(0));
        assert_eq!(lighting.handle_list, before);
    }

    #[test]
    fn a_full_atlas_leaves_the_extra_slots_without_a_handle_and_counts_them() {
        let mut lighting = lit(4);
        assert_eq!(assign_handles(&mut lighting, &[0, 1, 2, 3], None), Some(1));
        assert_eq!(lighting.handle_list[3], 0);
        assert!(lighting.handle_list[..3].iter().all(|&handle| handle != 0));
    }

    #[test]
    fn a_model_idle_for_four_frames_gives_its_tile_to_a_new_one() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        // Two frames in which nobody is wanted: with the one that asks, only
        // three frames since they were used.
        for _ in 0..2 {
            assign_handles(&mut lighting, &[], None);
        }
        assert_eq!(assign_handles(&mut lighting, &[3], None), Some(1));
        assert_eq!(lighting.handle_list[3], 0);

        // One frame later they have been idle four frames; the newest
        // allocation is taken first.
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        for _ in 0..3 {
            assign_handles(&mut lighting, &[], None);
        }
        assert_eq!(assign_handles(&mut lighting, &[3], None), Some(0));
        assert_eq!(lighting.handle_list[3], 4);
        assert_eq!(lighting.handle_list[2], 0);
        assert_eq!(&lighting.handle_list[..2], [2, 3]);
    }

    #[test]
    fn a_model_used_this_frame_is_never_evicted() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        for _ in 0..3 {
            assign_handles(&mut lighting, &[], None);
        }
        // Slot 2 is wanted again, so slot 3 must take the tile of slot 1
        // instead, which was handed out just before it.
        assert_eq!(assign_handles(&mut lighting, &[2, 3], None), Some(0));
        assert_eq!(lighting.handle_list, [2, 0, 4, 3]);
    }

    #[test]
    fn tables_that_do_not_fit_the_allocator_stop_the_pass() {
        let mut lighting = LevelPropIllum::new(dims(70_000), 3, &[sample(0, 1)]);
        assert_eq!(assign_handles(&mut lighting, &[0], None), None);
        assert!(dirty_tile_slots(&mut lighting).is_none());
    }

    #[test]
    fn a_newly_seated_slot_is_dirty_once_and_a_kept_one_is_not() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        assert_eq!(
            dirty_tile_slots(&mut lighting),
            Some(vec![(0, 2), (1, 3), (2, 4)])
        );
        assert_eq!(dirty_tile_slots(&mut lighting), Some(Vec::new()));
        assign_handles(&mut lighting, &[0, 1, 2], None);
        assert_eq!(dirty_tile_slots(&mut lighting), Some(Vec::new()));
    }

    #[test]
    fn only_the_slots_given_a_tile_this_frame_are_written() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1], None);
        dirty_tile_slots(&mut lighting);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        assert_eq!(dirty_tile_slots(&mut lighting), Some(vec![(2, 4)]));
    }

    fn real_atlas() -> (Assets<Image>, LevelModelIllumAtlas) {
        let mut images = Assets::<Image>::default();
        let dims = lighting_t6::illum_atlas_dims(1).expect("dims");
        let atlas = LevelModelIllumAtlas::new(&mut images, dims);
        (images, atlas)
    }

    fn pattern_tile() -> [u8; ILLUM_TILE_LEN] {
        let mut cell_tile = [0u8; ILLUM_TILE_LEN];
        for (i, byte) in cell_tile.iter_mut().enumerate() {
            *byte = (i % 251) as u8 + 1;
        }
        cell_tile
    }

    #[test]
    fn a_tile_lands_in_the_atlas_at_its_handles_place() {
        let (mut images, atlas) = real_atlas();
        let tile = pattern_tile();
        write_tiles(&mut images, &atlas, &[Some(tile)], &[(0, 2)]);

        let data = images.get(&atlas.image).unwrap().data.clone().unwrap();
        let row_pitch = lighting_t6::MODEL_LIGHT_ATLAS_TEXELS_ACROSS as usize * 4;
        let slice_pitch = row_pitch * atlas.dims.rows_tall as usize;
        // Handle 2 is tile 1: four texels in from the left, in the top row.
        let at = 4 * 4;
        assert_eq!(data[at..at + 16], tile[0..16]);
        // Its second row sits one atlas row down, and layer 1 starts at tile[64].
        assert_eq!(data[row_pitch + at..row_pitch + at + 16], tile[16..32]);
        assert_eq!(data[slice_pitch + at..slice_pitch + at + 16], tile[64..80]);
        // The neighbouring tile is untouched.
        assert!(data[..16].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn slots_without_a_sample_or_a_handle_write_nothing() {
        let (mut images, atlas) = real_atlas();
        let tiles = [Some(pattern_tile()), None];
        write_tiles(&mut images, &atlas, &tiles, &[(1, 2), (0, 0), (5, 3)]);
        let data = images.get(&atlas.image).unwrap().data.clone().unwrap();
        assert!(data.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn an_atlas_whose_image_is_gone_is_left_alone() {
        let mut images = Assets::<Image>::default();
        let atlas = LevelModelIllumAtlas {
            image: Handle::default(),
            dims: lighting_t6::illum_atlas_dims(1).expect("dims"),
        };
        write_tiles(&mut images, &atlas, &[Some(pattern_tile())], &[(0, 2)]);
    }
}
