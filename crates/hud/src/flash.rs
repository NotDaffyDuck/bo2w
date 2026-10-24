//! The flashbang whiteout: a full-screen white quad over a faded copy of the
//! saved screen, packed once and rewritten only when its alpha or the window
//! size changes.

use bevy::prelude::*;
use frame::{AppScreen, LifeStarted};
use hud_t6::flashbang::{FlashDraw, ShockViewType, shellshock_flash_draw, shellshock_remaining_ms};
use net::{FrameClock, LocalPresentClient, PresentedSnapshot};

use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, Draw2dQuad, tessellate};
use crate::gaps::{GapCause, HudGap, HudPresentationGaps};
use crate::gpu_list::{PackedList, pack_modulate, pack_saved_screen};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};
use crate::surface::Hud2dSurface;

const WHITE_IMAGE: &str = "white";

/// The packed whiteout and the state it was packed for.
#[derive(Resource, Default)]
pub(crate) struct FlashWhiteoutLatch {
    pub(crate) saved: PackedList,
    pub(crate) packed: PackedList,
    packed_alpha: Option<(f32, f32)>,
    packed_width: Option<f32>,
    packed_height: Option<f32>,
    shock_start: Option<i32>,
    pub(crate) save_sequence: u64,
}

impl FlashWhiteoutLatch {
    /// A new life: nothing from the last one may still be on screen.
    fn forget(&mut self) {
        self.packed = PackedList::default();
        self.saved = PackedList::default();
        self.packed_alpha = None;
        self.shock_start = None;
    }

    fn is_current(&self, alpha: (f32, f32), width: f32, height: f32) -> bool {
        !self.packed.is_empty()
            && self.packed_alpha == Some(alpha)
            && self.packed_width == Some(width)
            && self.packed_height == Some(height)
    }
}

/// What the flush should do with the latch this frame.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashGpuJob {
    #[default]
    Idle,
    Hide,
    Write,
}

impl FlashGpuJob {
    /// Asks for a hide, but only when something is packed to hide.
    fn hide_if_packed(&mut self, latch: &FlashWhiteoutLatch) {
        if !latch.packed.is_empty() {
            *self = FlashGpuJob::Hide;
        }
    }
}

/// One full-window white quad with the given alpha, tessellated.
fn full_window_quad(width: f32, height: f32, alpha: f32, site: &'static str) -> Option<Draw2dQuad> {
    let list = Draw2dList {
        cmds: vec![Draw2dCmd {
            material_namespace: HUD_CHROME_NAMESPACE,
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
            color: [1.0, 1.0, 1.0, alpha],
            material: WHITE_IMAGE.into(),
            op: Draw2dOp::StretchPic,
            provenance: Draw2dProvenance::CgDraw { site },
            layer: 0,
        }],
    };
    tessellate(&list).0.into_iter().next()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_flash_whiteout(
    clock: Res<FrameClock>,
    screen: Res<AppScreen>,
    surface: Res<Hud2dSurface>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut latch: ResMut<FlashWhiteoutLatch>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut job: ResMut<FlashGpuJob>,
    mut started: MessageReader<LifeStarted>,
) {
    *job = FlashGpuJob::Idle;
    for life in started.read() {
        if life.client == local.0.0 {
            latch.forget();
        }
    }
    gaps.clear(HudGap::FlashWhiteout);

    if !matches!(*screen, AppScreen::InGame) || !surface.is_ready() {
        job.hide_if_packed(&latch);
        return;
    }
    let (Some(player), Some(shock)) = (presented.player(local.0), presented.shellshock(local.0))
    else {
        job.hide_if_packed(&latch);
        return;
    };

    let remaining_ms =
        shellshock_remaining_ms(clock.time(), player.shellshock_time, player.shellshock_duration)
            .filter(|_| shock.screen_blend.kind == ShockViewType::Flashed)
            .unwrap_or(0);
    // The saved screen is this system's own latch, so the draw only needs the blend.
    let FlashDraw::Blend {
        whiteout,
        screengrab,
    } = shellshock_flash_draw(
        player.shellshock_time,
        player.shellshock_duration,
        remaining_ms,
        &shock.screen_blend,
        true,
    )
    else {
        latch.shock_start = None;
        job.hide_if_packed(&latch);
        return;
    };

    if latch.shock_start != Some(player.shellshock_time) {
        latch.shock_start = Some(player.shellshock_time);
        latch.save_sequence = latch.save_sequence.wrapping_add(1);
        latch.packed_alpha = None;
    }

    let (width, height) = (surface.width(), surface.height());
    let Some(white) = hud_images.get(HUD_CHROME_NAMESPACE, WHITE_IMAGE, &mut images) else {
        gaps.raise(GapCause::FlashWhiteoutImageMissing {
            name: WHITE_IMAGE.to_owned(),
            miss: hud_images.miss_reason(),
        });
        job.hide_if_packed(&latch);
        return;
    };
    if latch.is_current((whiteout, screengrab), width, height) {
        *job = FlashGpuJob::Write;
        return;
    }

    let (Some(saved_quad), Some(white_quad)) = (
        full_window_quad(width, height, screengrab, "flash_saved_screen"),
        full_window_quad(width, height, whiteout, "flash_whiteout"),
    ) else {
        job.hide_if_packed(&latch);
        return;
    };
    latch.saved = pack_saved_screen(&saved_quad, white.clone());
    latch.packed = pack_modulate(&white_quad, white);
    latch.packed_width = Some(width);
    latch.packed_height = Some(height);
    latch.packed_alpha = Some((whiteout, screengrab));
    *job = FlashGpuJob::Write;
}
