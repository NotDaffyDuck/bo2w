use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::WorldGeneration;
use lighting_t6::SmodelSurfBucketLists;
use render_material::MaterialGenerationId;

use super::static_lod::{ChoiceInputs, ModelChoice, ModelChoices, ViewKey, choose_models};
use super::static_models_t6::{ModelSource, emit_model_items};
use super::static_world::{WorldSource, emit_world_items, lay_out_runs};
use super::{RetainedDrawItem, RetainedDrawKind, draw_order, route_to_products};
use crate::assemble::drawsurf::MaterialGeneration;
use crate::assemble::drawsurf::list::DrawSurfList;
use crate::assemble::drawsurf::tess::smodel::SmodelGpuPlan;
use crate::assemble::drawsurf::tess::world::WorldDrawGpuPlan;
use crate::prepare::scene::cull::DpvsFrameStats;
use crate::prepare::scene::smodel_geom_cache::{
    LodRampDvar, PretessDvar, SmcEnableDvar, WorldStaticModelCache,
};
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::view_parms::PreparedSceneView;
use crate::prepare::scene::world::WorldScene;

/// The draw items for everything in the world that does not move: BSP and brush model
/// surfaces, and static models.
///
/// The two halves are kept apart so a camera move that changes no model's detail level keeps
/// both, and a model change rebuilds only the models. The merged, sorted list is what the frame
/// products read.
#[derive(Resource, Clone, Debug, Default)]
pub struct StaticDrawLane {
    pub generation_id: MaterialGenerationId,
    pub world_generation: WorldGeneration,
    pub colour: Vec<RetainedDrawItem>,
    pub emissive: Vec<RetainedDrawItem>,
    pub distortion: Vec<RetainedDrawItem>,
    pub counts: StaticLaneCounts,
    /// Every world item's run, in sorted item order; an item's `run_off` indexes this.
    pub world_run_surfs: Vec<u16>,
    /// Goes up on every rebuild of the sorted list.
    pub membership_revision: u64,
    pub smodel_surf_lists: SmodelSurfBucketLists,
    pub smodel_pretess_indices: Arc<Vec<u16>>,
    /// Goes up whenever `smodel_pretess_indices` is replaced.
    pub smodel_index_layout_revision: u64,

    built_from: Option<BuildStamp>,
    sorted: Vec<RetainedDrawItem>,
    world: Vec<RetainedDrawItem>,
    /// Run members of `world` items, at each item's `run_off`, before sorting.
    world_members: Vec<u16>,
    models: Vec<RetainedDrawItem>,
    model_choices: Vec<ModelChoice>,
    logged_buckets: Option<[u32; 5]>,
}

/// What the last frame did with the lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticLaneCounts {
    /// Nothing the lane reads had changed, so it was used as it was.
    pub reused: bool,
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
    pub refused: u32,
}

/// The inputs the lane was last built from. Equal stamps in the same generation mean the lane
/// can be used again as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BuildStamp {
    world: u64,
    vis: u64,
    view: ViewKey,
}

impl StaticDrawLane {
    /// The sorted items, the colour, emissive and distortion lists, and the run table, or all
    /// empty when the lane was built for another world.
    pub fn live_for(
        &self,
        current: WorldGeneration,
    ) -> (
        &[RetainedDrawItem],
        &[RetainedDrawItem],
        &[RetainedDrawItem],
        &[RetainedDrawItem],
        &[u16],
    ) {
        if self.world_generation != current {
            return (&[], &[], &[], &[], &[]);
        }
        (
            &self.sorted,
            &self.colour,
            &self.emissive,
            &self.distortion,
            &self.world_run_surfs,
        )
    }

