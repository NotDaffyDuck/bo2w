//! How a Black Ops II player handles: the movement rules the scripts set, and the numbers the
//! movement code is handed each command.

use bevy_ecs::prelude::World;
use movement_t6::bridge::{
    ScopeBlendArgs, ScopeWishArgs, AirborneStepArgs, Bo2Feel, StrideScaleArgs, LeapKickoffArgs,
    MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT, LungeGunTimings, HullExtents, PRONE_LERP_MS,
    OneTickMotionArgs, DashArgs, LookRotationLimit, FootedStepArgs, get_max_sprint_time,
};

/// What the weapon in hand and the shellshock say about how the player may move and aim.
pub(super) struct HandlingFacts {
    /// Move speed scale, ADS move speed scale and sprint duration scale of the weapon.
    pub weapon_scales: (f32, f32, f32),
    pub sights_permitted: bool,
    pub scoped_aiming: bool,
    pub scope_raise_speed: f32,
    pub scope_lower_speed: f32,
    pub rack_bolt_when_scoped: bool,
    /// Whether a reload lowers the sights, the position-reload transition time, and whether
    /// the reload is segmented.
    pub ads_reload: (bool, i32, bool),
    pub fire_only_when_scoped: bool,
    pub strike_wait_millis: i32,
    pub lunge_windup_wait_millis: i32,
    pub hud_crosshair_layer: i32,
    pub shellshock_movement_scale: f32,
}

// Constants of BO2 multiplayer's movement that no dvar changes.
const PLAYER_SPRINT_TIME: f32 = 4.0;
/// Half the side of a standing body's square footprint.
pub(crate) const BODY_HALF_WIDTH: f32 = 15.0;
pub(crate) const STANDING_FOOTPRINT_LOW: [f32; 3] = [-BODY_HALF_WIDTH, -BODY_HALF_WIDTH, 0.0];
pub(crate) const STANDING_FOOTPRINT_HIGH: [f32; 3] = [BODY_HALF_WIDTH, BODY_HALF_WIDTH, movement_t6::bridge::UPRIGHT_HULL_TOP];
const MOVE_TRACE_MASK: u32 = crate::shot_trace::PLAYER_BODY_MASK;
const PITCH_LIMIT: f32 = 85.0;

// Fixed tuning of BO2 multiplayer's walking, jumping and sprinting.
const NEUTRAL_SCALE: f32 = 1.0;
const NO_GRAVITY_BIAS: f32 = 0.0;
const BACKWARD_SPEED: f32 = 0.7;
const STRAFE_SPEED: f32 = 0.8;
const SPRINT_SPEED: f32 = 1.5;
const LAST_STAND_CRAWL_SPEED: f32 = 0.15;
const LADDER_JUMP_PUSH: f32 = 128.0;
const SPRINT_MIN_SECONDS: f32 = 1.0;
const SPRINT_START_DELAY_SECONDS: f32 = 0.0;
const SPRINT_RECHARGE_PAUSE_SECONDS: f32 = 0.0;
const SPRINT_FORWARD_FLOOR: i32 = 105;
const JUMP_DIVES: bool = false;
const SPRINT_UNLIMITED: bool = false;
const ROOM_TO_STAND: bool = true;
const ADS_HOLDS_SPRINT: bool = false;
const WEAPON_PREVENTS_PRONE: bool = false;

