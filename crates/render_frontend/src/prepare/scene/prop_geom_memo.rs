//! The geometry cache of static models: lit, world-space copies of the
//! vertices of the models near the eye, skinned by worker commands and
//! uploaded as patches into the shared vertex and index buffers.

use crate::mesh_build::prop_mesh::{
    DetailRampArgs, PropCachedFacetSrc, PropGpuPlan, PropPlacement, prop_camera_detail,
};
use crate::prepare::scene::cull::prop_hide_dist_skips_slot;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::eye_parms::{StagedStageEye, FrozenVisibilitySet};
use crate::prepare::scene::world::LevelStage;
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use lighting_t6::{
    CachePropFacet, PROP_CACHE_INDEX_U16_TALLY, PropCacheBlocks, SmcCacheRow,
    CachedVertexIllum, PropIndexBakeFault, StaticModelLodCacheLayout,
    PropCachePatchLock, SkinnedCacheFacet, StaticMeshResidencyCache,
    bake_prop_indices, skin_cached_static_model_cmd_matrix, prop_lod_cache_plan,
};

use crate::{
    AddTaskOrder, SkinCachedPropOrder, TASK_ORDER_SMODELCACHE, TaskOrderBusyInput,
    TaskOrderQueues,
};

/// Forces the cache off (`Some(false)`) or leaves it to the default.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct PropCacheEnableTunable {
    pub enabled: Option<bool>,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct PrebuiltTunable {
    pub enabled: bool,
}

impl Default for PrebuiltTunable {
    /// Prebuilt geometry is on unless a dvar turns it off.
    fn default() -> Self {
        PrebuiltTunable { enabled: true }
    }
}

/// The level-of-detail ramp of rigid models. Unset values fall back to what
/// the model asset says.
#[derive(Resource, Clone, Copy, Debug)]
pub struct DetailRampTunable {
    pub mid_scale: Option<f32>,
    pub mid_bias: Option<f32>,
    pub ramp_end_scale: Option<f32>,
    pub unit_len: Option<f32>,
    pub classic_scale: f32,
    pub classic_bias: f32,
    pub classic_fov_threshold: f32,
    pub t5: asset_model::DetailParamsDirection,
}

impl Default for DetailRampTunable {
    fn default() -> Self {
        let unset = None;
        Self {
            mid_scale: unset,
            mid_bias: unset,
            ramp_end_scale: unset,
            unit_len: unset,
            classic_scale: asset_model::DEFAULT_STATIC_TIER_STRETCH,
            classic_bias: asset_model::DEFAULT_STATIC_TIER_SHIFT,
            classic_fov_threshold: asset_model::DEFAULT_VIEW_ANGLE_CUTOFF,
            t5: asset_model::DetailParamsDirection::default(),
        }
    }
}

impl DetailRampTunable {
    /// The ramp as the camera-LOD choice takes it; rigid models may always
    /// be culled out by distance.
    #[must_use]
    pub fn args(self) -> DetailRampArgs {
        let Self {
            mid_scale,
            mid_bias,
            ramp_end_scale: last_scale,
            unit_len: world_unit,
            t5,
            ..
        } = self;
        DetailRampArgs {
            mid_scale,
            mid_bias,
            prior_size: last_scale,
            map_unit: world_unit,
            t5,
            classic_no_detail_hide_out: false,
        }
    }
}

pub use render_scene::DetailRampSkinnedTunable;

/// The worker command queues the frontend feeds.
#[derive(Resource, Default)]
pub struct FrontendTaskOrders {
    pub queues: TaskOrderQueues,
}

/// Recomputes the per-axis LOD parameters for the current field of view.
pub fn update_detail_parms(
    fov_half_tan_y: f32,
    rigid: &mut DetailRampTunable,
    skinned: &mut DetailRampSkinnedTunable,
) {
    let ramps = [
        (&mut rigid.t5, [rigid.classic_scale, rigid.classic_bias, rigid.classic_fov_threshold]),
        (&mut skinned.t5, [skinned.classic_scale, skinned.classic_bias, skinned.classic_fov_threshold]),
    ];
    for (axis, [scale, bias, fov_threshold]) in ramps {
        axis.follow_view(fov_half_tan_y, scale, bias, fov_threshold);
    }
}

/// The room the allocator gave a missed surface.
#[derive(Clone, Copy, Debug)]
struct SmcGrant {
    entry: u16,
    first_vert: u32,
    vertex_points: u32,
}

