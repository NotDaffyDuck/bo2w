use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::LevelGeneration;
use lighting_t6::PropFacetBucketLists;
use render_material::MaterialEpochId;

use super::static_lod::{ChoiceInputs, ModelChoice, ModelChoices, ViewKey, choose_models};
use super::static_models_t6::{ModelSource, emit_model_items};
use super::static_world::{WorldSource, emit_world_items, lay_out_runs};
use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind, draw_order, route_to_products};
use crate::assemble::draw_items::MaterialEpoch;
use crate::assemble::draw_items::list::FacetDrawList;
use crate::assemble::draw_items::mesh_build::prop_mesh::PropGpuPlan;
use crate::assemble::draw_items::mesh_build::world::WorldGpuSubmission;
use crate::prepare::scene::cull::SightFrameStats;
use crate::prepare::scene::prop_geom_memo::DetailRampTunable;
use crate::prepare::scene::prop_geom_memo::PrebuiltTunable;
use crate::prepare::scene::prop_geom_memo::PropCacheEnableTunable;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::eye_parms::StagedStageEye;
use crate::prepare::scene::world::LevelStage;

/// The draw items for everything in the world that does not move: BSP and brush model
/// surfaces, and static models.
///
/// The two halves are kept apart so a camera move that changes no model's detail level keeps
/// both, and a model change rebuilds only the models. The merged, sorted list is what the frame
/// products read.
#[derive(Resource, Clone, Debug, Default)]
pub struct StaticDrawTrack {
    pub revision_stamp: MaterialEpochId,
    pub level_generation: LevelGeneration,
    pub tint_rgb: Vec<HeldDrawItem>,
    pub glow: Vec<HeldDrawItem>,
    pub warp_pass: Vec<HeldDrawItem>,
    pub counts: StaticLaneCounts,
    /// Every world item's run, in sorted item order; an item's `run_start_offset` indexes this.
    pub level_run_facets: Vec<u16>,
    /// Goes up on every rebuild of the sorted list.
    pub membership_edition: u64,
    pub smodel_surf_lists: PropFacetBucketLists,
    pub prop_prebuilt_indices: Arc<Vec<u16>>,
    /// Goes up whenever `prop_prebuilt_indices` is replaced.
    pub prop_index_layout_edition: u64,

    built_from: Option<BuildStamp>,
    sorted: Vec<HeldDrawItem>,
    world: Vec<HeldDrawItem>,
    /// Run members of `world` items, at each item's `run_start_offset`, before sorting.
    world_members: Vec<u16>,
    models: Vec<HeldDrawItem>,
    model_choices: Vec<ModelChoice>,
    logged_buckets: Option<[u32; 5]>,
}

/// What the last frame did with the lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticLaneCounts {
    /// Nothing the lane reads had changed, so it was used as it was.
    pub kept_over: bool,
    /// The view had changed but every model kept its detail level, so nothing was rebuilt.
    pub lod_held: bool,
    /// Placements that passed visibility and distance culling (0 when the lane was reused).
    pub models_queried: u32,
    /// How the current model items were filed.
    pub buckets: BucketTally,
}

/// Models filed into each kind of surface bucket while the model items were built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BucketTally {
    pub rigid: u32,
    pub skinned: u32,
    pub cached: u32,
    /// Models that could not be filed because their detail level does not say rigid or skinned.
    pub unread: u32,
    /// Filings that filled their bucket.
    pub flushes: u32,
    /// Buckets drained into items.
    pub consumed: u32,
    /// Models or drained entries left out because they did not fit the draw context.
    pub declined: u32,
}

/// The inputs the lane was last built from. Equal stamps in the same generation mean the lane
/// can be used again as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BuildStamp {
    world: u64,
    vis: u64,
    view: ViewKey,
}

