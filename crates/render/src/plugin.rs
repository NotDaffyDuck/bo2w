use bevy::prelude::*;
use frame::configure_render_sets;
use net::ClientSet;

use crate::diag::capture::{CaptureQueue, capture_frame, report_unwritten_captures};
use crate::diag::frame_spans::register_frame_spans;
use crate::diag::render_frame_diag::{register_render_frame_diag, sample_render_frame_diag};

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        configure_render_sets(app);
        app.add_plugins((
            render_anim::RenderAnimPlugin,
            render_fx::RenderFxPlugin,
            render_scene::RenderScenePlugin,
            render_frontend::RenderPreparePlugin,
            render_frontend::RenderAdaptersPlugin,
            render_frontend::RenderAssemblePlugin,
            render_gpu::RenderGpuPlugin,
        ))
        .add_systems(
            PostUpdate,
            (stamp_gpu_submit_ready, crate::extract::seal_render_frame)
                .chain()
                .after(frame::RenderSet::FrontendAssemble),
        )
        .add_systems(
            PostUpdate,
            publish_ui3d_frame
                .after(frame::RenderSet::FrontendAssemble)
                .before(crate::extract::seal_render_frame),
        )
        .add_systems(
            Update,
            publish_overhead_posed_players
                .after(frame::WorkerCmdSet::SkinModel)
                .after(render_anim::occupancy::t6_body::T6BodyPosesPublished)
                .in_set(hud::OverheadPosedPlayerFramePublished)
                .in_set(ClientSet::Present),
        );

        crate::diag::acceptance::register_acceptance_systems(app);
        register_render_frame_diag(app);
        // BO2MP_HEADLESS=1: no render world, so every column would stay NULL;
        // a headless run keeps its log (and the disk) quiet instead.
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            app.add_systems(Update, sample_render_frame_diag.in_set(ClientSet::Diag));
        }
        register_frame_spans(app);
        app.init_resource::<CaptureQueue>().add_systems(
            Update,
            (capture_frame, report_unwritten_captures)
                .chain()
                .in_set(ClientSet::Diag),
        );
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.add_systems(
                bevy::render::ExtractSchedule,
                (
                    extract_gpu_load_progress,
                    crate::extract::extract_exact_colour,
                    crate::extract::extract_image_handles,
                    crate::extract::extract_postfx,
                    crate::extract::extract_model_lighting_tiles,
                    crate::extract::extract_guest_weapons,
                ),
            );
            if crate::diag::gpu_report::gpu_report_enabled() {
                render_app.add_systems(
                    bevy::render::Render,
                    crate::diag::gpu_report::report_gpu_allocator,
                );
            }
            if render_gpu::geometry_diagnostic_enabled() {
                render_app.add_systems(
                    bevy::render::ExtractSchedule,
                    (
                        crate::extract::extract_geometry,
                        // bo2zm: the first-person gun and arms (and later
                        // effects) for the Black Ops II fallback draw.
                        crate::extract::extract_t6_dynamic,
                    ),
                );
            }
        }
    }
}

fn extract_gpu_load_progress(
    mut main_world: ResMut<bevy::render::MainWorld>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
) {
    use render_frontend::prepare::scene::world_gpu::GpuLoadProgress;

    let Some(mut progress) = main_world.get_resource_mut::<GpuLoadProgress>() else {
        return;
    };
    progress.waiting_n = pipelines.waiting_pipelines().count() as u32;
    progress.pipeline_n = pipelines.pipelines().count() as u32;
    let pending = std::mem::take(&mut progress.pending_shaders);
    progress.pending_shaders = waiting_pipeline_shaders(&pipelines, pending);
}

fn waiting_pipeline_shaders(
    pipelines: &bevy::render::render_resource::PipelineCache,
    mut out: Vec<bevy::asset::AssetId<bevy::shader::Shader>>,
) -> Vec<bevy::asset::AssetId<bevy::shader::Shader>> {
    use bevy::render::render_resource::PipelineDescriptor;

    out.clear();
    let waiting: std::collections::HashSet<_> = pipelines.waiting_pipelines().collect();
    if waiting.is_empty() {
        return out;
    }
    for (index, pipeline) in pipelines.pipelines().enumerate() {
        if !waiting.contains(&index) {
            continue;
        }
        if let PipelineDescriptor::RenderPipelineDescriptor(desc) = &pipeline.descriptor {
            out.push(desc.vertex.shader.id());
            out.extend(desc.fragment.iter().map(|fragment| fragment.shader.id()));
        }
    }
    out
}

