//! The geometry cache of static models: lit, world-space copies of the
//! vertices of the models near the eye, skinned by worker commands and
//! uploaded as patches into the shared vertex and index buffers.

use crate::assemble::drawsurf::tess::smodel::{LodRampArgs, SmodelGpuPlan, smodel_camera_lod};
use crate::prepare::scene::cull::smodel_cull_dist_skips_slot;
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::view_parms::{PreparedSceneView, RLockPvs};
use crate::prepare::scene::world::WorldScene;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use lighting_t6::{
    CacheStaticModelSurface, SMC_BANK_N, SMC_CACHE_INDEX_LODS, SMC_CLASS_N, SMC_INDEX_U16_N,
    SMC_LEAF_N, SMC_LINK_N, SMC_TREE_N, SmcAllocatorStorage, SmcCachedVertLighting,
    SmcIndexBakeError, SmcLeafStorage, SmcPatchLock, SmcSkinSurface, SmcTree, StaticModelCache,
    cache_static_model_indices, skin_cached_static_model_cmd_matrix, smc_lod_cache_spec,
};
use render_frontend::{
    AddWorkerCmd, SkinCachedStaticModelCmd, WORKER_CMD_SMODELCACHE, WorkerCmdBusyInput,
    WorkerCmdQueues,
};

/// Forces the cache off (`Some(false)`) or leaves it to the default.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SmcEnableDvar {
    pub enabled: Option<bool>,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct PretessDvar {
    pub enabled: bool,
}

impl Default for PretessDvar {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// The level-of-detail ramp of rigid models. Unset values fall back to what
/// the model asset says.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LodRampDvar {
    pub scale_mid: Option<f32>,
    pub bias_mid: Option<f32>,
    pub scale_last: Option<f32>,
    pub world_unit: Option<f32>,
    pub t5_scale: f32,
    pub t5_bias: f32,
    pub t5_fov_threshold: f32,
    pub t5: asset_model::LodParmsAxis,
}

impl Default for LodRampDvar {
    fn default() -> Self {
        Self {
            scale_mid: None,
            bias_mid: None,
            scale_last: None,
            world_unit: None,
            t5_scale: asset_model::R_LOD_SCALE_RIGID_DEFAULT,
            t5_bias: asset_model::R_LOD_BIAS_RIGID_DEFAULT,
            t5_fov_threshold: asset_model::R_FOV_SCALE_THRESHOLD_DEFAULT,
            t5: asset_model::LodParmsAxis::default(),
        }
    }
}

impl LodRampDvar {
    #[must_use]
    pub fn args(self) -> LodRampArgs {
        LodRampArgs {
            scale_mid: self.scale_mid,
            bias_mid: self.bias_mid,
            scale_last: self.scale_last,
            world_unit: self.world_unit,
            t5: self.t5,
            t5_no_lod_cull_out: false,
        }
    }
}

pub use render_scene::LodRampSkinnedDvar;

/// The worker command queues the frontend feeds.
#[derive(Resource, Default)]
pub struct FrontendWorkerCmds {
    pub queues: WorkerCmdQueues,
}

/// Recomputes the per-axis LOD parameters for the current field of view.
pub fn update_lod_parms(
    tan_half_fov_y: f32,
    rigid: &mut LodRampDvar,
    skinned: &mut LodRampSkinnedDvar,
) {
    let (scale, bias, threshold) = (rigid.t5_scale, rigid.t5_bias, rigid.t5_fov_threshold);
    rigid.t5.update(tan_half_fov_y, scale, bias, threshold);
    let (scale, bias, threshold) = (skinned.t5_scale, skinned.t5_bias, skinned.t5_fov_threshold);
    skinned.t5.update(tan_half_fov_y, scale, bias, threshold);
}

/// A skinning job that has been asked for but has not run yet.
struct PendingSmcSkin {
    authored: usize,
    mesh: usize,
    lod: u8,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    scale: f32,
    packed_light: [u8; 4],
    lighting_handle: Option<u16>,
    inv_height_x256: u32,
    cache_index: u16,
    base_vert_index: u32,
    class_verts: u32,
}

#[derive(Resource)]
pub struct WorldStaticModelCache {
    content_revision: u64,
    /// For each model slot, the cache entry it holds at each cached LOD.
    pub cache_index: Vec<[u16; SMC_CACHE_INDEX_LODS]>,

