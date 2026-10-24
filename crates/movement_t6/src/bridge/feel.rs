//! bo2zm: Black Ops II movement rules that differ from the legacy code this
//! crate started from.

use playerstate_t6::game::{PlayerState, pm_flags};

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

    /// The fall perk's bit in perks[0] (specialty_fallheight): no fall
    /// damage, in both modes. 0 = none.
    pub fall_damage_perk: u32,

    /// PhD Flopper's bit in perks[0] (specialty_flakjacket): in Zombies any
    /// fall damage becomes 1, so the scripts can set off its blast. 0 = none.
    pub flak_jacket_perk: u32,

    /// Quickdraw's bit in perks[0] (specialty_fastads). 0 = none.
    pub fast_ads_perk: u32,

    /// perk_weapAdsMultiplier: with Quickdraw, aiming in and out takes this
    /// share of the gun's own time (0.5 until read from real BO2); sniper rifles
    /// are left alone.
    pub fast_ads_multiplier: f32,

    /// Stamin-Up's bit in perks[0] (specialty_longersprint). 0 = none.
    pub longer_sprint_perk: u32,

    /// perk_sprintMultiplier: Stamin-Up multiplies the sprint time by this
    /// (BO2 default 2, still capped at 16383 ms).
    pub sprint_multiplier: f32,

    /// Move-faster's bit in perks[0] (specialty_movefaster). 0 = none.
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
    pub jump_height: f32,
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
        jump_height: 39.0,
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

/// Fall perk (specialty_fallheight).
pub const PERK_FALLHEIGHT: u32 = 1 << 29;

/// Flak Jacket, Zombies' PhD Flopper (specialty_flakjacket).
pub const PERK_FLAKJACKET: u32 = 1 << 31;

/// Quickdraw (specialty_fastads).
pub const PERK_FASTADS: u32 = 1 << 30;

/// Stamin-Up, multiplayer's Extreme Conditioning (specialty_longersprint).
pub const PERK_LONGERSPRINT: u32 = 1 << 26;

/// Stalker (specialty_stalker): aiming does not slow the walk.
pub const PERK_STALKER: u32 = 1 << 11;

/// BO2 feel M10b: the crouch-to-prone stage of the eye-height change takes
/// 600 ms (MW2 400), and the stance speed blends over the same time. The
/// curves themselves are the same.
pub const PRONE_LERP_MS: i32 = 600;

/// Move-faster (specialty_movefaster): multiplayer's Lightweight speed and
/// the Turned zombies' perk.
pub const PERK_MOVEFASTER: u32 = 1 << 9;

/// BO2 feel M8b: every frame the walk speed starts again from g_speed; with
/// move-faster it is multiplied by perk_speedMultiplier and cut to a whole
/// number (190 x 1.07 = 203).
#[must_use]
pub fn player_speed(ps: &PlayerState, feel: Bo2Feel) -> i32 {
    let mut speed = feel.g_speed;
    if feel.move_faster_perk != 0 && ps.perks[0] & feel.move_faster_perk != 0 {
        speed *= feel.speed_multiplier;
    }
    speed as i32
}

/// In Zombies, Stamin-Up raises the gun's move speed to at least this while
/// sprinting (BO2's own number).
pub const STAMIN_UP_SPRINT_MOVE_SPEED_SCALE: f32 = 1.1;

fn stamin_up(ps: &PlayerState, feel: Bo2Feel) -> bool {
    feel.longer_sprint_perk != 0 && (ps.perks[0] & feel.longer_sprint_perk) != 0
}

