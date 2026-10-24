//! Files static models into the game's surface buckets and drains the buckets into draw items.
//!
//! The static lane files the models it chose, and the sun-shadow bake files every model that
//! casts a shadow, so both go through one `Filer`. A model goes into the bucket its detail
//! level, lighting and cache state pick. A bucket drains when it fills and once more at the
//! end. Rigid and skinned models become one item per surface. Cached models drain in batches
//! that share an index run: the batch is either re-tessellated into the shared index list, or
//! each model points at its own copy of the run.

use bevy::prelude::Mat4;
use lighting_t6::{SMODEL_BUCKET_LIST_N, SmodelSurfBucketLists, SmodelSurfPath};

use super::{RetainedDrawItem, RetainedDrawKind, SmodelPretessRange, catalog_item};
use crate::assemble::drawsurf::tess::smodel::{SmodelGpuPlan, SmodelMeshSurfaces, SmodelPlacement};
use crate::assemble::drawsurf::{
    RuntimeMaterialCatalog, SurfaceReflectionProbeId, SurfaceSamplerInputs,
};
use crate::prepare::scene::smodel_geom_cache::WorldStaticModelCache;

/// The pass a filed model is drawn in. Colour takes the lit or unlit material the model's
/// lighting asks for. The sun pass takes whichever material exists and leaves out surfaces
/// whose material says they cast no sun shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    Colour,
    SunShadow,
}

/// A model waiting in a bucket until the bucket drains.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Queued {
    pub(crate) placement: u32,
    pub(crate) lod: u8,
    pub(crate) world_from_local: Mat4,
    pub(crate) lighting_handle: u32,
    pub(crate) packed_lighting: Option<[u8; 4]>,
    pub(crate) pass: Pass,
    /// Set when the model is filed: what the bucket stored for this entry.
    payload: u16,
}

impl Queued {
    pub(crate) fn new(
        placement: u32,
        lod: u8,
        world_from_local: Mat4,
        lighting_handle: u32,
        packed_lighting: Option<[u8; 4]>,
        pass: Pass,
    ) -> Self {
        Self {
            placement,
            lod,
            world_from_local,
            lighting_handle,
            packed_lighting,
            pass,
            payload: 0,
        }
    }
}

/// What picks a model's bucket.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BucketKey {
    pub(crate) lod: i32,
    pub(crate) smc_enabled: bool,
    pub(crate) lit: bool,
    pub(crate) smc_flag: u8,
    pub(crate) cache_index: u16,
    pub(crate) model_index: u16,
    /// `None` when the detail level does not say whether it is rigid or skinned.
    pub(crate) rigid: Option<bool>,
}

/// Where `Filer::file` put a model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Filed {
    /// The bucket, or `None` when the model could not be filed.
    pub(crate) bucket: Option<i32>,
    /// The bucket is now full and should drain.
    pub(crate) full: bool,
}

/// What one drain did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Drained {
    pub(crate) buckets: u32,
    /// Queue entries that did not match what the bucket list stored.
    pub(crate) refused: u32,
}

/// How the surfaces of one model came out.
#[derive(Clone, Copy, Debug, Default)]
struct Surfaces {
    drawn: u32,
    opted_out: u32,
}

impl Surfaces {
    fn add(&mut self, other: Surfaces) {
        self.drawn = self.drawn.saturating_add(other.drawn);
        self.opted_out = self.opted_out.saturating_add(other.opted_out);
    }
}

pub(crate) struct Filer<'a> {
    plan: &'a SmodelGpuPlan,
    catalog: &'a RuntimeMaterialCatalog,
    cache: Option<&'a WorldStaticModelCache>,
    rows: [Vec<Queued>; SMODEL_BUCKET_LIST_N],
    indices: IndexList,
    items: Vec<RetainedDrawItem>,
    opted_out: Vec<u32>,
}

/// What a finished `Filer` leaves behind.
pub(crate) struct Filings {
    pub(crate) items: Vec<RetainedDrawItem>,
    /// The index list pre-tessellated and cached draws read.
    pub(crate) indices: Vec<u16>,
    /// Placements that drew nothing because every surface opted out of the pass, sorted.
    pub(crate) opted_out: Vec<u32>,
}

