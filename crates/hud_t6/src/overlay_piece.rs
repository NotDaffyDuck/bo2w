//! BO2 script HUD elements (hudelem_s): the 17 types, flags, slot banks, script defaults, the
//! colour fade (truncating), font-scale lerp and font map, placement with move-over-time and the
//! whole-pixel rule, material size, the per-pass draw filter and the draw order.
//! All hudelem types are drawn in C (no LUI); the on-screen waypoint is a 3D surface.
//! Facts: scratch/hud-facts/hudelem.md.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::scrplace::{DisplayAnchoring, HIRES_SCALE_X};

/// hud.current[] and hud.archival[] each hold this many elems.
pub const OVERLAY_PIECE_LIMIT: usize = 0x1f;
/// hudelem_s (game_hudelem_s puts clientNum right after it).
pub const HUDELEM_STRIDE: usize = 0x7c;
/// game_hudelem_s: hudelem_s + clientNum + team + archived.
pub const GAME_HUDELEM_STRIDE: usize = 0x88;

/// he_type_t.
pub const PIECE_UNUSED: i32 = 0;
pub const ELEM_KIND_TEXT: i32 = 1;
pub const ELEM_KIND_VALUE: i32 = 2;
pub const ELEM_KIND_PLAYERNAME: i32 = 3;
pub const HE_TYPE_MAPNAME: i32 = 4;
pub const HE_TYPE_GAMETYPE: i32 = 5;
pub const HE_TYPE_DAMAGE_INDICATOR: i32 = 6;
pub const HE_TYPE_SCORE: i32 = 7;
pub const HE_TYPE_MATERIAL: i32 = 8;
pub const HE_TYPE_TIMER_DOWN: i32 = 9;
pub const HE_TYPE_TIMER_UP: i32 = 10;
pub const HE_TYPE_TENTHS_TIMER_DOWN: i32 = 11;
pub const HE_TYPE_TENTHS_TIMER_UP: i32 = 12;
pub const HE_TYPE_CLOCK_DOWN: i32 = 13;
pub const HE_TYPE_CLOCK_UP: i32 = 14;
pub const HE_TYPE_WAYPOINT: i32 = 15;
pub const HE_TYPE_PERKS: i32 = 16;
pub const HE_TYPE_COUNT: i32 = 17;

pub const HUDELEM_TYPE_NAMES: [&str; HE_TYPE_COUNT as usize] = [
    "free",
    "text",
    "value",
    "playername",
    "mapname",
    "gametype",
    "damage indicator",
    "score",
    "material",
    "timer down",
    "timer up",
    "tenths timer down",
    "tenths timer up",
    "clock down",
    "clock up",
    "waypoint",
    "perks",
];

/// hudelem_s flags.
pub mod piece_bits {
    pub const FOREGROUND: u32 = 0x1;
    pub const HIDEWHENDEAD: u32 = 0x2;
    pub const HIDEWHENINMENU: u32 = 0x4;
    pub const FONTSTYLE3D_SHADOWED: u32 = 0x8;
    pub const FONTSTYLE3D_SHADOWED_MORE: u32 = 0x10;
    pub const FONT3D_USE_GLOWCOLOR: u32 = 0x20;
    pub const HIDEWHENINKILLCAM: u32 = 0x40;
    pub const REDACT: u32 = 0x80;
    pub const FADEWHENTARGETED: u32 = 0x100;
    pub const HIDEWHENINDEMO: u32 = 0x200;
    pub const HIDEWHILEREMOTECONTROLING: u32 = 0x400;
    pub const COD7DECODE: u32 = 0x800;
    pub const NO_OFFSCREEN_WAYPOINT: u32 = 0x1000;
    pub const HIDEWHENINSCOPE: u32 = 0x2000;
    pub const TYPEWRITER: u32 = 0x4000;
    pub const IMMUNE_TO_DEMO_GAMEHUD: u32 = 0x8000;
    pub const IMMUNE_TO_DEMO_FREECAMERA: u32 = 0x10000;
    /// Shown to spectators of the elem's team (team elems only).
    pub const SHOW_PLAYER_TEAM_TO_SPECTATOR: u32 = 0x20000;
    /// newDebugHudElem: never copied to a client bank.
    pub const DEBUG: u32 = 0x40000;
}

