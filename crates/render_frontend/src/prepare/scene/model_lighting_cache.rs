//! Lighting for models that move: the first-person viewmodel, remote bodies,
//! glass pieces and effect models. Each asks for a sample of the light grid at
//! its origin; the answer is a handle to a tile of the model-lighting atlas.

use bevy::ecs::system::SystemParam;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use lighting_t6::{
    MODEL_LIGHTING_PIXEL_FREE_BITS_BUFFERS, ModelLightingAtlasDims, ModelLightingCacheAlloc,
    ModelLightingCacheGlob, ModelLightingTileIndex, dyn_pixel_free_bits_index,
    model_lighting_cache_free_bits_words, toggle_dyn_model_lighting_frame,
};
use render_anim::{SessionViewmodel, fpv_dobj_lighting_box_half};

use crate::prepare::scene::camera::FlyCamera;
use crate::prepare::scene::model_lighting_atlas::{
    WorldModelLightingAtlas, model_lighting_atlas_write_tile,
};
use crate::prepare::scene::world::WorldScene;

pub use render_scene::{
    ModelLightingAtlasTileWrites, ModelLightingOwner, ModelLightingRequest, ModelLightingRequests,
    ResolvedModelLighting, ResolvedModelLightingTable,
};

/// Handles for the moving models, and what each handle's tile was sampled from.
#[derive(Resource)]
pub struct WorldModelLightingCache {
    /// The handle the first-person eye model holds; 0 for none.
    pub handle: u16,
    body_handles: HashMap<ModelLightingOwner, u16>,
    rover: u32,
    alloc_fail: bool,
    mod_frame_count: u32,
    origins: Vec<[f32; 3]>,
    lighting_info: Vec<u16>,
    packed_lighting: Vec<Option<[u8; 4]>>,
    pixel_free_bits: [Vec<u32>; MODEL_LIGHTING_PIXEL_FREE_BITS_BUFFERS],
    warned_fail: bool,
    /// Handles of moving models start after the static models' entries.
    base_index: u32,
    logged_fpv_atpoint: bool,
}

impl WorldModelLightingCache {
    pub fn new(dims: ModelLightingAtlasDims) -> Self {
        let limit = dims.xmodel_entry_limit as usize;
        let all_free = vec![!0u32; model_lighting_cache_free_bits_words(dims.xmodel_entry_limit)];
        Self {
            handle: 0,
            body_handles: HashMap::new(),
            rover: 0,
            alloc_fail: false,
            mod_frame_count: 0,
            origins: vec![[0.0; 3]; limit],
            lighting_info: vec![0; limit],
            packed_lighting: vec![None; limit],
            pixel_free_bits: std::array::from_fn(|_| all_free.clone()),
            warned_fail: false,
            base_index: dims.smodel_entry_limit,
            logged_fpv_atpoint: false,
        }
    }

    /// Starts a frame: last frame's tiles become reusable.
    pub fn toggle_frame(&mut self) {
        let [b0, b1, b2, b3] = &mut self.pixel_free_bits;
        toggle_dyn_model_lighting_frame(
            &mut [b0.as_mut_slice(), b1.as_mut_slice(), b2.as_mut_slice(), b3.as_mut_slice()],
            &mut self.mod_frame_count,
            &mut self.alloc_fail,
        );
        self.warned_fail = false;
    }

    /// Forgets the glass pieces that are not in `live`.
    fn retain_glass(&mut self, live: &HashSet<ModelLightingOwner>) {
        self.body_handles
            .retain(|owner, _| !matches!(owner, ModelLightingOwner::Glass(_)) || live.contains(owner));
    }

