//! The kill feed: obituary lines in game message window 0, their kill icons and the packed
//! obituary event parm.
//!
//! Means of death are BO2's meansOfDeath_t numbers (glass_t6 MEANS_NAMES). Head shots are hit
//! location 1 or 2 (weapon-hits-pen-t6.md).
//!
//! TODO(fact): BO2 draws its kill feed in LUI; the window 0 values, kill icon materials and
//! sizes, which means of death pack into the event parm, the weapon type/class numbers in that
//! rule and the left-game string are not in the facts yet (QUESTIONS.md 10-21 Clean1 hud
//! gamemsg); until answered these are MW2's, keyed by means-of-death name.

extern crate alloc;

use alloc::string::String;

use crate::scrplace::{ALIGN_USER_MAX, ALIGN_USER_MIN};

pub const GAME_MSG_WIN0_LINE_COUNT: usize = 4;

pub const GAME_MSG_WIN0_MSG_TIME_MS: i32 = 5000;

pub const GAME_MSG_WIN0_X: f32 = 6.0;

pub const GAME_MSG_WIN0_Y: f32 = -60.0;

pub const GAME_MSG_WIN0_HORZ_ALIGN: i32 = ALIGN_USER_MIN;

pub const GAME_MSG_WIN0_VERT_ALIGN: i32 = ALIGN_USER_MAX;

pub const GAME_MSG_WIN0_TEXT_SCALE: f32 = 0.375;

pub const GAME_MSG_WIN0_TEXT_STYLE: i32 = 3;

pub const KILLICON_BASE_SIZE: f32 = 1.4;

pub const KILLICON_WIDE_SIZE: f32 = 2.8;

pub const KILLICON_SHORT_SIZE: f32 = 0.7;

pub const GAME_MSG_CHAR_EM: f32 = 48.0;

/// The embedded icon size byte: round(em * 32 + 2^-30), clamped to 16..=127, plus 16.
pub const EMBED_HUD_ICON_SIZE_SCALE: f64 = 32.0;

pub const EMBED_HUD_ICON_SIZE_BIAS: f64 = f64::from_bits(0x3e10_0000_0000_0000);

pub const EMBED_HUD_ICON_SIZE_CLAMP_MIN: i32 = 0x10;

pub const EMBED_HUD_ICON_SIZE_CLAMP_MAX: i32 = 0x7f;

pub const KILLICON_DIED: &str = "killicondied";

pub const KILLICON_MELEE: &str = "killiconmelee";

pub const KILLICON_HEADSHOT: &str = "killiconheadshot";

pub const KILLICON_CRUSH: &str = "killiconcrush";

pub const KILLICON_FALLING: &str = "killiconfalling";

pub const KILLICON_SUICIDE: &str = "killiconsuicide";

pub const KILLICON_IMPACT: &str = "killiconimpact";

pub const HITLOC_HELMET: u8 = 1;
pub const HITLOC_HEAD: u8 = 2;

pub const MOD_MELEE: i32 = 7;

pub const MOD_HEAD_SHOT: i32 = 9;

pub const MOD_CRUSH: i32 = 10;

pub const MOD_FALLING: i32 = 12;

pub const MOD_SUICIDE: i32 = 13;

pub const MOD_TRIGGER_HURT: i32 = 14;

pub const MOD_IMPACT: i32 = 16;

/// The weapon type that keeps its weapon in a melee obituary (MW2's grenade type).
pub const OBITUARY_MELEE_KEEP_WEAP_TYPE: i32 = 3;

/// The weapon class that keeps its weapon in an impact obituary (MW2's thrown class).
pub const OBITUARY_IMPACT_KEEP_WEAP_CLASS: i32 = 9;

pub const EXE_LEFTGAME: &str = "EXE_LEFTGAME";

#[must_use]
pub fn game_msg_win0_char_height() -> f32 {
    GAME_MSG_WIN0_TEXT_SCALE * GAME_MSG_CHAR_EM
}

/// Kill icon (width, height) in em for its ratio: 0 square, 1 wide, else wide and short.
#[must_use]
pub fn killicon_em_size(kill_icon_ratio: i32) -> (f32, f32) {
    let mut width = KILLICON_BASE_SIZE;
    let mut height = KILLICON_BASE_SIZE;
    if kill_icon_ratio != 0 {
        width = KILLICON_WIDE_SIZE;
        if kill_icon_ratio != 1 {
            height = KILLICON_SHORT_SIZE;
        }
    }
    (width, height)
}

/// Kill icon size in virtual units, after the round trip through the embedded size byte.
#[must_use]
pub fn killicon_virtual_size(kill_icon_ratio: i32) -> (f32, f32) {
    let (width_em, height_em) = killicon_em_size(kill_icon_ratio);
    (
        decode_hud_icon_size(embed_hud_icon_size_byte(width_em)),
        decode_hud_icon_size(embed_hud_icon_size_byte(height_em)),
    )
}

#[must_use]
pub fn embed_hud_icon_size_byte(em: f32) -> u8 {
    let rounded =
        libm::round(f64::from(em) * EMBED_HUD_ICON_SIZE_SCALE + EMBED_HUD_ICON_SIZE_BIAS) as i32;
    let clamped = rounded.clamp(EMBED_HUD_ICON_SIZE_CLAMP_MIN, EMBED_HUD_ICON_SIZE_CLAMP_MAX);
    u8::try_from(clamped + EMBED_HUD_ICON_SIZE_CLAMP_MIN).unwrap_or(u8::MAX)
}