impl SmcGrant {
    /// The room a cache answer hands out: only a miss needs filling.
    fn from_answer(answer: CachePropFacet) -> Option<Self> {
        let CachePropFacet::Miss {
            cache_slot_no: entry,
            base_vertex_index: first_vert,
            vertex_points,
        } = answer
        else {
            return None;
        };
        Some(Self { entry, first_vert, vertex_points })
    }
}

/// How a cached model is lit: its packed lighting word, its lighting atlas
/// handle and the atlas height factor.
#[derive(Clone, Copy, Debug, Default)]
struct SmcLight {
    word: [u8; 4],
    handle: Option<u16>,
    inv_height_x256: u32,
}

impl SmcLight {
    /// Bit 1 of a material's state flags: lit by the packed vertex word
    /// rather than the atlas tile. None for an atlas-lit material when the
    /// model has no handle.
    fn for_state_flags(self, condition_bits: u8) -> Option<CachedVertexIllum> {
        if condition_bits & 2 == 0 {
            return self.handle.map(|illum_handle| CachedVertexIllum::ViaHandle {
                illum_handle,
                inv_height_x256: self.inv_height_x256,
            });
        }
        Some(CachedVertexIllum::Fixed(self.word))
    }
}

/// A skinning job that has been asked for but has not run yet.
struct SkinJob {
    /// The authored placement slot.
    seat: usize,
    /// Index of the model in the plan's mesh table.
    model_mesh: usize,
    lod: u8,
    /// Model to world, and its 3x3 for surface_directions.
    to_world: [f32; 16],
    normal_axis: [f32; 9],
    light: SmcLight,
    grant: SmcGrant,
}

/// What each frame hands to the GPU, and the jobs not yet run.
#[derive(Default)]
struct FramePatches {
    vertices: Vec<(PropCachePatchLock, Vec<u8>)>,
    indices: Vec<(u32, Vec<u8>)>,
    jobs: HashMap<u16, SkinJob>,
}

#[derive(Resource)]
pub struct LevelPropCache {
    /// For each model slot, the cache entry it holds at each cached LOD.
    pub cache_slot_no: Vec<SmcCacheRow>,
    /// The baked index buffer, `PROP_CACHE_INDEX_U16_TALLY` entries.
    pub index_list: Vec<u16>,
    /// (slot, surface range) -> the baked indices themselves.
    pub index_stretches: HashMap<(u32, u32), Vec<u16>>,
    smc: PropCacheBlocks,
    /// Cache entries whose indices are baked.
    baked_entries: HashSet<u16>,
    edition: u64,
    patches: FramePatches,
}


impl LevelPropCache {
    pub fn new(slot_total: usize) -> Self {
        Self {
            cache_slot_no: vec![SmcCacheRow::default(); slot_total],
            index_list: vec![0; PROP_CACHE_INDEX_U16_TALLY],
            index_stretches: HashMap::default(),
            smc: PropCacheBlocks::new(),
            baked_entries: HashSet::default(),
            edition: 0,
            patches: FramePatches::default(),
        }
    }