impl<'a> Filer<'a> {
    pub(crate) fn new(
        plan: &'a SmodelGpuPlan,
        catalog: &'a RuntimeMaterialCatalog,
        cache: Option<&'a WorldStaticModelCache>,
        pretess_on: bool,
    ) -> Self {
        Self {
            plan,
            catalog,
            cache,
            rows: std::array::from_fn(|_| Vec::new()),
            indices: IndexList::new(pretess_on),
            items: Vec::new(),
            opted_out: Vec::new(),
        }
    }

    /// Files `model` into the bucket `key` picks. A model that cannot be filed is not queued.
    pub(crate) fn file(
        &mut self,
        lists: &mut SmodelSurfBucketLists,
        key: BucketKey,
        mut model: Queued,
    ) -> Filed {
        let none = Filed::default();
        let Some(rigid) = key.rigid else {
            return none;
        };
        let bucket = lighting_t6::add_static_model_surf_to_bucket(
            key.lod,
            key.smc_enabled,
            key.lit,
            key.smc_flag,
            key.cache_index,
            rigid,
        );
        let payload =
            lighting_t6::smodel_bucket_store_payload(bucket, key.model_index, key.cache_index);
        let Some(push) = lighting_t6::smodel_surf_bucket_push(lists, bucket, payload) else {
            return none;
        };
        let Some(row) = usize::try_from(bucket)
            .ok()
            .and_then(|bucket| self.rows.get_mut(bucket))
        else {
            return none;
        };
        model.payload = payload;
        row.push(model);
        Filed {
            bucket: Some(bucket),
            full: push == lighting_t6::SmodelBucketPush::Full,
        }
    }

    /// Records a model that could not be filed: if any of its surfaces opt out of the pass, it
    /// counts as opted out, as it would had it been expanded and drawn nothing.
    pub(crate) fn note_unfiled(&mut self, model: Queued) {
        let plan = self.plan;
        let Some(surfaces) = plan
            .placements
            .get(model.placement as usize)
            .and_then(|placement| plan.meshes.get(placement.mesh))
            .and_then(|mesh| mesh.surfaces_by_lod.get(usize::from(model.lod)))
        else {
            return;
        };
        if surfaces
            .iter()
            .any(|&(_, authored)| !casts_in_pass(model.pass, self.catalog, authored))
        {
            self.opted_out.push(model.placement);
        }
    }

    /// Drains the buckets in `mask`.
    pub(crate) fn drain(&mut self, lists: &mut SmodelSurfBucketLists, mask: u32) -> Drained {
        let mut taken = [None; SMODEL_BUCKET_LIST_N];
        let n = lighting_t6::smodel_bucket_lists_consume(lists, mask, &mut taken);
        let mut drained = Drained {
            buckets: n as u32,
            refused: 0,
        };
        for bucket in taken.into_iter().take(n).flatten() {
            let queued = std::mem::take(&mut self.rows[bucket.bucket as usize]);
            let listed = usize::from(bucket.count);
            if queued.len() != listed {
                drained.refused = drained
                    .refused
                    .saturating_add(u32::try_from(queued.len().abs_diff(listed)).unwrap_or(u32::MAX));
            }
            let usable = listed.min(queued.len());
            let mut at = 0;
            while at < usable {
                let model = queued[at];
                if model.payload != bucket.ids[at] || model.lod != bucket.lod {
                    drained.refused = drained.refused.saturating_add(1);
                    at += 1;
                    continue;
                }
                if bucket.source != SmodelSurfPath::Cached {
                    self.expand(model, bucket.source);
                    at += 1;
                    continue;
                }
                let batch = self.cached_batch(&queued[at..usable], &bucket.ids[at..usable], bucket.lod);
                self.expand_cached(&queued[at..at + batch]);
                at += batch;
            }
        }
        drained
    }

    /// Drains the bucket a model was just filed into, when that filled it.
    pub(crate) fn drain_filled(
        &mut self,
        lists: &mut SmodelSurfBucketLists,
        filed: Filed,
    ) -> Drained {
        let mask = filed
            .bucket
            .filter(|_| filed.full)
            .and_then(|bucket| u8::try_from(bucket).ok())
            .and_then(lighting_t6::smodel_bucket_mask);
        match mask {
            Some(mask) => self.drain(lists, mask),
            None => Drained::default(),
        }
    }