/// Everything the movement code needs for one command: BO2's walk, air, sprint, aim and melee
/// numbers, put together from the rules in force (`feel`), the weapon in hand and the buttons
/// the player held on the command before.
pub(super) fn pmove_context(prev_buttons: u32, handling: &HandlingFacts, feel: Bo2Feel) -> OneTickMotionArgs {
    let (weapon_move, weapon_ads_move, weapon_sprint_time) = handling.weapon_scales;
    let air_rules = AirborneStepArgs {
        observer_pace_ratio: NEUTRAL_SCALE,
        concussed_fall_ratio: NEUTRAL_SCALE,
        concussed_fall_offset: NO_GRAVITY_BIAS,
    };
    let cmd_scale = StrideScaleArgs {
        retreat_pace_ratio: BACKWARD_SPEED,
        sidestep_pace_ratio: STRAFE_SPEED,
        dash_pace_ratio: SPRINT_SPEED,
        downed_crawl_pace_ratio: LAST_STAND_CRAWL_SPEED,
        gun_stride_ratio: weapon_move,
        scoped_carry_pace: weapon_ads_move,
        shellshock_movement_scale: handling.shellshock_movement_scale,
        // Going prone takes longer in BO2 than it did before.
        prone_lerp_ms: PRONE_LERP_MS,
    };
    let jump = LeapKickoffArgs {
        leap_height: feel.leap_height,
        dive: JUMP_DIVES,
        squat_leap_ratio: NEUTRAL_SCALE,
        rung_kickoff_speed: LADDER_JUMP_PUSH,
    };
    let walk = FootedStepArgs {
        cmd_scale,
        gun_carry_pace: NEUTRAL_SCALE,
        last_tick_buttons: prev_buttons,
        jump,
        air: air_rules,
        feel,
        lunge: None,
    };
    let sprint = DashArgs {
        gun_dash_cap_secs: get_max_sprint_time(weapon_sprint_time, PLAYER_SPRINT_TIME),
        endless_dash: SPRINT_UNLIMITED,
        dash_least_secs: SPRINT_MIN_SECONDS,
        dash_warmup_secs: SPRINT_START_DELAY_SECONDS,
        dash_thrust_floor: SPRINT_FORWARD_FLOOR,
        rise_space_free: ROOM_TO_STAND,
        dash_refill_hold_secs: SPRINT_RECHARGE_PAUSE_SECONDS,
    };
    let ads_intent = ScopeWishArgs {
        sights_permitted: handling.sights_permitted,
        scoped_weapon: handling.hud_crosshair_layer != 0,
        dash_keeps_scope: ADS_HOLDS_SPRINT,
    };
    let (reload_lowers_ads, position_reload_trans_ms, segmented) = handling.ads_reload;
    let ads_frac = ScopeBlendArgs {
        scoped_aiming: handling.scoped_aiming,
        scope_raise_speed: handling.scope_raise_speed,
        scope_lower_speed: handling.scope_lower_speed,
        rack_bolt_when_scoped: handling.rack_bolt_when_scoped,
        fire_only_when_scoped: handling.fire_only_when_scoped,
        reload_lowers_ads,
        position_reload_trans_ms,
        staged_reload: segmented,
        weapon_brain_owns: false,
    };
    let melee_charge = LungeGunTimings {
        strike_wait_millis: handling.strike_wait_millis,
        lunge_windup_wait_millis: handling.lunge_windup_wait_millis,
    };
    let bounds = HullExtents { lo_corner: STANDING_FOOTPRINT_LOW, hi_corner: STANDING_FOOTPRINT_HIGH, collide_filter: MOVE_TRACE_MASK };
    let pitch_clamp = LookRotationLimit { tilt_upper_limit: PITCH_LIMIT, tilt_lower_limit: PITCH_LIMIT, free_tilt_flag: false };
    OneTickMotionArgs {
        walk,
        air: air_rules,
        bounds,
        look_rotation: pitch_clamp,
        sprint,
        ads_intent,
        ads_frac,
        melee_charge,
        knife_reach: MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        last_tick_buttons: prev_buttons,
        old_move: [0, 0],
        gun_forbids_lying: WEAPON_PREVENTS_PRONE,
    }
}

static HOST_FEEL: std::sync::Mutex<Option<Bo2Feel>> = std::sync::Mutex::new(None);

/// bo2zm: the Black Ops II movement rules, always on in a Black Ops II
/// game. Values the scripts set win over the defaults. `predicting` = this
/// is the local player's guess, not the game itself; `sent` = the rules the
/// game sent in its last snapshot.
pub(super) fn bo2_feel(
    ecs: &World,
    predicting: bool,
    sent: Option<Bo2Feel>,
) -> Bo2Feel {
    // The Black Ops II scripts' own dvars (setdvar); none = no T6 game.
    let Some(t6) = ecs.get_resource::<crate::zombie_rt::T6Runtime>() else {
        if predicting {
            // The game's own rules first: a second PC has no scripts (it
            // guessed with MW2's), and a game hosted here before may have
            // left its rules behind.
            return sent
                .or_else(|| HOST_FEEL.lock().ok().and_then(|f| *f))
                .unwrap_or_else(|| bo2_feel_rules(|_| None, false));
        }
        if let Ok(mut f) = HOST_FEEL.lock() {
            *f = None;
        }
        return bo2_feel_rules(|_| None, false);
    };
    let feel = bo2_feel_from(t6);
    if !predicting && let Ok(mut f) = HOST_FEEL.lock() {
        *f = Some(feel);
    }
    feel
}

