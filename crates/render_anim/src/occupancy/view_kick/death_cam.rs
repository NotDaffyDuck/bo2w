//! The death-camera log: what the view was made from each frame, and a count of the frames that jumped.

use bevy::prelude::*;
use net::FrameClock;

use super::camera::EyeCameraFilter;
use super::flags::death_cam_log_on;
use super::state::SessionViewKick;

impl SessionViewKick {
    /// bo2mp: BO2MP_DEATHCAMLOG=1 writes one line each time the view changes
    /// what it is made from (alive, eye = his own eyes while dead, corpse =
    /// his body, spectator, killcam), with the time, so a glitch after a
    /// death can be matched to the moment the source changed.
    pub(super) fn note_death_cam_source(&mut self, time_ms: i32, source: &'static str, pm_type: i32) {
        if self.death_cam_source == Some(source) {
            return;
        }
        if death_cam_log_on() {
            diag::info!(
                Fpv,
                "deathcam: t={} source={} was={} pm_type={}",
                time_ms,
                source,
                self.death_cam_source.unwrap_or("none"),
                pm_type
            );
        }
        self.death_cam_source = Some(source);
    }
}

/// One frame's camera move over this many units is logged as a jump.
const DEATH_CAM_JUMP_UNITS: f32 = 100.0;

/// What `log_death_cam_jumps` saw since he died.
#[derive(Default)]
pub struct DeathCamWatch {
    last: Option<(Vec3, &'static str)>,
    frames: u32,
    jumps: u32,
    max_move: f32,
}

/// bo2mp: BO2MP_DEATHCAMLOG=1 also follows the camera every frame from his death to his
/// respawn: a move of over 100 units in one frame is logged with what the view was made
/// from before and after it, and each death ends with a count line (row 33: the death
/// camera flying across the map and back).
pub fn log_death_cam_jumps(
    clock: Res<FrameClock>,
    kick: Res<SessionViewKick>,
    cameras: Query<&Transform, EyeCameraFilter>,
    mut watch: Local<DeathCamWatch>,
) {
    if !death_cam_log_on() {
        return;
    }
    let Some(at) = cameras.iter().next().map(|t| t.translation) else {
        return;
    };
    let source = kick.death_cam_source.unwrap_or("none");
    let was = watch.last.map_or("none", |(_, s)| s);
    if source == "alive" && was == "alive" {
        watch.last = Some((at, source));
        return;
    }
    if let Some((before, _)) = watch.last {
        let moved = before.distance(at);
        if source != "alive" {
            watch.frames += 1;
            watch.max_move = watch.max_move.max(moved);
        }
        if moved > DEATH_CAM_JUMP_UNITS {
            watch.jumps += 1;
            diag::info!(
                Fpv,
                "deathcam jump: t={} moved={:.0} source={} was={} from=({:.0} {:.0} {:.0}) to=({:.0} {:.0} {:.0})",
                clock.time(),
                moved,
                source,
                was,
                before.x,
                before.y,
                before.z,
                at.x,
                at.y,
                at.z
            );
        }
    }
    if source == "alive" && was != "alive" && watch.frames > 0 {
        diag::info!(
            Fpv,
            "deathcam life: t={} frames={} jumps={} max_move={:.0}",
            clock.time(),
            watch.frames,
            watch.jumps,
            watch.max_move
        );
        watch.frames = 0;
        watch.jumps = 0;
        watch.max_move = 0.0;
    }
    watch.last = Some((at, source));
}
