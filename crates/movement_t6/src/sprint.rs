//! Black Ops II's PM_UpdateSprint and its two key rules
//! (PM_SprintEndingButtons, PM_SprintStartInterferingButtons), in our
//! button, flag and gun-state numbers.
//!
//! Where BO2 differs from the shared (MW2) sprint:
//! - The aim key itself neither ends nor blocks a sprint; only aiming does
//!   (the ADS flag). With the sprint key held the aim key is ignored, so the
//!   sprint runs on.
//! - Pressing sprint again mid-sprint does nothing (no press-to-stop).
//! - Prone ends a sprint only if the gun allows prone.
//! - A script's sprint block ends a running sprint too.
//! - The gun states that end or block it are BO2's own (gun.rs): the knife
//!   from its wind-up, anything with a grenade or detonator, and the riot
//!   shield; putting night vision on or taking it off ends one but does not
//!   stop one starting. BO2's plain detonating state (35) does neither.
//! - Jumping out of a sprint moves its end 800 ms later.
//! - On slide ground a sprint ends and cannot start.
//! - Waist-deep water ends a running sprint, and a lowered gun (BO2's
//!   low-ready states) ends one and stops one starting.
//!
//! Not seen from here, so left out: BO2's 0x80 key with a non-equipment weapon, an equipment
//! weapon's firing and knife states on the start rule and the no-fire flag on
//! the key-up rule.
//! Without the BO2 gun brain the riot shield's states read as ready.

use crate::bridge::{
    PERK_MARATHON, SprintContext, SprintResult, end_sprint, sprint_forward_below_minimum,
    sprint_time_remaining,
};
use playerstate_t6::game::{PlayerState, UserCmd, pm_flags};

use crate::ground::PMF_SLIDING;

const BUTTON_ATTACK: u32 = 0x1;
const BUTTON_SPRINT: u32 = 0x2;
const BUTTON_MELEE: u32 = 0x4;
const BUTTON_RELOAD: u32 = 0x10;
const BUTTON_USE_RELOAD: u32 = 0x20;
const BUTTON_PRONE: u32 = 0x100;
const BUTTON_CROUCH: u32 = 0x200;
const BUTTON_JUMP: u32 = 0x400;
const BUTTON_FRAG: u32 = 0x4000;
const BUTTON_SMOKE: u32 = 0x8000;

/// Keys that both end a sprint and stop one starting: jump, reload, the
/// knife, fire, use-reload and both grenades.
const BUSY_KEYS: u32 = BUTTON_JUMP
    | BUTTON_RELOAD
    | BUTTON_MELEE
    | BUTTON_ATTACK
    | BUTTON_USE_RELOAD
    | BUTTON_FRAG
    | BUTTON_SMOKE;

/// States that end a sprint: aiming, a ladder, shell shock.
const ENDING_FLAGS: u32 = pm_flags::ADS_INTENT | pm_flags::LADDER | pm_flags::SHELLSHOCKED;

/// States that stop one starting: those plus a ledge climb.
const BLOCKING_FLAGS: u32 = ENDING_FLAGS | pm_flags::MANTLE;

/// Jumping out of a sprint moves its end this much later, so the next one is
/// shorter.
const JUMP_OUT_PENALTY_MS: i32 = 800;

/// PM_SprintEndingButtons: does this frame end a running sprint?
pub(crate) fn ending_buttons(
    ps: &PlayerState,
    cmd: &UserCmd,
    forward_minimum: i32,
    weapon_blocks_prone: bool,
) -> bool {
    if ps.pm_flags & ENDING_FLAGS != 0
        || ps.waterlevel > 1
        || sprint_forward_below_minimum(cmd.forwardmove, forward_minimum)
        || cmd.buttons & BUSY_KEYS != 0
    {
        return true;
    }
    if cmd.buttons & BUTTON_PRONE != 0 && !weapon_blocks_prone {
        return true;
    }
    if cmd.buttons & BUTTON_CROUCH != 0 || ps.leanf != 0.0 {
        return true;
    }
    crate::gun::knife_offhand_or_shield(ps)
        || crate::gun::night_vision(ps)
        || ps.pm_flags & (PMF_SLIDING | pm_flags::SPRINT_BLOCKED) != 0
}