#[must_use]
pub fn decode_hud_icon_size(stored: u8) -> f32 {
    let pixel_height = GAME_MSG_CHAR_EM as i32;
    let inner = (pixel_height * (i32::from(stored) - EMBED_HUD_ICON_SIZE_CLAMP_MIN) + 16) / 32;
    inner as f32 * GAME_MSG_WIN0_TEXT_SCALE
}

/// The icon's (u0, u1): a flipped icon runs right to left.
#[must_use]
pub fn killicon_stretch_uv(flip_kill_icon: bool) -> (f32, f32) {
    if flip_kill_icon { (1.0, 0.0) } else { (0.0, 1.0) }
}

/// The means of death packed in an obituary event parm, or None when it holds a weapon.
#[must_use]
pub fn obituary_mod(event_parm: i32) -> Option<i32> {
    (event_parm < 0).then(|| -1 - event_parm)
}

#[must_use]
pub fn obituary_mod_killicon(means_of_death: i32) -> Option<&'static str> {
    match means_of_death {
        MOD_MELEE => Some(KILLICON_MELEE),
        MOD_HEAD_SHOT => Some(KILLICON_HEADSHOT),
        MOD_CRUSH => Some(KILLICON_CRUSH),
        MOD_FALLING => Some(KILLICON_FALLING),
        MOD_SUICIDE => Some(KILLICON_SUICIDE),
        MOD_TRIGGER_HURT => Some(KILLICON_DIED),
        MOD_IMPACT => Some(KILLICON_IMPACT),
        _ => None,
    }
}

#[must_use]
pub fn obituary_is_headshot(hitloc: u8) -> bool {
    hitloc == HITLOC_HELMET || hitloc == HITLOC_HEAD
}

/// A game notify line: the name in place of "&&1", or the name before the template.
#[must_use]
pub fn gamenotify_line(template: &str, name: &str) -> String {
    if template.contains("&&") {
        return crate::centerprint::centerprint_replace_name(template, name);
    }
    if name.is_empty() {
        return String::from(template);
    }
    let mut out = String::from(name);
    out.push_str("^7 ");
    out.push_str(template);
    out
}

/// The obituary event parm: -1 - means of death for the deaths shown by their own icon,
/// else the weapon index.
#[must_use]
pub fn pack_obituary_event_parm(
    weapon: u32,
    means_of_death: i32,
    weap_type: i32,
    weap_class: i32,
) -> i32 {
    let pack_mod = match means_of_death {
        MOD_IMPACT => weap_class != OBITUARY_IMPACT_KEEP_WEAP_CLASS,
        MOD_MELEE => weap_type != OBITUARY_MELEE_KEEP_WEAP_TYPE,
        MOD_HEAD_SHOT | MOD_CRUSH | MOD_FALLING | MOD_SUICIDE => true,
        _ => false,
    };
    if pack_mod { -1 - means_of_death } else { weapon as i32 }
}

/// The y of a window 0 line counted from the newest (0).
#[must_use]
pub fn game_msg_win0_line_y(line_from_newest: usize) -> f32 {
    GAME_MSG_WIN0_Y - game_msg_win0_char_height() * (line_from_newest as f32 + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_head_shot_packs_its_mod_and_shows_its_icon() {
        let parm = pack_obituary_event_parm(5, MOD_HEAD_SHOT, 0, 0);
        assert_eq!(obituary_mod(parm), Some(MOD_HEAD_SHOT));
        assert_eq!(obituary_mod_killicon(MOD_HEAD_SHOT), Some(KILLICON_HEADSHOT));
        assert_eq!(pack_obituary_event_parm(5, 2, 0, 0), 5);
        assert_eq!(obituary_mod(5), None);
    }

    #[test]
    fn melee_and_impact_keep_the_weapon_for_their_kept_kinds() {
        assert_eq!(pack_obituary_event_parm(3, MOD_MELEE, OBITUARY_MELEE_KEEP_WEAP_TYPE, 0), 3);
        assert_eq!(pack_obituary_event_parm(3, MOD_MELEE, 0, 0), -1 - MOD_MELEE);
        assert_eq!(pack_obituary_event_parm(3, MOD_IMPACT, 0, OBITUARY_IMPACT_KEEP_WEAP_CLASS), 3);
        assert_eq!(pack_obituary_event_parm(3, MOD_IMPACT, 0, 0), -1 - MOD_IMPACT);
    }

    #[test]
    fn helmet_and_head_are_head_shots() {
        assert!(obituary_is_headshot(1) && obituary_is_headshot(2));
        assert!(!obituary_is_headshot(0) && !obituary_is_headshot(3));
    }

    #[test]
    fn kill_icons_round_trip_their_size_byte() {
        // 1.4 em -> byte 45 + 16 -> (48 * 45 + 16) / 32 = 68 -> 68 * 0.375.
        assert_eq!(killicon_virtual_size(0), (25.5, 25.5));
        assert_eq!(killicon_virtual_size(2).1, 12.375);
    }

    #[test]
    fn lines_stack_up_from_the_window() {
        assert_eq!(game_msg_win0_line_y(0), -78.0);
        assert_eq!(gamenotify_line("left the game", "Ann"), "Ann^7 left the game");
        assert_eq!(gamenotify_line("&&1 left", "Ann"), "Ann left");
    }
}
