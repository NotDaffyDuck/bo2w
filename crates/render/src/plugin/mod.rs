mod gpu_handoff;
mod hud_handoff;

use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderApp};
use frame::{DrawPassStage, order_draw_stages};
use net::ClientStep;

use crate::checks::trial::register_acceptance_systems;
use crate::checks::shots::{ScreenshotBacklog, grab_screenshot_frame, complain_uncaptured_outputs};
use crate::checks::frame_timing::register_frame_spans;
use crate::checks::gpu_report::{gpu_report_enabled, report_gpu_allocator};
use crate::checks::frame_report::{register_render_frame_diag, sample_frame_diag};
use crate::intake;

pub struct PixelsPlugin;

impl Plugin for PixelsPlugin {
    fn build(&self, app: &mut App) {
        order_draw_stages(app);
        app.add_plugins((
            render_anim::BodyAnimationPlugin,
            render_fx::EffectsRenderPlugin,
            render_scene::SceneRenderingModule,
            render_frontend::DrawPreparePlugin,
            render_frontend::DrawAdaptersPlugin,
            render_frontend::DrawAssemblePlugin,
            render_gpu::DrawGpuPlugin,
        ));
        add_main_world_handoffs(app);

        register_acceptance_systems(app);
        register_render_frame_diag(app);
        register_frame_spans(app);
        add_capture(app);

        let Some(render_sub) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_sub.add_systems(
            ExtractSchedule,
            (
                gpu_handoff::extract_gpu_load_progress,
                intake::pull_precise_colour,
                intake::pull_image_handles_to_render_world,
                intake::pull_screen_effects_to_render_world,
                intake::extract_model_lighting_tiles,
                intake::extract_guest_weapons,
            ),
        );
        if gpu_report_enabled() {
            render_sub.add_systems(Render, report_gpu_allocator);
        }
        if render_gpu::geometry_inspect_enabled() {
            // The first-person gun and arms (and later effects) are drawn by
            // the fallback path, which these feed.
            render_sub.add_systems(
                ExtractSchedule,
                (intake::pull_geometry_to_render_world, intake::extract_t6_dynamic),
            );
        }
        // Only reached with a render world: a headless run returned above, and
        // without one every column of this diagnostic would stay NULL.
        app.add_systems(Update, sample_frame_diag.in_set(ClientStep::Diag));
    }
}

/// What the main world hands to the render and HUD sides each frame, all of it
/// sealed behind the frontend's assembly.
fn add_main_world_handoffs(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (
            gpu_handoff::stamp_gpu_submit_ready.after(DrawPassStage::FrontBuild),
            hud_handoff::publish_ui3d_frame
                .after(DrawPassStage::FrontBuild)
                .before(intake::seal_render_frame),
        ),
    )
    .add_systems(
        PostUpdate,
        intake::seal_render_frame.after(gpu_handoff::stamp_gpu_submit_ready),
    )
    .add_systems(
        Update,
        hud_handoff::publish_overhead_posed_players
            .after(frame::JobQueueStage::SkinModel)
            .after(render_anim::occupancy::t6_body::T6BodyPosesPublished)
            .in_set(hud::NameplateBodyPoseShared)
            .in_set(ClientStep::Present),
    );
}

fn add_capture(app: &mut App) {
    app.init_resource::<ScreenshotBacklog>().add_systems(
        Update,
        (grab_screenshot_frame, complain_uncaptured_outputs)
            .chain()
            .in_set(ClientStep::Diag),
    );
}
