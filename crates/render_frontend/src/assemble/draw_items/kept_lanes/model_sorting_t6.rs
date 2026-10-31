//! Files static models into the game's surface buckets and drains the buckets into draw items.
//!
//! The static lane files the models it chose, and the sun-shadow bake files every model that
//! casts a shadow, so both go through one `Filer`. A model goes into the bucket its detail
//! level, lighting and cache state pick. A bucket drains when it fills and once more at the
//! end. Rigid and skinned models become one item per surface. Cached models drain in batches
//! that share an index run: the batch is either re-tessellated into the shared index list, or
//! each model points at its own copy of the run.

use bevy::prelude::Mat4;
use lighting_t6::{PROP_BUCKET_LIST_COUNT, PropFacetBucketLists, PropFacetPath};

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind, PropPrebuiltRange, catalog_item};
use crate::mesh_build::prop_mesh::{PropGpuPlan, PropMeshFacets, PropPlacement};
use crate::{
    LiveMaterialLedger, SurfaceCubeProbeId, FacetSamplerInputs,
};
use crate::prepare::scene::prop_geom_memo::LevelPropCache;

/// One surface of a detail level: its range and the material the asset gave it.

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
pub(crate) struct Waiting {
    pub(crate) placement: u32,
    pub(crate) lod: u8,
    pub(crate) world_from_local: Mat4,
    pub(crate) illum_handle: u32,
    pub(crate) compact_illum: Option<[u8; 4]>,
    pub(crate) pass: Pass,
    /// The id the bucket list holds for this entry, written when the model is filed.
    stored_id: u16,
}

impl Waiting {
    pub(crate) fn new(
        pose_at: u32,
        lod: u8,
        local_to_world: Mat4,
        illum_handle: u32,
        compact_illum: Option<[u8; 4]>,
        pass: Pass,
    ) -> Self {
        Waiting {
            stored_id: 0,
            placement: pose_at,
            lod,
            world_from_local: local_to_world,
            illum_handle,
            compact_illum,
            pass,
        }
    }

    /// Whether this entry is the one the bucket list recorded at the same position.
    fn same_slot(&self, id: u16, lod: u8) -> bool {
        self.stored_id == id && self.lod == lod
    }
}

/// What picks a model's bucket.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BucketKey {
    pub(crate) lod: i32,
    pub(crate) smc_enabled: bool,
    pub(crate) lit: bool,
    pub(crate) smc_flag: u8,
    pub(crate) cache_slot_no: u16,
    pub(crate) mesh_slot_number: u16,
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

/// Tally of a model's surfaces: how many made an item, and how many sat the pass out.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    made: u32,
    sat_out: u32,
}

impl Tally {
    const MADE: Tally = Tally {
        made: 1,
        sat_out: 0,
    };
    const SAT_OUT: Tally = Tally {
        made: 0,
        sat_out: 1,
    };

    fn fold_in(self, other: Tally) -> Tally {
        Tally {
            made: self.made.saturating_add(other.made),
            sat_out: self.sat_out.saturating_add(other.sat_out),
        }
    }

    /// True when nothing was drawn and the only reason was surfaces sitting the pass out.
    fn only_sat_out(self) -> bool {
        self.made == 0 && self.sat_out > 0
    }
}

pub(crate) struct Filer<'a> {
    gpu: &'a PropGpuPlan,
    materials: &'a LiveMaterialLedger,
    smc: Option<&'a LevelPropCache>,
    /// Per bucket, the models filed there in the order the bucket list took them.
    waiting: [Vec<Waiting>; PROP_BUCKET_LIST_COUNT],
    shared: SharedIndices,
    out: Vec<HeldDrawItem>,
    skipped: Vec<u32>,
}

/// What a finished `Filer` leaves behind.
pub(crate) struct Filings {
    pub(crate) items: Vec<HeldDrawItem>,
    /// The index list pre-tessellated and cached draws read.
    pub(crate) indices: Vec<u16>,
    /// Placements that drew nothing because every surface opted out of the pass, sorted.
    pub(crate) opted_out: Vec<u32>,
}

