//! bo2mp: the BO2 weapon brain (weapon_t6) drives the held gun's state,
//! timers and viewmodel anim in place of the old MW2 tick (weapon_t6::combat_facts). The legacy player state stays the
//! source for what he holds and his ammo; the T6 state lives in his match state between ticks.
//! Unset (or no gun facts in the map's content) = that old MW2 tick as before.
//!
//! The local player gets the viewmodel values in his snapshot (`ClientMatchState::t6_view`).
//! Shots: each T6 fire event becomes an accepted shot (bullets, projectiles) aimed with BO2's
//! spread (`weapon_t6::recoil`). Melee: the T6 swipe event plays the swing sound and the T6
//! melee event lands the hit with BO2's damage and reach. The dry-fire click, bolt, brass and
//! pull-back go out as the legacy path's predictable events (`fire_sound_event`).
//! Not driven yet: grenades from the T6 events.

use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick};
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, UserCmd, pm_flags as gpmf};
use std::sync::OnceLock;
use weapon_t6::ammo_table::{ammo_row_present, ammo_table_key, clip_row_present, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, set_ammo_not_in_clip, set_clip_for_hand};
use weapon_t6::cmd::T6Cmd;
use weapon_t6::env::{T6Env, perk, pmf};
use weapon_t6::events as ev;
use weapon_t6::facts::FactsTable;
use weapon_t6::facts::{fire_type, inventory};
use weapon_t6::player::T6WeaponPs;
use weapon_t6::pm::dual::{dual_wield_weapon, is_left_hand_weapon};
use weapon_t6::recoil::{
    AimSpreadInput, RecoilDvars, SpreadInput, SpreadOverride, adjust_aim_spread_scale, bullet_cone,
    spread_for_weapon, stack_fire_yaw,
};
use weapon_t6::{anim, tick};

/// What the T6 brain keeps for one player between ticks: its player state, plus the movement
/// and entity bits BO2 keeps in `pm_flags` / `eFlags2` that only the weapon code sets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct T6Gun {
    ps: T6WeaponPs,
    pm_flags: u32,
    e_flags2: u32,
    /// Third-person anim script events since the body anim last read them (window 14;
    /// server only, drained by `take_third`).
    third: Vec<weapon_t6::events::AnimScript>,
    /// The server frame `frame_aim_spread_scale` was saved in.
    frame_tick: Option<Tick>,
    /// BO2's `currentAimSpreadScale`: the aim spread scale as the last server frame ended. Its
    /// bullets aim with this, so a shot never sees this frame's moves or its own fire add.
    frame_aim_spread_scale: f32,
    /// The usercmds' client events since the script side last took them (window 13 sends their
    /// notifies: reload_start, weapon_fired, grenade_pullback ...).
    events: Vec<weapon_t6::events::Event>,
    /// Where in `events` each usercmd's new press of BO2's double-tap button fell: the script
    /// side tells `doubletap_detonate` there (ClientThink_real does it before the move).
    double_taps: Vec<usize>,
    /// The usercmd's weapon on his last move: the one he is switching to (BO2's `cmd.weapon`,
    /// which the body anims' next-weapon conditions read).
    cmd_weapon: u32,
}

/// Events kept for the script side at most (a map with no script pass never takes them).
const KEPT_EVENTS: usize = 256;

impl T6Gun {
    fn new() -> Self {
        Self {
            ps: T6WeaponPs::default(),
            pm_flags: 0,
            e_flags2: 0,
            third: Vec::new(),
            frame_tick: None,
            frame_aim_spread_scale: 0.0,
            events: Vec::new(),
            double_taps: Vec::new(),
            cmd_weapon: 0,
        }
    }
}

impl T6Gun {
    /// The raw weapon state and the offhand item in hand (T6 weapon index), for BO2's cancel
    /// check when the switch button is pressed.
    pub(crate) fn offhand_in_hand(&self) -> (i32, u32) {
        (self.ps.weaponstate, self.ps.off_hand_weapon)
    }

    /// The values the client's BO2 viewmodel player reads, as raw T6 numbers.
    pub(crate) fn view_inputs(&self) -> crate::T6ViewInputs {
        let ps = &self.ps;
        crate::T6ViewInputs {
            weap_anim: ps.weap_anim,
            weap_anim_left: ps.weap_anim_left,
            weaponstate: ps.weaponstate,
            weaponstate_left: ps.weaponstate_left,
            weapon_time: ps.weapon_time,
            weapon_time_left: ps.weapon_time_left,
            intro_shots_fired: ps.intro_shots_fired,
            ever_had: 0,
            needs_rechamber: 0,
        }
    }

    /// The raised-before bits for the snapshot, keyed by the player's legacy weapon slots (a gun
    /// the brain does not hold yet has never been raised).
    pub(crate) fn ever_had_bits(&self, weapons: &[i32]) -> u16 {
        weapons.iter().enumerate().fold(0, |bits, (i, &w)| {
            if w > 0 && self.ps.ever_had_weapon(w as u32) {
                bits | 1 << i
            } else {
                bits
            }
        })
    }

    /// The still-to-bolt bits for the snapshot, keyed like `ever_had_bits`.
    pub(crate) fn needs_rechamber_bits(&self, weapons: &[i32]) -> u16 {
        weapons.iter().enumerate().fold(0, |bits, (i, &w)| {
            if w > 0 && self.ps.held(w as u32).is_some_and(|h| h.needs_rechamber) {
                bits | 1 << i
            } else {
                bits
            }
        })
    }
}

/// The `pm_flags` bits the gun keeps for this frame: ADS granted, and the prone override, which
/// is the player's own bit: movement eases it off when the keys ease with the gun at rest, so the
/// gun reads it back from the player instead of from its own copy (which would hold the crawl
/// until the gun idles). Slide ground is the player's too: BO2's movement sets it while the
/// player stands on a slick surface.
fn kept_pm_flags(ps: &PlayerState, gun_flags: u32) -> u32 {
    let mut f = gun_flags & pmf::ADS;
    if ps.pm_flags & gpmf::PRONEMOVE_OVERRIDDEN != 0 {
        f |= pmf::PRONE_MOVE_OVERRIDE;
    }
    if ps.pm_flags & movement_t6::PMF_SLIDING != 0 {
        f |= pmf::SLIDE;
    }
    f
}

/// PM_SetProneMovementOverride reaches the crawl: firing, meleeing, throwing or switching
/// guns while prone holds the player still, and the gun's idle lets go.
fn share_prone_override(ps: &mut PlayerState, t6_flags: u32) {
    if t6_flags & pmf::PRONE_MOVE_OVERRIDE != 0 {
        ps.pm_flags |= gpmf::PRONEMOVE_OVERRIDDEN;
    } else {
        ps.pm_flags &= !gpmf::PRONEMOVE_OVERRIDDEN;
    }
}

/// `eFlags2` bits the T6 brain owns (empty raise).
const KEPT_E_FLAGS2: u32 = weapon_t6::switch::EF2_EMPTY_RAISE;


/// The BO2MP_GUN_TRACE file. Its own thread writes it: a write in place here held the
/// game 8-14 s at a time when D: was busy (his 10-15 play-guns freezes).
fn trace_file() -> Option<&'static diag::LineFile> {
    static FILE: OnceLock<Option<diag::LineFile>> = OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::var_os("BO2MP_GUN_TRACE")?;
        diag::LineFile::create(std::path::Path::new(&path))
    })
    .as_ref()
}

/// The game layer's `pm_flags` bits as BO2's.
fn pm_flags(game: u32) -> u32 {
    let mut f = 0;
    for (from, to) in [
        (gpmf::PRONE, pmf::PRONE),
        (gpmf::MANTLE, pmf::MANTLE),
        (gpmf::LADDER, pmf::LADDER),
        (gpmf::SPRINTING, pmf::SPRINT),
        (gpmf::MELEE_CHARGE, pmf::MELEE_CHARGE),
        (movement_t6::bridge::PMF_DIVE, pmf::DTP_AIR),
    ] {
        if game & from != 0 {
            f |= to;
        }
    }
    f
}

