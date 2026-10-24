//! Window 9: what the gun does while the player sprints, dives to prone, slides, mantles, crawls
//! and swims, plus night vision (BO2 MP has none: its check stays a no-op).
//!
//! Movement itself (speeds, when a dive or slide starts) is the movement code's; it reports
//! here through `pm_flags`, `jump_time`, `slide_time` and `last_dtp_end` in [`crate::env`].
//! Timing spec and per-weapon numbers: `docs/gun/SPRINT.md`.

use super::{Pm, fire};
use crate::anim;
use crate::base;
use crate::cmd::button;
use crate::env::{perk, pm_type, pmf};
use crate::events::{self, AnimScript};
use crate::facts::{fire_type, offhand_slot};
use crate::state;
use fastfile_t6::layout::WeaponDef as D;

/// Per-weapon times (ms) for the movement states.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveFacts {
    pub sprint_in: i32,
    pub sprint_loop: i32,
    pub sprint_out: i32,
    pub dtp_in: i32,
    pub dtp_loop: i32,
    pub dtp_out: i32,
    pub crawl_in: i32,
    pub crawl_forward: i32,
    pub crawl_back: i32,
    pub crawl_right: i32,
    pub crawl_left: i32,
    pub crawl_out_fire: i32,
    pub crawl_out: i32,
    pub slide_in: i32,
    pub sprint_duration_scale: f32,
    pub blocks_prone: bool,
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

