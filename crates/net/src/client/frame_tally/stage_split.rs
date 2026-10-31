//! How the time in the Present and Ui phases divides among the systems that run in them.
//!
//! The stamps here are intervals between systems and the bodies of the HUD flush systems. The
//! intervals include whatever else the executor ran between two systems, so they show where the
//! schedule put work, not what a system cost; `hud_tess_body_ms` is the HUD's own cost. Which
//! phase a stamp belongs to follows the schedule, not the field name: the HUD surfaces are
//! configured inside Present, the HUD root chain inside Ui.

use std::time::Instant;

use bevy::prelude::*;

use crate::tick_sets::ClientStep;

pub const OVERLAY_STAGE_TOTAL: usize = 9;

#[derive(Resource, Debug, Default, Clone)]
pub struct FrameUpdateTally {
    pub display_began: Option<Instant>,
    pub overlay_root_prep_millis: Option<f32>,
    /// Named for Present, but the stamp is taken in Ui.
    pub display_deferred_apply_millis: Option<f32>,
    pub overlay_visibility_millis: Option<f32>,
    /// The interval across all nine HUD stages.
    pub overlay_surface_millis: Option<f32>,
    /// The interval inside each HUD stage. The largest is part of `overlay_surface_millis`, not beside
    /// it.
    pub hud_stage_millis: [Option<f32>; OVERLAY_STAGE_TOTAL],
    /// Time inside the bodies of the HUD tess flush systems, summed over the frame, and the tess
    /// jobs those bodies applied.
    pub hud_tess_body_ms: Option<f32>,
    pub hud_tess_jobs: Option<u32>,
    pub announce_shown_millis: Option<f32>,
}

/// The index and value of the largest measured stage; the first wins a tie.
fn slowest_stage(stages: &[Option<f32>]) -> Option<(usize, f32)> {
    let mut slowest: Option<(usize, f32)> = None;
    for (at, ms) in stages.iter().enumerate() {
        let Some(ms) = *ms else { continue };
        if slowest.is_none_or(|(_, best)| ms > best) {
            slowest = Some((at, ms));
        }
    }
    slowest
}

fn reset(mut census: ResMut<FrameUpdateTally>) {
    *census = FrameUpdateTally::default();
}

fn mark_present_start(mut census: ResMut<FrameUpdateTally>) {
    census.display_began = Some(Instant::now());
}

fn publish(census: Res<FrameUpdateTally>) {
    if !perf::recording() {
        return;
    }
    let emit = |counter: perf::Counter, ms: Option<f32>| {
        if let Some(ms) = ms {
            counter.emit(f64::from(ms));
        }
    };
    emit(perf::Counter::PresentPublishMs, census.announce_shown_millis);
    emit(perf::Counter::HudSurfacesScheduleMs, census.overlay_surface_millis);
    emit(perf::Counter::HudTessBodyMs, census.hud_tess_body_ms);
    if let Some(jobs) = census.hud_tess_jobs {
        perf::Counter::HudTessJobs.emit(f64::from(jobs));
    }
    // The index is published with the size, so a long stage can be found without bisecting.
    if let Some((at, ms)) = slowest_stage(&census.hud_stage_millis) {
        perf::Counter::HudStageMaxScheduleMs.emit(f64::from(ms));
        perf::Counter::HudStageMaxScheduleAt.emit(at as f64);
    }
    emit(perf::Counter::UiHudSetupMs, census.overlay_root_prep_millis);
    emit(
        perf::Counter::UiApplyDeferredMs,
        census.display_deferred_apply_millis,
    );
    emit(perf::Counter::UiHudVisibilityMs, census.overlay_visibility_millis);
}

pub(super) fn add_to_app(app: &mut App) {
    app.init_resource::<FrameUpdateTally>().add_systems(
        Update,
        (
            reset.before(ClientStep::Load),
            mark_present_start.in_set(ClientStep::Send),
            // Last: after Present and Ui, and before the next frame's reset.
            publish.in_set(ClientStep::Diag),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_measured_stage_has_no_slowest() {
        assert_eq!(slowest_stage(&[None; OVERLAY_STAGE_TOTAL]), None);
    }

    #[test]
    fn the_slowest_stage_is_reported_with_its_index() {
        let mut stages = [None; OVERLAY_STAGE_TOTAL];
        stages[2] = Some(0.5);
        stages[6] = Some(2.25);
        stages[8] = Some(1.0);
        assert_eq!(slowest_stage(&stages), Some((6, 2.25)));
    }

    #[test]
    fn a_tie_goes_to_the_earlier_stage() {
        let stages = [Some(1.0), None, Some(1.0)];
        assert_eq!(slowest_stage(&stages), Some((0, 1.0)));
    }

    #[test]
    fn the_start_of_a_frame_clears_every_stamp() {
        let mut app = App::new();
        app.insert_resource(FrameUpdateTally {
            hud_tess_jobs: Some(4),
            hud_stage_millis: [Some(1.0); OVERLAY_STAGE_TOTAL],
            ..Default::default()
        });
        app.add_systems(Update, reset);
        app.update();
        let census = app.world().resource::<FrameUpdateTally>();
        assert_eq!(census.hud_tess_jobs, None);
        assert!(census.hud_stage_millis.iter().all(Option::is_none));
    }

    #[test]
    fn present_start_is_stamped() {
        let mut app = App::new();
        app.init_resource::<FrameUpdateTally>();
        app.add_systems(Update, mark_present_start);
        app.update();
        assert!(app.world().resource::<FrameUpdateTally>().display_began.is_some());
    }
}
