//! Lighting for models that move: the first-person viewmodel, remote bodies,
//! glass pieces and effect models. Each asks for a sample of the light grid at
//! its origin; the answer is a handle to a tile of the model-lighting atlas.

use bevy::ecs::system::SystemParam;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use lighting_t6::{
    MODEL_LIGHT_VACANCY_BITSET_COUNT, ILLUM_TILE_LEN, ModelIllumAtlasDims,
    IllumSlotGrant, ModelLightCacheBundle, IllumTileNo,
    free_texel_word,
};
use render_anim::{ActiveGunMeshes, viewmodel_light_extent};

use crate::prepare::scene::camera::FreeRoamCamera;
use crate::prepare::scene::model_illum_atlas::{
    LevelModelIllumAtlas, light_atlas_store_tile,
};
use crate::prepare::scene::world::LevelStage;

pub use render_scene::{
    ModelLightingAtlasTileWrites, ModelIllumOwner, ModelIllumRequest, ModelIllumRequests,
    ResolvedModelIllum, ResolvedModelIllumTable,
};

/// What each cache slot was last lit from: the origin it was sampled at, the
/// packed light and probe bytes, and the packed vertex colour.
struct TileRecords {
    positions_at: Vec<[f32; 3]>,
    info: Vec<u16>,
    colour: Vec<Option<[u8; 4]>>,
}

/// The free-slot bitmaps of the last four frames and which one is this
/// frame's. A slot is free when no frame of the last three used it.
struct FreeRing {
    bits: [Vec<u32>; MODEL_LIGHT_VACANCY_BITSET_COUNT],
    frame: u32,
    /// An allocation failed this frame; nothing more is tried until the next.
    fail_count: bool,
}

impl FreeRing {
    fn all_free(slots: u32) -> Self {
        let words = lighting_t6::illum_free_word_count(slots);
        Self {
            bits: std::array::from_fn(|_| vec![u32::MAX; words]),
            frame: 0,
            fail_count: false,
        }
    }

    fn advance(&mut self) {
        let Self { bits, frame, fail_count: failed } = self;
        let mut views = bits.each_mut().map(Vec::as_mut_slice);
        lighting_t6::toggle_debris_model_illum_frame(&mut views, frame, failed);
    }

    /// The bitmaps of two frames ago, last frame and this frame, and the
    /// failure flag.
    fn views(&mut self) -> Option<(&[u32], &[u32], &mut [u32], &mut bool)> {
        let Self { bits, frame, fail_count: failed } = self;
        let [now, last, older] = [0, 1, 2].map(|age| free_texel_word(*frame, age));
        let (mut this_frame, mut last_frame, mut before) = (None, None, None);
        for (index, buffer) in bits.iter_mut().enumerate() {
            if index == now {
                this_frame = Some(buffer.as_mut_slice());
            } else if index == last {
                last_frame = Some(&buffer[..]);
            } else if index == older {
                before = Some(&buffer[..]);
            }
        }
        Some((before?, last_frame?, this_frame?, failed))
    }
}

/// What seating an owner needs besides the cache: where tiles go and the
/// world they are sampled in.
struct SeatCtx<'a> {
    atlas: &'a LevelModelIllumAtlas,
    scene: Option<&'a LevelStage>,
    images: &'a mut Assets<Image>,
    tile_writes: &'a mut ModelLightingAtlasTileWrites,
}

impl SeatCtx<'_> {
    /// Writes a freshly sampled tile into the atlas image and queues it for
    /// the GPU. The image is changed untracked: only `tile_writes` carries
    /// the tile across.
    fn upload(&mut self, tile_handle: u16, tile: &[u8; ILLUM_TILE_LEN]) {
        let Some(entry) = IllumTileNo::from_handle(tile_handle) else {
            return;
        };
        let Some(image) = self.images.get_mut_untracked(&self.atlas.image) else {
            return;
        };
        if light_atlas_store_tile(image, self.atlas.dims, entry, tile) {
            self.tile_writes.push(self.atlas.image.id(), entry, tile);
        }
    }
}

