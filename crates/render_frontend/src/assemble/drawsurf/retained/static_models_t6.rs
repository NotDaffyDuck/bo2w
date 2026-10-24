use lighting_t6::{SMODEL_BUCKET_CACHED, SMODEL_BUCKET_SKINNED, SmodelSurfBucketLists};

use super::RetainedDrawItem;
use super::model_filing_t6::{BucketKey, Filer, Pass, Queued, lighting_nonzero, lod_is_rigid, lod_smc_flag};
use super::static_lane::BucketTally;
use super::static_lod::ModelChoices;
use crate::assemble::drawsurf::material_runtime::RuntimeMaterialCatalog;
use crate::assemble::drawsurf::tess::smodel::SmodelGpuPlan;
use crate::prepare::scene::smodel_geom_cache::WorldStaticModelCache;

/// What turning chosen models into items reads besides the choices.
pub(super) struct ModelSource<'a> {
    pub(super) plan: &'a SmodelGpuPlan,
    pub(super) cache: Option<&'a WorldStaticModelCache>,
    pub(super) catalog: &'a RuntimeMaterialCatalog,
    pub(super) cache_on: bool,
    pub(super) pretess_on: bool,
}

/// The model items of one rebuild, the bucket tally behind them, and the index list their
/// pre-tessellated draws read.
pub(super) struct ModelItems {
    pub(super) items: Vec<RetainedDrawItem>,
    pub(super) tally: BucketTally,
    pub(super) pretess_indices: Vec<u16>,
}

/// Files every chosen model into the game's surface buckets and drains them into items.
///
/// A bucket that fills is drained at once; whatever is left drains after the last model. A
/// model whose detail level does not say whether it is rigid or skinned cannot be filed and
/// is not drawn.
pub(super) fn emit_model_items(
    source: &ModelSource,
    choices: &ModelChoices,
    lists: &mut SmodelSurfBucketLists,
) -> ModelItems {
    let plan = source.plan;
    let mut filer = Filer::new(plan, source.catalog, source.cache, source.pretess_on);
    let mut tally = BucketTally::default();
    for choice in &choices.list {
        let index = choice.placement as usize;
        let Some(placement) = plan.placements.get(index) else {
            continue;
        };
        let Some(mesh) = plan.meshes.get(placement.mesh) else {
            continue;
        };
        let Ok(model_index) = u16::try_from(placement.lighting_slot.unwrap_or(index)) else {
            tally.refused += 1;
            continue;
        };
        let lod = usize::from(choice.lod);
        let key = BucketKey {
            lod: i32::from(choice.lod),
            smc_enabled: source.cache_on,
            lit: lighting_nonzero(choice.packed_lighting),
            smc_flag: lod_smc_flag(mesh, lod),
            cache_index: choice.cache_index,
            model_index,
            rigid: lod_is_rigid(mesh, lod),
        };
        let model = Queued::new(
            choice.placement,
            choice.lod,
            placement.world_from_local,
            choice.lighting_handle,
            choice.packed_lighting,
            Pass::Colour,
        );
        let filed = filer.file(lists, key, model);
        tally.note(filed.bucket, filed.full);
        let drained = filer.drain_filled(lists, filed);
        tally.consumed += drained.buckets;
        tally.refused += drained.refused;
    }
    if tally.unread > 0 {
        diag::warn!(
            World,
            "smodel arm unknown: {} of {} admitted placements carry no XSurface+1 byte (rigid/skinned undecidable) — dropped, not drawn",
            tally.unread,
            choices.queried
        );
    }
    let drained = filer.drain_rest(lists);
    tally.consumed += drained.buckets;
    tally.refused += drained.refused;
    let filings = filer.finish();
    ModelItems {
        items: filings.items,
        tally,
        pretess_indices: filings.indices,
    }
}

impl BucketTally {
    /// Counts one model by the bucket it went to; a model with no bucket counts as unread.
    fn note(&mut self, bucket: Option<i32>, full: bool) {
        let Some(bucket) = bucket else {
            self.unread += 1;
            return;
        };
        match bucket & 3 {
            SMODEL_BUCKET_CACHED => self.cached += 1,
            SMODEL_BUCKET_SKINNED => self.skinned += 1,
            _ => self.rigid += 1,
        }
        self.flushes += u32::from(full);
    }
}
