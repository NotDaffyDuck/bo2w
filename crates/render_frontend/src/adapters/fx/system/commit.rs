//! Closing an effects frame: what `generate_fx_transaction` produced becomes
//! this frame's draw plans, lights and fallback-pass meshes.
//!
//! Every kind of product has a stage of its own below, and
//! `commit_fx_transaction` fixes their order. Marks are planned before
//! models, and the tracer beams are appended after the sprites and trails
//! so that they land in the same plan.

use bevy::prelude::*;
use fx::{
    CodeMeshStep, FxElemLightInstance, FxGapCause, FxModelInstance, FxSpriteInstance,
    FxSystemHost, FxTrailMeshInstance,
};
use fx_t6::FxPostLight;
use render_fx::{
    CombatFxDump, FxDumpRequest, FxFrameOutcome, FxGeneratedFrame, FxJournalCursor,
    FxUnavailableCause, FxWorldColorImages, HostFxDlights, HostFxPostLights, HostFxSystem,
    PreparedFxCatalog, TracerWorld, clear_fx_owned_plans, publish_empty_fx_owned_plans,
};

use super::dump::{self, FrameTally};
use super::mark_plan::fill_mark_mesh_plan;
use super::mesh_bind::{
    FxCodeMeshBind, FxPresentSkip, code_mesh_bind, code_mesh_bind_asset, count_fx_present_skip,
    spark_elem_asset_id,
};
use super::model_plan::fill_fx_model_plan;
use super::sprite_runs::{batch_by_material, sprite_transform};
use super::{
    FxFrameTransaction, FxPresentEnv, T6_DYN_LIGHTS, T6_FX_ELEM_DRAW_WITH_VIEWMODEL, T6DynLights,
    T6FxMesh, T6MarkMesh, t6_fx_filtered_out, t6_fx_material, take_t6_marks,
};
use crate::adapters::fx::tracer::present_tracer_beams;
use crate::assemble::drawsurf::{
    FxCodeMeshPlan, FxModelDrawPlan, FxParticleCloudPlan, GfxMarkMeshPlan, MaterialGeneration,
};
use crate::prepare::scene::{
    model_lighting_cache::{ModelLightingRequests, WorldModelLightingCache},
    world::WorldScene,
};

/// The material tables the draw stages look their materials up in.
struct Binder<'a> {
    colors: &'a FxWorldColorImages,
    runtime: &'a MaterialGeneration,
}

