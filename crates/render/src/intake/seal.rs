//! Seals each frame's drawing data behind the frontend's assembly.
//!
//! The installed world holds what outlives a frame (static geometry, image
//! handles); the published frame holds what this frame draws. Both are cut
//! from the plans the frontend assembled, and only when the frontend says
//! those plans belong to one coherent frame.

use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{LevelGeneration, WorldProducts};
use render_frame::{FrameOutputState, FrameOutputsSnapshot};
use render_frontend::assemble::draw_items::mesh_build::fx::FxCodeDrawPlan;
use render_frontend::assemble::draw_items::mesh_build::mark::VisualStainMeshPlan;
use render_frontend::assemble::draw_items::mesh_build::particle_cloud::FxCloudDrawPlan;
use render_frontend::assemble::draw_items::mesh_build::prop_mesh::PropGpuPlan;
use render_frontend::assemble::draw_items::mesh_build::world::WorldGpuSubmission;
use render_frontend::assemble::draw_items::mesh_build::figure_mesh::FigureDrawPlan;
use render_frontend::assemble::draw_items::{
    FrameBuildFeed, MapSunEffects, MaterialFrameFeed, MaterialEpoch,
    FrameRenderOutputs, LiveImageHandles, StaticDrawTrack,
    SunEffectsFrameInput, material_run,
};
use render_frontend::prepare::scene::prop_geom_memo::LevelPropCache;
use render_material::MaterialEpochId;
use render_gpu::{
    ExtractedRenderFrameOutputs, StaticGeometryExtract, SubmitReadyGpu, InstalledRenderWorld,
    PublishedRenderFrame, RenderFrameData, RenderWorldData,
};

use super::shares::share;

/// Everything the seal reads. Each is absent until its owner has built it.
#[derive(SystemParam)]
pub struct SealInputs<'w> {
    runtime: Option<Res<'w, MaterialEpoch>>,
    material: Option<Res<'w, MaterialFrameFeed>>,
    assembly: Option<Res<'w, FrameBuildFeed>>,
    products: Option<Res<'w, FrameRenderOutputs>>,
    world: Option<Res<'w, WorldGpuSubmission>>,
    smodel: Option<Res<'w, PropGpuPlan>>,
    smc: Option<Res<'w, LevelPropCache>>,
    retained: Option<Res<'w, StaticDrawTrack>>,
    level_generation: Option<Res<'w, LevelGeneration>>,
    world_products: Option<Res<'w, WorldProducts>>,
    xmodel: Option<Res<'w, FigureDrawPlan>>,
    fx: Option<Res<'w, FxCodeDrawPlan>>,
    particle_cloud: Option<Res<'w, FxCloudDrawPlan>>,
    mark_mesh: Option<Res<'w, VisualStainMeshPlan>>,
    images: Option<Res<'w, LiveImageHandles>>,
    submit: Option<Res<'w, SubmitReadyGpu>>,
    sun_map: Option<Res<'w, MapSunEffects>>,
    sun_frame: Option<Res<'w, SunEffectsFrameInput>>,
    installed: Option<Res<'w, InstalledRenderWorld>>,
    published: Option<Res<'w, PublishedRenderFrame>>,
}

/// With nothing sealed the render world is told there is nothing to draw.
pub(super) fn install_empty(commands: &mut Commands) {
    commands.insert_resource(InstalledRenderWorld::default());
    commands.insert_resource(PublishedRenderFrame::default());
}

/// The identity of the frame being sealed.
struct Basis<'a> {
    epoch_id: MaterialEpochId,
    level_generation: LevelGeneration,
    world_products: WorldProducts,
    material: &'a MaterialFrameFeed,
    snapshot: Arc<FrameOutputsSnapshot>,
}

impl<'a> Basis<'a> {
    /// None unless the materials, their frame inputs and the product list are
    /// all there and the assembly vouches that they are one frame.
    fn of(input: &'a SealInputs) -> Option<Self> {
        let generation = input.runtime.as_deref()?.catalog.revision_stamp;
        let material = input.material.as_deref()?;
        let snapshot = input.products.as_deref()?.published();
        let level_generation = input
            .level_generation
            .as_deref()
            .map_or(LevelGeneration(None), |generation| *generation);
        let world_products = input.world_products.as_deref().copied().unwrap_or_default();
        let assembled = input.assembly.as_deref().is_some_and(|assembly| {
            assembly.frame_seq_no == snapshot.frame_seq_no
                && assembly.ledger_generation == generation
                && assembly.level_generation == level_generation
        });
        let current = snapshot.products.iter().all(|output| {
            matches!(output.state, FrameOutputState::Missing(_))
                || output.revision_stamp == generation
        });
        (assembled && current).then_some(Self {
            epoch_id: generation,
            level_generation,
            world_products,
            material,
            snapshot,
        })
    }
}