    /// Drains every bucket that still holds models.
    pub(crate) fn drain_rest(&mut self, lists: &mut SmodelSurfBucketLists) -> Drained {
        let mut mask = 0;
        for (bucket, &count) in lists.count.iter().enumerate() {
            if count > 0
                && let Some(bit) = lighting_t6::smodel_bucket_mask(bucket as u8)
            {
                mask |= bit;
            }
        }
        self.drain(lists, mask)
    }

    pub(crate) fn finish(mut self) -> Filings {
        self.opted_out.sort_unstable();
        self.opted_out.dedup();
        Filings {
            items: self.items,
            indices: self.indices.list,
            opted_out: self.opted_out,
        }
    }

    /// How many models from the front of `queued` can be drained as one cached batch: they
    /// share a bank, and have the same mesh, lighting and light slots.
    fn cached_batch(&self, queued: &[Queued], ids: &[u16], lod: u8) -> usize {
        let state = |model: &Queued| {
            self.plan
                .placements
                .get(model.placement as usize)
                .map(batch_state)
        };
        let same_bank = lighting_t6::smodel_same_bank_count(ids).min(queued.len());
        let first = state(&queued[0]);
        let mut n = 1;
        while n < same_bank
            && queued[n].payload == ids[n]
            && queued[n].lod == lod
            && state(&queued[n]) == first
        {
            n += 1;
        }
        n
    }

    /// Makes the items of one rigid, skinned or cached-alone model.
    fn expand(&mut self, model: Queued, path: SmodelSurfPath) {
        let plan = self.plan;
        let Some(surfaces) = plan
            .placements
            .get(model.placement as usize)
            .and_then(|placement| plan.meshes.get(placement.mesh))
            .and_then(|mesh| mesh.surfaces_by_lod.get(usize::from(model.lod)))
        else {
            return;
        };
        let stream = lighting_t6::smodel_dest_path(path, false);
        let mut result = Surfaces::default();
        for &(surface, authored) in surfaces {
            result.add(self.push(model, surface, authored, stream, None));
        }
        self.note(model.placement, result);
    }

    /// Makes the items of a batch of cached models that share one index run per surface.
    fn expand_cached(&mut self, batch: &[Queued]) {
        let (plan, cache) = (self.plan, self.cache);
        let Some(&first) = batch.first() else {
            return;
        };
        let Some(surfaces) = plan
            .placements
            .get(first.placement as usize)
            .and_then(|placement| plan.meshes.get(placement.mesh))
            .and_then(|mesh| mesh.surfaces_by_lod.get(usize::from(first.lod)))
            .filter(|surfaces| !surfaces.is_empty())
        else {
            return;
        };
        let run_of = |model: &Queued, surface: u32| {
            plan.placements
                .get(model.placement as usize)
                .and_then(|placement| cached_run(cache, placement, surface))
        };
        let ids: Vec<u16> = batch.iter().map(|model| model.payload).collect();
        let mut runs: Vec<&[u16]> = Vec::new();
        let mut complete = true;
        for &(surface, _) in surfaces {
            for model in batch {
                match run_of(model, surface) {
                    Some(run) => runs.push(run),
                    None => complete = false,
                }
            }
        }
        let usable: &[&[u16]] = if complete { &runs } else { &[] };
        let decision = self.indices.decide(&ids, usable);
        if decision.cmd_kind == lighting_t6::SmodelCmdKind::Unchanged && decision.consumed == 0 {
            for &model in batch {
                self.expand(model, SmodelSurfPath::Cached);
            }
            return;
        }
        if let Some(alloc) = decision.pretess
            && decision.dest == SmodelSurfPath::Pretess
            && let Some(range) = self.indices.commit(alloc, &runs)
        {
            // One item per surface for the whole batch, each over its slice of the new range.
            let mut start = range.start;
            let mut result = Surfaces::default();
            for &(surface, authored) in surfaces {
                let count = batch
                    .iter()
                    .filter_map(|model| run_of(model, surface))
                    .fold(0u32, |sum, run| sum.saturating_add(run.len() as u32));
                let slice = (count > 0).then_some(SmodelPretessRange { start, count });
                start = start.saturating_add(count);
                result.add(self.push(first, surface, authored, SmodelSurfPath::Pretess, slice));
            }
            self.note(first.placement, result);
            return;
        }
        for &model in batch {
            if plan.placements.get(model.placement as usize).is_none() {
                continue;
            }
            let mut result = Surfaces::default();
            for &(surface, authored) in surfaces {
                let own = run_of(&model, surface).and_then(|run| self.indices.append_copy(run));
                result.add(self.push(model, surface, authored, SmodelSurfPath::Cached, own));
            }
            self.note(model.placement, result);
        }
    }

