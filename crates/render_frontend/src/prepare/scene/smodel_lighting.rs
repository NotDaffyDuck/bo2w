//! Lighting for static models: which model in view holds a tile of the
//! model-lighting atlas, and writing the tiles that changed this frame.

use bevy::prelude::*;
use lighting_t6::{
    MODEL_LIGHTING_TILE_BYTES, ModelLightingAtlasDims, ModelLightingTileIndex,
    SMODEL_LIGHTING_WARN_TOO_MUCH, SModelDirtyLightingAction, SModelLightingAlloc,
    SModelLightingCounters, SModelLightingGlob, SModelLightingGlobError,
    dirty_smodel_lighting_action, smodel_lighting_bits_words,
};

use crate::prepare::scene::cull::smodel_cull_dist_skips_slot;
use crate::prepare::scene::model_lighting_atlas::{
    WorldModelLightingAtlas, model_lighting_atlas_write_tile,
};
use crate::prepare::scene::smodel_geom_cache::{LodRampDvar, WorldStaticModelCache};
use crate::prepare::scene::view_parms::{PreparedSceneView, RLockPvs};
use crate::prepare::scene::world::{WorldCull, WorldScene, WorldSmodelLightingSample};

type Tile = [u8; MODEL_LIGHTING_TILE_BYTES];

/// The host flags the dirty walk is told; none are known yet.
const NO_HOST_FLAGS: u8 = 0;

/// `BO2MP_SMODEL_LIGHTING=0` (or false, off, no) turns static-model lighting off.
fn smodel_lighting_enabled() -> bool {
    !matches!(
        std::env::var("BO2MP_SMODEL_LIGHTING").as_deref(),
        Ok("0" | "false" | "off" | "no")
    )
}

/// The lighting of every static model slot of the world.
#[derive(Resource)]
pub struct WorldSmodelLighting {
    smodel_entry_limit: u32,
    /// 256 divided by the atlas height.
    pub inv_height_x256: u32,
    /// The authored tile of each slot, if the map has a sample for it.
    pub tiles: Vec<Option<Tile>>,
    /// The packed lighting word of each slot, likewise.
    pub packed_lighting: Vec<Option<[u8; 4]>>,
    /// The atlas handle each slot holds now; 0 for none.
    pub handles: Vec<u16>,
    /// The entity spawned for each slot.
    pub parents: Vec<Option<Entity>>,
    counters: SModelLightingCounters,
    freeable_handles: Vec<u16>,
    smodel_index: Vec<u16>,
    used_frame_count: Vec<i32>,
    lighting_bits: Vec<u32>,
    warned_too_much: bool,
    /// Slots found lit when the world spawned.
    pub spawn_lit_n: u32,
    /// Slots whose sample had no material technique to light with.
    pub spawn_sample_without_technique: u32,
}

impl WorldSmodelLighting {
    pub fn new(
        dims: ModelLightingAtlasDims,
        slot_count: usize,
        samples: &[WorldSmodelLightingSample],
    ) -> Self {
        let entry_limit = dims.smodel_entry_limit as usize;
        let mut tiles = vec![None; slot_count];
        let mut packed_lighting = vec![None; slot_count];
        for sample in samples {
            if let Some(tile) = tiles.get_mut(sample.authored_slot) {
                *tile = Some(sample.tile_rgba);
            }
            if let Some(packed) = packed_lighting.get_mut(sample.authored_slot) {
                *packed = Some(sample.packed_lighting);
            }
        }
        Self {
            smodel_entry_limit: dims.smodel_entry_limit,
            inv_height_x256: 0x100 / dims.image_height,
            tiles,
            packed_lighting,
            handles: vec![0; slot_count],
            parents: vec![None; slot_count],
            counters: SModelLightingCounters::AFTER_LIGHT_WALK,
            freeable_handles: vec![0; entry_limit],
            smodel_index: vec![0; entry_limit],
            used_frame_count: vec![0; entry_limit],
            lighting_bits: vec![0; smodel_lighting_bits_words(slot_count as u32)],
            warned_too_much: false,
            spawn_lit_n: 0,
            spawn_sample_without_technique: 0,
        }
    }

    /// Borrows the allocator state together with the handle table. Logs and
    /// returns None, naming `phase`, if the tables do not fit each other.
    fn glob(&mut self, phase: &str) -> Option<(SModelLightingGlob<'_>, &mut [u16])> {
        let (limit, slots) = (self.smodel_entry_limit, self.tiles.len());
        let borrowed: Result<_, SModelLightingGlobError> = SModelLightingGlob::new(
            limit,
            slots as u32,
            &mut self.counters,
            &mut self.freeable_handles,
            &mut self.smodel_index,
            &mut self.used_frame_count,
            &mut self.lighting_bits,
        );
        match borrowed {
            Ok(glob) => Some((glob, self.handles.as_mut_slice())),
            Err(_) => {
                diag::error!(
                    World,
                    "smodel lighting {phase}: glob borrow failed, entryLimit={limit} slots={slots}"
                );
                None
            }
        }
    }