impl<'a> Filer<'a> {
    pub(crate) fn new(
        gpu: &'a PropGpuPlan,
        materials: &'a LiveMaterialLedger,
        smc: Option<&'a LevelPropCache>,
        pretess_on: bool,
    ) -> Self {
        Filer {
            gpu,
            materials,
            smc,
            waiting: Default::default(),
            shared: SharedIndices::starting(pretess_on),
            out: Vec::new(),
            skipped: Vec::new(),
        }
    }

    /// Files `model` into the bucket `key` picks. A model that cannot be filed is not queued.
    pub(crate) fn file(
        &mut self,
        store: &mut PropFacetBucketLists,
        key: BucketKey,
        model: Waiting,
    ) -> Filed {
        self.try_file(store, key, model).unwrap_or_default()
    }

    fn try_file(
        &mut self,
        store: &mut PropFacetBucketLists,
        key: BucketKey,
        mut model: Waiting,
    ) -> Option<Filed> {
        let rigid = key.rigid?;
        let which = lighting_t6::add_static_model_surf_to_bucket(
            key.lod,
            key.smc_enabled,
            key.lit,
            key.smc_flag,
            key.cache_slot_no,
            rigid,
        );
        model.stored_id = lighting_t6::smodel_bucket_store_payload(which, key.mesh_slot_number, key.cache_slot_no);
        let outcome = lighting_t6::push_prop_facet(store, which, model.stored_id)?;
        let queue = self.waiting.get_mut(usize::try_from(which).ok()?)?;
        queue.push(model);
        Some(Filed {
            bucket: Some(which),
            full: outcome == lighting_t6::StaticModelBucketInsertResult::Full,
        })
    }

    /// Records a model that could not be filed: if any of its surfaces opt out of the pass, it
    /// counts as opted out, as it would had it been expanded and drawn nothing.
    pub(crate) fn note_unfiled(&mut self, model: Waiting) {
        let materials = self.materials;
        let sits_out = self
            .surfaces_of(model.placement, model.lod)
            .is_some_and(|list| list.iter().any(|s| !draws_in(model.pass, materials, s.1)));
        if sits_out {
            self.skipped.push(model.placement);
        }
    }

    /// Drains the buckets in `mask`.
    pub(crate) fn drain(&mut self, store: &mut PropFacetBucketLists, mask: u32) -> Drained {
        let mut handed = [None; PROP_BUCKET_LIST_COUNT];
        let count = lighting_t6::static_bucket_lists_drain(store, mask, &mut handed);
        let mut report = Drained::default();
        report.buckets = count as u32;
        for taken in handed.iter().take(count).flatten() {
            let queue = std::mem::take(&mut self.waiting[taken.bin as usize]);
            let recorded = usize::from(taken.count);
            let mismatch = queue.len().abs_diff(recorded);
            if mismatch > 0 {
                let extra = u32::try_from(mismatch).unwrap_or(u32::MAX);
                report.refused = report.refused.saturating_add(extra);
            }
            let end = queue.len().min(recorded);
            let ids = &taken.members[..end];
            let mut next = 0;
            while next < end {
                let model = queue[next];
                let step = if !model.same_slot(ids[next], taken.lod) {
                    report.refused = report.refused.saturating_add(1);
                    1
                } else if taken.source == PropFacetPath::Cached {
                    let len = self.batch_len(&queue[next..end], &ids[next..], taken.lod);
                    self.emit_cached_batch(&queue[next..next + len]);
                    len
                } else {
                    self.emit_single(model, taken.source);
                    1
                };
                next += step;
            }
        }
        report
    }

    /// Drains the bucket a model was just filed into, when that filled it.
    pub(crate) fn drain_filled(
        &mut self,
        store: &mut PropFacetBucketLists,
        filed: Filed,
    ) -> Drained {
        if !filed.full {
            return Drained::default();
        }
        let bit = filed
            .bucket
            .and_then(|which| u8::try_from(which).ok())
            .and_then(lighting_t6::smodel_bucket_mask);
        bit.map_or_else(Drained::default, |bit| self.drain(store, bit))
    }

