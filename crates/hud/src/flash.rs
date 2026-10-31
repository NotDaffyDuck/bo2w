//! The flashbang whiteout: a full-screen white quad over a faded copy of the
//! saved screen, packed once and rewritten only when its alpha or the window
//! size changes.

use bevy::prelude::*;
use frame::ShellScreen;
use frame::SpawnLifeBegun;
use hud_t6::flashbang::FlashDraw;
use hud_t6::flashbang::ShellshockScreenBlend;
use hud_t6::flashbang::ShockViewType;
use hud_t6::flashbang::shellshock_flash_draw;
use hud_t6::flashbang::shellshock_left_ms;
use net::{FrameClock, HostViewClient, ShownWorldFrame};

use crate::draw2d::FlatPaintEntry;
use crate::draw2d::FlatPaintQueue;
use crate::draw2d::FlatPaintAction;
use crate::draw2d::FlatPaintOrigin;
use crate::draw2d::FlatPaintRect;
use crate::gaps::GapReason;
use crate::gaps::HudGapBook;
use crate::gaps::HudShortage;
use crate::draw_batch::LayerList;
use crate::draw_batch::pack_plain;
use crate::draw_batch::pack_saved_screen;
use crate::images::{CHROME_NAMESPACE, HudPictures};
use crate::surface::FlatOverlayCanvas;
use crate::draw2d::expand_paint_list;

const WHITE_IMAGE: &str = "white";

/// What a packed whiteout was built for: the two intensities and the window.
#[derive(Clone, Copy, PartialEq, Debug)]
struct PackedFor {
    intensities: (f32, f32),
    window: (f32, f32),
}

/// The packed whiteout and what it was packed for.
#[derive(Resource, Default)]
pub(crate) struct WhiteoutCache {
    /// The faded copy of the saved screen.
    pub(crate) grab: LayerList,
    /// The white quad over it.
    pub(crate) overlay: LayerList,
    packed_for: Option<PackedFor>,
    flash_began: Option<i32>,
    pub(crate) grab_serial: u64,
}

impl WhiteoutCache {
    /// A new life: nothing from the last one may still be on screen.
    fn forget(&mut self) {
        self.overlay = LayerList::default();
        self.grab = LayerList::default();
        self.packed_for = None;
        self.flash_began = None;
    }

    /// Whether the packed lists already match this frame's want.
    fn is_current(&self, wanted: PackedFor) -> bool {
        !self.overlay.is_empty() && self.packed_for == Some(wanted)
    }

    /// Notes which flash is running; a new one asks for a fresh screen grab
    /// and a repack.
    fn follow_flash(&mut self, began: i32) {
        if self.flash_began != Some(began) {
            self.flash_began = Some(began);
            self.grab_serial = self.grab_serial.wrapping_add(1);
            self.packed_for = None;
        }
    }
}

/// What the flush should do with the cache this frame.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WhiteoutJob {
    #[default]
    Untouched,
    Retire,
    Upload,
}

impl WhiteoutJob {
    /// Asks to retire the packed lists, but only when there are some.
    fn retire_if_packed(&mut self, cache: &WhiteoutCache) {
        if !cache.overlay.is_empty() {
            *self = WhiteoutJob::Retire;
        }
    }
}

/// One full-window white quad with the given alpha, tessellated.
fn white_cover(span_w: f32, span_h: f32, alpha: f32, site: &'static str) -> Option<FlatPaintRect> {
    let cover = FlatPaintEntry {
        surface_lineage: CHROME_NAMESPACE,
        x: 0.0,
        y: 0.0,
        w: span_w,
        h: span_h,
        s0: 0.0,
        t0: 0.0,
        s1: 1.0,
        t1: 1.0,
        rgba: [1.0_f32, 1.0, 1.0, alpha],
        paint_material: WHITE_IMAGE.into(),
        op: FlatPaintAction::Stretched,
        sample_origin_tag: FlatPaintOrigin::ScenePaint { site },
        draw_layer: 0,
    };
    expand_paint_list(&FlatPaintQueue { cmds: vec![cover] }).0.into_iter().next()
}

