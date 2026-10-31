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

use super::scrplace::{ANCHOR_USER_HIGH, ANCHOR_USER_LOW};

pub const FEED_LINE_COUNT: usize = 4;

pub const FEED_LINE_LIFETIME_MS: i32 = 5000;

pub const FEED_ORIGIN_X: f32 = 6.0;

pub const FEED_ORIGIN_Y: f32 = -60.0;

pub const FEED_ALIGN_H: i32 = ANCHOR_USER_LOW;

pub const FEED_ALIGN_V: i32 = ANCHOR_USER_HIGH;

pub const FEED_TEXT_SCALE: f32 = 0.375;

pub const FEED_TEXT_STYLE: i32 = 3;

pub const KILL_ICON_EM_NORMAL: f32 = 1.4;

pub const KILL_ICON_EM_WIDE: f32 = 2.8;

pub const KILL_ICON_EM_SHORT: f32 = 0.7;

pub const FEED_FONT_EM: f32 = 48.0;

/// The embedded icon size byte: round(em * 32 + 2^-30), clamped to 16..=127, plus 16.
pub const ICON_CODE_SCALE: f64 = 32.0;

pub const ICON_CODE_ROUND_BIAS: f64 = f64::from_bits(0x3e10_0000_0000_0000);

pub const ICON_CODE_MIN: i32 = 0x10;

pub const ICON_CODE_MAX: i32 = 0x7f;

pub const ICON_NAME_DIED: &str = "killicondied";

pub const ICON_NAME_MELEE: &str = "killiconmelee";

pub const ICON_NAME_HEADSHOT: &str = "killiconheadshot";

pub const ICON_NAME_CRUSH: &str = "killiconcrush";

pub const ICON_NAME_FALLING: &str = "killiconfalling";

pub const ICON_NAME_SUICIDE: &str = "killiconsuicide";

pub const ICON_NAME_IMPACT: &str = "killiconimpact";

pub const STRUCK_HELM: u8 = 1;
pub const STRUCK_SKULL: u8 = 2;

pub const MOD_MELEE: i32 = 7;

pub const MOD_HEADSHOT: i32 = 9;

pub const MOD_CRUSH: i32 = 10;

pub const MOD_FALLING: i32 = 12;

pub const MOD_SUICIDE: i32 = 13;

pub const MOD_TRIGGER_HURT: i32 = 14;

pub const MOD_IMPACT: i32 = 16;

/// The localized string a player-left notice prints.
pub const LEFT_GAME_NOTICE: &str = "EXE_LEFTGAME";

#[must_use]
pub fn feed_line_height() -> f32 {
    FEED_TEXT_SCALE * FEED_FONT_EM
}

/// Kill icon (width, height) in em for its ratio: 0 square, 1 wide, else wide and short.
#[must_use]
pub fn kill_icon_em_dims(kill_icon_ratio: i32) -> (f32, f32) {
    match kill_icon_ratio {
        0 => (KILL_ICON_EM_NORMAL, KILL_ICON_EM_NORMAL),
        1 => (KILL_ICON_EM_WIDE, KILL_ICON_EM_NORMAL),
        _ => (KILL_ICON_EM_WIDE, KILL_ICON_EM_SHORT),
    }
}

/// Kill icon size in virtual units, after the round trip through the embedded size byte.
#[must_use]
pub fn kill_icon_virtual_dims(kill_icon_ratio: i32) -> (f32, f32) {
    let (across, tall) = kill_icon_em_dims(kill_icon_ratio);
    let stored = |em| icon_size_from_code(icon_size_code(em));
    (stored(across), stored(tall))
}

#[must_use]
pub fn icon_size_code(em: f32) -> u8 {
    let scaled = f64::from(em) * ICON_CODE_SCALE + ICON_CODE_ROUND_BIAS;
    let code = (libm::round(scaled) as i32).clamp(ICON_CODE_MIN, ICON_CODE_MAX);
    u8::try_from(code + ICON_CODE_MIN).unwrap_or(u8::MAX)
}

#[must_use]
pub fn icon_size_from_code(stored: u8) -> f32 {
    let raster_rows = FEED_FONT_EM as i32;
    let inner = (raster_rows * (i32::from(stored) - ICON_CODE_MIN) + 16) / 32;
    inner as f32 * FEED_TEXT_SCALE
}

/// The icon's (u0, u1): a flipped icon runs right to left.
#[must_use]
pub fn kill_icon_uv_flip(mirrored: bool) -> (f32, f32) {
    let u = f32::from(u8::from(mirrored));
    (u, 1.0 - u)
}

/// The means of death packed in an obituary event parm, or None when it holds a weapon.
#[must_use]
pub fn obituary_cause(event_parm: i32) -> Option<i32> {
    (event_parm < 0).then(|| -1 - event_parm)
}