/// Handles for the moving models, and what each handle's tile was sampled from.
#[derive(Resource)]
pub struct LevelModelIllumCache {
    /// The handle the first-person eye model holds; 0 for none.
    pub handle: u16,
    owners: HashMap<ModelIllumOwner, u16>,
    tiles: TileRecords,
    ring: FreeRing,
    scan_cursor: u32,
    /// Handles of moving models start after the static models' entries.
    first_vertex: u32,
    /// This frame's allocation failure has been logged.
    failure_logged: bool,
    eye_sample_logged: bool,
}

impl LevelModelIllumCache {
    pub fn new(dims: ModelIllumAtlasDims) -> Self {
        let slots = dims.figure_entry_limit as usize;
        Self {
            handle: 0,
            owners: HashMap::default(),
            tiles: TileRecords {
                positions_at: vec![[0.0; 3]; slots],
                info: vec![0; slots],
                colour: vec![None; slots],
            },
            ring: FreeRing::all_free(dims.figure_entry_limit),
            scan_cursor: 0,
            first_vertex: dims.prop_entry_limit,
            failure_logged: false,
            eye_sample_logged: false,
        }
    }

    /// Starts a frame: last frame's tiles become reusable.
    pub fn flip_frame(&mut self) {
        self.ring.advance();
        self.failure_logged = false;
    }

    /// Forgets the glass pieces that are not in `live`.
    fn retain_glass(&mut self, live: &HashSet<ModelIllumOwner>) {
        self.owners.retain(|owner, _| !is_glass(*owner) || live.contains(owner));
    }

    /// Asks the allocator for a handle for `held` (0 if the owner has none),
    /// working in this frame's free-bits buffer. None when the limits make
    /// no allocator.
    fn allocate(
        &mut self,
        dims: ModelIllumAtlasDims,
        held: u16,
        pos: [f32; 3],
        allow_moved_recycle: bool,
    ) -> Option<IllumSlotGrant> {
        let Self {
            scan_cursor, tiles, ring, ..
        } = self;
        let (two_ago, last, now, failed) = ring.views()?;
        let mut glob = ModelLightCacheBundle::new(
            dims.prop_entry_limit,
            dims.figure_entry_limit,
            scan_cursor,
            failed,
            &mut tiles.positions_at,
            &mut tiles.info,
            two_ago,
            last,
            now,
        )
        .ok()?;
        Some(glob.alloc(held, pos, allow_moved_recycle))
    }

    fn log_failure_once(&mut self) {
        if std::mem::replace(&mut self.failure_logged, true) {
            return;
        }
        let warn = ModelLightCacheBundle::warn_index_on_fail();
        diag::info!(World, "model light cache alloc failed (warn {warn}), remote body skipped");
    }

    /// Gives `key` a tile lit as at `origin`, sampling the light grid if the
    /// tile is new. Returns the tile's handle, or 0 if there is none to give;
    /// an owner left with none is forgotten.
    fn seat_owner(
        &mut self,
        key: ModelIllumOwner,
        pos: [f32; 3],
        ctx: &mut SeatCtx<'_>,
        lookup_backup: u8,
        allow_moved_recycle: bool,
    ) -> u16 {
        let held = self.owners.get(&key).copied().unwrap_or(0);
        let seated = self.try_seat(key, held, pos, ctx, lookup_backup, allow_moved_recycle);
        match seated {
            Some(handle) => self.owners.insert(key, handle),
            None => self.owners.remove(&key),
        };
        seated.unwrap_or(0)
    }