    /// The allocator paired with the model table, for one operation.
    fn smc(&mut self) -> StaticMeshResidencyCache<'_> {
        StaticMeshResidencyCache::over(&mut self.smc, &mut self.cache_slot_no)
    }

    /// How much baked content there is: entries and index runs.
    fn baked_size(&self) -> [usize; 2] {
        [self.baked_entries.len(), self.index_stretches.len()]
    }

    fn bump_edition(&mut self) {
        self.edition = self.edition.wrapping_add(1);
    }

    /// Moves the edition on when the baked content grew or shrank since
    /// `before` was measured.
    fn note_change_since(&mut self, before: [usize; 2]) {
        if self.baked_size() != before {
            self.bump_edition();
        }
    }

    /// Releases every cache entry the slot holds and forgets what was baked
    /// for it.
    pub fn release_slot(&mut self, seat: usize) {
        let Some(&row) = self.cache_slot_no.get(seat) else {
            return;
        };
        let before = self.baked_size();
        let mut allocator = StaticMeshResidencyCache::over(&mut self.smc, &mut self.cache_slot_no);
        for entry in row {
            allocator.uncache_index(entry);
            self.baked_entries.remove(&entry);
        }
        let owner = seat as u32;
        self.index_stretches.retain(|key, _| key.0 != owner);
        self.note_change_since(before);
    }

    /// Bakes `tri_tally` triangles of `source_indices` into the shared index
    /// buffer for the block at `base_vertex_index`; returns where they start.
    pub fn bake_index_run(
        &mut self,
        base_vertex_index: u32,
        figure_part_base_index: u32,
        figure_part_vertex_offset: u16,
        tri_tally: u16,
        source_indices: &[u16],
    ) -> Result<u32, PropIndexBakeFault> {
        let write_target = self.index_list.as_mut_slice();
        let (block, base, offset) = (base_vertex_index, figure_part_base_index, figure_part_vertex_offset);
        bake_prop_indices(write_target, block, base, offset, tri_tally, source_indices)
    }

    pub fn vb_patches(&self) -> &[(PropCachePatchLock, Vec<u8>)] {
        self.patches.vertices.as_slice()
    }

    pub fn index_patches(&self) -> &[(u32, Vec<u8>)] {
        self.patches.indices.as_slice()
    }

    /// The cache entries whose indices are baked, ascending.
    pub fn baked_index_list(&self) -> Vec<u16> {
        let ordered: std::collections::BTreeSet<u16> = self.baked_entries.iter().copied().collect();
        ordered.into_iter().collect()
    }

    /// Changes whenever baked indices are added or dropped.
    pub fn content_edition(&self) -> u64 {
        let Self { edition, .. } = self;
        *edition
    }

    /// Asks the allocator for room for `vertex_points` vertices of the slot's model
    /// at the LOD slot given.
    fn cache_facet(
        &mut self,
        seat: usize,
        detail_slot: u8,
        bank: u8,
        size_band: u8,
        vertex_points: u32,
    ) -> CachePropFacet {
        let Ok(model) = u16::try_from(seat) else {
            return CachePropFacet::Declined;
        };
        let row = self.cache_slot_no.get(seat);
        let held = row.and_then(|entries| entries.get(usize::from(detail_slot)).copied()).unwrap_or_default();
        self.smc().cache_facet(bank, size_band, model, detail_slot, held, vertex_points, true)
    }

    /// Empties the cache and everything derived from it.
    fn clear_all(&mut self) {
        let frame = self.smc.frame;
        self.smc().clear(frame);
        let before = self.baked_size();
        self.baked_entries.clear();
        self.index_stretches.clear();
        self.note_change_since(before);
        self.patches = FramePatches::default();
    }

    /// Starts a frame: ages the cache and forgets last frame's patches.
    fn begin_frame(&mut self) {
        self.smc.begin_frame();
        self.patches = FramePatches::default();
    }

    /// Bakes one surface's indices for the model in `slot`, whose block
    /// starts at `first_vert`, and queues their upload. False when the
    /// surface's indices do not fit.
    fn bake_surface(
        &mut self,
        plan_indices: &[u32],
        seat: usize,
        first_vert: u32,
        surf: &PropCachedFacetSrc,
    ) -> bool {
        let Some(local) =
            local_u16_indices(plan_indices, surf.first_triangle_slot, surf.idx_count, surf.base_vertex_at)
        else {
            return false;
        };
        let triangles = (surf.idx_count / 3) as u16;
        let base = u32::from(surf.figure_part_base_index);
        let baked = self.bake_index_run(first_vert, base, surf.figure_part_vertex_offset, triangles, &local);
        let Ok(at) = baked else {
            return false;
        };
        // Triangles are baked in pairs, six indices a pair.
        let len = usize::from(triangles / 2) * 6;
        let from = at as usize;
        if let Some(run) = self.index_list.get(from..from.saturating_add(len)).map(<[u16]>::to_vec) {
            let bytes = bytemuck::cast_slice::<u16, u8>(&run).to_vec();
            self.patches.indices.push((at.saturating_mul(2), bytes));
            self.index_stretches.insert((seat as u32, surf.span_idx), run);
        }
        true
    }
}

/// What one visible placement needs cached, once all the gates have passed.
struct CacheRequest {
    lod: u8,
    spec: StaticModelLodCacheLayout,
    vertex_tally: u32,
    word: [u8; 4],
    handle: Option<u16>,
}

/// What `cache_visible_prop_facets` reads and writes.
#[derive(SystemParam)]
pub(crate) struct SmcCacheFrame<'w> {
    cache: Option<ResMut<'w, LevelPropCache>>,
    plan: Option<Res<'w, PropGpuPlan>>,
    lit_info: Option<Res<'w, LevelPropIllum>>,
    scene: Res<'w, LevelStage>,
    live_view: Res<'w, StagedStageEye>,
    pvs_lock: Res<'w, FrozenVisibilitySet>,
    switch: Res<'w, PropCacheEnableTunable>,
    ramp: Res<'w, DetailRampTunable>,
    workers: ResMut<'w, FrontendTaskOrders>,
}

