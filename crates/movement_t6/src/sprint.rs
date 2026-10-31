//! Black Ops II's PM_UpdateSprint and its two key rules
//! (PM_SprintEndingButtons, PM_SprintStartInterferingButtons), in our
//! button, flag and gun-state numbers.
//!
//! Where BO2 differs from the shared (MW2) sprint:
//! - The aim key itself neither ends nor blocks a sprint; only aiming does
//!   (the ADS flag). With the sprint key held the aim key is ignored, so the
//!   sprint runs on.
//! - Pressing sprint again mid-sprint does nothing (no press-to-stop).
//! - Aiming ends a sprint without the sprint key needing a fresh press: let go of
//!   aim with sprint still held and it goes on (bo2zm).
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
    INTENT_SCOPE_BIT, PERK_UNLIMITEDSPRINT, DashArgs, DashVerdict, end_sprint,
};
use crate::bridge::sprint_forward_below_minimum;
use crate::bridge::sprint_time_remaining;
use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};

use crate::ground::PMF_SLIDING;

const BUTTON_ATTACK: u32 = 0x1;
const INTENT_DASH_BIT: u32 = 0x2;
const BUTTON_MELEE: u32 = 0x4;
const BUTTON_RELOAD: u32 = 0x10;
const BUTTON_USE_RELOAD: u32 = 0x20;
const BUTTON_PRONE: u32 = 0x100;
const BUTTON_CROUCH: u32 = 0x200;
const INTENT_LEAP_BIT: u32 = 0x400;
const BUTTON_FRAG: u32 = 0x4000;
const BUTTON_SMOKE: u32 = 0x8000;

/// Keys that both end a sprint and stop one starting: jump, reload, the
/// knife, fire, use-reload and both grenades.
const BUSY_KEYS: u32 = INTENT_LEAP_BIT
    | BUTTON_RELOAD
    | BUTTON_MELEE
    | BUTTON_ATTACK
    | BUTTON_USE_RELOAD
    | BUTTON_FRAG
    | BUTTON_SMOKE;

/// States that end a sprint: aiming, a ladder, shell shock.
const ENDING_FLAGS: u32 = movement_state_bits::ADS_INTENT | movement_state_bits::LADDER | movement_state_bits::SHELLSHOCKED;

/// States that stop one starting: those plus a ledge climb.
const BLOCKING_FLAGS: u32 = ENDING_FLAGS | movement_state_bits::MANTLE;

/// Jumping out of a sprint moves its end this much later, so the next one is
/// shorter.
const JUMP_OUT_PENALTY_MS: i32 = 800;

/// PM_SprintEndingButtons: does this frame end a running sprint?
pub(crate) fn ending_buttons(
    avatar: &AvatarState,
    ucmd: &InputCommand,
    thrust_floor: i32,
    gun_forbids_lying: bool,
) -> bool {
    if avatar.movement_state_bits & ENDING_FLAGS != 0
        || avatar.waterlevel > 1
        || sprint_forward_below_minimum(ucmd.thrust_ahead, thrust_floor)
        || ucmd.buttons & BUSY_KEYS != 0
    {
        return true;
    }
    if ucmd.buttons & BUTTON_PRONE != 0 && !gun_forbids_lying {
        return true;
    }
    if ucmd.buttons & BUTTON_CROUCH != 0 || avatar.tilt_sideways != 0.0 {
        return true;
    }
    crate::gun::knife_offhand_or_shield(avatar)
        || crate::gun::night_vision(avatar)
        || avatar.movement_state_bits & (PMF_SLIDING | movement_state_bits::SPRINT_BLOCKED) != 0
}