/// bo2zm: the Controls setting "Omni Movement" (10-10: off by
/// default, an option in the Controls menu for whoever wants it).
static OMNI_SETTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// bo2zm: his "Omni Movement" setting; the next game started here reads it.
pub fn set_omni_movement(on: bool) {
    OMNI_SETTING.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// bo2zm: the "Omni Movement" setting as a game starting now takes it.
pub(crate) fn omni_movement_setting() -> bool {
    OMNI_SETTING.load(std::sync::atomic::Ordering::Relaxed)
}

fn bo2_feel_from(t6: &crate::zombie_rt::T6Runtime) -> Bo2Feel {
    bo2_feel_rules(|name| t6.vm.dvars.get(name).map(String::as_str), t6.omni)
}

/// The Black Ops II movement rules from the scripts' dvars (`dvar`); a dvar
/// no script set takes BO2's default. With no scripts at all (`|_| None`)
/// these are BO2 multiplayer's own rules.
pub(super) fn bo2_feel_rules<'a>(dvar: impl Fn(&str) -> Option<&'a str>, omni: bool) -> Bo2Feel {
    let number = |name: &str, default: f32| {
        dvar(name)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(default)
    };
    let zombies = dvar("mapname").is_some_and(|m| m.starts_with("zm_"));
    Bo2Feel {
        sprint_strafe_speed_scale: number("player_sprintStrafeSpeedScale", 0.667),
        // BO2 turns the repeated-jump slowdown off in Zombies and on in
        // multiplayer, unless a script sets the dvar.
        jump_slowdown: number("jump_slowdownEnable", if zombies { 0.0 } else { 1.0 }) != 0.0,
        // Both modes read the same fall heights.
        fall_damage_min_height: number("bg_fallDamageMinHeight", 128.0),
        fall_damage_max_height: number("bg_fallDamageMaxHeight", 300.0),
        zombies,
        gravity: number("bg_gravity", 800.0) as i32,
        // The fall perk works in both modes (Turned's zombies get it);
        // Flak Jacket only changes falls in Zombies, where it is PhD Flopper.
        fall_damage_perk: playerstate_t6::perks::PERK_FALLHEIGHT,
        flak_jacket_perk: playerstate_t6::perks::PERK_FLAKJACKET,
        fast_ads_perk: playerstate_t6::perks::PERK_FASTADS,
        fast_ads_multiplier: number("perk_weapAdsMultiplier", 0.5),
        // Stamin-Up: sprint time x perk_sprintMultiplier, and in Zombies a
        // sprint at least 1.1 times the gun's speed.
        longer_sprint_perk: playerstate_t6::perks::PERK_LONGERSPRINT,
        sprint_multiplier: number("perk_sprintMultiplier", 2.0),
        // Move-faster: the walk speed x perk_speedMultiplier (190 -> 203).
        move_faster_perk: playerstate_t6::perks::PERK_MOVEFASTER,
        speed_multiplier: number("perk_speedMultiplier", 1.07),
        g_speed: number("g_speed", 190.0),
        // dtp: BO2's dive to prone switch, on in both modes.
        dive: number("dtp", 1.0) != 0.0,
        // Set per player from the gun in hand.
        sprint_cycle_scale: 1.0,
        ducked_sprint_cycle_scale: 1.0,
        dtp_cycle_scale: 1.0,
        // BO2 sprints forward only (thrust_ahead above
        // player_sprintForwardMinimum); 10-10: off by default, the
        // Controls menu's "Omni Movement" turns it on (read at game start).
        omni,
        // Low climbs keep the gun in hand (defaults until real BO2 is read).
        mantle_weapon_height: number("mantle_weapon_height", 39.0),
        mantle_weapon_anim_height: number("mantle_weapon_anim_height", 32.0),
        // jump_height (BO2 39; the mod menu's Super Jump sets it).
        leap_height: number("jump_height", 39.0),
    }
}