/// How much static geometry there is, for telling whether the installed
/// copy is still the one the plans describe.
#[derive(PartialEq, Eq)]
struct Shape {
    level_vertex_total: usize,
    level_indices: usize,
    world_layers: usize,
    static_model_verts_used: usize,
    static_model_slots_used: usize,
}

impl Shape {
    fn installed(installed: &InstalledRenderWorld) -> Self {
        let rows = &installed.static_geometry;
        Self {
            level_vertex_total: rows.level_vertex_total.len(),
            level_indices: rows.level_indices.len(),
            world_layers: rows.world_layer.len(),
            static_model_verts_used: rows.static_model_verts_used.len(),
            static_model_slots_used: rows.static_model_slots_used.len(),
        }
    }

    /// A plan whose vertices are refused counts as empty.
    fn planned(world: Option<&WorldGpuSubmission>, smodel: Option<&PropGpuPlan>) -> Self {
        let (level_vertex_total, level_indices, world_layers) = world
            .filter(|plan| plan.precise_compact_vertices().is_ok())
            .map_or((0, 0, 0), |plan| {
                (
                    plan.precise_compact_vertices().map_or(0, <[_]>::len),
                    plan.indices().len(),
                    plan.vertex_layer_rows().len(),
                )
            });
        let (static_model_verts_used, static_model_slots_used) = smodel
            .filter(|plan| plan.precise_compact_vertices().is_ok())
            .map_or((0, 0), |plan| {
                (
                    plan.precise_compact_vertices().map_or(0, <[_]>::len),
                    plan.indices().len(),
                )
            });
        Self {
            level_vertex_total,
            level_indices,
            world_layers,
            static_model_verts_used,
            static_model_slots_used,
        }
    }
}

/// Which parts of the installed world can stay as they are.
#[derive(Clone, Copy)]
struct Kept {
    statics: bool,
    smc_maps: bool,
}

impl Kept {
    fn decide(input: &SealInputs, basis: &Basis, prop_cache_edition: Option<u64>) -> Self {
        let Some(installed) = input.installed.as_deref() else {
            return Self {
                statics: false,
                smc_maps: false,
            };
        };
        let same_world = installed.level_generation == basis.level_generation
            || installed.world_products.same_as(basis.world_products);
        let statics = same_world
            && Shape::installed(installed)
                == Shape::planned(input.world.as_deref(), input.smodel.as_deref());
        Self {
            statics,
            smc_maps: statics && installed.prop_cache_edition == prop_cache_edition,
        }
    }
}

fn fill_world(plan: Option<&WorldGpuSubmission>, rows: &mut StaticGeometryExtract) {
    let Some(plan) = plan else { return };
    match plan.precise_compact_vertices() {
        Ok(_) => {
            rows.level_vertex_total = share(plan.vertex_share.as_ref());
            rows.world_layer = share(plan.layer_share.as_ref());
            rows.level_indices = share(plan.index_portion.as_ref());
            rows.world_surface_ranges = share(plan.range_portion.as_ref());
        }
        Err(cause) => rows.world_vertex_refusal = Some(cause),
    }
}

fn fill_smodel(plan: Option<&PropGpuPlan>, rows: &mut StaticGeometryExtract) {
    let Some(plan) = plan else { return };
    match plan.precise_compact_vertices() {
        Ok(_) => {
            rows.static_model_verts_used = share(plan.compact_portion.as_ref());
            rows.static_model_slots_used = share(plan.index_portion.as_ref());
            rows.smodel_surface_ranges = share(plan.range_portion.as_ref());
            rows.smodel_cached_vertices = share(plan.cached_share.as_ref());
            rows.smodel_surface_verts = share(plan.vert_range_share.as_ref());
        }
        Err(cause) => rows.smodel_vertex_refusal = Some(cause),
    }
}

fn fill_xmodel(plan: Option<&FigureDrawPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.figure_edition = plan.revision;
    out.figure_topology_edition = plan.topology_edition;
    out.xmodel_packed_segments = plan.compact_segments;
    match plan.precise_compact_vertices() {
        Ok(_) => {
            out.xmodel_verts = share(plan.compact_portion.as_ref());
            out.figure_indices = share(plan.index_portion.as_ref());
            out.figure_facet_ranges = share(plan.range_portion.as_ref());
        }
        Err(cause) => out.xmodel_vertex_refusal = Some(cause),
    }
}

fn fill_fx(plan: Option<&FxCodeDrawPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.effect_edition = plan.revision;
    match plan.precise_compact_vertices() {
        Ok(_) => {
            out.fx_verts = Arc::clone(&plan.vertices);
            out.effect_indices = Arc::clone(&plan.indices);
            out.fx_surface_ranges = share(plan.range_portion.as_ref());
        }
        Err(cause) => out.fx_vertex_refusal = Some(cause),
    }
}