/// BO2's climb bits for the gun (window 7): Mantle_IsWeaponEquipped, Mantle_IsWeaponInactive, the
/// `mantleState.flags` bits the gun reads (0x100 the climb ended this frame, 0x2000 fast mantle) and the climb
/// time still to run. Low climbs keep the gun (and play its climb anim), high ones put it away.
fn mantle_env(ps: &PlayerState, env: &mut T6Env) {
    use playerstate_t6::game::mantle_flags as mf;
    let mut flags = 0;
    if ps.mantle_state.flags & mf::QUICK != 0 {
        flags |= 0x100;
    }
    if ps.mantle_state.flags & mf::FAST_MANTLE != 0 {
        flags |= 0x2000;
    }
    env.mantle_flags = flags;
    if ps.pm_flags & gpmf::MANTLE == 0 {
        return;
    }
    let inactive = movement_t6::bridge::mantle::is_weapon_inactive(ps, true);
    env.mantle_weapon_inactive = inactive;
    env.mantle_weapon_equipped = !inactive;
    env.mantle_time_left = (mf::bo2_duration_ms(ps.mantle_state.flags) - ps.mantle_state.timer).max(0);
}

/// BO2's dive `jumpTime`: above 0 from take-off, below 0 once the landed dive slides.
fn dtp_jump_time(ps: &PlayerState) -> i32 {
    if movement_t6::bridge::dive_sliding(ps) {
        ps.jump_time
    } else if movement_t6::bridge::dive_in_air(ps) {
        ps.jump_time.max(1)
    } else {
        0
    }
}

fn ammo_keys(world: &crate::world::SimState, w: u32) -> Option<(i32, i32)> {
    let f = world.combat_facts_for(w)?;
    Some((
        ammo_table_key(f.ammo_index, w),
        clip_table_key(f.clip_index, w),
    ))
}

/// BO2's G_GivePlayerWeapon also holds the gun's alternate weapon (a launcher attachment, the
/// other fire mode) as a weapon of its own, except for a charge-shot gun: the mode held with `w`
/// among the `held` guns (0: none).
pub(crate) fn alt_mode_held_with(facts: &FactsTable, held: &[u32], w: u32) -> u32 {
    let c = facts.get(w).core;
    if c.fire_type == fire_type::CHARGESHOT || held.contains(&c.alt_weapon) {
        0
    } else {
        c.alt_weapon
    }
}

/// Held weapons and ammo: legacy -> T6 (script gives, takes and pickups land in the legacy state).
fn sync_in(
    world: &crate::world::SimState,
    facts: &FactsTable,
    ps: &PlayerState,
    t6: &mut T6WeaponPs,
) {
    let held: Vec<u32> = ps
        .weapons
        .iter()
        .filter(|&&w| w > 0)
        .map(|&w| w as u32)
        .collect();
    // BO2 holds a dual gun's left-hand weapon as a weapon of its own; legacy lists only the gun.
    let lefts: Vec<u32> = held
        .iter()
        .map(|&w| dual_wield_weapon(facts, w))
        .filter(|&l| l != 0)
        .collect();
    let alts: Vec<u32> = held
        .iter()
        .map(|&w| alt_mode_held_with(facts, &held, w))
        .filter(|&a| a != 0)
        .collect();
    for h in t6.held {
        if h.weapon != 0
            && !held.contains(&h.weapon)
            && !lefts.contains(&h.weapon)
            && !alts.contains(&h.weapon)
        {
            t6.take(h.weapon);
        }
    }
    for &w in held.iter().chain(&alts) {
        t6.give(w);
        let left = dual_wield_weapon(facts, w);
        if left != 0 {
            t6.give(left);
        }
        if let Some((ammo, clip)) = ammo_keys(world, w) {
            // BO2 keeps one count per owner (BG_GetWeaponForAmmoClip / Pool): an alt mode that
            // fires from its gun's magazine and a charge level read their owner's count, so only
            // owners sync in; sync_out mirrors the shared count back to every row.
            if facts.ammo_clip_weapon(w) == w {
                t6.set_clip_slot(w, get_clip_for_hand(&ps.ammoclip, clip, 0));
            }
            // legacy keeps a dual gun's left clip as the second hand of its own row; BO2 keeps it
            // under the left-hand weapon.
            if left != 0 {
                t6.set_clip_slot(left, get_clip_for_hand(&ps.ammoclip, clip, 1));
            }
            // A left-hand dual gun draws from its partner's reserve (BO2): only pools sync.
            if facts.ammo_pool_weapon(w) == w {
                t6.set_pool_slot(w, get_ammo_not_in_clip(&ps.ammo, ammo));
            }
        }
    }
    hold_in_game_slots(t6, &ps.weapons);
    // BO2: giving a melee-inventory weapon makes it the melee weapon, taking it clears it. Every
    // class gives `knife_mp`, which the game layer's loadouts leave out, so it stands in when none is held.
    let is_melee = |w: u32| w != 0 && facts.get(w).core.inventory_type == inventory::MELEE;
    t6.melee_weapon = held
        .iter()
        .copied()
        .find(|&w| is_melee(w))
        .or_else(|| {
            world
                .weapon_index_by_script_name("knife_mp")
                .filter(|&w| is_melee(w))
        })
        .unwrap_or(0);
}

/// Window 5: each held weapon sits in the same slot as in the game layer's list (BO2 sends its held
/// weapons by slot). The slot order picks the offhand: the first grenade held is the lethal, the
/// next the tactical, so a client that held a swapped-in semtex after his concussion threw the
/// concussion. Weapons that list leaves out (a dual gun's left hand) keep their order in the rest.
fn hold_in_game_slots(t6: &mut T6WeaponPs, weapons: &[i32]) {
    use weapon_t6::player::{HeldWeapon, MAX_HELD};
    let old = t6.held;
    let mut held = [HeldWeapon::default(); MAX_HELD];
    let mut placed = [false; MAX_HELD];
    for (i, &w) in weapons.iter().enumerate().take(MAX_HELD) {
        if w <= 0 {
            continue;
        }
        if let Some(j) = old.iter().position(|h| h.weapon == w as u32) {
            held[i] = old[j];
            placed[j] = true;
        }
    }
    let listed = |i: usize| weapons.get(i).is_some_and(|&w| w > 0);
    for (j, h) in old.iter().enumerate() {
        if h.weapon == 0 || placed[j] {
            continue;
        }
        if let Some(k) = (0..MAX_HELD).find(|&k| held[k].weapon == 0 && !listed(k)) {
            held[k] = *h;
        }
    }
    t6.held = held;
}

/// Ammo: T6 -> legacy and the match state's per-weapon rows (the HUD reads those).
fn sync_out(
    world: &mut FrameWorld,
    facts: &FactsTable,
    id: ClientId,
    ps: &mut PlayerState,
    t6: &T6WeaponPs,
) {
    for h in t6.held {
        let w = h.weapon;
        // A left-hand weapon's clip goes out with its dual gun's row below.
        if w == 0 || is_left_hand_weapon(facts, w) {
            continue;
        }
        let Some((ammo, clip)) = ammo_keys(world, w) else {
            continue;
        };
        // Remapped reads: a weapon that shares its owner's magazine or reserve shows that count.
        let stock = t6.ammo_not_in_clip(facts, w);
        let in_clip = t6.ammo_in_clip(facts, w);
        if ammo_row_present(&ps.ammo, ammo) {
            set_ammo_not_in_clip(&mut ps.ammo, ammo, stock);
        }
        if clip_row_present(&ps.ammoclip, clip) {
            set_clip_for_hand(&mut ps.ammoclip, clip, 0, in_clip);
            let left = dual_wield_weapon(facts, w);
            if left != 0 {
                set_clip_for_hand(&mut ps.ammoclip, clip, 1, t6.ammo_in_clip(facts, left));
            }
        }
        world.client_meta_mut(id).set_ammo(w, in_clip, stock);
    }
    world.client_meta_mut(id).mirror_held_ammo(t6.weapon);
}

/// legacy pm types for noclip and ufo (fly) moves.
const PM_TYPE_NOCLIP: i32 = 2;
const PM_TYPE_UFO: i32 = 3;

