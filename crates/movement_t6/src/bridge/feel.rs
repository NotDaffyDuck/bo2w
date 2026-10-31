//! bo2zm: Black Ops II movement rules that differ from the legacy code this
//! crate started from.

use playerstate_t6::game::{AvatarState, movement_state_bits};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bo2Feel {
    /// player_sprintStrafeSpeedScale: sideways input is scaled by this while
    /// sprinting.
    pub sprint_strafe_speed_scale: f32,

    /// jump_slowdownEnable: landing a jump slows you down (BO2: on in
    /// multiplayer, off in Zombies).
    pub jump_slowdown: bool,

    /// Falls higher than this hurt (bg_fallDamageMinHeight, 128 in both modes).
    pub fall_damage_min_height: f32,

    /// Falls this high or higher do full damage (bg_fallDamageMaxHeight, 300
    /// in both modes).
    pub fall_damage_max_height: f32,

    /// Zombies: no fall damage while this player's gravity is low, and PhD
    /// Flopper turns any fall damage into 1.
    pub zombies: bool,

    /// bg_gravity, for the Zombies low-gravity rule.
    pub gravity: i32,

    /// The fall perk's number (specialty_fallheight): no fall damage, in both
    /// modes. `PERK_UNKNOWN` = none.
    pub fall_damage_perk: u32,

    /// PhD Flopper's number (specialty_flakjacket): in Zombies any fall
    /// damage becomes 1, so the scripts can set off its blast. `PERK_UNKNOWN`
    /// = none.
    pub flak_jacket_perk: u32,

    /// Quickdraw's number (specialty_fastads). `PERK_UNKNOWN` = none.
    pub fast_ads_perk: u32,

    /// perk_weapAdsMultiplier: with Quickdraw, aiming in and out takes this
    /// share of the gun's own time (0.5 until read from real BO2); sniper rifles
    /// are left alone.
    pub fast_ads_multiplier: f32,

    /// Stamin-Up's number (specialty_longersprint). `PERK_UNKNOWN` = none.
    pub longer_sprint_perk: u32,

    /// perk_sprintMultiplier: Stamin-Up multiplies the sprint time by this
    /// (BO2 default 2, still capped at 16383 ms).
    pub sprint_multiplier: f32,

    /// Move-faster's number (specialty_movefaster). `PERK_UNKNOWN` = none.
    pub move_faster_perk: u32,

    /// perk_speedMultiplier: move-faster multiplies the walk speed by this
    /// (BO2 default 1.07: 190 -> 203).
    pub speed_multiplier: f32,

    /// g_speed: the walk speed every frame starts from (190).
    pub g_speed: f32,

    /// dtp: dive to prone is allowed (BO2 default 1, in both modes).
    pub dive: bool,

    /// The gun in hand's footstep rhythm scale while sprinting
    /// (fSprintCycleScale), while sprinting crouched
    /// (fDuckedSprintCycleScale) and through a dive (fDtpCycleScale).
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,

    /// Sprint in any direction at full speed. Not BO2 (it sprints forward
    /// only), so off unless his Controls menu's "Omni Movement" is on
    /// (10-10: off by default, an option for whoever wants it).
    pub omni: bool,

    /// mantle_weapon_height: a climb this low (by its climb table height)
    /// keeps the gun in hand; mantle_weapon_anim_height: one higher than
    /// this also plays the gun's climb camera animation. 0 lowers the gun
    /// for every climb.
    pub mantle_weapon_height: f32,
    pub mantle_weapon_anim_height: f32,

    /// jump_height (39 in both modes; the mod menu's Super Jump raises it).
    pub leap_height: f32,
}

impl Bo2Feel {
    /// Black Ops II multiplayer's rules when no script has set a dvar.
    pub const BO2_MP: Self = Self {
        sprint_strafe_speed_scale: 0.667,
        jump_slowdown: true,
        fall_damage_min_height: 128.0,
        fall_damage_max_height: 300.0,
        zombies: false,
        gravity: 800,
        fall_damage_perk: PERK_FALLHEIGHT,
        flak_jacket_perk: PERK_FLAKJACKET,
        fast_ads_perk: PERK_FASTADS,
        fast_ads_multiplier: 0.5,
        longer_sprint_perk: PERK_LONGERSPRINT,
        sprint_multiplier: 2.0,
        move_faster_perk: PERK_MOVEFASTER,
        speed_multiplier: 1.07,
        g_speed: 190.0,
        dive: true,
        sprint_cycle_scale: 1.0,
        ducked_sprint_cycle_scale: 1.0,
        dtp_cycle_scale: 1.0,
        omni: false,
        mantle_weapon_height: 39.0,
        mantle_weapon_anim_height: 32.0,
        leap_height: 39.0_f32,
    };
}