    // Backing arrays of the cache, lent to `StaticModelCache` per call.
    leaf_word0: Vec<u32>,
    leaf_base: Vec<u32>,
    leaf_frame: Vec<i32>,
    leaf_verts: Vec<u32>,
    link_next: Vec<u16>,
    link_prev: Vec<u16>,
    trees: Vec<SmcTree>,
    used_head_next: Vec<u16>,
    used_head_prev: Vec<u16>,
    level_sweep: Vec<i32>,
    budget: Vec<i32>,
    frame: i32,
    patch_surfs: u32,
    patch_verts: u32,

    /// The baked index buffer, `SMC_INDEX_U16_N` entries.
    pub indices: Vec<u16>,
    pending_vb: Vec<(SmcPatchLock, Vec<u8>)>,
    pending_ib: Vec<(u32, Vec<u8>)>,
    /// Cache entries whose indices are baked.
    index_baked: HashSet<u16>,
    /// (slot, surface range) -> (first index, index count) in `indices`.
    pub draw_ranges: HashMap<(u32, u32), (u32, u32)>,
    /// (slot, surface range) -> the baked indices themselves.
    pub index_runs: HashMap<(u32, u32), Vec<u16>>,
    pending_skin: HashMap<u16, PendingSmcSkin>,
}

impl WorldStaticModelCache {
    pub fn new(slot_count: usize) -> Self {
        let mut cache = Self {
            content_revision: 0,
            cache_index: vec![[0; SMC_CACHE_INDEX_LODS]; slot_count],
            leaf_word0: vec![0; SMC_LEAF_N],
            leaf_base: vec![0; SMC_LEAF_N],
            leaf_frame: vec![0; SMC_LEAF_N],
            leaf_verts: vec![0; SMC_LEAF_N],
            link_next: vec![0; SMC_LINK_N],
            link_prev: vec![0; SMC_LINK_N],
            trees: vec![SmcTree::default(); SMC_TREE_N],
            used_head_next: vec![0; SMC_BANK_N],
            used_head_prev: vec![0; SMC_BANK_N],
            level_sweep: vec![0; SMC_BANK_N * SMC_CLASS_N],
            budget: vec![0; SMC_BANK_N * SMC_CLASS_N],
            frame: 0,
            patch_surfs: 0,
            patch_verts: 0,
            indices: vec![0; SMC_INDEX_U16_N],
            pending_vb: Vec::new(),
            pending_ib: Vec::new(),
            index_baked: HashSet::new(),
            draw_ranges: HashMap::new(),
            index_runs: HashMap::new(),
            pending_skin: HashMap::new(),
        };
        cache.with_smc(|smc| smc.clear(0));
        cache
    }

    /// Lends the backing arrays to a `StaticModelCache` for the duration of `f`.
    fn with_smc<R>(&mut self, f: impl FnOnce(&mut StaticModelCache<'_>) -> R) -> R {
        let leaves = SmcLeafStorage::new(
            &mut self.leaf_word0,
            &mut self.leaf_base,
            &mut self.leaf_frame,
            self.cache_index.as_flattened_mut(),
            &mut self.leaf_verts,
        )
        .expect("SMC leaf arrays are sized at spawn");
        let allocator = SmcAllocatorStorage::new(
            &mut self.link_next,
            &mut self.link_prev,
            &mut self.used_head_next,
            &mut self.used_head_prev,
            &mut self.level_sweep,
            &mut self.budget,
            &mut self.trees,
        )
        .expect("SMC allocator arrays are sized at spawn");
        let mut smc = StaticModelCache::new(leaves, allocator);
        smc.frame = self.frame;
        smc.patch_surfs = self.patch_surfs;
        smc.patch_verts = self.patch_verts;
        let out = f(&mut smc);
        self.frame = smc.frame;
        self.patch_surfs = smc.patch_surfs;
        self.patch_verts = smc.patch_verts;
        out
    }

    /// How much baked content there is: entries, draw ranges, index runs.
    fn content_size(&self) -> (usize, usize, usize) {
        (
            self.index_baked.len(),
            self.draw_ranges.len(),
            self.index_runs.len(),
        )
    }

    fn bump_content_revision(&mut self) {
        self.content_revision = self.content_revision.wrapping_add(1);
    }

    /// Releases every cache entry the slot holds and forgets what was baked
    /// for it.
    pub fn uncache_slot(&mut self, slot: usize) {
        let Some(entries) = self.cache_index.get(slot).copied() else {
            return;
        };
        let before = self.content_size();
        self.with_smc(|smc| {
            for entry in entries {
                smc.uncache_index(entry);
            }
        });
        for entry in entries {
            self.index_baked.remove(&entry);
        }
        let owner = slot as u32;
        self.draw_ranges.retain(|&(held_by, _), _| held_by != owner);
        self.index_runs.retain(|&(held_by, _), _| held_by != owner);
        if self.content_size() != before {
            self.bump_content_revision();
        }
    }

    pub fn bake_indices(
        &mut self,
        base_vert_index: u32,
        xsurface_base_index: u32,
        xsurface_vert_offset: u16,
        tri_count: u16,
        src_indices: &[u16],
    ) -> Result<u32, SmcIndexBakeError> {
        cache_static_model_indices(
            &mut self.indices,
            base_vert_index,
            xsurface_base_index,
            xsurface_vert_offset,
            tri_count,
            src_indices,
        )
    }

    pub fn vb_patches(&self) -> &[(SmcPatchLock, Vec<u8>)] {
        &self.pending_vb
    }

    pub fn ib_patches(&self) -> &[(u32, Vec<u8>)] {
        &self.pending_ib
    }

    /// The cache entries whose indices are baked, ascending.
    pub fn baked_cache_indices(&self) -> Vec<u16> {
        let mut entries: Vec<u16> = self.index_baked.iter().copied().collect();
        entries.sort_unstable();
        entries
    }

    /// Changes whenever baked indices or draw ranges are added or dropped.
    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }

