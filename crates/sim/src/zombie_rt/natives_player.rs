//! Player methods: spawn, weapons, ammo, perks, controls, buttons, pose.

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, arg, client, entnum, flag, frame, int, list, num, origin_of, text, tick, vec3, weapon, weapon_text};
use crate::client_script;

/// The player field that marks him invulnerable (no script name has `$`).
pub(super) const INVULNERABLE: &str = "$invulnerable";
/// The player field that marks him as one hits never push
/// (`playerknockback(0)`; BO2's entity flag 8).
pub(super) const NO_KNOCKBACK: &str = "$noknockback";
use crate::sim_parts::ClientId;

/// `no_spectate` bits beside the teams' (BO2's PlayerCmd_AllowSpectateTeam).
pub(super) const SPECTATE_FREELOOK: u32 = 0x400;
pub(super) const SPECTATE_LOCALPLAYERS: u32 = 0x800;

/// BO2's `setstance` on a player who is not a bot: the real stance (movement_state_bits),
/// the eye height it moves to (stand 60, crouch 40, prone 11), and, going
/// prone, the way he lies (his view yaw). The entity's stance flags follow at
/// once so `getstance` reads it the same frame. An unknown name does nothing.
pub(super) fn set_stance(ps: &mut playerstate_t6::game::AvatarState, stance: &str) {
    use playerstate_t6::game::{entity_flag_bits, movement_state_bits};
    let (flag, view, e) = match stance {
        "stand" => (0, 60, 0),
        "crouch" => (movement_state_bits::DUCK_BUTTON_BIT, 40, entity_flag_bits::CROUCH_BIT),
        "prone" => (movement_state_bits::LYING_FLAT, 11, entity_flag_bits::LYING_FLAT),
        _ => return,
    };
    if flag == movement_state_bits::LYING_FLAT && ps.movement_state_bits & movement_state_bits::LYING_FLAT == 0 {
        ps.prone_direction = ps.look_angles[1];
    }
    ps.movement_state_bits = (ps.movement_state_bits & !(movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT)) | flag;
    ps.eye_height_goal = view;
    ps.e_flags = (ps.e_flags & !(entity_flag_bits::CROUCH_BIT | entity_flag_bits::LYING_FLAT)) | e;
}

/// BO2's CM_GetWaterHeight (the map alone; no MP map sets a
/// waterbrush_entity): CM_TracePointDown from `up` above `at` to `down`
/// below it through water (contents 0x20). A hit counts when its surface
/// has a type (0x3f00000), a start inside the water gives the start when
/// the trace has surface flags; else -131072.
fn water_height(world: &mut World, at: [f32; 3], up: f32, down: f32) -> f32 {
    const NONE: f32 = -131072.0;
    let start = [at[0], at[1], at[2] + up];
    let end = [at[0], at[1], at[2] + down];
    let t = frame(world).trace_static_world(start, end, [0.0; 3], [0.0; 3], 0x20);
    let z = if t.fraction != 1.0 && t.sflags & 0x03f0_0000 != 0 {
        start[2] + (end[2] - start[2]) * t.fraction
    } else if t.starts_inside && t.sflags != 0 {
        start[2]
    } else {
        return NONE;
    };
    if z - NONE >= 0.0 { z } else { NONE }
}

/// A client system's state: "musicCmd" is the music (zm_nuked_amb.csc's
/// states: WAVE loops Nuketown's underscore, the rest are silence); others
/// are not drawn yet.
/// bo2mp: a level notify for the client scripts (maps/mp/_utility's
/// `clientnotify` sends it as the "levelNotify" system's state): the map's
/// client parts that wait on it (Nuketown 2025's end-game cars and flags).
fn level_notify(vm: &mut Vm<World>, game: &mut World, cid: i32, status: &str) {
    let is_level = usize::try_from(cid)
        .ok()
        .and_then(|i| game.resource::<Zm>().client_sys.get(i).cloned())
        .is_some_and(|n| n.eq_ignore_ascii_case("levelnotify"));
    if is_level {
        super::clientfields::client_notify(vm, game, status);
    }
}

/// bo2mp: the level notifies the client scripts' sound code waits on
/// (clientscripts/mp/_audio.csc): the Reaper's start and end, the match's
/// end and its full mix, the final killcam's start and end.
const SND_NOTIFIES: [&str; 6] = ["krms", "krme", "pm", "pmf", "fkcb", "fkce"];

/// A sound notify for one client: its `bo2mp_sndnotify` becomes
/// "<n> <state> ... <state>", its last four, the newest last and numbered n
/// (the client plays each one it has not seen; a dvar can drop a change).
fn snd_notify(game: &mut World, peer: u32, status: &str) {
    let value = {
        let mut zm = game.resource_mut::<Zm>();
        let (n, last) = zm.snd_notify.entry(peer).or_default();
        *n += 1;
        last.push(status.to_owned());
        if last.len() > 4 {
            last.remove(0);
        }
        format!("{n} {}", last.join(" "))
    };
    diag::info!(Sim, "bo2mp sndnotify: client {peer} \"{value}\"");
    super::set_client_dvar(game, peer, "bo2mp_sndnotify", &value);
}

fn client_sys_state(world: &mut World, id: i32, state: &str, who: Option<u32>) {
    let name = usize::try_from(id)
        .ok()
        .and_then(|i| world.resource::<Zm>().client_sys.get(i).cloned())
        .unwrap_or_default();
    // A level notify for the client scripts (maps/mp/_utility::clientnotify
    // sends it this way): "znfg", Nuketown's game over, turns every local
    // player's world fog to bank 2 (zm_nuked.csc intermission_settings), so
    // the rocket shot sees the town.
    if name.eq_ignore_ascii_case("levelnotify") {
        if state == "znfg" {
            let clients: Vec<u32> = match who {
                Some(n) => vec![n],
                None => world.resource::<Zm>().players.keys().copied().collect(),
            };
            for n in clients {
                super::set_client_dvar(world, n, "bo2zm_fogbank", "2");
            }
        }
        if let Some(notify) = SND_NOTIFIES.iter().find(|s| s.eq_ignore_ascii_case(state)) {
            let clients: Vec<u32> = match who {
                Some(n) => vec![n],
                None => world.resource::<Zm>().players.keys().copied().collect(),
            };
            for n in clients {
                snd_notify(world, n, notify);
            }
        }
        return;
    }
    // Down: BO2's client script plays the heartbeat and the low loop
    // while "lsm" is "1" (audio laststand.rs).
    if name.eq_ignore_ascii_case("lsm") {
        let clients: Vec<u32> = match who {
            Some(n) => vec![n],
            None => world.resource::<Zm>().players.keys().copied().collect(),
        };
        for n in clients {
            super::set_client_dvar(world, n, "bo2zm_lsm", state);
        }
        return;
    }
    if !name.eq_ignore_ascii_case("musiccmd") {
        return;
    }
    let alias = match state.to_ascii_uppercase().as_str() {
        "WAVE" => "mus_nuked_underscore",
        _ => "",
    };
    let clients: Vec<u32> = match who {
        Some(n) => vec![n],
        None => world.resource::<Zm>().players.keys().copied().collect(),
    };
    for n in clients {
        super::set_client_dvar(world, n, "bo2zm_music", alias);
    }
}

fn controls(
    vm: &Vm<World>,
    world: &mut World,
    s: &ScriptValue,
    set: impl FnOnce(&mut crate::player_ledger::ScriptControls),
) -> Result<ScriptValue, String> {
    let id = client(vm, world, s)?;
    let mut f = frame(world);
    if f.client_meta(id).is_some() {
        set(&mut f.client_meta_mut(id).controls);
    }
    Ok(ScriptValue::Undefined)
}

/// A shellshock ends (`stopshellshock` or its time ran out): the timer, the
/// SHELLSHOCKED flag and the speed factor it put on the scale go.
pub(super) fn end_shock(world: &mut World, n: u32, why: &str) {
    let id = ClientId(n);
    let taken = world
        .resource_mut::<Zm>()
        .players
        .get_mut(&n)
        .and_then(|p| {
            let label = p.shock.take()?.0;
            let factor = std::mem::replace(&mut p.shock_factor, 1.0);
            Some((label, factor))
        });
    let Some((label, factor)) = taken else {
        // Nothing this script started: still clear the timer.
        if let Some(ps) = frame(world).occupant_mut(id) {
            ps.stun_length = 0;
            ps.movement_state_bits &= !playerstate_t6::game::movement_state_bits::SHELLSHOCKED;
        }
        return;
    };
    let level = crate::level_time_ms(tick(world));
    let mut f = frame(world);
    if let Some(ps) = f.occupant_mut(id) {
        ps.stun_length = 0;
        ps.movement_state_bits &= !playerstate_t6::game::movement_state_bits::SHELLSHOCKED;
        ps.run_speed_modifier /= factor;
    }
    f.client_meta_mut(id).shellshock = None;
    diag::info!(
        Sim,
        "bo2mp shellshock end: client {n} '{label}' at {level} ms ({why}), speed x{factor} off"
    );
}