    fn try_seat(
        &mut self,
        key: ModelIllumOwner,
        held: u16,
        pos: [f32; 3],
        ctx: &mut SeatCtx<'_>,
        lookup_backup: u8,
        allow_moved_recycle: bool,
    ) -> Option<u16> {
        let granted = self.allocate(ctx.atlas.dims, held, pos, allow_moved_recycle)?;
        let (slot, handle) = match granted {
            IllumSlotGrant::Recycled { owner: handle, .. } => return Some(handle),
            IllumSlotGrant::SlotFilled { owner: handle, slot_no: seat } => (seat as usize, handle),
            IllumSlotGrant::Failed => {
                self.log_failure_once();
                return None;
            }
        };
        let lit = self.light_new_tile(key, handle, slot, pos, ctx, lookup_backup);
        lit.then_some(handle)
    }

    /// Samples the light grid for a new tile and records the result. False
    /// when there is no grid or the sample is blocked.
    fn light_new_tile(
        &mut self,
        key: ModelIllumOwner,
        handle: u16,
        seat: usize,
        pos: [f32; 3],
        ctx: &mut SeatCtx<'_>,
        lookup_backup: u8,
    ) -> bool {
        let Some(grid) = ctx.scene.and_then(|scene| scene.light_grid.as_ref()) else {
            return false;
        };
        let view = grid.view();
        let Ok(taken) = asset_model::probe_light_cells_with_backup(&view, pos, lookup_backup)
        else {
            self.tiles.colour[seat] = None;
            return false;
        };
        if key == ModelIllumOwner::Eye && !std::mem::replace(&mut self.eye_sample_logged, true) {
            let (main_one, path, live) = (taken.chosen_key_light, taken.path, taken.active_corner_total);
            diag::info!(
                World,
                "fpv AtPoint origin={pos:?} primary={main_one} path={path:?} live={live} need_sample=1"
            );
        }
        ctx.upload(handle, &taken.texels);
        let probe = ctx.scene.map_or(0, |scene| scene.env_probe_for_illum_origin(pos));
        self.tiles.info[seat] = lighting_t6::illum_word_from_bytes(taken.chosen_key_light, probe);
        let [r, g, b] = taken.packed;
        self.tiles.colour[seat] = Some([r, g, b, 0xff]);
        true
    }

    /// The cache slot behind a handle given out by `seat_owner`.
    fn slot_of(&self, handle: u32) -> Option<usize> {
        let handle = u16::try_from(handle).ok()?;
        let slot = lighting_t6::illum_entry_slot(self.first_vertex, handle)?;
        Some(slot as usize)
    }

    /// One byte of the handle's packed light and probe record; 0 when the
    /// handle names no slot.
    fn info_byte(&self, id: u32, byte: fn(u16) -> u8) -> u8 {
        let Some(&info) = self.slot_of(id).and_then(|slot| self.tiles.info.get(slot)) else {
            return 0;
        };
        byte(info)
    }

    pub fn env_probe_index_for_handle(&self, id: u32) -> u8 {
        self.info_byte(id, lighting_t6::lighting_word_reflection_probe)
    }

    pub fn stage_light_index_for_handle(&self, id: u32) -> u8 {
        self.info_byte(id, lighting_t6::light_info_to_scene_light_slot)
    }

    pub fn compact_illum_for_handle(&self, id: u32) -> Option<[u8; 4]> {
        *self.tiles.colour.get(self.slot_of(id)?)?
    }

    /// What `owner` is answered: its seated tile, or a failure. The eye's
    /// handle is remembered for the viewmodel.
    fn answer(&mut self, req: ModelIllumRequest, ctx: &mut SeatCtx<'_>) -> ResolvedModelIllum {
        let owner = req.owner;
        let moving_glass = is_glass(owner);
        let at = req.origin;
        let handle = self.seat_owner(owner, at, ctx, req.lookup_backup, moving_glass);
        if owner == ModelIllumOwner::Eye {
            self.handle = handle;
        }
        let handle = u32::from(handle);
        if handle == 0 {
            return ResolvedModelIllum::Failed;
        }
        ResolvedModelIllum::Seated {
            handle,
            stage_light_index: self.stage_light_index_for_handle(handle),
            env_probe_index: self.env_probe_index_for_handle(handle),
            compact_illum: self.compact_illum_for_handle(handle),
        }
    }
}

