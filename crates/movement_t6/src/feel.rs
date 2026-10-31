//! Black Ops II's own rules the frame runs that the legacy movement keeps to
//! itself: the crawl-and-aim hold, the aiming (walking) flag, Stamin-Up and
//! sprint in any direction (BO2's sprint itself is in sprint.rs). bo2mp: copied
//! from the legacy movement's feel for the BO2 frame, the BO2 side only, plus BO2's
//! turret check on the crawl hold.

use crate::bridge::{INTENT_SCOPE_BIT, Bo2Feel, StrideScaleArgs, add_event};
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::AvatarState;
use playerstate_t6::game::InputCommand;
use playerstate_t6::game::movement_state_bits;
use playerstate_t6::perks::PERK_STALKER;

use crate::gun::t6::READY;
use crate::gun::{Crawl, crawl, lets_the_crawl_hold_go, reloading};

/// SIGNAL_SCOPE_RESTART: the gun drops out of its aim.
const SIGNAL_SCOPE_RESTART: i32 = 0x11;

/// On a turret (BO2's 0x300).
use playerstate_t6::game::entity_flag_bits::TURRET_MASK;

const INTENT_DASH_BIT: u32 = 0x2;

/// In Zombies, Stamin-Up raises the gun's move speed to at least this while
/// sprinting.
const STAMIN_UP_SPRINT_MOVE_SPEED_SCALE: f32 = 1.1;

/// A landing counts as "landed higher" only this far above the take-off.
pub(crate) const JUMP_LAND_RAISE: f32 = 18.0;

/// Ground friction scale during the hard-landing stun.
pub(crate) const HARD_LANDING_FRICTION_SCALE: f32 = 0.3;

/// Walking acceleration on slick or sliding ground.
pub(crate) const SLICK_ACCEL: f32 = 2.0;

/// Walking acceleration while the knockback timer (movement_state_bits 0x100) runs.
pub(crate) const KNOCKBACK_ACCEL: f32 = 1.0;

/// player_sliding_friction: sliding ground still slows you, this much times
/// your speed each second.
pub(crate) const SLIDING_FRICTION: f32 = 1.5;

/// Black Ops II's crawl and aim (PmoveSingle, before the aim update, so it
/// reads last frame's aim). Lying down (or downed) and aiming holds the
/// player still, unless a scope is fully raised. Pressing a move key harder
/// than last frame drops the aim and crawls, unless the gun is busy (BO2's
/// PM_InteruptWeaponWithProneMove, in gun.rs); a crawl from some gun states
/// puts the gun to rest. The hold stays until the keys ease off with the gun
/// at rest. On a turret there is no hold. Also sets the run-backwards flag
/// from the keys.
pub(crate) fn prone_aim(avatar: &mut AvatarState, ucmd: &mut InputCommand, old_move: [i8; 2], scope_raised: bool) {
    let diving = crate::dive::active(avatar);
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0 || scope_raised || avatar.e_flags & TURRET_MASK != 0 {
        avatar.movement_state_bits &= !movement_state_bits::PRONEMOVE_OVERRIDDEN;
    } else if !pushed_harder(ucmd.thrust_ahead, old_move[0])
        && !pushed_harder(ucmd.thrust_lateral, old_move[1])
    {
        // Ready, raising, dropping or reloading.
        if avatar.movement_state_bits & movement_state_bits::ADS_INTENT == 0 && lets_the_crawl_hold_go(avatar) {
            avatar.movement_state_bits &= !movement_state_bits::PRONEMOVE_OVERRIDDEN;
        }
    } else if !diving {
        let crawl = crawl(avatar);
        if crawl == Crawl::IdlesTheGun {
            rest_the_gun(avatar);
        }
        if crawl != Crawl::Blocked {
            avatar.movement_state_bits &= !(movement_state_bits::PRONEMOVE_OVERRIDDEN | movement_state_bits::ADS_INTENT);
            add_event(avatar, SIGNAL_SCOPE_RESTART);
        }
    }

    let lying = matches!(avatar.eye_height_goal, 11 | 22);
    let aiming = avatar.movement_state_bits & movement_state_bits::ADS_INTENT != 0 && !scope_raised;
    if lying && aiming && !diving {
        ucmd.thrust_ahead = 0;
        ucmd.thrust_lateral = 0;
    }
    if ucmd.thrust_ahead < 0 {
        avatar.movement_state_bits |= movement_state_bits::BACKWARDS_RUN;
    } else if ucmd.thrust_ahead > 0 || ucmd.thrust_lateral != 0 {
        avatar.movement_state_bits &= !movement_state_bits::BACKWARDS_RUN;
    }
    if lying && avatar.movement_state_bits & movement_state_bits::PRONEMOVE_OVERRIDDEN != 0 && !diving {
        ucmd.thrust_ahead = 0;
        ucmd.thrust_lateral = 0;
    }
}

fn pushed_harder(now: i8, before: i8) -> bool {
    now != before && i16::from(now).abs() > i16::from(before).abs()
}