    /// Drains every bucket that still holds models.
    pub(crate) fn drain_rest(&mut self, store: &mut PropFacetBucketLists) -> Drained {
        let occupied = store
            .filled
            .iter()
            .enumerate()
            .filter(|&(_, &held)| held > 0)
            .filter_map(|(which, _)| lighting_t6::smodel_bucket_mask(which as u8))
            .fold(0, |all, bit| all | bit);
        self.drain(store, occupied)
    }

    pub(crate) fn finish(self) -> Filings {
        let mut skipped = self.skipped;
        skipped.sort_unstable();
        skipped.dedup();
        Filings {
            items: self.out,
            indices: self.shared.data,
            opted_out: skipped,
        }
    }

    fn placement_of(&self, id: u32) -> Option<&'a PropPlacement> {
        self.gpu.placements.get(id as usize)
    }

    /// The surfaces a placement draws at one detail level.
    fn surfaces_of(&self, id: u32, lod: u8) -> Option<&'a [(u32, Option<assets::SurfaceMaterialId>)]> {
        let gpu = self.gpu;
        let mesh = gpu.meshes.get(self.placement_of(id)?.model_slot)?;
        Some(mesh.facets_by_detail.get(usize::from(lod))?.as_slice())
    }

    /// The cached run one surface of a queued model reads from.
    fn run_for(&self, model: &Waiting, surface: u32) -> Option<&'a [u16]> {
        cached_triangles(self.smc, self.placement_of(model.placement)?, surface)
    }

    /// How many models from the front of `queue` can be drained as one cached batch: they
    /// share a bank, and have the same mesh, lighting and light slots.
    fn batch_len(&self, queue: &[Waiting], id_list: &[u16], lod: u8) -> usize {
        let shape = |model: &Waiting| self.placement_of(model.placement).map(batch_shape);
        let limit = lighting_t6::same_bank_run(id_list).min(queue.len());
        let lead = shape(&queue[0]);
        let joins = |k: usize| queue[k].same_slot(id_list[k], lod) && shape(&queue[k]) == lead;
        1 + (1..limit).take_while(|&k| joins(k)).count()
    }

    /// Makes the items of one rigid, skinned or cached-alone model.
    fn emit_single(&mut self, model: Waiting, path: PropFacetPath) {
        let Some(list) = self.surfaces_of(model.placement, model.lod) else {
            return;
        };
        let target = lighting_t6::smodel_dest_path(path, false);
        let tally = list
            .iter()
            .fold(Tally::default(), |sum, &(surface, given)| {
                sum.fold_in(self.make_item(model, surface, given, target, None))
            });
        self.record(model.placement, tally);
    }

    /// Makes the items of a batch of cached models that share one index run per surface.
    fn emit_cached_batch(&mut self, batch: &[Waiting]) {
        let Some(&lead) = batch.first() else {
            return;
        };
        let list = match self.surfaces_of(lead.placement, lead.lod) {
            Some(list) if !list.is_empty() => list,
            _ => return,
        };
        let ids: Vec<u16> = batch.iter().map(|model| model.stored_id).collect();
        // Surface-major: every model's run for the first surface, then the second, and so on.
        let gathered: Vec<Option<&[u16]>> = list
            .iter()
            .flat_map(|&(facet, _)| batch.iter().map(move |model| (model, facet)))
            .map(|(placed, facet)| self.run_for(placed, facet))
            .collect();
        let found: Vec<&[u16]> = gathered.iter().copied().flatten().collect();
        let every_run_found = found.len() == gathered.len();
        let plan = self
            .shared
            .plan(&ids, if every_run_found { &found } else { &[] });

        if plan.cmd_kind == lighting_t6::StaticModelCmdVariety::Unchanged && plan.models_taken == 0 {
            for &model in batch {
                self.emit_single(model, PropFacetPath::Cached);
            }
            return;
        }
        let tessellated = match plan.grant {
            Some(alloc) if plan.write_target == PropFacetPath::Pretess => {
                self.shared.write_tessellated(alloc, &found)
            }
            _ => None,
        };
        match tessellated {
            Some(range) => self.emit_tessellated(batch, list, range.start),
            None => self.emit_own_copies(batch, list),
        }
    }

    /// One item per surface for the whole batch, each over its slice of the new range.
    fn emit_tessellated(&mut self, batch: &[Waiting], list: &[(u32, Option<assets::SurfaceMaterialId>)], from: u32) {
        let lead = batch[0];
        let mut cursor = from;
        let mut tally = Tally::default();
        for &(facet, given) in list {
            let len = batch
                .iter()
                .filter_map(|model| self.run_for(model, facet))
                .map(|run| run.len() as u32)
                .fold(0u32, u32::saturating_add);
            let slice = if len == 0 {
                None
            } else {
                Some(PropPrebuiltRange {
                    start: cursor,
                    count: len,
                })
            };
            cursor = cursor.saturating_add(len);
            let made = self.make_item(lead, facet, given, PropFacetPath::Pretess, slice);
            tally = tally.fold_in(made);
        }
        self.record(lead.placement, tally);
    }

    /// Each model of the batch points at its own copy of its cached runs.
    fn emit_own_copies(&mut self, batch: &[Waiting], list: &[(u32, Option<assets::SurfaceMaterialId>)]) {
        for &model in batch {
            if self.placement_of(model.placement).is_none() {
                continue;
            }
            let mut tally = Tally::default();
            for &(facet, given) in list {
                let copy = match self.run_for(&model, facet) {
                    Some(run) => self.shared.copy_in(run),
                    None => None,
                };
                let made = self.make_item(model, facet, given, PropFacetPath::Cached, copy);
                tally = tally.fold_in(made);
            }
            self.record(model.placement, tally);
        }
    }

    /// Remembers a placement that drew nothing only because its surfaces sat the pass out.
    fn record(&mut self, pose_at: u32, tally: Tally) {
        if tally.only_sat_out() {
            self.skipped.push(pose_at);
        }
    }

    /// The material a surface of `spot` is drawn with in `pass`.
    fn material_for(
        &self,
        pass: Pass,
        spot: &PropPlacement,
        given: Option<assets::SurfaceMaterialId>,
    ) -> Option<u32> {
        let gpu = self.gpu;
        let lit_one = || {
            gpu.shaded_material_key
                .get(&(given, spot.env_probe_index))
        };
        let unlit_one = || gpu.flat_material_key.get(&given);
        let picked = match (pass, spot.lit_list) {
            (Pass::SunShadow, _) => lit_one().or_else(unlit_one),
            (Pass::Colour, true) => lit_one(),
            (Pass::Colour, false) => unlit_one(),
        };
        picked.copied()
    }

    /// Makes the item of one surface of `model`, or tallies why there is none.
    fn make_item(
        &mut self,
        model: Waiting,
        surface: u32,
        given: Option<assets::SurfaceMaterialId>,
        target: PropFacetPath,
        range: Option<PropPrebuiltRange>,
    ) -> Tally {
        match self.build_item(model, surface, given, target, range) {
            Ok(item) => {
                self.out.push(item);
                Tally::MADE
            }
            Err(tally) => tally,
        }
    }

    fn build_item(
        &self,
        model: Waiting,
        surface: u32,
        given: Option<assets::SurfaceMaterialId>,
        target: PropFacetPath,
        range: Option<PropPrebuiltRange>,
    ) -> Result<HeldDrawItem, Tally> {
        let nothing = Tally::default();
        let spot = self.placement_of(model.placement).ok_or(nothing)?;
        // Draw-surface object ids count from one, and must fit the key's 16 bits.
        let object = u16::try_from(u64::from(model.placement) + 1).map_err(|_| nothing)?;
        let mats = self.materials;
        if !draws_in(model.pass, mats, given) {
            return Err(Tally::SAT_OUT);
        }
        let material = self.material_for(model.pass, spot, given).ok_or(nothing)?;
        let entry = self.gpu.materials.get(material as usize).ok_or(nothing)?;
        let wrong_lighting = spot.lit_list != entry.model_illum_required;
        if model.pass == Pass::Colour && wrong_lighting {
            return Err(nothing);
        }
        let rank = entry.material_draw_rank_slot.ok_or(nothing)?;
        let probe = spot.env_probe_index;
        let sort = surface_sort_key(
            entry.order_key,
            rank,
            object,
            probe,
            spot.key_light_index,
            target,
        );
        // Cached and pre-tessellated draws read their vertices from the index list, not from
        // the model's own pose.
        let from_list = matches!(target, PropFacetPath::Cached | PropFacetPath::Pretess);
        let kind = HeldDrawKind::Smodel {
            surface,
            material,
            model_to_world: if from_list {
                Mat4::IDENTITY
            } else {
                model.world_from_local
            },
            illum_handle: model.illum_handle,
            compact_illum: model.compact_illum,
            placement: model.placement,
            stream: Some(target),
            cache_slot_no: from_list.then_some(model.stored_id),
            pretess: range,
        };
        let mut samplers = FacetSamplerInputs::default();
        samplers.reflection_probe = Some(SurfaceCubeProbeId(probe));
        Ok(catalog_item(sort, rank, kind, samplers, self.materials))
    }
}

