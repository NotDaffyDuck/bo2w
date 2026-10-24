//! Black Ops II's own rules the frame runs that the legacy movement keeps to
//! itself: the crawl-and-aim hold, the aiming (walking) flag, Stamin-Up and
//! sprint in any direction (BO2's sprint itself is in sprint.rs). bo2mp: copied
//! from the legacy movement's feel for the BO2 frame, the BO2 side only, plus BO2's
//! turret check on the crawl hold.

use crate::bridge::{BUTTON_ADS, Bo2Feel, CmdScaleWalkContext, add_event};
use playerstate_t6::game::{PM_TYPE_DEAD, PlayerState, UserCmd, pm_flags};

use crate::gun::{self, Crawl};

/// EV_RESET_ADS: the gun drops out of its aim.
const EV_RESET_ADS: i32 = 0x11;

/// On a turret (our e_flags; BO2's 0x300).
pub(crate) const EF_TURRET: u32 = 0xc00;

/// Stalker (specialty_stalker): aiming does not slow the walk.
const PERK_STALKER: u32 = 1 << 11;

const BUTTON_SPRINT: u32 = 0x2;

/// In Zombies, Stamin-Up raises the gun's move speed to at least this while
/// sprinting.
const STAMIN_UP_SPRINT_MOVE_SPEED_SCALE: f32 = 1.1;

/// A landing counts as "landed higher" only this far above the take-off.
pub(crate) const JUMP_LAND_RAISE: f32 = 18.0;

/// Ground friction scale during the hard-landing stun.
pub(crate) const HARD_LANDING_FRICTION_SCALE: f32 = 0.3;

/// Walking acceleration on slick or sliding ground.
pub(crate) const SLICK_ACCEL: f32 = 2.0;

/// Walking acceleration while the knockback timer (pm_flags 0x100) runs.
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
pub(crate) fn prone_aim(ps: &mut PlayerState, cmd: &mut UserCmd, old_move: [i8; 2], scope_raised: bool) {
    let diving = crate::dive::active(ps);
    if ps.pm_flags & pm_flags::PRONE == 0 || scope_raised || ps.e_flags & EF_TURRET != 0 {
        ps.pm_flags &= !pm_flags::PRONEMOVE_OVERRIDDEN;
    } else if !pushed_harder(cmd.forwardmove, old_move[0])
        && !pushed_harder(cmd.rightmove, old_move[1])
    {
        // Ready, raising, dropping or reloading.
        if ps.pm_flags & pm_flags::ADS_INTENT == 0 && gun::lets_the_crawl_hold_go(ps) {
            ps.pm_flags &= !pm_flags::PRONEMOVE_OVERRIDDEN;
        }
    } else if !diving {
        let crawl = gun::crawl(ps);
        if crawl == Crawl::IdlesTheGun {
            rest_the_gun(ps);
        }
        if crawl != Crawl::Blocked {
            ps.pm_flags &= !(pm_flags::PRONEMOVE_OVERRIDDEN | pm_flags::ADS_INTENT);
            add_event(ps, EV_RESET_ADS);
        }
    }

    let lying = matches!(ps.view_height_target, 11 | 22);
    let aiming = ps.pm_flags & pm_flags::ADS_INTENT != 0 && !scope_raised;
    if lying && aiming && !diving {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
    }
    if cmd.forwardmove < 0 {
        ps.pm_flags |= pm_flags::BACKWARDS_RUN;
    } else if cmd.forwardmove > 0 || cmd.rightmove != 0 {
        ps.pm_flags &= !pm_flags::BACKWARDS_RUN;
    }
    if lying && ps.pm_flags & pm_flags::PRONEMOVE_OVERRIDDEN != 0 && !diving {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
    }
}

fn pushed_harder(now: i8, before: i8) -> bool {
    now != before && i16::from(now).abs() > i16::from(before).abs()
}

/// BO2's PM_Weapon_Idle from a crawl: the gun goes to ready. Movement only
/// sets the state; the BO2 gun brain sees it changed under it and rests the
/// gun itself (its timers, the offhand flag, the idle animation) before its
/// next frame. Only the BO2 gun brain's states ask for this.
fn rest_the_gun(ps: &mut PlayerState) {
    ps.weapon_state_t6 = gun::t6::READY;
    ps.weaponstate_primary = 0;
}

/// Sprint in any direction (his Controls menu's "Omni Movement"): the sprint
/// rules see how far the keys push in any direction as a push forward.
/// Without it, BO2's client sends no sprint key while moving back.
pub(crate) fn omni_sprint_cmd(cmd: &UserCmd, feel: Bo2Feel) -> UserCmd {
    let mut out = *cmd;
    if feel.omni {
        let (f, r) = (f32::from(cmd.forwardmove), f32::from(cmd.rightmove));
        out.forwardmove = libm::sqrtf(f * f + r * r).min(127.0) as i8;
    } else if cmd.forwardmove < 0 {
        out.buttons &= !BUTTON_SPRINT;
    }
    out
}