/// Asks for cache room for every visible, near enough static model and
/// queues the skinning of each one that missed.
pub(crate) fn cache_visible_prop_facets(frame: SmcCacheFrame) {
    let SmcCacheFrame {
        mut cache,
        plan,
        lit_info: lighting,
        scene,
        live_view,
        pvs_lock,
        switch,
        ramp,
        mut workers,
    } = frame;
    let (Some(cache), Some(plan)) = (cache.as_deref_mut(), plan) else {
        return;
    };
    if plan.precise_compact_vertices().is_err() {
        return;
    }
    let switched_off = switch.enabled == Some(false);
    if switched_off {
        cache.clear_all();
    } else {
        cache.begin_frame();
    }
    let _ = workers.queues.reset_kind(TASK_ORDER_SMODELCACHE);
    if switched_off {
        return;
    }
    let Some(lighting) = lighting.as_deref() else {
        return;
    };

    let eye = live_view.primed.then(|| pvs_lock.walk_eye(&live_view));
    let culled = scene.culling.as_ref();
    let dists = culled.map(|c| c.prop_hide_dists.as_slice()).unwrap_or_default();
    let shown = |slot: usize| culled.and_then(|c| c.prop_vis.get(slot)).is_some_and(|&v| v != 0);
    let wanted = plan.placements.iter().filter_map(|placement| {
        let slot = placement.illum_slot?;
        let near =
            shown(slot) && !prop_hide_dist_skips_slot(dists, slot, placement.at_pos, eye, ramp.ramp_end_scale);
        near.then_some((slot, placement))
    });
    for (slot, placement) in wanted {
        let _ = queue_skin(cache, &plan, placement, slot, lighting, eye, &ramp, &mut workers.queues);
    }
}

/// Gets cache room for one placement; on a miss queues its skinning (or,
/// with the queue full, skins it now) and returns the cache entry used.
#[allow(clippy::too_many_arguments)]
fn queue_skin(
    cache: &mut LevelPropCache,
    plan: &PropGpuPlan,
    placement: &PropPlacement,
    slot_no: usize,
    lighting: &LevelPropIllum,
    eye_pos: Option<Vec3>,
    ramp: &DetailRampTunable,
    queues: &mut TaskOrderQueues,
) -> Option<u16> {
    let request = cache_request(plan, placement, lighting, eye_pos, ramp)?;
    let patch_vert = cache.smc.patched_vertex_total;
    let spec = request.spec;
    let answer =
        cache.cache_facet(slot_no, spec.detail_slot, spec.bank_no, spec.size_band, request.vertex_tally);
    let grant = SmcGrant::from_answer(answer)?;
    let first_patch_vertex = u16::try_from(patch_vert).ok()?;

    let job = SkinJob {
        seat: slot_no,
        model_mesh: placement.model_slot,
        lod: request.lod,
        to_world: skin_cached_static_model_cmd_matrix(placement.at_pos, placement.axes_set, placement.size_k),
        normal_axis: lighting_t6::unit_axis_transform(&skin_cached_static_model_cmd_matrix(
            [0.0; 3],
            placement.axes_set,
            1.0,
        )),
        light: SmcLight {
            word: request.word,
            handle: request.handle,
            inv_height_x256: lighting.inv_height_x256,
        },
        grant,
    };
    let cmd = SkinCachedPropOrder {
        cache_slot_no: grant.entry,
        first_patch_vertex,
    };
    match queues.queue_record(TASK_ORDER_SMODELCACHE, &cmd.as_bytes_le()).ok()? {
        AddTaskOrder::Queued => {
            cache.patches.jobs.insert(grant.entry, job);
        }
        AddTaskOrder::SpillInline => run_skin_job(cache, plan, job),
    }
    Some(grant.entry)
}