/// Older spelling other crates still call; the rename sweep moves it over.

/// HudElem_SetDefaults: no target entity.
pub const HUDELEM_NO_TARGET_ENT: i32 = 0x3ff;
/// HudElem_SetDefaults: not bound to a UI3D window.
pub const HUDELEM_NO_UI3D_WINDOW: i32 = -1;
/// newHudElem / newTeamHudElem: clientNum for an elem every client may see.
pub const HUDELEM_ALL_CLIENTS: i32 = 0x3ff;
/// g_hudelems: slots in the server's elem pool.
pub const SCRIPT_PIECE_POOL: usize = 1308;
/// SetPulseFX: each client's sound ids wrap through 1..=31.
pub const HUDELEM_PULSE_SOUND_IDS: i32 = 32;
/// Config strings a MAPNAME / GAMETYPE elem prints.
pub const CS_MAPNAME: usize = 0x13;
pub const CS_GAMETYPE: usize = 0x14;

/// eFlags2 bit for riding a remote vehicle: keeps HIDEWHENDEAD elems and hides
/// HIDEWHILEREMOTECONTROLING ones.
pub const EF2_REMOTE_CONTROLLING: u32 = 0x1000_0000;
/// movement_mode below this counts as alive for HIDEWHENDEAD.
pub const PM_TYPE_DEAD_FIRST: i32 = 10;
/// movement_mode of a player watching the killcam.
pub const PM_TYPE_KILLCAM: i32 = 5;

/// PROFILE_SHOUTCASTER_PERKS values that change PERKS elems.
pub const SHOUTCASTER_PERKS_HIDDEN: i32 = 0;
pub const SHOUTCASTER_PERKS_OPAQUE: i32 = 2;

/// hudelem_s, field for field (times in ms, colours r in the low byte).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayPiece {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub text_size: f32,
    pub text_size_was: f32,
    pub text_grow_from: i32,
    pub tint_rgba: u32,
    pub tint_was: u32,
    pub fade_from: i32,
    pub grow_from: i32,
    pub was_x: f32,
    pub was_y: f32,
    pub slide_from: i32,
    pub time: i32,
    pub lasts: i32,
    pub val: f32,
    pub sort: f32,
    pub glow_rgba: u32,
    pub fx_born: i32,
    pub bits: u32,
    pub aim_ent: i32,
    pub text_grow_ms: i32,
    pub fade_len: i32,
    /// String-table index of the label (drawn first).
    pub caption: i32,
    pub width: i32,
    pub height: i32,
    pub was_wide: i32,
    pub was_tall: i32,
    pub grow_ms: i32,
    pub slide_ms: i32,
    /// String-table index of the text.
    pub words: i32,
    pub fx_char_ms: i32,
    pub fx_fade_from: i32,
    pub fx_fade_len: i32,
    pub fx_redact_decay_start_time: i32,
    pub fx_redact_decay_duration: i32,
    pub piece_kind: i32,
    pub font: i32,
    pub anchor: i32,
    pub screen_anchor: i32,
    /// Server material table index (CG_ServerMaterialName).
    pub skin_no: i32,
    pub offscreen_material_index: i32,
    pub anchor_was: i32,
    pub screen_anchor_was: i32,
    pub sound_no: i32,
    /// Negative: drawn on the 2D HUD; else bound to that UI3D window.
    pub ui3d_window: i32,
}

impl OverlayPiece {
    /// HudElem_SetDefaults: all zero but TEXT, opaque white, font scale 1, no target entity and
    /// no UI3D window.
    #[must_use]
    pub fn script_default() -> Self {
        Self {
            piece_kind: ELEM_KIND_TEXT,
            tint_rgba: 0xffff_ffff,
            text_size: 1.0,
            aim_ent: HUDELEM_NO_TARGET_ENT,
            ui3d_window: HUDELEM_NO_UI3D_WINDOW,
            ..Self::default()
        }
    }
}

/// game_hudelem_s beyond the elem itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptOverlayPiece {
    pub seat: i32,
    pub side: i32,
    pub archived: bool,
}