fn fill_particle_cloud(plan: Option<&FxCloudDrawPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.cloud_verts = Arc::clone(&plan.vertices);
    out.cloud_indices = Arc::clone(&plan.indices);
    out.particle_cloud_surface_ranges = share(plan.range_portion.as_ref());
    out.particle_cloud_template = plan.kept_template_rows();
    out.particle_cloud_revision = plan.revision;
}

fn fill_mark_mesh(plan: Option<&VisualStainMeshPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.mark_verts = Arc::clone(&plan.vertices);
    out.stain_mesh_indices = Arc::clone(&plan.indices);
    out.mark_mesh_surface_ranges = share(plan.range_portion.as_ref());
    out.mark_mesh_surface_packed = Arc::new(plan.draws.iter().map(|draw| draw.minor_key.packed).collect());
    out.stain_mesh_edition = plan.revision;
}

/// This frame's draw data, from the plans the frontend assembled.
fn frame_data(input: &SealInputs, basis: Basis, smc: Option<&LevelPropCache>) -> RenderFrameData {
    let submit = input.submit.as_deref();
    let mut exec_rows = input
        .published
        .as_deref()
        .map(|published| published.exec_frame.clone())
        .unwrap_or_default();
    material_run::refresh(&mut exec_rows, basis.material, input.assembly.as_deref());
    let mut out = RenderFrameData {
        frame_products: ExtractedRenderFrameOutputs(basis.snapshot),
        generation: basis.epoch_id,
        level_generation: basis.level_generation,
        exec_frame: exec_rows,
        sun_shadow: basis.material.sun_shadow,
        sun_effects: input.sun_frame.as_deref().and_then(|sun| sun.frame),
        prewarmed_pipelines: submit.is_some_and(|job| job.prewarmed_pipelines),
        pipeline_level_materials: submit
            .map(|job| job.pipeline_level_materials.clone())
            .unwrap_or_default(),
        pipeline_prop_materials: submit
            .map(|job| job.pipeline_prop_materials.clone())
            .unwrap_or_default(),
        pipeline_demand_revision: submit.map_or(0, |job| job.pipeline_demand_revision),
        smc_vb_patches: smc.map(|cache| cache.vb_patches().to_vec()).unwrap_or_default(),
        prop_cache_ib_patches: smc.map(|cache| cache.index_patches().to_vec()).unwrap_or_default(),
        ..Default::default()
    };
    fill_xmodel(input.xmodel.as_deref(), &mut out);
    fill_fx(input.fx.as_deref(), &mut out);
    fill_particle_cloud(input.particle_cloud.as_deref(), &mut out);
    fill_mark_mesh(input.mark_mesh.as_deref(), &mut out);
    out
}

pub fn seal_render_frame(mut commands: Commands, input: SealInputs) {
    let Some(basis) = Basis::of(&input) else {
        install_empty(&mut commands);
        return;
    };
    let smc = input.smc.as_deref();
    let prop_cache_edition = smc.map(|cache| cache.content_edition());
    let kept = Kept::decide(&input, &basis, prop_cache_edition);
    let installed = input.installed.as_deref();

    let static_geometry = match installed.filter(|_| kept.statics) {
        Some(installed) => installed.static_geometry.clone(),
        None => {
            let mut rows = StaticGeometryExtract::default();
            fill_world(input.world.as_deref(), &mut rows);
            fill_smodel(input.smodel.as_deref(), &mut rows);
            Arc::new(rows)
        }
    };
    let baked_smc = match installed.filter(|_| kept.smc_maps) {
        Some(installed) => installed.smc_index_baked.clone(),
        None => Arc::new(smc.map(|cache| cache.baked_index_list()).unwrap_or_default()),
    };
    let retained = input.retained.as_deref();
    let world = InstalledRenderWorld::new(RenderWorldData {
        generation: basis.epoch_id,
        level_generation: basis.level_generation,
        world_products: basis.world_products,
        prop_cache_edition,
        static_geometry,
        smc_index_baked: baked_smc,
        prop_prebuilt_indices: retained.map_or_else(Arc::default, |lane| {
            Arc::clone(&lane.prop_prebuilt_indices)
        }),
        prop_index_layout_edition: retained.map_or(0, |lane| lane.prop_index_layout_edition),
        image_handles: input.images.as_deref().cloned().unwrap_or_default(),
        sun_effects: input.sun_map.as_deref().and_then(|sun| sun.def),
    });
    let data = frame_data(&input, basis, smc);
    commands.insert_resource(world.clone());
    commands.insert_resource(PublishedRenderFrame::seal(world, data));
}