    #[inline]
    pub fn packed_lighting_for_slot(&self, slot: usize) -> Option<[u8; 4]> {
        self.packed_lighting.get(slot).copied().flatten()
    }
}

/// None when lighting is off or the map has no samples.
pub(crate) fn prepare_smodel_lighting(
    dims: ModelLightingAtlasDims,
    samples: &[WorldSmodelLightingSample],
    slot_count: usize,
) -> Option<WorldSmodelLighting> {
    if !smodel_lighting_enabled() || samples.is_empty() {
        return None;
    }
    Some(WorldSmodelLighting::new(dims, slot_count, samples))
}

pub(crate) fn slot_has_lighting_sample(lighting: &WorldSmodelLighting, index: usize) -> bool {
    lighting.tiles.get(index).is_some_and(Option::is_some)
}

/// True if any of the surfaces' materials reads the model-lighting constant.
pub(crate) fn surfaces_take_model_lighting(
    surface_materials: impl IntoIterator<Item = Option<assets::MaterialIndex>>,
    materials: &crate::assemble::drawsurf::RuntimeMaterialCatalog,
) -> bool {
    surface_materials.into_iter().any(|material| {
        material
            .and_then(|id| materials.derived(id))
            .is_some_and(|m| m.uses_model_lighting_const)
    })
}

pub(crate) fn slot_is_lit_candidate(
    lighting: &WorldSmodelLighting,
    index: usize,
    surfaces_take_model_lighting: bool,
) -> bool {
    surfaces_take_model_lighting && slot_has_lighting_sample(lighting, index)
}

/// The slots that need a lighting tile now: spawned, sampled, in view and
/// not too far to draw.
fn slots_needing_tiles(
    lighting: &WorldSmodelLighting,
    scene: &WorldScene,
    cull: &WorldCull,
    eye: Option<Vec3>,
    scale_last: Option<f32>,
) -> Vec<usize> {
    (0..lighting.tiles.len())
        .filter(|&slot| {
            slot_has_lighting_sample(lighting, slot)
                && lighting.parents.get(slot).copied().flatten().is_some()
                && cull.smodel_vis.get(slot).is_some_and(|&seen| seen != 0)
        })
        .filter(|&slot| {
            let origin = scene
                .static_model_instances
                .get(slot)
                .and_then(Option::as_ref)
                .map_or([0.0; 3], |inst| inst.origin);
            !smodel_cull_dist_skips_slot(&cull.static_model_cull_dists, slot, origin, eye, scale_last)
        })
        .collect()
}

/// Gives each of `slots` an atlas handle, evicting colder models when the
/// atlas is full. Returns how many got none, or None if the allocator could
/// not be borrowed.
fn assign_handles(
    lighting: &mut WorldSmodelLighting,
    slots: &[usize],
    mut geom_cache: Option<&mut WorldStaticModelCache>,
) -> Option<u32> {
    let (mut glob, handles) = lighting.glob("allocation")?;
    glob.toggle_frame();
    glob.clear_dirty_bits();

    let mut failed = 0u32;
    for &slot in slots {
        match glob.alloc_static_model_lighting(slot as u32, handles[slot]) {
            SModelLightingAlloc::Reused { handle } | SModelLightingAlloc::Assigned { handle } => {
                handles[slot] = handle;
            }
            SModelLightingAlloc::Evicted {
                handle,
                evicted_smodel_index,
            } => {
                let displaced = usize::from(evicted_smodel_index);
                handles[displaced] = 0;
                if let Some(cache) = geom_cache.as_deref_mut() {
                    cache.uncache_slot(displaced);
                }
                handles[slot] = handle;
            }
            SModelLightingAlloc::Failed => {
                failed += 1;
                handles[slot] = 0;
            }
        }
    }
    Some(failed)
}