fn is_glass(owner: ModelIllumOwner) -> bool {
    matches!(owner, ModelIllumOwner::Pane(_))
}

/// Clears last frame's requests and answers, and starts the cache's frame.
pub(crate) fn begin_debris_model_illum_frame(
    cache: Option<ResMut<LevelModelIllumCache>>,
    mut asked: ResMut<ModelIllumRequests>,
    mut answered: ResMut<ResolvedModelIllumTable>,
) {
    // Last frame's questions and answers go; the cache itself is kept.
    asked.clear();
    answered.clear();
    if let Some(mut cache) = cache {
        cache.flip_frame();
    }
}

/// What the first-person viewmodel's lighting request is made from.
#[derive(SystemParam)]
pub struct FpvLightingInputs<'w, 's> {
    at_spot: Res<'w, render_scene::LooseAtSpotLookup>,
    first_person_meshes: Option<Res<'w, assets::ReadiedFpvMeshes>>,
    session_vm: Option<Res<'w, ActiveGunMeshes>>,
    cameras: Query<'w, 's, &'static Transform, With<FreeRoamCamera>>,
    shown: Option<Res<'w, net::ShownWorldFrame>>,
    viewer: Option<Res<'w, net::HostViewClient>>,
}

impl FpvLightingInputs<'_, '_> {
    /// Where the viewmodel is lit from: the player's eye, else the fly
    /// camera.
    fn origin(&self) -> Option<[f32; 3]> {
        let player = self
            .shown
            .as_deref()
            .zip(self.viewer.as_deref())
            .and_then(|(snapshot, local)| snapshot.player(local.0));
        if let Some(ps) = player {
            let yaw = ps.look_angles[1];
            let eye_height = ps.eye_height_now;
            return Some(lighting_t6::held_view_light_point(
                ps.origin,
                eye_height,
                yaw,
                ps.tilt_sideways,
            ));
        }
        self.cameras.single().ok().map(|camera| camera.translation.to_array())
    }
}

/// Requests lighting for the first-person viewmodel.
pub fn enqueue_first_person_model_illum(
    inputs: FpvLightingInputs,
    mut requests: ResMut<ModelIllumRequests>,
) {
    let Some(viewmodel) = inputs.session_vm.as_deref().and_then(|session| session.0.as_ref()) else {
        return;
    };
    let Some(origin) = inputs.origin() else {
        return;
    };
    let same_catalog = |catalog: &&assets::ReadiedFpvMeshes| catalog.0.identity() == viewmodel.catalog_id;
    let box_half = inputs.first_person_meshes.as_deref().filter(same_catalog).and_then(|catalog| {
        viewmodel_light_extent(&catalog.0, viewmodel.fpv.hands_index, viewmodel.fpv.gun_index)
    });
    let lookup_backup = inputs.at_spot.fallback(origin, box_half);
    requests.queue(ModelIllumRequest {
        owner: ModelIllumOwner::Eye,
        origin,
        lookup_backup,
    });
}


/// Everything answering this frame's lighting requests touches.
#[derive(SystemParam)]
pub(crate) struct LightingDrain<'w> {
    cache: Option<ResMut<'w, LevelModelIllumCache>>,
    lit_atlas: Option<Res<'w, LevelModelIllumAtlas>>,
    stage: Option<Res<'w, LevelStage>>,
    pixel_store: ResMut<'w, Assets<Image>>,
    tile_writes: ResMut<'w, ModelLightingAtlasTileWrites>,
    requests: ResMut<'w, ModelIllumRequests>,
    resolved: ResMut<'w, ResolvedModelIllumTable>,
}

