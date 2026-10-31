//! The gun states movement reads: Black Ops II's own numbers, from the gun
//! brain (weapon_t6) through `AvatarState::weapon_state_t6`. With no gun brain
//! (-1, a world without BO2's weapons) the gun reads as ready.

use playerstate_t6::game::AvatarState;

/// BO2's gun state numbers (the gun brain's `weapon_t6::state`).
pub(crate) mod t6 {
    pub const READY: i32 = 0;
    pub const DROPPING_ALTSWITCH: i32 = 5;
    pub const FIRING: i32 = 6;
    pub const RECHAMBER_START: i32 = 7;
    pub const RECHAMBERING: i32 = 8;
    pub const RELOADING: i32 = 11;
    pub const RELOAD_QUICK_EMPTY: i32 = 18;
    pub const MELEE_INIT: i32 = 19;
    #[cfg(test)]
    pub const OFFHAND_INIT: i32 = 22;
    pub const RIOTSHIELD_PREPARE: i32 = 30;
    pub const RIOTSHIELD_END: i32 = 34;
    pub const DETONATING: i32 = 35;
    #[cfg(test)]
    pub const SPRINT_RAISE: i32 = 36;
    pub const CONT_FIRE_LOOP: i32 = 40;
    pub const NIGHTVISION_WEAR: i32 = 42;
    pub const NIGHTVISION_REMOVE: i32 = 43;
    pub const LOWREADY_RAISE: i32 = 56;
    pub const LOWREADY_DROP: i32 = 58;
    pub const CRAWL_IN: i32 = 59;
    pub const CRAWL_OUT: i32 = 64;
}

/// The gun's state in BO2's numbers; ready with no BO2 gun brain.
pub(crate) fn state(avatar: &AvatarState) -> i32 {
    avatar.weapon_state_t6.max(t6::READY)
}

/// A knife, a grenade or detonator, or the riot shield at work (BO2's states
/// 19 to 34): no sprint starts, and a running one ends.
pub(crate) fn knife_offhand_or_shield(avatar: &AvatarState) -> bool {
    (t6::MELEE_INIT..=t6::RIOTSHIELD_END).contains(&state(avatar))
}

/// Putting night vision on or taking it off.
pub(crate) fn night_vision(avatar: &AvatarState) -> bool {
    matches!(state(avatar), t6::NIGHTVISION_WEAR | t6::NIGHTVISION_REMOVE)
}

/// The riot shield being raised, planted or put away (BO2's states 30 to 34).
pub(crate) fn riot_shield(avatar: &AvatarState) -> bool {
    (t6::RIOTSHIELD_PREPARE..=t6::RIOTSHIELD_END).contains(&state(avatar))
}

/// The gun lowered (BO2's states 56 to 58): no sprint starts, and a running
/// one ends.
pub(crate) fn low_ready(avatar: &AvatarState) -> bool {
    (t6::LOWREADY_RAISE..=t6::LOWREADY_DROP).contains(&state(avatar))
}

/// Reloading, by BO2's whole reload (not a dual-wield gun's one-hand reload).
pub(crate) fn reloading(avatar: &AvatarState) -> bool {
    (t6::RELOADING..=t6::RELOAD_QUICK_EMPTY).contains(&state(avatar))
}

/// The gun lets a prone player's crawl hold go: ready, raising, dropping, or
/// in the reload itself.
pub(crate) fn lets_the_crawl_hold_go(avatar: &AvatarState) -> bool {
    matches!(state(avatar), t6::READY..=t6::DROPPING_ALTSWITCH | t6::RELOADING)
}

/// What a crawl does to the gun (BO2's PM_InteruptWeaponWithProneMove).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Crawl {
    /// The gun is busy: firing, the knife, a throw or detonator, the riot
    /// shield, a continuous-fire loop or night vision. The hold stays.
    Blocked,
    /// The crawl goes ahead and the gun carries on (ready, raising,
    /// dropping, rechambering, reloading, or already crawling).
    GoesAhead,
    /// The crawl goes ahead and puts the gun to rest (anything else, such as
    /// the sprint raise).
    IdlesTheGun,
}