    /// Asks the allocator for room for `verts` vertices of the slot's model
    /// at the LOD slot given.
    fn cache_surface(
        &mut self,
        authored_slot: usize,
        lod_slot: u8,
        bank: u8,
        size_class: u8,
        verts: u32,
    ) -> CacheStaticModelSurface {
        let held = self
            .cache_index
            .get(authored_slot)
            .and_then(|row| row.get(usize::from(lod_slot)))
            .copied()
            .unwrap_or(0);
        let Ok(smodel_index) = u16::try_from(authored_slot) else {
            return CacheStaticModelSurface::Refused;
        };
        self.with_smc(|smc| {
            smc.cache_surface(bank, size_class, smodel_index, lod_slot, held, verts, true)
        })
    }

    /// Empties the cache and everything derived from it.
    fn clear_all(&mut self) {
        for row in &mut self.cache_index {
            *row = [0; SMC_CACHE_INDEX_LODS];
        }
        self.with_smc(|smc| {
            let frame = smc.frame;
            smc.clear(frame);
        });
        let before = self.content_size();
        self.index_baked.clear();
        self.draw_ranges.clear();
        self.index_runs.clear();
        if before != self.content_size() {
            self.bump_content_revision();
        }
        self.drop_pending();
    }

    fn drop_pending(&mut self) {
        self.pending_vb.clear();
        self.pending_ib.clear();
        self.pending_skin.clear();
    }

    /// Starts a frame: ages the cache and forgets last frame's patches.
    fn begin_frame(&mut self) {
        self.with_smc(|smc| {
            smc.frame = smc.frame.wrapping_add(1);
            smc.patch_surfs = 0;
            smc.patch_verts = 0;
        });
        self.drop_pending();
    }
}

/// What one visible placement needs cached, once all the gates have passed.
struct CacheRequest {
    lod: u8,
    bank: u8,
    size_class: u8,
    lod_slot: u8,
    vert_count: u32,
    packed_light: [u8; 4],
    lighting_handle: Option<u16>,
}

