//! The diagnostic geometry copy: the plans' decoded rows, ranges and lighting
//! tables the fallback draw reads, kept between frames and refreshed only where
//! they changed.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::Extract;
use render_frontend::assemble::drawsurf::tess::smodel::SmodelGpuPlan;
use render_frontend::assemble::drawsurf::tess::world::{WorldDrawGpuPlan, t6_probe_slot, t6_sky_slot};
use render_frontend::assemble::drawsurf::tess::xmodel::XModelDrawPlan;
use render_frontend::assemble::drawsurf::{MapDirPrimaryLight, MapT5SunParseExposure, MaterialGeneration};
use render_material::MaterialGenerationId;
use render_frontend::prepare::scene::cull::DpvsFrameStats;
use render_frontend::prepare::scene::world::WorldScene;
use render_gpu::{ExtractedDiagnosticGeometry, GpuSubmitReady};

use super::shares::decoded_shares;

type Lighting = [[f32; 4]; 3];
type PropsKey = (MaterialGenerationId, usize);
type Props = (Arc<Vec<[u32; 4]>>, Arc<Vec<[f32; 28]>>, Arc<Vec<[u32; 3]>>);

/// The world lighting the fallback draw adds to lightmaps: the sun's
/// direction and exposure scale, then its colour, then the sky.
fn sun_rows(
    sun: Option<&MapDirPrimaryLight>,
    exposure: Option<&MapT5SunParseExposure>,
    sky: Option<[f32; 4]>,
) -> Option<Lighting> {
    let (sun, exposure) = (sun?, exposure?);
    let [dx, dy, dz] = sun.direction;
    let [r, g, b] = sun.color;
    Some([
        [dx, dy, dz, exposure.exposure],
        [r, g, b, 1.0],
        sky.unwrap_or([1.0, 0.0, 0.0, 1.0]),
    ])
}

/// Replaces the rows only when the new ones differ, so unchanged tables keep
/// their share.
fn refresh<T: PartialEq>(slot: &mut Arc<Vec<T>>, next: Vec<T>) {
    if **slot != next {
        *slot = Arc::new(next);
    }
}

/// Takes the plan's share when it is a different one from the held share.
fn adopt<T: ?Sized>(slot: &mut Arc<T>, next: &Arc<T>) {
    if !Arc::ptr_eq(slot, next) {
        *slot = Arc::clone(next);
    }
}

/// What the rows below are cut from this frame.
struct Sources<'a> {
    runtime: &'a MaterialGeneration,
    world: Option<&'a WorldDrawGpuPlan>,
    smodel: Option<&'a SmodelGpuPlan>,
    xmodel: Option<&'a XModelDrawPlan>,
    scene: Option<&'a WorldScene>,
    g0_world_surfs: Vec<u16>,
    sun: Option<Lighting>,
    props_key: PropsKey,
    props: Props,
}

impl Sources<'_> {
    /// Brings every row that follows the plans up to date.
    fn settle(&self, rows: &mut ExtractedDiagnosticGeometry) {
        let catalog = &self.runtime.catalog;
        let (_, _, world_ranges) = decoded_shares!(self.world);
        let (_, _, smodel_ranges) = decoded_shares!(self.smodel);
        let (xmodel_vertices, xmodel_indices, xmodel_ranges) = decoded_shares!(self.xmodel);
        rows.world_surface_ranges = world_ranges;
        rows.smodel_surface_ranges = smodel_ranges;
        rows.xmodel_vertices = xmodel_vertices;
        rows.xmodel_indices = xmodel_indices;
        rows.xmodel_surface_ranges = xmodel_ranges;
        rows.xmodel_revision = self.xmodel.map_or(0, |plan| plan.revision);
        rows.g0_world_surfs = self.g0_world_surfs.clone();

        let (colors, draws, lightmaps) = self
            .world
            .map(WorldDrawGpuPlan::fallback_surface_styles)
            .unwrap_or_default();
        refresh(&mut rows.world_surface_color, colors);
        refresh(&mut rows.world_surface_draw, draws);
        refresh(&mut rows.world_surface_lightmap, lightmaps);
        refresh(
            &mut rows.world_surface_layers,
            self.world.map_or_else(Vec::new, WorldDrawGpuPlan::fallback_surface_layers),
        );
        refresh(
            &mut rows.world_surface_shine,
            self.world.map_or_else(Vec::new, WorldDrawGpuPlan::fallback_surface_shine),
        );
        refresh(
            &mut rows.world_surface_primary,
            self.world.map_or_else(Vec::new, |plan| plan.surface_primary_light.clone()),
        );
        if let Some(plan) = self.world {
            adopt(&mut rows.world_layer_uvs, &plan.t6_layer_share);
        }
        if let Some(scene) = self.scene {
            adopt(&mut rows.t6_probes, &scene.t6_probes);
            adopt(&mut rows.t6_lights, &scene.t6_lights);
        }
        rows.t6_probe_slot = t6_probe_slot(catalog);
        rows.t6_sky_slot = t6_sky_slot(catalog);
        rows.t6_lighting = self.sun;
        rows.t6_sun_sample_size_near = self.scene.map_or(0.0, |scene| scene.t6_sun_sample_size_near);
        rows.t6_props = Arc::clone(&self.props.0);
        rows.t6_prop_instances = Arc::clone(&self.props.1);
        rows.t6_prop_shine = Arc::clone(&self.props.2);
        rows.t6_props_key = Some(self.props_key);
    }
}