    /// Records a model that drew nothing only because its surfaces opted out of the pass.
    fn note(&mut self, placement: u32, result: Surfaces) {
        if result.drawn == 0 && result.opted_out > 0 {
            self.opted_out.push(placement);
        }
    }

    /// Makes the item of one surface of `model`, or says why there is none.
    fn push(
        &mut self,
        model: Queued,
        surface: u32,
        authored: Option<assets::MaterialIndex>,
        stream: SmodelSurfPath,
        pretess: Option<SmodelPretessRange>,
    ) -> Surfaces {
        let none = Surfaces::default();
        let plan = self.plan;
        let Some(placement) = plan.placements.get(model.placement as usize) else {
            return none;
        };
        let Some(object_id) = model
            .placement
            .checked_add(1)
            .and_then(|id| u16::try_from(id).ok())
        else {
            return none;
        };
        if !casts_in_pass(model.pass, self.catalog, authored) {
            return Surfaces {
                drawn: 0,
                opted_out: 1,
            };
        }
        let probe = placement.reflection_probe_index;
        let material = match model.pass {
            Pass::Colour if placement.lit => plan.lit_material_key.get(&(authored, probe)).copied(),
            Pass::Colour => plan.unlit_material_key.get(&authored).copied(),
            Pass::SunShadow => plan
                .lit_material_key
                .get(&(authored, probe))
                .copied()
                .or_else(|| plan.unlit_material_key.get(&authored).copied()),
        };
        let Some(material) = material else {
            return none;
        };
        let Some(info) = plan.materials.get(material as usize) else {
            return none;
        };
        if model.pass == Pass::Colour && placement.lit != info.model_lighting_required {
            return none;
        }
        let Some(rank) = info.material_sorted_index else {
            return none;
        };
        let key = drawsurf_key(
            info.sort_key,
            rank,
            object_id,
            probe,
            placement.primary_light_index,
            stream,
        );
        // Cached and pre-tessellated draws read their vertices from the index list, not from
        // the model's own pose.
        let world_from_local = match stream {
            SmodelSurfPath::Cached | SmodelSurfPath::Pretess => Mat4::IDENTITY,
            SmodelSurfPath::Rigid | SmodelSurfPath::Skinned => model.world_from_local,
        };
        let cache_index = matches!(stream, SmodelSurfPath::Cached | SmodelSurfPath::Pretess)
            .then_some(model.payload);
        self.items.push(catalog_item(
            key,
            rank,
            RetainedDrawKind::Smodel {
                surface,
                material,
                world_from_local,
                lighting_handle: model.lighting_handle,
                packed_lighting: model.packed_lighting,
                placement: model.placement,
                stream: Some(stream),
                cache_index,
                pretess,
            },
            SurfaceSamplerInputs {
                reflection_probe: Some(SurfaceReflectionProbeId(probe)),
                ..Default::default()
            },
            self.catalog,
        ));
        Surfaces {
            drawn: 1,
            opted_out: 0,
        }
    }
}

/// What models in one cached batch must share.
fn batch_state(placement: &SmodelPlacement) -> (usize, bool, u8, u8) {
    (
        placement.mesh,
        placement.lit,
        placement.reflection_probe_index,
        placement.primary_light_index,
    )
}

/// The cached index run of one surface of `placement`, when it has a usable one.
fn cached_run<'a>(
    cache: Option<&'a WorldStaticModelCache>,
    placement: &SmodelPlacement,
    surface: u32,
) -> Option<&'a [u16]> {
    let slot = u32::try_from(placement.lighting_slot?).ok()?;
    let run = cache?.index_runs.get(&(slot, surface))?;
    (!run.is_empty() && run.len().is_multiple_of(3)).then_some(run.as_slice())
}

/// Whether a surface draws in `pass`: only the sun pass lets a material opt out.
fn casts_in_pass(
    pass: Pass,
    catalog: &RuntimeMaterialCatalog,
    authored: Option<assets::MaterialIndex>,
) -> bool {
    if pass != Pass::SunShadow {
        return true;
    }
    authored
        .and_then(|id| catalog.derived(id))
        .is_none_or(|material| lighting_t6::smodel_surf_sun_shadow_emits(material.info_game_flags))
}

