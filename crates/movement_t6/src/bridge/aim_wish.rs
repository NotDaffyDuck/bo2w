//! Whether the player is asking to aim down the sights this frame.

use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};

use crate::bridge::queue_forecast_happening;

/// The sprint button in `InputCommand::buttons`.
const INTENT_DASH_BIT: u32 = 0x2;
/// The aim button.
pub const INTENT_SCOPE_BIT: u32 = 0x800;
/// The button that holds a scope up while sprinting.
const INTENT_BREATH_BIT: u32 = 0x2000;

/// The event that tells the client to drop out of aiming.
const SIGNAL_SCOPE_RESTART: i32 = 0x11;

/// What the held weapon and the server settings say about aiming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeWishArgs {
    /// The weapon and the player's state allow aiming at all.
    pub sights_permitted: bool,
    /// The weapon is a scope.
    pub scoped_weapon: bool,
    /// The server lets a sprint carry on into aiming.
    pub dash_keeps_scope: bool,
}

/// What the aim update decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeWishOutcome {
    /// The aim animation should play.
    pub scope_clip_allowed: bool,
    /// A sprint just forced the player out of aiming.
    pub leave_scope_signal: bool,
}

/// A scope stays up while crawling only once it is fully raised.
fn scope_fully_raised(avatar: &AvatarState, frame_ctx: ScopeWishArgs) -> bool {
    frame_ctx.scoped_weapon && avatar.weapon_raise_fraction >= 1.0
}

/// Set or clear the aim intent from this frame's buttons. Sprinting cancels
/// it (and says so with a reset event) unless a held scope or a sprint-into-aim
/// setting keeps it; a prone player who is already crawling needs a fresh
/// press, or to stand still, to start aiming.
pub fn update_ads_intent(
    avatar: &mut AvatarState,
    ucmd: &InputCommand,
    last_tick_buttons: u32,
    frame_ctx: ScopeWishArgs,
) -> ScopeWishOutcome {
    avatar.movement_state_bits &= !movement_state_bits::ADS_INTENT;

    let mut sights_permitted = frame_ctx.sights_permitted;
    let mut leave_scope_signal = false;

    let scope_held = frame_ctx.scoped_weapon && ucmd.buttons & INTENT_BREATH_BIT != 0;
    let sprint_carries_aim = frame_ctx.dash_keeps_scope && avatar.weapon_raise_fraction != 0.0;
    if ucmd.buttons & INTENT_DASH_BIT != 0 && !scope_held && !sprint_carries_aim {
        queue_forecast_happening(avatar, SIGNAL_SCOPE_RESTART, 0);
        leave_scope_signal = true;
        avatar.movement_state_bits &= !movement_state_bits::ADS_INTENT;
        sights_permitted = false;
    }

    if ucmd.buttons & INTENT_SCOPE_BIT != 0 && sights_permitted {
        if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 && !scope_fully_raised(avatar, frame_ctx) {
            let moving = ucmd.thrust_ahead != 0 || ucmd.thrust_lateral != 0;
            let fresh_press = last_tick_buttons & INTENT_SCOPE_BIT == 0;
            if fresh_press || !moving {
                avatar.movement_state_bits |= movement_state_bits::ADS_INTENT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
            }
        } else {
            avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        }
    }

    ScopeWishOutcome {
        scope_clip_allowed: avatar.movement_state_bits & movement_state_bits::ADS_INTENT != 0,
        leave_scope_signal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: ScopeWishArgs = ScopeWishArgs {
        sights_permitted: true,
        scoped_weapon: false,
        dash_keeps_scope: false,
    };

    fn pressing(buttons: u32) -> InputCommand {
        InputCommand {
            buttons,
            ..InputCommand::default()
        }
    }

    #[test]
    fn holding_aim_while_standing_sets_the_intent() {
        let mut avatar = AvatarState::ZERO;
        let result = update_ads_intent(&mut avatar, &pressing(INTENT_SCOPE_BIT), 0, OPEN);
        assert!(result.scope_clip_allowed && !result.leave_scope_signal);
    }

    #[test]
    fn aim_is_refused_when_the_weapon_cannot_aim() {
        let mut avatar = AvatarState::ZERO;
        let frame_ctx = ScopeWishArgs {
            sights_permitted: false,
            ..OPEN
        };
        assert!(!update_ads_intent(&mut avatar, &pressing(INTENT_SCOPE_BIT), 0, frame_ctx).scope_clip_allowed);
    }

    #[test]
    fn sprinting_drops_the_aim_and_raises_a_reset_event() {
        let mut avatar = AvatarState::ZERO;
        let result = update_ads_intent(&mut avatar, &pressing(INTENT_SCOPE_BIT | INTENT_DASH_BIT), 0, OPEN);
        assert!(!result.scope_clip_allowed && result.leave_scope_signal);
        assert_eq!(avatar.predictable_events[0], SIGNAL_SCOPE_RESTART);
    }

    #[test]
    fn a_held_scope_keeps_aiming_through_a_sprint() {
        let mut avatar = AvatarState::ZERO;
        let frame_ctx = ScopeWishArgs {
            scoped_weapon: true,
            ..OPEN
        };
        let buttons = INTENT_SCOPE_BIT | INTENT_DASH_BIT | INTENT_BREATH_BIT;
        assert!(update_ads_intent(&mut avatar, &pressing(buttons), 0, frame_ctx).scope_clip_allowed);
    }

    #[test]
    fn a_crawling_player_needs_a_fresh_press_to_start_aiming() {
        let crawl = InputCommand {
            thrust_ahead: 127,
            buttons: INTENT_SCOPE_BIT,
            ..InputCommand::default()
        };
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        assert!(!update_ads_intent(&mut avatar, &crawl, INTENT_SCOPE_BIT, OPEN).scope_clip_allowed);
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        let result = update_ads_intent(&mut avatar, &crawl, 0, OPEN);
        assert!(result.scope_clip_allowed);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::PRONEMOVE_OVERRIDDEN, 0);
    }

    /// A scope held while crawling stays up only once fully raised.
    #[test]
    fn crawling_keeps_a_scope_only_when_fully_raised() {
        let crawl = InputCommand {
            thrust_ahead: 127,
            buttons: INTENT_SCOPE_BIT,
            ..InputCommand::default()
        };
        let frame_ctx = ScopeWishArgs {
            scoped_weapon: true,
            ..OPEN
        };
        for (raised, aims) in [(0.5, false), (1.0, true)] {
            let mut avatar = AvatarState::ZERO;
            avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
            avatar.weapon_raise_fraction = raised;
            let result = update_ads_intent(&mut avatar, &crawl, INTENT_SCOPE_BIT, frame_ctx);
            assert_eq!(result.scope_clip_allowed, aims, "raised {raised}");
        }
    }
}