/// Vertex and index counts of the world and static model plans.
fn counts(world: Option<&WorldDrawGpuPlan>, smodel: Option<&SmodelGpuPlan>) -> [usize; 4] {
    [
        world.map_or(0, |plan| plan.decoded_vertices().len()),
        world.map_or(0, |plan| plan.indices().len()),
        smodel.map_or(0, |plan| plan.decoded_vertices().len()),
        smodel.map_or(0, |plan| plan.indices().len()),
    ]
}

/// The BO2 props, kept while the material generation and the placements stay
/// the same, otherwise rebuilt.
fn props_for(
    held: Option<&ExtractedDiagnosticGeometry>,
    key: PropsKey,
    runtime: &MaterialGeneration,
    smodel: Option<&SmodelGpuPlan>,
    sun: Option<Lighting>,
) -> Props {
    if let Some(held) = held.filter(|held| held.t6_props_key == Some(key)) {
        return (
            Arc::clone(&held.t6_props),
            Arc::clone(&held.t6_prop_instances),
            Arc::clone(&held.t6_prop_shine),
        );
    }
    let exposure = sun.map_or(1.0, |rows| rows[0][3]);
    let (draws, instances, shine) = smodel
        .map(|plan| plan.t6_fallback_props(&runtime.catalog, exposure))
        .unwrap_or_default();
    (Arc::new(draws), Arc::new(instances), Arc::new(shine))
}

pub fn extract_geometry(
    mut commands: Commands,
    world: Extract<Option<Res<WorldDrawGpuPlan>>>,
    smodel: Extract<Option<Res<SmodelGpuPlan>>>,
    xmodel: Extract<Option<Res<XModelDrawPlan>>>,
    runtime: Extract<Option<Res<MaterialGeneration>>>,
    dpvs: Extract<Option<Res<DpvsFrameStats>>>,
    spawn_job: Extract<Option<Res<GpuSubmitReady>>>,
    held: Option<ResMut<ExtractedDiagnosticGeometry>>,
    lighting: (
        Extract<Option<Res<MapDirPrimaryLight>>>,
        Extract<Option<Res<MapT5SunParseExposure>>>,
        Extract<Option<Res<WorldScene>>>,
    ),
) {
    if !render_gpu::geometry_diagnostic_enabled() {
        return;
    }
    let Some(runtime) = runtime.as_deref() else {
        return;
    };
    let (world, smodel, xmodel) = (world.as_deref(), smodel.as_deref(), xmodel.as_deref());
    let scene = lighting.2.as_deref();
    let sun = sun_rows(
        lighting.0.as_deref(),
        lighting.1.as_deref(),
        scene.and_then(|scene| scene.t5_sky_dynamic_intensity),
    );
    let generation = runtime.catalog.generation_id;
    let props_key = (generation, smodel.map_or(0, |plan| plan.placements.len()));
    let sources = Sources {
        runtime,
        world,
        smodel,
        xmodel,
        scene,
        g0_world_surfs: dpvs
            .as_deref()
            .map(|stats| stats.g0_world_surfs.clone())
            .unwrap_or_default(),
        sun,
        props_key,
        props: props_for(held.as_deref(), props_key, runtime, smodel, sun),
    };

    let waiting = spawn_job.as_deref().is_some_and(|job| job.overlay_gpu_wait);
    let current = counts(world, smodel);
    let reusable = waiting
        || held.as_deref().is_some_and(|rows| {
            rows.generation == generation
                && [
                    rows.world_vertices.len(),
                    rows.world_indices.len(),
                    rows.smodel_vertices.len(),
                    rows.smodel_indices.len(),
                ] == current
        });
    if reusable {
        let Some(mut rows) = held else { return };
        rows.overlay_gpu_wait = waiting;
        if !waiting {
            sources.settle(&mut rows);
        }
        return;
    }

    let (world_vertices, world_indices, _) = decoded_shares!(world);
    let (smodel_vertices, smodel_indices, _) = decoded_shares!(smodel);
    let mut next = ExtractedDiagnosticGeometry {
        generation,
        overlay_gpu_wait: false,
        world_vertices,
        world_indices,
        smodel_vertices,
        smodel_indices,
        ..Default::default()
    };
    sources.settle(&mut next);
    match held {
        Some(mut rows) => *rows = next,
        None => commands.insert_resource(next),
    }
}