    /// Asks the allocator for a handle for `current` (0 if it has none),
    /// working in this frame's free-bits buffer.
    fn allocate(
        &mut self,
        dims: ModelLightingAtlasDims,
        current: u16,
        origin: [f32; 3],
        allow_moved_reuse: bool,
    ) -> Option<ModelLightingCacheAlloc> {
        let this_frame = dyn_pixel_free_bits_index(self.mod_frame_count, 0);
        let last_frame = dyn_pixel_free_bits_index(self.mod_frame_count, 1);
        let before_that = dyn_pixel_free_bits_index(self.mod_frame_count, 2);
        let mut bits = std::mem::take(&mut self.pixel_free_bits[this_frame]);
        let alloc = {
            let Self {
                rover,
                alloc_fail,
                origins,
                lighting_info,
                pixel_free_bits,
                ..
            } = self;
            ModelLightingCacheGlob::new(
                dims.smodel_entry_limit,
                dims.xmodel_entry_limit,
                rover,
                alloc_fail,
                origins,
                lighting_info,
                &pixel_free_bits[before_that],
                &pixel_free_bits[last_frame],
                &mut bits,
            )
            .ok()
            .map(|mut glob| glob.alloc(current, origin, allow_moved_reuse))
        };
        self.pixel_free_bits[this_frame] = bits;
        alloc
    }

    /// Gives `key` a tile lit as at `origin`, sampling the light grid if the
    /// tile is new. Returns the tile's handle, or 0 if there is none to give.
    fn seat_owner(
        &mut self,
        key: ModelLightingOwner,
        origin: [f32; 3],
        atlas: &WorldModelLightingAtlas,
        scene: Option<&WorldScene>,
        (images, tile_writes): (&mut Assets<Image>, &mut ModelLightingAtlasTileWrites),
        lookup_fallback: u8,
        allow_moved_reuse: bool,
    ) -> u32 {
        let dims = atlas.dims;
        let current = self.body_handles.get(&key).copied().unwrap_or(0);
        let (handle, slot) = match self.allocate(dims, current, origin, allow_moved_reuse) {
            Some(ModelLightingCacheAlloc::Reused { handle, .. }) => {
                self.body_handles.insert(key, handle);
                return u32::from(handle);
            }
            Some(ModelLightingCacheAlloc::Assigned { handle, slot }) => {
                self.body_handles.insert(key, handle);
                (handle, slot as usize)
            }
            Some(ModelLightingCacheAlloc::Failed) => {
                if !self.warned_fail {
                    self.warned_fail = true;
                    diag::info!(
                        World,
                        "model light cache alloc failed (warn {}), remote body skipped",
                        ModelLightingCacheGlob::warn_index_on_fail()
                    );
                }
                self.body_handles.remove(&key);
                return 0;
            }
            None => {
                self.body_handles.remove(&key);
                return 0;
            }
        };

        // A new tile: light it from the grid.
        let Some(grid) = scene.and_then(|scene| scene.light_grid.as_ref()) else {
            self.body_handles.remove(&key);
            return 0;
        };
        let sampled = match asset_model::sample_light_grid_with_lookup_fallback(
            &grid.view(),
            origin,
            lookup_fallback,
        ) {
            Ok(sampled) => sampled,
            Err(_) => {
                self.packed_lighting[slot] = None;
                self.body_handles.remove(&key);
                return 0;
            }
        };
        if key == ModelLightingOwner::Eye && !self.logged_fpv_atpoint {
            self.logged_fpv_atpoint = true;
            diag::info!(
                World,
                "fpv AtPoint origin={:?} primary={} path={:?} live={} need_sample=1",
                origin,
                sampled.picked_primary,
                sampled.path,
                sampled.live_corners
            );
        }
        // Untracked: only `tile_writes` carries the tile to the GPU.
        if let (Some(entry), Some(image)) = (
            ModelLightingTileIndex::from_handle(handle),
            images.get_mut_untracked(&atlas.image),
        ) && model_lighting_atlas_write_tile(image, dims, entry, &sampled.tile)
        {
            tile_writes.push(atlas.image.id(), entry, &sampled.tile);
        }
        self.lighting_info[slot] = lighting_t6::lighting_info_from_bytes(
            sampled.picked_primary,
            scene.map_or(0, |scene| scene.reflection_probe_for_lighting_origin(origin)),
        );
        let [r, g, b] = sampled.compressed;
        self.packed_lighting[slot] = Some([r, g, b, 0xff]);
        u32::from(handle)
    }

    /// The cache slot behind a handle given out by `seat_owner`.
    fn slot_of(&self, handle: u32) -> Option<usize> {
        let handle = u16::try_from(handle).ok()?;
        lighting_t6::model_lighting_cache_slot(self.base_index, handle).map(|slot| slot as usize)
    }

