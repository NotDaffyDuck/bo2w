//! How a Black Ops II player handles: the movement rules the scripts set, and the numbers the
//! movement code is handed each command.

use bevy_ecs::prelude::World;
use movement_t6::bridge::{
    AdsFracContext, AdsIntentContext, AirMoveContext, Bo2Feel, CmdScaleWalkContext, JumpLaunchContext,
    MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT, MeleeChargeWeaponDelays, MoveBounds, PRONE_LERP_MS,
    PmoveSingleContext, SprintContext, ViewAngleClamp, WalkMoveContext, get_max_sprint_time,
};

/// What the weapon in hand and the shellshock say about how the player may move and aim.
pub(super) struct HandlingFacts {
    /// Move speed scale, ADS move speed scale and sprint duration scale of the weapon.
    pub weapon_scales: (f32, f32, f32),
    pub ads_allowed: bool,
    pub aim_down_sight: bool,
    pub ads_in_rate: f32,
    pub ads_out_rate: f32,
    pub rechamber_while_ads: bool,
    /// Whether a reload lowers the sights, the position-reload transition time, and whether
    /// the reload is segmented.
    pub ads_reload: (bool, i32, bool),
    pub ads_fire_only: bool,
    pub melee_delay_ms: i32,
    pub melee_charge_delay_ms: i32,
    pub overlay_reticle: i32,
    pub shellshock_movement_scale: f32,
}

// Constants of BO2 multiplayer's movement that no dvar changes.
const PLAYER_SPRINT_TIME: f32 = 4.0;
const MOVE_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const MOVE_MAXS: [f32; 3] = [15.0, 15.0, 70.0];
const MOVE_TRACE_MASK: u32 = 0x0281_0011;
const PITCH_LIMIT: f32 = 85.0;

/// Everything the movement code needs for one command: BO2's walk, air, sprint, aim and melee
/// numbers, put together from the rules in force (`feel`), the weapon in hand and the buttons
/// the player held on the command before.
pub(super) fn pmove_context(old_buttons: u32, handling: &HandlingFacts, feel: Bo2Feel) -> PmoveSingleContext {
    let air = AirMoveContext {
        player_spectate_speed_scale: 1.0,
        shellshock_gravity_scale: 1.0,
        shellshock_gravity_bias: 0.0,
    };
    let cmd_scale = CmdScaleWalkContext {
        player_back_speed_scale: 0.7,
        player_strafe_speed_scale: 0.8,
        player_sprint_speed_scale: 1.5,
        player_last_stand_crawl_speed_scale: 0.15,
        weapon_move_speed_scale: handling.weapon_scales.0,
        weapon_ads_move_speed_scale: handling.weapon_scales.1,
        shellshock_movement_scale: handling.shellshock_movement_scale,
        // Going prone takes longer in BO2 than it did before.
        prone_lerp_ms: PRONE_LERP_MS,
    };
    let walk = WalkMoveContext {
        cmd_scale,
        weapon_move_scale: 1.0,
        old_buttons,
        jump: JumpLaunchContext {
            jump_height: feel.jump_height,
            dive: false,
            crouch_jump_scale: 1.0,
            jump_ladder_push_vel: 128.0,
        },
        air,
        feel,
        lunge: None,
    };
    let sprint = SprintContext {
        weapon_max_sprint_time: get_max_sprint_time(handling.weapon_scales.2, PLAYER_SPRINT_TIME),
        sprint_forever: false,
        min_sprint_time_seconds: 1.0,
        sprint_delay_seconds: 0.0,
        sprint_forward_minimum: 105,
        stand_up_clear: true,
        sprint_recharge_pause_seconds: 0.0,
    };
    let ads_intent = AdsIntentContext {
        ads_allowed: handling.ads_allowed,
        weapon_def_scope: handling.overlay_reticle != 0,
        sprint_hold_ads: false,
    };
    let (reload_lowers_ads, position_reload_trans_ms, segmented_reload) = handling.ads_reload;
    let ads_frac = AdsFracContext {
        aim_down_sight: handling.aim_down_sight,
        ads_in_rate: handling.ads_in_rate,
        ads_out_rate: handling.ads_out_rate,
        rechamber_while_ads: handling.rechamber_while_ads,
        ads_fire_only: handling.ads_fire_only,
        reload_lowers_ads,
        position_reload_trans_ms,
        segmented_reload,
        weapon_brain_owns: false,
    };
    PmoveSingleContext {
        walk,
        air,
        bounds: MoveBounds { mins: MOVE_MINS, maxs: MOVE_MAXS, tracemask: MOVE_TRACE_MASK },
        view_angles: ViewAngleClamp { pitch_up: PITCH_LIMIT, pitch_down: PITCH_LIMIT, unclamped_pitch_bit: false },
        sprint,
        ads_intent,
        ads_frac,
        melee_charge: MeleeChargeWeaponDelays {
            melee_delay_ms: handling.melee_delay_ms,
            melee_charge_delay_ms: handling.melee_charge_delay_ms,
        },
        player_melee_range: MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        old_buttons,
        old_move: [0, 0],
        weapon_blocks_prone: false,
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
    let Some(t6) = ecs.get_resource::<crate::t6::T6Runtime>() else {
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

fn bo2_feel_from(t6: &crate::t6::T6Runtime) -> Bo2Feel {
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
        // BO2 sprints forward only (forwardmove above
        // player_sprintForwardMinimum); 10-10: off by default, the
        // Controls menu's "Omni Movement" turns it on (read at game start).
        omni,
        // Low climbs keep the gun in hand (defaults until real BO2 is read).
        mantle_weapon_height: number("mantle_weapon_height", 39.0),
        mantle_weapon_anim_height: number("mantle_weapon_anim_height", 32.0),
        // jump_height (BO2 39; the mod menu's Super Jump sets it).
        jump_height: number("jump_height", 39.0),
    }
}