pub(crate) fn cache_visible_smodel_surfaces(
    mut cache: Option<ResMut<WorldStaticModelCache>>,
    plan: Option<Res<SmodelGpuPlan>>,
    lighting: Option<Res<WorldSmodelLighting>>,
    scene: Res<WorldScene>,
    prepared: Res<PreparedSceneView>,
    lock_pvs: Res<RLockPvs>,
    smc_enable: Res<SmcEnableDvar>,
    lod_ramp: Res<LodRampDvar>,
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
) {
    let (Some(cache), Some(plan)) = (cache.as_deref_mut(), plan) else {
        return;
    };
    if plan.exact_packed_vertices().is_err() {
        return;
    }
    if smc_enable.enabled == Some(false) {
        cache.clear_all();
        let _ = worker_cmds.queues.reset_type(WORKER_CMD_SMODELCACHE);
        return;
    }
    cache.begin_frame();
    let _ = worker_cmds.queues.reset_type(WORKER_CMD_SMODELCACHE);

    let Some(lighting) = lighting.as_deref() else {
        return;
    };
    let eye = prepared.ready.then(|| lock_pvs.dpvs_eye(&prepared));
    let cull = scene.cull.as_ref();
    let cull_dists = cull.map_or(&[][..], |c| c.static_model_cull_dists.as_slice());

    for placement in &plan.placements {
        let Some(authored) = placement.lighting_slot else {
            continue;
        };
        let seen = cull
            .and_then(|c| c.smodel_vis.get(authored))
            .is_some_and(|&v| v != 0);
        if !seen
            || smodel_cull_dist_skips_slot(
                cull_dists,
                authored,
                placement.origin,
                eye,
                lod_ramp.scale_last,
            )
        {
            continue;
        }
        let Some(request) = cache_request(&plan, placement, lighting, eye, &lod_ramp) else {
            continue;
        };

        let first_patch_vert = cache.patch_verts;
        let granted = cache.cache_surface(
            authored,
            request.lod_slot,
            request.bank,
            request.size_class,
            request.vert_count,
        );
        let CacheStaticModelSurface::Miss {
            cache_index,
            base_vert_index,
            verts: class_verts,
        } = granted
        else {
            continue;
        };
        let Ok(first_patch_vert) = u16::try_from(first_patch_vert) else {
            continue;
        };
        let cmd = SkinCachedStaticModelCmd {
            cache_index,
            first_patch_vert,
        };
        let job = PendingSmcSkin {
            authored,
            mesh: placement.mesh,
            lod: request.lod,
            origin: placement.origin,
            axis: placement.axis,
            scale: placement.scale,
            packed_light: request.packed_light,
            lighting_handle: request.lighting_handle,
            inv_height_x256: lighting.inv_height_x256,
            cache_index,
            base_vert_index,
            class_verts,
        };
        match worker_cmds
            .queues
            .add(WORKER_CMD_SMODELCACHE, &cmd.to_bytes())
        {
            Ok(AddWorkerCmd::Queued) => {
                cache.pending_skin.insert(cache_index, job);
            }
            Ok(AddWorkerCmd::OverflowInline) => skin_cached_static_model_job(cache, &plan, job),
            Err(_) => {}
        }
    }
}

/// Decides whether the placement is worth caching and, if so, at which LOD
/// and in which bank. None when it has no lit sample, no surface at its LOD
/// or no cache class.
fn cache_request(
    plan: &SmodelGpuPlan,
    placement: &crate::assemble::drawsurf::tess::smodel::SmodelPlacement,
    lighting: &WorldSmodelLighting,
    eye: Option<Vec3>,
    lod_ramp: &LodRampDvar,
) -> Option<CacheRequest> {
    let authored = placement.lighting_slot?;
    let packed_light = lighting
        .packed_lighting_for_slot(authored)
        .filter(|word| *word != [0; 4])?;
    let lighting_handle = lighting.handles.get(authored).copied().filter(|&h| h != 0);
    let mesh = plan.meshes.get(placement.mesh)?;
    let lod = smodel_camera_lod(
        mesh.lod,
        placement.origin,
        placement.scale,
        eye,
        lod_ramp.args(),
    )?;
    let lod_i = usize::from(lod);
    if mesh.smc_surfs_by_lod.get(lod_i)?.is_empty() {
        return None;
    }
    let class_bytes = mesh
        .lod_smc_rows
        .and_then(|rows| rows.get(lod_i).copied())
        .or(mesh.lod_smc)?;
    let spec = smc_lod_cache_spec(class_bytes, placement.flags)?;
    Some(CacheRequest {
        lod,
        bank: spec.bank,
        size_class: spec.size_class,
        lod_slot: spec.lod_slot,
        vert_count: mesh.vert_count_by_lod.get(lod_i).copied().unwrap_or(0),
        packed_light,
        lighting_handle,
    })
}

/// Runs the skinning jobs the workers have been told to do.
pub(crate) fn skin_cached_static_model_cmd(
    mut cache: Option<ResMut<WorldStaticModelCache>>,
    plan: Option<Res<SmodelGpuPlan>>,
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
) {
    let (Some(cache), Some(plan)) = (cache.as_deref_mut(), plan) else {
        return;
    };
    let mut commands = Vec::new();
    let _ = worker_cmds.queues.wait_of_type(
        WORKER_CMD_SMODELCACHE,
        WorkerCmdBusyInput::default(),
        |data| commands.extend(SkinCachedStaticModelCmd::from_bytes(data)),
    );
    for cmd in commands {
        if let Some(job) = cache.pending_skin.remove(&cmd.cache_index) {
            skin_cached_static_model_job(cache, &plan, job);
        }
    }
}