/// PmoveSingle runs PM_AdjustAimSpreadScale before the weapon (BO2): turning, moving and
/// jumping grow the spread scale, else it decays.
fn adjust_spread(
    ps: &mut PlayerState,
    gun: &mut T6Gun,
    facts: &FactsTable,
    cmd: &UserCmd,
    old_angles: [i32; 3],
    msec: i32,
    dvars: &RecoilDvars,
) {
    let mut ov = SpreadOverride {
        value: ps.spread_override,
        state: ps.spread_override_state,
    };
    let dead_or_free = matches!(
        ps.pm_type,
        playerstate_t6::game::PM_TYPE_DEAD | playerstate_t6::game::PM_TYPE_DEAD_LINKED
    ) || matches!(ps.pm_type, PM_TYPE_NOCLIP | PM_TYPE_UFO);
    let input = AimSpreadInput {
        frametime: msec as f32 / 1000.0,
        in_air: ps.ground_entity_num == ENTITYNUM_NONE && !dead_or_free,
        prone: ps.pm_flags & gpmf::PRONE != 0,
        ducked: ps.pm_flags & gpmf::CROUCH != 0,
        frac: gun.ps.f_weapon_pos_frac,
        quick_scope_scale: gun.ps.quick_scope_scale,
        quick_scope_timer: gun.ps.quick_scope_timer,
        cmd_angles: [cmd.angles[0], cmd.angles[1]],
        old_cmd_angles: [old_angles[0], old_angles[1]],
        moving: cmd.forwardmove != 0 || cmd.rightmove != 0,
        velocity_xy: [ps.velocity[0], ps.velocity[1]],
        speed: ps.speed.max(1),
        vehicle: None,
    };
    let spread = facts.get(gun.ps.weapon).spread;
    adjust_aim_spread_scale(
        &mut gun.ps.aim_spread_scale,
        &mut ov,
        &spread,
        &input,
        dvars,
    );
    ps.spread_override_state = ov.state;
}

/// One T6 fire event -> the shot the bullet and projectile phases fire (the legacy path's
/// ShotAccepted steps: shot id, body anim, combat seed, everyone's fire event, script note).
#[allow(clippy::too_many_arguments)]
fn accept_shot(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    ps: &PlayerState,
    gun: &T6Gun,
    facts: &FactsTable,
    event: u16,
    shot: u32,
    spread_scale: f32,
    perks: u64,
    dvars: &RecoilDvars,
) -> Option<crate::combat::AcceptedShot> {
    let left = matches!(
        event,
        ev::EV_FIRE_WEAPON_LEFT | ev::EV_FIRE_WEAPON_LASTSHOT_LEFT
    );
    let last_shot = matches!(
        event,
        ev::EV_FIRE_WEAPON_LASTSHOT | ev::EV_FIRE_WEAPON_LASTSHOT_LEFT
    );
    let weapon = if left {
        facts.get(gun.ps.weapon).core.dual_wield_weapon
    } else {
        gun.ps.weapon
    };
    if weapon == 0 {
        return None;
    }
    let wf = facts.get(weapon);
    let hand = u8::from(left);
    let shot_id = world.alloc_shot_id();
    let combat_seed = world.combat_rng_mut().next_u32();
    let origin = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    world.push_entity_event(
        tick,
        crate::match_state::EventAudience::All,
        entity_t6::predicted_weapon_fire_event(i32::from(hand), last_shot),
        crate::EntityEventPayload {
            number: id.0 as i32,
            weapon,
            correlation: shot_id.0,
            origin,
            direction: ps.viewangles,
            ..Default::default()
        },
    );
    world
        .weapon_notes
        .push(crate::equipment::WeaponNote::Fired { owner: id });

    let frac = gun.ps.f_weapon_pos_frac.clamp(0.0, 1.0);
    let (min, max) = spread_for_weapon(
        &wf.spread,
        &SpreadInput {
            view_height: ps.view_height_current,
            override_: SpreadOverride {
                value: ps.spread_override,
                state: ps.spread_override_state,
            },
            stacked: wf.core.fire_type == fire_type::STACKED,
            clip_size: wf.core.clip_size,
            player_perks: perks,
            zombies: false,
            frac,
        },
        dvars,
    );
    // A BO2 shot is aimed with the spread before its own fire add widened it.
    let spread_degrees = bullet_cone(&wf.spread, spread_scale, min, max, frac);
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "t6 shot: weapon {weapon} hand {hand} ads {frac:.2} scale {spread_scale:.0} cone [{min:.2}, {max:.2}] spread {spread_degrees:.2}"
        );
    }
    let life = world
        .client_meta(id)
        .map(|m| m.life_sequence)
        .unwrap_or_default();
    // A stacked gun fans its stack's bullets across its stack spread (yaw only).
    let mut angles = ps.viewangles;
    if wf.core.fire_type == fire_type::STACKED {
        angles[1] += stack_fire_yaw(
            wf.spread.stack_fire_spread,
            wf.core.clip_size,
            shot,
            gun.ps.weapon_shot_count,
        );
    }
    // Bullet_FirePenetrate: the bullet-penetration perk, the player's or one the gun in the
    // arms grants (FMJ), and a charged shot's level scale how deep the bullet goes.
    let pen_perk = perks & (1u64 << perk::BULLETPENETRATION) != 0
        || weapon_t6::script::weapon_grants_perk(&gun.ps, facts, perk::BULLETPENETRATION);
    let charge_level =
        (wf.core.fire_type == fire_type::CHARGESHOT).then_some(gun.ps.charge_shot_level);
    Some(crate::combat::AcceptedShot {
        shot_id,
        attacker: id,
        attacker_life: life,
        hand,
        weapon,
        ammo_used: 1,
        origin,
        angles,
        ads_frac: frac,
        view_height_current: ps.view_height_current,
        aim_spread_scale: spread_scale,
        perks0: ps.perks[0],
        combat_seed,
        owner_velocity: ps.velocity,
        spread_degrees,
        pen_scale: Some(bullet_pen_scale(pen_perk, charge_level)),
    })
}

/// perk_bulletPenetrationMultiplier.
pub const PERK_BULLET_PENETRATION_MULTIPLIER: f32 = 2.0;
/// bg_chargeShotPenetrationMultiplier.
pub const CHARGE_SHOT_PENETRATION_MULTIPLIER: f32 = 1.0;

/// Bullet_FirePenetrate's depth scale: the bullet-penetration perk sets it to
/// perk_bulletPenetrationMultiplier, and a charged shot past level 1 adds its level times
/// bg_chargeShotPenetrationMultiplier (`charge_level` is `None` for a gun that does not
/// charge). A scale above zero multiplies every depth; zero leaves them as they are.
pub(crate) fn bullet_pen_scale(pen_perk: bool, charge_level: Option<i32>) -> f32 {
    let mut scale = if pen_perk {
        PERK_BULLET_PENETRATION_MULTIPLIER
    } else {
        0.0
    };
    if let Some(level) = charge_level.filter(|&l| l > 1) {
        scale += level as f32 * CHARGE_SHOT_PENETRATION_MULTIPLIER;
    }
    scale
}

/// NOAMMO parm bits (window 2): the left gun's event, and no click (a clip-only gun or one that
/// stays up when empty). The event still goes out silent: the client's out-of-ammo switch
/// (`out_of_ammo_pick`) runs on it, and BO2 switches a clip-only gun too.
pub const NOAMMO_PARM_LEFT: i32 = 1;
pub const NOAMMO_PARM_SILENT: i32 = 2;

/// The T6 fire-side events the legacy path sends as predictable player events (dry-fire click,
/// bolt, brass, thrown-weapon pull-back): the legacy event and parm, or None. legacy has no left
/// dry-fire event; the left gun's click is the same gun's. T6 only ejects brass at the bolt
/// step, and bolt guns are never dual, so its brass is the right hand's. The client plays the
/// click for every gun, so the gun in hand's rule goes here: no click for a clip-only gun or one
/// that stays up when empty (BO1's CG_EntityEvent) goes out as the silent bit. Not sent (no legacy event or HUD hint yet):
/// can't plant, the lock-on and target hints, the charge-shot pair.
fn fire_sound_event(e: &ev::Event, facts: &FactsTable, weapon: u32) -> Option<(i32, i32)> {
    use entity_t6::EntityEventKind as K;
    let (kind, parm) = match e.id {
        ev::EV_NOAMMO | ev::EV_NOAMMO_LEFT => {
            let f = facts.get(weapon);
            let mut parm = if e.id == ev::EV_NOAMMO_LEFT {
                NOAMMO_PARM_LEFT
            } else {
                0
            };
            if f.core.clip_only || f.switch.cancel_auto_holster_when_empty {
                parm |= NOAMMO_PARM_SILENT;
            }
            (K::NOAMMO, parm)
        }
        ev::EV_RECHAMBER_WEAPON => (K::RECHAMBER_WEAPON, 0),
        ev::EV_EJECT_BRASS => (K::EJECT_BRASS, 0),
        ev::EV_PULLBACK_WEAPON => (K::PULLBACK_WEAPON, e.parm as i32),
        _ => return None,
    };
    Some((kind.0, parm))
}