impl Default for ScriptOverlayPiece {
    /// HudElem_SetDefaults sets archived.
    fn default() -> Self {
        Self { seat: 0, side: 0, archived: true }
    }
}

pub const H_CENTER: i32 = 2;
pub const V_MIDDLE: i32 = 2;
/// alignScreen: horizontal in bits 4-7, vertical in bits 0-3.
pub const ALIGN_SCREEN_HORZ_SHIFT: i32 = 4;
const ALIGN_SCREEN_FIELD: i32 = 15;
/// alignOrg: horizontal in bits 2-3, vertical in bits 0-1.
const ALIGN_ORG_HORZ_SHIFT: i32 = 2;
const ALIGN_ORG_FIELD: i32 = 3;
pub const ORG_LEADING: i32 = 0;
pub const ORG_MIDDLE: i32 = 1;
pub const ORG_TRAILING: i32 = 2;
/// s_alignScale.
const ALIGN_SCALE: [f32; 4] = [0.0, 0.5, 1.0, 0.0];

#[must_use]
pub const fn align_screen(across_axis: i32, down: i32) -> i32 {
    (across_axis << ALIGN_SCREEN_HORZ_SHIFT) | down
}

#[must_use]
pub const fn align_org(across_axis: i32, down: i32) -> i32 {
    (across_axis << ALIGN_ORG_HORZ_SHIFT) | down
}

/// (horizontal, vertical) ScrPlace align of an alignScreen byte.
#[must_use]
pub const fn hud_elem_screen_align(align_screen: i32) -> (i32, i32) {
    ((align_screen >> ALIGN_SCREEN_HORZ_SHIFT) & ALIGN_SCREEN_FIELD, align_screen & ALIGN_SCREEN_FIELD)
}

#[must_use]
pub const fn pack_color(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    // Red in the lowest byte, alpha in the highest.
    u32::from_le_bytes([red, green, blue, alpha])
}

#[must_use]
pub const fn expand_color_word(dword: u32) -> [u8; 4] {
    dword.to_le_bytes()
}

/// A start-time/duration lerp fraction, or None once done (or when the duration is not positive).
fn lerp_frac(now: i32, start: i32, duration: i32) -> Option<f32> {
    let elapsed = now.wrapping_sub(start);
    (duration > 0 && elapsed < duration).then(|| elapsed.max(0) as f32 / duration as f32)
}

/// BG_LerpHudColors: from fromColor to color over fadeTime; each channel TRUNCATED
/// ((int)((to - from) * t + from)).
#[must_use]
pub fn lerp_hud_colors(piece: &OverlayPiece, now: i32) -> [u8; 4] {
    let to = expand_color_word(piece.tint_rgba);
    let Some(t) = lerp_frac(now, piece.fade_from, piece.fade_len) else { return to };
    let from = expand_color_word(piece.tint_was);
    core::array::from_fn(|i| ((to[i] as f32 - from[i] as f32) * t + from[i] as f32) as u8)
}

/// BG_LerpFontScale: fromFontScale to fontScale over fontScaleTime.
#[must_use]
pub fn blend_piece_type_size(piece: &OverlayPiece, now: i32) -> f32 {
    match lerp_frac(now, piece.text_grow_from, piece.text_grow_ms) {
        Some(t) => piece.text_size_was + (piece.text_size - piece.text_size_was) * t,
        None => piece.text_size,
    }
}

/// A font-scale animation is still running (positions are then not rounded).
#[must_use]
pub fn hud_elem_font_scale_animating(piece: &OverlayPiece, now: i32) -> bool {
    lerp_frac(now, piece.text_grow_from, piece.text_grow_ms).is_some()
}

/// The elem's font byte to (UI font id, base scale). Other values assert in BO2.
#[must_use]
pub const fn hud_elem_font(font: i32) -> Option<(i32, f32)> {
    use crate::font::{
        UI_FONT_BIG, UI_FONT_BOLD, UI_FONT_CONSOLE, UI_FONT_DEFAULT, UI_FONT_EXTRABIG, UI_FONT_EXTRASMALL,
        UI_FONT_NORMAL, UI_FONT_SMALL,
    };
    Some(match font {
        0 => (UI_FONT_DEFAULT, 0.25),
        1 => (UI_FONT_BOLD, 0.5),
        2 => (UI_FONT_CONSOLE, 0.3333),
        3 => (UI_FONT_NORMAL, 0.25),
        4 => (UI_FONT_BIG, 0.25),
        5 => (UI_FONT_SMALL, 0.25),
        6 => (UI_FONT_EXTRABIG, 0.25),
        7 => (UI_FONT_EXTRASMALL, 0.25),
        _ => return None,
    })
}

