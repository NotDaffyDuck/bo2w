use lighting_t6::{STATIC_BUCKET_KIND_CACHED, STATIC_BUCKET_KIND_BONE_DRIVEN, PropFacetBucketLists};

use super::HeldDrawItem;
use super::model_sorting_t6::BucketKey;
use super::model_sorting_t6::Filer;
use super::model_sorting_t6::Pass;
use super::model_sorting_t6::Waiting;
use super::model_sorting_t6::has_lighting;
use super::model_sorting_t6::lod_is_rigid;
use super::model_sorting_t6::lod_smc_flag;
use super::fixed_lane::BucketTally;
use super::static_lod::ModelChoices;
use crate::assemble::draw_items::material_live::LiveMaterialLedger;
use crate::assemble::draw_items::mesh_build::prop_mesh::PropGpuPlan;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;

/// What turning chosen models into items reads besides the choices.
pub(super) struct ModelSource<'a> {
    pub(super) plan: &'a PropGpuPlan,
    pub(super) cache: Option<&'a LevelPropCache>,
    pub(super) catalog: &'a LiveMaterialLedger,
    pub(super) cache_on: bool,
    pub(super) pretess_on: bool,
}

/// The model items of one rebuild, the bucket tally behind them, and the index list their
/// pre-tessellated draws read.
pub(super) struct ModelItems {
    pub(super) items: Vec<HeldDrawItem>,
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
    lists: &mut PropFacetBucketLists,
) -> ModelItems {
    let plan = source.plan;
    let mut filer = Filer::new(plan, source.catalog, source.cache, source.pretess_on);
    let mut tally = BucketTally::default();
    for choice in &choices.list {
        let index = choice.placement as usize;
        let Some(placement) = plan.placements.get(index) else {
            continue;
        };
        let Some(mesh_h) = plan.meshes.get(placement.model_slot) else {
            continue;
        };
        let Ok(mesh_slot_number) = u16::try_from(placement.illum_slot.unwrap_or(index)) else {
            tally.declined += 1;
            continue;
        };
        let lod = usize::from(choice.lod);
        let key = BucketKey {
            lod: i32::from(choice.lod),
            smc_enabled: source.cache_on,
            lit: has_lighting(choice.compact_illum),
            smc_flag: lod_smc_flag(mesh_h, lod),
            cache_slot_no: choice.cache_slot_no,
            mesh_slot_number,
            rigid: lod_is_rigid(mesh_h, lod),
        };
        let model = Waiting::new(
            choice.placement,
            choice.lod,
            placement.local_to_world,
            choice.illum_handle,
            choice.compact_illum,
            Pass::Colour,
        );
        let filed = filer.file(lists, key, model);
        tally.count_model(filed.bucket, filed.full);
        let drained = filer.drain_filled(lists, filed);
        tally.consumed += drained.buckets;
        tally.declined += drained.refused;
    }
    if tally.unread > 0 {
        diag::warn!(
            World,
            "static models skipped: {} of {} placed models lack the XSurface+1 byte that says rigid or skinned, so they are not drawn",
            tally.unread,
            choices.queried
        );
    }
    let drained = filer.drain_rest(lists);
    tally.consumed += drained.buckets;
    tally.declined += drained.refused;
    let filings = filer.finish();
    ModelItems {
        items: filings.items,
        tally,
        pretess_indices: filings.indices,
    }
}

impl BucketTally {
    /// Counts one model by the bucket it went to; a model with no bucket counts as unread.
    fn count_model(&mut self, bin: Option<i32>, all_full: bool) {
        let Some(group_no) = bin else {
            self.unread += 1;
            return;
        };
        match group_no & 3 {
            STATIC_BUCKET_KIND_CACHED => self.cached += 1,
            STATIC_BUCKET_KIND_BONE_DRIVEN => self.skinned += 1,
            _ => self.rigid += 1,
        }
        self.flushes += u32::from(all_full);
    }
}