impl StaticDrawTrack {
    /// The sorted items, the colour, emissive and distortion lists, and the run table, or all
    /// empty when the lane was built for another world.
    pub fn alive_for(
        &self,
        current: LevelGeneration,
    ) -> (
        &[HeldDrawItem],
        &[HeldDrawItem],
        &[HeldDrawItem],
        &[HeldDrawItem],
        &[u16],
    ) {
        if self.level_generation != current {
            return (&[], &[], &[], &[], &[]);
        }
        (
            &self.sorted,
            &self.tint_rgb,
            &self.glow,
            &self.warp_pass,
            &self.level_run_facets,
        )
    }

    fn refill(&mut self, inputs: &StaticLaneInputs, stamp: BuildStamp, same_generation: bool) {
        let catalog = &inputs.materials.catalog;
        let world_kept = same_generation && self.built_from.is_some_and(|b| b.world == stamp.world);
        let choices = inputs.choose_models();
        let models_kept =
            same_generation && self.built_from.is_some() && self.model_choices == choices.list;
        self.counts.kept_over = false;
        if world_kept && models_kept {
            self.counts.lod_held = true;
            self.counts.models_queried = choices.queried;
            self.model_choices = choices.list;
            return;
        }

        let kept_tally = self.counts.buckets;
        self.counts = StaticLaneCounts::default();
        if !world_kept {
            self.world.clear();
            self.world_members.clear();
            self.level_run_facets.clear();
            let source = WorldSource {
                list: &inputs.world_list,
                plan: inputs.level_plan.as_deref(),
                scene: inputs.stage.as_deref(),
            };
            emit_world_items(&source, catalog, &mut self.world, &mut self.world_members);
        }
        if models_kept {
            self.counts.buckets = kept_tally;
            self.counts.models_queried = choices.queried;
            self.model_choices = choices.list;
        } else {
            self.clear_models();
            if let Some(plan) = inputs.model_set.as_deref() {
                self.emit_models(inputs, plan, choices);
            }
        }
        self.compose();
    }

    fn clear_models(&mut self) {
        self.models.clear();
        self.model_choices.clear();
        self.smodel_surf_lists = PropFacetBucketLists::default();
        self.prop_prebuilt_indices = Arc::default();
        self.prop_index_layout_edition = self.prop_index_layout_edition.wrapping_add(1);
    }

    fn emit_models(&mut self, inputs: &StaticLaneInputs, plan: &PropGpuPlan, choices: ModelChoices) {
        let source = ModelSource {
            plan,
            cache: inputs.cache.as_deref(),
            catalog: &inputs.materials.catalog,
            cache_on: inputs.cache_dvar.enabled != Some(false),
            pretess_on: inputs.pretess_dvar.enabled,
        };
        let built = emit_model_items(&source, &choices, &mut self.smodel_surf_lists);
        self.counts.models_queried = choices.queried;
        self.counts.buckets = built.tally;
        self.log_buckets();
        if *self.prop_prebuilt_indices != built.pretess_indices {
            self.prop_index_layout_edition = self.prop_index_layout_edition.wrapping_add(1);
            self.prop_prebuilt_indices = Arc::new(built.pretess_indices);
        }
        self.models = built.items;
        self.model_choices = choices.list;
    }

    /// Logs the bucket tally when it differs from the last one logged.
    fn log_buckets(&mut self) {
        let tally = self.counts.buckets;
        let row = [
            tally.rigid,
            tally.skinned,
            tally.cached,
            tally.unread,
            tally.consumed,
        ];
        if self.logged_buckets == Some(row) {
            return;
        }
        self.logged_buckets = Some(row);
        let [rigid, skinned, cached, unread, consume] = row;
        diag::info!(
            World,
            "smodel buckets: rigid={rigid} skinned={skinned} cached={cached} unread={unread} consume={consume}",
        );
    }