/// Decides whether the placement is worth caching and, if so, at which LOD
/// and in which bank. None when it has no lit sample, no surface at its LOD
/// or no cache class.
fn cache_request(
    plan: &PropGpuPlan,
    placement: &PropPlacement,
    lighting: &LevelPropIllum,
    eye_pos: Option<Vec3>,
    ramp: &DetailRampTunable,
) -> Option<CacheRequest> {
    let slot = placement.illum_slot?;
    let word = lighting.compact_illum_for_slot(slot)?;
    if word == [0; 4] {
        return None;
    }
    let handle = lighting.handle_list.get(slot).copied().filter(|&h| h != 0);
    let model = plan.meshes.get(placement.model_slot)?;
    let lod = prop_camera_detail(model.lod, placement.at_pos, placement.size_k, eye_pos, ramp.args())?;
    let at = usize::from(lod);
    let has_surfaces = model.prop_cache_facets_by_detail.get(at).is_some_and(|surfs| !surfs.is_empty());
    if !has_surfaces {
        return None;
    }
    let row_bytes = model.detail_prop_cache_rows.and_then(|rows| rows.get(at).copied());
    let spec = prop_lod_cache_plan(row_bytes.or(model.detail_prop_cache)?, placement.mode_bits)?;
    Some(CacheRequest {
        lod,
        spec,
        vertex_tally: model.vertex_tally_by_detail.get(at).copied().unwrap_or_default(),
        word,
        handle,
    })
}

/// What `skin_cached_prop_order` reads and writes.
#[derive(SystemParam)]
pub(crate) struct SmcSkinFrame<'w> {
    cache: Option<ResMut<'w, LevelPropCache>>,
    plan: Option<Res<'w, PropGpuPlan>>,
    workers: ResMut<'w, FrontendTaskOrders>,
}

/// Runs the skinning jobs the workers have been told to do.
pub(crate) fn skin_cached_prop_order(frame: SmcSkinFrame) {
    let SmcSkinFrame {
        mut cache,
        plan,
        mut workers,
    } = frame;
    let (Some(cache), Some(plan)) = (cache.as_deref_mut(), plan) else {
        return;
    };
    let mut entries: Vec<u16> = Vec::new();
    let _ = workers.queues.wait_for_type(
        TASK_ORDER_SMODELCACHE,
        TaskOrderBusyInput::default(),
        |bytes| entries.extend(SkinCachedPropOrder::parse_bytes(bytes).map(|cmd| cmd.cache_slot_no)),
    );
    let jobs: Vec<SkinJob> = entries
        .into_iter()
        .filter_map(|entry| cache.patches.jobs.remove(&entry))
        .collect();
    for job in jobs {
        run_skin_job(cache, &plan, job);
    }
}

/// Skins the job's model into a vertex patch and bakes its indices.
fn run_skin_job(cache: &mut LevelPropCache, plan: &PropGpuPlan, job: SkinJob) {
    let Ok(rows) = plan.precise_compact_vertices() else {
        return;
    };
    let grant = job.grant;
    let Some(lock) = lighting_t6::lock_prop_cache_patch(grant.first_vert, grant.vertex_points)
    else {
        return;
    };
    let Some(mesh) = plan.meshes.get(job.model_mesh) else {
        return;
    };
    let surfaces: &[PropCachedFacetSrc] = mesh
        .prop_cache_facets_by_detail
        .get(job.lod as usize)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let skin_input = |surf: &PropCachedFacetSrc| {
        let from = surf.compact_off as usize;
        let packed = rows.get(from..from.saturating_add(surf.compact_tally as usize))?;
        let condition_bits = *plan.mat_state_flags.get(&surf.mat?)?;
        Some(SkinnedCacheFacet {
            packed,
            first_vertex: surf.figure_part_vertex_offset,
            lighting: job.light.for_state_flags(condition_bits)?,
        })
    };
    let skin: Vec<SkinnedCacheFacet<'_>> = surfaces.iter().filter_map(skin_input).collect();
    let mut patch = vec![0u8; lock.payload_extent_bytes as usize];
    let _ =
        lighting_t6::skin_cached_prop_order(&mut patch, &job.to_world, &job.normal_axis, &skin);

    let indices = plan.indices();
    let any_baked = surfaces.iter().fold(false, |baked, facet| {
        cache.bake_surface(indices, job.seat, grant.first_vert, facet) | baked
    });
    cache.patches.vertices.push((lock, patch));
    if any_baked {
        cache.baked_entries.insert(grant.entry);
        cache.bump_edition();
    }
}