/// The T6 melee event became a swing that lands now: tell everyone, then (server only) trace
/// the hit with the swing weapon's damage and reach. The swing weapon is the held weapon when
/// it is use-as-melee, else the player's melee weapon (the knife).
fn melee_hit(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    ps: &PlayerState,
    gun: &T6Gun,
    facts: &FactsTable,
    env: &T6Env,
) {
    let weapon = if facts.get(gun.ps.weapon).melee.use_as_melee {
        gun.ps.weapon
    } else {
        gun.ps.melee_weapon
    };
    if weapon == 0 {
        return;
    }
    let wf = facts.get(weapon);
    let origin = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    world.push_entity_event(
        tick,
        crate::match_state::EventAudience::All,
        entity_t6::EntityEventKind::FIRE_MELEE,
        crate::EntityEventPayload {
            number: id.0 as i32,
            weapon,
            origin,
            direction: ps.viewangles,
            ..Default::default()
        },
    );
    if !world.publishes_snapshot() {
        return;
    }
    let Some(life) = world.client_meta(id).map(|m| m.life_sequence) else {
        return;
    };
    let swing = crate::combat::MeleeSwing {
        damage: wf.melee.damage,
        range: wf.melee.reach(&env.dvars),
        knife: wf.core.weap_type != weapon_t6::facts::weap_type::RIOTSHIELD,
    };
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "t6 melee: weapon {weapon} damage {} range {:.0}",
            swing.damage,
            swing.range
        );
    }
    crate::combat::fire_melee_hit(world, tick, id, life, weapon, origin, ps.viewangles, swing);
}

/// The lunge target in the command: the player's own pick, or for a bot (no client to pick)
/// the server's.
fn lunge_cmd(
    world: &mut FrameWorld,
    id: ClientId,
    cmd: &UserCmd,
    ps: &PlayerState,
    facts: &FactsTable,
) -> (Option<u16>, u8) {
    let bot = world.publishes_snapshot()
        && world
            .ecs()
            .get_resource::<crate::t6::Zm>()
            .is_some_and(|zm| zm.players.get(&id.0).is_some_and(|p| p.test_client));
    if cmd.melee_charge_ent.is_some()
        || !bot
        || cmd.buttons & playerstate_t6::game::buttons::MELEE_CHARGE == 0
    {
        return (cmd.melee_charge_ent, cmd.melee_charge_dist);
    }
    crate::t6_lunge::bot_pick(world, id, ps, facts, false)
        .map_or((None, 0), |p| (Some(p.ent), p.dist))
}

/// A lunge the weapon tick started goes to the move (which launches it next move); one the
/// tick ended stops.
fn mirror_lunge(ps: &mut PlayerState, t6_flags: u32, t6: &T6WeaponPs) {
    let lunging = t6_flags & pmf::MELEE_CHARGE != 0;
    let moving = ps.pm_flags & gpmf::MELEE_CHARGE != 0;
    if lunging && !moving {
        ps.pm_flags |= gpmf::MELEE_CHARGE;
        ps.melee_charge_dist = t6.melee_charge_dist as i32;
        ps.melee_charge_time = 0;
    } else if !lunging && moving {
        movement_t6::bridge::bo2_lunge_clear(ps);
    }
}

/// The trace's lunge column while one runs: time left, distance sent, flat speed.
fn lunge_trace(ps: &PlayerState) -> String {
    if ps.pm_flags & gpmf::MELEE_CHARGE == 0 {
        return String::new();
    }
    let v = ps.velocity;
    format!(
        " | lunge t{} d{} v{:.0}",
        ps.melee_charge_time,
        ps.melee_charge_dist,
        (v[0] * v[0] + v[1] * v[1]).sqrt()
    )
}

/// A running lunge's target and the swing's time to its hit, for the move.
pub(crate) fn lunge_state(world: &FrameWorld, id: ClientId) -> (Option<u16>, i32) {
    world
        .client_meta(id)
        .and_then(|m| m.t6_gun.as_deref())
        .map_or((None, 0), |g| (g.ps.melee_charge_ent, g.ps.weapon_delay.max(0)))
}

/// Switching to a riot shield (BO2's weapFlags shield-switch bit): he does not flinch.
pub(crate) fn riotshield_switching(world: &FrameWorld, id: ClientId) -> bool {
    world
        .client_meta(id)
        .and_then(|m| m.t6_gun.as_deref())
        .is_some_and(|g| g.ps.weap_flags & weapon_t6::player::wf::RIOTSHIELD_SWITCH != 0)
}

/// The weapons BO2's body anims read: the offhand item while one is in use (BG_GetPlayerStateWeapon
/// reads it in place of the weapon in hand), and the one he is switching to
/// (BG_GetNextPlayerStateWeapon: that offhand, else the usercmd's weapon). None without the brain.
pub(crate) fn anim_weapons(world: &FrameWorld, id: ClientId) -> Option<(Option<u32>, u32)> {
    let g = world.client_meta(id)?.t6_gun.as_deref()?;
    let offhand = (g.ps.weap_flags & weapon_t6::player::wf::USING_OFFHAND != 0
        && g.ps.off_hand_weapon != 0)
        .then_some(g.ps.off_hand_weapon);
    let cmd = if g.cmd_weapon != 0 {
        g.cmd_weapon
    } else {
        g.ps.weapon
    };
    Some((offhand, offhand.unwrap_or(cmd)))
}

/// BO2's PM_IsAdsAllowed for the gun in his hand, asked by the move before the gun's tick:
/// BO2 answers it once per Pmove (PM_UpdateAimDownSightFlag, before the sprint and walk), so
/// the move's aim and the gun's aim are one answer. None without the brain.
pub(crate) fn ads_allowed(world: &FrameWorld, id: ClientId, ps: &PlayerState) -> Option<bool> {
    let g = world.client_meta(id)?.t6_gun.as_deref()?;
    let facts = world.gun_facts_shared();
    let mut gun_ps = g.ps.clone();
    sync_in(world, &facts, ps, &mut gun_ps);
    let mut env = T6Env {
        pm_type: ps.pm_type,
        pm_flags: pm_flags(ps.pm_flags) | kept_pm_flags(ps, g.pm_flags),
        e_flags: ps.e_flags,
        e_flags2: g.e_flags2 & KEPT_E_FLAGS2,
        other_flags: weapon_t6::env::of::WEAPON_USABLE,
        waterlevel: ps.waterlevel,
        ..T6Env::default()
    };
    mantle_env(ps, &mut env);
    let cmd = T6Cmd::default();
    let mut out = weapon_t6::pm::FrameOut::default();
    let pm = weapon_t6::pm::Pm {
        ps: &mut gun_ps,
        cmd: &cmd,
        env: &mut env,
        facts: &facts,
        out: &mut out,
        delay_fired: false,
    };
    Some(weapon_t6::pm::ads::is_ads_allowed(&pm))
}