/// PM_SprintStartInterferingButtons: is something stopping a sprint from
/// starting? Waist-deep water (level 2) and deeper does.
pub(crate) fn start_interfering_buttons(ps: &PlayerState, cmd: &UserCmd, forward_minimum: i32) -> bool {
    if ps.pm_flags & pm_flags::LADDER != 0
        || ps.waterlevel > 1
        || sprint_forward_below_minimum(cmd.forwardmove, forward_minimum)
        || cmd.buttons & BUSY_KEYS != 0
    {
        return true;
    }
    if ps.leanf != 0.0 || ps.pm_flags & BLOCKING_FLAGS != 0 {
        return true;
    }
    // Just landed from a jump, with no landing slowdown left: free to go.
    if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time == 0 {
        return false;
    }
    crate::gun::knife_offhand_or_shield(ps) || ps.pm_flags & PMF_SLIDING != 0
}

/// PM_CanCrouch and PM_CanStand: crouched or lying down, the player must have
/// room to stand to start a sprint (`stand_up_clear`).
fn has_room(ps: &PlayerState, stand_up_clear: bool) -> bool {
    ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH) == 0 || stand_up_clear
}

/// PM_UpdateSprint.
pub(crate) fn update(
    ps: &mut PlayerState,
    cmd: &UserCmd,
    context: SprintContext,
    weapon_blocks_prone: bool,
) -> SprintResult {
    let held = cmd.buttons & BUTTON_SPRINT != 0;
    if ps.sprint_state.sprint_button_up_required != 0 && !held {
        ps.sprint_state.sprint_button_up_required = 0;
    }
    let sprinting = ps.pm_flags & pm_flags::SPRINTING != 0;

    // Not walking normally, a lowered gun, or a gun that cannot sprint: any
    // sprint stops.
    if (ps.pm_type != 0 && ps.pm_type != 1)
        || crate::gun::low_ready(ps)
        || context.weapon_max_sprint_time <= 0
    {
        if !sprinting {
            return SprintResult::Unchanged;
        }
        end_sprint(ps, cmd);
        return SprintResult::Ended;
    }

    if sprinting {
        let unlimited = ps.perks[0] & PERK_MARATHON != 0 || context.sprint_forever;
        if !unlimited
            && ps.sprint_state.sprint_start_max_length <= cmd.server_time.wrapping_sub(ps.sprint_state.last_sprint_start)
        {
            end_sprint(ps, cmd);
            ps.sprint_state.sprint_delay = 1;
            return SprintResult::Ended;
        }
        if !ending_buttons(ps, cmd, context.sprint_forward_minimum, weapon_blocks_prone) {
            return SprintResult::Unchanged;
        }
        end_sprint(ps, cmd);
        if cmd.buttons & BUTTON_JUMP != 0 {
            ps.sprint_state.last_sprint_end = ps.sprint_state.last_sprint_end.wrapping_add(JUMP_OUT_PENALTY_MS);
        }
        return SprintResult::Ended;
    }

    if ps.sprint_state.sprint_delay != 0 {
        let since_end = cmd.server_time.wrapping_sub(ps.sprint_state.last_sprint_end) as f32;
        if since_end < context.sprint_delay_seconds * 1000.0 {
            return SprintResult::Unchanged;
        }
    }
    if !held
        || ps.pm_flags & pm_flags::SPRINT_BLOCKED != 0
        || ps.sprint_state.sprint_button_up_required != 0
        || start_interfering_buttons(ps, cmd, context.sprint_forward_minimum)
        || !has_room(ps, context.stand_up_clear)
    {
        return SprintResult::Unchanged;
    }
    let left = sprint_time_remaining(ps, cmd.server_time, context);
    if left as f32 <= context.min_sprint_time_seconds * 1000.0 {
        return SprintResult::Unchanged;
    }
    ps.sprint_state.sprint_start_max_length = left;
    ps.sprint_state.last_sprint_start = cmd.server_time;
    ps.pm_flags |= pm_flags::SPRINTING;
    SprintResult::Started
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gun::t6;

    const BUTTON_ADS: u32 = 0x800;

    fn context() -> SprintContext {
        SprintContext {
            weapon_max_sprint_time: 4000,
            sprint_forever: false,
            min_sprint_time_seconds: 1.0,
            sprint_delay_seconds: 0.0,
            sprint_forward_minimum: 105,
            stand_up_clear: true,
            sprint_recharge_pause_seconds: 0.0,
        }
    }

    fn cmd(time: i32, buttons: u32) -> UserCmd {
        UserCmd {
            server_time: time,
            buttons,
            forwardmove: 127,
            ..UserCmd::default()
        }
    }

    fn sprinting() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        let started = update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false);
        assert_eq!(started, SprintResult::Started);
        ps
    }

    #[test]
    fn the_aim_key_alone_does_not_end_a_sprint() {
        let mut ps = sprinting();
        let frame = cmd(1050, BUTTON_SPRINT | BUTTON_ADS);
        assert_eq!(update(&mut ps, &frame, context(), false), SprintResult::Unchanged);
        // Aiming itself (the flag) does.
        ps.pm_flags |= pm_flags::ADS_INTENT;
        assert_eq!(update(&mut ps, &frame, context(), false), SprintResult::Ended);
    }

    #[test]
    fn the_aim_key_does_not_stop_a_sprint_starting() {
        let mut ps = PlayerState::ZERO;
        let frame = cmd(1000, BUTTON_SPRINT | BUTTON_ADS);
        assert_eq!(update(&mut ps, &frame, context(), false), SprintResult::Started);
    }

    #[test]
    fn pressing_sprint_again_keeps_sprinting() {
        let mut ps = sprinting();
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Unchanged);
        assert_eq!(update(&mut ps, &cmd(1100, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);
        assert_ne!(ps.pm_flags & pm_flags::SPRINTING, 0);
    }

    #[test]
    fn prone_ends_it_unless_the_gun_blocks_prone() {
        let mut ps = sprinting();
        assert_eq!(update(&mut ps, &cmd(1050, BUTTON_PRONE), context(), true), SprintResult::Unchanged);
        assert_eq!(update(&mut ps, &cmd(1100, BUTTON_PRONE), context(), false), SprintResult::Ended);
    }

    #[test]
    fn a_script_block_ends_a_running_sprint() {
        let mut ps = sprinting();
        ps.pm_flags |= pm_flags::SPRINT_BLOCKED;
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Ended);
    }

    #[test]
    fn slide_ground_ends_a_sprint_and_stops_one_starting() {
        let mut ps = sprinting();
        ps.pm_flags |= PMF_SLIDING;
        assert_eq!(update(&mut ps, &cmd(1050, BUTTON_SPRINT), context(), false), SprintResult::Ended);
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = PMF_SLIDING;
        assert_ne!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Started);
    }

    #[test]
    fn jumping_out_moves_the_end_800_ms() {
        let mut ps = sprinting();
        assert_eq!(update(&mut ps, &cmd(1500, BUTTON_JUMP), context(), false), SprintResult::Ended);
        assert_eq!(ps.sprint_state.last_sprint_end, 2300);
        assert_eq!(ps.sprint_state.sprint_delay, 0);
    }

    #[test]
    fn running_out_sets_the_delay_and_no_jump_penalty() {
        let mut ps = sprinting();
        let out = update(&mut ps, &cmd(5000, BUTTON_JUMP), context(), false);
        assert_eq!(out, SprintResult::Ended);
        assert_eq!(ps.sprint_state.sprint_delay, 1);
        assert_eq!(ps.sprint_state.last_sprint_end, 5000);
    }

    #[test]
    fn the_knife_ends_it_from_its_wind_up() {
        let mut ps = sprinting();
        ps.weapon_state_t6 = t6::MELEE_INIT;
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Ended);
    }

    #[test]
    fn night_vision_ends_it_going_on_or_coming_off() {
        for state in [t6::NIGHTVISION_WEAR, t6::NIGHTVISION_REMOVE] {
            let mut ps = sprinting();
            ps.weapon_state_t6 = state;
            assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Ended, "{state}");
        }
    }

    #[test]
    fn night_vision_does_not_stop_one_starting() {
        let mut ps = PlayerState::ZERO;
        ps.weapon_state_t6 = t6::NIGHTVISION_WEAR;
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Started);
    }

    #[test]
    fn the_riot_shield_ends_and_blocks_it_but_bo2s_detonating_does_neither() {
        let mut ps = sprinting();
        ps.weapon_state_t6 = t6::RIOTSHIELD_PREPARE;
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Ended);
        let mut ps = PlayerState::ZERO;
        ps.weapon_state_t6 = t6::RIOTSHIELD_END;
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);

        let mut ps = sprinting();
        ps.weapon_state_t6 = t6::DETONATING;
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Unchanged);
    }

    #[test]
    fn a_fresh_landing_may_start_one_through_a_busy_gun() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.weapon_state_t6 = t6::OFFHAND_INIT;
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Started);
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.pm_time = 100;
        ps.weapon_state_t6 = t6::OFFHAND_INIT;
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);
    }

    #[test]
    fn crouched_needs_room_to_stand() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::CROUCH;
        let low = SprintContext {
            stand_up_clear: false,
            ..context()
        };
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), low, false), SprintResult::Unchanged);
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Started);
    }

    #[test]
    fn ending_with_the_key_held_wants_it_let_go() {
        let mut ps = sprinting();
        let out = update(&mut ps, &cmd(1050, BUTTON_SPRINT | BUTTON_RELOAD), context(), false);
        assert_eq!(out, SprintResult::Ended);
        assert_eq!(ps.sprint_state.sprint_button_up_required, 1);
        assert_eq!(update(&mut ps, &cmd(1100, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);
        assert_eq!(update(&mut ps, &cmd(1150, 0), context(), false), SprintResult::Unchanged);
        assert_eq!(ps.sprint_state.sprint_button_up_required, 0);
        assert_eq!(update(&mut ps, &cmd(2300, BUTTON_SPRINT), context(), false), SprintResult::Started);
    }

    #[test]
    fn waist_deep_water_ends_a_running_sprint() {
        let mut ps = sprinting();
        ps.waterlevel = 1;
        assert_eq!(update(&mut ps, &cmd(1050, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);
        ps.waterlevel = 2;
        assert_eq!(update(&mut ps, &cmd(1100, BUTTON_SPRINT), context(), false), SprintResult::Ended);
    }

    #[test]
    fn a_lowered_gun_ends_a_sprint_and_stops_one_starting() {
        let mut ps = sprinting();
        ps.weapon_state_t6 = t6::LOWREADY_RAISE;
        assert_eq!(update(&mut ps, &cmd(1050, 0), context(), false), SprintResult::Ended);
        let mut ps = PlayerState::ZERO;
        ps.weapon_state_t6 = t6::LOWREADY_DROP;
        assert_eq!(update(&mut ps, &cmd(1000, BUTTON_SPRINT), context(), false), SprintResult::Unchanged);
        ps.weapon_state_t6 = t6::READY;
        assert_eq!(update(&mut ps, &cmd(1050, BUTTON_SPRINT), context(), false), SprintResult::Started);
    }

    #[test]
    fn dead_ends_it() {
        let mut ps = sprinting();
        ps.pm_type = 10;
        assert_eq!(update(&mut ps, &cmd(1050, BUTTON_SPRINT), context(), false), SprintResult::Ended);
        assert_eq!(ps.sprint_state.sprint_button_up_required, 1);
    }
}
