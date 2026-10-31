//! BO2's player enums, with BO2's own numbers.

/// BO2's `pmtype_t`: how the player moves this frame.
pub mod movement_mode {
    pub const NORMAL: i32 = 0;
    pub const NORMAL_LINKED: i32 = 1;
    pub const NOCLIP: i32 = 2;
    pub const UFO: i32 = 3;
    pub const MPVIEWER: i32 = 4;
    pub const SPECTATOR: i32 = 5;
    pub const INTERMISSION: i32 = 6;
    pub const LASTSTAND: i32 = 7;
    /// Down and waiting to be revived.
    pub const REVIVEE: i32 = 8;
    pub const LASTSTAND_TRANSITION: i32 = 9;
    pub const DEAD: i32 = 10;
    pub const DEAD_LINKED: i32 = 11;

    /// Dead, linked or not.
    #[must_use]
    pub const fn is_dead(movement_mode: i32) -> bool {
        movement_mode >= DEAD
    }
}

/// BO2's `weaponstate_t` (`WEAPON_*`). The misspelled INTERUPT names are
/// BO2's own.
pub mod weapon_state {
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
    pub const RELOAD_START_INTERUPT: i32 = 14;
    pub const RELOAD_END: i32 = 15;
    pub const RELOAD_QUICK: i32 = 16;
    pub const RELOAD_QUICK_EMPTY: i32 = 17;
    pub const MELEE_INIT: i32 = 18;
    pub const MELEE_FIRE: i32 = 19;
    pub const MELEE_END: i32 = 20;
    pub const OFFHAND_INIT: i32 = 21;
    pub const OFFHAND_PREPARE: i32 = 22;
    pub const OFFHAND_HOLD: i32 = 23;
    pub const OFFHAND_DETONATE: i32 = 24;
    pub const OFFHAND_START: i32 = 25;
    pub const OFFHAND: i32 = 26;
    pub const OFFHAND_HOLDUSE: i32 = 27;
    pub const OFFHAND_END: i32 = 28;
    pub const RIOTSHIELD_PREPARE: i32 = 29;
    pub const RIOTSHIELD_START: i32 = 30;
    pub const RIOTSHIELD_HOLD: i32 = 31;
    pub const RIOTSHIELD: i32 = 32;
    pub const RIOTSHIELD_END: i32 = 33;
    pub const DETONATING: i32 = 34;
    pub const SPRINT_RAISE: i32 = 35;
    pub const SPRINT_LOOP: i32 = 36;
    pub const SPRINT_DROP: i32 = 37;
    pub const CONT_FIRE_IN: i32 = 38;
    pub const CONT_FIRE_LOOP: i32 = 39;
    pub const CONT_FIRE_OUT: i32 = 40;
    pub const NIGHTVISION_WEAR: i32 = 41;
    pub const NIGHTVISION_REMOVE: i32 = 42;
    pub const DEPLOYING: i32 = 43;
    pub const DEPLOYED: i32 = 44;
    pub const BREAKING_DOWN: i32 = 45;
    pub const SWIM_IN: i32 = 46;
    pub const SWIM_OUT: i32 = 47;
    /// Dive to prone ("DTP").
    pub const DTP_IN: i32 = 48;
    pub const DTP_LOOP: i32 = 49;
    pub const DTP_OUT: i32 = 50;
    pub const SLIDE_IN: i32 = 51;
    pub const FIRING_LEFT: i32 = 52;
    pub const FIRING_BOTH: i32 = 53;
    pub const JAMMED: i32 = 54;
    pub const LOWREADY_RAISE: i32 = 55;
    pub const LOWREADY_LOOP: i32 = 56;
    pub const LOWREADY_DROP: i32 = 57;
    pub const CRAWL_IN: i32 = 58;
    pub const CRAWL_FORWARD: i32 = 59;
    pub const CRAWL_BACK: i32 = 60;
    pub const CRAWL_RIGHT: i32 = 61;
    pub const CRAWL_LEFT: i32 = 62;
    pub const CRAWL_OUT: i32 = 63;
    pub const COUNT: i32 = 64;

    /// The states `PM_IsAdsAllowed` refuses aim in: offhand prepare
    /// through detonate (22..=34) and dive-to-prone loop through slide in
    /// (49..=51).
    #[must_use]
    pub const fn blocks_ads(state: i32) -> bool {
        matches!(state, OFFHAND_PREPARE..=DETONATING | DTP_LOOP..=SLIDE_IN)
    }
}