/// Sprint in any direction at full speed: while sprinting, moving back or
/// sideways is not slowed.
pub(crate) fn omni_cmd_scale(ps: &PlayerState, scale: CmdScaleWalkContext, feel: Bo2Feel) -> CmdScaleWalkContext {
    if feel.omni && ps.pm_flags & pm_flags::SPRINTING != 0 {
        CmdScaleWalkContext {
            player_back_speed_scale: 1.0,
            player_strafe_speed_scale: 1.0,
            ..scale
        }
    } else {
        scale
    }
}

/// PM_UpdatePlayerWalkingFlag (after the sprint update): holding aim while
/// the aim is on marks the player as walking, which slows them and quiets
/// footsteps. Not prone, not while reloading, not with Stalker.
pub(crate) fn update_walking_flag(ps: &mut PlayerState, cmd: &UserCmd) {
    ps.pm_flags &= !pm_flags::WALKING;
    if ps.pm_type < PM_TYPE_DEAD
        && cmd.buttons & BUTTON_ADS != 0
        && ps.pm_flags & pm_flags::PRONE == 0
        && ps.pm_flags & pm_flags::ADS_INTENT != 0
        && !gun::reloading(ps)
        && ps.perks[0] & PERK_STALKER == 0
    {
        ps.pm_flags |= pm_flags::WALKING;
    }
}

fn stamin_up(ps: &PlayerState, feel: Bo2Feel) -> bool {
    feel.longer_sprint_perk != 0 && (ps.perks[0] & feel.longer_sprint_perk) != 0
}

/// Stamin-Up multiplies the sprint time, capped at 16383 ms.
pub(crate) fn stamin_up_sprint_time(ps: &PlayerState, max_ms: i32, feel: Bo2Feel) -> i32 {
    if !stamin_up(ps, feel) {
        return max_ms;
    }
    ((max_ms as f32 * feel.sprint_multiplier) as i32).min(0x3fff)
}