/// What models in one cached batch must share: mesh, lighting, probe and light slot.
fn batch_shape(spot: &PropPlacement) -> (usize, bool, u8, u8) {
    let PropPlacement {
        model_slot: mesh,
        lit_list: lit,
        env_probe_index,
        key_light_index,
        ..
    } = *spot;
    (mesh, lit, env_probe_index, key_light_index)
}

/// Whether a run of indices is whole triangles and not empty.
fn whole_triangles(run: &[u16]) -> bool {
    run.len() % 3 == 0 && !run.is_empty()
}

/// The cached index run of one surface of `spot`, when it has a usable one.
fn cached_triangles<'a>(
    smc: Option<&'a LevelPropCache>,
    spot: &PropPlacement,
    surface: u32,
) -> Option<&'a [u16]> {
    let slot = spot
        .illum_slot
        .and_then(|slot| u32::try_from(slot).ok())?;
    let found = smc.and_then(|smc| smc.index_stretches.get(&(slot, surface)))?;
    Some(found.as_slice()).filter(|run| whole_triangles(run))
}

/// Whether a surface draws in `pass`: only the sun pass lets a material opt out.
fn draws_in(
    pass: Pass,
    materials: &LiveMaterialLedger,
    given: Option<assets::SurfaceMaterialId>,
) -> bool {
    match pass {
        Pass::Colour => true,
        Pass::SunShadow => match given.and_then(|index| materials.derived(index)) {
            Some(found) => lighting_t6::prop_facet_casts_sun(found.game_flag_info),
            None => true,
        },
    }
}

