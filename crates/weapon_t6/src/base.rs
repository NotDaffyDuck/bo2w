//! Small helpers every mechanic shares: idle, viewmodel anim starts, the prone override.

use crate::anim;
use crate::env::{movement_mode, pmf};
use crate::player::wf;
use crate::pm::Pm;
use crate::state;

/// PM_Weapon_Idle (also BO2's PM_ResetWeaponState): both hands back to READY with no timers.
pub fn idle(pm: &mut Pm) {
    pm.ps.weapon_state_bits &= !wf::USING_OFFHAND;
    pm.env.movement_state_bits &= !pmf::PRONE_MOVE_OVERRIDE;
    pm.server_print("end weapon (idle)");
    let ps = &mut *pm.ps;
    ps.gun_timer_ms = Default::default();
    ps.weapon_ready_wait = 0;
    ps.gun_phase = state::READY;
    ps.weapon_time_left = 0;
    ps.weapon_delay_left = 0;
    ps.weaponstate_left = state::READY;
    if pm.env.movement_mode < movement_mode::DEAD {
        ps.gun_clip = anim::start(ps.gun_clip, anim::REST);
        ps.weap_anim_left = anim::start(ps.weap_anim_left, anim::REST);
    }
}

/// PM_StartWeaponAnim: restart `a` on one hand (living players only).
pub fn start_anim(pm: &mut Pm, a: i32, left: bool) {
    if pm.env.movement_mode >= movement_mode::DEAD {
        return;
    }
    if left {
        pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, a);
    } else {
        pm.ps.gun_clip = anim::start(pm.ps.gun_clip, a);
    }
}

/// PM_ContinueWeaponAnim: start `a` on the hand being run unless it is already playing.
pub fn continue_anim(pm: &mut Pm, a: i32) {
    let left = pm.left();
    let cur = if left {
        pm.ps.weap_anim_left
    } else {
        pm.ps.gun_clip
    };
    if anim::number(cur) != a {
        start_anim(pm, a, left);
    }
}

/// PM_SetProneMovementOverride.
pub fn set_prone_override(pm: &mut Pm) {
    if pm.env.movement_state_bits & pmf::LYING_FLAT != 0 {
        pm.env.movement_state_bits |= pmf::PRONE_MOVE_OVERRIDE;
    }
}
