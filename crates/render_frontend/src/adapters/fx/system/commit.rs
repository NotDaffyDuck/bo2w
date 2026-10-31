//! Closing an effects frame: what `generate_fx_transaction` produced becomes
//! this frame's draw plans, lights and fallback-pass meshes.
//!
//! Every kind of product has a stage of its own below, and
//! `commit_fx_transaction` fixes their order. Marks are planned before
//! models, and the tracer beams are appended after the sprites and trails
//! so that they land in the same plan.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::{
    ProceduralMeshPhase, ParticleGlowCopy, EffectHoleReason, MeshParticleCopy, EffectSpriteInstance,
    EffectSystemHost, StreakMeshCopy,
};
use fx_t6::AfterglowSprite;
use render_fx::{
    BattleEffectsSnapshot, EffectDumpRequest, FxFrameResult, EffectJournalCursor, FxSkipCause,
    EffectLevelColorImages, FxFrameView, GeneratedFxFrame, EffectsDynamicLightSet,
    PostEffectLightSet, EffectsSystemHandle, JournalCount, PostLightFate, ReadyFxBook,
    StreakLevel,
};

use super::dump::{FrameTally, write_fx_dump, DumpInput};
use super::mark_plan::fill_mark_mesh_plan;
use super::mesh_bind::{
    EffectEngineMeshBind, EffectPresentSkip, code_mesh_bind, code_mesh_bind_asset, tally_fx_present_skip,
    spark_elem_asset_id,
};
use super::model_plan::fill_effect_model_plan;
use super::sprite_runs::{batch_by_material, sprite_transform};
use super::{
    EffectFrameTransaction, EffectPresentEnv, T6_DYN_LIGHTS, T6_FX_ELEM_DRAW_WITH_VIEWMODEL, T6DynLights,
    T6FxMesh, T6MarkMesh, t6_fx_filtered_out, t6_fx_material, take_t6_marks,
};
use crate::adapters::fx::tracer::present_streak_beams;
use crate::{
    FxCodeDrawPlan, FxModelDrawList, FxCloudDrawPlan, VisualStainMeshPlan, MaterialEpoch,
};
use crate::prepare::scene::model_illum_cache::{ModelIllumRequests, LevelModelIllumCache};
use crate::prepare::scene::world::LevelStage;

/// The plans an effects frame fills.
#[derive(SystemParam)]
pub(super) struct FxPlans<'w> {
    code: ResMut<'w, FxCodeDrawPlan>,
    clouds: ResMut<'w, FxCloudDrawPlan>,
    marks: ResMut<'w, VisualStainMeshPlan>,
    models: ResMut<'w, FxModelDrawList>,
}

impl FxPlans<'_> {
    /// Empties the code-mesh, particle-cloud and mark-mesh plans before a frame is built.
    fn reset(&mut self) {
        self.code.reset_frame();
        self.clouds.drop_frame_draws();
        self.marks.clear();
    }

    /// Publishes the code-mesh and particle-cloud plans as empty, bumping their
    /// revisions so consumers notice that the content went away.
    fn publish_blank(&mut self) {
        self.code.advance_revision();
        self.code.publish_draw_ranges();
        self.clouds.advance_revision();
        self.clouds.publish_draw_ranges();
    }
}

/// The material tables the draw stages look their materials up in.
struct Binder<'a> {
    colour_images: &'a EffectLevelColorImages,
    runtime: &'a MaterialEpoch,
}

