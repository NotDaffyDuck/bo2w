//! BO2 HUD ammo counter values (scratch/hud-facts/ammo.md): which weapon is shown, the clip and
//! stock text, the low-clip / low-stock colour, the 800 ms red low-clip flash and the low-ammo
//! warning gate. Pip graphics are not transcribed yet.

/// Low-ammo red, also the fallback warning colour.
pub const AMMO_LOW_RGB: [f32; 3] = [0.89, 0.18, 0.01];
/// Stock counts as low at or below this share of the max reserve.
pub const AMMO_LOW_STOCK_FRACTION: f32 = 0.2;
/// The low-clip red copy fades out over this many ms.
pub const AMMO_LOW_CLIP_FLASH_MS: i32 = 800;
/// The combined value's "|" separator sits this many units left of centre.
pub const AMMO_SEPARATOR_LEFT_OF_CENTRE: f32 = 5.0;

/// ps.otherFlags bit that hides the counter.
pub const OTHER_FLAGS_HIDE_AMMO: u32 = 0x80;
/// nextSnap ps.otherFlags bit that shows the forced-select weapon.
pub const OTHER_FLAGS_WEAPON_FORCED_SELECT: u32 = 0x4;
/// eFlags bits that silence the low-ammo warning.
pub const LOW_AMMO_WARNING_EFLAGS_SILENCE: u32 = 0x300;

/// ammoCounterClip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmmoCounterClip {
    None,
    AltWeapon,
    Magazine,
    ShortMagazine,
    Shotgun,
    Rocket,
    Beltfed,
}

impl AmmoCounterClip {
    #[must_use]
    pub fn from_index(i: u32) -> Option<Self> {
        Some(match i {
            0 => Self::None,
            1 => Self::AltWeapon,
            2 => Self::Magazine,
            3 => Self::ShortMagazine,
            4 => Self::Shotgun,
            5 => Self::Rocket,
            6 => Self::Beltfed,
            _ => return None,
        })
    }
}

/// Which weapon index the counter shows. The forced-select weapon wins when the next snapshot flags
/// it and the player has it; a NONE counter shows nothing unless it is a fuel tank; ALTWEAPON shows
/// the alt weapon.
#[must_use]
pub fn ammo_counter_weapon(
    ps_weapon: u32,
    forced_select: Option<u32>,
    next_snap_other_flags: u32,
    has_forced: bool,
    counter: AmmoCounterClip,
    fuel_tank: bool,
    alt_weapon: u32,
) -> Option<u32> {
    let shown = match forced_select {
        Some(w) if next_snap_other_flags & OTHER_FLAGS_WEAPON_FORCED_SELECT != 0 && has_forced => w,
        _ => ps_weapon,
    };
    if shown == 0 {
        return None;
    }
    match counter {
        AmmoCounterClip::None if !fuel_tank => None,
        AmmoCounterClip::AltWeapon => Some(alt_weapon),
        _ => Some(shown),
    }
}

/// Clip digits: "%01i" below 10 rounds a clip, "%02i" otherwise, 3 digits for 100+ belts.
#[must_use]
pub fn clip_text_digits(clip_size: i32) -> usize {
    if clip_size < 10 {
        1
    } else if clip_size < 100 {
        2
    } else {
        3
    }
}

/// Low clip: at or below the weapon's lowAmmoWarningThreshold.
#[must_use]
pub fn is_low_clip(clip: i32, low_ammo_warning_threshold: i32) -> bool {
    clip <= low_ammo_warning_threshold
}

/// Low stock: at or below 20% of the max reserve.
#[must_use]
pub fn is_low_stock(reserve: i32, max_reserve: i32) -> bool {
    reserve as f32 <= max_reserve as f32 * AMMO_LOW_STOCK_FRACTION
}

/// AmmoColor: red when low, else the hud colour (alpha kept).
#[must_use]
pub fn ammo_color(hud: [f32; 4], low: bool) -> [f32; 4] {
    if low { [AMMO_LOW_RGB[0], AMMO_LOW_RGB[1], AMMO_LOW_RGB[2], hud[3]] } else { hud }
}

/// The red low-clip copy: restarts the hint time when outside the 800 ms window, then alpha =
/// (hint - time + 800) / 800, capped by the fade alpha. Returns (alpha, new hint time).
#[must_use]
pub fn low_clip_flash(invalid_cmd_hint_time: i32, time: i32, fade_alpha: f32) -> (f32, i32) {
    let since = time - invalid_cmd_hint_time;
    let hint = if !(0..=AMMO_LOW_CLIP_FLASH_MS).contains(&since) { time } else { invalid_cmd_hint_time };
    let a = ((hint - time) + AMMO_LOW_CLIP_FLASH_MS) as f32 * (1.0 / AMMO_LOW_CLIP_FLASH_MS as f32);
    (a.min(fade_alpha), hint)
}