/// BO2 feel M14: a shellshock slows walking by the shock file's own number
/// (bg_shock_movement: explosion and pain 1, so not slowed; flashbang 0.8;
/// most others 0.4). `movement` is the file's switch; a host that puts the
/// number on the move-speed multiplier itself (bo2mp's `shellshock`)
/// turns it off, so the shock never slows twice.
#[must_use]
pub fn shellshock_walk_scale(movement: bool, movement_scale: f32) -> f32 {
    if movement && movement_scale > 0.0 {
        movement_scale
    } else {
        1.0
    }
}

// The perks these rules test, by BO2's perksEnum number (Stalker: aiming does
// not slow the walk; move-faster: multiplayer's Lightweight speed and the
// Turned zombies' perk; flak jacket: Zombies' PhD Flopper; longer sprint:
// Stamin-Up).
pub use playerstate_t6::perks::{
    PERK_FALLHEIGHT, PERK_FASTADS, PERK_FLAKJACKET, PERK_LONGERSPRINT, PERK_MOVEFASTER, PERK_STALKER,
    PERK_UNKNOWN,
};

/// BO2 feel M10b: the crouch-to-prone stage of the eye-height change takes
/// 600 ms (MW2 400), and the stance speed blends over the same time. The
/// curves themselves are the same.
pub const PRONE_LERP_MS: i32 = 600;

/// BO2 feel M8b: every frame the walk speed starts again from g_speed; with
/// move-faster it is multiplied by perk_speedMultiplier and cut to a whole
/// number (190 x 1.07 = 203).
#[must_use]
pub fn player_speed(avatar: &AvatarState, feel: Bo2Feel) -> i32 {
    let mut speed = feel.g_speed;
    if avatar.has_perk_bit(feel.move_faster_perk) {
        speed *= feel.speed_multiplier;
    }
    speed as i32
}

/// In Zombies, Stamin-Up raises the gun's move speed to at least this while
/// sprinting (BO2's own number).
pub const STAMIN_UP_SPRINT_MOVE_SPEED_SCALE: f32 = 1.1;

fn stamin_up(avatar: &AvatarState, feel: Bo2Feel) -> bool {
    avatar.has_perk_bit(feel.longer_sprint_perk)
}

/// BO2 feel M13: in Zombies, Stamin-Up sprints with the gun's move speed at
/// least 1.1 (most guns are 1, so 10% faster). A gun with no move speed of
/// its own keeps its aiming speed.
#[must_use]
pub fn stamin_up_cmd_scale(
    avatar: &AvatarState,
    scale: crate::bridge::StrideScaleArgs,
    feel: Bo2Feel,
) -> crate::bridge::StrideScaleArgs {
    if feel.zombies
        && stamin_up(avatar, feel)
        && avatar.movement_state_bits & movement_state_bits::SPRINTING != 0
        && scale.gun_stride_ratio > 0.0
    {
        crate::bridge::StrideScaleArgs {
            gun_stride_ratio: scale
                .gun_stride_ratio
                .max(STAMIN_UP_SPRINT_MOVE_SPEED_SCALE),
            ..scale
        }
    } else {
        scale
    }
}

/// BO2 feel G3h: Quickdraw speeds up aiming in and out, except on sniper
/// rifles. Returns the aim-in and aim-out rates (per ms).
#[must_use]
pub fn quickdraw_ads_rates(
    avatar: &AvatarState,
    scope_raise_speed: f32,
    scope_lower_speed: f32,
    sniper: bool,
    feel: Bo2Feel,
) -> (f32, f32) {
    if sniper
        || !avatar.has_perk_bit(feel.fast_ads_perk)
        || feel.fast_ads_multiplier <= 0.0
    {
        return (scope_raise_speed, scope_lower_speed);
    }
    (
        scope_raise_speed / feel.fast_ads_multiplier,
        scope_lower_speed / feel.fast_ads_multiplier,
    )
}