/// GetHudElemInfo's final font scale: lerped scale * the font's base * cg_hudSplitscreenScale *
/// the hi-res X scale.
#[must_use]
pub fn hud_elem_font_scale(place: &DisplayAnchoring, piece: &OverlayPiece, now: i32) -> Option<f32> {
    let (_, base) = hud_elem_font(piece.font)?;
    Some(blend_piece_type_size(piece, now) * base * place.hud_splitscreen_scale * HIRES_SCALE_X)
}

/// GetSortedHudElems' per-bank walk: everything before the first FREE slot.
#[must_use]
pub fn clone_occupied_head(src: &[OverlayPiece]) -> &[OverlayPiece] {
    let n = src.iter().take(OVERLAY_PIECE_LIMIT).position(|e| e.piece_kind == PIECE_UNUSED);
    &src[..n.unwrap_or(src.len().min(OVERLAY_PIECE_LIMIT))]
}

/// GetSortedHudElems: current[] then archival[], each up to its first FREE slot, sorted by sort.
/// TODO(fact): the sort step was not re-read; equal sorts keep bank order here.
#[must_use]
pub fn sorted_hud_elems<'a>(current: &'a [OverlayPiece], archival: &'a [OverlayPiece]) -> Vec<&'a OverlayPiece> {
    let mut list: Vec<&OverlayPiece> = clone_occupied_head(current).iter().chain(clone_occupied_head(archival)).collect();
    list.sort_by(|a, b| a.sort.total_cmp(&b.sort));
    list
}

/// CG_Draw2dHudElems' "dead-visible": riding a remote vehicle or movement_mode below the dead types.
#[must_use]
pub const fn hud_dead_visible(e_flags2: u32, movement_mode: i32) -> bool {
    e_flags2 & EF2_REMOTE_CONTROLLING != 0 || movement_mode < PM_TYPE_DEAD_FIRST
}

/// CG_Draw2dHudElems' "killcam": a killcam entity or the killcam movement_mode.
#[must_use]
pub const fn hud_in_killcam(death_replay_target: i32, movement_mode: i32) -> bool {
    death_replay_target != 0 || movement_mode == PM_TYPE_KILLCAM
}

/// What CG_Draw2dHudElemList needs to know about the viewer this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HudElemViewer {
    pub dead_visible: bool,
    pub in_sniper_scope: bool,
    pub killcam: bool,
    /// CG_IsTvguided or eFlags2 EF2_REMOTE_CONTROLLING.
    pub remote_controlling: bool,
}

/// CG_Draw2dHudElemList's filter for one pass (false = background pass, true = foreground).
/// Demo_ShouldFilterHudElement can still veto afterwards.
#[must_use]
pub fn hud_elem_passes_filter(piece: &OverlayPiece, foreground_pass: bool, viewer: &HudElemViewer) -> bool {
    let has = |f: u32| piece.bits & f != 0;
    piece.ui3d_window < 0
        && has(piece_bits::FOREGROUND) == foreground_pass
        && (!has(piece_bits::HIDEWHENDEAD) || viewer.dead_visible)
        && !(has(piece_bits::HIDEWHENINSCOPE) && viewer.in_sniper_scope)
        && !(has(piece_bits::HIDEWHENINKILLCAM) && viewer.killcam)
        && !(has(piece_bits::HIDEWHILEREMOTECONTROLING) && viewer.remote_controlling)
}

/// What DrawSingleHudElem2d does with an elem's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudElemBody {
    Nothing,
    /// DrawHudElemString (text, value, names, score, timers).
    String,
    Material,
    Clock,
    Perks,
    /// Only the off-screen arrow; the on-screen part is a 3D surface.
    OffscreenWaypoint,
}