/// One weapon tick by the T6 brain; its fire events come back as accepted shots.
pub(crate) fn advance(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    cmd: &UserCmd,
    msec: i32,
) -> Vec<crate::combat::AcceptedShot> {
    let Some(mut ps) = world.player(id).copied() else {
        return Vec::new();
    };
    let mut gun = world
        .client_meta(id)
        .and_then(|m| m.t6_gun.as_deref().cloned())
        .unwrap_or_else(T6Gun::new);
    // ClientThink_real: a new press of BO2's double-tap button (two quick Use/Reload taps on
    // the client) tells the script "doubletap_detonate" (C4's alt detonate), before the move.
    let old_wire = world.old_buttons(id).unwrap_or(0);
    if cmd.buttons & !old_wire & playerstate_t6::game::buttons::DOUBLE_TAP != 0 && gun.double_taps.len() < KEPT_EVENTS {
        gun.double_taps.push(gun.events.len());
    }
    let perks = world.client_meta(id).map_or(0, |m| m.t6_perks);
    let facts = world.gun_facts_shared();
    sync_in(world, &facts, &ps, &mut gun.ps);
    // The first move of a server frame sees the scale every move of the last frame left behind.
    if gun.frame_tick != Some(tick) {
        gun.frame_tick = Some(tick);
        gun.frame_aim_spread_scale = gun.ps.aim_spread_scale;
    }
    let spread_scale = gun.frame_aim_spread_scale;
    let dvars = RecoilDvars::default();
    let old_angles = world.old_cmd_angles(id).unwrap_or(cmd.angles);
    adjust_spread(&mut ps, &mut gun, &facts, cmd, old_angles, msec, &dvars);
    // One lunge for both sides (BO2 keeps it in one player state): ended by the move, ended here.
    if ps.pm_flags & gpmf::MELEE_CHARGE == 0 {
        gun.ps.melee_charge_ent = None;
        gun.ps.melee_charge_dist = 0;
        gun.ps.melee_charge_time = 0;
    }
    let (charge_ent, charge_dist) = lunge_cmd(world, id, cmd, &ps, &facts);

    let script_trigger = crate::t6::mp_specials::owns_trigger(world.weapon_script_name(ps.weapon as u32));
    // SV_UserMove: a weapon in his command that he does not hold (or none) reads as the one in
    // his hand (BG_IsWeaponValid), so a stray command never puts his gun away.
    let cmd_weapon = match u32::from(cmd.weapon) {
        w if gun.ps.is_weapon_valid(w, facts.len()) => w,
        _ => gun.ps.weapon,
    };
    let t6cmd = T6Cmd {
        msec,
        server_time: cmd.server_time,
        weapon: cmd_weapon,
        last_weapon_alt_mode_switch: weapon_t6::switch::ClientSwitch::cmd_alt_memory(
            &gun.ps,
            &facts,
            cmd_weapon,
        ),
        buttons: gun_buttons(cmd.buttons, script_trigger),
        old_buttons: gun_buttons(world.old_buttons(id).unwrap_or(0), script_trigger),
        melee_charge_ent: charge_ent,
        melee_charge_dist: charge_dist,
        forwardmove: cmd.forwardmove,
        rightmove: cmd.rightmove,
        last_input: cmd.last_input,
        ..T6Cmd::default()
    };
    gun.cmd_weapon = t6cmd.weapon;
    if crate::t6::plant::entity_blocks_tac_insert(world, &ps, &gun.ps, &facts) {
        gun.ps.weap_flags |= weapon_t6::player::wf::RIOTSHIELD_CANT_PLACE;
    }
    let (plant_spot_ok, tac_insert_allowed) =
        crate::t6::plant::answers(world, &ps, &gun.ps, &facts, t6cmd.buttons, pm_flags(ps.pm_flags));
    let mut env = T6Env {
        pm_type: ps.pm_type,
        pm_flags: pm_flags(ps.pm_flags) | kept_pm_flags(&ps, gun.pm_flags),
        e_flags: ps.e_flags,
        e_flags2: gun.e_flags2 & KEPT_E_FLAGS2,
        on_ground: ps.ground_entity_num != ENTITYNUM_NONE,
        prone_change: (pm_flags(ps.pm_flags) ^ gun.pm_flags) & pmf::PRONE != 0,
        velocity_z: ps.velocity[2],
        // Both movement brains land viewHeightCurrent exactly on the target, as BO2's does.
        view_height_settled: ps.view_height_current == ps.view_height_target as f32,
        shellshock_anim: ps.pm_flags & gpmf::SHELLSHOCKED != 0
            && world
                .client_meta(id)
                .and_then(|m| m.shellshock.as_ref())
                .is_some_and(|s| s.animation),
        plant_spot_ok,
        tac_insert_allowed,
        riotshield_blocked: crate::t6::riotshield::blocked(world, &ps, &gun.ps, &facts),
        other_flags: weapon_t6::env::of::WEAPON_USABLE,
        is_server: world.publishes_snapshot(),
        perks,
        rapid_fire: ps.perks[1] & crate::t6::menu::PERK_MENU_RAPID_FIRE != 0,
        jump_time: dtp_jump_time(&ps),
        last_dtp_end: ps.dive_end_time,
        // When the player first stood on slide ground (BO2's slideTime, set by its movement):
        // the gun plays SLIDE_IN on that tick.
        slide_time: ps.slide_time,
        // Movement's crawl put the gun to rest: it set BO2's state to ready under the gun
        // (movement_t6 feel.rs).
        crawl_idle: ps.weapon_state_t6 == weapon_t6::state::READY
            && gun.ps.weaponstate != weapon_t6::state::READY,
        // How deep the player stands, from movement's water check this tick.
        waterlevel: ps.waterlevel,
        ..T6Env::default()
    };
    mantle_env(&ps, &mut env);
    let mut out = tick::pre_weapon(&mut gun.ps, &t6cmd, &mut env, &facts);
    out.append(tick::run(&mut gun.ps, &t6cmd, &mut env, &facts));
    gun.pm_flags = env.pm_flags;
    gun.e_flags2 = env.e_flags2;
    share_prone_override(&mut ps, env.pm_flags);
    if world.publishes_snapshot() {
        gun.third.extend(out.scripts.iter().copied());
    }

    let t6 = &gun.ps;
    ps.weapon = t6.weapon;
    ps.weapon_primary = primary_of(&facts, t6.weapon);
    // BO2's own number, for movement's rules (movement_t6 gun.rs).
    ps.weapon_state_t6 = t6.weaponstate;
    ps.last_weapon_hand = hands_drawn(&facts, t6.weapon);
    ps.weapon_time = t6.weapon_time;
    ps.weapon_delay = t6.weapon_delay;
    // BO2's own numbers for both hands.
    ps.weap_anim_t6 = t6.weap_anim;
    ps.weapon_state_left_t6 = t6.weaponstate_left;
    ps.weap_anim_left_t6 = t6.weap_anim_left;
    // The T6 ADS lerp and spread own these (movement skips its own ADS lerp under the brain).
    ps.f_weapon_pos_frac = t6.f_weapon_pos_frac;
    ps.aim_spread_scale = t6.aim_spread_scale;
    if anim::number(t6.weap_anim_left) != anim::LEFT_NONE {
        ps.weapon_time_secondary = t6.weapon_time_left;
        ps.weapon_delay_secondary = t6.weapon_delay_left;
    }
    mirror_offhand_view(&mut ps, t6);
    sync_out(world, &facts, id, &mut ps, t6);
    mirror_lunge(&mut ps, env.pm_flags, t6);
    if let Some(slot) = world.player_mut(id) {
        *slot = ps;
    }

    if let Some(file) = trace_file() {
        let line = weapon_t6::trace::frame_line(&facts, cmd.server_time, &gun.ps, &out);
        // The aim spread is the game's own step (the runner has none), so it rides at the end
        // where a runner-vs-game compare can cut it off.
        file.line(format!(
            "c{} {line} | spread {:.1}{}",
            id.0,
            gun.ps.aim_spread_scale,
            lunge_trace(&ps)
        ));
    }
    gun.events.extend_from_slice(&out.events);
    let extra = gun.events.len().saturating_sub(KEPT_EVENTS);
    gun.events.drain(..extra);
    for at in &mut gun.double_taps {
        *at = at.saturating_sub(extra);
    }
    let mut shots = Vec::new();
    for e in &out.events {
        if (ev::EV_FIRE_WEAPON..=ev::EV_FIRE_WEAPON_LASTSHOT_LEFT).contains(&e.id)
            && let Some(shot) = accept_shot(
                world,
                tick,
                id,
                &ps,
                &gun,
                &facts,
                e.id,
                e.parm,
                spread_scale,
                perks,
                &dvars,
            )
        {
            shots.push(shot);
        }
        if let Some((event, parm)) = fire_sound_event(e, &facts, gun.ps.weapon)
            && let Some(slot) = world.player_mut(id)
        {
            movement_t6::bridge::add_predictable_event(slot, event, parm);
        }
        match e.id {
            ev::EV_MELEE_SWIPE => {
                if let Some(slot) = world.player_mut(id) {
                    movement_t6::bridge::add_predictable_event(
                        slot,
                        entity_t6::EntityEventKind::MELEE_SWIPE.0,
                        0,
                    );
                }
            }
            ev::EV_FIRE_MELEE => melee_hit(world, tick, id, &ps, &gun, &facts, &env),
            // The grenade leaves the hand. The brain has spent the round already; a cooked
            // grenade keeps what is left of its fuse, any other starts its own.
            ev::EV_USE_OFFHAND => {
                let w = e.parm;
                let fuse = facts
                    .get(w)
                    .offhand
                    .cook_off_hold
                    .then_some(gun.ps.grenade_time_left);
                if crate::equipment::spawn_offhand_projectile(world, id, w, tick, fuse) {
                    let origin = [
                        ps.origin[0],
                        ps.origin[1],
                        ps.origin[2] + ps.view_height_current,
                    ];
                    world.push_entity_event(
                        tick,
                        crate::match_state::EventAudience::All,
                        entity_t6::EntityEventKind::USE_OFFHAND,
                        crate::EntityEventPayload {
                            number: id.0 as i32,
                            weapon: w,
                            origin,
                            event_parm: w as i32,
                            ..Default::default()
                        },
                    );
                }
            }
            // Held past its fuse: BO2 MP kills the holder (its thrower's kill when thrown back),
            // and the death lets the grenade go with its spent fuse, so it blows up on them.
            ev::EV_GRENADE_SUICIDE => {
                let thrower = gun.ps.throw_back_grenade_owner.map(u32::from);
                world.client_meta_mut(id).t6_gun = Some(Box::new(gun.clone()));
                if !crate::t6::grenade_suicide(world.ecs(), tick, id.0, thrower, e.parm) {
                    crate::script::grenade_suicide(world.ecs(), tick, id.0, thrower, e.parm);
                }
            }
            _ => {}
        }
    }
    let world_publishes = world.publishes_snapshot();
    let meta = world.client_meta_mut(id);
    if !world_publishes {
        // Client prediction: a replayed move takes the place of the one it repeats.
        meta.t6_gun_history.retain(|(t, _)| *t < cmd.server_time);
        if meta.t6_gun_history.len() >= T6_GUN_HISTORY {
            meta.t6_gun_history.remove(0);
        }
        meta.t6_gun_history
            .push((cmd.server_time, Box::new(gun.clone())));
    }
    meta.t6_gun = Some(Box::new(gun));
    shots
}