/// PM_SprintStartInterferingButtons: is something stopping a sprint from
/// starting? Waist-deep water (level 2) and deeper does.
pub(crate) fn start_interfering_buttons(avatar: &AvatarState, ucmd: &InputCommand, thrust_floor: i32) -> bool {
    if avatar.movement_state_bits & movement_state_bits::LADDER != 0
        || avatar.waterlevel > 1
        || sprint_forward_below_minimum(ucmd.thrust_ahead, thrust_floor)
        || ucmd.buttons & BUSY_KEYS != 0
    {
        return true;
    }
    if avatar.tilt_sideways != 0.0 || avatar.movement_state_bits & BLOCKING_FLAGS != 0 {
        return true;
    }
    // Just landed from a jump, with no landing slowdown left: free to go.
    if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer == 0 {
        return false;
    }
    crate::gun::knife_offhand_or_shield(avatar) || avatar.movement_state_bits & PMF_SLIDING != 0
}

/// PM_CanCrouch and PM_CanStand: crouched or lying down, the player must have
/// room to stand to start a sprint (`rise_space_free`).
fn has_room(avatar: &AvatarState, rise_space_free: bool) -> bool {
    avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT) == 0 || rise_space_free
}

/// PM_UpdateSprint.
pub(super) fn update(
    avatar: &mut AvatarState,
    ucmd: &InputCommand,
    frame_ctx: DashArgs,
    gun_forbids_lying: bool,
) -> DashVerdict {
    let held = ucmd.buttons & INTENT_DASH_BIT != 0;
    if avatar.sprint_state.sprint_needs_release != 0 && !held {
        avatar.sprint_state.sprint_needs_release = 0;
    }
    let dashing = avatar.movement_state_bits & movement_state_bits::SPRINTING != 0;

    // Not walking normally, a lowered gun, or a gun that cannot sprint: any
    // sprint stops.
    if (avatar.movement_mode != 0 && avatar.movement_mode != 1)
        || crate::gun::low_ready(avatar)
        || frame_ctx.gun_dash_cap_secs <= 0
    {
        if !dashing {
            return DashVerdict::Unchanged;
        }
        end_sprint(avatar, ucmd);
        return DashVerdict::Ended;
    }

    if dashing {
        let endless = avatar.has_perk_bit(PERK_UNLIMITEDSPRINT) || frame_ctx.endless_dash;
        if !endless
            && avatar.sprint_state.sprint_launch_cap <= ucmd.authority_clock_ms.wrapping_sub(avatar.sprint_state.sprint_begin_ms)
        {
            end_sprint(avatar, ucmd);
            avatar.sprint_state.sprint_wait = 1;
            return DashVerdict::Ended;
        }
        if !ending_buttons(avatar, ucmd, frame_ctx.dash_thrust_floor, gun_forbids_lying) {
            return DashVerdict::Unchanged;
        }
        // bo2zm: aiming stops a sprint without needing sprint pressed again:
        // let go of aim with sprint still held and it goes on.
        let aiming = ucmd.buttons & INTENT_SCOPE_BIT != 0 || avatar.movement_state_bits & movement_state_bits::ADS_INTENT != 0;
        end_sprint(avatar, ucmd);
        if aiming {
            avatar.sprint_state.sprint_needs_release = 0;
        }
        if ucmd.buttons & INTENT_LEAP_BIT != 0 {
            avatar.sprint_state.sprint_stop_ms = avatar.sprint_state.sprint_stop_ms.wrapping_add(JUMP_OUT_PENALTY_MS);
        }
        return DashVerdict::Ended;
    }

    if avatar.sprint_state.sprint_wait != 0 {
        let since_end = ucmd.authority_clock_ms.wrapping_sub(avatar.sprint_state.sprint_stop_ms) as f32;
        if since_end < frame_ctx.dash_warmup_secs * 1000.0 {
            return DashVerdict::Unchanged;
        }
    }
    if !held
        || avatar.movement_state_bits & movement_state_bits::SPRINT_BLOCKED != 0
        || avatar.sprint_state.sprint_needs_release != 0
        || start_interfering_buttons(avatar, ucmd, frame_ctx.dash_thrust_floor)
        || !has_room(avatar, frame_ctx.rise_space_free)
    {
        return DashVerdict::Unchanged;
    }
    let left = sprint_time_remaining(avatar, ucmd.authority_clock_ms, frame_ctx);
    if left as f32 <= frame_ctx.dash_least_secs * 1000.0 {
        return DashVerdict::Unchanged;
    }
    avatar.sprint_state.sprint_launch_cap = left;
    avatar.sprint_state.sprint_begin_ms = ucmd.authority_clock_ms;
    avatar.movement_state_bits |= movement_state_bits::SPRINTING;
    DashVerdict::Started
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gun::t6;

    const INTENT_SCOPE_BIT: u32 = 0x800;

    fn frame_ctx() -> DashArgs {
        DashArgs {
            gun_dash_cap_secs: 4000,
            endless_dash: false,
            dash_least_secs: 1.0,
            dash_warmup_secs: 0.0,
            dash_thrust_floor: 105,
            rise_space_free: true,
            dash_refill_hold_secs: 0.0,
        }
    }

    fn ucmd(time: i32, buttons: u32) -> InputCommand {
        InputCommand {
            authority_clock_ms: time,
            buttons,
            thrust_ahead: 127,
            ..InputCommand::default()
        }
    }

    fn sprinting() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        let started = update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false);
        assert_eq!(started, DashVerdict::Started);
        avatar
    }

    #[test]
    fn the_aim_key_alone_does_not_end_a_sprint() {
        let mut avatar = sprinting();
        let frame = ucmd(1050, INTENT_DASH_BIT | INTENT_SCOPE_BIT);
        assert_eq!(update(&mut avatar, &frame, frame_ctx(), false), DashVerdict::Unchanged);
        // Aiming itself (the flag) does.
        avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        assert_eq!(update(&mut avatar, &frame, frame_ctx(), false), DashVerdict::Ended);
    }

    #[test]
    fn letting_go_of_aim_with_sprint_held_sprints_again() {
        let mut avatar = sprinting();
        avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        let held = ucmd(1050, INTENT_DASH_BIT);
        assert_eq!(update(&mut avatar, &held, frame_ctx(), false), DashVerdict::Ended);
        assert_eq!(avatar.sprint_state.sprint_needs_release, 0);
        avatar.movement_state_bits &= !movement_state_bits::ADS_INTENT;
        assert_eq!(update(&mut avatar, &ucmd(1100, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn the_aim_key_does_not_stop_a_sprint_starting() {
        let mut avatar = AvatarState::ZERO;
        let frame = ucmd(1000, INTENT_DASH_BIT | INTENT_SCOPE_BIT);
        assert_eq!(update(&mut avatar, &frame, frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn pressing_sprint_again_keeps_sprinting() {
        let mut avatar = sprinting();
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Unchanged);
        assert_eq!(update(&mut avatar, &ucmd(1100, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::SPRINTING, 0);
    }

    #[test]
    fn prone_ends_it_unless_the_gun_blocks_prone() {
        let mut avatar = sprinting();
        assert_eq!(update(&mut avatar, &ucmd(1050, BUTTON_PRONE), frame_ctx(), true), DashVerdict::Unchanged);
        assert_eq!(update(&mut avatar, &ucmd(1100, BUTTON_PRONE), frame_ctx(), false), DashVerdict::Ended);
    }

    #[test]
    fn a_script_block_ends_a_running_sprint() {
        let mut avatar = sprinting();
        avatar.movement_state_bits |= movement_state_bits::SPRINT_BLOCKED;
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Ended);
    }

    #[test]
    fn slide_ground_ends_a_sprint_and_stops_one_starting() {
        let mut avatar = sprinting();
        avatar.movement_state_bits |= PMF_SLIDING;
        assert_eq!(update(&mut avatar, &ucmd(1050, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Ended);
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = PMF_SLIDING;
        assert_ne!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn jumping_out_moves_the_end_800_ms() {
        let mut avatar = sprinting();
        assert_eq!(update(&mut avatar, &ucmd(1500, INTENT_LEAP_BIT), frame_ctx(), false), DashVerdict::Ended);
        assert_eq!(avatar.sprint_state.sprint_stop_ms, 2300);
        assert_eq!(avatar.sprint_state.sprint_wait, 0);
    }

    #[test]
    fn running_out_sets_the_delay_and_no_jump_penalty() {
        let mut avatar = sprinting();
        let out = update(&mut avatar, &ucmd(5000, INTENT_LEAP_BIT), frame_ctx(), false);
        assert_eq!(out, DashVerdict::Ended);
        assert_eq!(avatar.sprint_state.sprint_wait, 1);
        assert_eq!(avatar.sprint_state.sprint_stop_ms, 5000);
    }

    #[test]
    fn the_knife_ends_it_from_its_wind_up() {
        let mut avatar = sprinting();
        avatar.weapon_state_t6 = t6::MELEE_INIT;
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Ended);
    }

    #[test]
    fn night_vision_ends_it_going_on_or_coming_off() {
        for state in [t6::NIGHTVISION_WEAR, t6::NIGHTVISION_REMOVE] {
            let mut avatar = sprinting();
            avatar.weapon_state_t6 = state;
            assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Ended, "{state}");
        }
    }

    #[test]
    fn night_vision_does_not_stop_one_starting() {
        let mut avatar = AvatarState::ZERO;
        avatar.weapon_state_t6 = t6::NIGHTVISION_WEAR;
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn the_riot_shield_ends_and_blocks_it_but_bo2s_detonating_does_neither() {
        let mut avatar = sprinting();
        avatar.weapon_state_t6 = t6::RIOTSHIELD_PREPARE;
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Ended);
        let mut avatar = AvatarState::ZERO;
        avatar.weapon_state_t6 = t6::RIOTSHIELD_END;
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);

        let mut avatar = sprinting();
        avatar.weapon_state_t6 = t6::DETONATING;
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Unchanged);
    }

    #[test]
    fn a_fresh_landing_may_start_one_through_a_busy_gun() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.weapon_state_t6 = t6::OFFHAND_INIT;
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.move_timer = 100;
        avatar.weapon_state_t6 = t6::OFFHAND_INIT;
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);
    }

    #[test]
    fn crouched_needs_room_to_stand() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        let low = DashArgs {
            rise_space_free: false,
            ..frame_ctx()
        };
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), low, false), DashVerdict::Unchanged);
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn ending_with_the_key_held_wants_it_let_go() {
        let mut avatar = sprinting();
        let out = update(&mut avatar, &ucmd(1050, INTENT_DASH_BIT | BUTTON_RELOAD), frame_ctx(), false);
        assert_eq!(out, DashVerdict::Ended);
        assert_eq!(avatar.sprint_state.sprint_needs_release, 1);
        assert_eq!(update(&mut avatar, &ucmd(1100, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);
        assert_eq!(update(&mut avatar, &ucmd(1150, 0), frame_ctx(), false), DashVerdict::Unchanged);
        assert_eq!(avatar.sprint_state.sprint_needs_release, 0);
        assert_eq!(update(&mut avatar, &ucmd(2300, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn waist_deep_water_ends_a_running_sprint() {
        let mut avatar = sprinting();
        avatar.waterlevel = 1;
        assert_eq!(update(&mut avatar, &ucmd(1050, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);
        avatar.waterlevel = 2;
        assert_eq!(update(&mut avatar, &ucmd(1100, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Ended);
    }

    #[test]
    fn a_lowered_gun_ends_a_sprint_and_stops_one_starting() {
        let mut avatar = sprinting();
        avatar.weapon_state_t6 = t6::LOWREADY_RAISE;
        assert_eq!(update(&mut avatar, &ucmd(1050, 0), frame_ctx(), false), DashVerdict::Ended);
        let mut avatar = AvatarState::ZERO;
        avatar.weapon_state_t6 = t6::LOWREADY_DROP;
        assert_eq!(update(&mut avatar, &ucmd(1000, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Unchanged);
        avatar.weapon_state_t6 = t6::READY;
        assert_eq!(update(&mut avatar, &ucmd(1050, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Started);
    }

    #[test]
    fn dead_ends_it() {
        let mut avatar = sprinting();
        avatar.movement_mode = 10;
        assert_eq!(update(&mut avatar, &ucmd(1050, INTENT_DASH_BIT), frame_ctx(), false), DashVerdict::Ended);
        assert_eq!(avatar.sprint_state.sprint_needs_release, 1);
    }
}
