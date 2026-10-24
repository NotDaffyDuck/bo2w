//! BO2's viewmodel animation numbers (`weapAnim` / `weapAnimLeft`), by BO2's numbers.
//!
//! The low bits hold the number; bit 0x400 flips every time an anim is (re)started so the
//! client restarts it even when the number is the same. BO2 only touches these while
//! `pm_type < PM_DEAD`.

pub const IDLE: i32 = 0;
pub const FORCE_IDLE: i32 = 1;
pub const ATTACK: i32 = 2;
pub const ATTACK_LEFT: i32 = 3;
pub const ATTACK_LASTSHOT: i32 = 4;
pub const ATTACK_LASTSHOT_LEFT: i32 = 5;
pub const RECHAMBER: i32 = 6;
pub const ADS_ATTACK: i32 = 7;
pub const ADS_ATTACK_LASTSHOT: i32 = 8;
pub const ADS_RECHAMBER: i32 = 9;
pub const MELEE_ATTACK: i32 = 10;
pub const MELEE_ATTACK_EMPTY: i32 = 11;
pub const MELEE_CHARGE: i32 = 12;
pub const MELEE_CHARGE_EMPTY: i32 = 13;
pub const DROP: i32 = 14;
pub const RAISE: i32 = 15;
pub const FIRST_RAISE: i32 = 16;
pub const RELOAD: i32 = 17;
pub const RELOAD_EMPTY: i32 = 18;
pub const RELOAD_START: i32 = 19;
pub const RELOAD_END: i32 = 20;
pub const RELOAD_QUICK: i32 = 21;
pub const RELOAD_QUICK_EMPTY: i32 = 22;
pub const ALTSWITCHFROM: i32 = 23;
pub const ALTSWITCHTO: i32 = 24;
pub const QUICK_DROP: i32 = 25;
pub const QUICK_RAISE: i32 = 26;
pub const EMPTY_DROP: i32 = 27;
pub const EMPTY_RAISE: i32 = 28;
pub const SPRINT_IN: i32 = 29;
pub const SPRINT_LOOP: i32 = 30;
pub const SPRINT_OUT: i32 = 31;
pub const SPRINT_EMPTY_IN: i32 = 32;
pub const SPRINT_EMPTY_LOOP: i32 = 33;
pub const SPRINT_EMPTY_OUT: i32 = 34;
pub const LOWREADY_IN: i32 = 35;
pub const LOWREADY_LOOP: i32 = 36;
pub const LOWREADY_OUT: i32 = 37;
pub const CONT_FIRE_IN: i32 = 38;
pub const CONT_FIRE_LOOP: i32 = 39;
pub const CONT_FIRE_OUT: i32 = 40;
pub const HOLD_FIRE: i32 = 41;
pub const DETONATE: i32 = 42;
pub const NIGHTVISION_WEAR: i32 = 43;
pub const NIGHTVISION_REMOVE: i32 = 44;
pub const DEPLOYING: i32 = 45;
pub const BREAKING_DOWN: i32 = 46;
pub const DTP_IN: i32 = 47;
pub const DTP_LOOP: i32 = 48;
pub const DTP_OUT: i32 = 49;
pub const DTP_EMPTY_IN: i32 = 50;
pub const DTP_EMPTY_LOOP: i32 = 51;
pub const DTP_EMPTY_OUT: i32 = 52;
pub const SLIDE_IN: i32 = 53;
pub const MANTLE: i32 = 54;
pub const JAM: i32 = 55;
pub const CRAWL_IN: i32 = 56;
pub const CRAWL_FORWARD: i32 = 57;
pub const CRAWL_BACK: i32 = 58;
pub const CRAWL_RIGHT: i32 = 59;
pub const CRAWL_LEFT: i32 = 60;
pub const CRAWL_OUT: i32 = 61;
pub const CRAWL_EMPTY_IN: i32 = 62;
pub const CRAWL_EMPTY_FORWARD: i32 = 63;
pub const CRAWL_EMPTY_BACK: i32 = 64;
pub const CRAWL_EMPTY_RIGHT: i32 = 65;
pub const CRAWL_EMPTY_LEFT: i32 = 66;
pub const CRAWL_EMPTY_OUT: i32 = 67;

