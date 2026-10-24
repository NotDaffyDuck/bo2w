//! Seals each frame's drawing data behind the frontend's assembly.
//!
//! The installed world holds what outlives a frame (static geometry, image
//! handles); the published frame holds what this frame draws. Both are cut
//! from the plans the frontend assembled, and only when the frontend says
//! those plans belong to one coherent frame.

use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{WorldGeneration, WorldProducts};
use render_frame::{FrameProductStatus, FrameProductsSnapshot};
use render_frontend::assemble::drawsurf::tess::fx::FxCodeMeshPlan;
use render_frontend::assemble::drawsurf::tess::mark::GfxMarkMeshPlan;
use render_frontend::assemble::drawsurf::tess::particle_cloud::FxParticleCloudPlan;
use render_frontend::assemble::drawsurf::tess::smodel::SmodelGpuPlan;
use render_frontend::assemble::drawsurf::tess::world::WorldDrawGpuPlan;
use render_frontend::assemble::drawsurf::tess::xmodel::XModelDrawPlan;
use render_frontend::assemble::drawsurf::{
    FrameAssemblyInputs, MapSunEffects, MaterialFrameInputs, MaterialGeneration,
    RenderFrameProducts, RuntimeImageHandles, StaticDrawLane,
    SunEffectsFrameInput, material_exec,
};
use render_frontend::prepare::scene::smodel_geom_cache::WorldStaticModelCache;
use render_material::MaterialGenerationId;
use render_gpu::{
    ExtractedRenderFrameProducts, ExtractedStaticGeometry, GpuSubmitReady, InstalledRenderWorld,
    PublishedRenderFrame, RenderFrameData, RenderWorldData,
};

use super::shares::share;

/// Everything the seal reads. Each is absent until its owner has built it.
#[derive(SystemParam)]
pub struct SealInputs<'w> {
    runtime: Option<Res<'w, MaterialGeneration>>,
    material: Option<Res<'w, MaterialFrameInputs>>,
    assembly: Option<Res<'w, FrameAssemblyInputs>>,
    products: Option<Res<'w, RenderFrameProducts>>,
    world: Option<Res<'w, WorldDrawGpuPlan>>,
    smodel: Option<Res<'w, SmodelGpuPlan>>,
    smc: Option<Res<'w, WorldStaticModelCache>>,
    retained: Option<Res<'w, StaticDrawLane>>,
    world_generation: Option<Res<'w, WorldGeneration>>,
    world_products: Option<Res<'w, WorldProducts>>,
    xmodel: Option<Res<'w, XModelDrawPlan>>,
    fx: Option<Res<'w, FxCodeMeshPlan>>,
    particle_cloud: Option<Res<'w, FxParticleCloudPlan>>,
    mark_mesh: Option<Res<'w, GfxMarkMeshPlan>>,
    images: Option<Res<'w, RuntimeImageHandles>>,
    submit: Option<Res<'w, GpuSubmitReady>>,
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
    generation: MaterialGenerationId,
    world_generation: WorldGeneration,
    world_products: WorldProducts,
    material: &'a MaterialFrameInputs,
    snapshot: Arc<FrameProductsSnapshot>,
}

