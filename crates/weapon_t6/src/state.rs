//! BO2's 65 weapon states (`weaponstate` / `weaponstateLeft`), by BO2's numbers.
//!
//! Stored as `i32` like BO2 so the tick can switch on the same values.

pub const READY: i32 = 0;
pub const RAISING: i32 = 1;
pub const RAISING_ALTSWITCH: i32 = 2;
pub const DROPPING: i32 = 3;
pub const DROPPING_QUICK: i32 = 4;
pub const DROPPING_ALTSWITCH: i32 = 5;
pub const FIRING: i32 = 6;
pub const RECHAMBER_START: i32 = 7;
pub const RECHAMBERING: i32 = 8;
pub const RELOADING_RIGHT: i32 = 9;
pub const RELOADING_LEFT: i32 = 10;
pub const RELOADING: i32 = 11;
pub const RELOADING_INTERUPT: i32 = 12;
pub const RELOAD_START: i32 = 13;
pub const RELOAD_START_RECHAMBER: i32 = 14;
pub const RELOAD_START_INTERUPT: i32 = 15;
pub const RELOAD_END: i32 = 16;
pub const RELOAD_QUICK: i32 = 17;
pub const RELOAD_QUICK_EMPTY: i32 = 18;
pub const MELEE_INIT: i32 = 19;
pub const MELEE_FIRE: i32 = 20;
pub const MELEE_END: i32 = 21;
pub const OFFHAND_INIT: i32 = 22;
pub const OFFHAND_PREPARE: i32 = 23;
pub const OFFHAND_HOLD: i32 = 24;
pub const OFFHAND_DETONATE: i32 = 25;
pub const OFFHAND_START: i32 = 26;
pub const OFFHAND: i32 = 27;
pub const OFFHAND_HOLDUSE: i32 = 28;
pub const OFFHAND_END: i32 = 29;
pub const RIOTSHIELD_PREPARE: i32 = 30;
pub const RIOTSHIELD_START: i32 = 31;
pub const RIOTSHIELD_HOLD: i32 = 32;
pub const RIOTSHIELD: i32 = 33;
pub const RIOTSHIELD_END: i32 = 34;
pub const DETONATING: i32 = 35;
pub const SPRINT_RAISE: i32 = 36;
pub const SPRINT_LOOP: i32 = 37;
pub const SPRINT_DROP: i32 = 38;
pub const CONT_FIRE_IN: i32 = 39;
pub const CONT_FIRE_LOOP: i32 = 40;
pub const CONT_FIRE_OUT: i32 = 41;
pub const NIGHTVISION_WEAR: i32 = 42;
pub const NIGHTVISION_REMOVE: i32 = 43;
pub const DEPLOYING: i32 = 44;
pub const DEPLOYED: i32 = 45;
pub const BREAKING_DOWN: i32 = 46;
pub const SWIM_IN: i32 = 47;
pub const SWIM_OUT: i32 = 48;
pub const DTP_IN: i32 = 49;
pub const DTP_LOOP: i32 = 50;
pub const DTP_OUT: i32 = 51;
pub const SLIDE_IN: i32 = 52;
pub const FIRING_LEFT: i32 = 53;
pub const FIRING_BOTH: i32 = 54;
pub const JAMMED: i32 = 55;
pub const LOWREADY_RAISE: i32 = 56;
pub const LOWREADY_LOOP: i32 = 57;
pub const LOWREADY_DROP: i32 = 58;
pub const CRAWL_IN: i32 = 59;
pub const CRAWL_FORWARD: i32 = 60;
pub const CRAWL_BACK: i32 = 61;
pub const CRAWL_RIGHT: i32 = 62;
pub const CRAWL_LEFT: i32 = 63;
pub const CRAWL_OUT: i32 = 64;

/// Number of states (0..=64).
pub const COUNT: usize = 65;

const NAMES: [&str; COUNT] = [
    "READY", "RAISING", "RAISING_ALTSWITCH", "DROPPING", "DROPPING_QUICK", "DROPPING_ALTSWITCH",
    "FIRING", "RECHAMBER_START", "RECHAMBERING", "RELOADING_RIGHT", "RELOADING_LEFT", "RELOADING",
    "RELOADING_INTERUPT", "RELOAD_START", "RELOAD_START_RECHAMBER", "RELOAD_START_INTERUPT",
    "RELOAD_END", "RELOAD_QUICK", "RELOAD_QUICK_EMPTY", "MELEE_INIT", "MELEE_FIRE", "MELEE_END",
    "OFFHAND_INIT", "OFFHAND_PREPARE", "OFFHAND_HOLD", "OFFHAND_DETONATE", "OFFHAND_START",
    "OFFHAND", "OFFHAND_HOLDUSE", "OFFHAND_END", "RIOTSHIELD_PREPARE", "RIOTSHIELD_START",
    "RIOTSHIELD_HOLD", "RIOTSHIELD", "RIOTSHIELD_END", "DETONATING", "SPRINT_RAISE",
    "SPRINT_LOOP", "SPRINT_DROP", "CONT_FIRE_IN", "CONT_FIRE_LOOP", "CONT_FIRE_OUT",
    "NIGHTVISION_WEAR", "NIGHTVISION_REMOVE", "DEPLOYING", "DEPLOYED", "BREAKING_DOWN",
    "SWIM_IN", "SWIM_OUT", "DTP_IN", "DTP_LOOP", "DTP_OUT", "SLIDE_IN", "FIRING_LEFT",
    "FIRING_BOTH", "JAMMED", "LOWREADY_RAISE", "LOWREADY_LOOP", "LOWREADY_DROP", "CRAWL_IN",
    "CRAWL_FORWARD", "CRAWL_BACK", "CRAWL_RIGHT", "CRAWL_LEFT", "CRAWL_OUT",
];

/// BO2's name for a state, for traces.
pub fn name(state: i32) -> &'static str {
    usize::try_from(state)
        .ok()
        .and_then(|i| NAMES.get(i).copied())
        .unwrap_or("?")
}

/// The three dropping states (a switch is under way).
pub fn is_dropping(state: i32) -> bool {
    matches!(state, DROPPING | DROPPING_QUICK | DROPPING_ALTSWITCH)
}

/// The two raising states.
pub fn is_raising(state: i32) -> bool {
    matches!(state, RAISING | RAISING_ALTSWITCH)
}

/// Offhand states 22..=29 (the offhand weapon is the one in hand).
pub fn is_offhand(state: i32) -> bool {
    (OFFHAND_INIT..=OFFHAND_END).contains(&state)
}

/// Reload states that BO2 treats as "reloading" when a switch or event interrupts them.
pub fn is_reload_family(state: i32) -> bool {
    matches!(
        state,
        RELOADING
            | RELOADING_INTERUPT
            | RELOAD_START
            | RELOAD_START_RECHAMBER
            | RELOAD_START_INTERUPT
            | RELOAD_END
            | RELOAD_QUICK
            | RELOAD_QUICK_EMPTY
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_line_up() {
        assert_eq!(super::name(super::CRAWL_OUT), "CRAWL_OUT");
        assert_eq!(super::name(super::JAMMED), "JAMMED");
        assert_eq!(super::name(super::DEPLOYED), "DEPLOYED");
        assert_eq!(super::name(65), "?");
    }
}