impl Binder<'_> {
    fn by_name(&self, name: &str) -> EffectEngineMeshBind {
        code_mesh_bind(name, self.colour_images, self.runtime)
    }

    fn by_asset(&self, asset_id: usize) -> EffectEngineMeshBind {
        code_mesh_bind_asset(asset_id, self.colour_images, self.runtime)
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

    /// A lens flare whose ghosts the renderer lays out (`T6Draw::flare`).
    fn flare(&self, asset_id: usize) -> bool {
        let id = assets::SurfaceMaterialId::from_sequence(asset_id);
        let material = self.runtime.catalog.derived(id);
        material.and_then(|m| m.t6_draw.as_ref()).is_some_and(|draw| draw.flare)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_fx_transaction(
    In(tx): In<EffectFrameTransaction>,
    mut fx_host: ResMut<EffectsSystemHandle>,
    fx_book: Option<Res<ReadyFxBook>>,
    colour_targets: Res<EffectLevelColorImages>,
    runtime: Res<MaterialEpoch>,
    level: Option<Res<LevelStage>>,
    mut journal: ResMut<EffectJournalCursor>,
    mut plans: FxPlans,
    (mut t6_fx, mut t6_marks, mut t6_lights): (
        ResMut<T6FxMesh>,
        ResMut<T6MarkMesh>,
        ResMut<T6DynLights>,
    ),
    mut dump_req: ResMut<EffectDumpRequest>,
    mut streak_level: ResMut<StreakLevel>,
    mut combat_res: ResMut<BattleEffectsSnapshot>,
    mut env: EffectPresentEnv,
    lighting_cache: Option<Res<LevelModelIllumCache>>,
    mut illum_requests: ResMut<ModelIllumRequests>,
) {
    // The span guard lives to the end of the commit.
    let EffectFrameTransaction { outcome, _present_span: _show_span } = tx;
    reset_frame(&mut plans, &mut t6_fx, &mut env);

    let frame = match outcome {
        FxFrameResult::Ready(frame) => frame,
        FxFrameResult::Skipped(FxSkipCause::NoCatalog) => {
            fx_host.0.last_stain_mesh = None;
            publish_nothing(&mut fx_host.0, &mut plans, &runtime);
            return;
        }
    };
    let GeneratedFxFrame {
        products: mut made,
        gaps: vertices_gaps,
        marks: mark_mesh,
        after_lights,
        tracers: mut beams,
        view,
    } = frame;
    let FxFrameView {
        camera: cam_tf,
        frustum: view_volume_planes,
        view_proj: world_to_clip,
        fov_half_tan,
        level_present,
    } = view;
    let Some(effect_book) = fx_book else {
        *streak_level = beams;
        publish_nothing(&mut fx_host.0, &mut plans, &runtime);
        return;
    };
    let binder = Binder {
        colour_images: &colour_targets,
        runtime: &runtime,
    };

    hide_test_elems(&mut made);
    take_t6_marks(&fx_host.0, mark_mesh.as_ref(), &runtime, &mut t6_marks);
    fill_mark_mesh_plan(&mut fx_host.0, mark_mesh, &mut plans.marks, &runtime);

    let eye = cam_tf.translation;
    let model_catalog = env.models.as_deref().map(|prepared| &prepared.0);
    let (atlas, model_light) = (env.atlas.as_deref(), lighting_cache.as_deref());
    let plain_models = route_models(&made.models, model_catalog, &runtime, &mut t6_fx);
    fill_effect_model_plan(
        &plain_models,
        model_catalog,
        atlas,
        level.as_deref(),
        &env.at_spot,
        eye,
        &view_volume_planes,
        env.detail_skinned.args(),
        model_light,
        &mut illum_requests,
        &mut env.staged_models.0,
    );
    plans.models.publish_staged(&mut env.staged_models.0);

    stage_scene_lights(&made.spot_lights, &made.omni_lights, level_present, &mut env.dlights);
    t6_lights.lights.clear();
    t6_lights
        .lights
        .extend(nearest_omni_lights(&made.omni_lights, eye, T6_DYN_LIGHTS));
    log_dynamic_lights(&t6_lights.lights);
    draw_post_lights(
        &after_lights,
        eye,
        &binder,
        &mut plans.code,
        &mut journal,
        &mut env.after_lights,
    );

    let sprites_drawn = draw_sprites(&made.sprites, &cam_tf, &binder, &mut plans.code, &mut t6_fx, &mut journal);
    let trails = made.trail_strips.iter();
    let trails_drawn = trails.filter(|mesh| draw_trail(mesh, &binder, &mut plans.code, &mut fx_host.0, &mut journal)).count();
    present_streak_beams(
        &mut beams,
        &cam_tf,
        world_to_clip,
        fov_half_tan,
        &colour_targets,
        &runtime,
        &mut plans.code,
        &mut combat_res,
        code_mesh_bind_asset,
        &mut t6_fx,
        |asset_id| binder.fallback_pass(asset_id) && !binder.hidden(asset_id),
    );
    *streak_level = beams;
    draw_clouds(&made.clouds, &effect_book.0, &binder, &mut plans.clouds, &mut t6_fx, &mut journal);

    plans.clouds.advance_revision();
    plans.clouds.publish_draw_ranges();
    plans.code.advance_revision();
    plans.code.publish_draw_ranges();

    if let Some(radius) = dump_req.take() {
        write_fx_dump(&DumpInput {
            host: &fx_host.0,
            out: &made,
            vertices_gaps: &vertices_gaps,
            tally: FrameTally {
                sprites_drawn,
                trails_drawn,
            },
            camera: &cam_tf,
            rad: radius,
            model_plan: &plans.models,
            plan: &plans.code,
            spark_plan: &plans.clouds,
            dlights: &env.dlights,
            after_lights: &env.after_lights,
            cursor: &journal,
            combat: &combat_res,
        });
    }
}

/// Empties everything this frame will refill.
fn reset_frame(plans: &mut FxPlans, t6_fx: &mut T6FxMesh, env: &mut EffectPresentEnv) {
    plans.reset();
    t6_fx.clear();

    let staged = &mut env.staged_models.0;
    staged.reset_frame();
    match &env.model_geometry {
        Some(prepared) => staged.adopt_prepared(&prepared.0),
        None => *staged = FxModelDrawList::without_geometry(0),
    }

    env.dlights.begin_frame();
    env.after_lights.begin_frame();
}

/// A frame with nothing to draw: no marks, and the plans published empty.
fn publish_nothing(host: &mut EffectSystemHost, plans: &mut FxPlans, runtime: &MaterialEpoch) {
    fill_mark_mesh_plan(host, None, &mut plans.marks, runtime);
    plans.publish_blank();
}

/// Sends the effect models Black Ops II draws itself (shell casings,
/// debris) to the fallback pass, posed on the spot, and returns the rest
/// for the model plan.
fn route_models(
    models: &[MeshParticleCopy],
    catalog: Option<&asset_game::EffectMeshRoster>,
    runtime: &MaterialEpoch,
    t6_fx: &mut T6FxMesh,
) -> Vec<MeshParticleCopy> {
    let mut rest = Vec::with_capacity(models.len());
    for copy in models {
        let entry = catalog.and_then(|models| models.record_at(copy.mesh_slot_number));
        if let Some(entry) = entry {
            // `fx_decal_*` models are volumes Black Ops II projects blood
            // onto a body from; drawn as models they show as black blocks.
            if entry.bone_rig.name.starts_with("fx_decal") {
                continue;
            }
            let fallback_material = |surface: usize| {
                entry
                    .surface_slot(surface)
                    .map(|index| index.order())
                    .filter(|&order| t6_fx_material(runtime, order))
            };
            if (0..entry.bone_rig.facet_materials.len()).any(|s| fallback_material(s).is_some()) {
                t6_fx.push_model(
                    &entry.bone_rig,
                    |surface| fallback_material(surface).map(|order| order as u32),
                    copy.place_at,
                    copy.axes_m,
                    copy.size_k,
                );
                continue;
            }
        }
        rest.push(copy.clone());
    }
    rest
}

/// Hands the effects' spot and omni lights to the scene, as many as the
/// scene admits.
fn stage_scene_lights(
    spots: &[ParticleGlowCopy],
    omnis: &[ParticleGlowCopy],
    level_present: bool,
    dlights: &mut EffectsDynamicLightSet,
) {
    let cone = lighting_t6::ConeLightTuningVars::default_registrations();
    for glow in spots {
        let [forward, ..] = glow.frame_rows;
        let pack = || lighting_t6::spot_light_pack(glow.anchor, forward, glow.rad, glow.tint_blue_first, cone);
        dlights.admit(level_present, glow.rad, pack);
    }
    for glow in omnis {
        let pack = || lighting_t6::pack_point_lamp(glow.anchor, glow.rad, glow.tint_blue_first);
        dlights.admit(level_present, glow.rad, pack);
    }
}

/// bo2zm test aid: BO2MP_FX_HIDE=<effect name part>:<element>[,<element>...] leaves those
/// elements of that effect undrawn (which part draws what).
fn hide_test_elems(out: &mut fx::CornerBuildOutput) {
    static HIDE: std::sync::OnceLock<Option<(String, Vec<u8>)>> = std::sync::OnceLock::new();
    let hide = HIDE.get_or_init(|| {
        let v = std::env::var("BO2MP_FX_HIDE").ok()?;
        let (name, list) = v.split_once(':')?;
        Some((name.to_owned(), list.split(',').filter_map(|s| s.trim().parse().ok()).collect()))
    });
    let Some((want, elems)) = hide else {
        return;
    };
    let hidden = |name: &str, i: u8| name.contains(want.as_str()) && elems.contains(&i);
    out.sprites.retain(|s| !hidden(&s.definition_name, s.definition_slot));
    out.trail_strips.retain(|t| !hidden(&t.definition_name, t.definition_slot));
    out.models.retain(|m| !hidden(&m.definition_name, m.definition_slot));
    out.clouds.retain(|c| !hidden(&c.definition_name, c.definition_slot));
}

/// bo2zm test aid: BO2MP_FX_DLIGHT_LOG=1 logs the effect lights about once a
/// second (place, radius, colour).
fn log_dynamic_lights(lights: &[[f32; 8]]) {
    static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if !*LOG.get_or_init(|| std::env::var_os("BO2MP_FX_DLIGHT_LOG").is_some())
        || FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 != 0
    {
        return;
    }
    let row_list: Vec<String> = lights
        .iter()
        .map(|l| {
            format!(
                "({:.0} {:.0} {:.0}) r{:.0} rgb {:.2} {:.2} {:.2}",
                l[0], l[1], l[2], l[3], l[4], l[5], l[6]
            )
        })
        .collect();
    diag::info!(World, "fx dlights {}: {}", row_list.len(), row_list.join(" | "));
}

/// The omni lights for the fallback pass as `[x, y, z, radius, r, g, b, 0]`
/// rows, most important first, at most `limit`: as in Black Ops II, the
/// smallest (eye distance / radius) squared, so a big light further off can
/// beat a small one close by.
fn nearest_omni_lights(lights: &[ParticleGlowCopy], eye: Vec3, limit: usize) -> Vec<[f32; 8]> {
    let mut lit: Vec<(f32, [f32; 8])> = lights
        .iter()
        .filter(|light| light.rad > 0.0 && light.tint_blue_first.iter().any(|&c| c > 0.0))
        .map(|light| {
            let d = Vec3::from_array(light.anchor).distance(eye);
            let rank = (d * d) / (light.rad * light.rad);
            let [b, g, r] = light.tint_blue_first;
            let [x, y, z] = light.anchor;
            (rank, [x, y, z, light.rad, r, g, b, 0.0])
        })
        .collect();
    lit.sort_by(|a, b| a.0.total_cmp(&b.0));
    lit.into_iter().take(limit).map(|(_, row)| row).collect()
}

fn draw_post_lights(
    lights: &[AfterglowSprite],
    eye: Vec3,
    binder: &Binder,
    plan: &mut FxCodeDrawPlan,
    cursor: &mut EffectJournalCursor,
    stats: &mut PostEffectLightSet,
) {
    for light in lights {
        let Some(tess) = fx_t6::post_light_generate_verts(light, eye.to_array()) else {
            stats.log_fate(PostLightFate::TooShort);
            continue;
        };
        match binder.by_name(light.surface_title) {
            EffectEngineMeshBind::Skip(EffectPresentSkip::MissingColorMap) => {
                stats.log_fate(PostLightFate::Unbound);
                plan.note_unmatched(1);
            }
            EffectEngineMeshBind::Skip(cause) => {
                tally_fx_present_skip(cursor, cause);
                stats.log_fate(PostLightFate::Unbound);
            }
            EffectEngineMeshBind::Ready {
                tint: color,
                order_key: sort_key,
                ord_no: ord,
            } => {
                let appended = plan.in_run(color, sort_key, Some(ord), |run| {
                    run.append_post_light(&tess)
                });
                if appended {
                    stats.log_fate(PostLightFate::Drawn);
                }
            }
        }
    }
}

/// Draws the frame's sprites and returns how many made it into a plan.
fn draw_sprites(
    sprites: &[EffectSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    plan: &mut FxCodeDrawPlan,
    t6_fx: &mut T6FxMesh,
    cursor: &mut EffectJournalCursor,
) -> usize {
    let grouped = batch_by_material(
        sprites.iter().map(|sprite| (sprite.shader_at, sprite)),
        |asset_id| binder.fallback_pass(asset_id),
    );
    cursor.add(JournalCount::UnboundDraws, grouped.without_material);
    plan.note_unmatched(grouped.without_material);

    let mut drawn = 0;
    for (asset_no, run) in &grouped.batches {
        drawn += if binder.fallback_pass(*asset_no) {
            draw_fallback_sprites(*asset_no, run, camera, binder, t6_fx)
        } else {
            draw_code_mesh_sprites(*asset_no, run, camera, binder, plan, cursor)
        };
    }
    drawn
}

/// One run of Black Ops II sprites for the fallback pass. The sprites that
/// draw with the viewmodel are a draw of their own after the others.
fn draw_fallback_sprites(
    run_material: usize,
    run: &[&EffectSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    t6_fx: &mut T6FxMesh,
) -> usize {
    if binder.hidden(run_material) {
        return 0;
    }
    let flare = binder.flare(run_material);
    let mut drawn = 0;
    for viewmodel in [false, true] {
        let first = t6_fx.indices.len() as u32;
        for spr in run
            .iter()
            .filter(|s| (s.bit_word & T6_FX_ELEM_DRAW_WITH_VIEWMODEL != 0) == viewmodel)
        {
            let xf = sprite_transform(spr, camera);
            if flare {
                t6_fx.push_flare(xf.translation, spr.rgba_tint[3]);
            } else {
                t6_fx.push_quad(xf, spr.rgba_tint, spr.atlas);
            }
            drawn += 1;
        }
        let count = t6_fx.indices.len() as u32 - first;
        if count == 0 {
            continue;
        }
        let draws = if viewmodel { &mut t6_fx.viewmodel_draws } else { &mut t6_fx.draws };
        draws.push((first, count, run_material as u32));
    }
    drawn
}

/// One run of sprites for the code mesh pass.
fn draw_code_mesh_sprites(
    run_material: usize,
    run: &[&EffectSpriteInstance],
    camera: &Transform,
    binder: &Binder,
    plan: &mut FxCodeDrawPlan,
    cursor: &mut EffectJournalCursor,
) -> usize {
    match binder.by_asset(run_material) {
        EffectEngineMeshBind::Skip(EffectPresentSkip::MissingColorMap) => {
            let missed = run.len() as u32;
            cursor.add(JournalCount::UnboundDraws, missed);
            plan.note_unmatched(missed);
            if cursor.tally_of(JournalCount::UnboundDraws) == missed {
                let name = run.first().map_or("?", |s| s.surface_title.as_ref());
                diag::warn!(
                    World,
                    "fx: sprite material {run_material} (`{name}`) has no colour image; further misses are only counted"
                );
            }
            0
        }
        EffectEngineMeshBind::Skip(cause) => {
            tally_fx_present_skip(cursor, cause);
            0
        }
        EffectEngineMeshBind::Ready {
            tint: color,
            order_key: sort_key,
            ord_no: ord,
        } => {
            plan.in_run(color, sort_key, Some(ord), |open| {
                let appended = |spr: &&&EffectSpriteInstance| open.append_sprite_quad(sprite_transform(spr, camera), spr.rgba_tint, spr.atlas);
                run.iter().filter(appended).count()
            })
        }
    }
}

/// Draws one trail mesh; true when it went into the plan.
fn draw_trail(
    mesh: &StreakMeshCopy,
    binder: &Binder,
    plan: &mut FxCodeDrawPlan,
    host: &mut EffectSystemHost,
    cursor: &mut EffectJournalCursor,
) -> bool {
    let refuse = |host: &mut EffectSystemHost, step| {
        host.gaps.log_hole(EffectHoleReason::TrailCodeMeshRefused { step });
    };
    let (color, sort_key, ord) = match mesh.shader_slot_ix.map(|id| binder.by_asset(id)) {
        None | Some(EffectEngineMeshBind::Skip(EffectPresentSkip::MissingColorMap)) => {
            refuse(host, ProceduralMeshPhase::Bind);
            cursor.bump(JournalCount::UnboundDraws);
            return false;
        }
        Some(EffectEngineMeshBind::Skip(cause)) => {
            tally_fx_present_skip(cursor, cause);
            return false;
        }
        Some(EffectEngineMeshBind::Ready {
            tint: color,
            order_key: sort_key,
            ord_no: ordinal,
        }) => (color, sort_key, ordinal),
    };
    let appended = plan.in_run(color, sort_key, Some(ord), |open| append_trail(open, mesh));
    match appended {
        Ok(leftover_pairs) => {
            if leftover_pairs {
                refuse(host, ProceduralMeshPhase::TriangleSlotClaim);
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
fn append_trail(plan: &mut FxCodeDrawPlan, mesh: &StreakMeshCopy) -> Result<bool, ProceduralMeshPhase> {
    let verts_before = plan.pools.vertices_consumed;
    let indices_before = plan.pools.index_fill;
    for vert in &mesh.vertex_points {
        let fits = plan.append_trail_vertex(
            vert.spot,
            vert.tint,
            [vert.packed_surface_uv, vert.facing_word, vert.surface_dir_word],
        );
        if !fits {
            plan.cut_vertices_to(verts_before);
            return Err(ProceduralMeshPhase::CornerClaim);
        }
    }
    let (index_list, leftover) = trail_indices(verts_before, &mesh.pair_indices);
    if !index_list.is_empty() && !plan.append_indices(&index_list) {
        plan.cut_vertices_to(verts_before);
        plan.cut_indices_to(indices_before);
        return Err(ProceduralMeshPhase::TriangleSlotClaim);
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
    clouds: &[fx::SwarmCopy],
    catalog: &asset_game::FxDefinitions,
    binder: &Binder,
    spark_plan: &mut FxCloudDrawPlan,
    t6_fx: &mut T6FxMesh,
    cursor: &mut EffectJournalCursor,
) {
    for cloud in clouds {
        let Some(asset_id) = spark_elem_asset_id(catalog, cloud.ledger_index, cloud.definition_slot)
        else {
            spark_plan.note_unmatched(1);
            continue;
        };
        if binder.fallback_pass(asset_id) {
            if !binder.hidden(asset_id) {
                t6_fx.push_cloud(&cloud.cloud, cloud.particles, asset_id as u32);
            }
            continue;
        }
        match binder.by_asset(asset_id) {
            EffectEngineMeshBind::Skip(EffectPresentSkip::MissingColorMap) => {
                spark_plan.note_unmatched(1);
            }
            EffectEngineMeshBind::Skip(cause) => tally_fx_present_skip(cursor, cause),
            EffectEngineMeshBind::Ready {
                tint: color,
                order_key: sort_key,
                ord_no: ord,
            } => {
                spark_plan.open_cloud_run([cloud.cloud; 3], color, sort_key, Some(ord));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(origin: [f32; 3], reach_r: f32, tint_blue_first: [f32; 3]) -> ParticleGlowCopy {
        ParticleGlowCopy {
            definition_name: String::new(),
            definition_slot: 0,
            is_cone: false,
            anchor: origin,
            rad: reach_r,
            tint_blue_first,
            frame_rows: [[0.0; 3]; 3],
        }
    }

    #[test]
    fn trail_pairs_make_two_triangles_per_three() {
        let (index_list, leftover) = trail_indices(10, &[[0, 1], [2, 3], [4, 5]]);
        assert_eq!(index_list, vec![10, 11, 12, 11, 13, 12]);
        assert!(!leftover);
    }

    #[test]
    fn trail_pairs_beyond_whole_threes_are_reported() {
        let (index_list, leftover) = trail_indices(0, &[[0, 1], [2, 3], [4, 5], [6, 7]]);
        assert_eq!(index_list.len(), 6);
        assert!(leftover);
        let (tri_list, leftover) = trail_indices(0, &[[0, 1], [2, 3]]);
        assert!(tri_list.is_empty());
        assert!(leftover);
    }

    #[test]
    fn omni_lights_come_most_important_first_and_swap_to_rgb() {
        let eye = Vec3::ZERO;
        let far_big = light([100.0, 0.0, 0.0], 80.0, [0.1, 0.2, 0.3]); // (100/80)^2
        let near_small = light([30.0, 0.0, 0.0], 5.0, [1.0, 0.0, 0.0]); // (30/5)^2
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
        assert_eq!(nearest_omni_lights(&lights, Vec3::splat(0.0), 8).len(), 1);
    }

    #[test]
    fn only_the_limit_is_kept() {
        let lights: Vec<_> = (0..5)
            .map(|i| light([i as f32 * 10.0, 0.0, 0.0], 1.0, [1.0; 3]))
            .collect();
        let rows = nearest_omni_lights(&lights, Vec3::splat(0.0), 3);
        assert_eq!(rows.iter().map(|r| r[0]).collect::<Vec<_>>(), vec![0.0, 10.0, 20.0]);
    }

    fn pack() -> lighting_t6::LampRecord {
        lighting_t6::pack_point_lamp([0.0; 3], 10.0, [1.0; 3])
    }

    #[test]
    fn a_light_needs_a_world_and_a_radius() {
        let mut dlights = EffectsDynamicLightSet::default();
        dlights.admit(false, 10.0, pack);
        dlights.admit(true, 0.0, pack);
        assert!(dlights.scene.is_empty());
        assert_eq!(dlights.over_cap(), 0);
        dlights.admit(true, 10.0, pack);
        assert_eq!(dlights.scene.len(), 1);
    }

    #[test]
    fn a_full_scene_counts_the_refusals() {
        let mut dlights = EffectsDynamicLightSet::default();
        for _ in 0..100 {
            dlights.admit(true, 10.0, pack);
        }
        assert_eq!(dlights.scene.len() + dlights.over_cap() as usize, 100);
        assert!(dlights.over_cap() > 0);
        // The pack is only built for lights that were taken.
        let mut built = 0;
        dlights.admit(true, 10.0, || {
            built += 1;
            pack()
        });
        assert_eq!(built, 0);
    }
}