/// What the counter draws for one weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmmoCounterValues {
    /// None when hidden (negative, clip-only weapon or reserve suppressed).
    pub clip: Option<i32>,
    pub stock: Option<i32>,
    /// Clip-only weapons draw the total alone, centred.
    pub total_centred: Option<i32>,
}

#[must_use]
pub fn ammo_counter_values(clip: i32, reserve: i32, clip_only: bool, suppress_reserve: bool) -> AmmoCounterValues {
    if suppress_reserve {
        return AmmoCounterValues { clip: None, stock: None, total_centred: None };
    }
    if clip_only {
        return AmmoCounterValues { clip: None, stock: None, total_centred: Some(clip.max(0) + reserve.max(0)) };
    }
    AmmoCounterValues {
        clip: (clip >= 0).then_some(clip),
        stock: (reserve >= 0).then_some(reserve),
        total_centred: None,
    }
}

/// CG_DrawPlayerWeaponLowAmmoWarning outer gate: true when the warning may draw.
#[must_use]
pub fn low_ammo_warning_outer_gate(weaponstate: u32, pm_type: u32, waterlevel: u32, eflags: u32) -> bool {
    !(0xb..=0x12).contains(&weaponstate)
        && pm_type <= 9
        && pm_type != 5
        && waterlevel <= 3
        && eflags & LOW_AMMO_WARNING_EFLAGS_SILENCE == 0
}

/// Which message the warning shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LowAmmoWarningKind {
    OutOfAmmo,
    LowAmmo,
    Reload,
}

/// Out of ammo when clip and reserve are empty; reload when the clip is low and stock remains; low
/// when either is low. The facts give the three messages and their order, not the exact tests (the
/// dual-wield, no-partial-reload, TV-guided and killstreak cases are the caller's).
#[must_use]
pub fn low_ammo_warning_kind(clip: i32, reserve: i32, low_clip: bool, low_stock: bool) -> Option<LowAmmoWarningKind> {
    if clip <= 0 && reserve <= 0 {
        Some(LowAmmoWarningKind::OutOfAmmo)
    } else if low_clip && reserve > 0 {
        Some(LowAmmoWarningKind::Reload)
    } else if low_clip || low_stock {
        Some(LowAmmoWarningKind::LowAmmo)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_and_values() {
        let w = ammo_counter_weapon(7, Some(9), OTHER_FLAGS_WEAPON_FORCED_SELECT, true, AmmoCounterClip::Magazine, false, 3);
        assert_eq!(w, Some(9));
        assert_eq!(ammo_counter_weapon(7, Some(9), 0, true, AmmoCounterClip::Magazine, false, 3), Some(7));
        assert_eq!(ammo_counter_weapon(7, None, 0, false, AmmoCounterClip::None, false, 3), None);
        assert_eq!(ammo_counter_weapon(7, None, 0, false, AmmoCounterClip::None, true, 3), Some(7));
        assert_eq!(ammo_counter_weapon(7, None, 0, false, AmmoCounterClip::AltWeapon, false, 3), Some(3));
        assert_eq!(ammo_counter_weapon(0, None, 0, false, AmmoCounterClip::Magazine, false, 3), None);
        let v = ammo_counter_values(-1, 30, false, false);
        assert_eq!((v.clip, v.stock), (None, Some(30)));
        assert_eq!(ammo_counter_values(2, 3, true, false).total_centred, Some(5));
        assert_eq!(clip_text_digits(8), 1);
        assert_eq!(clip_text_digits(30), 2);
        assert_eq!(clip_text_digits(150), 3);
    }

    #[test]
    fn low_rules_and_flash() {
        assert!(is_low_clip(5, 5));
        assert!(is_low_stock(24, 120));
        assert!(!is_low_stock(25, 120));
        assert_eq!(ammo_color([1.0, 1.0, 1.0, 0.5], true), [0.89, 0.18, 0.01, 0.5]);
        assert_eq!(low_clip_flash(1000, 1400, 1.0), (0.5, 1000));
        assert_eq!(low_clip_flash(0, 5000, 1.0), (1.0, 5000));
        assert_eq!(low_clip_flash(1000, 1000, 0.25).0, 0.25);
        assert!(low_ammo_warning_outer_gate(0, 1, 0, 0));
        assert!(!low_ammo_warning_outer_gate(0xb, 1, 0, 0));
        assert!(!low_ammo_warning_outer_gate(0, 5, 0, 0));
        assert!(!low_ammo_warning_outer_gate(0, 1, 0, 0x100));
        assert_eq!(low_ammo_warning_kind(0, 0, true, true), Some(LowAmmoWarningKind::OutOfAmmo));
        assert_eq!(low_ammo_warning_kind(2, 50, true, false), Some(LowAmmoWarningKind::Reload));
        assert_eq!(low_ammo_warning_kind(20, 10, false, true), Some(LowAmmoWarningKind::LowAmmo));
    }
}