/// BO2's death grenade drop, run on every death: a player who dies with a grenade's fuse running
/// in their hand lets it go (an equipment-slot item and a killstreak item never drop). The fuse is
/// spent on the way out, so nothing drops twice.
pub(crate) fn death_grenade_drop(world: &mut FrameWorld, tick: Tick, id: ClientId) {
    if !world.publishes_snapshot() {
        return;
    }
    let Some(gun) = world.client_meta_mut(id).t6_gun.as_deref_mut() else {
        return;
    };
    let fuse = gun.ps.grenade_time_left;
    if fuse == 0 {
        return;
    }
    gun.ps.grenade_time_left = 0;
    let weapon = if gun.ps.weap_flags & weapon_t6::player::wf::USING_OFFHAND != 0 {
        gun.ps.off_hand_weapon
    } else {
        gun.ps.weapon
    };
    if weapon == 0 {
        return;
    }
    let facts = world.gun_facts_shared().get(weapon);
    if facts.core.offhand_slot == weapon_t6::facts::offhand_slot::EQUIPMENT
        || facts.core.inventory_type == inventory::ITEM
    {
        return;
    }
    crate::equipment::drop_grenade_on_death(world, id, weapon, tick, fuse);
}

/// Predicted moves the client keeps the brain for (the move history holds no more).
const T6_GUN_HISTORY: usize = 128;

/// Client prediction, on a snapshot: before the unacknowledged moves replay, the brain goes back
/// to where it stood after the move the server last ran, with the server's own gun values on top.
/// BO2 sends the weapon state in the player state, so its client restarts from the server's;
/// without this a replayed move would run the gun's timers twice.
pub(crate) fn rewind(meta: &mut crate::match_state::ClientMatchState, ps: &PlayerState) {
    let acked = ps.command_time;
    meta.t6_gun_history.retain(|(t, _)| *t >= acked);
    let Some(mut gun) = meta
        .t6_gun_history
        .iter()
        .find(|(t, _)| *t == acked)
        .map(|(_, g)| g.clone())
        .or_else(|| meta.t6_gun.clone())
    else {
        return;
    };
    if let Some(v) = meta.t6_view {
        gun.ps.weapon = ps.weapon;
        gun.ps.weap_anim = v.weap_anim;
        gun.ps.weap_anim_left = v.weap_anim_left;
        gun.ps.weaponstate = v.weaponstate;
        gun.ps.weaponstate_left = v.weaponstate_left;
        gun.ps.weapon_time = v.weapon_time;
        gun.ps.weapon_time_left = v.weapon_time_left;
        gun.ps.intro_shots_fired = v.intro_shots_fired;
        // BO2 sends the raised-before bits whole, so a gun the script just gave (or cleared
        // with initialweaponraise) picks the server's first raise.
        for (i, &w) in ps.weapons.iter().enumerate() {
            if w > 0 {
                gun.ps.give(w as u32);
                gun.ps.set_ever_had(w as u32, v.ever_had & 1 << i != 0);
                if let Some(h) = gun.ps.held_mut(w as u32) {
                    h.needs_rechamber = v.needs_rechamber & 1 << i != 0;
                }
            }
        }
        hold_in_game_slots(&mut gun.ps, &ps.weapons);
        // Window 5: the offhand in hand and whether it shows, as the server has them (BO2 sends
        // both). Without them a client that missed the press drew nothing through a frag pull
        // and hold, and one that picked another grenade drew that grenade's pull.
        use playerstate_t6::game::weap_flags::OFFHAND_VIEW;
        use weapon_t6::player::wf::USING_OFFHAND;
        let off = u32::try_from(ps.off_hand_index).unwrap_or(0);
        if off != 0 && gun.ps.has_weapon(off) {
            gun.ps.off_hand_weapon = off;
        }
        if ps.weap_flags & OFFHAND_VIEW != 0 {
            gun.ps.weap_flags |= USING_OFFHAND;
        } else {
            gun.ps.weap_flags &= !USING_OFFHAND;
        }
    }
    // The server's spread too (a re-given gun's first raise resets it to the max).
    gun.ps.aim_spread_scale = ps.aim_spread_scale;
    meta.t6_gun = Some(gun);
}

/// the legacy weapon_primary <- T6's gun in hand: an alt mode's base gun (the client's alt key goes back
/// to it), else the gun itself.
fn primary_of(facts: &FactsTable, w: u32) -> u32 {
    if w == 0 {
        return 0;
    }
    let c = facts.get(w).core;
    if c.inventory_type == inventory::ALTMODE && c.alt_weapon != 0 {
        c.alt_weapon
    } else {
        w
    }
}

/// Window 14: the third-person anim script events the T6 brain fired since the last call
/// (drained), and its weaponstate now. None = no T6 state for this player.
pub(crate) fn take_third(
    world: &mut FrameWorld,
    id: ClientId,
) -> Option<(Vec<weapon_t6::events::AnimScript>, i32)> {
    let gun = world.client_meta_mut(id).t6_gun.as_deref_mut()?;
    Some((std::mem::take(&mut gun.third), gun.ps.weaponstate))
}

/// legacy gun in hand and its state <- T6.
fn mirror_hand(ps: &mut PlayerState, t6: &T6WeaponPs, facts: &FactsTable) {
    ps.weapon = t6.weapon;
    ps.weapon_primary = primary_of(facts, t6.weapon);
    // BO2's own number, for movement's rules (movement_t6 gun.rs).
    ps.weapon_state_t6 = t6.weaponstate;
    ps.last_weapon_hand = hands_drawn(facts, t6.weapon);
    mirror_offhand_view(ps, t6);
}

/// The view's "left gun drawn" flag (the legacy akimbo hand count) <- T6. BO2 draws the left
/// viewmodel, its flash and its brass whenever the gun in hand is a dual wield weapon.
fn hands_drawn(facts: &FactsTable, weapon: u32) -> i32 {
    i32::from(weapon_t6::pm::dual::is_dual_wield(facts, weapon))
}

/// legacy offhand view <- T6. BO2's view draws the offhand item while one is in hand
/// (PWF_USING_OFFHAND), the same bit the legacy viewmodel reads; without it the view showed no arm
/// and no grenade through a grenade hold or a pin pull.
fn mirror_offhand_view(ps: &mut PlayerState, t6: &T6WeaponPs) {
    use playerstate_t6::game::weap_flags::OFFHAND_VIEW;
    if t6.weap_flags & weapon_t6::player::wf::USING_OFFHAND != 0 {
        ps.weap_flags |= OFFHAND_VIEW;
        ps.off_hand_index = t6.off_hand_weapon as i32;
    } else {
        ps.weap_flags &= !OFFHAND_VIEW;
    }
}