/// The atlas as the world scene holds it, for before the atlas resource
/// exists.
fn scene_atlas(scene: Option<&LevelStage>) -> Option<LevelModelIllumAtlas> {
    let scene = scene?;
    let image = scene.model_illum_image.clone()?;
    let dims = scene.model_illum_dims?;
    Some(LevelModelIllumAtlas { image, dims })
}

impl LightingDrain<'_> {
    /// Answers every pending request. Moving glass is sampled afresh each
    /// frame; with `prune_glass`, glass that asked nothing this frame is
    /// forgotten afterwards.
    fn answer_requests(&mut self, prune_glass: bool) {
        let pending = self.requests.take_queued();
        let live_glass: HashSet<ModelIllumOwner> =
            pending.iter().map(|request| request.owner).filter(|&owner| is_glass(owner)).collect();
        let fallback_atlas = scene_atlas(self.stage.as_deref());
        let Self {
            cache,
            lit_atlas: atlas,
            stage: scene,
            pixel_store: images,
            tile_writes,
            resolved,
            ..
        } = self;
        let atlas = atlas.as_deref().or(fallback_atlas.as_ref());
        match (cache.as_deref_mut(), atlas) {
            (Some(cache), Some(atlas)) => {
                let mut ctx = SeatCtx {
                    atlas,
                    scene: scene.as_deref(),
                    images: &mut **images,
                    tile_writes: &mut **tile_writes,
                };
                for asked in pending {
                    // Glass is lit afresh every time it is asked about.
                    if is_glass(asked.owner) {
                        resolved.forget(asked.owner);
                    }
                    resolved.keep_first_with(asked.owner, || cache.answer(asked, &mut ctx));
                }
                if prune_glass {
                    cache.retain_glass(&live_glass);
                }
            }
            _ => {
                for unlit in pending {
                    resolved.keep_first(unlit.owner, ResolvedModelIllum::Failed);
                }
            }
        }
        if prune_glass {
            resolved.retain_glass(&live_glass);
        }
    }
}

// Three systems, so that each can be ordered on its own (glass lights after
// the models that move with the skeletons, effect models after their vertices).

pub(crate) fn refresh_dirty_model_lighting(mut drain: LightingDrain) {
    drain.answer_requests(false);
}

pub(crate) fn refresh_glass_dyn_lighting(mut drain: LightingDrain) {
    drain.answer_requests(true);
}