/// BO2's PM_Weapon_Idle from a crawl: the gun goes to ready. Movement only
/// sets the state; the BO2 gun brain sees it changed under it and rests the
/// gun itself (its timers, the offhand flag, the idle animation) before its
/// next frame. Only the BO2 gun brain's states ask for this.
fn rest_the_gun(avatar: &mut AvatarState) {
    avatar.weapon_state_t6 = READY;
    avatar.weaponstate_primary = 0;
}

/// Sprint in any direction (his Controls menu's "Omni Movement"): the sprint
/// rules see how far the keys push in any direction as a push forward.
/// Without it, BO2's client sends no sprint key while moving back.
pub(crate) fn omni_sprint_cmd(ucmd: &InputCommand, feel: Bo2Feel) -> InputCommand {
    let mut out = *ucmd;
    if feel.omni {
        let (f, r) = (f32::from(ucmd.thrust_ahead), f32::from(ucmd.thrust_lateral));
        out.thrust_ahead = libm::sqrtf(f * f + r * r).min(127.0) as i8;
    } else if ucmd.thrust_ahead < 0 {
        out.buttons &= !INTENT_DASH_BIT;
    }
    out
}

/// Sprint in any direction at full speed: while sprinting, moving back or
/// sideways is not slowed.
pub(crate) fn omni_cmd_scale(avatar: &AvatarState, scale: StrideScaleArgs, feel: Bo2Feel) -> StrideScaleArgs {
    if feel.omni && avatar.movement_state_bits & movement_state_bits::SPRINTING != 0 {
        StrideScaleArgs {
            retreat_pace_ratio: 1.0,
            sidestep_pace_ratio: 1.0,
            ..scale
        }
    } else {
        scale
    }
}

/// PM_UpdatePlayerWalkingFlag (after the sprint update): holding aim while
/// the aim is on marks the player as walking, which slows them and quiets
/// footsteps. Not prone, not while reloading, not with Stalker.
pub(crate) fn update_walking_flag(avatar: &mut AvatarState, ucmd: &InputCommand) {
    avatar.movement_state_bits &= !movement_state_bits::WALKING;
    if avatar.movement_mode < MOVE_KIND_DEAD
        && ucmd.buttons & INTENT_SCOPE_BIT != 0
        && avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0
        && avatar.movement_state_bits & movement_state_bits::ADS_INTENT != 0
        && !reloading(avatar)
        && !avatar.has_perk_bit(PERK_STALKER)
    {
        avatar.movement_state_bits |= movement_state_bits::WALKING;
    }
}

fn stamin_up(avatar: &AvatarState, feel: Bo2Feel) -> bool {
    avatar.has_perk_bit(feel.longer_sprint_perk)
}

/// Stamin-Up multiplies the sprint time, capped at 16383 ms.
pub(crate) fn stamin_up_sprint_time(avatar: &AvatarState, max_ms: i32, feel: Bo2Feel) -> i32 {
    if !stamin_up(avatar, feel) {
        return max_ms;
    }
    ((max_ms as f32 * feel.sprint_multiplier) as i32).min(0x3fff)
}