    pub fn reflection_probe_index_for_handle(&self, handle: u32) -> u8 {
        self.slot_of(handle)
            .and_then(|slot| self.lighting_info.get(slot).copied())
            .map_or(0, lighting_t6::lighting_info_reflection_probe_index)
    }

    pub fn scene_light_index_for_handle(&self, handle: u32) -> u8 {
        self.slot_of(handle)
            .and_then(|slot| self.lighting_info.get(slot).copied())
            .map_or(0, lighting_t6::lighting_info_scene_light_index)
    }

    pub fn packed_lighting_for_handle(&self, handle: u32) -> Option<[u8; 4]> {
        let slot = self.slot_of(handle)?;
        self.packed_lighting.get(slot).copied().flatten()
    }
}

/// Clears last frame's requests and answers, and starts the cache's frame.
pub(crate) fn begin_dyn_model_lighting_frame(
    cache: Option<ResMut<WorldModelLightingCache>>,
    mut requests: ResMut<ModelLightingRequests>,
    mut resolved: ResMut<ResolvedModelLightingTable>,
) {
    requests.clear();
    resolved.clear();
    if let Some(mut cache) = cache {
        cache.toggle_frame();
    }
}

/// Where the first-person viewmodel is lit from: the player's eye, else the
/// fly camera.
fn viewmodel_origin(
    presented: Option<&net::PresentedSnapshot>,
    local: Option<&net::LocalPresentClient>,
    cameras: &Query<&Transform, With<FlyCamera>>,
) -> Option<[f32; 3]> {
    let from_player = || {
        let ps = presented?.player(local?.0)?;
        Some(lighting_t6::viewmodel_lighting_origin(
            ps.origin,
            ps.view_height_current,
            ps.viewangles[1],
            ps.leanf,
        ))
    };
    from_player().or_else(|| {
        let cam = cameras.single().ok()?;
        Some(cam.translation.to_array())
    })
}

/// Requests lighting for the first-person viewmodel.
pub fn enqueue_fpv_model_lighting(
    atpoint: Res<render_scene::DynAtPointLookup>,
    fpv_meshes: Option<Res<assets::PreparedFpvMeshes>>,
    session_vm: Option<Res<SessionViewmodel>>,
    cameras: Query<&Transform, With<FlyCamera>>,
    presented: Option<Res<net::PresentedSnapshot>>,
    local: Option<Res<net::LocalPresentClient>>,
    mut requests: ResMut<ModelLightingRequests>,
) {
    let Some(session) = session_vm.as_deref() else {
        return;
    };
    let Some(handles) = session.0.as_ref() else {
        return;
    };
    let Some(origin) = viewmodel_origin(presented.as_deref(), local.as_deref(), &cameras) else {
        return;
    };

    let box_half = fpv_meshes.as_deref().and_then(|catalog| {
        (catalog.0.identity() == handles.catalog_id).then_some(())?;
        fpv_dobj_lighting_box_half(&catalog.0, handles.fpv.hands_index, handles.fpv.gun_index)
    });
    requests.request(ModelLightingRequest {
        owner: ModelLightingOwner::Eye,
        origin,
        lookup_fallback: atpoint.fallback(origin, box_half),
    });
}

/// Everything answering this frame's lighting requests touches.
#[derive(SystemParam)]
pub(crate) struct LightingDrain<'w> {
    cache: Option<ResMut<'w, WorldModelLightingCache>>,
    atlas: Option<Res<'w, WorldModelLightingAtlas>>,
    scene: Option<Res<'w, WorldScene>>,
    images: ResMut<'w, Assets<Image>>,
    tile_writes: ResMut<'w, ModelLightingAtlasTileWrites>,
    requests: ResMut<'w, ModelLightingRequests>,
    resolved: ResMut<'w, ResolvedModelLightingTable>,
}