/// The slots whose tile must be written, each with the atlas handle to write
/// it at. A slot that would need a patch upload is only counted and logged.
fn dirty_tile_slots(lighting: &mut WorldSmodelLighting) -> Option<Vec<(usize, u16)>> {
    let mut dirty = Vec::new();
    let (mut glob, _) = lighting.glob("dirty walk")?;
    glob.update_dirty_smodel_lighting(|smodel_index| dirty.push(smodel_index));

    let mut writes = Vec::with_capacity(dirty.len());
    let mut patches_wanted = 0u32;
    for smodel_index in dirty {
        let slot = smodel_index as usize;
        let handle = lighting.handles.get(slot).copied().unwrap_or(0);
        match dirty_smodel_lighting_action(smodel_index, handle, NO_HOST_FLAGS) {
            Some(SModelDirtyLightingAction::SampleImmediate { entry, .. }) => {
                writes.push((slot, entry.wrapping_add(1)));
            }
            Some(SModelDirtyLightingAction::EnqueuePatch { .. }) => patches_wanted += 1,
            None => {}
        }
    }
    if patches_wanted > 0 {
        diag::error!(
            World,
            "smodel lighting: {patches_wanted} dirty smodel(s) want EnqueuePatch, \
             but lighting patch upload is missing"
        );
    }
    Some(writes)
}

/// Writes the tile of each `(slot, handle)` into the atlas image.
fn write_tiles(
    images: &mut Assets<Image>,
    atlas: &WorldModelLightingAtlas,
    tiles: &[Option<Tile>],
    writes: &[(usize, u16)],
) {
    let Some(mut image) = images.get_mut(&atlas.image) else {
        return;
    };
    for &(slot, handle) in writes {
        let Some(Some(tile)) = tiles.get(slot) else {
            continue;
        };
        let Some(entry) = ModelLightingTileIndex::from_handle(handle) else {
            continue;
        };
        if !model_lighting_atlas_write_tile(&mut image, atlas.dims, entry, tile) {
            diag::error!(
                World,
                "smodel lighting: atlas refused the tile of slot {slot} (handle {handle})"
            );
        }
    }
}