/// In Zombies, Stamin-Up sprints with the gun's move speed at least 1.1.
pub(crate) fn stamin_up_cmd_scale(avatar: &AvatarState, scale: StrideScaleArgs, feel: Bo2Feel) -> StrideScaleArgs {
    if feel.zombies
        && stamin_up(avatar, feel)
        && avatar.movement_state_bits & movement_state_bits::SPRINTING != 0
        && scale.gun_stride_ratio > 0.0
    {
        StrideScaleArgs {
            gun_stride_ratio: scale.gun_stride_ratio.max(STAMIN_UP_SPRINT_MOVE_SPEED_SCALE),
            ..scale
        }
    } else {
        scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gun::t6::{
        DETONATING, FIRING, MELEE_INIT, NIGHTVISION_WEAR, RELOADING, RELOAD_QUICK_EMPTY, RIOTSHIELD_PREPARE,
        SPRINT_RAISE,
    };

    fn prone_aim_frame(flags: u32, weapon_state: i32, keys: i8, old: i8, scope: bool) -> (i8, u32) {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT | flags;
        avatar.eye_height_goal = 11;
        avatar.weapon_state_t6 = weapon_state;
        let mut ucmd = InputCommand {
            thrust_ahead: keys,
            ..InputCommand::default()
        };
        prone_aim(&mut avatar, &mut ucmd, [old, 0], scope);
        (ucmd.thrust_ahead, avatar.movement_state_bits)
    }

    #[test]
    fn aiming_lying_down_holds_you_still() {
        let aim = movement_state_bits::ADS_INTENT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
        assert_eq!(prone_aim_frame(aim, 0, 127, 127, false).0, 0);
        let (keys, flags) = prone_aim_frame(aim, 0, 127, 127, true);
        assert_eq!(keys, 127);
        assert_eq!(flags & movement_state_bits::PRONEMOVE_OVERRIDDEN, 0);
        let (keys, flags) = prone_aim_frame(aim, 0, 127, 0, false);
        assert_eq!(keys, 127);
        assert_eq!(flags & aim, 0);
        for busy in [6, 0x13] {
            let (keys, flags) = prone_aim_frame(aim, busy, 127, 0, false);
            assert_eq!(keys, 0, "weapon state {busy}");
            assert_eq!(flags & aim, aim);
        }
        assert_eq!(prone_aim_frame(movement_state_bits::PRONEMOVE_OVERRIDDEN, 6, 127, 127, false).0, 0);
        assert_eq!(prone_aim_frame(movement_state_bits::PRONEMOVE_OVERRIDDEN, 0, 127, 127, false).0, 127);
    }

    #[test]
    fn the_knife_wind_up_night_vision_and_a_detonator_hold_the_crawl() {
        let aim = movement_state_bits::ADS_INTENT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
        for busy in [MELEE_INIT, NIGHTVISION_WEAR, DETONATING] {
            assert_eq!(prone_aim_frame(aim, busy, 127, 0, false).0, 0, "weapon state {busy}");
        }
        // The sprint raise does not hold it.
        assert_eq!(prone_aim_frame(aim, SPRINT_RAISE, 127, 0, false).0, 127);
    }

    /// Lying down aiming, a BO2 gun in `state`, pressing forward from still.
    fn bo2_crawl(state: i32) -> (i8, AvatarState) {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT | movement_state_bits::ADS_INTENT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
        avatar.eye_height_goal = 11;
        avatar.weapon_state_t6 = state;
        avatar.weaponstate_primary = 0x17;
        let mut ucmd = InputCommand {
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        prone_aim(&mut avatar, &mut ucmd, [0, 0], false);
        (ucmd.thrust_ahead, avatar)
    }

    #[test]
    fn a_crawl_follows_the_bo2_gun() {
        // The riot shield going up holds the crawl.
        let (keys, avatar) = bo2_crawl(RIOTSHIELD_PREPARE);
        assert_eq!(keys, 0);
        assert_eq!(avatar.weapon_state_t6, RIOTSHIELD_PREPARE);
        // A reload crawls and carries on.
        let (keys, avatar) = bo2_crawl(RELOADING);
        assert_eq!(keys, 127);
        assert_eq!(avatar.weapon_state_t6, RELOADING);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::ADS_INTENT, 0);
        // The sprint raise crawls and the gun is put to rest.
        let (keys, avatar) = bo2_crawl(SPRINT_RAISE);
        assert_eq!(keys, 127);
        assert_eq!(avatar.weapon_state_t6, READY);
        assert_eq!(avatar.weaponstate_primary, 0);
    }

    #[test]
    fn the_bo2_reload_lets_the_hold_go_when_the_keys_ease() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
        avatar.eye_height_goal = 11;
        avatar.weapon_state_t6 = RELOADING;
        let mut ucmd = InputCommand {
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        prone_aim(&mut avatar, &mut ucmd, [127, 0], false);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::PRONEMOVE_OVERRIDDEN, 0);
        assert_eq!(ucmd.thrust_ahead, 127);
    }

    #[test]
    fn aiming_while_reloading_is_not_walking() {
        let walking = |t6: i32| {
            let mut avatar = AvatarState::ZERO;
            avatar.movement_state_bits = movement_state_bits::ADS_INTENT;
            avatar.weapon_state_t6 = t6;
            let ucmd = InputCommand {
                buttons: INTENT_SCOPE_BIT,
                ..InputCommand::default()
            };
            update_walking_flag(&mut avatar, &ucmd);
            avatar.movement_state_bits & movement_state_bits::WALKING != 0
        };
        assert!(walking(-1));
        assert!(!walking(RELOADING));
        assert!(!walking(RELOAD_QUICK_EMPTY));
        // BO2's one-hand reloads (9, 10) still walk.
        assert!(walking(9));
    }

    #[test]
    fn a_turret_ends_the_crawl_hold() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT | movement_state_bits::PRONEMOVE_OVERRIDDEN;
        avatar.eye_height_goal = 11;
        avatar.weapon_state_t6 = FIRING;
        avatar.e_flags = playerstate_t6::game::entity_flag_bits::TURRET_STAND;
        let mut ucmd = InputCommand {
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        prone_aim(&mut avatar, &mut ucmd, [127, 0], false);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::PRONEMOVE_OVERRIDDEN, 0);
        assert_eq!(ucmd.thrust_ahead, 127);
    }

    #[test]
    fn backing_up_sets_the_backwards_flag() {
        let mut avatar = AvatarState::ZERO;
        let keys = |forward: i8, right: i8, avatar: &mut AvatarState| {
            let mut ucmd = InputCommand {
                thrust_ahead: forward,
                thrust_lateral: right,
                ..InputCommand::default()
            };
            prone_aim(avatar, &mut ucmd, [0, 0], false);
            avatar.movement_state_bits & movement_state_bits::BACKWARDS_RUN != 0
        };
        assert!(keys(-127, 0, &mut avatar));
        assert!(keys(0, 0, &mut avatar), "no keys keeps it");
        assert!(!keys(0, 127, &mut avatar), "a side key clears it");
        assert!(keys(-127, 0, &mut avatar));
        assert!(!keys(127, 0, &mut avatar));
    }
}