/// Skins the job's model into a vertex patch and bakes its indices.
fn skin_cached_static_model_job(
    cache: &mut WorldStaticModelCache,
    plan: &SmodelGpuPlan,
    job: PendingSmcSkin,
) {
    let Ok(packed_rows) = plan.exact_packed_vertices() else {
        return;
    };
    let Some(lock) =
        lighting_t6::patch_static_model_cache_lock(job.base_vert_index, job.class_verts)
    else {
        return;
    };
    let Some(mesh) = plan.meshes.get(job.mesh) else {
        return;
    };
    let surfaces = mesh
        .smc_surfs_by_lod
        .get(usize::from(job.lod))
        .map_or(&[][..], Vec::as_slice);

    let to_world = skin_cached_static_model_cmd_matrix(job.origin, job.axis, job.scale);
    let normal_matrix = skin_cached_static_model_cmd_matrix([0.0; 3], job.axis, 1.0);
    let normal_axis = lighting_t6::setup_transform_unit_vec(&normal_matrix);

    let skin_surfaces: Vec<SmcSkinSurface<'_>> = surfaces
        .iter()
        .filter_map(|surf| {
            let first = surf.packed_off as usize;
            let packed = packed_rows.get(first..first.saturating_add(surf.packed_n as usize))?;
            let state_flags = plan.material_state_flags.get(&surf.material?)?;
            // Bit 1 of a material's state flags: lit by the packed vertex
            // word rather than the atlas tile.
            let lighting = if state_flags & 2 != 0 {
                SmcCachedVertLighting::Packed(job.packed_light)
            } else {
                SmcCachedVertLighting::FromHandle {
                    lighting_handle: job.lighting_handle?,
                    inv_height_x256: job.inv_height_x256,
                }
            };
            Some(SmcSkinSurface {
                packed,
                vert_offset: surf.xsurface_vert_offset,
                lighting,
            })
        })
        .collect();

    let mut dest = vec![0u8; lock.byte_count as usize];
    let _ = lighting_t6::skin_cached_static_model_cmd(
        &mut dest,
        &to_world,
        &normal_axis,
        &skin_surfaces,
    );

    let mut baked_any = false;
    for surf in surfaces {
        let Some(local) = local_u16_indices(
            plan.indices(),
            surf.index_start,
            surf.index_count,
            surf.vert_base,
        ) else {
            continue;
        };
        let tri_count = (surf.index_count / 3) as u16;
        // Triangles are baked in pairs, six indices a pair.
        let written = usize::from(tri_count >> 1) * 6;
        let Ok(first) = cache.bake_indices(
            job.base_vert_index,
            u32::from(surf.xsurface_base_index),
            surf.xsurface_vert_offset,
            tri_count,
            &local,
        ) else {
            continue;
        };
        let start = first as usize;
        let run = cache
            .indices
            .get(start..start.saturating_add(written))
            .map(<[u16]>::to_vec);
        let key = (job.authored as u32, surf.range_idx);
        if let Some(run) = run {
            cache
                .pending_ib
                .push((first.saturating_mul(2), bytemuck::cast_slice(&run).to_vec()));
            cache.index_runs.insert(key, run);
        }
        cache.draw_ranges.insert(key, (first, written as u32));
        baked_any = true;
    }
    cache.pending_vb.push((lock, dest));
    if baked_any {
        cache.index_baked.insert(job.cache_index);
        cache.bump_content_revision();
    }
}