/// The `count` indices from `start`, rebased to the surface's first vertex.
/// None if they run off the buffer, precede `base_vertex_at` or exceed 16 bits.
fn local_u16_indices(index_list: &[u32], start: u32, count: u32, base_vertex_at: u32) -> Option<Vec<u16>> {
    let from = start as usize;
    let to = from.checked_add(count as usize)?;
    let rebase = |&index: &u32| index.checked_sub(base_vertex_at).and_then(|i| u16::try_from(i).ok());
    index_list.get(from..to)?.iter().map(rebase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lighting_t6::STATIC_MODEL_CACHE_DETAIL_LEVELS;

    /// Asks for room for a model of 30 vertices (size class 3) in bank 0.
    fn ask(cache: &mut LevelPropCache, seat: usize, lod: u8) -> CachePropFacet {
        cache.cache_facet(seat, lod, 0, 3, 30)
    }

    /// Room for a model of 32 vertices (size class 3) in bank 0.
    fn miss(cache: &mut LevelPropCache, seat: usize, lod: u8) -> SmcGrant {
        let answer = ask(cache, seat, lod);
        SmcGrant::from_answer(answer).unwrap_or_else(|| panic!("expected a miss, got {answer:?}"))
    }

    fn is_miss(answer: CachePropFacet) -> bool {
        matches!(answer, CachePropFacet::Miss { .. })
    }

    fn hit(entry: u16) -> CachePropFacet {
        CachePropFacet::Hit { cache_slot_no: entry }
    }

    fn job(grant: SmcGrant) -> SkinJob {
        SkinJob {
            seat: 0,
            model_mesh: 0,
            lod: 0,
            to_world: [0.0; 16],
            normal_axis: [0.0; 9],
            light: SmcLight::default(),
            grant,
        }
    }

    fn empty_row() -> SmcCacheRow {
        [0; STATIC_MODEL_CACHE_DETAIL_LEVELS]
    }

    #[test]
    fn a_new_cache_holds_nothing() {
        let cache = LevelPropCache::new(5);
        assert_eq!(cache.cache_slot_no, vec![empty_row(); 5]);
        assert_eq!(cache.index_list.len(), PROP_CACHE_INDEX_U16_TALLY);
        assert_eq!(cache.content_edition(), 0);
        assert!(cache.baked_index_list().is_empty());
        assert!(cache.vb_patches().is_empty() && cache.index_patches().is_empty());
    }

    #[test]
    fn a_surface_misses_once_then_hits() {
        let mut cache = LevelPropCache::new(4);
        let grant = miss(&mut cache, 2, 1);
        assert_ne!(grant.entry, 0);
        // Size class 3 holds 32 vertices.
        assert_eq!(grant.vertex_points, 32);
        assert_eq!(cache.cache_slot_no[2][1], grant.entry);
        assert_eq!((cache.smc.patched_surface_total, cache.smc.patched_vertex_total), (1, 32));
        assert_eq!(ask(&mut cache, 2, 1), hit(grant.entry));
        // A hit asks for no patch.
        assert_eq!(cache.smc.patched_surface_total, 1);
    }

    #[test]
    fn two_models_get_two_places() {
        let mut cache = LevelPropCache::new(4);
        let a = miss(&mut cache, 0, 0);
        let b = miss(&mut cache, 1, 0);
        assert_ne!(a.entry, b.entry);
        assert_ne!(a.first_vert, b.first_vert);
        // The same model at another LOD is another entry too.
        let c = miss(&mut cache, 0, 1);
        assert!(![a.entry, b.entry].contains(&c.entry));
    }

    #[test]
    fn what_cannot_be_cached_is_refused() {
        let mut cache = LevelPropCache::new(2);
        let refused = [
            // A slot a 16-bit model index cannot name.
            (70_000, 0, 3, 30),
            // A size class that does not exist, and too many vertices for any.
            (0, 0, 200, 5_000),
            // A bank that does not exist.
            (0, 9, 3, 30),
        ];
        for (slot, bank, class, vertex_points) in refused {
            let answer = cache.cache_facet(slot, 0, bank, class, vertex_points);
            assert_eq!(answer, CachePropFacet::Declined, "{slot} {bank} {class}");
        }
        assert_eq!((cache.smc.patched_surface_total, cache.smc.patched_vertex_total), (0, 0));
    }

    #[test]
    fn a_bad_size_class_is_chosen_again_from_the_vertex_count() {
        let mut cache = LevelPropCache::new(2);
        // 100 vertices fit the class of 102.
        let grant = SmcGrant::from_answer(cache.cache_facet(0, 0, 0, 200, 100));
        assert_eq!(grant.map(|g| g.vertex_points), Some(102));
    }

    #[test]
    fn a_frame_takes_no_more_than_its_patch_allowance() {
        let mut cache = LevelPropCache::new(300);
        // The lighting cache's per-frame patch allowance (0x100 surfaces).
        let allowance = 0x100_usize;
        let small = |cache: &mut LevelPropCache, slot| cache.cache_facet(slot, 0, 0, 0, 10);
        assert!((0..allowance).all(|slot| is_miss(small(&mut cache, slot))));
        assert_eq!(small(&mut cache, allowance), CachePropFacet::Declined);
        // The next frame starts the allowance again.
        cache.begin_frame();
        assert!(is_miss(small(&mut cache, allowance)));
    }

    #[test]
    fn a_new_frame_forgets_the_patches_but_keeps_the_entries() {
        let mut cache = LevelPropCache::new(2);
        let grant = miss(&mut cache, 1, 0);
        cache.patches.indices.push((0, vec![1, 2]));
        cache.patches.jobs.insert(grant.entry, job(grant));
        let frame = cache.smc.frame;
        cache.begin_frame();
        assert_eq!(cache.smc.frame, frame.wrapping_add(1));
        assert_eq!((cache.smc.patched_surface_total, cache.smc.patched_vertex_total), (0, 0));
        assert!(cache.index_patches().is_empty() && cache.vb_patches().is_empty());
        assert!(cache.patches.jobs.is_empty());
        assert_eq!(cache.cache_facet(1, 0, 0, 3, 30), hit(grant.entry));
    }

    #[test]
    fn dropping_a_slot_drops_its_entries_and_what_was_baked_for_it() {
        let mut cache = LevelPropCache::new(6);
        let a = miss(&mut cache, 3, 0).entry;
        let b = miss(&mut cache, 3, 2).entry;
        let other = miss(&mut cache, 4, 0).entry;
        cache.baked_entries.extend([a, b, other]);
        cache.index_stretches.insert((3, 0), vec![1; 6]);
        cache.index_stretches.insert((3, 7), vec![3; 6]);
        cache.index_stretches.insert((4, 0), vec![2; 6]);

        cache.release_slot(3);
        assert_eq!(cache.cache_slot_no[3], empty_row());
        assert_eq!(cache.cache_slot_no[4][0], other);
        assert_eq!(cache.baked_index_list(), [other]);
        assert_eq!(cache.index_stretches.keys().copied().collect::<Vec<_>>(), [(4, 0)]);
        assert_eq!(cache.content_edition(), 1);
        // It is a miss again, and the other model's entry is still a hit.
        assert!(is_miss(ask(&mut cache, 3, 0)));
        assert_eq!(ask(&mut cache, 4, 0), hit(other));
    }

    #[test]
    fn dropping_a_slot_with_nothing_baked_leaves_the_revision_alone() {
        let mut cache = LevelPropCache::new(3);
        miss(&mut cache, 1, 0);
        cache.release_slot(1);
        assert_eq!(cache.content_edition(), 0);
        // And a slot that does not exist is not an error.
        cache.release_slot(99);
        assert_eq!(cache.content_edition(), 0);
    }

    #[test]
    fn clearing_the_cache_empties_everything_derived_from_it() {
        let mut cache = LevelPropCache::new(3);
        let grant = miss(&mut cache, 0, 0);
        miss(&mut cache, 2, 3);
        cache.baked_entries.insert(grant.entry);
        cache.index_stretches.insert((0, 0), vec![0; 6]);
        cache.patches.indices.push((0, vec![0; 2]));
        cache.patches.jobs.insert(grant.entry, job(grant));

        cache.clear_all();
        assert!(cache.cache_slot_no.iter().all(|row| *row == empty_row()));
        assert!(cache.baked_index_list().is_empty() && cache.index_stretches.is_empty());
        assert!(cache.index_patches().is_empty() && cache.patches.jobs.is_empty());
        assert_eq!(cache.content_edition(), 1);
        assert!(is_miss(ask(&mut cache, 0, 0)));
        // Nothing was left to drop the second time.
        cache.clear_all();
        assert_eq!(cache.content_edition(), 1);
    }

    #[test]
    fn baked_entries_are_listed_in_ascending_order() {
        let mut cache = LevelPropCache::new(1);
        cache.baked_entries.extend([9, 3, 5]);
        assert_eq!(cache.baked_index_list(), [3, 5, 9]);
    }

    #[test]
    fn baked_indices_are_rebased_to_the_cache_vertex() {
        let mut cache = LevelPropCache::new(1);
        let src: Vec<u16> = (0..12).collect();
        // Two pairs of triangles: 12 indices at 10 * 4 + 2 * 3.
        assert_eq!(cache.bake_index_run(10, 2, 5, 4, &src), Ok(46));
        let want: Vec<u16> = src.iter().map(|i| i + 15).collect();
        assert_eq!(cache.index_list[46..58], want[..]);
        assert_eq!((cache.index_list[45], cache.index_list[58]), (0, 0));
    }

    #[test]
    fn an_odd_triangle_is_not_baked() {
        let mut cache = LevelPropCache::new(1);
        assert!(cache.bake_index_run(0, 0, 0, 3, &[1u16; 9]).is_ok());
        assert_eq!(cache.index_list[..7], [1, 1, 1, 1, 1, 1, 0]);
    }

    #[test]
    fn baked_indices_wrap_at_16_bits() {
        let mut cache = LevelPropCache::new(1);
        let src = [0xffff, 0, 0, 0, 0, 0];
        assert!(cache.bake_index_run(0, 0, 2, 2, &src).is_ok());
        assert_eq!(cache.index_list[..2], [1, 2]);
    }

    #[test]
    fn baking_says_why_it_cannot() {
        let mut cache = LevelPropCache::new(1);
        let vb_verts = (PROP_CACHE_INDEX_U16_TALLY / 4) as u32;
        let cases = [
            (0, &[0u16; 11][..], PropIndexBakeFault::InputTooShort),
            (vb_verts, &[0; 12][..], PropIndexBakeFault::ShortDest),
            (u32::MAX, &[0; 12][..], PropIndexBakeFault::SlotsExhausted),
        ];
        for (block, src, why) in cases {
            assert_eq!(cache.bake_index_run(block, 0, 0, 4, src), Err(why));
        }
    }

    #[test]
    fn indices_are_cut_from_the_plan_and_rebased() {
        let plan = [100u32, 101, 102, 103, 104];
        assert_eq!(local_u16_indices(&plan, 1, 3, 100), Some(vec![1, 2, 3]));
        assert_eq!(local_u16_indices(&plan, 5, 0, 100), Some(Vec::new()));
    }

    #[test]
    fn indices_that_do_not_fit_are_refused() {
        let plan = [100u32, 101, 102, 103, 104];
        let refused = [
            // Off the end of the buffer.
            (&plan[..], 3, 3, 0),
            (&plan[..], 9, 1, 0),
            // Below the surface's first vertex.
            (&plan[..], 0, 2, 101),
            // More than 16 bits once rebased.
            (&[70_000][..], 0, 1, 0),
            (&[70_000][..], 0, 1, 4_464),
        ];
        for (src, start, count, base) in refused {
            assert_eq!(local_u16_indices(src, start, count, base), None, "{start} {count} {base}");
        }
        // 65 535 is the last one that does.
        assert_eq!(local_u16_indices(&[70_000], 0, 1, 4_465), Some(vec![65_535]));
    }

    #[test]
    fn the_ramp_dvar_hands_its_values_on() {
        let set = DetailRampTunable {
            mid_scale: Some(1.5),
            mid_bias: Some(0.25),
            ramp_end_scale: Some(2.0),
            unit_len: Some(3.0),
            ..DetailRampTunable::default()
        }
        .args();
        let values = |a: &DetailRampArgs| (a.mid_scale, a.mid_bias, a.prior_size, a.map_unit);
        assert_eq!(values(&set), (Some(1.5), Some(0.25), Some(2.0), Some(3.0)));
        assert!(!set.classic_no_detail_hide_out);
        assert_eq!(values(&DetailRampTunable::default().args()), (None, None, None, None));
    }

    #[test]
    fn the_cache_is_on_unless_a_dvar_says_otherwise() {
        assert_eq!(PropCacheEnableTunable::default().enabled, None);
        assert!(PrebuiltTunable::default().enabled);
    }

    #[test]
    fn an_atlas_lit_material_needs_a_handle() {
        let light = SmcLight {
            word: [1, 2, 3, 4],
            handle: None,
            inv_height_x256: 7,
        };
        assert!(light.for_state_flags(0).is_none());
        assert!(matches!(light.for_state_flags(2), Some(CachedVertexIllum::Fixed([1, 2, 3, 4]))));
        let with_handle = SmcLight {
            handle: Some(9),
            ..light
        };
        assert!(matches!(
            with_handle.for_state_flags(0),
            Some(CachedVertexIllum::ViaHandle {
                illum_handle: 9,
                inv_height_x256: 7
            })
        ));
    }
}
