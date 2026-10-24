mod gpu_handoff;
mod hud_handoff;

use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderApp};
use frame::{RenderSet, configure_render_sets};
use net::ClientSet;

use crate::diag::acceptance::register_acceptance_systems;
use crate::diag::capture::{CaptureQueue, capture_frame, report_unwritten_captures};
use crate::diag::frame_spans::register_frame_spans;
use crate::diag::gpu_report::{gpu_report_enabled, report_gpu_allocator};
use crate::diag::render_frame_diag::{register_render_frame_diag, sample_render_frame_diag};
use crate::extract;

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
        ));
        add_main_world_handoffs(app);

        register_acceptance_systems(app);
        register_render_frame_diag(app);
        register_frame_spans(app);
        add_capture(app);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.add_systems(
            ExtractSchedule,
            (
                gpu_handoff::extract_gpu_load_progress,
                extract::extract_exact_colour,
                extract::extract_image_handles,
                extract::extract_postfx,
                extract::extract_model_lighting_tiles,
                extract::extract_guest_weapons,
            ),
        );
        if gpu_report_enabled() {
            render_app.add_systems(Render, report_gpu_allocator);
        }
        if render_gpu::geometry_diagnostic_enabled() {
            // The first-person gun and arms (and later effects) are drawn by
            // the fallback path, which these feed.
            render_app.add_systems(
                ExtractSchedule,
                (extract::extract_geometry, extract::extract_t6_dynamic),
            );
        }
        // Only reached with a render world: a headless run returned above, and
        // without one every column of this diagnostic would stay NULL.
        app.add_systems(Update, sample_render_frame_diag.in_set(ClientSet::Diag));
    }
}

/// What the main world hands to the render and HUD sides each frame, all of it
/// sealed behind the frontend's assembly.
fn add_main_world_handoffs(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (
            gpu_handoff::stamp_gpu_submit_ready.after(RenderSet::FrontendAssemble),
            hud_handoff::publish_ui3d_frame
                .after(RenderSet::FrontendAssemble)
                .before(extract::seal_render_frame),
        ),
    )
    .add_systems(
        PostUpdate,
        extract::seal_render_frame.after(gpu_handoff::stamp_gpu_submit_ready),
    )
    .add_systems(
        Update,
        hud_handoff::publish_overhead_posed_players
            .after(frame::WorkerCmdSet::SkinModel)
            .after(render_anim::occupancy::t6_body::T6BodyPosesPublished)
            .in_set(hud::OverheadPosedPlayerFramePublished)
            .in_set(ClientSet::Present),
    );
}

fn add_capture(app: &mut App) {
    app.init_resource::<CaptureQueue>().add_systems(
        Update,
        (capture_frame, report_unwritten_captures)
            .chain()
            .in_set(ClientSet::Diag),
    );
}