impl<'a> Basis<'a> {
    /// None unless the materials, their frame inputs and the product list are
    /// all there and the assembly vouches that they are one frame.
    fn of(input: &'a SealInputs) -> Option<Self> {
        let generation = input.runtime.as_deref()?.catalog.generation_id;
        let material = input.material.as_deref()?;
        let snapshot = input.products.as_deref()?.published();
        let world_generation = input
            .world_generation
            .as_deref()
            .map_or(WorldGeneration(None), |generation| *generation);
        let world_products = input.world_products.as_deref().copied().unwrap_or_default();
        let assembled = input.assembly.as_deref().is_some_and(|assembly| {
            assembly.frame_id == snapshot.frame_id
                && assembly.catalog_generation == generation
                && assembly.world_generation == world_generation
        });
        let current = snapshot.products.iter().all(|product| {
            matches!(product.status, FrameProductStatus::Missing(_))
                || product.generation_id == generation
        });
        (assembled && current).then_some(Self {
            generation,
            world_generation,
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
    world_vertices: usize,
    world_indices: usize,
    world_layers: usize,
    smodel_vertices: usize,
    smodel_indices: usize,
}

impl Shape {
    fn installed(installed: &InstalledRenderWorld) -> Self {
        let rows = &installed.static_geometry;
        Self {
            world_vertices: rows.world_vertices.len(),
            world_indices: rows.world_indices.len(),
            world_layers: rows.world_layer.len(),
            smodel_vertices: rows.smodel_vertices.len(),
            smodel_indices: rows.smodel_indices.len(),
        }
    }

    /// A plan whose vertices are refused counts as empty.
    fn planned(world: Option<&WorldDrawGpuPlan>, smodel: Option<&SmodelGpuPlan>) -> Self {
        let (world_vertices, world_indices, world_layers) = world
            .filter(|plan| plan.exact_packed_vertices().is_ok())
            .map_or((0, 0, 0), |plan| {
                (
                    plan.exact_packed_vertices().map_or(0, <[_]>::len),
                    plan.indices().len(),
                    plan.vertex_layer_rows().len(),
                )
            });
        let (smodel_vertices, smodel_indices) = smodel
            .filter(|plan| plan.exact_packed_vertices().is_ok())
            .map_or((0, 0), |plan| {
                (
                    plan.exact_packed_vertices().map_or(0, <[_]>::len),
                    plan.indices().len(),
                )
            });
        Self {
            world_vertices,
            world_indices,
            world_layers,
            smodel_vertices,
            smodel_indices,
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
    fn decide(input: &SealInputs, basis: &Basis, smc_revision: Option<u64>) -> Self {
        let Some(installed) = input.installed.as_deref() else {
            return Self {
                statics: false,
                smc_maps: false,
            };
        };
        let same_world = installed.world_generation == basis.world_generation
            || installed.world_products.same_as(basis.world_products);
        let statics = same_world
            && Shape::installed(installed)
                == Shape::planned(input.world.as_deref(), input.smodel.as_deref());
        Self {
            statics,
            smc_maps: statics && installed.smc_revision == smc_revision,
        }
    }
}

fn fill_world(plan: Option<&WorldDrawGpuPlan>, rows: &mut ExtractedStaticGeometry) {
    let Some(plan) = plan else { return };
    match plan.exact_packed_vertices() {
        Ok(_) => {
            rows.world_vertices = share(plan.vertex_share.as_ref());
            rows.world_layer = share(plan.layer_share.as_ref());
            rows.world_indices = share(plan.index_share.as_ref());
            rows.world_surface_ranges = share(plan.range_share.as_ref());
        }
        Err(cause) => rows.world_vertex_refusal = Some(cause),
    }
}

fn fill_smodel(plan: Option<&SmodelGpuPlan>, rows: &mut ExtractedStaticGeometry) {
    let Some(plan) = plan else { return };
    match plan.exact_packed_vertices() {
        Ok(_) => {
            rows.smodel_vertices = share(plan.packed_share.as_ref());
            rows.smodel_indices = share(plan.index_share.as_ref());
            rows.smodel_surface_ranges = share(plan.range_share.as_ref());
            rows.smodel_cached_vertices = share(plan.cached_share.as_ref());
            rows.smodel_surface_verts = share(plan.vert_range_share.as_ref());
        }
        Err(cause) => rows.smodel_vertex_refusal = Some(cause),
    }
}

fn fill_xmodel(plan: Option<&XModelDrawPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.xmodel_revision = plan.revision;
    out.xmodel_topology_revision = plan.topology_revision;
    out.xmodel_packed_segments = plan.packed_segments;
    match plan.exact_packed_vertices() {
        Ok(_) => {
            out.xmodel_vertices = share(plan.packed_share.as_ref());
            out.xmodel_indices = share(plan.index_share.as_ref());
            out.xmodel_surface_ranges = share(plan.range_share.as_ref());
        }
        Err(cause) => out.xmodel_vertex_refusal = Some(cause),
    }
}

fn fill_fx(plan: Option<&FxCodeMeshPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.fx_revision = plan.revision;
    match plan.exact_packed_vertices() {
        Ok(_) => {
            out.fx_vertices = Arc::clone(&plan.vertices);
            out.fx_indices = Arc::clone(&plan.indices);
            out.fx_surface_ranges = share(plan.range_share.as_ref());
        }
        Err(cause) => out.fx_vertex_refusal = Some(cause),
    }
}

fn fill_particle_cloud(plan: Option<&FxParticleCloudPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.particle_cloud_vertices = Arc::clone(&plan.vertices);
    out.particle_cloud_indices = Arc::clone(&plan.indices);
    out.particle_cloud_surface_ranges = share(plan.range_share.as_ref());
    out.particle_cloud_template = plan.template_counts();
    out.particle_cloud_revision = plan.revision;
}

fn fill_mark_mesh(plan: Option<&GfxMarkMeshPlan>, out: &mut RenderFrameData) {
    let Some(plan) = plan else { return };
    out.mark_mesh_vertices = Arc::clone(&plan.vertices);
    out.mark_mesh_indices = Arc::clone(&plan.indices);
    out.mark_mesh_surface_ranges = share(plan.range_share.as_ref());
    out.mark_mesh_surface_packed = Arc::new(plan.draws.iter().map(|draw| draw.sub_key.packed).collect());
    out.mark_mesh_revision = plan.revision;
}

/// This frame's draw data, from the plans the frontend assembled.
fn frame_data(input: &SealInputs, basis: Basis, smc: Option<&WorldStaticModelCache>) -> RenderFrameData {
    let submit = input.submit.as_deref();
    let mut exec_frame = input
        .published
        .as_deref()
        .map(|published| published.exec_frame.clone())
        .unwrap_or_default();
    material_exec::refresh(&mut exec_frame, basis.material, input.assembly.as_deref());
    let mut out = RenderFrameData {
        frame_products: ExtractedRenderFrameProducts(basis.snapshot),
        generation: basis.generation,
        world_generation: basis.world_generation,
        exec_frame,
        sun_shadow: basis.material.sun_shadow,
        sun_effects: input.sun_frame.as_deref().and_then(|sun| sun.frame),
        warm_pipelines: submit.is_some_and(|job| job.warm_pipelines),
        pipeline_world_materials: submit
            .map(|job| job.pipeline_world_materials.clone())
            .unwrap_or_default(),
        pipeline_smodel_materials: submit
            .map(|job| job.pipeline_smodel_materials.clone())
            .unwrap_or_default(),
        pipeline_demand_revision: submit.map_or(0, |job| job.pipeline_demand_revision),
        smc_vb_patches: smc.map(|cache| cache.vb_patches().to_vec()).unwrap_or_default(),
        smc_ib_patches: smc.map(|cache| cache.ib_patches().to_vec()).unwrap_or_default(),
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
    let smc_revision = smc.map(|cache| cache.content_revision());
    let kept = Kept::decide(&input, &basis, smc_revision);
    let installed = input.installed.as_deref();

    let static_geometry = match installed.filter(|_| kept.statics) {
        Some(installed) => installed.static_geometry.clone(),
        None => {
            let mut rows = ExtractedStaticGeometry::default();
            fill_world(input.world.as_deref(), &mut rows);
            fill_smodel(input.smodel.as_deref(), &mut rows);
            Arc::new(rows)
        }
    };
    let smc_index_baked = match installed.filter(|_| kept.smc_maps) {
        Some(installed) => installed.smc_index_baked.clone(),
        None => Arc::new(smc.map(|cache| cache.baked_cache_indices()).unwrap_or_default()),
    };
    let retained = input.retained.as_deref();
    let world = InstalledRenderWorld::new(RenderWorldData {
        generation: basis.generation,
        world_generation: basis.world_generation,
        world_products: basis.world_products,
        smc_revision,
        static_geometry,
        smc_index_baked,
        smodel_pretess_indices: retained.map_or_else(Arc::default, |lane| {
            Arc::clone(&lane.smodel_pretess_indices)
        }),
        smodel_index_layout_revision: retained.map_or(0, |lane| lane.smodel_index_layout_revision),
        image_handles: input.images.as_deref().cloned().unwrap_or_default(),
        sun_effects: input.sun_map.as_deref().and_then(|sun| sun.def),
    });
    let data = frame_data(&input, basis, smc);
    commands.insert_resource(world.clone());
    commands.insert_resource(PublishedRenderFrame::seal(world, data));
}