pub(crate) fn refresh_fx_dyn_lighting(mut drain: LightingDrain) {
    drain.answer_requests(false);
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::*;

    const BASE: u32 = 10;

    fn dims(entries: u32) -> ModelIllumAtlasDims {
        ModelIllumAtlasDims {
            figure_entry_limit: entries,
            prop_entry_limit: BASE,
            rows_tall: 64,
        }
    }

    fn cache(entries: u32) -> LevelModelIllumCache {
        LevelModelIllumCache::new(dims(entries))
    }

    fn origin(x: f32) -> [f32; 3] {
        [x, 0.0, 0.0]
    }

    fn assigned(handle: u16, seat: u32) -> Option<IllumSlotGrant> {
        Some(IllumSlotGrant::SlotFilled { owner: handle, slot_no: seat })
    }

    /// Allocates for a new owner at `x` in a cache of `entries` slots.
    fn fresh(c: &mut LevelModelIllumCache, entries: u32, x: f32) -> Option<IllumSlotGrant> {
        c.allocate(dims(entries), 0, origin(x), false)
    }

    #[test]
    fn a_new_cache_holds_nothing_and_every_tile_is_free() {
        let c = cache(40);
        assert_eq!(c.handle, 0);
        assert!(c.owners.is_empty());
        assert_eq!(c.tiles.positions_at.len(), 40);
        assert!(c.tiles.colour.iter().all(Option::is_none));
        assert!(c.ring.bits.iter().all(|bits| *bits == [u32::MAX; 2]));
    }

    #[test]
    fn toggling_the_frame_steps_through_four_buffers_and_clears_the_failure() {
        let mut c = cache(4);
        c.ring.fail_count = true;
        c.failure_logged = true;
        let frames: Vec<u32> = (0..5)
            .map(|_| {
                c.flip_frame();
                c.ring.frame
            })
            .collect();
        assert_eq!(frames, [1, 2, 3, 0, 1]);
        assert!(!c.ring.fail_count && !c.failure_logged);
    }

    #[test]
    fn new_owners_take_the_free_tiles_in_order_after_the_static_models() {
        let mut c = cache(4);
        assert_eq!(fresh(&mut c, 4, 1.0), assigned(11, 0));
        assert_eq!(fresh(&mut c, 4, 2.0), assigned(12, 1));
        assert_eq!(c.tiles.positions_at[..2], [origin(1.0), origin(2.0)]);
    }

    #[test]
    fn an_owner_that_has_not_moved_keeps_its_tile_and_its_lighting() {
        let mut c = cache(4);
        fresh(&mut c, 4, 1.0);
        c.tiles.info[0] = 0x0305;
        let again = c.allocate(dims(4), 11, origin(1.0), false);
        assert_eq!(again, Some(IllumSlotGrant::Recycled { owner: 11, bits_info: 0x0305 }));
    }

    #[test]
    fn an_owner_that_moved_gets_a_new_tile_unless_it_may_keep_its_own() {
        let mut c = cache(4);
        fresh(&mut c, 4, 1.0);
        assert_eq!(c.allocate(dims(4), 11, origin(5.0), false), assigned(12, 1));
        assert_eq!(c.allocate(dims(4), 11, origin(6.0), true), assigned(11, 0));
        assert_eq!(c.tiles.positions_at[0], origin(6.0));
    }

    #[test]
    fn a_tile_stays_held_for_three_frames_after_it_was_last_used() {
        let mut c = cache(1);
        assert!(matches!(fresh(&mut c, 1, 1.0), Some(IllumSlotGrant::SlotFilled { .. })));
        for frame in 1..=3 {
            c.flip_frame();
            let want = if frame < 3 { Some(IllumSlotGrant::Failed) } else { assigned(11, 0) };
            assert_eq!(fresh(&mut c, 1, 2.0), want, "frame {frame}");
        }
    }

    #[test]
    fn once_the_cache_is_full_it_fails_until_the_next_frame() {
        let mut c = cache(2);
        fresh(&mut c, 2, 1.0);
        fresh(&mut c, 2, 2.0);
        assert_eq!(fresh(&mut c, 2, 3.0), Some(IllumSlotGrant::Failed));
        assert!(c.ring.fail_count);
        c.flip_frame();
        assert!(!c.ring.fail_count);
    }

    #[test]
    fn a_cache_with_bad_limits_answers_nothing() {
        let mut c = cache(4);
        assert_eq!(fresh(&mut c, 0, 1.0), None);
    }

    #[test]
    fn handles_map_back_to_slots_past_the_static_models() {
        let c = cache(4);
        let cases = [(0, None), (BASE, None), (BASE + 1, Some(0)), (BASE + 4, Some(3)), (70_000, None)];
        for (id, slot) in cases {
            assert_eq!(c.slot_of(id), slot, "handle {id}");
        }
    }

    #[test]
    fn lighting_lookups_by_handle_read_the_tiles_record() {
        let mut c = cache(4);
        c.tiles.info[1] = lighting_t6::illum_word_from_bytes(7, 9);
        c.tiles.colour[1] = Some([1, 2, 3, 0xff]);
        let handle = BASE + 2;
        assert_eq!(c.stage_light_index_for_handle(handle), 7);
        assert_eq!(c.env_probe_index_for_handle(handle), 9);
        assert_eq!(c.compact_illum_for_handle(handle), Some([1, 2, 3, 0xff]));
        // A tile never lit, a handle of none, and a handle past the cache.
        assert_eq!(c.compact_illum_for_handle(BASE + 1), None);
        for none in [0, BASE + 99] {
            assert_eq!(c.stage_light_index_for_handle(none), 0);
            assert_eq!(c.env_probe_index_for_handle(none), 0);
            assert_eq!(c.compact_illum_for_handle(none), None);
        }
    }

    #[test]
    fn forgetting_glass_keeps_every_other_owner() {
        let mut c = cache(4);
        c.owners.extend([
            (ModelIllumOwner::Pane(1), 11),
            (ModelIllumOwner::Pane(2), 12),
            (ModelIllumOwner::Eye, 13),
            (ModelIllumOwner::Shell(5), 14),
        ]);
        c.retain_glass(&HashSet::from_iter([ModelIllumOwner::Pane(2)]));
        let mut left: Vec<u16> = c.owners.values().copied().collect();
        left.sort_unstable();
        assert_eq!(left, [12, 13, 14]);
    }

    /// Everything `seat_owner` needs except a world with a light grid.
    struct Rig {
        atlas: LevelModelIllumAtlas,
        images: Assets<Image>,
        writes: ModelLightingAtlasTileWrites,
    }

    impl Rig {
        fn new(entries: u32) -> Self {
            Self {
                atlas: LevelModelIllumAtlas {
                    image: Handle::default(),
                    dims: dims(entries),
                },
                images: Assets::default(),
                writes: ModelLightingAtlasTileWrites::default(),
            }
        }

        fn seat(&mut self, c: &mut LevelModelIllumCache, key: ModelIllumOwner, at: f32) -> u16 {
            let mut ctx = SeatCtx {
                atlas: &self.atlas,
                scene: None,
                images: &mut self.images,
                tile_writes: &mut self.writes,
            };
            c.seat_owner(key, origin(at), &mut ctx, 0, false)
        }
    }

    #[test]
    fn without_a_light_grid_a_new_tile_is_given_back_and_the_owner_has_none() {
        let (mut c, mut rig) = (cache(4), Rig::new(4));
        let key = ModelIllumOwner::OtherPlayerSlot(3);
        assert_eq!(rig.seat(&mut c, key, 1.0), 0);
        assert!(!c.owners.contains_key(&key));
        assert!(rig.writes.tiles.is_empty());
    }

    #[test]
    fn an_owner_already_seated_where_it_stands_is_answered_without_a_grid() {
        let (mut c, mut rig) = (cache(4), Rig::new(4));
        let key = ModelIllumOwner::Item(8);
        c.owners.insert(key, 12);
        c.tiles.positions_at[1] = origin(1.0);
        assert_eq!(rig.seat(&mut c, key, 1.0), 12);
        assert_eq!(c.owners.get(&key), Some(&12));
    }

    #[test]
    fn a_full_cache_gives_no_handle_and_warns_once() {
        let (mut c, mut rig) = (cache(1), Rig::new(1));
        let (a, b) = (ModelIllumOwner::Item(1), ModelIllumOwner::Item(2));
        rig.seat(&mut c, a, 1.0);
        assert!(!c.failure_logged);
        assert_eq!(rig.seat(&mut c, b, 2.0), 0);
        assert!(c.failure_logged);
        assert!(!c.owners.contains_key(&b));
    }

    fn lighting_world(with_cache: bool) -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<ModelLightingAtlasTileWrites>();
        world.init_resource::<ModelIllumRequests>();
        world.init_resource::<ResolvedModelIllumTable>();
        if with_cache {
            world.insert_resource(cache(4));
            world.insert_resource(Rig::new(4).atlas);
        }
        world
    }

    fn ask(world: &mut World, owner: ModelIllumOwner) {
        let request = ModelIllumRequest {
            owner,
            origin: origin(1.0),
            lookup_backup: 0,
        };
        world.resource_mut::<ModelIllumRequests>().queue(request);
    }

    fn answer(world: &World, owner: ModelIllumOwner) -> Option<ResolvedModelIllum> {
        world.resource::<ResolvedModelIllumTable>().answer_for(owner)
    }

    fn nothing_pending(world: &mut World) -> bool {
        world.resource_mut::<ModelIllumRequests>().take_queued().is_empty()
    }

    #[test]
    fn a_new_frame_empties_the_requests_and_answers_and_advances_the_cache() {
        let mut world = lighting_world(true);
        ask(&mut world, ModelIllumOwner::Eye);
        world
            .resource_mut::<ResolvedModelIllumTable>()
            .keep_first(ModelIllumOwner::Eye, ResolvedModelIllum::Failed);
        assert!(world.run_system_once(begin_debris_model_illum_frame).is_ok());
        assert!(nothing_pending(&mut world));
        assert_eq!(answer(&world, ModelIllumOwner::Eye), None);
        assert_eq!(world.resource::<LevelModelIllumCache>().ring.frame, 1);
    }

    #[test]
    fn a_new_frame_without_a_cache_still_clears_the_tables() {
        let mut world = lighting_world(false);
        ask(&mut world, ModelIllumOwner::Eye);
        assert!(world.run_system_once(begin_debris_model_illum_frame).is_ok());
        assert!(nothing_pending(&mut world));
    }

    #[test]
    fn with_no_cache_every_request_is_answered_failed() {
        let mut world = lighting_world(false);
        let owners = [ModelIllumOwner::Eye, ModelIllumOwner::EffectModel(2)];
        owners.iter().for_each(|&owner| ask(&mut world, owner));
        assert!(world.run_system_once(refresh_fx_dyn_lighting).is_ok());
        for owner in owners {
            assert_eq!(answer(&world, owner), Some(ResolvedModelIllum::Failed));
        }
    }

    #[test]
    fn with_no_light_grid_the_eye_is_failed_and_holds_no_handle() {
        let mut world = lighting_world(true);
        world.resource_mut::<LevelModelIllumCache>().handle = 77;
        ask(&mut world, ModelIllumOwner::Eye);
        assert!(world.run_system_once(refresh_dirty_model_lighting).is_ok());
        assert_eq!(answer(&world, ModelIllumOwner::Eye), Some(ResolvedModelIllum::Failed));
        assert_eq!(world.resource::<LevelModelIllumCache>().handle, 0);
    }

    #[test]
    fn only_the_glass_pass_forgets_glass_nobody_asked_about() {
        let (stale, live) = (ModelIllumOwner::Pane(9), ModelIllumOwner::Pane(1));
        for prune in [false, true] {
            let mut world = lighting_world(false);
            world
                .resource_mut::<ResolvedModelIllumTable>()
                .keep_first(stale, ResolvedModelIllum::Failed);
            ask(&mut world, live);
            let ran = if prune {
                world.run_system_once(refresh_glass_dyn_lighting)
            } else {
                world.run_system_once(refresh_dirty_model_lighting)
            };
            assert!(ran.is_ok());
            assert_eq!(answer(&world, stale).is_some(), !prune);
            assert!(answer(&world, live).is_some());
        }
    }

    #[test]
    fn the_glass_pass_forgets_the_cached_handles_of_glass_nobody_asked_about() {
        let mut world = lighting_world(true);
        world
            .resource_mut::<LevelModelIllumCache>()
            .owners
            .extend([(ModelIllumOwner::Pane(9), 11), (ModelIllumOwner::Item(1), 12)]);
        ask(&mut world, ModelIllumOwner::Pane(1));
        assert!(world.run_system_once(refresh_glass_dyn_lighting).is_ok());
        let owners = &world.resource::<LevelModelIllumCache>().owners;
        assert!(!owners.contains_key(&ModelIllumOwner::Pane(9)));
        assert!(owners.contains_key(&ModelIllumOwner::Item(1)));
    }
}
