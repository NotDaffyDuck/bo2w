//! Small helpers every mechanic shares: idle, viewmodel anim starts, the prone override.

use crate::anim;
use crate::env::{pm_type, pmf};
use crate::player::wf;
use crate::pm::Pm;
use crate::state;

/// PM_Weapon_Idle (also BO2's PM_ResetWeaponState): both hands back to READY with no timers.
pub fn idle(pm: &mut Pm) {
    pm.ps.weap_flags &= !wf::USING_OFFHAND;
    pm.env.pm_flags &= !pmf::PRONE_MOVE_OVERRIDE;
    pm.server_print("end weapon (idle)");
    let ps = &mut *pm.ps;
    ps.weapon_time = 0;
    ps.weapon_delay = 0;
    ps.weaponstate = state::READY;
    ps.weapon_time_left = 0;
    ps.weapon_delay_left = 0;
    ps.weaponstate_left = state::READY;
    if pm.env.pm_type < pm_type::DEAD {
        ps.weap_anim = anim::start(ps.weap_anim, anim::IDLE);
        ps.weap_anim_left = anim::start(ps.weap_anim_left, anim::IDLE);
    }
}

/// PM_StartWeaponAnim: restart `a` on one hand (living players only).
pub fn start_anim(pm: &mut Pm, a: i32, left: bool) {
    if pm.env.pm_type >= pm_type::DEAD {
        return;
    }
    if left {
        pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, a);
    } else {
        pm.ps.weap_anim = anim::start(pm.ps.weap_anim, a);
    }
}

/// PM_ContinueWeaponAnim: start `a` on the hand being run unless it is already playing.
pub fn continue_anim(pm: &mut Pm, a: i32) {
    let left = pm.left();
    let cur = if left {
        pm.ps.weap_anim_left
    } else {
        pm.ps.weap_anim
    };
    if anim::number(cur) != a {
        start_anim(pm, a, left);
    }
}

/// PM_SetProneMovementOverride.
pub fn set_prone_override(pm: &mut Pm) {
    if pm.env.pm_flags & pmf::PRONE != 0 {
        pm.env.pm_flags |= pmf::PRONE_MOVE_OVERRIDE;
    }
}