impl LightingDrain<'_> {
    /// Answers every pending request. Moving glass is sampled afresh each
    /// frame; with `prune_glass`, glass that asked nothing this frame is
    /// forgotten afterwards.
    fn answer_requests(&mut self, prune_glass: bool) {
        let Self {
            cache,
            atlas,
            scene,
            images,
            tile_writes,
            requests,
            resolved,
        } = self;
        let pending = requests.take_pending();
        let live_glass: HashSet<ModelLightingOwner> = pending
            .iter()
            .map(|request| request.owner)
            .filter(|owner| matches!(owner, ModelLightingOwner::Glass(_)))
            .collect();
        // The atlas resource may not exist yet; the scene holds the same image.
        let scene_atlas = scene.as_deref().and_then(|scene| {
            Some(WorldModelLightingAtlas {
                image: scene.model_lighting_image.clone()?,
                dims: scene.model_lighting_dims?,
            })
        });
        let (Some(cache), Some(atlas)) = (cache.as_deref_mut(), atlas.as_deref().or(scene_atlas.as_ref()))
        else {
            for request in pending {
                resolved.insert_if_absent(request.owner, ResolvedModelLighting::Failed);
            }
            if prune_glass {
                resolved.retain_glass(&live_glass);
            }
            return;
        };

        let scene = scene.as_deref();
        for request in pending {
            let owner = request.owner;
            let moving_glass = matches!(owner, ModelLightingOwner::Glass(_));
            if moving_glass {
                resolved.remove(owner);
            }
            resolved.get_or_insert_with(owner, || {
                let handle = cache.seat_owner(
                    owner,
                    request.origin,
                    atlas,
                    scene,
                    (&mut **images, &mut **tile_writes),
                    request.lookup_fallback,
                    moving_glass,
                );
                if owner == ModelLightingOwner::Eye {
                    cache.handle = handle as u16;
                }
                if handle == 0 {
                    return ResolvedModelLighting::Failed;
                }
                ResolvedModelLighting::Seated {
                    handle,
                    scene_light_index: cache.scene_light_index_for_handle(handle),
                    reflection_probe_index: cache.reflection_probe_index_for_handle(handle),
                    packed_lighting: cache.packed_lighting_for_handle(handle),
                }
            });
        }
        if prune_glass {
            cache.retain_glass(&live_glass);
            resolved.retain_glass(&live_glass);
        }
    }
}

// Three systems, so that each can be ordered on its own (glass lights after
// the models that move with the skeletons, effect models after their vertices).

pub(crate) fn update_dirty_model_lighting(mut drain: LightingDrain) {
    drain.answer_requests(false);
}

pub(crate) fn update_glass_dyn_lighting(mut drain: LightingDrain) {
    drain.answer_requests(true);
}