/// The script side (window 13): a GSC builtin acting on a player's T6 state between usercmds.
/// The state is made (from the legacy one) when no usercmd has run yet; the held weapons and ammo
/// come in from the legacy state first, since other builtins (setweaponammostock) still write there;
/// afterwards the gun in hand and the ammo go back to the legacy state, as a tick leaves them.
pub(crate) fn script_edit<R>(
    world: &mut FrameWorld,
    id: ClientId,
    f: impl FnOnce(&mut T6WeaponPs, &FactsTable, &mut u32) -> R,
) -> Option<R> {
    let mut ps = world.player(id).copied()?;
    world.client_meta(id)?;
    let facts = world.gun_facts_shared();
    let mut gun = match world.client_meta_mut(id).t6_gun.take() {
        Some(g) => *g,
        None => {
            let mut g = T6Gun::new();
            g.ps.weapon = ps.weapon;
            g
        }
    };
    sync_in(world, &facts, &ps, &mut gun.ps);
    let r = f(&mut gun.ps, &facts, &mut gun.pm_flags);
    mirror_hand(&mut ps, &gun.ps, &facts);
    sync_out(world, &facts, id, &mut ps, &gun.ps);
    if let Some(slot) = world.player_mut(id) {
        *slot = ps;
    }
    world.client_meta_mut(id).t6_gun = Some(Box::new(gun));
    Some(r)
}

/// The script side: read a player's T6 state (None before his first usercmd).
pub(crate) fn script_read<R>(
    world: &FrameWorld,
    id: ClientId,
    f: impl FnOnce(&T6WeaponPs, &FactsTable) -> R,
) -> Option<R> {
    let gun = world.client_meta(id)?.t6_gun.as_deref()?;
    Some(f(&gun.ps, world.gun_facts()))
}

/// A copy of a player's T6 state with the held weapons and ammo synced in from the legacy state, as
/// a builtin sees it (None before his first usercmd).
pub(crate) fn synced_copy(world: &FrameWorld, id: ClientId) -> Option<T6WeaponPs> {
    let ps = world.player(id)?;
    let mut t6 = world.client_meta(id)?.t6_gun.as_deref()?.ps;
    sync_in(world, world.gun_facts(), ps, &mut t6);
    Some(t6)
}

/// The script side: the client events since the last call, in order.
pub(crate) fn script_take_events(
    world: &mut FrameWorld,
    id: ClientId,
) -> Vec<weapon_t6::events::Event> {
    world
        .client_meta_mut(id)
        .t6_gun
        .as_mut()
        .map_or_else(Vec::new, |g| std::mem::take(&mut g.events))
}

/// The script side: where each double-tap press fell among the events [`script_take_events`]
/// hands over; take both together.
pub(crate) fn script_take_double_taps(world: &mut FrameWorld, id: ClientId) -> Vec<usize> {
    world
        .client_meta_mut(id)
        .t6_gun
        .as_mut()
        .map_or_else(Vec::new, |g| std::mem::take(&mut g.double_taps))
}

/// BO2's out-of-ammo switch (window 2, `ClientSwitch::on_no_ammo`): the gun the client picks
/// after the local player's NOAMMO. `selected` / `last_primary` are the client's weapon pick,
/// `left` the left gun's event. `Some(0)` = no weapon; None = stay (or the T6 brain is off).
pub fn out_of_ammo_pick(
    world: &crate::SimWorld,
    id: ClientId,
    selected: u32,
    last_primary: u32,
    last_mode: &[u32; 15],
    left: bool,
) -> Option<u32> {
    if world.gun_facts().is_empty() {
        return None;
    }
    let ps = world.player(id)?;
    let gun = world.client_meta(id).and_then(|m| m.t6_gun.as_deref());
    let mut t6 = gun.map_or_else(T6WeaponPs::default, |g| g.ps.clone());
    let facts = world.gun_facts_shared();
    sync_in(world, &facts, ps, &mut t6);
    if gun.is_none() {
        t6.weapon = ps.weapon;
    }
    let env = T6Env {
        pm_type: ps.pm_type,
        e_flags2: gun.map_or(0, |g| g.e_flags2 & KEPT_E_FLAGS2),
        ..T6Env::default()
    };
    let mut pick = weapon_t6::switch::ClientSwitch {
        selected,
        last_primary,
        ..Default::default()
    };
    // The client's memory is kept by `ps.weapons` slot; the T6 player holds its guns in its own order.
    for (&w, &mode) in ps.weapons.iter().zip(last_mode) {
        if let Some(i) = t6.held_slot(w.max(0) as u32).filter(|_| w > 0 && mode != 0) {
            pick.last_mode[i] = mode;
        }
    }
    pick.on_no_ammo(&t6, &env, &facts, left)
}

fn alt_core(world: &crate::SimWorld, w: u32) -> Option<weapon_t6::facts::CoreFacts> {
    (w != 0 && !world.gun_facts().is_empty()).then(|| world.gun_facts().get(w).core)
}

/// BO2's weapon cycle from an alt mode (window 2, `ClientSwitch::cycle`): the base gun it goes back to,
/// or None when the cycle walks on (the mode disables switching: select fire, Hybrid; or not an alt mode).
pub fn alt_mode_cycle_base(world: &crate::SimWorld, selected: u32) -> Option<u32> {
    alt_core(world, selected)
        .filter(|c| c.inventory_type == inventory::ALTMODE && !c.alt_weapon_disable_switching)
        .map(|c| c.alt_weapon)
        .filter(|&b| b != 0)
}

/// The knife the melee key swings (BO2 weapon type melee, or the melee slot): BO2's wheel never
/// brings it up as the gun in hand. bo2mp, his 10-20 ask: the scroll walked onto knife_mp and he
/// held it like a gun, the old gun's name still on the screen.
pub fn is_melee_knife(world: &crate::SimWorld, w: u32) -> bool {
    alt_core(world, w).is_some_and(|c| {
        c.inventory_type == inventory::MELEE
            || c.weap_type == weapon_t6::facts::weap_type::MELEE && !world.gun_facts().get(w).melee.use_as_melee
    })
}

/// BO2's CG_AltToggleAllowedByState: the attachment key does nothing while the gun comes up from or
/// goes down for a mode switch or a swap (BO2 states 2-5), knifes or handles an offhand item (19-29),
/// on a charge-shot gun, or mid-climb. A state of -1 (no T6 brain) passes the state test.
pub fn alt_toggle_allowed_by_state(world: &crate::SimWorld, ps: &PlayerState) -> bool {
    use weapon_t6::state as st;
    let busy = matches!(
        ps.weapon_state_t6,
        st::RAISING_ALTSWITCH..=st::DROPPING_ALTSWITCH | st::MELEE_INIT..=st::OFFHAND_END
    );
    let charge = alt_core(world, ps.weapon).is_some_and(|c| c.fire_type == fire_type::CHARGESHOT);
    !busy && !charge && ps.pm_flags & gpmf::MANTLE == 0
}

/// BO2's attachment key on a gun with no attachment mode (CG_AltWeaponToggleIndex): the key walks on to the
/// next held primary that has one, forward from `selected`'s slot, a gun with ammo first, and brings that mode
/// out (the launcher can swap from a sniper). Returns that gun; None when there is none or its mode disables
/// switching. `skip` drops guns that are never cycled to (scorestreaks).
pub fn alt_toggle_walk(
    world: &crate::SimWorld,
    weapons: &[i32; 15],
    selected: u32,
    skip: impl Fn(u32) -> bool,
    has_ammo: impl Fn(u32) -> bool,
) -> Option<u32> {
    let start = weapons
        .iter()
        .position(|&w| selected != 0 && w == selected as i32)
        .unwrap_or(0);
    let walk = |need_ammo: bool| {
        (1..15)
            .map(|step| weapons[(start + step) % 15])
            .filter(|&w| w > 0)
            .map(|w| w as u32)
            .find(|&w| {
                alt_core(world, w).is_some_and(|c| {
                    c.inventory_type == inventory::PRIMARY && c.alt_weapon != 0
                }) && !skip(w)
                    && (!need_ammo || has_ammo(w))
            })
    };
    walk(true)
        .or_else(|| walk(false))
        .filter(|&w| alt_core(world, w).is_some_and(|c| !c.alt_weapon_disable_switching))
}

/// The mode the cycle brings the held gun `w` up in: the alt mode it was last left in, else `w`
/// (BO2's CG_GetLastWeaponForAlt: only the gun's own alt counts). `last_mode` is by `weapons` slot.
pub fn remembered_mode(
    world: &crate::SimWorld,
    weapons: &[i32; 15],
    last_mode: &[u32; 15],
    w: u32,
) -> u32 {
    let Some(c) = alt_core(world, w) else {
        return w;
    };
    weapons
        .iter()
        .position(|&h| h == w as i32)
        .map(|i| last_mode[i])
        .filter(|&m| m != 0 && m == c.alt_weapon)
        .unwrap_or(w)
}