/// The `count` indices from `start`, rebased to the surface's first vertex.
/// None if they run off the buffer, precede `vert_base` or exceed 16 bits.
fn local_u16_indices(indices: &[u32], start: u32, count: u32, vert_base: u32) -> Option<Vec<u16>> {
    let first = start as usize;
    let run = indices.get(first..first.checked_add(count as usize)?)?;
    run.iter()
        .map(|&i| u16::try_from(i.checked_sub(vert_base)?).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Room for a model of 32 vertices (size class 3) in bank 0.
    fn miss(cache: &mut WorldStaticModelCache, slot: usize, lod: u8) -> (u16, u32, u32) {
        match cache.cache_surface(slot, lod, 0, 3, 30) {
            CacheStaticModelSurface::Miss {
                cache_index,
                base_vert_index,
                verts,
            } => (cache_index, base_vert_index, verts),
            other => panic!("expected a miss, got {other:?}"),
        }
    }

    fn pending(cache_index: u16) -> PendingSmcSkin {
        PendingSmcSkin {
            authored: 0,
            mesh: 0,
            lod: 0,
            origin: [0.0; 3],
            axis: [[0.0; 3]; 3],
            scale: 1.0,
            packed_light: [0; 4],
            lighting_handle: None,
            inv_height_x256: 0,
            cache_index,
            base_vert_index: 0,
            class_verts: 0,
        }
    }

    #[test]
    fn a_new_cache_holds_nothing() {
        let cache = WorldStaticModelCache::new(5);
        assert_eq!(cache.cache_index.len(), 5);
        assert!(cache.cache_index.iter().all(|row| *row == [0; SMC_CACHE_INDEX_LODS]));
        assert_eq!(cache.indices.len(), SMC_INDEX_U16_N);
        assert_eq!(cache.content_revision(), 0);
        assert!(cache.baked_cache_indices().is_empty());
        assert!(cache.vb_patches().is_empty() && cache.ib_patches().is_empty());
    }

    #[test]
    fn a_surface_misses_once_then_hits() {
        let mut cache = WorldStaticModelCache::new(4);
        let (index, _, verts) = miss(&mut cache, 2, 1);
        assert_ne!(index, 0);
        // Size class 3 holds 32 vertices.
        assert_eq!(verts, 32);
        assert_eq!(cache.cache_index[2][1], index);
        assert_eq!(cache.patch_surfs, 1);
        assert_eq!(cache.patch_verts, 32);
        assert_eq!(
            cache.cache_surface(2, 1, 0, 3, 30),
            CacheStaticModelSurface::Hit { cache_index: index }
        );
        // A hit asks for no patch.
        assert_eq!(cache.patch_surfs, 1);
    }

    #[test]
    fn two_models_get_two_places() {
        let mut cache = WorldStaticModelCache::new(4);
        let (a, a_base, _) = miss(&mut cache, 0, 0);
        let (b, b_base, _) = miss(&mut cache, 1, 0);
        assert_ne!(a, b);
        assert_ne!(a_base, b_base);
        // The same model at another LOD is another entry too.
        let (c, _, _) = miss(&mut cache, 0, 1);
        assert!(c != a && c != b);
    }

    #[test]
    fn what_cannot_be_cached_is_refused() {
        let mut cache = WorldStaticModelCache::new(2);
        // A slot a 16-bit model index cannot name.
        assert_eq!(
            cache.cache_surface(70_000, 0, 0, 3, 30),
            CacheStaticModelSurface::Refused
        );
        // A size class that does not exist, and too many vertices for any.
        assert_eq!(
            cache.cache_surface(0, 0, 0, 200, 5_000),
            CacheStaticModelSurface::Refused
        );
        // A bank that does not exist.
        assert_eq!(
            cache.cache_surface(0, 0, 9, 3, 30),
            CacheStaticModelSurface::Refused
        );
        assert_eq!((cache.patch_surfs, cache.patch_verts), (0, 0));
    }

    #[test]
    fn a_bad_size_class_is_chosen_again_from_the_vertex_count() {
        let mut cache = WorldStaticModelCache::new(2);
        // 100 vertices fit the class of 102.
        match cache.cache_surface(0, 0, 0, 200, 100) {
            CacheStaticModelSurface::Miss { verts, .. } => assert_eq!(verts, 102),
            other => panic!("expected a miss, got {other:?}"),
        }
    }

    #[test]
    fn a_frame_takes_no_more_than_its_patch_allowance() {
        let mut cache = WorldStaticModelCache::new(300);
        // The lighting cache's per-frame patch allowance (0x100 surfaces).
        let allowance = 0x100_usize;
        for slot in 0..allowance {
            assert!(
                matches!(
                    cache.cache_surface(slot, 0, 0, 0, 10),
                    CacheStaticModelSurface::Miss { .. }
                ),
                "slot {slot}"
            );
        }
        assert_eq!(
            cache.cache_surface(allowance, 0, 0, 0, 10),
            CacheStaticModelSurface::Refused
        );
        // The next frame starts the allowance again.
        cache.begin_frame();
        assert!(matches!(
            cache.cache_surface(allowance, 0, 0, 0, 10),
            CacheStaticModelSurface::Miss { .. }
        ));
    }

    #[test]
    fn a_new_frame_forgets_the_patches_but_keeps_the_entries() {
        let mut cache = WorldStaticModelCache::new(2);
        let (index, _, _) = miss(&mut cache, 1, 0);
        cache.pending_ib.push((0, vec![1, 2]));
        cache.pending_skin.insert(index, pending(index));
        let frame = cache.frame;
        cache.begin_frame();
        assert_eq!(cache.frame, frame.wrapping_add(1));
        assert_eq!((cache.patch_surfs, cache.patch_verts), (0, 0));
        assert!(cache.ib_patches().is_empty() && cache.vb_patches().is_empty());
        assert!(cache.pending_skin.is_empty());
        assert_eq!(
            cache.cache_surface(1, 0, 0, 3, 30),
            CacheStaticModelSurface::Hit { cache_index: index }
        );
    }

    #[test]
    fn dropping_a_slot_drops_its_entries_and_what_was_baked_for_it() {
        let mut cache = WorldStaticModelCache::new(6);
        let (a, _, _) = miss(&mut cache, 3, 0);
        let (b, _, _) = miss(&mut cache, 3, 2);
        let (other, _, _) = miss(&mut cache, 4, 0);
        cache.index_baked.extend([a, b, other]);
        cache.draw_ranges.insert((3, 0), (0, 6));
        cache.draw_ranges.insert((3, 7), (6, 6));
        cache.draw_ranges.insert((4, 0), (12, 6));
        cache.index_runs.insert((3, 0), vec![1; 6]);
        cache.index_runs.insert((4, 0), vec![2; 6]);

        cache.uncache_slot(3);
        assert_eq!(cache.cache_index[3], [0; SMC_CACHE_INDEX_LODS]);
        assert_eq!(cache.cache_index[4][0], other);
        assert_eq!(cache.baked_cache_indices(), [other]);
        assert_eq!(cache.draw_ranges.keys().copied().collect::<Vec<_>>(), [(4, 0)]);
        assert_eq!(cache.index_runs.keys().copied().collect::<Vec<_>>(), [(4, 0)]);
        assert_eq!(cache.content_revision(), 1);
        // It is a miss again, and the other model's entry is still a hit.
        assert!(matches!(
            cache.cache_surface(3, 0, 0, 3, 30),
            CacheStaticModelSurface::Miss { .. }
        ));
        assert_eq!(
            cache.cache_surface(4, 0, 0, 3, 30),
            CacheStaticModelSurface::Hit { cache_index: other }
        );
    }

    #[test]
    fn dropping_a_slot_with_nothing_baked_leaves_the_revision_alone() {
        let mut cache = WorldStaticModelCache::new(3);
        miss(&mut cache, 1, 0);
        cache.uncache_slot(1);
        assert_eq!(cache.content_revision(), 0);
        // And a slot that does not exist is not an error.
        cache.uncache_slot(99);
        assert_eq!(cache.content_revision(), 0);
    }

    #[test]
    fn clearing_the_cache_empties_everything_derived_from_it() {
        let mut cache = WorldStaticModelCache::new(3);
        let (index, _, _) = miss(&mut cache, 0, 0);
        miss(&mut cache, 2, 3);
        cache.index_baked.insert(index);
        cache.draw_ranges.insert((0, 0), (0, 6));
        cache.index_runs.insert((0, 0), vec![0; 6]);
        cache.pending_ib.push((0, vec![0; 2]));
        cache.pending_skin.insert(index, pending(index));

        cache.clear_all();
        assert!(cache.cache_index.iter().all(|row| *row == [0; SMC_CACHE_INDEX_LODS]));
        assert!(cache.baked_cache_indices().is_empty());
        assert!(cache.draw_ranges.is_empty() && cache.index_runs.is_empty());
        assert!(cache.ib_patches().is_empty() && cache.pending_skin.is_empty());
        assert_eq!(cache.content_revision(), 1);
        assert!(matches!(
            cache.cache_surface(0, 0, 0, 3, 30),
            CacheStaticModelSurface::Miss { .. }
        ));
        // Nothing was left to drop the second time.
        cache.clear_all();
        assert_eq!(cache.content_revision(), 1);
    }

    #[test]
    fn baked_entries_are_listed_in_ascending_order() {
        let mut cache = WorldStaticModelCache::new(1);
        cache.index_baked.extend([9, 3, 5]);
        assert_eq!(cache.baked_cache_indices(), [3, 5, 9]);
    }

    #[test]
    fn baked_indices_are_rebased_to_the_cache_vertex() {
        let mut cache = WorldStaticModelCache::new(1);
        let src: Vec<u16> = (0..12).collect();
        // Two pairs of triangles: 12 indices at 10 * 4 + 2 * 3.
        let at = cache.bake_indices(10, 2, 5, 4, &src).expect("baked");
        assert_eq!(at, 46);
        let want: Vec<u16> = src.iter().map(|i| i + 15).collect();
        assert_eq!(&cache.indices[46..58], want.as_slice());
        assert_eq!(cache.indices[45], 0);
        assert_eq!(cache.indices[58], 0);
    }

    #[test]
    fn an_odd_triangle_is_not_baked() {
        let mut cache = WorldStaticModelCache::new(1);
        let src = [1u16; 9];
        cache.bake_indices(0, 0, 0, 3, &src).expect("baked");
        assert_eq!(&cache.indices[..6], &[1; 6]);
        assert_eq!(cache.indices[6], 0);
    }

    #[test]
    fn baked_indices_wrap_at_16_bits() {
        let mut cache = WorldStaticModelCache::new(1);
        let mut src = [0u16; 6];
        src[0] = 0xffff;
        cache.bake_indices(0, 0, 2, 2, &src).expect("baked");
        assert_eq!(cache.indices[0], 1);
        assert_eq!(cache.indices[1], 2);
    }

    #[test]
    fn baking_says_why_it_cannot() {
        let mut cache = WorldStaticModelCache::new(1);
        assert_eq!(
            cache.bake_indices(0, 0, 0, 4, &[0; 11]),
            Err(SmcIndexBakeError::ShortSource)
        );
        let vb_verts = (SMC_INDEX_U16_N / 4) as u32;
        assert_eq!(
            cache.bake_indices(vb_verts, 0, 0, 4, &[0; 12]),
            Err(SmcIndexBakeError::ShortDest)
        );
        assert_eq!(
            cache.bake_indices(u32::MAX, 0, 0, 4, &[0; 12]),
            Err(SmcIndexBakeError::SlotOverflow)
        );
    }

    #[test]
    fn indices_are_cut_from_the_plan_and_rebased() {
        let indices = [100u32, 101, 102, 103, 104];
        assert_eq!(
            local_u16_indices(&indices, 1, 3, 100),
            Some(vec![1, 2, 3])
        );
        assert_eq!(local_u16_indices(&indices, 5, 0, 100), Some(Vec::new()));
    }

    #[test]
    fn indices_that_do_not_fit_are_refused() {
        let indices = [100u32, 101, 102, 103, 104];
        // Off the end of the buffer.
        assert_eq!(local_u16_indices(&indices, 3, 3, 0), None);
        assert_eq!(local_u16_indices(&indices, 9, 1, 0), None);
        // Below the surface's first vertex.
        assert_eq!(local_u16_indices(&indices, 0, 2, 101), None);
        // More than 16 bits once rebased.
        assert_eq!(local_u16_indices(&[70_000], 0, 1, 0), None);
        // 65 535 is the last one that does.
        assert_eq!(local_u16_indices(&[70_000], 0, 1, 4_464), None);
        assert_eq!(local_u16_indices(&[70_000], 0, 1, 4_465), Some(vec![65_535]));
    }

    #[test]
    fn the_ramp_dvar_hands_its_values_on() {
        let ramp = LodRampDvar {
            scale_mid: Some(1.5),
            bias_mid: Some(0.25),
            scale_last: Some(2.0),
            world_unit: Some(3.0),
            ..LodRampDvar::default()
        };
        let args = ramp.args();
        assert_eq!(args.scale_mid, Some(1.5));
        assert_eq!(args.bias_mid, Some(0.25));
        assert_eq!(args.scale_last, Some(2.0));
        assert_eq!(args.world_unit, Some(3.0));
        assert!(!args.t5_no_lod_cull_out);
        let unset = LodRampDvar::default().args();
        assert_eq!(
            (unset.scale_mid, unset.bias_mid, unset.scale_last, unset.world_unit),
            (None, None, None, None)
        );
    }

    #[test]
    fn the_cache_is_on_unless_a_dvar_says_otherwise() {
        assert_eq!(SmcEnableDvar::default().enabled, None);
        assert!(PretessDvar::default().enabled);
    }
}