#[must_use]
pub fn obituary_icon_name(kill_cause: i32) -> Option<&'static str> {
    match kill_cause {
        MOD_MELEE => Some(ICON_NAME_MELEE),
        MOD_HEADSHOT => Some(ICON_NAME_HEADSHOT),
        MOD_CRUSH => Some(ICON_NAME_CRUSH),
        MOD_FALLING => Some(ICON_NAME_FALLING),
        MOD_SUICIDE => Some(ICON_NAME_SUICIDE),
        MOD_TRIGGER_HURT => Some(ICON_NAME_DIED),
        MOD_IMPACT => Some(ICON_NAME_IMPACT),
        _ => None,
    }
}

#[must_use]
pub fn kill_notice_was_skull(hitloc: u8) -> bool {
    matches!(hitloc, STRUCK_HELM | STRUCK_SKULL)
}

/// A game notify line: the name in place of "&&1", or the name before the template.
#[must_use]
pub fn notify_line_text(template: &str, who: &str) -> String {
    let has_slot = template.contains("&&");
    match (has_slot, who.is_empty()) {
        (true, _) => crate::centerprint::banner_swap_player_tag(template, who),
        (false, true) => template.into(),
        (false, false) => [who, "^7 ", template].concat(),
    }
}

/// The obituary event parm: -1 - means of death for the deaths shown by their own icon,
/// else the weapon index. BO2's `GScr_Obituary` keeps the weapon for a melee death only when
/// the weapon is a riot shield, and for an impact death only when its impact type is a blade
/// (a ballistic or throwing knife); every other melee or impact death shows the plain icon.
#[must_use]
pub fn encode_kill_notice_arg(
    gun_row: u32,
    kill_cause: i32,
    riot_shield: bool,
    blade_impact: bool,
) -> i32 {
    let shows_own_icon = match kill_cause {
        MOD_IMPACT => !blade_impact,
        MOD_MELEE => !riot_shield,
        MOD_HEADSHOT | MOD_CRUSH | MOD_FALLING | MOD_SUICIDE => true,
        _ => false,
    };
    match shows_own_icon {
        true => -1 - kill_cause,
        false => gun_row as i32,
    }
}

/// The y of a window 0 line counted from the newest (0).
#[must_use]
pub fn feed_line_y(line_from_newest: usize) -> f32 {
    FEED_ORIGIN_Y - feed_line_height() * (line_from_newest as f32 + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_head_shot_packs_its_mod_and_shows_its_icon() {
        let parm = encode_kill_notice_arg(5, MOD_HEADSHOT, false, false);
        assert_eq!(obituary_cause(parm), Some(MOD_HEADSHOT));
        assert_eq!(obituary_icon_name(MOD_HEADSHOT), Some(ICON_NAME_HEADSHOT));
        assert_eq!(encode_kill_notice_arg(5, 2, false, false), 5);
        assert_eq!(obituary_cause(5), None);
    }

    #[test]
    fn melee_and_impact_keep_the_weapon_for_their_kept_kinds() {
        assert_eq!(encode_kill_notice_arg(3, MOD_MELEE, true, false), 3);
        assert_eq!(encode_kill_notice_arg(3, MOD_MELEE, false, true), -1 - MOD_MELEE);
        assert_eq!(encode_kill_notice_arg(3, MOD_IMPACT, false, true), 3);
        assert_eq!(encode_kill_notice_arg(3, MOD_IMPACT, true, false), -1 - MOD_IMPACT);
    }

    #[test]
    fn crush_falling_and_suicide_never_keep_the_weapon_and_a_trigger_hurt_always_does() {
        for means in [MOD_CRUSH, MOD_FALLING, MOD_SUICIDE] {
            assert_eq!(encode_kill_notice_arg(3, means, true, true), -1 - means);
        }
        assert_eq!(encode_kill_notice_arg(3, MOD_TRIGGER_HURT, false, false), 3);
    }

    #[test]
    fn helmet_and_head_are_head_shots() {
        assert!(kill_notice_was_skull(1) && kill_notice_was_skull(2));
        assert!(!kill_notice_was_skull(0) && !kill_notice_was_skull(3));
    }

    #[test]
    fn kill_icons_round_trip_their_size_byte() {
        // 1.4 em -> byte 45 + 16 -> (48 * 45 + 16) / 32 = 68 -> 68 * 0.375.
        assert_eq!(kill_icon_virtual_dims(0), (25.5, 25.5));
        assert_eq!(kill_icon_virtual_dims(2).1, 12.375);
    }

    #[test]
    fn lines_stack_up_from_the_window() {
        assert_eq!(feed_line_y(0), -78.0);
        assert_eq!(notify_line_text("left the game", "Ann"), "Ann^7 left the game");
        assert_eq!(notify_line_text("&&1 left", "Ann"), "Ann left");
    }
}