/// Number of anim numbers (MAX_WP_ANIMATIONS).
pub const COUNT: usize = 68;

/// Restart bit: flipped on every new anim.
pub const TOGGLE: i32 = 0x400;

/// `weapAnimLeft` value meaning "the left hand is not running its own anim".
pub const LEFT_NONE: i32 = 0x58;

const NAMES: [&str; COUNT] = [
    "IDLE", "FORCE_IDLE", "ATTACK", "ATTACK_LEFT", "ATTACK_LASTSHOT", "ATTACK_LASTSHOT_LEFT",
    "RECHAMBER", "ADS_ATTACK", "ADS_ATTACK_LASTSHOT", "ADS_RECHAMBER", "MELEE_ATTACK",
    "MELEE_ATTACK_EMPTY", "MELEE_CHARGE", "MELEE_CHARGE_EMPTY", "DROP", "RAISE", "FIRST_RAISE",
    "RELOAD", "RELOAD_EMPTY", "RELOAD_START", "RELOAD_END", "RELOAD_QUICK", "RELOAD_QUICK_EMPTY",
    "ALTSWITCHFROM", "ALTSWITCHTO", "QUICK_DROP", "QUICK_RAISE", "EMPTY_DROP", "EMPTY_RAISE",
    "SPRINT_IN", "SPRINT_LOOP", "SPRINT_OUT", "SPRINT_EMPTY_IN", "SPRINT_EMPTY_LOOP",
    "SPRINT_EMPTY_OUT", "LOWREADY_IN", "LOWREADY_LOOP", "LOWREADY_OUT", "CONT_FIRE_IN",
    "CONT_FIRE_LOOP", "CONT_FIRE_OUT", "HOLD_FIRE", "DETONATE", "NIGHTVISION_WEAR",
    "NIGHTVISION_REMOVE", "DEPLOYING", "BREAKING_DOWN", "DTP_IN", "DTP_LOOP", "DTP_OUT",
    "DTP_EMPTY_IN", "DTP_EMPTY_LOOP", "DTP_EMPTY_OUT", "SLIDE_IN", "MANTLE", "JAM", "CRAWL_IN",
    "CRAWL_FORWARD", "CRAWL_BACK", "CRAWL_RIGHT", "CRAWL_LEFT", "CRAWL_OUT", "CRAWL_EMPTY_IN",
    "CRAWL_EMPTY_FORWARD", "CRAWL_EMPTY_BACK", "CRAWL_EMPTY_RIGHT", "CRAWL_EMPTY_LEFT",
    "CRAWL_EMPTY_OUT",
];

/// BO2's name for an anim number (toggle bit ignored), for traces.
pub fn name(anim: i32) -> &'static str {
    if anim == LEFT_NONE {
        return "NONE";
    }
    usize::try_from(anim & !TOGGLE)
        .ok()
        .and_then(|i| NAMES.get(i).copied())
        .unwrap_or("?")
}

/// The anim number without the restart bit.
pub fn number(anim: i32) -> i32 {
    anim & !TOGGLE
}

/// Start `anim`: flip the restart bit of `current` and set the number (BO2's
/// `~cur & 0x400 | anim`).
pub fn start(current: i32, anim: i32) -> i32 {
    (!current & TOGGLE) | anim
}

#[cfg(test)]
mod tests {
    #[test]
    fn toggle_flips() {
        let a = super::start(0, super::RAISE);
        assert_eq!(a, 0x400 | 15);
        let b = super::start(a, super::IDLE);
        assert_eq!(b, 0);
        assert_eq!(super::name(a), "RAISE");
        assert_eq!(super::name(super::CRAWL_EMPTY_OUT), "CRAWL_EMPTY_OUT");
        assert_eq!(super::name(super::LEFT_NONE), "NONE");
    }
}