fn drawsurf_key(
    sort_key: u8,
    material_rank: u32,
    object_id: u16,
    reflection_probe_index: u8,
    scene_light_index: u8,
    stream: SmodelSurfPath,
) -> u64 {
    dpvs_t6::pack(dpvs_t6::GfxDrawSurfFields {
        object_id,
        reflection_probe_index,
        primary_light_index: scene_light_index,
        surf_type: lighting_t6::smodel_surf_type(stream),
        material_sorted_index: render_material::sort_band(material_rank),
        primary_sort_key: dpvs_t6::material_sort_key_row(sort_key),
        ..Default::default()
    })
    .packed
}

/// The index list pre-tessellated and cached draws read, and the budget it is built against.
struct IndexList {
    enabled: bool,
    commands_used: usize,
    list: Vec<u16>,
}

impl IndexList {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            commands_used: 0,
            list: Vec::new(),
        }
    }

    /// What the game's cached-draw planner would do with a batch of `ids` and their runs.
    fn decide(&self, ids: &[u16], runs: &[&[u16]]) -> lighting_t6::SmodelCachedCmdPlan {
        lighting_t6::smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            ids,
            self.enabled,
            false,
            self.commands_used,
            usize::MAX,
            u32::try_from(self.list.len()).unwrap_or(u32::MAX),
            render_frame::DYNAMIC_INDEX_BUFFER_CAPACITY,
            runs,
        )
    }

    /// Appends the tessellated runs the planner allocated; `None` leaves the list as it was.
    fn commit(
        &mut self,
        alloc: lighting_t6::SmodelPretessAlloc,
        runs: &[&[u16]],
    ) -> Option<SmodelPretessRange> {
        let start = usize::try_from(alloc.first_index).ok()?;
        let count = usize::try_from(alloc.index_count).ok()?;
        if start != self.list.len() {
            return None;
        }
        self.list.resize(start.saturating_add(count), 0);
        if !lighting_t6::smodel_pretess_indices_copy(&mut self.list, alloc, runs) {
            self.list.truncate(start);
            return None;
        }
        self.commands_used = self.commands_used.saturating_add(alloc.cmd.len());
        Some(SmodelPretessRange {
            start: alloc.first_index,
            count: alloc.index_count,
        })
    }

    /// Appends one model's own copy of a cached run, when it is whole triangles and fits.
    fn append_copy(&mut self, run: &[u16]) -> Option<SmodelPretessRange> {
        if run.is_empty() || !run.len().is_multiple_of(3) {
            return None;
        }
        let start = u32::try_from(self.list.len()).ok()?;
        let count = u32::try_from(run.len()).ok()?;
        if start.checked_add(count)? > render_frame::DYNAMIC_INDEX_BUFFER_CAPACITY {
            return None;
        }
        self.list.extend_from_slice(run);
        Some(SmodelPretessRange { start, count })
    }
}

/// The detail level's flag byte from the model cache table, or 0 when it has none.
pub(crate) fn lod_smc_flag(mesh: &SmodelMeshSurfaces, lod: usize) -> u8 {
    mesh.lod_smc_rows
        .and_then(|rows| rows.get(lod).copied())
        .or_else(|| (lod == 0).then_some(mesh.lod_smc).flatten())
        .map(|row| row[1])
        .unwrap_or(0)
}

/// Whether a detail level is rigid, or `None` when any of its surfaces lacks the byte that says.
pub(crate) fn lod_is_rigid(mesh: &SmodelMeshSurfaces, lod: usize) -> Option<bool> {
    let bytes = mesh.xsurface_plus_1_by_lod.get(lod)?;
    let raw: Option<Vec<u8>> = bytes.iter().copied().collect();
    Some(lighting_t6::smodel_lod_is_rigid(&raw?))
}

pub(crate) fn lighting_nonzero(packed: Option<[u8; 4]>) -> bool {
    packed.is_some_and(|bytes| u32::from_le_bytes(bytes) != 0)
}

/// The cache table's index for a lit model at a detail level, or 0 when it has none.
pub(crate) fn cache_index(
    cache: Option<&WorldStaticModelCache>,
    lighting_slot: Option<usize>,
    lod: usize,
) -> u16 {
    lighting_slot
        .and_then(|slot| cache.and_then(|c| c.cache_index.get(slot)))
        .and_then(|row| row.get(lod).copied())
        .unwrap_or(0)
}