#[must_use]
pub const fn hud_elem_body(kind: i32, bits: u32) -> HudElemBody {
    match kind {
        ELEM_KIND_TEXT | ELEM_KIND_VALUE | ELEM_KIND_PLAYERNAME | HE_TYPE_MAPNAME | HE_TYPE_GAMETYPE | HE_TYPE_SCORE
        | HE_TYPE_TIMER_DOWN..=HE_TYPE_TENTHS_TIMER_UP => HudElemBody::String,
        HE_TYPE_MATERIAL => HudElemBody::Material,
        HE_TYPE_CLOCK_DOWN | HE_TYPE_CLOCK_UP => HudElemBody::Clock,
        HE_TYPE_PERKS => HudElemBody::Perks,
        HE_TYPE_WAYPOINT if bits & piece_bits::NO_OFFSCREEN_WAYPOINT == 0 => HudElemBody::OffscreenWaypoint,
        // FREE, DAMAGE_INDICATOR (asserts) and anything else.
        _ => HudElemBody::Nothing,
    }
}

/// The colour an elem draws with, or None when it is not drawn (alpha 0, or PERKS hidden for a
/// shoutcaster). `shoutcaster_perks` is the profile value while shoutcasting.
#[must_use]
pub fn hud_elem_draw_color(piece: &OverlayPiece, now: i32, shoutcaster_perks: Option<i32>) -> Option<[u8; 4]> {
    let mut color = lerp_hud_colors(piece, now);
    if piece.piece_kind == HE_TYPE_PERKS {
        match shoutcaster_perks {
            Some(SHOUTCASTER_PERKS_HIDDEN) => return None,
            Some(SHOUTCASTER_PERKS_OPAQUE) => color[3] = 255,
            _ => {}
        }
    }
    (color[3] != 0).then_some(color)
}