impl Default for Bo2Feel {
    fn default() -> Self {
        Self::BO2_MP
    }
}

/// Head bob and footstep rhythm per stance: stand, prone, crouch, then the
/// same three running backwards. Columns: running, walking.
const SWAY_WEIGHTS: [[f32; 2]; 6] = [
    [0.335, 0.28],
    [0.25, 0.24],
    [0.34, 0.29],
    [0.36, 0.3],
    [0.25, 0.24],
    [0.34, 0.29],
];

/// player_moveThreshhold: slower than this counts as standing still.
pub const PLAYER_MOVE_THRESHHOLD: f32 = 2.0;

/// player_runbkThreshhold: standing up and slower than this walks.
const PLAYER_RUNBK_THRESHHOLD: f32 = 60.0;

/// player_sprintThreshhold: standing up, sprinting only counts this fast.
const PLAYER_SPRINT_THRESHHOLD: f32 = 185.0;

/// player_sprintCameraBob.
const DASH_VIEW_SWAY: f32 = 0.5;

/// Moves the head bob and footstep rhythm one frame on the ground. The caller
/// has already ruled out standing still, no input, the air and turrets.
pub(crate) fn sway_phase(
    avatar: &mut AvatarState,
    msec: i32,
    thrust_ahead: i8,
    thrust_lateral: i8,
    ground_velocity: f32,
    authority_clock_ms: i32,
    scales: crate::bridge::StrideScaleArgs,
    feel: Bo2Feel,
) {
    let scales = stamin_up_cmd_scale(avatar, scales, feel);
    // Downed (22) uses the prone row.
    let stance = match avatar.eye_height_goal {
        22 | 11 => 1,
        40 => 2,
        _ => 0,
    };
    let backward = (avatar.movement_state_bits & movement_state_bits::BACKWARDS_RUN) != 0;
    let row = stance + 3 * usize::from(backward);
    let mut walking = (avatar.movement_state_bits & movement_state_bits::WALKING) != 0 || avatar.tilt_sideways != 0.0;
    let mut sprinting = (avatar.movement_state_bits & movement_state_bits::SPRINTING) != 0;
    if stance == 0 {
        walking |= ground_velocity <= PLAYER_RUNBK_THRESHHOLD;
        sprinting &= ground_velocity >= PLAYER_SPRINT_THRESHHOLD;
    }
    let top_velocity = crate::bridge::get_bob_max_speed(
        avatar,
        thrust_ahead,
        thrust_lateral,
        walking,
        sprinting,
        authority_clock_ms,
        scales,
    ) * avatar.run_speed_modifier;
    if top_velocity <= 0.0 {
        return;
    }
    let factor = if row == 0 && sprinting {
        DASH_VIEW_SWAY
    } else {
        SWAY_WEIGHTS[row][usize::from(walking)]
    };
    let bobmove = ground_velocity / top_velocity * factor;
    // The gun in hand speeds up or slows the rhythm while the sprint flag
    // is on (crouched: its own scale), whatever the speed.
    let sprint_flag = (avatar.movement_state_bits & movement_state_bits::SPRINTING) != 0;
    let scale = if sprint_flag && avatar.eye_height_goal == 40 {
        feel.ducked_sprint_cycle_scale
    } else if sprint_flag {
        feel.sprint_cycle_scale
    } else {
        1.0
    };
    avatar.sway_phase = step_cycle(avatar.sway_phase, msec, bobmove, scale);
}

/// Rhythm speed through a dive (dtpBobMove).
const DTP_BOB_MOVE: f32 = 0.24;

/// Through a whole dive, air and slide, the rhythm runs at its own pace
/// (times the gun's dive scale) and the normal step rhythm is skipped.
/// Returns true while diving.
pub(crate) fn dive_bob_cycle(avatar: &mut AvatarState, msec: i32, feel: Bo2Feel) -> bool {
    if !crate::bridge::dive::active(avatar) {
        return false;
    }
    avatar.sway_phase = step_cycle(avatar.sway_phase, msec, DTP_BOB_MOVE, feel.dtp_cycle_scale);
    true
}