fn stamp_gpu_submit_ready(
    demand: Option<Res<render_frontend::prepare::scene::world_gpu::GpuSubmitDemand>>,
    mut ready: ResMut<render_gpu::GpuSubmitReady>,
) {
    let Some(demand) = demand.as_ref() else {
        *ready = render_gpu::GpuSubmitReady::default();
        return;
    };
    *ready = render_gpu::GpuSubmitReady {
        world_generation: demand.world_generation,
        warm_pipelines: demand.warm_pipelines,
        overlay_gpu_wait: demand.overlay_gpu_wait,
        pipeline_world_materials: demand.pipeline_world_materials.clone(),
        pipeline_smodel_materials: demand.pipeline_smodel_materials.clone(),
        pipeline_demand_revision: demand.pipeline_demand_revision,
    };
}

fn publish_overhead_posed_players(
    source: Res<render_anim::PosedPlayerFrame>,
    mut destination: Option<ResMut<hud::OverheadPosedPlayerFrame>>,
) {
    let Some(destination) = destination.as_mut() else {
        return;
    };
    destination.replace(source.iter().map(|player| {
        let head = match player.head {
            render_anim::PosedPlayerHead::Exact(world) => {
                hud::OverheadPosedHead::ExactWorld(world.to_array())
            }
            render_anim::PosedPlayerHead::NoDObjOrHead => hud::OverheadPosedHead::NoDObjOrHead,
        };
        (player.entnum, head)
    }));
}

/// bo2mp: the Target Finder's sight screen, into the UI3D texture (Streaks
/// facts/target-finder.md): hud's enemy spots placed through the held gun's
/// `tag_hud`, then `hud_t6::rangefinder::frame_quads`, drawn by render_gpu
/// drawsurf/ui3d.rs. Screen off: no quads, the texture stays cleared.
#[allow(clippy::too_many_arguments)]
fn publish_ui3d_frame(
    spots: Option<Res<hud::RangefinderSpots>>,
    bolts: Res<render_anim::FpvBoltTargets>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
    mut atlas: Local<Option<Handle<Image>>>,
    mut frame: ResMut<render_gpu::drawsurf::Ui3dFrame>,
) {
    use hud_t6::rangefinder as rf;
    let quads = match spots.as_deref().filter(|spots| spots.draws_screen) {
        None => Vec::new(),
        Some(spots) => {
            let eye = cameras
                .iter()
                .find(|(camera, _)| camera.is_active)
                .map(|(_, transform)| transform.translation().to_array());
            let diamonds: Vec<[f32; 4]> = match (bolts.hud, eye) {
                (Some(hud), Some(eye)) if rf::DIAMOND_INDICATOR => {
                    let tag = rf::TagHud {
                        origin: hud.orientation.origin,
                        axis: hud.orientation.axis,
                    };
                    spots
                        .spots
                        .iter()
                        .filter_map(|(origin, stance)| {
                            rf::diamond_rect(&tag, eye, *origin, *stance, rf::DIAMOND_SIZE)
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            rf::frame_quads(spots.targeting, &diamonds)
                .into_iter()
                .map(|quad| render_gpu::drawsurf::Ui3dQuad {
                    xy: quad.corners.map(|corner| corner.xy),
                    uv: quad.corners.map(|corner| corner.uv),
                    textured: quad.textured,
                })
                .collect()
        }
    };
    if atlas.is_none()
        && let Some((_, picture)) = icons
            .as_deref()
            .and_then(|icons| icons.0.iter().find(|(name, _)| name == rf::BACKGROUND_MATERIAL))
    {
        let mut picture = (**picture).clone();
        picture.asset_usage = bevy::asset::RenderAssetUsages::default();
        *atlas = Some(images.add(picture));
    }
    let next = render_gpu::drawsurf::Ui3dFrame {
        quads,
        atlas: atlas.clone(),
    };
    if *frame != next {
        *frame = next;
    }
}