/// The (whiteout, screengrab) intensities of a flash that began at `began`
/// and lasts `duration`, or `None` when none is showing. Only a flashed shock
/// counts; a blurred stun draws elsewhere.
fn flash_intensities(
    now: i32,
    began: i32,
    span_ms: i32,
    blend: &ShellshockScreenBlend,
) -> Option<(f32, f32)> {
    let left = shellshock_left_ms(now, began, span_ms)
        .filter(|_| blend.kind == ShockViewType::Flashed)
        .unwrap_or(0);
    // The saved screen is this system's own cache, so the draw only needs the blend.
    match shellshock_flash_draw(began, span_ms, left, blend, true) {
        FlashDraw::Blend {
            whiteout,
            screengrab,
        } => Some((whiteout, screengrab)),
        FlashDraw::Clear | FlashDraw::SaveScreen => None,
    }
}

/// Packs both layers for the wanted intensities and window.
fn repack(cache: &mut WhiteoutCache, white: &Handle<Image>, wanted: PackedFor) -> bool {
    let ((whiteout, screengrab), (width, height)) = (wanted.intensities, wanted.window);
    let (Some(grab_quad), Some(cover_quad)) = (
        white_cover(width, height, screengrab, "flash_saved_screen"),
        white_cover(width, height, whiteout, "flash_whiteout"),
    ) else {
        return false;
    };
    cache.grab = pack_saved_screen(&grab_quad, white.clone());
    cache.overlay = pack_plain(&cover_quad, white.clone());
    cache.packed_for = Some(wanted);
    true
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_whiteout(
    frame_clock: Res<FrameClock>,
    app_screen: Res<ShellScreen>,
    surface: Res<FlatOverlayCanvas>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    mut cache: ResMut<WhiteoutCache>,
    mut pictures: ResMut<HudPictures>,
    mut image_store: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudGapBook>,
    mut job: ResMut<WhiteoutJob>,
    mut started: MessageReader<SpawnLifeBegun>,
) {
    *job = WhiteoutJob::Untouched;
    if started.read().any(|life| life.client == local.0.0) {
        cache.forget();
    }
    gaps.settle(HudShortage::FlashWhite);

    if *app_screen != ShellScreen::InGame || !surface.has_size() {
        job.retire_if_packed(&cache);
        return;
    }
    let (Some(player), Some(shock)) = (presented.player(local.0), presented.shellshock(local.0))
    else {
        job.retire_if_packed(&cache);
        return;
    };
    let Some(intensities) = flash_intensities(
        frame_clock.time(),
        player.stun_start,
        player.stun_length,
        &shock.screen_blend,
    ) else {
        cache.flash_began = None;
        job.retire_if_packed(&cache);
        return;
    };
    cache.follow_flash(player.stun_start);

    let Some(white) = pictures.get(CHROME_NAMESPACE, WHITE_IMAGE, &mut image_store) else {
        gaps.log_gap(GapReason::FlashImageAbsent {
            asset_key: WHITE_IMAGE.to_owned(),
            miss: pictures.lookup_failure(),
        });
        job.retire_if_packed(&cache);
        return;
    };
    let wanted = PackedFor {
        intensities,
        window: (surface.width(), surface.height()),
    };
    if cache.is_current(wanted) || repack(&mut cache, &white, wanted) {
        *job = WhiteoutJob::Upload;
    } else {
        job.retire_if_packed(&cache);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flashed(fade_ms: i32) -> ShellshockScreenBlend {
        ShellshockScreenBlend {
            blurred_fade_ms: 0,
            blurred_effect_ms: 0,
            flash_white_fade_ms: fade_ms,
            flash_shot_fade_ms: fade_ms,
            kind: ShockViewType::Flashed,
        }
    }

    fn packed(for_what: PackedFor) -> WhiteoutCache {
        let mut cache = WhiteoutCache::default();
        let white = Handle::<Image>::default();
        assert!(repack(&mut cache, &white, for_what));
        cache
    }

    const WANT: PackedFor = PackedFor {
        intensities: (0.5, 0.25),
        window: (1280.0, 720.0),
    };

    #[test]
    fn no_flash_before_it_begins_or_without_a_start() {
        let blend = flashed(1000);
        assert_eq!(flash_intensities(500, 0, 3000, &blend), None);
        assert_eq!(flash_intensities(500, 1000, 3000, &blend), None);
    }

    #[test]
    fn a_running_flash_has_intensities_in_range() {
        let (whiteout, screengrab) = flash_intensities(1100, 1000, 3000, &flashed(1000))
            .expect("a flash is running");
        assert!((0.0..=1.0).contains(&whiteout) && (0.0..=1.0).contains(&screengrab));
    }

    #[test]
    fn a_blurred_stun_is_not_a_flash() {
        let mut blend = flashed(1000);
        blend.kind = ShockViewType::Blurred;
        assert_eq!(flash_intensities(1100, 1000, 3000, &blend), None);
    }

    #[test]
    fn a_finished_flash_is_gone() {
        assert_eq!(flash_intensities(5000, 1000, 3000, &flashed(1000)), None);
    }

    #[test]
    fn the_cover_is_one_quad_the_size_of_the_window() {
        let quad = white_cover(640.0, 480.0, 0.5, "test").expect("one quad");
        assert!((quad.rgba[3] - 0.5).abs() < 1.0 / 255.0, "{:?}", quad.rgba);
        let xs = quad.corners.map(|corner| corner[0]);
        let ys = quad.corners.map(|corner| corner[1]);
        assert_eq!(xs.iter().cloned().fold(f32::MIN, f32::max), 640.0);
        assert_eq!(ys.iter().cloned().fold(f32::MIN, f32::max), 480.0);
    }

    #[test]
    fn a_repack_is_current_only_for_what_it_packed() {
        let cache = packed(WANT);
        assert!(cache.is_current(WANT));
        let other_window = PackedFor {
            window: (1920.0, 1080.0),
            ..WANT
        };
        let other_alpha = PackedFor {
            intensities: (0.6, 0.25),
            ..WANT
        };
        assert!(!cache.is_current(other_window));
        assert!(!cache.is_current(other_alpha));
        assert!(!WhiteoutCache::default().is_current(WANT));
    }

    #[test]
    fn a_new_flash_bumps_the_grab_serial_and_forces_a_repack() {
        let mut cache = packed(WANT);
        cache.follow_flash(1000);
        assert_eq!(cache.grab_serial, 1);
        assert!(!cache.is_current(WANT));
        let again = packed(WANT);
        cache.packed_for = again.packed_for;
        cache.follow_flash(1000);
        assert_eq!(cache.grab_serial, 1, "the same flash keeps its grab");
        assert!(cache.packed_for.is_some());
        cache.follow_flash(2000);
        assert_eq!(cache.grab_serial, 2);
    }

    #[test]
    fn a_new_life_empties_the_cache() {
        let mut cache = packed(WANT);
        cache.follow_flash(1000);
        cache.forget();
        assert!(cache.overlay.is_empty() && cache.grab.is_empty());
        assert_eq!(cache.packed_for, None);
        assert_eq!(cache.flash_began, None);
    }

    #[test]
    fn retiring_needs_something_packed() {
        let mut job = WhiteoutJob::Untouched;
        job.retire_if_packed(&WhiteoutCache::default());
        assert_eq!(job, WhiteoutJob::Untouched);
        job.retire_if_packed(&packed(WANT));
        assert_eq!(job, WhiteoutJob::Retire);
    }
}