impl MoveFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let _ = variant;
        Self {
            sprint_in: rd_i32(def, D::sprintInTime),
            sprint_loop: rd_i32(def, D::sprintLoopTime),
            sprint_out: rd_i32(def, D::sprintOutTime),
            dtp_in: rd_i32(def, D::dtpInTime),
            dtp_loop: rd_i32(def, D::dtpLoopTime),
            dtp_out: rd_i32(def, D::dtpOutTime),
            crawl_in: rd_i32(def, D::crawlInTime),
            crawl_forward: rd_i32(def, D::crawlForwardTime),
            crawl_back: rd_i32(def, D::crawlBackTime),
            crawl_right: rd_i32(def, D::crawlRightTime),
            crawl_left: rd_i32(def, D::crawlLeftTime),
            crawl_out_fire: rd_i32(def, D::crawlOutFireTime),
            crawl_out: rd_i32(def, D::crawlOutTime),
            slide_in: rd_i32(def, D::slideInTime),
            sprint_duration_scale: f32::from_bits(rd_i32(def, D::sprintDurationScale) as u32),
            blocks_prone: def.get(D::blocksProne).is_some_and(|&b| b != 0),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------------------------

/// Firing, rechambering, continuous fire or a melee swing: no movement state may cut in.
fn attacking(s: i32) -> bool {
    matches!(
        s,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    ) || (state::MELEE_INIT..=state::MELEE_END).contains(&s)
}

fn switching(s: i32) -> bool {
    (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&s)
}

fn offhand(s: i32) -> bool {
    (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
}

fn shield(s: i32) -> bool {
    (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
}

fn diving(s: i32) -> bool {
    (state::DTP_IN..=state::DTP_OUT).contains(&s)
}

fn crawling(s: i32) -> bool {
    (state::CRAWL_IN..=state::CRAWL_LEFT).contains(&s)
}

fn crawl_or_out(s: i32) -> bool {
    (state::CRAWL_IN..=state::CRAWL_OUT).contains(&s)
}

/// The gun is under water deep enough to put away, or no gun is wanted.
fn no_gun(pm: &Pm) -> bool {
    pm.env.waterlevel > 3 || pm.cmd.weapon == 0
}

/// The clip of the hand being run is empty (unlimited ammo never is): picks the EMPTY anims.
fn clip_empty(pm: &Pm) -> bool {
    if pm.weapon_facts().core.unlimited_ammo {
        return false;
    }
    pm.ps.ammo_in_clip(pm.hand_weapon()) == 0
}

fn pick(pm: &Pm, full: i32, empty: i32) -> i32 {
    if clip_empty(pm) { empty } else { full }
}

/// Restart `a` on both hands.
fn start_both(pm: &mut Pm, a: i32) {
    base::start_anim(pm, a, false);
    base::start_anim(pm, a, true);
}

/// The anim of the hand being run.
fn current_anim(pm: &Pm) -> i32 {
    if pm.left() {
        pm.ps.weap_anim_left
    } else {
        pm.ps.weap_anim
    }
}

/// Start `a` on the right hand unless the hand being run already plays it; with `left_too`,
/// then the same for the left hand (tested again after the first write, as BO2 does).
fn continue_on(pm: &mut Pm, a: i32, left_too: bool) {
    if pm.env.pm_type >= pm_type::DEAD {
        return;
    }
    if anim::number(current_anim(pm)) != a {
        pm.ps.weap_anim = anim::start(pm.ps.weap_anim, a);
    }
    if left_too && anim::number(current_anim(pm)) != a {
        pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, a);
    }
}

/// Right-hand state, timer and delay.
fn set_right(pm: &mut Pm, st: i32, time: i32) {
    pm.ps.weaponstate = st;
    pm.ps.weapon_time = time;
    pm.ps.weapon_delay = 0;
}

/// The same state and timer on both hands (the dive moves both guns together).
fn set_both(pm: &mut Pm, st: i32, time: i32) {
    set_right(pm, st, time);
    pm.ps.weaponstate_left = st;
    pm.ps.weapon_time_left = time;
    pm.ps.weapon_delay_left = 0;
}

/// The hand being run has fired every shot its trigger pull allows.
fn shot_limit_reached(pm: &Pm) -> bool {
    let core = pm.weapon_facts().core;
    let count = pm.hand().shot_count as i64;
    match core.fire_type {
        fire_type::STACKED => i64::from(core.clip_size) <= count,
        fire_type::CHARGESHOT => i64::from(pm.ps.charge_shot_level) <= count,
        fire_type::SINGLESHOT => count > 0,
        fire_type::BURSTFIRE2 => count > 1,
        fire_type::BURSTFIRE3 => count > 2,
        fire_type::BURSTFIRE4 => count > 3,
        fire_type::BURSTFIRE5 => count > 4,
        _ => false,
    }
}

/// The trigger wants a shot from the hand being run.
fn wants_shot(pm: &mut Pm) -> bool {
    fire::should_be_firing(pm, true) && !shot_limit_reached(pm)
}

// ---------------------------------------------------------------------------------------------------------------
// Night vision (not in BO2 MP)
// ---------------------------------------------------------------------------------------------------------------

/// PM_Weapon_CheckForNightVision. BO2 toggles on a fresh press of button bit 13; BO2 MP never
/// gives night vision, and [`crate::cmd::T6Cmd`] carries no previous-command buttons to see a
/// fresh press with, so this stays a no-op.
pub fn check_for_night_vision(pm: &mut Pm) {
    let _ = pm;
}

/// PM_Weapon_FinishNightVisionWear.
pub fn finish_night_vision_wear(pm: &mut Pm) {
    finish_night_vision(pm, state::NIGHTVISION_WEAR);
}

/// PM_Weapon_FinishNightVisionRemove.
pub fn finish_night_vision_remove(pm: &mut Pm) {
    finish_night_vision(pm, state::NIGHTVISION_REMOVE);
}

fn finish_night_vision(pm: &mut Pm, st: i32) {
    if pm.ps.weaponstate == st && pm.ps.weapon_time == 0 {
        pm.ps.weaponstate = state::READY;
        base::start_anim(pm, anim::IDLE, false);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Sprint
// ---------------------------------------------------------------------------------------------------------------

/// Sprint check: the gun goes up into the sprint pose when sprint starts, and comes back down
/// (SPRINT_DROP, then READY) when it stops. No shot until the drop has run out.
pub fn check_for_sprint(pm: &mut Pm) {
    if no_gun(pm) {
        return;
    }
    let s = pm.ps.weaponstate;
    if attacking(s) || attacking(pm.ps.weaponstate_left) {
        return;
    }
    if switching(s) || offhand(s) || diving(s) || shield(s) || s == state::DETONATING {
        return;
    }
    if pm.env.pm_flags & pmf::DTP_AIR != 0 {
        return;
    }
    let sprinting = pm.env.pm_flags & pmf::SPRINT != 0;
    if sprinting && !(state::SPRINT_RAISE..=state::SPRINT_DROP).contains(&s) {
        sprint_raise(pm);
    } else if !sprinting && matches!(s, state::SPRINT_RAISE | state::SPRINT_LOOP) {
        sprint_drop(pm);
    }
}

fn sprint_raise(pm: &mut Pm) {
    let t = pm.weapon_facts().moves.sprint_in;
    set_right(pm, state::SPRINT_RAISE, t);
    let a = pick(pm, anim::SPRINT_IN, anim::SPRINT_EMPTY_IN);
    start_both(pm, a);
}

/// Sprint_State_Loop (state SPRINT_RAISE ran out): hold the sprint pose.
pub fn sprint_loop(pm: &mut Pm) {
    set_right(pm, state::SPRINT_LOOP, 0);
    let a = pick(pm, anim::SPRINT_LOOP, anim::SPRINT_EMPTY_LOOP);
    start_both(pm, a);
}

/// Coming out of a sprint. Sleight-style sprint recovery (perk 57) cuts the drop to
/// `perk_sprintRecoveryMultiplier` of the weapon's time.
fn sprint_drop(pm: &mut Pm) {
    let mut t = pm.weapon_facts().moves.sprint_out;
    if pm.env.has_perk(perk::SPRINTRECOVERY) {
        t = (t as f32 * pm.env.dvars.perk_sprint_recovery_multiplier) as i32;
    }
    set_right(pm, state::SPRINT_DROP, t);
    let a = pick(pm, anim::SPRINT_OUT, anim::SPRINT_EMPTY_OUT);
    start_both(pm, a);
}

// ---------------------------------------------------------------------------------------------------------------
// Dive to prone
// ---------------------------------------------------------------------------------------------------------------

/// Dive check: a dive in the air moves both guns into DTP_IN. No shot until the dive lands and
/// DTP_OUT has run out.
pub fn check_for_dtp(pm: &mut Pm) {
    if no_gun(pm) {
        return;
    }
    let s = pm.ps.weaponstate;
    if attacking(s) || switching(s) || offhand(s) || shield(s) || diving(s) {
        return;
    }
    if pm.env.pm_flags & pmf::DTP_AIR == 0 {
        return;
    }
    if !dtp_is_sliding(pm) {
        dtp_in(pm);
    }
}

fn dtp_in(pm: &mut Pm) {
    let t = pm.weapon_facts().moves.dtp_in;
    set_both(pm, state::DTP_IN, t);
    let a = pick(pm, anim::DTP_IN, anim::DTP_EMPTY_IN);
    continue_on(pm, a, true);
}

/// Dtp_State_Loop (state DTP_IN ran out): hold until the dive lands.
pub fn dtp_loop(pm: &mut Pm) {
    set_both(pm, state::DTP_LOOP, 0);
    let a = pick(pm, anim::DTP_LOOP, anim::DTP_EMPTY_LOOP);
    continue_on(pm, a, true);
}

/// Dtp_IsSliding: the dive has landed and is sliding along the ground.
pub fn dtp_is_sliding(pm: &Pm) -> bool {
    pm.env.dvars.dtp && pm.env.pm_flags & pmf::DTP_AIR != 0 && pm.env.jump_time < 0
}

/// Dtp_State_Out (state DTP_LOOP, once the dive lands): the guns come back up.
pub fn dtp_out(pm: &mut Pm) {
    let t = pm.weapon_facts().moves.dtp_out;
    set_both(pm, state::DTP_OUT, t);
    let a = pick(pm, anim::DTP_OUT, anim::DTP_EMPTY_OUT);
    continue_on(pm, a, true);
}

/// Dtp_CanMove: just after a dive ends the player stays put for `dtp_post_move_pause` ms.
fn dtp_can_move(pm: &Pm) -> bool {
    let env = &pm.env;
    if env.dvars.dtp && env.pm_flags & pmf::DTP_AIR == 0 && env.last_dtp_end != 0 {
        let since = (pm.cmd.server_time - env.last_dtp_end) as f32;
        if since < env.dvars.dtp_post_move_pause {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------------------------------------------
// Slide (stepping onto slide ground)
// ---------------------------------------------------------------------------------------------------------------

/// Slide check: on the tick the slide starts, the right gun plays SLIDE_IN.
pub fn check_for_slide(pm: &mut Pm) {
    if no_gun(pm) {
        return;
    }
    let s = pm.ps.weaponstate;
    if attacking(s) || switching(s) || offhand(s) || shield(s) || diving(s) || s == state::SLIDE_IN
    {
        return;
    }
    if pm.env.pm_flags & pmf::SLIDE == 0 || pm.env.slide_time != pm.cmd.server_time {
        return;
    }
    let t = pm.weapon_facts().moves.slide_in;
    set_right(pm, state::SLIDE_IN, t);
    continue_on(pm, anim::SLIDE_IN, false);
}

// ---------------------------------------------------------------------------------------------------------------
// Mantle
// ---------------------------------------------------------------------------------------------------------------

/// PM_Weapon_CheckForMantle: the gun plays MANTLE while the mantle holds it; once the mantle is
/// over, a gun still in MANTLE goes back to idle.
pub fn check_for_mantle(pm: &mut Pm) {
    if pm.env.mantle_weapon_equipped {
        if pm.ps.weaponstate == state::SPRINT_DROP {
            pm.ps.weapon_time = 0;
        }
        continue_on(pm, anim::MANTLE, false);
    } else if pm.env.pm_flags & pmf::MANTLE == 0 && anim::number(pm.ps.weap_anim) == anim::MANTLE {
        base::idle(pm);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Swim
// ---------------------------------------------------------------------------------------------------------------

/// PM_WeaponSwimIn (state SWIM_IN ran out): the gun is put away.
pub fn swim_in(pm: &mut Pm) {
    pm.add_event(events::EV_PUTAWAY_WEAPON);
    if pm.ps.weap_flags & crate::player::wf::SWIM_LOWERED == 0 {
        continue_on(pm, anim::DROP, false);
    } else {
        base::start_anim(pm, anim::DROP, false);
    }
}

/// PM_WeaponSwimOut (state SWIM_OUT ran out): the gun comes back up. No raise time is set, so
/// RAISING finishes on the next tick.
pub fn swim_out(pm: &mut Pm) {
    pm.ps.weaponstate = state::RAISING;
    pm.add_event(events::EV_RAISE_WEAPON);
    base::start_anim(pm, anim::RAISE, false);
}

// ---------------------------------------------------------------------------------------------------------------
// Prone crawl
// ---------------------------------------------------------------------------------------------------------------

/// PM_Weapon_CheckForProne: moving while prone lowers the gun into the crawl; firing, standing
/// or stopping brings it back out (the fire exit uses the shorter `crawlOutFireTime`).
pub fn check_for_prone(pm: &mut Pm) {
    if pm.left() || no_gun(pm) {
        return;
    }
    let s = pm.ps.weaponstate;
    if attacking(s) || attacking(pm.ps.weaponstate_left) {
        return;
    }
    if (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&s)
        || switching(s)
        || offhand(s)
        || shield(s)
        || diving(s)
        || (state::SPRINT_RAISE..=state::SPRINT_DROP).contains(&s)
        || s == state::DETONATING
    {
        return;
    }
    if pm.env.pm_flags & pmf::DTP_AIR != 0 {
        return;
    }

    let limit_r = shot_limit_reached(pm);
    let fire_r = fire::should_be_firing(pm, true);
    let dual = pm.weapon_facts().core.dual_wield;
    let (mut fire_l, mut limit_l) = (false, false);
    if dual {
        pm.ps.b_run_left_gun = true;
        fire_l = fire::should_be_firing(pm, true);
        limit_l = shot_limit_reached(pm);
        pm.ps.b_run_left_gun = false;
    }

    if ((fire_r && !limit_r) || (fire_l && !limit_l)) && crawling(pm.ps.weaponstate) {
        crawl_out(pm, true);
    }
    let prone = pm.env.pm_flags & pmf::PRONE != 0;
    if !prone && (state::CRAWL_FORWARD..=state::CRAWL_LEFT).contains(&pm.ps.weaponstate) {
        crawl_out(pm, false);
    }
    if fire_r && (!limit_r || crawl_or_out(pm.ps.weaponstate)) {
        return;
    }
    if dual && fire_l && (!limit_l || crawl_or_out(pm.ps.weaponstate)) {
        return;
    }
    if prone {
        let moving = pm.cmd.forwardmove != 0 || pm.cmd.rightmove != 0;
        if moving {
            if !crawl_or_out(pm.ps.weaponstate) {
                crawl_in(pm);
            }
            return;
        }
        if !dtp_can_move(pm) {
            return;
        }
    }
    if crawling(pm.ps.weaponstate) {
        crawl_out(pm, false);
    }
}

fn crawl_in(pm: &mut Pm) {
    let t = pm.weapon_facts().moves.crawl_in;
    set_right(pm, state::CRAWL_IN, t);
    let a = pick(pm, anim::CRAWL_IN, anim::CRAWL_EMPTY_IN);
    start_both(pm, a);
}

fn crawl_out(pm: &mut Pm, fire: bool) {
    let m = pm.weapon_facts().moves;
    set_right(
        pm,
        state::CRAWL_OUT,
        if fire { m.crawl_out_fire } else { m.crawl_out },
    );
    let a = pick(pm, anim::CRAWL_OUT, anim::CRAWL_EMPTY_OUT);
    start_both(pm, a);
}

/// Crawl_Set_Direction (state CRAWL_IN ran out, and every crawl tick on the client): the crawl
/// anim follows the stick. Sideways wins a tie (`4f² <= r²`), so no stick reads as LEFT.
pub fn crawl_set_direction(pm: &mut Pm) {
    if wants_shot(pm) {
        return crawl_out(pm, true);
    }
    let f = i32::from(pm.cmd.forwardmove);
    let r = i32::from(pm.cmd.rightmove);
    let (st, a) = if 4 * f * f <= r * r {
        if r < 1 {
            (state::CRAWL_LEFT, anim::CRAWL_LEFT)
        } else {
            (state::CRAWL_RIGHT, anim::CRAWL_RIGHT)
        }
    } else if f < 1 {
        (state::CRAWL_BACK, anim::CRAWL_BACK)
    } else {
        (state::CRAWL_FORWARD, anim::CRAWL_FORWARD)
    };
    let a = if clip_empty(pm) {
        a + (anim::CRAWL_EMPTY_IN - anim::CRAWL_IN)
    } else {
        a
    };
    if pm.ps.weaponstate != st {
        set_right(pm, st, 0);
        start_both(pm, a);
    }
}

/// Crawl_State_Continue_Loop (states CRAWL_FORWARD..CRAWL_LEFT): a shot request ends the crawl;
/// otherwise the client keeps the anim on the stick (the server leaves the direction alone).
pub fn crawl_continue_loop(pm: &mut Pm) {
    if wants_shot(pm) {
        return crawl_out(pm, true);
    }
    if !pm.env.is_server {
        crawl_set_direction(pm);
    }
}

// Third-person starts that BO2 fires from its movement code, not the weapon tick. The game
// calls these at the same points its own movement reaches them; window 14 plays them.

/// `player_moveThreshhold` (BO2 default): slower than this counts as standing still.
const MOVE_THRESHOLD: f32 = 2.0;
/// `player_runThreshhold` (BO2 default): the speed a run-to-idle needs.
const RUN_THRESHOLD: f32 = 110.0;

/// The dive leaves the ground (right after DTP_AIR is set).
pub fn dtp_takeoff(pm: &mut Pm) {
    pm.out.scripts.push(AnimScript::DtpTakeoff);
}

/// The dive update on the ground: fires once, while DTP_AIR is still set and `jump_time` is
/// above 0. Returns true when it fired; the movement code then raises EV_DTP_LAND and sets
/// `jump_time` below 0 (the landed slide).
pub fn dtp_land(pm: &mut Pm) -> bool {
    let e = &pm.env;
    if !(e.dvars.dtp && e.pm_flags & pmf::DTP_AIR != 0 && e.jump_time > 0) {
        return false;
    }
    pm.out.scripts.push(AnimScript::DtpLand);
    true
}

/// Footsteps: moving at run speed with the stick let go. `xyspeed` = horizontal speed.
pub fn run_to_idle(pm: &mut Pm, xyspeed: f32) -> bool {
    let e = &pm.env;
    let f = e.pm_flags;
    let in_air = !e.on_ground
        && e.waterlevel == 0
        && f & (pmf::DTP_AIR | pmf::SLIDE) == 0
        && f & pmf::MANTLE == 0;
    let moving = (xyspeed >= MOVE_THRESHOLD && e.pm_type != pm_type::NORMAL_LINKED)
        || f & (pmf::SLIDE | pmf::MANTLE) != 0;
    let fires = e.pm_type < pm_type::DEAD
        // eFlags 0x300: BO2 holds the legs idle and skips footsteps.
        && e.e_flags & 0x300 == 0
        && !in_air
        && moving
        && pm.cmd.forwardmove == 0
        && pm.cmd.rightmove == 0
        && f & (pmf::SLIDE | pmf::DTP_AIR) == 0
        && xyspeed >= RUN_THRESHOLD;
    if fires {
        pm.out.scripts.push(AnimScript::RunToIdle);
    }
    fires
}

// Sprint button rules. The movement code asks these before it starts or keeps a sprint; the
// weapon only answers. Buttons are read as BO2 sends them: CROUCH / PRONE carry the stance the
// player wants (BO2 adds the current stance to every command), not only the key.

/// `player_sprintForwardMinimum` (BO2 default): the stick must push forward past this.
const SPRINT_FORWARD_MINIMUM: i32 = 105;
/// pm_flags bits these rules test whose meaning is not identified yet.
const PMF_GATE_4000: u32 = 0x4000;
const PMF_GATE_10000: u32 = 0x1_0000;
const PMF_GATE_40000: u32 = 0x4_0000;

/// Buttons that stop a sprint from starting or going on, in BO2 MP: jump, reload, melee,
/// attack, use-reload, lethal, tactical.
const SPRINT_BREAKERS: u32 = button::JUMP
    | button::RELOAD
    | button::MELEE
    | button::ATTACK
    | button::USE_RELOAD
    | button::FRAG
    | button::SMOKE;

/// Zombies keeps only melee, attack and ads-throw as breakers (BO2 tests this with the default
/// forward minimum, which is above 0).
fn zombies_breaks(buttons: u32) -> bool {
    buttons & (button::MELEE | button::ATTACK | button::TOGGLE_ADS) != 0
}

/// ADS-throw pressed with a held weapon that is not equipment.
fn ads_throw_breaks(pm: &Pm, buttons: u32) -> bool {
    buttons & button::TOGGLE_ADS != 0
        && pm.weapon_facts().core.offhand_slot != offhand_slot::EQUIPMENT
}

/// Melee, offhand or riot shield states.
fn melee_offhand_or_shield(s: i32) -> bool {
    (state::MELEE_INIT..=state::MELEE_END).contains(&s)
        || (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
}

/// The sprint-flag part of BO2's sprint update, for the trace runner and any caller without its
/// own movement code. Sprint stamina (length, cooldown, minimum left) and the room-to-crouch and
/// room-to-stand checks are left out: they belong to movement. `up_required` is BO2's "let go of
/// sprint first" latch, kept by the caller between ticks. Letting go of sprint does not end a
/// sprint; a breaker button does, and if sprint is still held then it must be let go before the
/// next sprint.
pub fn update_sprint_flag(pm: &mut Pm, up_required: &mut bool, pm_time: i32) {
    let held = pm.cmd.held(button::SPRINT);
    if *up_required && !held && pm.env.pm_flags & pmf::NO_FIRE == 0 {
        *up_required = false;
    }
    let sprinting = pm.env.pm_flags & pmf::SPRINT != 0;
    let alive = matches!(pm.env.pm_type, pm_type::NORMAL | pm_type::NORMAL_LINKED);
    let low_ready = (state::LOWREADY_RAISE..=state::LOWREADY_DROP).contains(&pm.ps.weaponstate);
    let end = if !alive || low_ready {
        sprinting
    } else if sprinting {
        sprint_ending_buttons(pm, 0.0)
    } else {
        if held
            && !*up_required
            && pm.env.pm_flags & PMF_GATE_40000 == 0
            && !sprint_start_interfering_buttons(
                pm,
                pm.cmd.forwardmove,
                pm.cmd.buttons,
                0.0,
                pm_time,
            )
        {
            pm.env.pm_flags |= pmf::SPRINT;
        }
        false
    };
    if end {
        pm.env.pm_flags &= !pmf::SPRINT;
        if held {
            *up_required = true;
        }
    }
}

/// PM_SprintEndingButtons: true = this command ends the running sprint. `leanf` is the lean
/// fraction (BO2 MP has no lean key, so it stays 0).
pub fn sprint_ending_buttons(pm: &Pm, leanf: f32) -> bool {
    let e = &pm.env;
    if e.waterlevel > 1 || e.pm_flags & (PMF_GATE_10000 | pmf::LADDER | pmf::ADS) != 0 {
        return true;
    }
    if i32::from(pm.cmd.forwardmove) <= SPRINT_FORWARD_MINIMUM {
        return true;
    }
    let b = pm.cmd.buttons;
    if e.zombies {
        return zombies_breaks(b);
    }
    if b & SPRINT_BREAKERS != 0 {
        return true;
    }
    if b & button::PRONE != 0 && !pm.weapon_facts().moves.blocks_prone {
        return true;
    }
    if b & button::CROUCH != 0 || leanf != 0.0 || ads_throw_breaks(pm, b) {
        return true;
    }
    let s = pm.ps.weaponstate;
    if melee_offhand_or_shield(s)
        || matches!(s, state::NIGHTVISION_WEAR | state::NIGHTVISION_REMOVE)
    {
        return true;
    }
    e.pm_flags & pmf::SLIDE != 0 || e.pm_flags & PMF_GATE_40000 != 0
}

/// PM_SprintStartInterferingButtons: true = a sprint may not start with this command.
/// `pm_time` is the movement timer (BO2 `pm_time`).
pub fn sprint_start_interfering_buttons(
    pm: &Pm,
    forwardmove: i8,
    buttons: u32,
    leanf: f32,
    pm_time: i32,
) -> bool {
    let e = &pm.env;
    if e.pm_flags & pmf::LADDER != 0 || e.waterlevel > 1 {
        return true;
    }
    if i32::from(forwardmove) <= SPRINT_FORWARD_MINIMUM {
        return true;
    }
    if e.zombies {
        return zombies_breaks(buttons);
    }
    if buttons & SPRINT_BREAKERS != 0 || ads_throw_breaks(pm, buttons) {
        return true;
    }
    if leanf != 0.0 || e.pm_flags & (PMF_GATE_10000 | pmf::ADS | pmf::LADDER | pmf::MANTLE) != 0 {
        return true;
    }
    if e.pm_flags & PMF_GATE_4000 != 0 && pm_time == 0 {
        return false;
    }
    let s = pm.ps.weaponstate;
    if melee_offhand_or_shield(s) {
        return true;
    }
    // Equipment in hand: still firing or rechambering also blocks the start.
    if pm.weapon_facts().core.offhand_slot == offhand_slot::EQUIPMENT
        && matches!(
            s,
            state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
        )
    {
        return true;
    }
    e.pm_flags & pmf::SLIDE != 0 || e.pm_flags & pmf::DTP_AIR != 0
}

#[cfg(test)]
mod tests {
    use super::MoveFacts;
    use crate::cmd::{T6Cmd, button};
    use crate::env::{T6Env, pmf};
    use crate::facts::{FactsTable, T6WeaponFacts};
    use crate::player::T6WeaponPs;
    use crate::pm::FrameOut;
    use crate::trace::Runner;
    use crate::{anim, events, state, tick};

    /// The AN-94 and Kard with their patch-zone numbers (SPRINT.md section 10); 50 ms ticks.
    fn facts() -> FactsTable {
        let mut t = FactsTable::new();
        let an94 = MoveFacts {
            sprint_in: 200,
            sprint_loop: 877,
            sprint_out: 250,
            dtp_in: 150,
            dtp_loop: 700,
            dtp_out: 250,
            crawl_in: 300,
            crawl_forward: 1100,
            crawl_back: 1000,
            crawl_right: 1200,
            crawl_left: 1100,
            crawl_out_fire: 48,
            crawl_out: 100,
            slide_in: 500,
            sprint_duration_scale: 1.0,
            blocks_prone: false,
        };
        let kard = MoveFacts {
            sprint_in: 200,
            sprint_loop: 550,
            sprint_out: 200,
            dtp_in: 80,
            crawl_in: 150,
            crawl_out_fire: 32,
            ..an94
        };
        for (i, name, m) in [(1u32, "an94_mp", an94), (2, "kard_mp", kard)] {
            let mut f = T6WeaponFacts::default();
            f.core.clip_size = 30;
            f.switch.raise = 500;
            f.switch.drop = 400;
            f.moves = m;
            t.set(i, name, f);
        }
        t
    }

    fn run(script: &str) -> String {
        let facts = facts();
        let r = Runner::new(&facts).run_script(script).expect("script runs");
        let text = r.lines.join("\n");
        assert_eq!(r.failures, 0, "{text}");
        text
    }

    /// A ready AN-94 for the checks the script runner has no command for (mantle, swim dvar).
    struct Bench {
        facts: FactsTable,
        ps: T6WeaponPs,
        env: T6Env,
        cmd: T6Cmd,
    }

    impl Bench {
        fn new() -> Self {
            let mut ps = T6WeaponPs::default();
            ps.give(1);
            ps.set_ammo_in_clip(1, 30);
            ps.weapon = 1;
            Self {
                facts: facts(),
                ps,
                env: T6Env {
                    is_server: true,
                    view_height_settled: true,
                    ..T6Env::default()
                },
                cmd: T6Cmd {
                    msec: 50,
                    weapon: 1,
                    ..T6Cmd::default()
                },
            }
        }

        fn step(&mut self) -> FrameOut {
            self.cmd.server_time += self.cmd.msec;
            tick::run(&mut self.ps, &self.cmd, &mut self.env, &self.facts)
        }

        fn steps(&mut self, n: usize) {
            for _ in 0..n {
                self.step();
            }
        }
    }

    #[test]
    fn sprint_raise_loop_drop_then_ready() {
        run("
            spawn an94_mp
            run 1
            stick 127 0
            hold sprint
            run 1
            expect state SPRINT_RAISE
            expect anim SPRINT_IN
            run 3
            expect state SPRINT_RAISE
            run 1   # 200 ms in
            expect state SPRINT_LOOP
            expect anim SPRINT_LOOP
            run 10
            expect state SPRINT_LOOP
            stick 0 0 # stick let go: the sprint ends
            run 1
            expect state SPRINT_DROP
            expect anim SPRINT_OUT
            run 4
            expect state SPRINT_DROP
            run 1   # 250 ms out: free to fire from the next tick
            expect state READY
            expect anim IDLE
        ");
    }

    #[test]
    fn sprint_recovery_cuts_the_drop_to_three_tenths() {
        run("
            spawn an94_mp
            perk 57
            stick 127 0
            hold sprint
            run 6
            expect state SPRINT_LOOP
            stick 0 0 # stick let go: the sprint ends
            run 1   # 250 * 0.3 = 75 ms
            expect state SPRINT_DROP
            run 1
            expect state SPRINT_DROP
            run 1
            expect state READY
        ");
    }

    #[test]
    fn empty_clip_picks_the_empty_sprint() {
        run("
            spawn kard_mp
            clip kard_mp 0
            stick 127 0
            hold sprint
            run 1
            expect anim SPRINT_EMPTY_IN
            run 4
            expect anim SPRINT_EMPTY_LOOP
            stick 0 0 # stick let go: the sprint ends
            run 1
            expect anim SPRINT_EMPTY_OUT
        ");
    }

    #[test]
    fn dive_in_loop_land_out() {
        run("
            spawn an94_mp
            stick 127 0
            hold sprint
            run 6
            expect state SPRINT_LOOP
            pmflags 0x208000 # dive from the sprint
            release sprint
            run 1
            expect state DTP_IN
            expect left DTP_IN
            expect anim DTP_IN
            run 3   # 150 ms
            expect state DTP_LOOP
            expect anim DTP_LOOP
            run 10  # still in the air
            expect state DTP_LOOP
            pmflags 0x1 # landed prone
            run 1
            expect state DTP_OUT
            expect anim DTP_OUT
            run 4
            expect state DTP_OUT
            run 1   # 250 ms
            expect state READY
            expect left READY
        ");
    }

    #[test]
    fn landed_dive_sliding_ends_the_loop() {
        run("
            spawn an94_mp
            pmflags 0x200000
            run 4
            expect state DTP_LOOP
            jumptime -1
            run 1
            expect state DTP_OUT
        ");
    }

    #[test]
    fn slide_ground_plays_slide_in_on_the_right_gun() {
        run("
            spawn an94_mp
            run 1
            pmflags 0x400000
            run 2
            expect state READY # the slide has not started
            slidenext
            run 1
            expect state SLIDE_IN
            expect anim SLIDE_IN
            run 9
            expect state SLIDE_IN
            run 1   # 500 ms
            expect state READY
        ");
    }

    #[test]
    fn crawl_in_follows_the_stick_on_the_client_then_out() {
        run("
            server off
            spawn an94_mp
            pmflags 0x1
            run 2
            expect state READY
            stick 127 0
            run 1
            expect state CRAWL_IN
            expect anim CRAWL_IN
            run 5
            expect state CRAWL_IN
            run 1   # 300 ms
            expect state CRAWL_FORWARD
            expect anim CRAWL_FORWARD
            stick 0 127
            run 1
            expect state CRAWL_RIGHT
            stick 0 -127
            run 1
            expect state CRAWL_LEFT
            stick -127 20
            run 1
            expect state CRAWL_BACK
            expect anim CRAWL_BACK
            stick 0 0
            run 1
            expect state CRAWL_OUT
            expect anim CRAWL_OUT
            run 1
            expect state CRAWL_OUT
            run 1   # 100 ms
            expect state READY
        ");
    }

    #[test]
    fn server_keeps_the_crawl_direction() {
        run("
            server on
            spawn an94_mp
            pmflags 0x1
            stick 127 0
            run 7
            expect state CRAWL_FORWARD
            stick 0 127
            run 3
            expect state CRAWL_FORWARD
        ");
    }

    #[test]
    fn standing_up_mid_crawl_brings_the_gun_out() {
        run("
            server off
            spawn an94_mp
            pmflags 0x1
            stick 127 0
            run 7
            expect state CRAWL_FORWARD
            pmflags 0
            run 1
            expect state CRAWL_OUT
        ");
    }

    #[test]
    fn no_crawl_straight_after_a_dive() {
        // Prone, still, a dive just ended: the gun holds (no crawl-out, no crawl-in).
        run("
            server off
            spawn an94_mp
            pmflags 0x200000
            run 4
            pmflags 0x1
            run 6
            expect state READY
            dtpend
            run 1
            expect state READY
        ");
    }

    #[test]
    fn mantle_cuts_the_sprint_drop_and_holds_mantle() {
        let mut b = Bench::new();
        b.env.pm_flags = pmf::SPRINT;
        b.steps(6);
        b.env.pm_flags = 0;
        b.step();
        assert_eq!(b.ps.weaponstate, state::SPRINT_DROP);
        b.env.mantle_weapon_equipped = true;
        b.env.pm_flags = pmf::MANTLE;
        b.step();
        assert_eq!(b.ps.weaponstate, state::READY, "the drop is cut short");
        b.step();
        assert_eq!(anim::number(b.ps.weap_anim), anim::MANTLE);
        let held = b.ps.weap_anim;
        b.step();
        assert_eq!(b.ps.weap_anim, held, "held, not restarted");
        b.env.mantle_weapon_equipped = false;
        b.env.pm_flags = 0;
        b.step();
        assert_eq!(anim::number(b.ps.weap_anim), anim::IDLE);
    }

    #[test]
    fn swim_puts_the_gun_away_and_brings_it_back() {
        let mut b = Bench::new();
        b.step();
        b.env.dvars.player_disable_weapons_in_water = true;
        b.env.waterlevel = 4;
        let out = b.step();
        assert_eq!(b.ps.weaponstate, state::SWIM_IN);
        assert_eq!(anim::number(b.ps.weap_anim), anim::DROP);
        assert!(out.events.iter().any(|e| e.id == events::EV_PUTAWAY_WEAPON));
        b.steps(5);
        assert_eq!(b.ps.weaponstate, state::SWIM_IN);
        b.env.waterlevel = 0;
        b.step();
        assert_eq!(b.ps.weaponstate, state::SWIM_OUT);
        let mut ticks = 0;
        loop {
            ticks += 1;
            let out = b.step();
            if out.events.iter().any(|e| e.id == events::EV_RAISE_WEAPON) {
                break;
            }
            assert!(ticks < 40, "the gun never came back up");
        }
        // The water block sets 1000 ms before the timer step of the same tick: 1 + 19 ticks.
        assert_eq!(ticks, 19, "SWIM_OUT runs 1000 ms");
        assert_eq!(b.ps.weaponstate, state::RAISING);
        assert_eq!(anim::number(b.ps.weap_anim), anim::RAISE);
        b.step();
        assert_eq!(b.ps.weaponstate, state::READY, "no extra raise time");
    }

    #[test]
    fn third_person_dive_and_run_to_idle_starts() {
        let mut b = Bench::new();
        let mut out = FrameOut::default();
        let mut hook = |b: &mut Bench, f: &dyn Fn(&mut crate::pm::Pm) -> bool| {
            let mut pm = crate::pm::Pm {
                ps: &mut b.ps,
                cmd: &b.cmd,
                env: &mut b.env,
                facts: &b.facts,
                out: &mut out,
                delay_fired: false,
            };
            f(&mut pm)
        };
        // Takeoff always fires; land only while flagged in the air with a positive jump time.
        hook(&mut b, &|pm| {
            super::dtp_takeoff(pm);
            true
        });
        b.env.pm_flags = pmf::DTP_AIR;
        b.env.jump_time = 0;
        assert!(!hook(&mut b, &|pm| super::dtp_land(pm)));
        b.env.jump_time = 1000;
        assert!(hook(&mut b, &|pm| super::dtp_land(pm)));
        // Run-to-idle: on the ground, stick let go, at run speed (110) or faster.
        b.env.pm_flags = 0;
        b.env.on_ground = true;
        assert!(!hook(&mut b, &|pm| super::run_to_idle(pm, 109.0)));
        assert!(hook(&mut b, &|pm| super::run_to_idle(pm, 190.0)));
        b.cmd.forwardmove = 127;
        assert!(!hook(&mut b, &|pm| super::run_to_idle(pm, 190.0)));
        b.cmd.forwardmove = 0;
        b.env.on_ground = false;
        assert!(!hook(&mut b, &|pm| super::run_to_idle(pm, 190.0)));
        use events::AnimScript as A;
        assert_eq!(out.scripts, vec![A::DtpTakeoff, A::DtpLand, A::RunToIdle]);
    }

    #[test]
    fn sprint_button_rules() {
        let mut b = Bench::new();
        let mut out = FrameOut::default();
        let mut ask = |b: &mut Bench, f: &dyn Fn(&crate::pm::Pm) -> bool| {
            let pm = crate::pm::Pm {
                ps: &mut b.ps,
                cmd: &b.cmd,
                env: &mut b.env,
                facts: &b.facts,
                out: &mut out,
                delay_fired: false,
            };
            f(&pm)
        };
        let ends = |pm: &crate::pm::Pm| super::sprint_ending_buttons(pm, 0.0);
        b.ps.weaponstate = state::READY;
        b.cmd.forwardmove = 127;
        assert!(
            !ask(&mut b, &ends),
            "full stick, no buttons: sprint goes on"
        );
        b.cmd.forwardmove = 105;
        assert!(ask(&mut b, &ends), "105 is not past the forward minimum");
        b.cmd.forwardmove = 127;
        for bit in [
            button::ATTACK,
            button::MELEE,
            button::RELOAD,
            button::USE_RELOAD,
            button::JUMP,
            button::FRAG,
            button::SMOKE,
            button::CROUCH,
            button::PRONE,
            button::TOGGLE_ADS,
        ] {
            b.cmd.buttons = bit;
            assert!(ask(&mut b, &ends), "button {bit:#x} ends the sprint");
        }
        b.cmd.buttons = button::USE;
        assert!(!ask(&mut b, &ends), "use does not");
        b.cmd.buttons = 0;
        b.env.pm_flags = pmf::ADS;
        assert!(ask(&mut b, &ends));
        b.env.pm_flags = pmf::SLIDE;
        assert!(ask(&mut b, &ends));
        b.env.pm_flags = 0;
        b.ps.weaponstate = state::MELEE_INIT;
        assert!(ask(&mut b, &ends));
        b.ps.weaponstate = state::RELOADING;
        assert!(!ask(&mut b, &ends), "a reload in progress does not end it");
        // Zombies: only melee, attack and ads-throw.
        b.env.zombies = true;
        b.cmd.buttons = button::RELOAD;
        assert!(!ask(&mut b, &ends));
        b.cmd.buttons = button::ATTACK;
        assert!(ask(&mut b, &ends));
        b.env.zombies = false;
        b.cmd.buttons = 0;

        let starts_blocked = |fwd: i8, buttons: u32, pm_time: i32| {
            move |pm: &crate::pm::Pm| {
                super::sprint_start_interfering_buttons(pm, fwd, buttons, 0.0, pm_time)
            }
        };
        b.ps.weaponstate = state::READY;
        assert!(!ask(&mut b, &starts_blocked(127, 0, 0)));
        assert!(ask(&mut b, &starts_blocked(100, 0, 0)));
        assert!(ask(&mut b, &starts_blocked(127, button::ATTACK, 0)));
        assert!(
            !ask(&mut b, &starts_blocked(127, button::CROUCH, 0)),
            "crouch held may start"
        );
        b.ps.weaponstate = state::FIRING;
        assert!(
            !ask(&mut b, &starts_blocked(127, 0, 0)),
            "a gun mid-shot may start a sprint"
        );
        b.env.pm_flags = pmf::DTP_AIR;
        b.ps.weaponstate = state::READY;
        assert!(ask(&mut b, &starts_blocked(127, 0, 0)));
        b.env.pm_flags = super::PMF_GATE_4000 | pmf::DTP_AIR;
        assert!(
            !ask(&mut b, &starts_blocked(127, 0, 0)),
            "0x4000 with pm_time 0 allows it"
        );
        assert!(ask(&mut b, &starts_blocked(127, 0, 50)));
    }
}