impl Binder<'_> {
    fn by_name(&self, name: &str) -> FxCodeMeshBind {
        code_mesh_bind(name, self.colors, self.runtime)
    }

    fn by_asset(&self, asset_id: usize) -> FxCodeMeshBind {
        code_mesh_bind_asset(asset_id, self.colors, self.runtime)
    }

    /// Black Ops II effect materials have no technique the code mesh could
    /// draw with; the renderer's fallback pass draws them from the quads
    /// collected in `T6FxMesh`.
    fn fallback_pass(&self, asset_id: usize) -> bool {
        t6_fx_material(self.runtime, asset_id)
    }

    /// Switched off by the `BO2MP_T6_FX_ONLY` / `_SKIP` filters.
    fn hidden(&self, asset_id: usize) -> bool {
        t6_fx_filtered_out(self.runtime, asset_id)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_fx_transaction(
    In(transaction): In<FxFrameTransaction>,
    mut host: ResMut<HostFxSystem>,
    catalog: Option<Res<PreparedFxCatalog>>,
    color_images: Res<FxWorldColorImages>,
    runtime: Res<MaterialGeneration>,
    scene: Option<Res<WorldScene>>,
    mut cursor: ResMut<FxJournalCursor>,
    (mut plan, mut t6_fx, mut t6_marks, mut t6_lights): (
        ResMut<FxCodeMeshPlan>,
        ResMut<T6FxMesh>,
        ResMut<T6MarkMesh>,
        ResMut<T6DynLights>,
    ),
    mut spark_plan: ResMut<FxParticleCloudPlan>,
    mut mark_plan: ResMut<GfxMarkMeshPlan>,
    mut model_plan: ResMut<FxModelDrawPlan>,
    mut dump_req: ResMut<FxDumpRequest>,
    mut tracer_world: ResMut<TracerWorld>,
    mut combat: ResMut<CombatFxDump>,
    mut env: FxPresentEnv,
    lighting_cache: Option<Res<WorldModelLightingCache>>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
) {
    let FxFrameTransaction {
        outcome,
        _present_span,
    } = transaction;
    reset_frame(&mut plan, &mut spark_plan, &mut mark_plan, &mut t6_fx, &mut env);

    let frame = match outcome {
        FxFrameOutcome::Generated(frame) => frame,
        FxFrameOutcome::Unavailable(FxUnavailableCause::CatalogAbsent) => {
            host.0.last_mark_mesh = None;
            publish_nothing(&mut host.0, &mut plan, &mut spark_plan, &mut mark_plan, &runtime);
            return;
        }
    };
    let FxGeneratedFrame {
        out,
        verts_gaps,
        mark_mesh,
        post_lights,
        mut tracers,
        cam_tf,
        frustum_planes,
        clip_from_world,
        tan_half_fov,
        world_present,
    } = frame;
    let Some(catalog) = catalog else {
        *tracer_world = tracers;
        publish_nothing(&mut host.0, &mut plan, &mut spark_plan, &mut mark_plan, &runtime);
        return;
    };
    let binder = Binder {
        colors: &color_images,
        runtime: &runtime,
    };

    take_t6_marks(&host.0, mark_mesh.as_ref(), &runtime, &mut t6_marks);
    fill_mark_mesh_plan(&mut host.0, mark_mesh, &mut mark_plan, &runtime);

    let model_catalog = env.models.as_deref().map(|prepared| &prepared.0);
    let plain_models = route_models(&out.models, model_catalog, &runtime, &mut t6_fx);
    fill_fx_model_plan(
        &plain_models,
        model_catalog,
        env.atlas.as_deref(),
        scene.as_deref(),
        &env.atpoint,
        cam_tf.translation,
        &frustum_planes,
        env.lod_skinned.args(),
        lighting_cache.as_deref(),
        &mut lighting_requests,
        &mut env.staged_models.0,
    );
    model_plan.publish_rebuild(&mut env.staged_models.0);

    stage_scene_lights(&out.spot_lights, &out.omni_lights, world_present, &mut env.dlights);
    t6_lights.lights.clear();
    t6_lights
        .lights
        .extend(nearest_omni_lights(&out.omni_lights, cam_tf.translation, T6_DYN_LIGHTS));
    draw_post_lights(
        &post_lights,
        cam_tf.translation,
        &binder,
        &mut plan,
        &mut cursor,
        &mut env.post_lights,
    );

    let sprites_drawn = draw_sprites(
        &out.sprites,
        &cam_tf,
        &binder,
        &mut plan,
        &mut t6_fx,
        &mut cursor,
    );
    let mut trails_drawn = 0usize;
    for mesh in &out.trail_meshes {
        if draw_trail(mesh, &binder, &mut plan, &mut host.0, &mut cursor) {
            trails_drawn += 1;
        }
    }
    present_tracer_beams(
        &mut tracers,
        &cam_tf,
        clip_from_world,
        tan_half_fov,
        &color_images,
        &runtime,
        &mut plan,
        &mut combat,
        code_mesh_bind_asset,
        &mut t6_fx,
        |asset_id| binder.fallback_pass(asset_id) && !binder.hidden(asset_id),
    );
    *tracer_world = tracers;
    draw_clouds(
        &out.clouds,
        &catalog.0,
        &binder,
        &mut spark_plan,
        &mut t6_fx,
        &mut cursor,
    );

    spark_plan.bump();
    spark_plan.publish_share();
    plan.bump();
    plan.publish_share();

    if dump_req.pending {
        dump::write_fx_dump(&dump::DumpInput {
            host: &host.0,
            out: &out,
            verts_gaps: &verts_gaps,
            tally: FrameTally {
                sprites_drawn,
                trails_drawn,
            },
            camera: &cam_tf,
            radius: dump_req.radius,
            model_plan: &model_plan,
            plan: &plan,
            spark_plan: &spark_plan,
            dlights: &env.dlights,
            post_lights: &env.post_lights,
            cursor: &cursor,
            combat: &combat,
        });
        dump_req.pending = false;
    }
}

/// Empties everything this frame will refill.
fn reset_frame(
    plan: &mut FxCodeMeshPlan,
    spark_plan: &mut FxParticleCloudPlan,
    mark_plan: &mut GfxMarkMeshPlan,
    t6_fx: &mut T6FxMesh,
    env: &mut FxPresentEnv,
) {
    clear_fx_owned_plans(plan, spark_plan, mark_plan);
    t6_fx.clear();

    env.staged_models.0.clear();
    match &env.model_geometry {
        Some(prepared) => env.staged_models.0.use_prepared(&prepared.0),
        None => env.staged_models.0 = FxModelDrawPlan::default(),
    }

    env.dlights.scene.clear();
    env.dlights.cap_full = 0;
    let post = &mut *env.post_lights;
    post.queued.clear();
    (post.cap_full, post.drawn, post.skipped_short, post.miss_material) = (0, 0, 0, 0);
}

/// A frame with nothing to draw: no marks, and the plans published empty.
fn publish_nothing(
    host: &mut FxSystemHost,
    plan: &mut FxCodeMeshPlan,
    spark_plan: &mut FxParticleCloudPlan,
    mark_plan: &mut GfxMarkMeshPlan,
    runtime: &MaterialGeneration,
) {
    fill_mark_mesh_plan(host, None, mark_plan, runtime);
    publish_empty_fx_owned_plans(plan, spark_plan);
}

/// Sends the effect models Black Ops II draws itself (shell casings,
/// debris) to the fallback pass, posed on the spot, and returns the rest
/// for the model plan.
fn route_models(
    models: &[FxModelInstance],
    catalog: Option<&asset_game::FxModelCatalog>,
    runtime: &MaterialGeneration,
    t6_fx: &mut T6FxMesh,
) -> Vec<FxModelInstance> {
    let mut rest = Vec::with_capacity(models.len());
    for instance in models {
        let entry = catalog.and_then(|models| models.get_at(instance.model_index));
        if let Some(entry) = entry {
            // `fx_decal_*` models are volumes Black Ops II projects blood
            // onto a body from; drawn as models they show as black blocks.
            if entry.skel.name.starts_with("fx_decal") {
                continue;
            }
            let fallback_material = |surface: usize| {
                entry
                    .material_index(surface)
                    .map(|index| index.order())
                    .filter(|&order| t6_fx_material(runtime, order))
            };
            if (0..entry.skel.surface_materials.len()).any(|s| fallback_material(s).is_some()) {
                t6_fx.push_model(
                    &entry.skel,
                    |surface| fallback_material(surface).map(|order| order as u32),
                    instance.origin,
                    instance.axis,
                    instance.scale,
                );
                continue;
            }
        }
        rest.push(instance.clone());
    }
    rest
}

/// Hands the effects' spot and omni lights to the scene, as many as the
/// scene admits.
fn stage_scene_lights(
    spots: &[FxElemLightInstance],
    omnis: &[FxElemLightInstance],
    world_present: bool,
    dlights: &mut HostFxDlights,
) {
    let cone = lighting_t6::SpotLightConeDvars::register_defaults();
    for light in spots {
        admit_scene_light(dlights, world_present, light.radius, || {
            lighting_t6::spot_light_pack(
                light.origin,
                light.axis[0],
                light.radius,
                light.color_bgr,
                cone,
            )
        });
    }
    for light in omnis {
        admit_scene_light(dlights, world_present, light.radius, || {
            lighting_t6::omni_light_pack(light.origin, light.radius, light.color_bgr)
        });
    }
}

fn admit_scene_light(
    dlights: &mut HostFxDlights,
    world_present: bool,
    radius: f32,
    pack: impl FnOnce() -> lighting_t6::GfxLightPack,
) {
    let live = dlights.scene.len() as u32;
    match lighting_t6::add_omni_light_to_scene_allows(world_present, radius, live) {
        Ok(()) => dlights.scene.push(pack()),
        Err(lighting_t6::AddOmniLightRefuse::Cap) => {
            dlights.cap_full = dlights.cap_full.saturating_add(1);
        }
        Err(_) => {}
    }
}

/// The omni lights for the fallback pass as `[x, y, z, radius, r, g, b, 0]`
/// rows, the ones whose edge is nearest the eye first, at most `limit`.
fn nearest_omni_lights(lights: &[FxElemLightInstance], eye: Vec3, limit: usize) -> Vec<[f32; 8]> {
    let mut lit: Vec<(f32, [f32; 8])> = lights
        .iter()
        .filter(|light| light.radius > 0.0 && light.color_bgr.iter().any(|&c| c > 0.0))
        .map(|light| {
            let edge = Vec3::from_array(light.origin).distance(eye) - light.radius;
            let [b, g, r] = light.color_bgr;
            let [x, y, z] = light.origin;
            (edge, [x, y, z, light.radius, r, g, b, 0.0])
        })
        .collect();
    lit.sort_by(|a, b| a.0.total_cmp(&b.0));
    lit.into_iter().take(limit).map(|(_, row)| row).collect()
}

fn draw_post_lights(
    lights: &[FxPostLight],
    eye: Vec3,
    binder: &Binder,
    plan: &mut FxCodeMeshPlan,
    cursor: &mut FxJournalCursor,
    stats: &mut HostFxPostLights,
) {
    for light in lights {
        let Some(tess) = fx_t6::post_light_generate_verts(light, eye.to_array()) else {
            stats.skipped_short = stats.skipped_short.saturating_add(1);
            continue;
        };
        match binder.by_name(light.material_name) {
            FxCodeMeshBind::Skip(FxPresentSkip::NoColorMap) => {
                stats.miss_material = stats.miss_material.saturating_add(1);
                plan.miss_material = plan.miss_material.saturating_add(1);
            }
            FxCodeMeshBind::Skip(cause) => {
                count_fx_present_skip(cursor, cause);
                stats.miss_material = stats.miss_material.saturating_add(1);
            }
            FxCodeMeshBind::Ready {
                color,
                sort_key,
                ordinal,
            } => {
                let draw = plan.begin_material_draw(color, sort_key, Some(ordinal));
                if plan.push_post_light(&tess) {
                    stats.drawn = stats.drawn.saturating_add(1);
                }
                plan.end_material_draw(draw);
            }
        }
    }
}

/// Draws the frame's sprites and returns how many made it into a plan.
fn draw_sprites(
    sprites: &[FxSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    plan: &mut FxCodeMeshPlan,
    t6_fx: &mut T6FxMesh,
    cursor: &mut FxJournalCursor,
) -> usize {
    let grouped = batch_by_material(
        sprites.iter().map(|sprite| (sprite.material_index, sprite)),
        |asset_id| binder.fallback_pass(asset_id),
    );
    cursor.draw_miss_material = cursor
        .draw_miss_material
        .saturating_add(grouped.without_material);
    plan.miss_material = plan.miss_material.saturating_add(grouped.without_material);

    let mut drawn = 0;
    for (asset_id, run) in &grouped.batches {
        drawn += if binder.fallback_pass(*asset_id) {
            draw_fallback_sprites(*asset_id, run, camera, binder, t6_fx)
        } else {
            draw_code_mesh_sprites(*asset_id, run, camera, binder, plan, cursor)
        };
    }
    drawn
}

/// One run of Black Ops II sprites for the fallback pass. The sprites that
/// draw with the viewmodel are a draw of their own after the others.
fn draw_fallback_sprites(
    asset_id: usize,
    run: &[&FxSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    t6_fx: &mut T6FxMesh,
) -> usize {
    if binder.hidden(asset_id) {
        return 0;
    }
    let mut drawn = 0;
    for viewmodel in [false, true] {
        let first = t6_fx.indices.len() as u32;
        for sprite in run
            .iter()
            .filter(|s| (s.flags & T6_FX_ELEM_DRAW_WITH_VIEWMODEL != 0) == viewmodel)
        {
            t6_fx.push_quad(sprite_transform(sprite, camera), sprite.color_rgba, sprite.atlas);
            drawn += 1;
        }
        let count = t6_fx.indices.len() as u32 - first;
        if count == 0 {
            continue;
        }
        let draw = (first, count, asset_id as u32);
        if viewmodel {
            t6_fx.viewmodel_draws.push(draw);
        } else {
            t6_fx.draws.push(draw);
        }
    }
    drawn
}

/// One run of sprites for the code mesh pass.
fn draw_code_mesh_sprites(
    asset_id: usize,
    run: &[&FxSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    plan: &mut FxCodeMeshPlan,
    cursor: &mut FxJournalCursor,
) -> usize {
    match binder.by_asset(asset_id) {
        FxCodeMeshBind::Skip(FxPresentSkip::NoColorMap) => {
            let missed = run.len() as u32;
            cursor.draw_miss_material = cursor.draw_miss_material.saturating_add(missed);
            plan.miss_material = plan.miss_material.saturating_add(missed);
            if cursor.draw_miss_material == missed {
                let name = run.first().map_or("?", |s| s.material_name.as_ref());
                diag::warn!(
                    World,
                    "fx: sprite material {asset_id} (`{name}`) has no colour image; further misses are only counted"
                );
            }
            0
        }
        FxCodeMeshBind::Skip(cause) => {
            count_fx_present_skip(cursor, cause);
            0
        }
        FxCodeMeshBind::Ready {
            color,
            sort_key,
            ordinal,
        } => {
            let draw = plan.begin_material_draw(color, sort_key, Some(ordinal));
            let drawn = run
                .iter()
                .filter(|s| plan.push_quad(sprite_transform(s, camera), s.color_rgba, s.atlas))
                .count();
            plan.end_material_draw(draw);
            drawn
        }
    }
}

/// Draws one trail mesh; true when it went into the plan.
fn draw_trail(
    mesh: &FxTrailMeshInstance,
    binder: &Binder,
    plan: &mut FxCodeMeshPlan,
    host: &mut FxSystemHost,
    cursor: &mut FxJournalCursor,
) -> bool {
    let refuse = |host: &mut FxSystemHost, step| {
        host.gaps.raise(FxGapCause::TrailCodeMeshRefused { step });
    };
    let (color, sort_key, ordinal) = match mesh.material_index.map(|id| binder.by_asset(id)) {
        None | Some(FxCodeMeshBind::Skip(FxPresentSkip::NoColorMap)) => {
            refuse(host, CodeMeshStep::Bind);
            cursor.draw_miss_material = cursor.draw_miss_material.saturating_add(1);
            return false;
        }
        Some(FxCodeMeshBind::Skip(cause)) => {
            count_fx_present_skip(cursor, cause);
            return false;
        }
        Some(FxCodeMeshBind::Ready {
            color,
            sort_key,
            ordinal,
        }) => (color, sort_key, ordinal),
    };
    let draw = plan.begin_material_draw(color, sort_key, Some(ordinal));
    let appended = append_trail(plan, mesh);
    plan.end_material_draw(draw);
    match appended {
        Ok(leftover_pairs) => {
            if leftover_pairs {
                refuse(host, CodeMeshStep::IndexReserve);
            }
            true
        }
        Err(step) => {
            refuse(host, step);
            false
        }
    }
}

/// Adds a trail's vertices and indices to the plan, all or nothing. On
/// success says whether index pairs were left over that formed no quad.
fn append_trail(plan: &mut FxCodeMeshPlan, mesh: &FxTrailMeshInstance) -> Result<bool, CodeMeshStep> {
    let verts_before = plan.mesh.vert_used;
    let indices_before = plan.mesh.index_used;
    for vert in &mesh.verts {
        let fits = plan.push_trail_vert(
            vert.xyz,
            vert.color_rgba,
            vert.texcoord_packed,
            vert.normal_packed,
            vert.tangent_packed,
        );
        if !fits {
            plan.shrink_verts_to(verts_before);
            return Err(CodeMeshStep::VertReserve);
        }
    }
    let (indices, leftover) = trail_indices(verts_before, &mesh.index_pairs);
    if !indices.is_empty() && !plan.extend_indices(&indices) {
        plan.shrink_verts_to(verts_before);
        plan.shrink_indices_to(indices_before);
        return Err(CodeMeshStep::IndexReserve);
    }
    Ok(leftover)
}

/// Triangle indices for a trail mesh whose first vertex is `base`. The
/// emitted index pairs come in threes, two triangles to a three; the flag
/// says whether pairs were left over at the end.
fn trail_indices(base: u32, pairs: &[[u16; 2]]) -> (Vec<u32>, bool) {
    let mut indices = Vec::with_capacity(pairs.len() / 3 * 6);
    for three in pairs.chunks_exact(3) {
        for tri in fx_t6::runtime::trail_index_quad_tris([three[0], three[1], three[2]]) {
            indices.extend(tri.map(|i| base.saturating_add(u32::from(i))));
        }
    }
    (indices, pairs.len() % 3 != 0)
}

fn draw_clouds(
    clouds: &[fx::FxCloudInstance],
    catalog: &asset_game::FxDefinitions,
    binder: &Binder,
    spark_plan: &mut FxParticleCloudPlan,
    t6_fx: &mut T6FxMesh,
    cursor: &mut FxJournalCursor,
) {
    for cloud in clouds {
        let Some(asset_id) = spark_elem_asset_id(catalog, cloud.catalog_index, cloud.def_index)
        else {
            spark_plan.miss_material = spark_plan.miss_material.saturating_add(1);
            continue;
        };
        if binder.fallback_pass(asset_id) {
            if !binder.hidden(asset_id) {
                t6_fx.push_cloud(&cloud.cloud, cloud.particles, asset_id as u32);
            }
            continue;
        }
        match binder.by_asset(asset_id) {
            FxCodeMeshBind::Skip(FxPresentSkip::NoColorMap) => {
                spark_plan.miss_material = spark_plan.miss_material.saturating_add(1);
            }
            FxCodeMeshBind::Skip(cause) => count_fx_present_skip(cursor, cause),
            FxCodeMeshBind::Ready {
                color,
                sort_key,
                ordinal,
            } => {
                spark_plan.begin_material_draw(
                    color,
                    sort_key,
                    Some(ordinal),
                    [cloud.cloud; 3],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(origin: [f32; 3], radius: f32, color_bgr: [f32; 3]) -> FxElemLightInstance {
        FxElemLightInstance {
            def_name: String::new(),
            def_index: 0,
            is_spot: false,
            origin,
            radius,
            color_bgr,
            axis: [[0.0; 3]; 3],
        }
    }

    #[test]
    fn trail_pairs_make_two_triangles_per_three() {
        let (indices, leftover) = trail_indices(10, &[[0, 1], [2, 3], [4, 5]]);
        assert_eq!(indices, vec![10, 11, 12, 11, 13, 12]);
        assert!(!leftover);
    }

    #[test]
    fn trail_pairs_beyond_whole_threes_are_reported() {
        let (indices, leftover) = trail_indices(0, &[[0, 1], [2, 3], [4, 5], [6, 7]]);
        assert_eq!(indices.len(), 6);
        assert!(leftover);
        let (indices, leftover) = trail_indices(0, &[[0, 1], [2, 3]]);
        assert!(indices.is_empty());
        assert!(leftover);
    }

    #[test]
    fn omni_lights_come_nearest_edge_first_and_swap_to_rgb() {
        let eye = Vec3::ZERO;
        let far_big = light([100.0, 0.0, 0.0], 80.0, [0.1, 0.2, 0.3]); // edge at 20
        let near_small = light([30.0, 0.0, 0.0], 5.0, [1.0, 0.0, 0.0]); // edge at 25
        let rows = nearest_omni_lights(&[near_small, far_big], eye, 8);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], [100.0, 0.0, 0.0, 80.0, 0.3, 0.2, 0.1, 0.0]);
        assert_eq!(rows[1][..4], [30.0, 0.0, 0.0, 5.0]);
        assert_eq!(rows[1][4..7], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn dark_and_empty_lights_are_left_out() {
        let lights = [
            light([0.0; 3], 0.0, [1.0; 3]),
            light([0.0; 3], 10.0, [0.0; 3]),
            light([0.0; 3], 10.0, [0.0, 0.5, 0.0]),
        ];
        assert_eq!(nearest_omni_lights(&lights, Vec3::ZERO, 8).len(), 1);
    }

    #[test]
    fn only_the_limit_is_kept() {
        let lights: Vec<_> = (0..5)
            .map(|i| light([i as f32 * 10.0, 0.0, 0.0], 1.0, [1.0; 3]))
            .collect();
        let rows = nearest_omni_lights(&lights, Vec3::ZERO, 3);
        assert_eq!(rows.iter().map(|r| r[0]).collect::<Vec<_>>(), vec![0.0, 10.0, 20.0]);
    }

    fn pack() -> lighting_t6::GfxLightPack {
        lighting_t6::omni_light_pack([0.0; 3], 10.0, [1.0; 3])
    }

    #[test]
    fn a_light_needs_a_world_and_a_radius() {
        let mut dlights = HostFxDlights::default();
        admit_scene_light(&mut dlights, false, 10.0, pack);
        admit_scene_light(&mut dlights, true, 0.0, pack);
        assert!(dlights.scene.is_empty());
        assert_eq!(dlights.cap_full, 0);
        admit_scene_light(&mut dlights, true, 10.0, pack);
        assert_eq!(dlights.scene.len(), 1);
    }

    #[test]
    fn a_full_scene_counts_the_refusals() {
        let mut dlights = HostFxDlights::default();
        for _ in 0..100 {
            admit_scene_light(&mut dlights, true, 10.0, pack);
        }
        assert_eq!(dlights.scene.len() + dlights.cap_full as usize, 100);
        assert!(dlights.cap_full > 0);
        // The pack is only built for lights that were taken.
        let mut built = 0;
        admit_scene_light(&mut dlights, true, 10.0, || {
            built += 1;
            pack()
        });
        assert_eq!(built, 0);
    }
}
