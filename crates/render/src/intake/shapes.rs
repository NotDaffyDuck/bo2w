//! The diagnostic geometry copy: the plans' decoded rows, ranges and lighting
//! tables the fallback draw reads, kept between frames and refreshed only where
//! they changed.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::Extract;
use render_frontend::assemble::draw_items::mesh_build::prop_mesh::PropGpuPlan;
use render_frontend::assemble::draw_items::mesh_build::world::{WorldGpuSubmission, t6_probe_slot, t6_sky_slot};
use render_frontend::assemble::draw_items::mesh_build::figure_mesh::FigureDrawPlan;
use render_frontend::assemble::draw_items::{MapDirKeyLight, MapT5DaylightParseExposure, MaterialEpoch};
use render_material::MaterialEpochId;
use render_frontend::prepare::scene::cull::SightFrameStats;
use render_frontend::prepare::scene::world::LevelStage;
use render_gpu::{ExtractedInspectGeometry, SubmitReadyGpu};

use super::shares::decoded_shares;

type Lighting = [[f32; 4]; 4];
type PropsKey = (MaterialEpochId, usize, u32);
type Props = (Arc<Vec<[u32; 4]>>, Arc<Vec<[f32; 28]>>, Arc<Vec<[u32; 3]>>);

/// The world lighting the fallback draw adds to lightmaps: the sun's
/// direction and exposure scale, then its colour, the sky's rotation and
/// brightness, then its decode and fog share. The colour grade is the final
/// image's (`bo2_bloom`). A T6 world fills the T5 sun and exposure carriers;
/// only T6 surfaces read this.
fn sun_rows(
    sun: Option<&MapDirKeyLight>,
    exposure: Option<&MapT5DaylightParseExposure>,
    sky: Option<[f32; 4]>,
    sky_parms: Option<[f32; 2]>,
    [sun_r, sun_g, sun_b, sky_scale]: [f32; 4],
) -> Option<Lighting> {
    let (sun, exposure) = (sun?, exposure?);
    let [step_along_x, dy, dz] = sun.direction;
    let [r, g, b] = sun.color;
    Some([
        [step_along_x, dy, dz, exposure.exposure],
        [r * sun_r, g * sun_g, b * sun_b, exposure.saturation],
        // The sky's brightness (w, its sign the transition) times the sky intensity (`skyColorMultiplier`).
        sky.map_or([1.0, 0.0, 0.0, 1.0], |[x, y, z, w]| [x, y, z, w * sky_scale]),
        sky_parms.map_or([1.0, 1.0, 0.0, 0.0], |[x, w]| [x, w, 0.0, 0.0]),
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
    runtime: &'a MaterialEpoch,
    world: Option<&'a WorldGpuSubmission>,
    smodel: Option<&'a PropGpuPlan>,
    xmodel: Option<&'a FigureDrawPlan>,
    scene: Option<&'a LevelStage>,
    g0_level_facets: Vec<u16>,
    sun: Option<Lighting>,
    props_key: PropsKey,
    props: Props,
}

impl Sources<'_> {
    /// Brings every row that follows the plans up to date.
    fn settle(&self, rows: &mut ExtractedInspectGeometry) {
        let catalog = &self.runtime.catalog;
        let (_, _, level_ranges) = decoded_shares!(self.world);
        let (_, _, prop_ranges) = decoded_shares!(self.smodel);
        let (xmodel_vertices, figure_indices, figure_ranges) = decoded_shares!(self.xmodel);
        rows.world_surface_ranges = level_ranges;
        rows.smodel_surface_ranges = prop_ranges;
        rows.xmodel_vertices = xmodel_vertices;
        rows.figure_indices = figure_indices;
        rows.figure_facet_ranges = figure_ranges;
        rows.figure_edition = self.xmodel.map_or(0, |figure| figure.revision);
        rows.g0_level_facets = self.g0_level_facets.clone();

        let (surface_colors, surface_draws, surface_lightmaps) = self
            .world
            .map(WorldGpuSubmission::fallback_surface_styles)
            .unwrap_or_default();
        refresh(&mut rows.world_surface_color, surface_colors);
        refresh(&mut rows.world_surface_draw, surface_draws);
        refresh(&mut rows.world_surface_lightmap, surface_lightmaps);
        refresh(
            &mut rows.world_surface_layers,
            self.world.map_or_else(Vec::new, WorldGpuSubmission::fallback_surface_layers),
        );
        refresh(
            &mut rows.world_surface_shine,
            self.world.map_or_else(Vec::new, WorldGpuSubmission::fallback_surface_shine),
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
fn counts(world: Option<&WorldGpuSubmission>, smodel: Option<&PropGpuPlan>) -> [usize; 4] {
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
    held: Option<&ExtractedInspectGeometry>,
    key: PropsKey,
    runtime: &MaterialEpoch,
    smodel: Option<&PropGpuPlan>,
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

pub fn pull_geometry_to_render_world(
    mut commands: Commands,
    level_plan: Extract<Option<Res<WorldGpuSubmission>>>,
    smodel: Extract<Option<Res<PropGpuPlan>>>,
    xmodel: Extract<Option<Res<FigureDrawPlan>>>,
    runtime: Extract<Option<Res<MaterialEpoch>>>,
    dpvs: Extract<Option<Res<SightFrameStats>>>,
    spawn_job: Extract<Option<Res<SubmitReadyGpu>>>,
    held: Option<ResMut<ExtractedInspectGeometry>>,
    lighting: (
        Extract<Option<Res<MapDirKeyLight>>>,
        Extract<Option<Res<MapT5DaylightParseExposure>>>,
        Extract<Option<Res<LevelStage>>>,
    ),
) {
    if !render_gpu::geometry_inspect_enabled() {
        return;
    }
    let Some(runtime) = runtime.as_deref() else {
        return;
    };
    let (world, smodel, xmodel) = (level_plan.as_deref(), smodel.as_deref(), xmodel.as_deref());
    let scene = lighting.2.as_deref();
    let sun = sun_rows(
        lighting.0.as_deref(),
        lighting.1.as_deref(),
        scene.and_then(|scene| scene.classic_sky_dynamic_intensity),
        scene.and_then(|scene| scene.t6_sky_parms),
        scene.map_or([1.0; 4], |scene| scene.t6_sun_sky_scale),
    );
    let stamp = runtime.catalog.revision_stamp;
    // The exposure too: the props carry it baked in, and a T6 map changes it per exposure volume (in 1/16 stop
    // steps).
    let props_key = (
        stamp,
        smodel.map_or(0, |plan| plan.placements.len()),
        sun.map_or(0, |rows| rows[0][3].to_bits()),
    );
    let sources = Sources {
        runtime,
        world,
        smodel,
        xmodel,
        scene,
        g0_level_facets: dpvs
            .as_deref()
            .map(|stats| stats.g0_level_facets.clone())
            .unwrap_or_default(),
        sun,
        props_key,
        props: props_for(held.as_deref(), props_key, runtime, smodel, sun),
    };

    let waiting = spawn_job.as_deref().is_some_and(|job| job.topmost_gpu_wait);
    let current = counts(world, smodel);
    let reusable = waiting
        || held.as_deref().is_some_and(|rows| {
            rows.generation == stamp
                && [
                    rows.level_vertex_total.len(),
                    rows.level_indices.len(),
                    rows.static_model_verts_used.len(),
                    rows.static_model_slots_used.len(),
                ] == current
        });
    if reusable {
        let Some(mut rows) = held else { return };
        rows.topmost_gpu_wait = waiting;
        if !waiting {
            sources.settle(&mut rows);
        }
        return;
    }

    let (level_vertex_total, level_indices, _) = decoded_shares!(world);
    let (static_model_verts_used, static_model_slots_used, _) = decoded_shares!(smodel);
    let mut next = ExtractedInspectGeometry {
        generation: stamp,
        topmost_gpu_wait: false,
        level_vertex_total,
        level_indices,
        static_model_verts_used,
        static_model_slots_used,
        ..Default::default()
    };
    sources.settle(&mut next);
    match held {
        Some(mut rows) => *rows = next,
        None => commands.insert_resource(next),
    }
}