/// The rhythm counter steps by whole units, rounding down.
fn step_cycle(old: i32, msec: i32, bobmove: f32, factor: f32) -> i32 {
    let old = old as u8;
    i32::from((f32::from(old) + msec as f32 * bobmove * factor) as i32 as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::eye_level::{LYING_FLAT, UPRIGHT};
    use playerstate_t6::game::InputCommand;

    const BO2: Bo2Feel = Bo2Feel {
        sprint_strafe_speed_scale: 0.667,
        jump_slowdown: true,
        ..Bo2Feel::BO2_MP
    };

    /// BO2's Jump_GetLandFactor: a jump taken again just after landing one
    /// launches with its energy cut by 2.5, easing back to full; none of it
    /// with the jump slowdown off. Ending a jump keeps last stand.
    #[test]
    fn bo2_jump_soon_after_landing_goes_lower() {
        let mut avatar = AvatarState::ZERO;
        avatar.move_timer = 1750;
        assert_eq!(crate::bridge::leap::bo2_land_factor(&avatar, true), 2.5);
        assert_eq!(crate::bridge::leap::bo2_land_factor(&avatar, false), 1.0);
        avatar.move_timer = 850;
        assert!((crate::bridge::leap::bo2_land_factor(&avatar, true) - 1.75).abs() < 1e-4);
        avatar.move_timer = 0;
        assert_eq!(crate::bridge::leap::bo2_land_factor(&avatar, true), 1.0);

        avatar.movement_state_bits = movement_state_bits::JUMPING | movement_state_bits::LYING_FLAT;
        crate::bridge::leap::clear_state(&mut avatar);
        assert_eq!(avatar.movement_state_bits, movement_state_bits::LYING_FLAT);
    }

    fn scales() -> crate::bridge::StrideScaleArgs {
        crate::bridge::StrideScaleArgs {
            retreat_pace_ratio: 0.7,
            sidestep_pace_ratio: 0.8,
            dash_pace_ratio: 1.5,
            downed_crawl_pace_ratio: 0.15,
            gun_stride_ratio: 1.0,
            scoped_carry_pace: 1.0,
            shellshock_movement_scale: 1.0,
            prone_lerp_ms: 0,
        }
    }

    /// BO2's PM_CmdScale_Walk: aiming prone slows
    /// by 0.4 and uses the gun's aiming speed; sprint speed stops
    /// once the eye heads below standing height.
    #[test]
    fn bo2_walk_scale_keeps_prone_aim_slow() {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.weapon = 1;
        avatar.eye_height_goal = LYING_FLAT;
        let ucmd = InputCommand {
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        let bo2 = crate::bridge::StrideScaleArgs {
            scoped_carry_pace: 0.5,
            ..scales()
        };

        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        avatar.weapon_raise_fraction = 0.0;
        let crawl = crate::bridge::cmd_scale_walk(&avatar, &ucmd, bo2);
        avatar.weapon_raise_fraction = 1.0;
        let aimed = crate::bridge::cmd_scale_walk(&avatar, &ucmd, bo2);
        assert!((aimed / crawl - 0.4 * 0.5).abs() < 1e-5, "{aimed} {crawl}");

        avatar.movement_state_bits = movement_state_bits::SPRINTING;
        avatar.weapon_raise_fraction = 0.0;
        let no_sprint = crate::bridge::StrideScaleArgs { dash_pace_ratio: 1.0, ..bo2 };
        let low_sprint = crate::bridge::cmd_scale_walk(&avatar, &ucmd, bo2);
        assert_eq!(low_sprint, crate::bridge::cmd_scale_walk(&avatar, &ucmd, no_sprint), "no sprint speed low down");
        avatar.eye_height_goal = UPRIGHT;
        let standing = crate::bridge::cmd_scale_walk(&avatar, &ucmd, bo2);
        let standing_walk = crate::bridge::cmd_scale_walk(&avatar, &ucmd, no_sprint);
        assert!((standing / standing_walk - 1.5).abs() < 1e-5, "{standing} {standing_walk}");
    }

    /// One 50 ms step of the head bob rhythm from 0, standing on the ground.
    fn bob_step(feel: Bo2Feel, speed: f32, fwd: i8, flags: u32) -> i32 {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.eye_height_goal = 60;
        avatar.standing_on = 1022;
        avatar.movement_state_bits = flags;
        avatar.velocity = [speed, 0.0, 0.0];
        crate::bridge::footsteps_bob_cycle(&mut avatar, 50, fwd, 0, true, 1000, scales(), feel);
        avatar.sway_phase
    }

    #[test]
    fn running_rhythm_rounds_down_in_bo2() {
        // 190 / 190 * 0.335 * 50 = 16.75.
        assert_eq!(bob_step(BO2, 190.0, 127, 0), 16);
    }

    #[test]
    fn slow_running_bobs_like_walking_in_bo2() {
        // Under 60: walking column 0.28 against 190 x 0.4.
        // 50 / 76 * 0.28 * 50 = 9.2.
        assert_eq!(bob_step(BO2, 50.0, 127, 0), 9);
    }

    #[test]
    fn running_backwards_uses_its_own_row_in_bo2() {
        // 133 / (190 x 0.7) * 0.36 * 50 = 18.
        let back = movement_state_bits::BACKWARDS_RUN;
        assert_eq!(bob_step(BO2, 133.0, -127, back), 18);
    }

    #[test]
    fn script_move_speed_counts_in_bo2() {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        avatar.run_speed_modifier = 0.5;
        avatar.eye_height_goal = 60;
        avatar.standing_on = 1022;
        avatar.velocity = [95.0, 0.0, 0.0];
        crate::bridge::footsteps_bob_cycle(&mut avatar, 50, 127, 0, true, 1000, scales(), BO2);
        // 95 / (190 x 0.5) * 0.335 * 50 = 16.75.
        assert_eq!(avatar.sway_phase, 16);
    }

    #[test]
    fn quickdraw_halves_aim_time_except_on_snipers() {
        let feel = Bo2Feel {
            fast_ads_perk: PERK_FASTADS,
            fast_ads_multiplier: 0.5,
            ..BO2
        };
        let mut avatar = AvatarState::ZERO;
        let rates = (1.0 / 250.0, 1.0 / 200.0);
        assert_eq!(
            quickdraw_ads_rates(&avatar, rates.0, rates.1, false, feel),
            rates
        );
        avatar.set_perk(PERK_FASTADS, true);
        assert_eq!(
            quickdraw_ads_rates(&avatar, rates.0, rates.1, false, feel),
            (1.0 / 125.0, 1.0 / 100.0)
        );
        assert_eq!(
            quickdraw_ads_rates(&avatar, rates.0, rates.1, true, feel),
            rates
        );
    }

    #[test]
    fn sprint_rhythm_follows_the_gun_in_bo2() {
        let sprint = movement_state_bits::SPRINTING;
        let galil = Bo2Feel {
            sprint_cycle_scale: 0.85,
            ..BO2
        };
        let base = bob_step(BO2, 285.0, 127, sprint);
        let scaled = bob_step(galil, 285.0, 127, sprint);
        assert!(scaled < base, "{scaled} vs {base}");
        // Not sprinting: the gun's sprint scale does nothing.
        assert_eq!(bob_step(galil, 190.0, 127, 0), bob_step(BO2, 190.0, 127, 0));
    }

    #[test]
    fn diving_runs_its_own_rhythm_in_the_air_in_bo2() {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.eye_height_goal = 40;
        avatar.standing_on = playerstate_t6::game::NO_ENTITY_SLOT;
        avatar.movement_state_bits = crate::bridge::dive::PMF_DIVE;
        avatar.velocity = [300.0, 0.0, 50.0];
        crate::bridge::footsteps_bob_cycle(&mut avatar, 50, 127, 0, true, 1000, scales(), BO2);
        // 0.24 x 50 = 12.
        assert_eq!(avatar.sway_phase, 12);
    }

    #[test]
    fn move_faster_walks_at_203() {
        let feel = Bo2Feel {
            move_faster_perk: PERK_MOVEFASTER,
            speed_multiplier: 1.07,
            ..BO2
        };
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 150;
        assert_eq!(player_speed(&avatar, feel), 190);
        avatar.set_perk(PERK_MOVEFASTER, true);
        assert_eq!(player_speed(&avatar, feel), 203);
        // Move-faster lives in word 1: the old single-word test never saw it.
        assert_eq!(avatar.perks[0], 0);
        // A feel with no move-faster perk never speeds up.
        let none = Bo2Feel { move_faster_perk: PERK_UNKNOWN, ..feel };
        assert_eq!(player_speed(&avatar, none), 190);
    }
}