    /// Merges world and model items into one sorted list, lays out the world runs in that
    /// order, and files every item under its frame products. World items also go to the
    /// distortion list.
    fn compose(&mut self) {
        self.sorted.clear();
        self.sorted.extend_from_slice(&self.world);
        self.sorted.extend_from_slice(&self.models);
        self.sorted.sort_unstable_by_key(draw_order);
        lay_out_runs(&mut self.sorted, &self.world_members, &mut self.level_run_facets);
        self.tint_rgb.clear();
        self.glow.clear();
        self.warp_pass.clear();
        for &item in &self.sorted {
            let is_world = matches!(item.kind, HeldDrawKind::World { .. });
            let warp = is_world.then_some(&mut self.warp_pass);
            route_to_products(&mut self.tint_rgb, &mut self.glow, warp, item);
        }
        self.membership_edition = self.membership_edition.wrapping_add(1);
    }
}

/// Everything the static lane reads each frame.
#[derive(SystemParam)]
pub(crate) struct StaticLaneInputs<'w> {
    world_list: Res<'w, FacetDrawList>,
    level_plan: Option<Res<'w, WorldGpuSubmission>>,
    stage: Option<Res<'w, LevelStage>>,
    model_set: Option<Res<'w, PropGpuPlan>>,
    lit_info: Option<Res<'w, LevelPropIllum>>,
    dpvs: Option<Res<'w, SightFrameStats>>,
    cache: Option<Res<'w, LevelPropCache>>,
    cache_dvar: Res<'w, PropCacheEnableTunable>,
    pretess_dvar: Res<'w, PrebuiltTunable>,
    view: Option<Res<'w, StagedStageEye>>,
    detail_ramp: Res<'w, DetailRampTunable>,
    materials: Res<'w, MaterialEpoch>,
    level_generation: Option<Res<'w, LevelGeneration>>,
}

impl StaticLaneInputs<'_> {
    fn eye(&self) -> Option<Vec3> {
        self.view.as_deref().filter(|view| view.primed).map(|view| view.eye)
    }

    fn stamp(&self) -> BuildStamp {
        BuildStamp {
            world: self.world_list.draw_batch_list_id,
            vis: self.dpvs.as_deref().map_or(0, |dpvs| dpvs.prop_vis_id),
            view: ViewKey::new(self.eye(), self.detail_ramp.args()),
        }
    }

    fn choose_models(&self) -> ModelChoices {
        let Some(plan) = self.model_set.as_deref() else {
            return ModelChoices::default();
        };
        let cull = self.stage.as_deref().and_then(|scene| scene.culling.as_ref());
        let inputs = ChoiceInputs {
            vis: self.dpvs.as_deref().map_or(&[], |dpvs| &dpvs.prop_vis),
            hide_dists: cull.map_or(&[], |cull| &cull.prop_hide_dists),
            eye: self.eye(),
            ramp: self.detail_ramp.args(),
            lighting: self.lit_info.as_deref(),
            cache: self.cache.as_deref(),
        };
        choose_models(plan, &inputs)
    }
}

/// Brings the static lane up to date: used as it is when nothing it reads has changed,
/// otherwise rebuilt from whichever half (world or models) changed.
pub(crate) fn refill_static_draw_lane(mut lane: ResMut<StaticDrawTrack>, inputs: StaticLaneInputs) {
    let _span = perf::Span::ServerAfterRegenMillis.enter();
    let lane = lane.as_mut();
    let generation = inputs.materials.catalog.revision_stamp;
    let level_generation = inputs.level_generation.as_deref().copied().unwrap_or_default();
    let same_generation =
        lane.revision_stamp == generation && lane.level_generation == level_generation;
    lane.revision_stamp = generation;
    lane.level_generation = level_generation;
    let stamp = inputs.stamp();
    if same_generation && !lane.sorted.is_empty() && lane.built_from == Some(stamp) {
        lane.counts.kept_over = true;
        lane.counts.lod_held = false;
        lane.counts.models_queried = 0;
    } else {
        lane.refill(&inputs, stamp, same_generation);
    }
    lane.built_from = Some(stamp);
}
