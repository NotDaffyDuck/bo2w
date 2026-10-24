//! Whether the player is asking to aim down the sights this frame.

use playerstate_t6::game::{PlayerState, UserCmd, pm_flags};

use crate::bridge::add_predictable_event;

/// The sprint button in `UserCmd::buttons`.
const BUTTON_SPRINT: u32 = 0x2;
/// The aim button.
pub const BUTTON_ADS: u32 = 0x800;
/// The button that holds a scope up while sprinting.
const BUTTON_SCOPE_HOLD: u32 = 0x2000;

/// The event that tells the client to drop out of aiming.
const EV_RESET_ADS: i32 = 0x11;

/// What the held weapon and the server settings say about aiming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdsIntentContext {
    /// The weapon and the player's state allow aiming at all.
    pub ads_allowed: bool,
    /// The weapon is a scope.
    pub weapon_def_scope: bool,
    /// The server lets a sprint carry on into aiming.
    pub sprint_hold_ads: bool,
}

/// What the aim update decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdsIntentResult {
    /// The aim animation should play.
    pub ads_anim_enabled: bool,
    /// A sprint just forced the player out of aiming.
    pub exit_ads_event: bool,
}

/// A scope stays up while crawling only once it is fully raised.
fn scope_fully_raised(ps: &PlayerState, context: AdsIntentContext) -> bool {
    context.weapon_def_scope && ps.f_weapon_pos_frac >= 1.0
}

/// Set or clear the aim intent from this frame's buttons. Sprinting cancels
/// it (and says so with a reset event) unless a held scope or a sprint-into-aim
/// setting keeps it; a prone player who is already crawling needs a fresh
/// press, or to stand still, to start aiming.
pub fn update_ads_intent(
    ps: &mut PlayerState,
    cmd: &UserCmd,
    old_buttons: u32,
    context: AdsIntentContext,
) -> AdsIntentResult {
    ps.pm_flags &= !pm_flags::ADS_INTENT;

    let mut ads_allowed = context.ads_allowed;
    let mut exit_ads_event = false;

    let scope_held = context.weapon_def_scope && cmd.buttons & BUTTON_SCOPE_HOLD != 0;
    let sprint_carries_aim = context.sprint_hold_ads && ps.f_weapon_pos_frac != 0.0;
    if cmd.buttons & BUTTON_SPRINT != 0 && !scope_held && !sprint_carries_aim {
        add_predictable_event(ps, EV_RESET_ADS, 0);
        exit_ads_event = true;
        ps.pm_flags &= !pm_flags::ADS_INTENT;
        ads_allowed = false;
    }

    if cmd.buttons & BUTTON_ADS != 0 && ads_allowed {
        if ps.pm_flags & pm_flags::PRONE != 0 && !scope_fully_raised(ps, context) {
            let moving = cmd.forwardmove != 0 || cmd.rightmove != 0;
            let fresh_press = old_buttons & BUTTON_ADS == 0;
            if fresh_press || !moving {
                ps.pm_flags |= pm_flags::ADS_INTENT | pm_flags::PRONEMOVE_OVERRIDDEN;
            }
        } else {
            ps.pm_flags |= pm_flags::ADS_INTENT;
        }
    }

    AdsIntentResult {
        ads_anim_enabled: ps.pm_flags & pm_flags::ADS_INTENT != 0,
        exit_ads_event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: AdsIntentContext = AdsIntentContext {
        ads_allowed: true,
        weapon_def_scope: false,
        sprint_hold_ads: false,
    };

    fn pressing(buttons: u32) -> UserCmd {
        UserCmd {
            buttons,
            ..UserCmd::default()
        }
    }

    #[test]
    fn holding_aim_while_standing_sets_the_intent() {
        let mut ps = PlayerState::ZERO;
        let result = update_ads_intent(&mut ps, &pressing(BUTTON_ADS), 0, OPEN);
        assert!(result.ads_anim_enabled && !result.exit_ads_event);
    }

    #[test]
    fn aim_is_refused_when_the_weapon_cannot_aim() {
        let mut ps = PlayerState::ZERO;
        let context = AdsIntentContext {
            ads_allowed: false,
            ..OPEN
        };
        assert!(!update_ads_intent(&mut ps, &pressing(BUTTON_ADS), 0, context).ads_anim_enabled);
    }

    #[test]
    fn sprinting_drops_the_aim_and_raises_a_reset_event() {
        let mut ps = PlayerState::ZERO;
        let result = update_ads_intent(&mut ps, &pressing(BUTTON_ADS | BUTTON_SPRINT), 0, OPEN);
        assert!(!result.ads_anim_enabled && result.exit_ads_event);
        assert_eq!(ps.predictable_events[0], EV_RESET_ADS);
    }

    #[test]
    fn a_held_scope_keeps_aiming_through_a_sprint() {
        let mut ps = PlayerState::ZERO;
        let context = AdsIntentContext {
            weapon_def_scope: true,
            ..OPEN
        };
        let buttons = BUTTON_ADS | BUTTON_SPRINT | BUTTON_SCOPE_HOLD;
        assert!(update_ads_intent(&mut ps, &pressing(buttons), 0, context).ads_anim_enabled);
    }

    #[test]
    fn a_crawling_player_needs_a_fresh_press_to_start_aiming() {
        let crawl = UserCmd {
            forwardmove: 127,
            buttons: BUTTON_ADS,
            ..UserCmd::default()
        };
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE;
        assert!(!update_ads_intent(&mut ps, &crawl, BUTTON_ADS, OPEN).ads_anim_enabled);
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE;
        let result = update_ads_intent(&mut ps, &crawl, 0, OPEN);
        assert!(result.ads_anim_enabled);
        assert_ne!(ps.pm_flags & pm_flags::PRONEMOVE_OVERRIDDEN, 0);
    }

    /// A scope held while crawling stays up only once fully raised.
    #[test]
    fn crawling_keeps_a_scope_only_when_fully_raised() {
        let crawl = UserCmd {
            forwardmove: 127,
            buttons: BUTTON_ADS,
            ..UserCmd::default()
        };
        let context = AdsIntentContext {
            weapon_def_scope: true,
            ..OPEN
        };
        for (raised, aims) in [(0.5, false), (1.0, true)] {
            let mut ps = PlayerState::ZERO;
            ps.pm_flags = pm_flags::PRONE;
            ps.f_weapon_pos_frac = raised;
            let result = update_ads_intent(&mut ps, &crawl, BUTTON_ADS, context);
            assert_eq!(result.ads_anim_enabled, aims, "raised {raised}");
        }
    }
}