    fn rebuild(&mut self, inputs: &StaticLaneInputs, stamp: BuildStamp, same_generation: bool) {
        let catalog = &inputs.materials.catalog;
        let world_kept = same_generation && self.built_from.is_some_and(|b| b.world == stamp.world);
        let choices = inputs.choose_models();
        let models_kept =
            same_generation && self.built_from.is_some() && self.model_choices == choices.list;
        self.counts.reused = false;
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
            self.world_run_surfs.clear();
            let source = WorldSource {
                list: &inputs.world_list,
                plan: inputs.world_plan.as_deref(),
                scene: inputs.scene.as_deref(),
            };
            emit_world_items(&source, catalog, &mut self.world, &mut self.world_members);
        }
        if models_kept {
            self.counts.buckets = kept_tally;
            self.counts.models_queried = choices.queried;
            self.model_choices = choices.list;
        } else {
            self.clear_models();
            if let Some(plan) = inputs.models.as_deref() {
                self.emit_models(inputs, plan, choices);
            }
        }
        self.compose();
    }

    fn clear_models(&mut self) {
        self.models.clear();
        self.model_choices.clear();
        self.smodel_surf_lists = SmodelSurfBucketLists::default();
        self.smodel_pretess_indices = Arc::default();
        self.smodel_index_layout_revision = self.smodel_index_layout_revision.wrapping_add(1);
    }

    fn emit_models(&mut self, inputs: &StaticLaneInputs, plan: &SmodelGpuPlan, choices: ModelChoices) {
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
        if *self.smodel_pretess_indices != built.pretess_indices {
            self.smodel_index_layout_revision = self.smodel_index_layout_revision.wrapping_add(1);
            self.smodel_pretess_indices = Arc::new(built.pretess_indices);
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
        lay_out_runs(&mut self.sorted, &self.world_members, &mut self.world_run_surfs);
        self.colour.clear();
        self.emissive.clear();
        self.distortion.clear();
        for &item in &self.sorted {
            let is_world = matches!(item.kind, RetainedDrawKind::World { .. });
            let distortion = is_world.then_some(&mut self.distortion);
            route_to_products(&mut self.colour, &mut self.emissive, distortion, item);
        }
        self.membership_revision = self.membership_revision.wrapping_add(1);
    }
}

/// Everything the static lane reads each frame.
#[derive(SystemParam)]
pub(crate) struct StaticLaneInputs<'w> {
    world_list: Res<'w, DrawSurfList>,
    world_plan: Option<Res<'w, WorldDrawGpuPlan>>,
    scene: Option<Res<'w, WorldScene>>,
    models: Option<Res<'w, SmodelGpuPlan>>,
    lighting: Option<Res<'w, WorldSmodelLighting>>,
    dpvs: Option<Res<'w, DpvsFrameStats>>,
    cache: Option<Res<'w, WorldStaticModelCache>>,
    cache_dvar: Res<'w, SmcEnableDvar>,
    pretess_dvar: Res<'w, PretessDvar>,
    view: Option<Res<'w, PreparedSceneView>>,
    lod_ramp: Res<'w, LodRampDvar>,
    materials: Res<'w, MaterialGeneration>,
    world_generation: Option<Res<'w, WorldGeneration>>,
}

impl StaticLaneInputs<'_> {
    fn eye(&self) -> Option<Vec3> {
        self.view.as_deref().filter(|view| view.ready).map(|view| view.eye)
    }

    fn stamp(&self) -> BuildStamp {
        BuildStamp {
            world: self.world_list.draw_items_id,
            vis: self.dpvs.as_deref().map_or(0, |dpvs| dpvs.smodel_vis_id),
            view: ViewKey::new(self.eye(), self.lod_ramp.args()),
        }
    }

    fn choose_models(&self) -> ModelChoices {
        let Some(plan) = self.models.as_deref() else {
            return ModelChoices::default();
        };
        let cull = self.scene.as_deref().and_then(|scene| scene.cull.as_ref());
        let inputs = ChoiceInputs {
            vis: self.dpvs.as_deref().map_or(&[], |dpvs| &dpvs.smodel_vis),
            cull_dists: cull.map_or(&[], |cull| &cull.static_model_cull_dists),
            eye: self.eye(),
            ramp: self.lod_ramp.args(),
            lighting: self.lighting.as_deref(),
            cache: self.cache.as_deref(),
        };
        choose_models(plan, &inputs)
    }
}

/// Brings the static lane up to date: used as it is when nothing it reads has changed,
/// otherwise rebuilt from whichever half (world or models) changed.
pub(crate) fn rebuild_static_draw_lane(mut lane: ResMut<StaticDrawLane>, inputs: StaticLaneInputs) {
    let _span = perf::Span::HostPostRebuildMs.enter();
    let lane = lane.as_mut();
    let generation = inputs.materials.catalog.generation_id;
    let world_generation = inputs.world_generation.as_deref().copied().unwrap_or_default();
    let same_generation =
        lane.generation_id == generation && lane.world_generation == world_generation;
    lane.generation_id = generation;
    lane.world_generation = world_generation;
    let stamp = inputs.stamp();
    if same_generation && !lane.sorted.is_empty() && lane.built_from == Some(stamp) {
        lane.counts.reused = true;
        lane.counts.lod_held = false;
        lane.counts.models_queried = 0;
    } else {
        lane.rebuild(&inputs, stamp, same_generation);
    }
    lane.built_from = Some(stamp);
}