/// BO2's CG_UpdateLastWeaponForAlt after the client picks `new`: an alt mode is remembered on its base
/// gun's slot, any other pick forgets that gun's mode.
pub fn remember_mode(
    world: &crate::SimWorld,
    weapons: &[i32; 15],
    last_mode: &mut [u32; 15],
    new: u32,
) {
    let Some(c) = alt_core(world, new) else {
        return;
    };
    let (slot_of, mode) = if c.inventory_type == inventory::ALTMODE {
        (c.alt_weapon, new)
    } else {
        (new, 0)
    };
    if let Some(i) = weapons.iter().position(|&h| slot_of != 0 && h == slot_of as i32) {
        last_mode[i] = mode;
    }
}

/// The wire buttons the BO2 gun sees. Wire THROW is the hold-aim key's second bit
/// (`+speed_throw`), and the same value is the BO2 gun's change-zoom button, which no key sends
/// yet: it stays out, or aiming in would step a variable zoom scope. A guest wonder weapon (a Ray
/// Gun def whose trigger t6::mp_specials runs) fires no def shots.
fn gun_buttons(wire: u32, script_trigger: bool) -> u32 {
    let mut b = wire & !playerstate_t6::game::buttons::THROW;
    if script_trigger {
        b &= !playerstate_t6::game::buttons::ATTACK;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use entity_t6::EntityEventKind as K;

    #[test]
    fn the_penetration_perk_doubles_depth_and_a_charge_adds_its_level() {
        assert_eq!(bullet_pen_scale(false, None), 0.0);
        assert_eq!(bullet_pen_scale(true, None), 2.0);
        // A charge of 1 adds nothing; past it the level adds on, perk or not.
        assert_eq!(bullet_pen_scale(false, Some(1)), 0.0);
        assert_eq!(bullet_pen_scale(false, Some(3)), 3.0);
        assert_eq!(bullet_pen_scale(true, Some(3)), 5.0);
    }

    #[test]
    fn the_aim_key_never_reads_as_change_zoom() {
        use playerstate_t6::game::buttons as wire;
        use weapon_t6::cmd::button;
        let aim = wire::ADS | wire::THROW;
        assert_eq!(gun_buttons(aim, false) & button::CHANGE_ZOOM, 0);
        assert_eq!(gun_buttons(aim, false), button::ADS);
        assert_eq!(gun_buttons(wire::ATTACK | aim, true), button::ADS);
        assert_eq!(gun_buttons(wire::ATTACK, false), button::ATTACK);
    }

    fn e(id: u16, parm: u32) -> ev::Event {
        ev::Event {
            id,
            parm,
            predictable: false,
        }
    }

    /// Weapon 1 a plain gun, 2 clip-only, 3 stays up when empty.
    fn guns() -> FactsTable {
        use weapon_t6::facts::{CoreFacts, SwitchFacts, T6WeaponFacts};
        let mut t = FactsTable::new();
        t.set(1, "plain", T6WeaponFacts::default());
        let core = CoreFacts {
            clip_only: true,
            ..Default::default()
        };
        t.set(
            2,
            "cliponly",
            T6WeaponFacts {
                core,
                ..Default::default()
            },
        );
        let switch = SwitchFacts {
            cancel_auto_holster_when_empty: true,
            ..Default::default()
        };
        t.set(
            3,
            "staysup",
            T6WeaponFacts {
                switch,
                ..Default::default()
            },
        );
        t
    }

    fn sound(id: u16, parm: u32) -> Option<(i32, i32)> {
        fire_sound_event(&e(id, parm), &guns(), 1)
    }

    /// BO2 sends the held weapons' flags whole: on a snapshot the client's gun takes the server's
    /// "still to bolt" flag, so a missed or extra predicted shot never bolts at the wrong time.
    #[test]
    fn rewind_takes_the_servers_bolt_flag() {
        for (client, server) in [(true, false), (false, true)] {
            let mut gun = T6Gun::new();
            gun.ps.give(7);
            gun.ps.held_mut(7).unwrap().needs_rechamber = client;
            let mut meta = crate::match_state::ClientMatchState {
                t6_gun: Some(Box::new(gun)),
                t6_view: Some(crate::T6ViewInputs {
                    needs_rechamber: u16::from(server) << 2,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut ps = PlayerState::ZERO;
            ps.weapons[2] = 7;
            ps.weapon = 7;
            rewind(&mut meta, &ps);
            let held = meta.t6_gun.as_ref().unwrap().ps.held(7).copied().unwrap();
            assert_eq!(held.needs_rechamber, server);
        }
    }

    #[test]
    fn fire_side_events_become_the_game_player_events() {
        assert_eq!(sound(ev::EV_NOAMMO, 0), Some((K::NOAMMO.0, 0)));
        assert_eq!(
            sound(ev::EV_NOAMMO_LEFT, 0),
            Some((K::NOAMMO.0, NOAMMO_PARM_LEFT))
        );
        assert_eq!(
            sound(ev::EV_RECHAMBER_WEAPON, 0),
            Some((K::RECHAMBER_WEAPON.0, 0))
        );
        assert_eq!(sound(ev::EV_EJECT_BRASS, 0), Some((K::EJECT_BRASS.0, 0)));
        assert_eq!(
            sound(ev::EV_PULLBACK_WEAPON, 37),
            Some((K::PULLBACK_WEAPON.0, 37))
        );
    }

    #[test]
    fn shots_melee_and_hints_are_not_fire_sounds() {
        for id in [
            ev::EV_FIRE_WEAPON,
            ev::EV_FIRE_WEAPON_LASTSHOT_LEFT,
            ev::EV_MELEE_SWIPE,
            ev::EV_FIRE_MELEE,
            ev::EV_CANNOTPLANT,
            ev::EV_LOCKON_REQUIRED_HINT,
            ev::EV_INC_CHARGESHOT_LEVEL,
        ] {
            assert_eq!(sound(id, 0), None, "{id}");
        }
    }

    #[test]
    fn no_click_for_clip_only_or_stays_up_guns() {
        // Sent silent: the client still runs BO2's out-of-ammo switch on it.
        for (id, left) in [(ev::EV_NOAMMO, 0), (ev::EV_NOAMMO_LEFT, NOAMMO_PARM_LEFT)] {
            for gun in [2, 3] {
                assert_eq!(
                    fire_sound_event(&e(id, 0), &guns(), gun),
                    Some((K::NOAMMO.0, left | NOAMMO_PARM_SILENT))
                );
            }
        }
        // The bolt still sounds on those guns.
        assert_eq!(
            fire_sound_event(&e(ev::EV_RECHAMBER_WEAPON, 0), &guns(), 2),
            Some((K::RECHAMBER_WEAPON.0, 0))
        );
    }

    /// The prone override is one bit both sides share: the gun reads the player's (not its own
    /// copy) and hands back what it set or cleared.
    #[test]
    fn the_prone_override_is_the_players_bit() {
        let mut ps = PlayerState::ZERO;
        let gun_copy = pmf::ADS | pmf::PRONE_MOVE_OVERRIDE;
        assert_eq!(kept_pm_flags(&ps, gun_copy), pmf::ADS);
        ps.pm_flags = gpmf::PRONEMOVE_OVERRIDDEN;
        assert_eq!(kept_pm_flags(&ps, 0), pmf::PRONE_MOVE_OVERRIDE);

        let mut ps = PlayerState::ZERO;
        share_prone_override(&mut ps, pmf::PRONE_MOVE_OVERRIDE);
        assert_eq!(ps.pm_flags, gpmf::PRONEMOVE_OVERRIDDEN);
        share_prone_override(&mut ps, 0);
        assert_eq!(ps.pm_flags, 0);
    }

    /// Slide ground reaches the gun from the player, and the gun never keeps its own copy of it.
    #[test]
    fn slide_ground_reaches_the_gun_from_the_player() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = movement_t6::PMF_SLIDING;
        assert_eq!(kept_pm_flags(&ps, 0), pmf::SLIDE);
        assert_eq!(kept_pm_flags(&PlayerState::ZERO, pmf::SLIDE), 0);
        assert_eq!(pm_flags(movement_t6::PMF_SLIDING) & pmf::SLIDE, 0);
    }
}