/// BO2's PM_InteruptWeaponWithProneMove.
pub(crate) fn crawl(avatar: &AvatarState) -> Crawl {
    match state(avatar) {
        t6::READY..=t6::DROPPING_ALTSWITCH
        | t6::RECHAMBER_START
        | t6::RECHAMBERING
        | t6::RELOADING..=t6::RELOAD_QUICK_EMPTY
        | t6::CRAWL_IN..=t6::CRAWL_OUT => Crawl::GoesAhead,
        t6::FIRING
        | t6::MELEE_INIT..=t6::DETONATING
        | t6::CONT_FIRE_LOOP
        | t6::NIGHTVISION_WEAR
        | t6::NIGHTVISION_REMOVE => Crawl::Blocked,
        _ => Crawl::IdlesTheGun,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bo2(state: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.weapon_state_t6 = state;
        avatar
    }

    #[test]
    fn no_bo2_gun_brain_reads_as_ready() {
        assert_eq!(AvatarState::ZERO.weapon_state_t6, -1);
        assert_eq!(state(&AvatarState::ZERO), t6::READY);
        assert_eq!(crawl(&AvatarState::ZERO), Crawl::GoesAhead);
        assert!(!reloading(&AvatarState::ZERO) && lets_the_crawl_hold_go(&AvatarState::ZERO));
    }

    #[test]
    fn the_knife_from_its_wind_up_throws_and_the_shield_stop_a_sprint() {
        for s in 19..=34 {
            assert!(knife_offhand_or_shield(&bo2(s)), "BO2 state {s}");
        }
        for s in [0, 6, 18, t6::DETONATING, t6::SPRINT_RAISE, t6::NIGHTVISION_WEAR] {
            assert!(!knife_offhand_or_shield(&bo2(s)), "BO2 state {s}");
        }
    }

    #[test]
    fn night_vision_is_both_ways() {
        assert!(night_vision(&bo2(t6::NIGHTVISION_WEAR)) && night_vision(&bo2(t6::NIGHTVISION_REMOVE)));
        // BO2's 0x1d is an offhand state, not night vision.
        assert!(!night_vision(&bo2(0x1d)));
    }

    #[test]
    fn the_riot_shield_is_states_30_to_34() {
        for s in 30..=34 {
            assert!(riot_shield(&bo2(s)), "BO2 state {s}");
        }
        assert!(!riot_shield(&bo2(29)) && !riot_shield(&bo2(35)) && !riot_shield(&AvatarState::ZERO));
    }

    #[test]
    fn reloading_is_the_whole_reload() {
        for s in 11..=18 {
            assert!(reloading(&bo2(s)), "BO2 state {s}");
        }
        assert!(!reloading(&bo2(9)) && !reloading(&bo2(10)) && !reloading(&bo2(19)));
    }

    #[test]
    fn the_crawl_hold_goes_at_rest_or_reloading() {
        for s in [0, 1, 2, 3, 4, 5, 11] {
            assert!(lets_the_crawl_hold_go(&bo2(s)), "BO2 state {s}");
        }
        for s in [6, 7, 8, 12, 19] {
            assert!(!lets_the_crawl_hold_go(&bo2(s)), "BO2 state {s}");
        }
    }

    #[test]
    fn a_crawl_follows_bo2s_table() {
        for s in [0, 5, 7, 8, 11, 18, 59, 64] {
            assert_eq!(crawl(&bo2(s)), Crawl::GoesAhead, "BO2 state {s}");
        }
        for s in [6, 19, 21, 22, 29, 30, 34, 35, 40, 42, 43] {
            assert_eq!(crawl(&bo2(s)), Crawl::Blocked, "BO2 state {s}");
        }
        for s in [9, 10, 36, 37, 39, 41, 44, 52, 55, 56] {
            assert_eq!(crawl(&bo2(s)), Crawl::IdlesTheGun, "BO2 state {s}");
        }
    }
}