/// BO2 feel M13: in Zombies, Stamin-Up sprints with the gun's move speed at
/// least 1.1 (most guns are 1, so 10% faster). A gun with no move speed of
/// its own keeps its aiming speed.
#[must_use]
pub fn stamin_up_cmd_scale(
    ps: &PlayerState,
    scale: crate::bridge::CmdScaleWalkContext,
    feel: Bo2Feel,
) -> crate::bridge::CmdScaleWalkContext {
    if feel.zombies
        && stamin_up(ps, feel)
        && ps.pm_flags & pm_flags::SPRINTING != 0
        && scale.weapon_move_speed_scale > 0.0
    {
        crate::bridge::CmdScaleWalkContext {
            weapon_move_speed_scale: scale
                .weapon_move_speed_scale
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
    ps: &PlayerState,
    ads_in_rate: f32,
    ads_out_rate: f32,
    sniper: bool,
    feel: Bo2Feel,
) -> (f32, f32) {
    if sniper
        || feel.fast_ads_perk == 0
        || (ps.perks[0] & feel.fast_ads_perk) == 0
        || feel.fast_ads_multiplier <= 0.0
    {
        return (ads_in_rate, ads_out_rate);
    }
    (
        ads_in_rate / feel.fast_ads_multiplier,
        ads_out_rate / feel.fast_ads_multiplier,
    )
}

impl Default for Bo2Feel {
    fn default() -> Self {
        Self::BO2_MP
    }
}

/// Head bob and footstep rhythm per stance: stand, prone, crouch, then the
/// same three running backwards. Columns: running, walking.
const BOB_FACTOR_TABLE: [[f32; 2]; 6] = [
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
const PLAYER_SPRINT_CAMERA_BOB: f32 = 0.5;


/// Moves the head bob and footstep rhythm one frame on the ground. The caller
/// has already ruled out standing still, no input, the air and turrets.
pub(crate) fn bob_cycle(
    ps: &mut PlayerState,
    msec: i32,
    forwardmove: i8,
    rightmove: i8,
    xyspeed: f32,
    server_time: i32,
    scales: crate::bridge::CmdScaleWalkContext,
    feel: Bo2Feel,
) {
    let scales = stamin_up_cmd_scale(ps, scales, feel);
    // Downed (22) uses the prone row.
    let stance = match ps.view_height_target {
        22 | 11 => 1,
        40 => 2,
        _ => 0,
    };
    let backward = (ps.pm_flags & pm_flags::BACKWARDS_RUN) != 0;
    let row = stance + 3 * usize::from(backward);
    let mut walking = (ps.pm_flags & pm_flags::WALKING) != 0 || ps.leanf != 0.0;
    let mut sprinting = (ps.pm_flags & pm_flags::SPRINTING) != 0;
    if stance == 0 {
        walking |= xyspeed <= PLAYER_RUNBK_THRESHHOLD;
        sprinting &= xyspeed >= PLAYER_SPRINT_THRESHHOLD;
    }
    let max_speed = crate::bridge::get_bob_max_speed(
        ps,
        forwardmove,
        rightmove,
        walking,
        sprinting,
        server_time,
        scales,
    ) * ps.move_speed_scale_multiplier;
    if max_speed <= 0.0 {
        return;
    }
    let factor = if row == 0 && sprinting {
        PLAYER_SPRINT_CAMERA_BOB
    } else {
        BOB_FACTOR_TABLE[row][usize::from(walking)]
    };
    let bobmove = xyspeed / max_speed * factor;
    // The gun in hand speeds up or slows the rhythm while the sprint flag
    // is on (crouched: its own scale), whatever the speed.
    let sprint_flag = (ps.pm_flags & pm_flags::SPRINTING) != 0;
    let scale = if sprint_flag && ps.view_height_target == 40 {
        feel.ducked_sprint_cycle_scale
    } else if sprint_flag {
        feel.sprint_cycle_scale
    } else {
        1.0
    };
    ps.bob_cycle = step_cycle(ps.bob_cycle, msec, bobmove, scale);
}

/// Rhythm speed through a dive (dtpBobMove).
const DTP_BOB_MOVE: f32 = 0.24;

/// Through a whole dive, air and slide, the rhythm runs at its own pace
/// (times the gun's dive scale) and the normal step rhythm is skipped.
/// Returns true while diving.
pub(crate) fn dive_bob_cycle(ps: &mut PlayerState, msec: i32, feel: Bo2Feel) -> bool {
    if !crate::bridge::dive::active(ps) {
        return false;
    }
    ps.bob_cycle = step_cycle(ps.bob_cycle, msec, DTP_BOB_MOVE, feel.dtp_cycle_scale);
    true
}

/// The rhythm counter steps by whole units, rounding down.
fn step_cycle(old: i32, msec: i32, bobmove: f32, scale: f32) -> i32 {
    let old = old as u8;
    i32::from((f32::from(old) + msec as f32 * bobmove * scale) as i32 as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::UserCmd;

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
        let mut ps = PlayerState::ZERO;
        ps.pm_time = 1750;
        assert_eq!(crate::bridge::jump::bo2_land_factor(&ps, true), 2.5);
        assert_eq!(crate::bridge::jump::bo2_land_factor(&ps, false), 1.0);
        ps.pm_time = 850;
        assert!((crate::bridge::jump::bo2_land_factor(&ps, true) - 1.75).abs() < 1e-4);
        ps.pm_time = 0;
        assert_eq!(crate::bridge::jump::bo2_land_factor(&ps, true), 1.0);

        ps.pm_flags = pm_flags::JUMPING | pm_flags::PRONE;
        crate::bridge::jump::clear_state(&mut ps);
        assert_eq!(ps.pm_flags, pm_flags::PRONE);
    }

    fn scales() -> crate::bridge::CmdScaleWalkContext {
        crate::bridge::CmdScaleWalkContext {
            player_back_speed_scale: 0.7,
            player_strafe_speed_scale: 0.8,
            player_sprint_speed_scale: 1.5,
            player_last_stand_crawl_speed_scale: 0.15,
            weapon_move_speed_scale: 1.0,
            weapon_ads_move_speed_scale: 1.0,
            shellshock_movement_scale: 1.0,
            prone_lerp_ms: 0,
        }
    }

    /// BO2's PM_CmdScale_Walk: aiming prone slows
    /// by 0.4 and uses the gun's aiming speed; sprint speed stops
    /// once the eye heads below standing height.
    #[test]
    fn bo2_walk_scale_keeps_prone_aim_slow() {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.weapon = 1;
        ps.view_height_target = crate::bridge::stance::view_height::PRONE;
        let cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        let bo2 = crate::bridge::CmdScaleWalkContext {
            weapon_ads_move_speed_scale: 0.5,
            ..scales()
        };

        ps.pm_flags = pm_flags::PRONE;
        ps.f_weapon_pos_frac = 0.0;
        let crawl = crate::bridge::cmd_scale_walk(&ps, &cmd, bo2);
        ps.f_weapon_pos_frac = 1.0;
        let aimed = crate::bridge::cmd_scale_walk(&ps, &cmd, bo2);
        assert!((aimed / crawl - 0.4 * 0.5).abs() < 1e-5, "{aimed} {crawl}");

        ps.pm_flags = pm_flags::SPRINTING;
        ps.f_weapon_pos_frac = 0.0;
        let no_sprint = crate::bridge::CmdScaleWalkContext { player_sprint_speed_scale: 1.0, ..bo2 };
        let low_sprint = crate::bridge::cmd_scale_walk(&ps, &cmd, bo2);
        assert_eq!(low_sprint, crate::bridge::cmd_scale_walk(&ps, &cmd, no_sprint), "no sprint speed low down");
        ps.view_height_target = crate::bridge::stance::view_height::STAND;
        let standing = crate::bridge::cmd_scale_walk(&ps, &cmd, bo2);
        let standing_walk = crate::bridge::cmd_scale_walk(&ps, &cmd, no_sprint);
        assert!((standing / standing_walk - 1.5).abs() < 1e-5, "{standing} {standing_walk}");
    }

    /// One 50 ms step of the head bob rhythm from 0, standing on the ground.
    fn bob_step(feel: Bo2Feel, speed: f32, forward: i8, flags: u32) -> i32 {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.view_height_target = 60;
        ps.ground_entity_num = 1022;
        ps.pm_flags = flags;
        ps.velocity = [speed, 0.0, 0.0];
        crate::bridge::footsteps_bob_cycle(&mut ps, 50, forward, 0, true, 1000, scales(), feel);
        ps.bob_cycle
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
        let back = pm_flags::BACKWARDS_RUN;
        assert_eq!(bob_step(BO2, 133.0, -127, back), 18);
    }

    #[test]
    fn script_move_speed_counts_in_bo2() {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 0.5;
        ps.view_height_target = 60;
        ps.ground_entity_num = 1022;
        ps.velocity = [95.0, 0.0, 0.0];
        crate::bridge::footsteps_bob_cycle(&mut ps, 50, 127, 0, true, 1000, scales(), BO2);
        // 95 / (190 x 0.5) * 0.335 * 50 = 16.75.
        assert_eq!(ps.bob_cycle, 16);
    }

    #[test]
    fn quickdraw_halves_aim_time_except_on_snipers() {
        let feel = Bo2Feel {
            fast_ads_perk: PERK_FASTADS,
            fast_ads_multiplier: 0.5,
            ..BO2
        };
        let mut ps = PlayerState::ZERO;
        let rates = (1.0 / 250.0, 1.0 / 200.0);
        assert_eq!(
            quickdraw_ads_rates(&ps, rates.0, rates.1, false, feel),
            rates
        );
        ps.perks[0] |= PERK_FASTADS;
        assert_eq!(
            quickdraw_ads_rates(&ps, rates.0, rates.1, false, feel),
            (1.0 / 125.0, 1.0 / 100.0)
        );
        assert_eq!(
            quickdraw_ads_rates(&ps, rates.0, rates.1, true, feel),
            rates
        );
    }

    #[test]
    fn sprint_rhythm_follows_the_gun_in_bo2() {
        let sprint = pm_flags::SPRINTING;
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
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.view_height_target = 40;
        ps.ground_entity_num = playerstate_t6::game::ENTITYNUM_NONE;
        ps.pm_flags = crate::bridge::dive::PMF_DIVE;
        ps.velocity = [300.0, 0.0, 50.0];
        crate::bridge::footsteps_bob_cycle(&mut ps, 50, 127, 0, true, 1000, scales(), BO2);
        // 0.24 x 50 = 12.
        assert_eq!(ps.bob_cycle, 12);
    }

    #[test]
    fn move_faster_walks_at_203() {
        let feel = Bo2Feel {
            move_faster_perk: PERK_MOVEFASTER,
            speed_multiplier: 1.07,
            ..BO2
        };
        let mut ps = PlayerState::ZERO;
        ps.speed = 150;
        assert_eq!(player_speed(&ps, feel), 190);
        ps.perks[0] |= PERK_MOVEFASTER;
        assert_eq!(player_speed(&ps, feel), 203);
    }
}