/// BO2's `allow*`: sets the switch and returns whether it was allowed before
/// (scripts keep it: `prone = self allowprone( 0 ); ... self allowprone( prone );`).
fn allow(
    vm: &Vm<World>,
    world: &mut World,
    s: &ScriptValue,
    swap: impl FnOnce(&mut crate::player_ledger::ScriptControls) -> bool,
) -> Result<ScriptValue, String> {
    let id = client(vm, world, s)?;
    let mut f = frame(world);
    let was_off = f.client_meta(id).is_some() && swap(&mut f.client_meta_mut(id).controls);
    Ok(ScriptValue::Int(if was_off { 0 } else { 1 }))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    m!("spawn", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let spot = vec3(a, 0)?;
        let angles = super::arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let state = world
            .resource::<Zm>()
            .players
            .get(&cid.0)
            .map_or_else(|| "playing".to_owned(), |p| p.sessionstate.clone());
        let t = tick(world);
        client_script::spawn(&mut frame(world), t, cid, spot, angles, &state);
        // bo2mp: BO2's `ClientSpawn` clears his client but a few fields, the
        // last weapon with them, so `weapon_change` fires on his first frame
        // alive even with the gun he died holding (`_weapons.gsc` builds his
        // stowed-weapon list off it).
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.last_weapon = 0;
        }
        // (Multiplayer's per-frame notifies keep their own memory.)
        super::gun_script::forget_watch(world, cid.0);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: BO2's `suicide` (PlayerCmd_Suicide): he dies by his own hand,
    // MOD_SUICIDE with no weapon ("none"), 100000 damage (switching teams
    // mid-life, _globallogic_ui.gsc). A player already dead stays so
    // (player_die gives up on a dead body).
    m!("suicide", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let Some(obj) = s.as_obj() else {
            return Ok(ScriptValue::Undefined);
        };
        let alive = frame(world)
            .client_meta(cid)
            .is_some_and(|m| m.lifecycle == crate::player_ledger::PlayerSessionStage::Alive);
        if alive {
            let t = tick(world);
            super::self_death(vm, world, t, cid.0, obj, None, "MOD_SUICIDE", "none");
        }
        Ok(ScriptValue::Undefined)
    });
    m!("giveweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name)?;
        if w == 0 {
            return Ok(ScriptValue::Undefined);
        }
        // BO2's engine holds a melee weapon it gives as his melee weapon
        // (the knife, the Bowie Knife, the Galvaknuckles): the one he stabs
        // with, replacing the last.
        if frame(world).weapon_is_melee_only(w)
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0)
        {
            p.melee_weapon = Some(name.clone());
            super::set_melee_weapon(cid.0, w);
        }
        client_script::give_weapon(&mut frame(world), cid, w, false)?;
        // bo2mp: the name he was given it by, when the catalog's differs
        // (attachments it does not hold under that name). Multiplayer only:
        // Zombies reads its names as before.
        let catalog = weapon_text(world, w);
        let mp = world.resource::<Zm>().mp;
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            if mp && name.contains('+') && name != catalog {
                p.given_names.insert(w, name);
            } else {
                p.given_names.remove(&w);
            }
        }
        // bo2zm: Black Ops II's engine puts a grenade it gives into its
        // offhand slot (frag/Semtex lethal on the grenade key, the monkey
        // bomb tactical); the old scripts named the class themselves.
        let mut f = frame(world);
        // Its `offhandSlot` picks the slot (the Hell's Retriever is a class-1
        // grenade in slot 2, the tactical).
        let class = f.gadget_stats_lookup(w).map_or(0, |eq| eq.select_class());
        if let Some(pstate) = f.occupant_mut(cid) {
            match class {
                1 => pstate.secondary_gear_main = 1,
                2 | 3 => pstate.secondary_gear_alt = class,
                _ => {}
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("takeweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        if held_melee(world, cid) == w
            && let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0)
        {
            p.melee_weapon = None;
            super::set_melee_weapon(cid.0, 0);
        }
        client_script::take_weapon(&mut frame(world), cid, w);
        // bo2mp: the name he was given it by goes with it (a gun picked up
        // later reads as its own).
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.given_names.remove(&w);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("takeallweapons", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.melee_weapon = None;
            super::set_melee_weapon(cid.0, 0);
        }
        client_script::take_all_weapons(&mut frame(world), cid);
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.given_names.clear();
            p.melee_weapon = None;
        }
        super::set_melee_weapon(cid.0, 0);
        Ok(ScriptValue::Undefined)
    });
    m!("hasweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let Ok(w) = weapon(world, &text(vm, a, 0)) else {
            return Ok(ScriptValue::Int(0));
        };
        Ok(ScriptValue::bool(
            w != 0
                && (held_melee(world, cid) == w
                    || client_script::holds_weapon(&frame(world), cid, w)),
        ))
    });
    m!("getcurrentweapon", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let w = frame(world).avatar(cid).map_or(0, |pstate| pstate.weapon);
        let n = super::weapon_text_of(world, Some(cid.0), w);
        Ok(vm.string(&n))
    });
    m!("getcurrentoffhand", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let w = frame(world)
            .avatar(cid)
            .map_or(0, |pstate| pstate.secondary_gear_main as u32);
        let n = weapon_text(world, w);
        Ok(vm.string(&n))
    });
    // True when he has it to switch to (a bot calling in a map-picked
    // scorestreak goes on only then).
    m!("switchtoweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        client_script::switch_to_weapon(&mut frame(world), cid, w);
        Ok(ScriptValue::bool(
            w != 0 && client_script::has_weapon(&frame(world), cid, w),
        ))
    });
    m!("switchtoweaponimmediate", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        client_script::switch_to_weapon(&mut frame(world), cid, w);
        Ok(ScriptValue::Undefined)
    });
    m!("setspawnweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        client_script::switch_to_weapon(&mut frame(world), cid, w);
        Ok(ScriptValue::Undefined)
    });
    // BO2: the alternate modes (held as weapons of their own) only with a true first argument.
    m!("getweaponslist", |vm, world, s, a| {
        if a.first().and_then(ScriptValue::as_int).is_some_and(|n| n != 0) {
            let cid = client(vm, world, s)?;
            let ws = client_script::weapons_with_alt_modes(&frame(world), cid);
            let mut out = Vec::new();
            for w in ws {
                let n = super::weapon_text_of(world, Some(cid.0), w);
                out.push(vm.string(&n));
            }
            return Ok(list(out));
        }
        weapon_list(vm, world, s, client_script::WeaponList::All)
    });
    m!("getweaponslistprimaries", |vm, world, s, _| weapon_list(
        vm,
        world,
        s,
        client_script::WeaponList::Primaries
    ));
    m!("setweaponammoclip", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let mut f = frame(world);
        // The left gun of a held dual-wield pair: its clip.
        match client_script::dual_wield_right_of(&f, cid, w) {
            Some(right) => client_script::set_left_clip(&mut f, cid, right, int(a, 1)?),
            None => client_script::set_ammo_clip(&mut f, cid, w, int(a, 1)?),
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2's PlayerCmd_SetWeaponAmmoStock / GetWeaponAmmoStock: a clip-only
    // weapon (bClipOnly: the tomahawk, the grenades) has no stock, so they
    // set and read its clip (a scavenger bag's +1 tomahawk lands in hand).
    // The clip goes through the setweaponammoclip / getweaponammoclip bound
    // last, so it lands where the gun brain keeps it when that is on.
    m!("setweaponammostock", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        if frame(world).gun_facts().get(w).core.motion_clip_exclusive {
            if !client_script::holds_weapon(&frame(world), cid, w) {
                return Ok(ScriptValue::Undefined);
            }
            return match vm.bound("setweaponammoclip", true) {
                Some(set_clip) => set_clip(vm, world, s, a),
                None => Ok(ScriptValue::Undefined),
            };
        }
        let count = int(a, 1)?;
        client_script::set_ammo_stock(&mut frame(world), cid, w, count);
        Ok(ScriptValue::Undefined)
    });
    m!("getweaponammoclip", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let f = frame(world);
        Ok(ScriptValue::Int(
            match client_script::dual_wield_right_of(&f, cid, w) {
                Some(right) => client_script::left_clip(&f, cid, right),
                None => client_script::ammo_clip(&f, cid, w),
            },
        ))
    });
    m!("getweaponammostock", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        if frame(world).gun_facts().get(w).core.motion_clip_exclusive {
            return match vm.bound("getweaponammoclip", true) {
                Some(get_clip) => get_clip(vm, world, s, a),
                None => Ok(ScriptValue::Int(0)),
            };
        }
        Ok(ScriptValue::Int(client_script::rounds_held(&frame(world), cid, w)))
    });
    m!("getammocount", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let f = frame(world);
        Ok(ScriptValue::Int(
            client_script::ammo_clip(&f, cid, w) + client_script::rounds_held(&f, cid, w),
        ))
    });
    m!("givemaxammo", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        client_script::give_max_ammo(&mut frame(world), cid, w);
        Ok(ScriptValue::Undefined)
    });
    // T6 gives a full clip on top of the start ammo, all of which goes to the
    // stock: real BO2 starts the M1911 (clip 8, start 4 clips) at 8/32. The
    // old rule takes the clip out of it (8/24), and then `ammo_give` sees the
    // stock under `weaponstartammo` and charges for ammo on a gun just bought
    // off the wall. Grenades and mines keep their own rule.
    m!("givestartammo", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let mut f = frame(world);
        let offhand = f.gadget_stats_lookup(w).is_some_and(|eq| eq.is_secondary_gear());
        match f.damage_profile_of(w) {
            Some(facts) if !offhand => {
                let start = facts.spawn_rounds.max(0);
                let clip = if start > 0 { facts.magazine_capacity.max(0) } else { 0 };
                client_script::set_ammo_clip(&mut f, cid, w, clip);
                client_script::set_ammo_stock(&mut f, cid, w, start);
            }
            _ => client_script::give_start_ammo(&mut f, cid, w),
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setperk", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let p = text(vm, a, 0);
        client_script::set_perk(&mut frame(world), cid, &p, true);
        if let Some(pl) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            pl.perks.insert(p);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("unsetperk", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let p = text(vm, a, 0);
        client_script::set_perk(&mut frame(world), cid, &p, false);
        if let Some(pl) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            pl.perks.remove(&p);
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2's poisoned player-state flag: the Denizen sets it while it holds
    // him (_zm_ai_screecher), TranZit's fog gas while he coughs.
    fn poison(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, on: bool) -> Result<ScriptValue, String> {
        let cid = client(vm, game, st)?;
        if let Some(pl) = game.resource_mut::<Zm>().players.get_mut(&cid.0) {
            pl.poisoned = on;
        }
        Ok(ScriptValue::Undefined)
    }
    m!("startpoisoning", |vm, world, s, _| poison(vm, world, s, true));
    m!("stoppoisoning", |vm, world, s, _| poison(vm, world, s, false));
    // bo2mp: BO2's PlayerCmd_AllowSpectateTeam: which teams his spectator
    // view may follow (a bit of `no_spectate` per team, "freelook" and
    // "localplayers" their own); follow_sync keeps to them.
    m!("allowspectateteam", |vm, world, s, lhs| {
        let cid = client(vm, world, s)?;
        let name = text(vm, lhs, 0);
        let bit = match name.as_str() {
            "freelook" => SPECTATE_FREELOOK,
            "localplayers" => SPECTATE_LOCALPLAYERS,
            team => 1u32 << super::goals::team_number(team)?,
        };
        let allow = int(lhs, 1)? != 0;
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            if allow { p.no_spectate &= !bit } else { p.no_spectate |= bit }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("ispoisoned", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::bool(world.resource::<Zm>().players.get(&cid.0).is_some_and(|p| p.poisoned)))
    });
    m!("clearperks", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        client_script::clear_perks(&mut frame(world), cid);
        if let Some(pl) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            pl.perks.clear();
        }
        Ok(ScriptValue::Undefined)
    });
    m!("hasperk", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let p = text(vm, a, 0);
        Ok(ScriptValue::bool(
            world
                .resource::<Zm>()
                .players
                .get(&cid.0)
                .is_some_and(|pl| pl.perks.contains(&p)),
        ))
    });
    // Every perk he has, by name (Buried's minigame loadout stops each one).
    m!("getperks", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let names: Vec<String> = world
            .resource::<Zm>()
            .players
            .get(&cid.0)
            .map(|pl| pl.perks.iter().cloned().collect())
            .unwrap_or_default();
        let out = names.iter().map(|n| vm.string(n)).collect();
        Ok(list(out))
    });
    // No gun has a fuel tank (weaponfuellife is 0), so its fuel is 0 and
    // setting it does nothing (the weapon fridge, Vulture Aid, the Time Bomb).
    m!("getweaponammofuel", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("setweaponammofuel", |_, _, _, _| Ok(ScriptValue::Undefined));
    m!("freezecontrols", |vm, world, s, a| {
        let on = flag(a, 0, true);
        controls(vm, world, s, |c| c.stuck = on)
    });
    // Die Rise's Sliquifier goo: `player forceslick( player.slick_count )`,
    // a count above 0 makes the ground ice under him (no friction).
    m!("forceslick", |vm, world, s, a| {
        let on = flag(a, 0, false);
        controls(vm, world, s, |c| c.slick = on)
    });
    m!("disableweapons", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.weapons_disabled = true
    ));
    m!("enableweapons", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.weapons_disabled = false
    ));
    m!("disableoffhandweapons", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.offhands_disabled = true
    ));
    m!("enableoffhandweapons", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.offhands_disabled = false
    ));
    m!("disableweaponcycling", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.switch_disabled = true
    ));
    m!("enableweaponcycling", |vm, world, s, _| controls(
        vm,
        world,
        s,
        |c| c.switch_disabled = false
    ));
    // bo2mp: BO2's `allow*` (false drops the request in constrain_cmd;
    // the answer is whether it was allowed before, as BO2's).
    m!("allowprone", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.prone_disabled, !on)
        })
    });
    m!("allowcrouch", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.crouch_disabled, !on)
        })
    });
    m!("allowstand", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.stand_disabled, !on)
        })
    });
    m!("allowsprint", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.sprint_disabled, !on)
        })
    });
    m!("allowmelee", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.melee_disabled, !on)
        })
    });
    m!("allowads", |vm, world, s, a| {
        let on = flag(a, 0, true);
        allow(vm, world, s, |c| {
            std::mem::replace(&mut c.ads_disabled, !on)
        })
    });
    // bo2mp: `setufomode(on)` (the mod menu's UFO Mode): movement_mode 3, so his
    // move (and his client's prediction) flies where he looks, through
    // walls. Only from walking, and back only from UFO; dying ends it.
    m!("setufomode", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let on = flag(a, 0, true);
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            if on && pstate.movement_mode == 0 {
                pstate.movement_mode = 3;
                pstate.velocity = [0.0; 3];
            } else if !on && pstate.movement_mode == 3 {
                pstate.movement_mode = 0;
                pstate.velocity = [0.0; 3];
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: `setmovespeedscale(f)` is the player's own scale; a shellshock's
    // `bg_shock_movement` rides on top while it lasts (`getmovespeedscale`
    // gives his own back). The feel window reads
    // `AvatarState::run_speed_modifier` (the product).
    m!("setmovespeedscale", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let scale = num(a, 0)?;
        let factor = world
            .resource::<Zm>()
            .players
            .get(&cid.0)
            .map_or(1.0, |p| p.shock_factor);
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            pstate.run_speed_modifier = scale * factor;
        }
        diag::info!(
            Sim,
            "bo2mp setmovespeedscale: client {} scale {scale} (shock x{factor} = {})",
            cid.0,
            scale * factor
        );
        Ok(ScriptValue::Undefined)
    });
    m!("getmovespeedscale", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let factor = world
            .resource::<Zm>()
            .players
            .get(&cid.0)
            .map_or(1.0, |p| p.shock_factor);
        Ok(ScriptValue::Float(
            frame(world)
                .avatar(cid)
                .map_or(1.0, |pstate| pstate.run_speed_modifier / factor),
        ))
    });
    // bo2mp: BO2's shellshock (shock/<name>.shock): the timer and duration
    // on the player state, the file's screen / look / sound / movement
    // values to his client. Our flashed overlay, look limits and tinnitus
    // run from those; the movement scale is the file's, not a constant.
    m!("shellshock", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0).to_ascii_lowercase();
        let seconds = num(a, 1)?;
        if seconds < 0.0 {
            return Err(format!("shellshock duration {seconds} is negative"));
        }
        let Some(shock) = frame(world).shock(&name).cloned() else {
            return Err(format!("no shock file for shellshock '{name}'"));
        };
        let now = crate::level_time_ms(tick(world));
        let index = {
            let mut zm = world.resource_mut::<Zm>();
            let at = match zm.shock_names.iter().position(|n| *n == name) {
                Some(at) => at,
                None => {
                    zm.shock_names.push(name.clone());
                    zm.shock_names.len() - 1
                }
            };
            at as i32 + 1
        };
        let new_factor = if shock.movement_scale > 0.0 {
            shock.movement_scale
        } else {
            1.0
        };
        let old_factor = world
            .resource::<Zm>()
            .players
            .get(&cid.0)
            .map_or(1.0, |p| p.shock_factor);
        let duration = (seconds * 1000.0) as i32;
        // BO2's file gives a scale, not a switch (hud_t6 ShellshockParms has
        // no walk switch): the scale goes through the move speed multiplier.
        let (look, sound, kick) = (
            shock.look_control.touches,
            shock.sound.loop_alias.clone(),
            shock.view.kick_radius,
        );
        let flashed = shock.screen_blend.kind == hud_t6::flashbang::ShockViewType::Flashed;
        let mut f = frame(world);
        let Some(pstate) = f.occupant_mut(cid) else {
            return Ok(ScriptValue::Undefined);
        };
        pstate.stun_effect_id = index;
        pstate.stun_start = now;
        pstate.stun_length = duration;
        pstate.movement_state_bits |= playerstate_t6::game::movement_state_bits::SHELLSHOCKED;
        pstate.run_speed_modifier = pstate.run_speed_modifier / old_factor * new_factor;
        f.client_meta_mut(cid).shellshock = Some(shock);
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.shock = Some((name.clone(), now.wrapping_add(duration)));
            p.shock_factor = new_factor;
        }
        diag::info!(
            Sim,
            "bo2mp shellshock: client {} '{name}' {seconds}s from {now} ms (index {index}, flash {flashed}, look {look}, loop '{sound}', speed x{new_factor}, view kick {kick})",
            cid.0
        );
        Ok(ScriptValue::Undefined)
    });
    m!("stopshellshock", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        end_shock(world, cid.0, "stopshellshock");
        Ok(ScriptValue::Undefined)
    });
    m!("allowjump", |vm, world, s, a| {
        let on = flag(a, 0, true);
        controls(vm, world, s, |c| c.jump_disabled = !on)
    });
    m!("getplayerangles", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::Vec3(
            frame(world).avatar(cid).map_or([0.0; 3], |pstate| pstate.look_angles),
        ))
    });
    // Last stand: up again (`reviveplayer`: full health, standing).
    m!("reviveplayer", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.laststand = false;
        }
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            use playerstate_t6::game::entity_flag_bits::{CROUCH_BIT, LYING_FLAT};
            use playerstate_t6::game::{MOVE_MODE_DOWNED, movement_state_bits};
            // Out of last stand now, not at the next sync: a move still run
            // as last stand put the prone flag back, and the last stand's
            // eye height read as prone, so he got up lying prone (10-09
            // trace). As the engine's own revive: crouched, and the move's
            // own room check stands him up the next tick. The stand event
            // resets his client's stance keys. (Black Ops II keeps last
            // stand in movement_mode alone: no movement_state_bits bit.)
            let before = (pstate.movement_mode, pstate.movement_state_bits, pstate.eye_height_goal, pstate.eye_blend_clock);
            if pstate.movement_mode == MOVE_MODE_DOWNED {
                pstate.movement_mode = 0;
            }
            pstate.movement_state_bits = (pstate.movement_state_bits & !movement_state_bits::LYING_FLAT) | movement_state_bits::DUCK_BUTTON_BIT;
            pstate.e_flags = (pstate.e_flags & !LYING_FLAT) | CROUCH_BIT;
            pstate.eye_height_goal = movement_t6::bridge::eye_level::DUCK_BUTTON_BIT;
            movement_t6::bridge::queue_forecast_happening(
                pstate,
                entity_t6::event::STANCE_FORCE_STAND as i32,
                0,
            );
            pstate.health = pstate.hit_point_ceiling.max(1);
            diag::info!(
                Sim,
                "bo2zm t6: reviveplayer {} (pm_type, pm_flags, target, lerp_time) before {before:?} after pm_flags {:#x}",
                cid.0,
                pstate.movement_state_bits
            );
        }
        Ok(ScriptValue::Undefined)
    });
    m!("undolaststand", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.laststand = false;
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2's GScr_DepthInWater: how far the water's surface (CM_GetWaterHeight,
    // 200 above to 200 below) is over his spot, nothing when it is not.
    // Frost's water deaths splash with it.
    m!("depthinwater", |vm, world, s, _| {
        let Some(at) = origin_of(vm, world, s) else {
            return Ok(ScriptValue::Float(0.0));
        };
        let depth = water_height(world, at, 200.0, -200.0) - at[2];
        Ok(ScriptValue::Float(if depth < 0.0 { 0.0 } else { depth }))
    });
    // bo2mp: the gun on his back (BO2's _weapons.gsc stow_on_back): every
    // client draws its world model on his tag_stowed_back.
    m!("setstowedweapon", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let stowed = super::weapon(world, &name).ok().filter(|&w| w != 0).map(|w| (w, name));
        super::playeranim::edit_body(world, cid.0, |l| l.stowed = stowed);
        Ok(ScriptValue::Undefined)
    });
    m!("clearstowedweapon", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        super::playeranim::edit_body(world, cid.0, |l| l.stowed = None);
        Ok(ScriptValue::Undefined)
    });
    // Camo/lens/reticle options for giveweapon (Pack-a-Punch): none drawn yet.
    m!("calcweaponoptions", |_, _, _, _| Ok(ScriptValue::Int(0)));
    vm.bind("getweaponstowedmodel", false, |vm, _, _, _| {
        Ok(vm.string(""))
    });
    for name in [
        "startrevive",
        "stoprevive",
        "docowardswayanims",
        "setrevivehintstring",
        // Stats only (Mob's afterlife weapon save/restore calls it).
        "seteverhadweaponall",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // Client systems (maps/mp/_utility::registerclientsys and
    // setclientsysstate call these): each registered name gets an cid; a
    // state set on one goes to every client, or as a method to one.
    vm.bind("clientsysregister", false, |vm, world, _, a| {
        let name = text(vm, a, 0);
        let mut zm = world.resource_mut::<Zm>();
        let cid = match zm
            .client_sys
            .iter()
            .position(|n| n.eq_ignore_ascii_case(&name))
        {
            Some(i) => i,
            None => {
                zm.client_sys.push(name);
                zm.client_sys.len() - 1
            }
        };
        Ok(ScriptValue::Int(cid as i32))
    });
    vm.bind("clientsyssetstate", false, |vm, world, _, a| {
        let cid = arg(a, 0).as_int().unwrap_or(-1);
        let state = text(vm, a, 1);
        super::highrise::client_notify(vm, world, cid, &state);
        client_sys_state(world, cid, &state, None);
        level_notify(vm, world, cid, &state);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("clientsyssetstate", true, |vm, world, s, a| {
        let cid = arg(a, 0).as_int().unwrap_or(-1);
        let state = text(vm, a, 1);
        let who = entnum(vm, s);
        super::highrise::client_notify(vm, world, cid, &state);
        client_sys_state(world, cid, &state, who);
        level_notify(vm, world, cid, &state);
        Ok(ScriptValue::Undefined)
    });
    // A player moved by a script (spawning, a game-over fall).
    m!("setorigin", |vm, world, s, a| {
        let to = vec3(a, 0)?;
        match client(vm, world, s) {
            Ok(cid) => super::teleport_player(world, cid, to),
            Err(_) => {
                if let Some(n) = super::entnum(vm, s)
                    && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
                {
                    e.origin = to;
                }
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setplayerangles", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        super::set_player_view(world, cid, vec3(a, 0)?);
        Ok(ScriptValue::Undefined)
    });
    m!("setvelocity", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let v = vec3(a, 0)?;
        if v.iter().map(|c| c * c).sum::<f32>() > 300.0 * 300.0 {
            // A script launch (Trample Steam, a Jumping Jack hit): rare, so logged for tests.
            diag::info!(Sim, "bo2zm t6 setvelocity client {cid:?} {v:?}");
        }
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            pstate.velocity = v;
        }
        Ok(ScriptValue::Undefined)
    });
    // player applyknockback(damage, dir): the push the engine gives a hit
    // (TranZit throws you off a zombie's head, Grief's shots shove). Damage
    // over 200 pushes no harder; 1000 is g_knockback, 250 the player's mass.
    m!("applyknockback", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let damage = super::num(a, 0)?.clamp(0.0, 200.0);
        let dir = vec3(a, 1)?;
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len > 0.0 {
            let push = 1000.0 * damage / 250.0 / len;
            if let Some(pstate) = frame(world).occupant_mut(cid) {
                for i in 0..3 {
                    pstate.velocity[i] += dir[i] * push;
                }
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getstance", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let st = client_script::stance(&frame(world), cid);
        Ok(vm.string(st))
    });
    m!("setstance", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let st = text(vm, a, 0);
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            set_stance(pstate, &st);
            // The forced-stance event sets the client's own stance keys
            // (else its next move stands straight back up).
            let event = match st.as_str() {
                "crouch" => entity_t6::event::STANCE_FORCE_CROUCH,
                "prone" => entity_t6::event::STANCE_FORCE_PRONE,
                _ => entity_t6::event::STANCE_FORCE_STAND,
            };
            movement_t6::bridge::queue_forecast_happening(pstate, event as i32, 0);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("isonground", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::bool(frame(world).avatar(cid).is_some_and(|pstate| {
            pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT
        })))
    });
    // The entity under the feet (any entity: Die Rise asks it of zombies,
    // equipment, buildable pieces; Buried of a thrown grenade), undefined
    // in the air. A script brush model or platform there is that entity
    // (`riders::ground_ent`); the map itself is `level` (_zm_buildables
    // checks `landed_on == level`), and `level getgroundent()` is level, so
    // object_is_on_elevator's walk down stops. _zm_utility::is_jumping is
    // !isdefined(getgroundent()): Afterlife's fake death waits on it.
    m!("getgroundent", |vm, world, s, _| {
        if s.as_obj() == Some(vm.level) {
            return Ok(ScriptValue::Object(vm.level));
        }
        // A player standing on another entity: the number his player state
        // holds names it (a gondola's cab, a lift, another player). The map
        // itself falls through to `level`, as below.
        if let Ok(cid) = client(vm, world, s)
            && let Some(g) = frame(world).avatar(cid).map(|pstate| pstate.standing_on)
            && g != playerstate_t6::game::NO_ENTITY_SLOT - 1
            && let Some(o) = ground_object(world.resource::<Zm>(), g)
        {
            return Ok(ScriptValue::Object(o));
        }
        let Some(spot) = origin_of(vm, world, s) else {
            return Ok(ScriptValue::Undefined);
        };
        // Not a player, in or on a vehicle and not linked to it (a zombie
        // that climbed into Tranzit's bus; one at a window is linked): that
        // vehicle. The bus's floor is not the map's, so the trace under such
        // a zombie found nothing and it was in the air: busupdatenearzombies
        // never counted it on the bus, its entering_bus stayed set and
        // zombiemoveonbus never ran (runs/pr4c: they stood by the driver).
        let vehicle = if client(vm, world, s).is_err() {
            entnum(vm, s)
                .filter(|n| world.resource::<Zm>().ents.get(n).is_some_and(|e| e.link.is_none()))
                .and_then(|_| super::riders::vehicle_under(world, spot))
        } else {
            None
        };
        let grounded = vehicle.is_some() || match client(vm, world, s) {
            Ok(cid) => frame(world)
                .avatar(cid)
                .is_some_and(|pstate| pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT),
            // Not a player: the map within a few units under it.
            Err(_) => {
                let down = [spot[0], spot[1], spot[2] - 12.0];
                let t = frame(world).trace_world(
                    [spot[0], spot[1], spot[2] + 2.0],
                    down,
                    [0.0; 3],
                    [0.0; 3],
                    crate::shot_trace::PLAYER_BODY_MASK,
                );
                t.fraction < 1.0 || t.starts_inside
            }
        };
        if !grounded {
            return Ok(ScriptValue::Undefined);
        }
        let under = vehicle.or_else(|| super::riders::ground_ent(world, spot));
        let zm = world.resource::<Zm>();
        if let Some(n) = under
            && std::env::var_os("BO2MP_T6_RIDELOG").is_some()
        {
            // Once per (asker, ground) pair.
            static SEEN: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());
            let me = entnum(vm, s).unwrap_or(u32::MAX);
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.contains(&(me, n)) {
                seen.push((me, n));
                diag::info!(
                    Sim,
                    "bo2zm t6 getgroundent ent{me} -> ent{n} {} at ({:.0} {:.0} {:.0})",
                    zm.ents.get(&n).map_or("", |e| e.class_tag.as_str()),
                    spot[0],
                    spot[1],
                    spot[2]
                );
            }
        }
        Ok(ScriptValue::Object(
            under
                .and_then(|n| zm.ents.get(&n))
                .and_then(|e| e.obj)
                .unwrap_or(vm.level),
        ))
    });
    // The moving platform he stands on (`riders`), undefined off one:
    // Tranzit counts the bus's riders by it.
    m!("getmoverent", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let zm = world.resource::<Zm>();
        Ok(zm
            .riders
            .on
            .get(&cid.0)
            .and_then(|n| zm.ents.get(n))
            .and_then(|e| e.obj)
            .map_or(ScriptValue::Undefined, ScriptValue::Object))
    });
    m!("isonladder", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::bool(frame(world).avatar(cid).is_some_and(|pstate| {
            pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::LADDER != 0
        })))
    });
    m!("issprinting", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::bool(frame(world).avatar(cid).is_some_and(|pstate| {
            pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::SPRINTING != 0
        })))
    });
    m!("ismantling", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::bool(frame(world).avatar(cid).is_some_and(|pstate| {
            pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::MANTLE != 0
        })))
    });
    macro_rules! button {
        ($($name:literal => $mask:expr),* $(,)?) => {$(
            vm.bind($name, true, |vm, world, s, _| {
                let cid = client(vm, world, s)?;
                let held = client_script::buttons(&mut frame(world), cid);
                Ok(ScriptValue::bool(held & ($mask) != 0))
            });
        )*};
    }
    {
        use playerstate_t6::game::buttons::*;
        button!(
            "attackbuttonpressed" => ATTACK,
            "usebuttonpressed" => INTERACT_KEY | INTERACT_RELOAD_KEY,
            "meleebuttonpressed" => MELEE_CHARGE,
            "fragbuttonpressed" => SAMPLE_FRAG_ASSET,
            "secondaryoffhandbuttonpressed" => OBSCURANT,
            "adsbuttonpressed" => ADS,
            "jumpbuttonpressed" => LEAP,
            "sprintbuttonpressed" => SPRINT,
            "stancebuttonpressed" => STANCE_LOCK_BIT | DUCK_BUTTON_BIT | LYING_FLAT,
            "throwbuttonpressed" => THROW,
        );
    }
    // Buttons our commands do not carry (d-pad slots, inventory, vehicle
    // seat): never held. Mob of the Dead's Afterlife polls them.
    for name in [
        "actionslotonebuttonpressed",
        "actionslottwobuttonpressed",
        "actionslotthreebuttonpressed",
        "actionslotfourbuttonpressed",
        "inventorybuttonpressed",
        "changeseatbuttonpressed",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::bool(false)));
    }
    m!("isfiring", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let held = client_script::buttons(&mut frame(world), cid);
        Ok(ScriptValue::bool(held & playerstate_t6::game::buttons::ATTACK != 0))
    });
    // Holding melee (Origins' staffs check it before a staff melee).
    m!("ismeleeing", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let held = client_script::buttons(&mut frame(world), cid);
        Ok(ScriptValue::bool(
            held & playerstate_t6::game::buttons::MELEE_CHARGE != 0,
        ))
    });
    // self islinkedto(ent): linked (linkto / playerlinkto) to that entity.
    m!("islinkedto", |vm, world, s, a| {
        let (Some(me), Some(parent)) = (entnum(vm, s), entnum(vm, arg(a, 0))) else {
            return Ok(ScriptValue::bool(false));
        };
        let zm = world.resource::<Zm>();
        let to = zm
            .players
            .get(&me)
            .and_then(|p| p.linked)
            .or_else(|| zm.ents.get(&me).and_then(|e| e.link.map(|l| l.0)));
        Ok(ScriptValue::bool(to == Some(parent)))
    });
    // Mob of the Dead's Afterlife saves the arms model and puts it back
    // (setviewmodel is not drawn yet): any text keeps the scripts going.
    m!("getviewmodel", |vm, _, _, _| Ok(vm.string("")));
    m!("geteye", |vm, world, s, _| {
        // Zombies have eyes too (Town's burning zombies singe players whose
        // eyes come within 25 units of theirs): the spot plus a standing eye.
        let Ok(cid) = client(vm, world, s) else {
            let o = origin_of(vm, world, s).ok_or("geteye: not an entity")?;
            return Ok(ScriptValue::Vec3([o[0], o[1], o[2] + 60.0]));
        };
        let f = frame(world);
        let (o, h) = f
            .avatar(cid)
            .map_or(([0.0; 3], 60.0), |pstate| (pstate.origin, pstate.eye_height_now));
        Ok(ScriptValue::Vec3([o[0], o[1], o[2] + h]))
    });
    // The eye without the view's bob (Buried ghosts' sight checks): the same eye.
    m!("geteyeapprox", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let f = frame(world);
        let (o, h) = f
            .avatar(cid)
            .map_or(([0.0; 3], 60.0), |pstate| (pstate.origin, pstate.eye_height_now));
        Ok(ScriptValue::Vec3([o[0], o[1], o[2] + h]))
    });
    // The view's place: the eye (no third-person or linked cameras here).
    m!("getplayercamerapos", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let f = frame(world);
        let (o, h) = f
            .avatar(cid)
            .map_or(([0.0; 3], 60.0), |pstate| (pstate.origin, pstate.eye_height_now));
        Ok(ScriptValue::Vec3([o[0], o[1], o[2] + h]))
    });
    // The eye's height over the feet right now, a number (the Jet Gun's smoke
    // takes it off the muzzle: BO2 reads the player state's current height).
    m!("getplayerviewheight", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let h = frame(world).avatar(cid).map_or(60.0, |pstate| pstate.eye_height_now);
        Ok(ScriptValue::Float(h))
    });
    m!("getvelocity", |vm, world, s, _| {
        // bo2mp: an actor's is how fast it went last tick (a K9 Unit dog's
        // turn and pain scripts ask).
        if let Some(n) = entnum(vm, s)
            && let Some(a) = world.resource::<Zm>().actors.get(&n)
        {
            return Ok(ScriptValue::Vec3(a.velocity));
        }
        if let Some(v) = entnum(vm, s).and_then(|n| super::vehicles::velocity(world, n)) {
            return Ok(ScriptValue::Vec3(v));
        }
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::Vec3(
            frame(world).avatar(cid).map_or([0.0; 3], |pstate| pstate.velocity),
        ))
    });
    // Juggernog's 160 (190 upgraded). A zombie's call does nothing (its
    // health is a script field).
    m!("setmaxhealth", |vm, world, s, a| {
        let Ok(cid) = client(vm, world, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let n = int(a, 0)?.max(1);
        let mut f = frame(world);
        f.client_meta_mut(cid).hit_point_ceiling = n;
        if let Some(pstate) = f.occupant_mut(cid) {
            pstate.hit_point_ceiling = n;
            pstate.health = pstate.health.min(n);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setnormalhealth", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let frac = super::num(a, 0)?;
        if let Some(pstate) = frame(world).occupant_mut(cid) {
            pstate.health = ((pstate.hit_point_ceiling as f32 * frac) as i32).max(1);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("getguid", |vm, _, _, _| Ok(vm.string("0")));
    m!("getxuid", |vm, _, _, _| Ok(vm.string("0")));
    // The first player in is the one running the game.
    m!("ishost", |vm, world, s, _| {
        Ok(ScriptValue::Int(i32::from(client(vm, world, s)? == crate::ClientId(0))))
    });
    m!("isplayeronsamemachine", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // No ufo or noclip here. BO2 answers nothing (not 0) outside them:
    // TranZit's bus fog asks isdefined() of it before it poisons him.
    m!("isinmovemode", |_, _, _, _| Ok(ScriptValue::Undefined));
    m!("isinvehicle", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("isremotecontrolling", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("getsnapshotackindex", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // How full a gun is (the scripts' grenade award gives 2, 3 or 4 by it).
    m!("getfractionstartammo", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        Ok(ScriptValue::Float(client_script::ammo_fraction(
            &frame(world),
            cid,
            w,
            true,
        )))
    });
    m!("getfractionmaxammo", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name)?;
        let f = frame(world);
        let frac = client_script::ammo_fraction(&f, cid, w, false);
        if std::env::var_os("BO2MP_T6_HINTLOG").is_some() {
            let eq = f.gadget_stats_lookup(w).map(|e| (e.magazine_capacity, e.opening_ammo));
            diag::info!(
                Sim,
                "bo2zm t6 getfractionmaxammo {name}: {frac} (clip {} stock {}, clip size/start {eq:?})",
                client_script::ammo_clip(&f, cid, w),
                client_script::rounds_held(&f, cid, w)
            );
        }
        Ok(ScriptValue::Float(frac))
    });
    for name in [
        "setclientuivisibilityflag",
        "setclientminiscoreboardhide",
        "setclientammocounterhide",
        "setclientscriptmainmenu",
        "setclientthirdpersonangle",
        "setclientcgobjectivetext",
        "setclientdvar",
        "setclientdvars",
        "openmenu",
        "closemenu",
        "closeingamemenu",
        "allowlean",
        "allowpitchangle",
        "allowedstances",
        "setlowready",
        "setsprintduration",
        "setsprintcooldown",
        "resetfov",
        "setburn",
        "addplayerstat",
        "addweaponstat",
        "incrementplayerstat",
        "addplayerstatwithgametype",
        "updatestatratio",
        "setplayercurrentstreak",
        "recordplayerdeathzombies",
        "recordplayerrevivezombies",
        "recordzombiezone",
        "recordkillmodifier",
        "setclientflag",
        "clearclientflag",
        "sendfaceevent",
        "setlaststandprevweap",
        "setspawnerteam",
        "setviewmodel",
        "enableafterlife",
        "disableafterlife",
        "dropscavengeritem",
        // Turned: back to human (zcleansed, _zm_turned); his zombie side
        // is read from `.is_zombie`.
        "turnedhuman",
        // _zm_stats / _globallogic: the match history the stats screen keeps.
        "gamehistorystartmatch",
        "gamehistoryfinishmatch",
        "setempjammed",
        "setfreecameralockonallowed",
        "luinotifyevent",
        "luinotifyeventtoplayer",
        // bo2mp: his client's talk icons (cg_drawTalk), the league's
        // loss report, and the developer's class menu and gun cycling.
        "setclientdrawtalk",
        "recordleaguepreloser",
        "nextplayerrenderoption",
        "sethighlighted",
        "giveweaponnextattachment",
        "magicgrenade",
        // bo2mp: _burnplayer.gsc's charred skin (BO1's flames: no Black Ops
        // II weapon burns), and the melee verdict, which melee.rs settles
        // as callback_playermelee does.
        "starttanning",
        "finishmeleehit",
        // Any entity (buildable pieces): hide it from demo playback.
        "ghostindemo",
        // Blood fx on hits to this entity ("none": Buried's ghosts don't bleed).
        "bloodimpact",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // bo2mp: the developer's button checks (_dev.gsc, _art.gsc): no key
    // reads as down.
    m!("buttonpressed", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // bo2mp: no player is the host's own on a dedicated server.
    m!("islocaltohost", |vm, world, s, _| {
        client(vm, world, s)?;
        Ok(ScriptValue::Int(0))
    });
    // bo2mp: the unlock is his console's or Steam's; the game keeps none.
    m!("giveachievement", |vm, world, s, a| {
        if text(vm, a, 0).is_empty() {
            return Err("giveachievement: achievement must be named".into());
        }
        client(vm, world, s).map_err(|_| "giveachievement: entity must be a player entity")?;
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: Black Ops II gives no pregame class (_pregame.gsc).
    m!("getpregameclass", |_, _, _, _| Ok(ScriptValue::Undefined));
    // bo2mp: the spectate menu's pick (_menus.gsc): whom he watches.
    m!("setcurrentspectatorclient", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let target = client(vm, world, arg(a, 0))?;
        let Some(o) = world.resource::<Zm>().players.get(&cid.0).map(|p| p.obj) else {
            return Ok(ScriptValue::Undefined);
        };
        let f = vm.intern("spectatorclient");
        vm.set_field(world, o, f, ScriptValue::Int(target.0 as i32));
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: how deep he stands in water (_burnplayer.gsc): his player
    // state's water level, 0 dry to 3 under.
    m!("depthofplayerinwater", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::Int(
            frame(world).avatar(cid).map_or(0, |pstate| pstate.waterlevel),
        ))
    });
    // bo2mp: _tabun.gsc's gas cough (his player state's poisoned flag):
    // BO1's tabun gas, which no Black Ops II weapon throws; nothing here
    // draws it.
    for name in ["startpoisoning", "stoppoisoning"] {
        vm.bind(name, true, |vm, world, s, _| {
            client(vm, world, s).map_err(|_| "USAGE: Must be called on a client")?;
            Ok(ScriptValue::Undefined)
        });
    }
    // bo2mp: no damage reaches him while it is on (finishplayerdamage),
    // as Black Ops II's god flag does; it lasts until turned off.
    m!("enableinvulnerability", |vm, world, s, _| {
        client(vm, world, s)?;
        let (ScriptValue::Object(o), f) = (s, vm.intern(INVULNERABLE)) else { return Ok(ScriptValue::Undefined) };
        vm.set_field(world, *o, f, ScriptValue::Int(1));
        Ok(ScriptValue::Undefined)
    });
    m!("disableinvulnerability", |vm, world, s, _| {
        client(vm, world, s)?;
        let (ScriptValue::Object(o), f) = (s, vm.intern(INVULNERABLE)) else { return Ok(ScriptValue::Undefined) };
        vm.set_field(world, *o, f, ScriptValue::Undefined);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: `playerknockback(on)`: 1 lets hits push him again, anything
    // else stops them (PlayerCmd_PlayerKnockback).
    m!("playerknockback", |vm, world, s, a| {
        client(vm, world, s)?;
        let (ScriptValue::Object(o), f) = (s, vm.intern(NO_KNOCKBACK)) else {
            return Ok(ScriptValue::Undefined);
        };
        let off = if int(a, 0)? == 1 {
            ScriptValue::Undefined
        } else {
            ScriptValue::Int(1)
        };
        vm.set_field(world, *o, f, off);
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: his view from behind (BO2's client dvar cg_thirdPerson): his
    // client dvar `cg_thirdperson`, which the drawing side reads.
    m!("setclientthirdperson", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let on = if gsc_t6::truthy(arg(a, 0)) { "1" } else { "0" };
        let mut f = frame(world);
        if f.client_meta(cid).is_some() {
            let dvars = &mut f.client_meta_mut(cid).client_dvars;
            match dvars.iter_mut().find(|(k, _)| k == "cg_thirdperson") {
                Some(row) => row.1 = on.to_owned(),
                None => dvars.push(("cg_thirdperson".to_owned(), on.to_owned())),
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: BO2's own menus open on the player's screen (lane C); the
    // server only hears his answer (`menuresponse`). BO2MP_AUTOCLASS=<class>
    // answers the class menu a second after it opens (a test aid: a hidden
    // test match spawns without anyone at the keyboard).
    m!("openmenu", |vm, world, s, a| {
        let menu = text(vm, a, 0);
        let client = super::entnum(vm, s).unwrap_or(u32::MAX);
        diag::info!(Sim, "bo2mp menu open: player {client} {menu}");
        if let (ScriptValue::Object(o), Ok(class)) = (s, std::env::var("BO2MP_AUTOCLASS"))
            && menu.contains("changeclass")
        {
            let mut zm = world.resource_mut::<Zm>();
            let due = zm.clock_millis + 1000;
            zm.menu_replies.push((due, *o, menu, class));
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2's own end-of-match blur (`roundenddof`: 0, 128, 512, 4000, 6, 1.8):
    // the view effect the renderer's depth-of-field pass reads. The engine
    // takes it at once. Bad numbers are logged and ignored (not a script
    // error), so no BO2 script can be stopped by it.
    m!("setdepthoffield", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let n = |i: usize| num(a, i);
        let dof = match (n(0), n(1), n(2), n(3), n(4), n(5)) {
            (Ok(a0), Ok(a1), Ok(a2), Ok(a3), Ok(a4), Ok(a5)) => {
                crate::gsc_host::depth_of_field_values(a0, a1, a2, a3, a4, a5)
            }
            _ => Err("six numbers expected".to_string()),
        };
        match dof {
            Ok(dof) => {
                let mut f = frame(world);
                if f.client_meta(cid).is_some() {
                    f.client_meta_mut(cid).view_effects.depth_of_field = dof;
                }
            }
            Err(e) => diag::warn!(Sim, "bo2mp setdepthoffield ignored: {e}"),
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2zm M4: setactionslot(slot, kind[, weapon]): what his d-pad slot
    // holds (BO2's HUD draws it: `bo2zm_actionslots`); "" empties it.
    m!("setactionslot", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let slot = arg(a, 0).as_int().unwrap_or(0) as i32;
        let kind = text(vm, a, 1);
        let held = if kind == "weapon" {
            text(vm, a, 2)
        } else {
            kind
        };
        // His d-pad key (`+actionslot N`) switches to that weapon: the
        // engine's own slot (BO2's equipment, Time Bomb, claymores).
        let (ty, param) = match held.to_ascii_lowercase().as_str() {
            "" | "none" => (0, 0),
            "altmode" => (2, 0),
            "nightvision" => (3, 0),
            w => (1, super::weapon(world, w).unwrap_or(0) as i32),
        };
        if let Ok(i) = usize::try_from(slot - 1)
            && i < 4
            && let Some(pstate) = frame(world).occupant_mut(cid)
        {
            pstate.dpad_kind[i] = if ty == 1 && param == 0 { 0 } else { ty };
            pstate.action_slot_param[i] = param;
        }
        if std::env::var_os("BO2MP_T6_WEAPLOG").is_some() {
            let has = frame(world)
                .avatar(cid)
                .is_some_and(|pstate| param != 0 && pstate.weapons.contains(&param));
            diag::info!(Sim, "bo2zm t6 action slot {slot}: {held} (type {ty}, weapon {param}, held {has})");
        }
        let mut zm = world.resource_mut::<super::Zm>();
        let slots = zm.action_slots.entry(cid.0).or_default();
        slots.retain(|(n, _)| *n != slot);
        if !held.is_empty() {
            slots.push((slot, held));
            slots.sort();
        }
        Ok(ScriptValue::Undefined)
    });
    // Shangri-La's shrink ray kicks a shrunken zombie only while he walks.
    m!("getnormalizedmovement", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        Ok(ScriptValue::Vec3(client_script::movement(&mut frame(world), cid)))
    });
    m!("getnormalizedcameramovement", |_, _, _, _| Ok(ScriptValue::Vec3(
        [0.0; 3]
    )));
    m!("iprintln", |vm, _, _, a| {
        let t = text(vm, a, 0);
        diag::info!(Sim, "bo2zm t6 iprintln: {t}");
        Ok(ScriptValue::Undefined)
    });
    m!("iprintlnbold", |vm, _, _, a| {
        let t = text(vm, a, 0);
        diag::info!(Sim, "bo2zm t6 iprintlnbold: {t}");
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: the sight on the gun (`reflex`) while he is fully aimed in,
    // else "" (_challenges.gsc's sight kills).
    m!("getweaponoptic", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let aimed = frame(world)
            .avatar(cid)
            .is_some_and(|pstate| pstate.weapon_raise_fraction >= 1.0);
        let optic = name
            .split('+')
            .skip(1)
            .find(|t| aimed && super::T6_SIGHTS.contains(t));
        Ok(vm.string(optic.unwrap_or("")))
    });
    // bo2zm M3: going down and the game's end.
    m!("playerads", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let Some(raise) = frame(world).avatar(cid).map(|pstate| pstate.weapon_raise_fraction) else {
            return Ok(ScriptValue::Float(0.0));
        };
        Ok(ScriptValue::Float(raise))
    });
    m!("getweaponmuzzlepoint", |vm, world, s, _| {
        // A zombie's killer may be no player (another zombie, the world):
        // where it stands.
        let Ok(cid) = client(vm, world, s) else {
            return Ok(origin_of(vm, world, s).map_or(ScriptValue::Undefined, ScriptValue::Vec3));
        };
        let Some(pstate) = frame(world).avatar(cid).copied() else {
            return Ok(ScriptValue::Undefined);
        };
        let (f, _, _) = gsc_t6::math::facing_axes(pstate.look_angles);
        let eye = [
            pstate.origin[0],
            pstate.origin[1],
            pstate.origin[2] + pstate.eye_height_now,
        ];
        Ok(ScriptValue::Vec3(std::array::from_fn(|i| eye[i] + f[i] * 16.0)))
    });
    m!("getweaponforwarddir", |vm, world, s, _| {
        let cid = client(vm, world, s)?;
        let angles = frame(world).avatar(cid).map_or([0.0; 3], |pstate| pstate.look_angles);
        Ok(ScriptValue::Vec3(gsc_t6::math::facing_axes(angles).0))
    });
    // Linked to an entity: the player goes where it goes (Nuketown's
    // game-over fall is a moving script_origin).
    m!("playerlinkto", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let to = entnum(vm, arg(a, 0));
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            p.linked = to;
        }
        Ok(ScriptValue::Undefined)
    });
    // He stays where playerlinkto put him relative to the entity: our link
    // keeps him on its spot either way (it takes no entity: binding it
    // with the others unlinked him).
    m!("playerlinkedoffsetenable", |_, _, _, _| Ok(ScriptValue::Undefined));
    for name in [
        "playerlinktodelta",
        "playerlinktoabsolute",
        "playerlinktoblend",
    ] {
        vm.bind(name, true, |vm, world, s, a| {
            let cid = client(vm, world, s)?;
            let to = entnum(vm, arg(a, 0));
            if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
                p.linked = to;
            }
            Ok(ScriptValue::Undefined)
        });
    }
    m!("getlinkedent", |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let linked = {
            let zm = world.resource::<Zm>();
            zm.players
                .get(&n)
                .and_then(|p| p.linked)
                .or_else(|| zm.ents.get(&n).and_then(|e| e.link.map(|l| l.0)))
        };
        Ok(linked
            .and_then(|l| world.resource::<Zm>().ents.get(&l).and_then(|e| e.obj))
            .map_or(ScriptValue::Undefined, ScriptValue::Object))
    });
    for name in [
        "fakedamagefrom",
        "recordplayerdownzombies",
        "recordplayerrevivezombies",
        "recordplayerdeathzombies",
        "setclientuivisibilityflag",
        "setclientminiscoreboardhide",
        "setburn",
        "setelectrified",
        "setlowready",
        "giveachievement",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // bo2mp: PlayerCmd_DualGrenadesActive / Inactive set and clear his
    // eFlags2 0x100, which only the client reads (IsWeaponRetrieveable: the
    // pick-it-back-up hint for a thrown lethal while he still holds the
    // other one). That hint is not drawn here, so they do nothing.
    for name in ["dualgrenadesactive", "dualgrenadesinactive"] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    screen_effects(vm);
    let _ = arg;
    let _ = list;
}

fn clock_millis(world: &World) -> i32 {
    crate::level_time_ms(world.resource::<crate::frame_step::FrameTicket>().tick)
}

/// Blur (the game-over rocket blurs). Depth of field and shellshock are
/// bound above with the engine's own rules.
fn screen_effects(vm: &mut Vm<World>) {
    vm.bind("setblur", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let to = super::num(a, 0)?.max(0.0);
        let secs = super::num(a, 1).unwrap_or(0.0).max(0.0);
        let now = clock_millis(world);
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            let view = &mut f.client_meta_mut(id).view_effects;
            view.blur = Some(crate::ScriptBlur {
                from: view.blur.map_or(0.0, |b| b.sample(now)),
                to,
                set_ms: now,
                lasts_ms: (secs * 1000.0).round() as i32,
            });
        }
        Ok(ScriptValue::Undefined)
    });
    // `startfadingblur(blur, time)`: a blur that fades from `blur` to none.
    vm.bind("startfadingblur", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let from = super::num(a, 0)?.max(0.0);
        let secs = super::num(a, 1).unwrap_or(0.0).max(0.0);
        let now = clock_millis(world);
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).view_effects.blur = Some(crate::ScriptBlur {
                from,
                to: 0.0,
                set_ms: now,
                lasts_ms: (secs * 1000.0).round() as i32,
            });
        }
        Ok(ScriptValue::Undefined)
    });
}

fn weapon_list(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &ScriptValue,
    which: client_script::WeaponList,
) -> Result<ScriptValue, String> {
    let id = client(vm, world, s)?;
    let all = matches!(which, client_script::WeaponList::All);
    let mut ws = client_script::weapons(&frame(world), id, which);
    // His melee weapon is held too (BO2 lists it with the rest).
    let knife = held_melee(world, id);
    if all && knife != 0 && !ws.contains(&knife) {
        ws.push(knife);
    }
    let mut out = Vec::new();
    for w in ws {
        let n = super::weapon_text_of(world, Some(id.0), w);
        out.push(vm.string(&n));
    }
    Ok(list(out))
}

/// The entity a player whose ground entity is `g` stands on, as `getgroundent`
/// gives it (BO2's GScr_GetGroundEnt): nothing in the air, the world entity on
/// the map, else the player, brush mover or model mover under him (each found
/// by the number it goes to the clients as).
pub(super) fn ground_object(zm: &Zm, g: i32) -> Option<gsc_t6::ObjRef> {
    const ENTITYNUM_WORLD: i32 = playerstate_t6::game::NO_ENTITY_SLOT - 1;
    if g < 0 || g == playerstate_t6::game::NO_ENTITY_SLOT {
        return None;
    }
    let ent = |n: &u32| zm.ents.get(n).and_then(|e| e.obj);
    if g == ENTITYNUM_WORLD {
        return zm.ents.values().find(|e| e.class_tag == "worldspawn").and_then(|e| e.obj);
    }
    if let Some(p) = u32::try_from(g).ok().and_then(|c| zm.players.get(&c)) {
        return Some(p.obj);
    }
    if let Some((n, _)) = zm.brush_rows.iter().find(|(_, r)| r.number == Some(g)) {
        return ent(n);
    }
    zm.presences
        .by_ent
        .iter()
        .find(|(_, shown)| shown.num == g)
        .and_then(|(n, _)| ent(n))
}

#[cfg(test)]
mod ground_tests {
    use super::ground_object;
    use crate::zombie_rt::Zm;
    use crate::zombie_rt::brushes::BrushRow;
    use crate::zombie_rt::presence::Shown;
    use gsc_t6::ObjRef;

    fn obj(index: u32) -> ObjRef {
        ObjRef { index, generation: 1 }
    }

    fn zm() -> Zm {
        let mut zm = Zm::default();
        for (nn, cls, o) in [(1, "worldspawn", 10), (40, "script_brushmodel", 11), (41, "script_model", 12)] {
            zm.ents.insert(
                nn,
                crate::zombie_rt::Ent {
                    obj: Some(obj(o)),
                    class_tag: cls.into(),
                    ..Default::default()
                },
            );
        }
        zm.brush_rows.insert(
            40,
            BrushRow {
                spot: [0.0; 3],
                rot: [0.0; 3],
                solid: true,
                unseen: false,
                passes_for: 0,
                number: Some(300),
                moving: false,
            },
        );
        zm.presences.by_ent.insert(
            41,
            Shown {
                id: Default::default(),
                num: 301,
                spot: [0.0; 3],
                rot: [0.0; 3],
                mesh: String::new(),
                attachments: Vec::new(),
                unseen: false,
                shown_to: 0,
                moving: false,
            },
        );
        zm
    }

    /// In the air nothing; on the map the world entity; on a mover the
    /// script entity it carries, found by the number it goes out as.
    #[test]
    fn getgroundent_names_what_he_stands_on() {
        let zm = zm();
        assert_eq!(ground_object(&zm, playerstate_t6::game::NO_ENTITY_SLOT), None);
        assert_eq!(ground_object(&zm, playerstate_t6::game::NO_ENTITY_SLOT - 1), Some(obj(10)));
        assert_eq!(ground_object(&zm, 300), Some(obj(11)));
        assert_eq!(ground_object(&zm, 301), Some(obj(12)));
        assert_eq!(ground_object(&zm, 302), None);
    }
}

/// The melee weapon BO2's engine holds for him (0: none).
fn held_melee(world: &mut World, cid: crate::ClientId) -> u32 {
    let name = world
        .resource::<Zm>()
        .players
        .get(&cid.0)
        .and_then(|p| p.melee_weapon.clone());
    name.and_then(|n| weapon(world, &n).ok()).unwrap_or(0)
}