fn surface_sort_key(
    order_key: u8,
    rank: u32,
    object_tag: u16,
    probe: u8,
    light: u8,
    target: PropFacetPath,
) -> u64 {
    let fields = dpvs_t6::SortedDrawParts {
        object_tag,
        env_probe_index: probe,
        key_light_index: light,
        kind: lighting_t6::prop_facet_kind(target),
        material_draw_rank_slot: render_material::sort_band(rank),
        leading_sort_bucket: dpvs_t6::material_sort_group(order_key),
        ..Default::default()
    };
    dpvs_t6::pack(fields).packed
}

/// The index list pre-tessellated and cached draws read, and the command budget spent on it.
struct SharedIndices {
    pretess_allowed: bool,
    cmds_spent: usize,
    data: Vec<u16>,
}

impl SharedIndices {
    fn starting(pretess_allowed: bool) -> Self {
        SharedIndices {
            pretess_allowed,
            cmds_spent: 0,
            data: Vec::new(),
        }
    }

    fn len_u32(&self) -> u32 {
        u32::try_from(self.data.len()).unwrap_or(u32::MAX)
    }

    /// What the game's cached-draw planner would do with a batch of `ids` and their runs.
    fn plan(&self, id_list: &[u16], found: &[&[u16]]) -> lighting_t6::PropCmdReplay {
        let capacity = render_frame::LIVE_INDEX_BUFFER_CAP;
        lighting_t6::plan_cached_prop_cmds(
            PropFacetPath::Cached,
            id_list,
            self.pretess_allowed,
            false,
            self.cmds_spent,
            usize::MAX,
            self.len_u32(),
            capacity,
            found,
        )
    }