pub(crate) fn update_smodel_lighting(
    mut lighting: Option<ResMut<WorldSmodelLighting>>,
    mut geom_cache: Option<ResMut<WorldStaticModelCache>>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    scene: Res<WorldScene>,
    mut images: ResMut<Assets<Image>>,
    prepared: Res<PreparedSceneView>,
    lock_pvs: Res<RLockPvs>,
    lod_ramp: Res<LodRampDvar>,
) {
    let (Some(lighting), Some(atlas), Some(cull)) =
        (lighting.as_deref_mut(), atlas, scene.cull.as_ref())
    else {
        return;
    };

    let eye = prepared.ready.then(|| lock_pvs.dpvs_eye(&prepared));
    let wanted = slots_needing_tiles(lighting, &scene, cull, eye, lod_ramp.scale_last);
    let Some(failed) = assign_handles(lighting, &wanted, geom_cache.as_deref_mut()) else {
        return;
    };
    if failed > 0 && !lighting.warned_too_much {
        lighting.warned_too_much = true;
        diag::error!(
            World,
            "smodel lighting: warning 0x{SMODEL_LIGHTING_WARN_TOO_MUCH:x}, {failed} visible lit \
             smodel(s) skipped this frame (atlas full, entryLimit={})",
            lighting.smodel_entry_limit
        );
    }

    let Some(writes) = dirty_tile_slots(lighting) else {
        return;
    };
    write_tiles(&mut images, &atlas, &lighting.tiles, &writes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(smodel_entries: u32) -> ModelLightingAtlasDims {
        ModelLightingAtlasDims {
            xmodel_entry_limit: 8,
            smodel_entry_limit: smodel_entries,
            image_height: 64,
        }
    }

    fn sample(slot: usize, shade: u8) -> WorldSmodelLightingSample {
        WorldSmodelLightingSample {
            authored_slot: slot,
            lighting_origin: [0.0; 3],
            tile_rgba: [shade; 256],
            packed_lighting: [shade, 0, 0, 0xff],
        }
    }

    /// Four slots, all with a sample, and room for three tiles: entry 0 of the
    /// atlas is never given out.
    fn lit(slots: usize) -> WorldSmodelLighting {
        let samples: Vec<_> = (0..slots).map(|slot| sample(slot, slot as u8 + 1)).collect();
        WorldSmodelLighting::new(dims(4), slots, &samples)
    }

    #[test]
    fn each_slot_holds_the_sample_authored_for_it() {
        let lighting = WorldSmodelLighting::new(dims(4), 3, &[sample(1, 9), sample(7, 5)]);
        assert_eq!(lighting.tiles.len(), 3);
        assert_eq!(lighting.tiles[1], Some([9; 256]));
        assert_eq!(lighting.packed_lighting_for_slot(1), Some([9, 0, 0, 0xff]));
        // Slot 7 does not exist, and the slots nobody sampled have nothing.
        assert!(lighting.tiles[0].is_none() && lighting.tiles[2].is_none());
        assert_eq!(lighting.packed_lighting_for_slot(0), None);
        assert_eq!(lighting.packed_lighting_for_slot(7), None);
        assert!(lighting.handles.iter().all(|&handle| handle == 0));
        assert!(lighting.parents.iter().all(Option::is_none));
    }

    #[test]
    fn the_lookup_scale_follows_the_atlas_height() {
        let lighting = WorldSmodelLighting::new(dims(4), 1, &[]);
        assert_eq!(lighting.inv_height_x256, 4);
    }

    #[test]
    fn a_slot_is_a_lit_candidate_only_with_both_a_sample_and_a_material_that_wants_it() {
        let lighting = WorldSmodelLighting::new(dims(4), 3, &[sample(1, 2)]);
        assert!(slot_has_lighting_sample(&lighting, 1));
        assert!(!slot_has_lighting_sample(&lighting, 0));
        assert!(!slot_has_lighting_sample(&lighting, 50));
        assert!(slot_is_lit_candidate(&lighting, 1, true));
        assert!(!slot_is_lit_candidate(&lighting, 1, false));
        assert!(!slot_is_lit_candidate(&lighting, 0, true));
    }

    #[test]
    fn no_samples_means_no_static_model_lighting() {
        assert!(prepare_smodel_lighting(dims(4), &[], 3).is_none());
    }

    #[test]
    fn samples_give_the_world_static_model_lighting() {
        // Unless the environment switches static-model lighting off.
        if !smodel_lighting_enabled() {
            return;
        }
        let lighting = prepare_smodel_lighting(dims(4), &[sample(0, 3)], 2).expect("lighting");
        assert_eq!(lighting.tiles.len(), 2);
    }

    #[test]
    fn visible_slots_take_atlas_entries_after_the_reserved_one() {
        let mut lighting = lit(4);
        assert_eq!(assign_handles(&mut lighting, &[0, 1, 2], None), Some(0));
        assert_eq!(&lighting.handles[..3], [2, 3, 4]);
        assert_eq!(lighting.handles[3], 0);
    }

    #[test]
    fn a_slot_that_already_holds_a_handle_keeps_it() {
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1], None);
        let before = lighting.handles.clone();
        assert_eq!(assign_handles(&mut lighting, &[0, 1], None), Some(0));
        assert_eq!(lighting.handles, before);
    }

    #[test]
    fn a_full_atlas_leaves_the_extra_slots_without_a_handle_and_counts_them() {
        let mut lighting = lit(4);
        assert_eq!(assign_handles(&mut lighting, &[0, 1, 2, 3], None), Some(1));
        assert_eq!(lighting.handles[3], 0);
        assert!(lighting.handles[..3].iter().all(|&handle| handle != 0));
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
        assert_eq!(lighting.handles[3], 0);

        // One frame later they have been idle four frames; the newest
        // allocation is taken first.
        let mut lighting = lit(4);
        assign_handles(&mut lighting, &[0, 1, 2], None);
        for _ in 0..3 {
            assign_handles(&mut lighting, &[], None);
        }
        assert_eq!(assign_handles(&mut lighting, &[3], None), Some(0));
        assert_eq!(lighting.handles[3], 4);
        assert_eq!(lighting.handles[2], 0);
        assert_eq!(&lighting.handles[..2], [2, 3]);
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
        assert_eq!(lighting.handles, [2, 0, 4, 3]);
    }

    #[test]
    fn tables_that_do_not_fit_the_allocator_stop_the_pass() {
        let mut lighting = WorldSmodelLighting::new(dims(70_000), 3, &[sample(0, 1)]);
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

    fn real_atlas() -> (Assets<Image>, WorldModelLightingAtlas) {
        let mut images = Assets::<Image>::default();
        let dims = lighting_t6::model_lighting_atlas_dims(1).expect("dims");
        let atlas = WorldModelLightingAtlas::new(&mut images, dims);
        (images, atlas)
    }

    fn pattern_tile() -> Tile {
        let mut tile = [0u8; MODEL_LIGHTING_TILE_BYTES];
        for (i, byte) in tile.iter_mut().enumerate() {
            *byte = (i % 251) as u8 + 1;
        }
        tile
    }

    #[test]
    fn a_tile_lands_in_the_atlas_at_its_handles_place() {
        let (mut images, atlas) = real_atlas();
        let tile = pattern_tile();
        write_tiles(&mut images, &atlas, &[Some(tile)], &[(0, 2)]);

        let data = images.get(&atlas.image).unwrap().data.clone().unwrap();
        let row_pitch = lighting_t6::MODEL_LIGHTING_ATLAS_WIDTH as usize * 4;
        let slice_pitch = row_pitch * atlas.dims.image_height as usize;
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
        let atlas = WorldModelLightingAtlas {
            image: Handle::default(),
            dims: lighting_t6::model_lighting_atlas_dims(1).expect("dims"),
        };
        write_tiles(&mut images, &atlas, &[Some(pattern_tile())], &[(0, 2)]);
    }
}