/// BO2's `OffhandClass`.
pub mod secondary_hand_category {
    pub const NONE: i32 = 0;
    pub const FRAG_GRENADE: i32 = 1;
    pub const SMOKE_GRENADE: i32 = 2;
    pub const FLASH_GRENADE: i32 = 3;
    pub const GEAR: i32 = 4;
    pub const SUPPLYDROP_MARKER: i32 = 5;
    pub const COUNT: i32 = 6;
}

/// BO2's `OffhandSlot`: which of the loadout's offhand slots a weapon fills.
pub mod offhand_slot {
    pub const NONE: i32 = 0;
    pub const LETHAL_GRENADE: i32 = 1;
    pub const TACTICAL_GRENADE: i32 = 2;
    pub const EQUIPMENT: i32 = 3;
    pub const SPECIFIC_USE: i32 = 4;
    pub const COUNT: i32 = 5;
}

/// BO2's `ViewLockTypes`.
pub mod view_lock {
    pub const NONE: i32 = 0;
    pub const FULL: i32 = 1;
    pub const WEAPONJITTER: i32 = 2;
}

/// BO2's `locSel_t`: what the map-select screen is choosing a spot for.
pub mod loc_sel {
    pub const NONE: i32 = 0;
    pub const ARTILLERY: i32 = 1;
    pub const AIRSTRIKE: i32 = 2;
    pub const MORTAR: i32 = 3;
    pub const NAPALM: i32 = 4;
    pub const COMLINK: i32 = 5;
}

/// BO2's `ActionSlotType`: what a d-pad slot does.
pub mod action_slot {
    pub const DONOTHING: i32 = 0;
    pub const SPECIFYWEAPON: i32 = 1;
    pub const ALTWEAPONTOGGLE: i32 = 2;
    pub const NIGHTVISION: i32 = 3;
    pub const COUNT: usize = 4;
}

/// BO2's `objectiveState_t`.
pub mod objective_state {
    pub const BLANK: i32 = 0;
    pub const ACTIVE: i32 = 1;
    pub const INVISIBLE: i32 = 2;
}

/// BO2's `he_type_t`: what a HUD element shows.
pub mod hud_type {
    pub const FREE: u8 = 0;
    pub const TEXT: u8 = 1;
    pub const VALUE: u8 = 2;
    pub const PLAYERNAME: u8 = 3;
    pub const MAPNAME: u8 = 4;
    pub const GAMETYPE: u8 = 5;
    pub const DAMAGE_INDICATOR: u8 = 6;
    pub const SCORE: u8 = 7;
    pub const MATERIAL: u8 = 8;
    pub const TIMER_DOWN: u8 = 9;
    pub const TIMER_UP: u8 = 10;
    pub const TENTHS_TIMER_DOWN: u8 = 11;
    pub const TENTHS_TIMER_UP: u8 = 12;
    pub const CLOCK_DOWN: u8 = 13;
    pub const CLOCK_UP: u8 = 14;
    pub const WAYPOINT: u8 = 15;
    pub const PERKS: u8 = 16;
}

/// Sizes of BO2's per-player arrays.
pub const MAX_HELD_WEAPONS: usize = 15;
pub const MAX_OBJECTIVES: usize = 32;
pub const MAX_HUDELEMS_CURRENT: usize = 31;
pub const MAX_HUDELEMS_ARCHIVAL: usize = 31;
pub const MAX_PREDICTABLE_EVENTS: usize = 4;
pub const MAX_STATS: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ads_blocked_states() {
        assert!(!weapon_state::blocks_ads(weapon_state::OFFHAND_INIT));
        assert!(weapon_state::blocks_ads(weapon_state::OFFHAND_PREPARE));
        assert!(weapon_state::blocks_ads(weapon_state::DETONATING));
        assert!(!weapon_state::blocks_ads(weapon_state::SPRINT_RAISE));
        assert!(!weapon_state::blocks_ads(weapon_state::DTP_IN));
        assert!(weapon_state::blocks_ads(weapon_state::DTP_LOOP));
        assert!(weapon_state::blocks_ads(weapon_state::SLIDE_IN));
        assert!(!weapon_state::blocks_ads(weapon_state::FIRING_LEFT));
    }

    #[test]
    fn dead_is_ten_and_up() {
        assert!(!movement_mode::is_dead(movement_mode::REVIVEE));
        assert!(movement_mode::is_dead(movement_mode::DEAD));
        assert!(movement_mode::is_dead(movement_mode::DEAD_LINKED));
    }
}