/// C's "%g" (VALUE and SCORE): 6 significant digits, trailing zeros dropped, exponent form below
/// 1e-4 or from 1e6.
#[must_use]
pub fn format_g(value: f32) -> String {
    let v = f64::from(value);
    if v.is_nan() {
        return String::from(if v.is_sign_negative() { "-nan" } else { "nan" });
    }
    if v.is_infinite() {
        return String::from(if v < 0.0 { "-inf" } else { "inf" });
    }
    if v == 0.0 {
        return String::from(if v.is_sign_negative() { "-0" } else { "0" });
    }
    let sci = format!("{v:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: &str| -> String {
        if s.contains('.') { String::from(s.trim_end_matches('0').trim_end_matches('.')) } else { String::from(s) }
    };
    if !(-4..6).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exp.unsigned_abs())
    } else {
        trim(&format!("{:.*}", (5 - exp) as usize, v))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudElemPlacement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl HudElemPlacement {
    #[must_use]
    pub fn text_baseline_y(&self) -> f32 {
        self.y + self.h
    }
}

/// Move-over-time fraction: 1 when moveTime <= 0 or done, 0 until a ms has passed.
#[must_use]
pub fn hud_elem_movement_frac(piece: &OverlayPiece, now: i32) -> f32 {
    if piece.slide_ms <= 0 {
        return 1.0;
    }
    let elapsed = now.wrapping_sub(piece.slide_from);
    if elapsed < 1 { 0.0 } else { (elapsed as f32 / piece.slide_ms as f32).min(1.0) }
}

/// Size-over-time fraction while scaleTime runs, else None.
#[must_use]
pub fn hud_elem_scale_frac(piece: &OverlayPiece, now: i32) -> Option<f32> {
    lerp_frac(now, piece.grow_from, piece.grow_ms)
}

/// GetHudElemOrg: ScrPlace_ApplyX/Y of the split-screen-scaled position by the alignScreen
/// nibbles, minus s_alignScale[alignOrg field] * the size.
#[must_use]
pub fn hud_elem_origin(
    place: &DisplayAnchoring,
    align_org: i32,
    align_screen: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> (f32, f32) {
    let (across_axis, vert) = hud_elem_screen_align(align_screen);
    let anchor = |shift: i32| ALIGN_SCALE[((align_org >> shift) & ALIGN_ORG_FIELD) as usize];
    (
        place.apply_pos(x, across_axis, 0, true) - anchor(ALIGN_ORG_HORZ_SHIFT) * width,
        place.apply_pos(y, vert, 1, true) - anchor(0) * height,
    )
}

/// HudElemMaterialWidth/Height: the virtual size by its align (full-screen scale for
/// ANCHOR_FULLSCREEN, else the 4:3 one), lerped from fromWidth/fromHeight while scaleTime runs.
/// TODO(fact): the scale branches and OffsetHudElemY are only partly known; a 0 size stays 0.
#[must_use]
pub fn hud_elem_material_size(place: &DisplayAnchoring, piece: &OverlayPiece, now: i32) -> (f32, f32) {
    let extents = |align_screen: i32, w: i32, h: i32| {
        let (across_axis, vert) = hud_elem_screen_align(align_screen);
        (place.apply_size(w as f32, across_axis, 0), place.apply_size(h as f32, vert, 1))
    };
    let (w, h) = extents(piece.screen_anchor, piece.width, piece.height);
    match hud_elem_scale_frac(piece, now) {
        Some(t) => {
            let (fw, fh) = extents(piece.screen_anchor_was, piece.was_wide, piece.was_tall);
            (fw + (w - fw) * t, fh + (h - fh) * t)
        }
        None => (w, h),
    }
}

/// SetHudElemPos: the from/to origins mixed by the move fraction. Once the move is done (or none),
/// the to-origin is rounded to whole pixels (half up) unless a font-scale animation runs.
#[must_use]
pub fn hud_elem_position(place: &DisplayAnchoring, piece: &OverlayPiece, now: i32, width: f32, height: f32) -> (f32, f32) {
    let to = hud_elem_origin(place, piece.anchor, piece.screen_anchor, piece.x, piece.y, width, height);
    let t = hud_elem_movement_frac(piece, now);
    if t >= 1.0 {
        if hud_elem_font_scale_animating(piece, now) {
            return to;
        }
        return (libm::floorf(to.0 + 0.5), libm::floorf(to.1 + 0.5));
    }
    let from = hud_elem_origin(place, piece.anchor_was, piece.screen_anchor_was, piece.was_x, piece.was_y, width, height);
    (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
}

/// Where an elem's body draws: materials by their size, everything else by the text width and
/// the font height.
#[must_use]
pub fn hud_elem_placement(
    place: &DisplayAnchoring,
    piece: &OverlayPiece,
    now: i32,
    text_across: f32,
    font_height: f32,
) -> HudElemPlacement {
    let (w, h) = if piece.piece_kind == HE_TYPE_MATERIAL {
        hud_elem_material_size(place, piece, now)
    } else {
        (text_across, font_height)
    };
    let (x, y) = hud_elem_position(place, piece, now, w, h);
    HudElemPlacement { x, y, w, h }
}

/// The glow colour, only with FONT3D_USE_GLOWCOLOR.
/// TODO(fact): whether the faded alpha multiplies it was not re-read.
#[must_use]
pub fn hud_elem_glow_color(piece: &OverlayPiece) -> Option<[u8; 4]> {
    (piece.bits & piece_bits::FONT3D_USE_GLOWCOLOR != 0).then(|| expand_color_word(piece.glow_rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::scrplace::{ANCHOR_CENTER, ANCHOR_SUBPART, PlacementInputs};

    fn place_640() -> DisplayAnchoring {
        DisplayAnchoring::setup_float_viewport(
            640.0,
            480.0,
            &PlacementInputs {
                display_size: [640.0, 480.0],
                safe_area: [1.0, 1.0],
                profile_safe_area: None,
                aspect_display_pixel: 1.0,
                dual_play_enable: false,
                hud_splitscreen_scale: None,
            },
        )
    }

    #[test]
    fn script_defaults() {
        let e = OverlayPiece::script_default();
        assert_eq!(e.piece_kind, ELEM_KIND_TEXT);
        assert_eq!(expand_color_word(e.tint_rgba), [255; 4]);
        assert_eq!((e.text_size, e.aim_ent, e.ui3d_window), (1.0, 0x3ff, -1));
        assert!(ScriptOverlayPiece::default().archived);
        assert_eq!(HUDELEM_TYPE_NAMES[HE_TYPE_MATERIAL as usize], "material");
        assert_eq!(HUDELEM_TYPE_NAMES[HE_TYPE_WAYPOINT as usize], "waypoint");
    }

    #[test]
    fn colour_fade_truncates() {
        let e = OverlayPiece {
            tint_rgba: pack_color(255, 0, 0, 255),
            tint_was: pack_color(0, 0, 0, 0),
            fade_from: 1000,
            fade_len: 1000,
            ..OverlayPiece::default()
        };
        // 255 * 0.5 = 127.5 truncates to 127 (MW2 rounded to 128).
        assert_eq!(lerp_hud_colors(&e, 1500), [127, 0, 0, 127]);
        assert_eq!(lerp_hud_colors(&e, 900), [0, 0, 0, 0]);
        assert_eq!(lerp_hud_colors(&e, 2000), [255, 0, 0, 255]);
        // Fading down truncates toward zero too.
        let down = OverlayPiece { tint_rgba: 0, tint_was: pack_color(255, 255, 255, 255), ..e };
        assert_eq!(lerp_hud_colors(&down, 1500), [127; 4]);
    }

    #[test]
    fn font_scale_and_map() {
        let e = OverlayPiece {
            font: 1,
            text_size: 2.0,
            text_size_was: 1.0,
            text_grow_from: 0,
            text_grow_ms: 100,
            ..OverlayPiece::default()
        };
        assert_eq!(blend_piece_type_size(&e, 50), 1.5);
        assert!(hud_elem_font_scale_animating(&e, 50) && !hud_elem_font_scale_animating(&e, 100));
        assert_eq!(hud_elem_font_scale(&place_640(), &e, 100), Some(1.0));
        assert_eq!(hud_elem_font(2), Some((crate::font::UI_FONT_CONSOLE, 0.3333)));
        assert_eq!(hud_elem_font(8), None);
    }

    #[test]
    fn banks_stop_at_first_free_then_sort() {
        let text = |sort: f32| OverlayPiece { piece_kind: ELEM_KIND_TEXT, sort, ..OverlayPiece::default() };
        let current = [text(3.0), text(1.0), OverlayPiece::default(), text(0.0)];
        let archival = [text(2.0)];
        let sorts: Vec<f32> = sorted_hud_elems(&current, &archival).iter().map(|e| e.sort).collect();
        assert_eq!(sorts, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn pass_filter() {
        let alive = HudElemViewer { dead_visible: true, ..HudElemViewer::default() };
        let e = OverlayPiece { bits: piece_bits::HIDEWHENDEAD, ui3d_window: -1, ..OverlayPiece::default() };
        assert!(hud_elem_passes_filter(&e, false, &alive));
        assert!(!hud_elem_passes_filter(&e, true, &alive));
        assert!(!hud_elem_passes_filter(&e, false, &HudElemViewer::default()));
        let fg = OverlayPiece { bits: piece_bits::FOREGROUND | piece_bits::HIDEWHENINSCOPE, ..e };
        assert!(hud_elem_passes_filter(&fg, true, &alive));
        assert!(!hud_elem_passes_filter(&fg, true, &HudElemViewer { in_sniper_scope: true, ..alive }));
        let in3d = OverlayPiece { ui3d_window: 0, ..e };
        assert!(!hud_elem_passes_filter(&in3d, false, &alive));
        assert!(hud_dead_visible(0, 0) && !hud_dead_visible(0, 10) && hud_dead_visible(EF2_REMOTE_CONTROLLING, 12));
        assert!(hud_in_killcam(0, PM_TYPE_KILLCAM) && hud_in_killcam(7, 0) && !hud_in_killcam(0, 0));
    }

    #[test]
    fn body_by_type() {
        assert_eq!(hud_elem_body(HE_TYPE_SCORE, 0), HudElemBody::String);
        assert_eq!(hud_elem_body(HE_TYPE_TENTHS_TIMER_UP, 0), HudElemBody::String);
        assert_eq!(hud_elem_body(HE_TYPE_MATERIAL, 0), HudElemBody::Material);
        assert_eq!(hud_elem_body(HE_TYPE_CLOCK_UP, 0), HudElemBody::Clock);
        assert_eq!(hud_elem_body(HE_TYPE_DAMAGE_INDICATOR, 0), HudElemBody::Nothing);
        assert_eq!(hud_elem_body(HE_TYPE_WAYPOINT, 0), HudElemBody::OffscreenWaypoint);
        assert_eq!(hud_elem_body(HE_TYPE_WAYPOINT, piece_bits::NO_OFFSCREEN_WAYPOINT), HudElemBody::Nothing);
    }

    #[test]
    fn shoutcaster_perks_and_alpha() {
        let perks = OverlayPiece { piece_kind: HE_TYPE_PERKS, tint_rgba: pack_color(9, 9, 9, 10), ..OverlayPiece::default() };
        assert_eq!(hud_elem_draw_color(&perks, 0, Some(0)), None);
        assert_eq!(hud_elem_draw_color(&perks, 0, Some(2)), Some([9, 9, 9, 255]));
        assert_eq!(hud_elem_draw_color(&perks, 0, None), Some([9, 9, 9, 10]));
        let clear = OverlayPiece { piece_kind: ELEM_KIND_TEXT, tint_rgba: pack_color(1, 1, 1, 0), ..OverlayPiece::default() };
        assert_eq!(hud_elem_draw_color(&clear, 0, None), None);
    }

    #[test]
    fn percent_g() {
        assert_eq!(format_g(0.0), "0");
        assert_eq!(format_g(100.0), "100");
        assert_eq!(format_g(1.5), "1.5");
        assert_eq!(format_g(123456.0), "123456");
        assert_eq!(format_g(1234567.0), "1.23457e+06");
        assert_eq!(format_g(0.0001), "0.0001");
        assert_eq!(format_g(0.00001), "1e-05");
        assert_eq!(format_g(-2.25), "-2.25");
    }

    #[test]
    fn placement_moves_then_rounds() {
        let place = place_640();
        let e = OverlayPiece {
            piece_kind: ELEM_KIND_TEXT,
            screen_anchor: align_screen(ANCHOR_CENTER, ANCHOR_SUBPART),
            anchor: align_org(ORG_MIDDLE, ORG_LEADING),
            x: 10.3,
            y: 20.6,
            was_x: 0.0,
            was_y: 0.0,
            screen_anchor_was: align_screen(ANCHOR_SUBPART, ANCHOR_SUBPART),
            slide_from: 0,
            slide_ms: 1000,
            ..OverlayPiece::default()
        };
        // Done: centre 320 + 10.3 - half of 50 = 305.3 -> 305; y 20.6 -> 21.
        assert_eq!(hud_elem_position(&place, &e, 1000, 50.0, 10.0), (305.0, 21.0));
        // Halfway: from (0, 0) (top-left, leading org) to (305.3, 20.6), not rounded.
        let (x, y) = hud_elem_position(&place, &e, 500, 50.0, 10.0);
        assert!((x - 152.65).abs() < 1e-3 && (y - 10.3).abs() < 1e-4);
        assert_eq!(hud_elem_movement_frac(&e, 0), 0.0);
        // A running font-scale animation keeps the fraction.
        let anim = OverlayPiece { text_grow_ms: 100, text_grow_from: 1000, ..e };
        let (x, _) = hud_elem_position(&place, &anim, 1050, 50.0, 10.0);
        assert!((x - 305.3).abs() < 1e-4);
    }

    #[test]
    fn material_size_lerps() {
        let place = place_640();
        let e = OverlayPiece {
            piece_kind: HE_TYPE_MATERIAL,
            width: 64,
            height: 32,
            was_wide: 32,
            was_tall: 16,
            grow_from: 0,
            grow_ms: 100,
            ..OverlayPiece::default()
        };
        assert_eq!(hud_elem_material_size(&place, &e, 50), (48.0, 24.0));
        assert_eq!(hud_elem_material_size(&place, &e, 100), (64.0, 32.0));
        let p = hud_elem_placement(&place, &e, 100, 0.0, 12.0);
        assert_eq!((p.w, p.h), (64.0, 32.0));
    }
}