/// In Zombies, Stamin-Up sprints with the gun's move speed at least 1.1.
pub(crate) fn stamin_up_cmd_scale(ps: &PlayerState, scale: CmdScaleWalkContext, feel: Bo2Feel) -> CmdScaleWalkContext {
    if feel.zombies
        && stamin_up(ps, feel)
        && ps.pm_flags & pm_flags::SPRINTING != 0
        && scale.weapon_move_speed_scale > 0.0
    {
        CmdScaleWalkContext {
            weapon_move_speed_scale: scale.weapon_move_speed_scale.max(STAMIN_UP_SPRINT_MOVE_SPEED_SCALE),
            ..scale
        }
    } else {
        scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prone_aim_frame(flags: u32, weaponstate: i32, keys: i8, old: i8, scope: bool) -> (i8, u32) {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE | flags;
        ps.view_height_target = 11;
        ps.weapon_state_t6 = weaponstate;
        let mut cmd = UserCmd {
            forwardmove: keys,
            ..UserCmd::default()
        };
        prone_aim(&mut ps, &mut cmd, [old, 0], scope);
        (cmd.forwardmove, ps.pm_flags)
    }

    #[test]
    fn aiming_lying_down_holds_you_still() {
        let aim = pm_flags::ADS_INTENT | pm_flags::PRONEMOVE_OVERRIDDEN;
        assert_eq!(prone_aim_frame(aim, 0, 127, 127, false).0, 0);
        let (keys, flags) = prone_aim_frame(aim, 0, 127, 127, true);
        assert_eq!(keys, 127);
        assert_eq!(flags & pm_flags::PRONEMOVE_OVERRIDDEN, 0);
        let (keys, flags) = prone_aim_frame(aim, 0, 127, 0, false);
        assert_eq!(keys, 127);
        assert_eq!(flags & aim, 0);
        for busy in [6, 0x13] {
            let (keys, flags) = prone_aim_frame(aim, busy, 127, 0, false);
            assert_eq!(keys, 0, "weapon state {busy}");
            assert_eq!(flags & aim, aim);
        }
        assert_eq!(prone_aim_frame(pm_flags::PRONEMOVE_OVERRIDDEN, 6, 127, 127, false).0, 0);
        assert_eq!(prone_aim_frame(pm_flags::PRONEMOVE_OVERRIDDEN, 0, 127, 127, false).0, 127);
    }

    #[test]
    fn the_knife_wind_up_night_vision_and_a_detonator_hold_the_crawl() {
        let aim = pm_flags::ADS_INTENT | pm_flags::PRONEMOVE_OVERRIDDEN;
        for busy in [gun::t6::MELEE_INIT, gun::t6::NIGHTVISION_WEAR, gun::t6::DETONATING] {
            assert_eq!(prone_aim_frame(aim, busy, 127, 0, false).0, 0, "weapon state {busy}");
        }
        // The sprint raise does not hold it.
        assert_eq!(prone_aim_frame(aim, gun::t6::SPRINT_RAISE, 127, 0, false).0, 127);
    }

    /// Lying down aiming, a BO2 gun in `state`, pressing forward from still.
    fn bo2_crawl(state: i32) -> (i8, PlayerState) {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE | pm_flags::ADS_INTENT | pm_flags::PRONEMOVE_OVERRIDDEN;
        ps.view_height_target = 11;
        ps.weapon_state_t6 = state;
        ps.weaponstate_primary = 0x17;
        let mut cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        prone_aim(&mut ps, &mut cmd, [0, 0], false);
        (cmd.forwardmove, ps)
    }

    #[test]
    fn a_crawl_follows_the_bo2_gun() {
        // The riot shield going up holds the crawl.
        let (keys, ps) = bo2_crawl(gun::t6::RIOTSHIELD_PREPARE);
        assert_eq!(keys, 0);
        assert_eq!(ps.weapon_state_t6, gun::t6::RIOTSHIELD_PREPARE);
        // A reload crawls and carries on.
        let (keys, ps) = bo2_crawl(gun::t6::RELOADING);
        assert_eq!(keys, 127);
        assert_eq!(ps.weapon_state_t6, gun::t6::RELOADING);
        assert_eq!(ps.pm_flags & pm_flags::ADS_INTENT, 0);
        // The sprint raise crawls and the gun is put to rest.
        let (keys, ps) = bo2_crawl(gun::t6::SPRINT_RAISE);
        assert_eq!(keys, 127);
        assert_eq!(ps.weapon_state_t6, gun::t6::READY);
        assert_eq!(ps.weaponstate_primary, 0);
    }

    #[test]
    fn the_bo2_reload_lets_the_hold_go_when_the_keys_ease() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE | pm_flags::PRONEMOVE_OVERRIDDEN;
        ps.view_height_target = 11;
        ps.weapon_state_t6 = gun::t6::RELOADING;
        let mut cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        prone_aim(&mut ps, &mut cmd, [127, 0], false);
        assert_eq!(ps.pm_flags & pm_flags::PRONEMOVE_OVERRIDDEN, 0);
        assert_eq!(cmd.forwardmove, 127);
    }

    #[test]
    fn aiming_while_reloading_is_not_walking() {
        let walking = |t6: i32| {
            let mut ps = PlayerState::ZERO;
            ps.pm_flags = pm_flags::ADS_INTENT;
            ps.weapon_state_t6 = t6;
            let cmd = UserCmd {
                buttons: BUTTON_ADS,
                ..UserCmd::default()
            };
            update_walking_flag(&mut ps, &cmd);
            ps.pm_flags & pm_flags::WALKING != 0
        };
        assert!(walking(-1));
        assert!(!walking(gun::t6::RELOADING));
        assert!(!walking(gun::t6::RELOAD_QUICK_EMPTY));
        // BO2's one-hand reloads (9, 10) still walk.
        assert!(walking(9));
    }

    #[test]
    fn a_turret_ends_the_crawl_hold() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE | pm_flags::PRONEMOVE_OVERRIDDEN;
        ps.view_height_target = 11;
        ps.weapon_state_t6 = gun::t6::FIRING;
        ps.e_flags = 0x400;
        let mut cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        prone_aim(&mut ps, &mut cmd, [127, 0], false);
        assert_eq!(ps.pm_flags & pm_flags::PRONEMOVE_OVERRIDDEN, 0);
        assert_eq!(cmd.forwardmove, 127);
    }

    #[test]
    fn backing_up_sets_the_backwards_flag() {
        let mut ps = PlayerState::ZERO;
        let keys = |forward: i8, right: i8, ps: &mut PlayerState| {
            let mut cmd = UserCmd {
                forwardmove: forward,
                rightmove: right,
                ..UserCmd::default()
            };
            prone_aim(ps, &mut cmd, [0, 0], false);
            ps.pm_flags & pm_flags::BACKWARDS_RUN != 0
        };
        assert!(keys(-127, 0, &mut ps));
        assert!(keys(0, 0, &mut ps), "no keys keeps it");
        assert!(!keys(0, 127, &mut ps), "a side key clears it");
        assert!(keys(-127, 0, &mut ps));
        assert!(!keys(127, 0, &mut ps));
    }
}