    /// Appends the tessellated runs the planner allocated; `None` leaves the list as it was.
    fn write_tessellated(
        &mut self,
        alloc: lighting_t6::PropPretessGrant,
        found: &[&[u16]],
    ) -> Option<PropPrebuiltRange> {
        let lighting_t6::PropPretessGrant {
            first_idx: first_index,
            index_count: idx_count,
            ..
        } = alloc;
        let (Ok(at), Ok(len)) = (usize::try_from(first_index), usize::try_from(idx_count)) else {
            return None;
        };
        // The planner places the batch at the end of the list; anything else is a stale plan.
        if at != self.data.len() {
            return None;
        }
        self.data.resize(at.saturating_add(len), 0);
        if lighting_t6::copy_presplit_indices(&mut self.data, alloc, found) {
            self.cmds_spent = self.cmds_spent.saturating_add(alloc.command.len());
            Some(PropPrebuiltRange {
                start: first_index,
                count: idx_count,
            })
        } else {
            self.data.truncate(at);
            None
        }
    }

    /// Appends one model's own copy of a cached run, when it is whole triangles and fits.
    fn copy_in(&mut self, run: &[u16]) -> Option<PropPrebuiltRange> {
        if !whole_triangles(run) {
            return None;
        }
        let at = u32::try_from(self.data.len()).ok()?;
        let len = u32::try_from(run.len()).ok()?;
        let fits = at
            .checked_add(len)
            .is_some_and(|end| end <= render_frame::LIVE_INDEX_BUFFER_CAP);
        if !fits {
            return None;
        }
        self.data.extend_from_slice(run);
        Some(PropPrebuiltRange {
            start: at,
            count: len,
        })
    }
}

/// The detail level's flag byte from the model cache table, or 0 when it has none.
pub(crate) fn lod_smc_flag(mesh: &PropMeshFacets, detail: usize) -> u8 {
    let per_lod = mesh.detail_prop_cache_rows.and_then(|table| table.get(detail).copied());
    let entry = match per_lod {
        Some(entry) => Some(entry),
        None if detail == 0 => mesh.detail_prop_cache,
        None => None,
    };
    entry.map_or(0, |bytes| bytes[1])
}

/// Whether a detail level is rigid, or `None` when any of its surfaces lacks the byte that says.
pub(crate) fn lod_is_rigid(mesh: &PropMeshFacets, detail: usize) -> Option<bool> {
    let flags = mesh
        .figure_part_plus_1_by_detail
        .get(detail)?
        .iter()
        .copied()
        .collect::<Option<Vec<u8>>>()?;
    Some(lighting_t6::prop_lod_rigid(&flags))
}

/// Whether a model's packed lighting was baked: present, and not all zero.
pub(crate) fn has_lighting(packed: Option<[u8; 4]>) -> bool {
    packed.is_some_and(|bytes| bytes != [0; 4])
}

/// The cache table's index for a lit model at a detail level, or 0 when it has none.
pub(crate) fn cache_slot_no(
    smc: Option<&LevelPropCache>,
    slot: Option<usize>,
    lod: usize,
) -> u16 {
    let lookup = || -> Option<u16> {
        let per_model = smc?.cache_slot_no.get(slot?)?;
        per_model.get(lod).copied()
    };
    lookup().unwrap_or(0)
}