pub(crate) fn update_fx_dyn_lighting(mut drain: LightingDrain) {
    drain.answer_requests(false);
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::*;

    const BASE: u32 = 10;

    fn dims(entries: u32) -> ModelLightingAtlasDims {
        ModelLightingAtlasDims {
            xmodel_entry_limit: entries,
            smodel_entry_limit: BASE,
            image_height: 64,
        }
    }

    fn cache(entries: u32) -> WorldModelLightingCache {
        WorldModelLightingCache::new(dims(entries))
    }

    fn origin(x: f32) -> [f32; 3] {
        [x, 0.0, 0.0]
    }

    #[test]
    fn a_new_cache_holds_nothing_and_every_tile_is_free() {
        let c = cache(40);
        assert_eq!(c.handle, 0);
        assert!(c.body_handles.is_empty());
        assert_eq!(c.origins.len(), 40);
        assert!(c.packed_lighting.iter().all(Option::is_none));
        for bits in &c.pixel_free_bits {
            assert_eq!(bits.len(), 2);
            assert!(bits.iter().all(|&word| word == !0));
        }
    }

    #[test]
    fn toggling_the_frame_steps_through_four_buffers_and_clears_the_failure() {
        let mut c = cache(4);
        c.alloc_fail = true;
        c.warned_fail = true;
        let mut seen = Vec::new();
        for _ in 0..5 {
            c.toggle_frame();
            seen.push(c.mod_frame_count);
        }
        assert_eq!(seen, [1, 2, 3, 0, 1]);
        assert!(!c.alloc_fail);
        assert!(!c.warned_fail);
    }

    #[test]
    fn new_owners_take_the_free_tiles_in_order_after_the_static_models() {
        let mut c = cache(4);
        let d = dims(4);
        assert_eq!(
            c.allocate(d, 0, origin(1.0), false),
            Some(ModelLightingCacheAlloc::Assigned { handle: 11, slot: 0 })
        );
        assert_eq!(
            c.allocate(d, 0, origin(2.0), false),
            Some(ModelLightingCacheAlloc::Assigned { handle: 12, slot: 1 })
        );
        assert_eq!(c.origins[0], origin(1.0));
        assert_eq!(c.origins[1], origin(2.0));
    }

    #[test]
    fn an_owner_that_has_not_moved_keeps_its_tile_and_its_lighting() {
        let mut c = cache(4);
        let d = dims(4);
        c.allocate(d, 0, origin(1.0), false);
        c.lighting_info[0] = 0x0305;
        assert_eq!(
            c.allocate(d, 11, origin(1.0), false),
            Some(ModelLightingCacheAlloc::Reused { handle: 11, info: 0x0305 })
        );
    }

    #[test]
    fn an_owner_that_moved_gets_a_new_tile_unless_it_may_keep_its_own() {
        let mut c = cache(4);
        let d = dims(4);
        c.allocate(d, 0, origin(1.0), false);
        assert_eq!(
            c.allocate(d, 11, origin(5.0), false),
            Some(ModelLightingCacheAlloc::Assigned { handle: 12, slot: 1 })
        );
        assert_eq!(
            c.allocate(d, 11, origin(6.0), true),
            Some(ModelLightingCacheAlloc::Assigned { handle: 11, slot: 0 })
        );
        assert_eq!(c.origins[0], origin(6.0));
    }

    #[test]
    fn a_tile_stays_held_for_three_frames_after_it_was_last_used() {
        let mut c = cache(1);
        let d = dims(1);
        assert!(matches!(
            c.allocate(d, 0, origin(1.0), false),
            Some(ModelLightingCacheAlloc::Assigned { .. })
        ));
        for _ in 0..2 {
            c.toggle_frame();
            assert_eq!(
                c.allocate(d, 0, origin(2.0), false),
                Some(ModelLightingCacheAlloc::Failed)
            );
        }
        c.toggle_frame();
        assert_eq!(
            c.allocate(d, 0, origin(2.0), false),
            Some(ModelLightingCacheAlloc::Assigned { handle: 11, slot: 0 })
        );
    }

    #[test]
    fn once_the_cache_is_full_it_fails_until_the_next_frame() {
        let mut c = cache(2);
        let d = dims(2);
        c.allocate(d, 0, origin(1.0), false);
        c.allocate(d, 0, origin(2.0), false);
        assert_eq!(c.allocate(d, 0, origin(3.0), false), Some(ModelLightingCacheAlloc::Failed));
        assert!(c.alloc_fail);
        c.toggle_frame();
        assert!(!c.alloc_fail);
    }

    #[test]
    fn a_cache_with_bad_limits_answers_nothing() {
        let mut c = cache(4);
        let bad = ModelLightingAtlasDims {
            xmodel_entry_limit: 0,
            ..dims(4)
        };
        assert_eq!(c.allocate(bad, 0, origin(1.0), false), None);
    }

    #[test]
    fn handles_map_back_to_slots_past_the_static_models() {
        let c = cache(4);
        assert_eq!(c.slot_of(0), None);
        assert_eq!(c.slot_of(BASE), None);
        assert_eq!(c.slot_of(BASE + 1), Some(0));
        assert_eq!(c.slot_of(BASE + 4), Some(3));
        assert_eq!(c.slot_of(70_000), None);
    }

    #[test]
    fn lighting_lookups_by_handle_read_the_tiles_record() {
        let mut c = cache(4);
        c.lighting_info[1] = lighting_t6::lighting_info_from_bytes(7, 9);
        c.packed_lighting[1] = Some([1, 2, 3, 0xff]);
        let handle = BASE + 2;
        assert_eq!(c.scene_light_index_for_handle(handle), 7);
        assert_eq!(c.reflection_probe_index_for_handle(handle), 9);
        assert_eq!(c.packed_lighting_for_handle(handle), Some([1, 2, 3, 0xff]));
        // A tile never lit, a handle of none, and a handle past the cache.
        assert_eq!(c.packed_lighting_for_handle(BASE + 1), None);
        assert_eq!(c.scene_light_index_for_handle(0), 0);
        assert_eq!(c.reflection_probe_index_for_handle(0), 0);
        assert_eq!(c.packed_lighting_for_handle(0), None);
        assert_eq!(c.scene_light_index_for_handle(BASE + 99), 0);
        assert_eq!(c.packed_lighting_for_handle(BASE + 99), None);
    }

    #[test]
    fn forgetting_glass_keeps_every_other_owner() {
        let mut c = cache(4);
        for (owner, handle) in [
            (ModelLightingOwner::Glass(1), 11),
            (ModelLightingOwner::Glass(2), 12),
            (ModelLightingOwner::Eye, 13),
            (ModelLightingOwner::Missile(5), 14),
        ] {
            c.body_handles.insert(owner, handle);
        }
        let live: HashSet<_> = [ModelLightingOwner::Glass(2)].into_iter().collect();
        c.retain_glass(&live);
        let mut left: Vec<u16> = c.body_handles.values().copied().collect();
        left.sort_unstable();
        assert_eq!(left, [12, 13, 14]);
    }

    /// Everything `seat_owner` needs except a world with a light grid.
    struct Rig {
        atlas: WorldModelLightingAtlas,
        images: Assets<Image>,
        writes: ModelLightingAtlasTileWrites,
    }

    impl Rig {
        fn new(entries: u32) -> Self {
            Self {
                atlas: WorldModelLightingAtlas {
                    image: Handle::default(),
                    dims: dims(entries),
                },
                images: Assets::default(),
                writes: ModelLightingAtlasTileWrites::default(),
            }
        }

        fn seat(&mut self, c: &mut WorldModelLightingCache, key: ModelLightingOwner, at: f32) -> u32 {
            c.seat_owner(
                key,
                origin(at),
                &self.atlas,
                None,
                (&mut self.images, &mut self.writes),
                0,
                false,
            )
        }
    }

    #[test]
    fn without_a_light_grid_a_new_tile_is_given_back_and_the_owner_has_none() {
        let mut c = cache(4);
        let mut rig = Rig::new(4);
        let key = ModelLightingOwner::RemoteClient(3);
        assert_eq!(rig.seat(&mut c, key, 1.0), 0);
        assert!(!c.body_handles.contains_key(&key));
        assert!(rig.writes.tiles.is_empty());
    }

    #[test]
    fn an_owner_already_seated_where_it_stands_is_answered_without_a_grid() {
        let mut c = cache(4);
        let mut rig = Rig::new(4);
        let key = ModelLightingOwner::Item(8);
        c.body_handles.insert(key, 12);
        c.origins[1] = origin(1.0);
        assert_eq!(rig.seat(&mut c, key, 1.0), 12);
        assert_eq!(c.body_handles.get(&key), Some(&12));
    }

    #[test]
    fn a_full_cache_gives_no_handle_and_warns_once() {
        let mut c = cache(1);
        let mut rig = Rig::new(1);
        let (a, b) = (ModelLightingOwner::Item(1), ModelLightingOwner::Item(2));
        rig.seat(&mut c, a, 1.0);
        assert!(!c.warned_fail);
        assert_eq!(rig.seat(&mut c, b, 2.0), 0);
        assert!(c.warned_fail);
        assert!(!c.body_handles.contains_key(&b));
    }

    fn lighting_world(with_cache: bool) -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<ModelLightingAtlasTileWrites>();
        world.init_resource::<ModelLightingRequests>();
        world.init_resource::<ResolvedModelLightingTable>();
        if with_cache {
            world.insert_resource(cache(4));
            world.insert_resource(WorldModelLightingAtlas {
                image: Handle::default(),
                dims: dims(4),
            });
        }
        world
    }

    fn ask(world: &mut World, owner: ModelLightingOwner) {
        world
            .resource_mut::<ModelLightingRequests>()
            .request(ModelLightingRequest {
                owner,
                origin: origin(1.0),
                lookup_fallback: 0,
            });
    }

    fn answer(world: &World, owner: ModelLightingOwner) -> Option<ResolvedModelLighting> {
        world.resource::<ResolvedModelLightingTable>().get(owner)
    }

    #[test]
    fn a_new_frame_empties_the_requests_and_answers_and_advances_the_cache() {
        let mut world = lighting_world(true);
        ask(&mut world, ModelLightingOwner::Eye);
        world
            .resource_mut::<ResolvedModelLightingTable>()
            .insert_if_absent(ModelLightingOwner::Eye, ResolvedModelLighting::Failed);
        world.run_system_once(begin_dyn_model_lighting_frame).unwrap();
        assert!(world.resource_mut::<ModelLightingRequests>().take_pending().is_empty());
        assert_eq!(answer(&world, ModelLightingOwner::Eye), None);
        assert_eq!(world.resource::<WorldModelLightingCache>().mod_frame_count, 1);
    }

    #[test]
    fn a_new_frame_without_a_cache_still_clears_the_tables() {
        let mut world = lighting_world(false);
        ask(&mut world, ModelLightingOwner::Eye);
        world.run_system_once(begin_dyn_model_lighting_frame).unwrap();
        assert!(world.resource_mut::<ModelLightingRequests>().take_pending().is_empty());
    }

    #[test]
    fn with_no_cache_every_request_is_answered_failed() {
        let mut world = lighting_world(false);
        ask(&mut world, ModelLightingOwner::Eye);
        ask(&mut world, ModelLightingOwner::FxModel(2));
        world.run_system_once(update_fx_dyn_lighting).unwrap();
        assert_eq!(answer(&world, ModelLightingOwner::Eye), Some(ResolvedModelLighting::Failed));
        assert_eq!(
            answer(&world, ModelLightingOwner::FxModel(2)),
            Some(ResolvedModelLighting::Failed)
        );
    }

    #[test]
    fn with_no_light_grid_the_eye_is_failed_and_holds_no_handle() {
        let mut world = lighting_world(true);
        world.resource_mut::<WorldModelLightingCache>().handle = 77;
        ask(&mut world, ModelLightingOwner::Eye);
        world.run_system_once(update_dirty_model_lighting).unwrap();
        assert_eq!(answer(&world, ModelLightingOwner::Eye), Some(ResolvedModelLighting::Failed));
        assert_eq!(world.resource::<WorldModelLightingCache>().handle, 0);
    }

    #[test]
    fn only_the_glass_pass_forgets_glass_nobody_asked_about() {
        let stale = ModelLightingOwner::Glass(9);
        let live = ModelLightingOwner::Glass(1);
        for (prune, kept) in [(false, true), (true, false)] {
            let mut world = lighting_world(false);
            world
                .resource_mut::<ResolvedModelLightingTable>()
                .insert_if_absent(stale, ResolvedModelLighting::Failed);
            ask(&mut world, live);
            if prune {
                world.run_system_once(update_glass_dyn_lighting).unwrap();
            } else {
                world.run_system_once(update_dirty_model_lighting).unwrap();
            }
            assert_eq!(answer(&world, stale).is_some(), kept);
            assert!(answer(&world, live).is_some());
        }
    }

    #[test]
    fn the_glass_pass_forgets_the_cached_handles_of_glass_nobody_asked_about() {
        let mut world = lighting_world(true);
        {
            let mut c = world.resource_mut::<WorldModelLightingCache>();
            c.body_handles.insert(ModelLightingOwner::Glass(9), 11);
            c.body_handles.insert(ModelLightingOwner::Item(1), 12);
        }
        ask(&mut world, ModelLightingOwner::Glass(1));
        world.run_system_once(update_glass_dyn_lighting).unwrap();
        let c = world.resource::<WorldModelLightingCache>();
        assert!(!c.body_handles.contains_key(&ModelLightingOwner::Glass(9)));
        assert!(c.body_handles.contains_key(&ModelLightingOwner::Item(1)));
    }
}
