//! bo2mp: engine functions only Black Ops II multiplayer's scripts call:
//! team scores, the match clock the HUD counts down, the map centre the
//! minimap turns about, the kill feed, and the small answers a local match
//! gives (no online services, no pregame lobby, no stats recorder).

use bevy_ecs::prelude::World;
use gsc_t6::{Array, ScriptValue, Vm};

use super::{Zm, arg, text, vec3};

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($ident:literal, $body:expr) => {
            vm.bind($ident, false, $body);
        };
    }
    macro_rules! m {
        ($ident:literal, $body:expr) => {
            vm.bind($ident, true, $body);
        };
    }
    // A round over (Search and Destroy, half time): the level starts again
    // at the next frame, `pers[]` kept when asked (`map_restart( 1 )`).
    f!("map_restart", |_, world, _, a| {
        world.resource_mut::<Zm>().pending_restart = Some(super::flag(a, 0, false));
        Ok(ScriptValue::Undefined)
    });
    // Team scores: what the score bar and the score limit read.
    f!("getteamscore", |vm, world, _, a| {
        let team = text(vm, a, 0);
        Ok(ScriptValue::Int(
            world
                .resource::<Zm>()
                .team_scores
                .get(&team)
                .copied()
                .unwrap_or(0),
        ))
    });
    f!("setteamscore", |vm, world, _, a| {
        let team = text(vm, a, 0);
        let score = arg(a, 1).as_int().unwrap_or(0);
        world.resource_mut::<Zm>().team_scores.insert(team, score);
        Ok(ScriptValue::Undefined)
    });
    // The server time the match ends at (0: no clock), for the HUD timer.
    f!("setgameendtime", |_, world, _, a| {
        world.resource_mut::<Zm>().game_end_time = arg(a, 0).as_int().unwrap_or(0);
        Ok(ScriptValue::Undefined)
    });
    f!("setmapcenter", |_, world, _, a| {
        world.resource_mut::<Zm>().map_center = vec3(a, 0)?;
        Ok(ScriptValue::Undefined)
    });
    // The kill feed: who killed whom with what (the HUD's obituary lines).
    f!("obituary", |vm, world, _, a| {
        let ident = |vm: &Vm<World>, v: &ScriptValue| match super::entnum(vm, v) {
            Some(n) => format!("player {n}"),
            None => vm.to_text(v),
        };
        let victim = ident(vm, arg(a, 0));
        let attacker = ident(vm, arg(a, 1));
        let weapon = text(vm, a, 2);
        let mod_ = text(vm, a, 3);
        diag::info!(
            Sim,
            "bo2mp obituary: {attacker} killed {victim} ({weapon}, {mod_})"
        );
        // lane C: the kill feed his HUD shows (names, not numbers; "self"
        // when nobody else killed him: a fall, his own grenade). BO2's
        // CG_Obituary names only a client: a killer that is no player (a
        // crane's crate, a trigger) writes no ident and gives no Killed By
        // card.
        let (v, k) = (player_name(vm, world, arg(a, 0)), player_name(vm, world, arg(a, 1)));
        let client = super::entnum(vm, arg(a, 1)).filter(|_| super::is_player(vm, world, arg(a, 1)));
        let own = client.is_none_or(|n| Some(n) == super::entnum(vm, arg(a, 0)));
        // The two sides, for the ident colours (yours or the other team's).
        let (kt, vt) = (player_team(vm, arg(a, 1)), player_team(vm, arg(a, 0)));
        feed(world).obituary(format!("{k}|{v}|{weapon}|{mod_}|{}|{kt}|{vt}", if own { "self" } else { "" }));
        if let (Some(dead), Some(killer)) = (super::entnum(vm, arg(a, 0)), client)
            && dead != killer
        {
            feed(world).killers.insert(dead as u32, (killer as i32, false));
        }
        Ok(ScriptValue::Undefined)
    });
    // A string as a localized-string reference (`istring("mpl_...")`).
    f!("istring", |vm, _, _, a| {
        let s = text(vm, a, 0);
        let id = vm.intern(&s);
        Ok(ScriptValue::IStr(id))
    });
    // The asset a table names: his zones hold every table the scripts ask
    // for under its own ident.
    f!("tablelookupfindcoreasset", |_, _, _, a| Ok(
        arg(a, 0).clone()
    ));
    f!("getvehicletreadfxarray", |_, _, _, _| Ok(ScriptValue::array(
        Array::new()
    )));
    // Split-screen only (BO2's engine asks whether the game mode is local
    // split-screen); a custom game is not, so the bots fill up to 18
    // players rather than 10.
    f!("islocalgame", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("ispregamegamestarted", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("setdvarint", |vm, _, _, a| {
        let ident = text(vm, a, 0).to_ascii_lowercase();
        let v = vm.to_text(arg(a, 1));
        vm.dvars.insert(ident, v);
        Ok(ScriptValue::Undefined)
    });
    f!("clientnotify", |vm, world, _, a| {
        let n = text(vm, a, 0);
        diag::info!(Sim, "bo2mp client notify: {n}");
        super::clientfields::client_notify(vm, world, &n);
        Ok(ScriptValue::Undefined)
    });
    // setheliheightpatchenabled( ident, on ): BO2's
    // GScr_SetHeliHeightPatchEnabled turns the map's `heli_height_lock`
    // patches with that targetname on or off (G_SpawnHeliHeightLock starts
    // each on); the helicopters' height lock (CM_GetHeliHeight) reads only
    // the ones on.
    f!("setheliheightpatchenabled", |vm, world, _, a| {
        let ident = text(vm, a, 0);
        let on = super::flag(a, 1, true);
        diag::info!(Sim, "bo2mp heli height patch {ident}: {}", if on { "on" } else { "off" });
        let mut zm = world.resource_mut::<Zm>();
        if on {
            zm.heli_patches_off.remove(&ident);
        } else {
            zm.heli_patches_off.insert(ident);
        }
        Ok(ScriptValue::Undefined)
    });
    let recorder_stubs: &[&str] = &[
        "matchrecorderincrementheaderstat",
        "recordgameresult",
        "recordleaguewinner",
        "recordmatchbegin",
        "recordplayerstats",
        "skillupdate",
        "changeadvertisedstatus",
        "pcserverupdateplaylist",
        "sethostmigrationstatus",
        "announcement",
    ];
    for &ident in recorder_stubs {
        vm.bind(ident, false, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // Player side: the kill camera's field of view, the rank on the
    // scoreboard, the spawn the client streams ahead (nothing to stream).
    let player_stubs: &[&str] = &[
        "setfovforkillcam",
        "setrank",
        "predictspawnpoint",
        "setpregameclass",
        "setpregameteam",
        "gamehistorystartmatch",
        "recordloadoutperksandkillstreaks",
        "bbclasschoice",
        "addgametypestat",
        "addbonuscardstat",
        // Spawn protection's on/off marks (the scripts time it themselves).
        "spawnprotectionactive",
        "spawnprotectioninactive",
        // A launcher's lock-on warnings (lane D: scorestreaks and launchers).
        "weaponlockfree",
        "weaponlocktargettooclose",
        "weaponlocknoclearance",
        "setweaponheatpercent",
        // Camo and reticle render options, the first raise animation.
        "setplayerrenderoptions",
        "initialweaponraise",
    ];
    for &ident in player_stubs {
        vm.bind(ident, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // BO2's placespawnpoint (every spawn point, as the match starts): a
    // player-sized box lifted up to 128 units (less under a ceiling), then
    // dropped straight down to the floor; the point moves there. A point the
    // map leaves in the air or inside a fence now stands on the ground, as a
    // player spawned there would. One still in solid gets BO2's warning.
    m!("placespawnpoint", |vm, world, s, _| {
        use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
        let Some(n) = super::entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(o) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
            return Ok(ScriptValue::Undefined);
        };
        let fl = super::frame(world);
        let up = fl.trace_clip(o, [o[0], o[1], o[2] + 128.0], BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
        let top = [o[0], o[1], o[2] + 128.0 * up.fraction];
        let bottom = [o[0], o[1], top[2] - 262_144.0];
        let fall = fl.trace_clip(top, bottom, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
        if fall.fraction >= 1.0 {
            // No floor under it at all: leave it where the map put it.
            return Ok(ScriptValue::Undefined);
        }
        let floor = [o[0], o[1], top[2] - 262_144.0 * fall.fraction];
        let stuck = fl.trace_clip(floor, floor, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK).starts_inside;
        drop(fl);
        if stuck {
            diag::info!(Sim, "bo2mp spawn point {n} is in solid at {:.0} {:.0} {:.0}", floor[0], floor[1], floor[2]);
        }
        if (floor[2] - o[2]).abs() > 1.0 {
            diag::info!(Sim, "bo2mp spawn point {n} placed: z {:.0} -> {:.0} at {:.0} {:.0}", o[2], floor[2], o[0], o[1]);
        }
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
            e.origin = floor;
        }
        Ok(ScriptValue::Undefined)
    });
    // ent allowtacticalinsertion(allow): BO2's ScrCmd_AllowTacticalInsertion
    // (Hardpoint bars them inside the live hill). The gun step reads it
    // (`plant::entity_blocks_tac_insert`).
    m!("allowtacticalinsertion", |vm, world, s, a| {
        let allow = arg(a, 0).as_int().unwrap_or(1) != 0;
        if let Some(n) = super::entnum(vm, s)
            && let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n)
        {
            e.no_tac_insert = !allow;
        }
        Ok(ScriptValue::Undefined)
    });
    // A weapon only some items use (none of a default class's).
    f!("isweaponspecificuse", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // Camo, reticle and player render options packed for the client: none
    // (the default look) until the class data carries them.
    m!("calcplayeroptions", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("calcweaponoptions", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // A test script's prints (BO2MP_DEBUG_GSC) land in the log; BO2's own
    // scripts only print from developer blocks, which never run.
    f!("println", |vm, _, _, a| {
        let line: Vec<String> = a.iter().map(|v| vm.to_text(v)).collect();
        diag::info!(Sim, "bo2mp println: {}", line.join(" "));
        Ok(ScriptValue::Undefined)
    });
    // A trace through the map (BO2's Scr_WorldTrace: CM_BoxTrace, the map
    // alone, solid and player clip, contents 0x10001): the result array the
    // scripts read. A hit gives its surface's ident (Com_SurfaceTypeToName,
    // "default" past the list); a miss gives the trace's own direction as
    // the normal and the surface "none" (Carrier's water reads it).
    f!("worldtrace", |vm, world, _, a| {
        const WORLDTRACE_MASK: u32 = 0x0001_0001;
        let (s, e) = (vec3(a, 0)?, vec3(a, 1)?);
        let t = super::frame(world).trace_static_world(s, e, [0.0; 3], [0.0; 3], WORLDTRACE_MASK);
        let fl = t.fraction.clamp(0.0, 1.0);
        let pos: [f32; 3] = std::array::from_fn(|i| s[i] + (e[i] - s[i]) * fl);
        let (normal, surface) = if fl < 1.0 {
            let i = ((t.sflags >> 20) & 0x3f) as usize;
            let ident = if (1..=31).contains(&i) {
                weapon_t6::surface::T6_SURFACE_TYPE_NAMES.get(i).copied().unwrap_or("default")
            } else {
                "default"
            };
            (t.normal, ident)
        } else {
            let d: [f32; 3] = std::array::from_fn(|i| e[i] - s[i]);
            let span = d.iter().map(|c| c * c).sum::<f32>().sqrt();
            let divisor = if span <= 0.0 { 1.0 } else { span };
            (d.map(|c| c / divisor), "none")
        };
        Ok(super::natives_game::trace_result(vm, fl, pos, normal, surface))
    });
    // refreshshieldattachment: t6::playeranim (the riot shield on the body).
    for ident in ["seteverhadweaponall", "stoplocalsound"] {
        vm.bind(ident, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // His view rides a tag away from his body (BO2's view-only link flag,
    // `PLF_WEAPONVIEW_ONLY`: a Lodestar pilot's), dead or alive.
    m!("isweaponviewonlylinked", |vm, world, s, _| {
        let on = super::entnum(vm, s).is_some_and(|n| {
            super::frame(world)
                .avatar(crate::sim_parts::ClientId(n))
                .is_some_and(|pstate| pstate.attach_bits & 4 != 0)
        });
        Ok(ScriptValue::bool(on))
    });
    // The spawn a player gets (`getsortedspawnpoints(point team, influencer
    // team, enemy mask, player, predicted)`): BO2's spawn system
    // (`spawnsystem.rs`) weighs the points the scripts gave the team
    // (`addspawnpoints`) by their influencers and takes the first safe one
    // of the best twenty. With no points given (or a team it cannot weigh
    // for), the older pick below: the team's points rated by the nearest
    // living enemy, one of the first three safe ones. (The scripts ask every
    // half second for each dead player's predicted spawn: the older pick
    // gives that the best-rated point and pays no sight traces.)
    f!("getsortedspawnpoints", |vm, world, _, a| {
        if let Some(o) = super::spawnsystem::sorted_point(vm, world, a)? {
            return Ok(super::list(vec![ScriptValue::Object(o)]));
        }
        let team = text(vm, a, 0);
        let player = arg(a, 3).clone();
        let predicted = super::flag(a, 4, false);
        let level = vm.level;
        // The game type's spawn point kinds: the team's (level.
        // spawn_point_team_class_names[team], e.g. mp_tdm_spawn), else every
        // kind it registered (level.spawn_point_class_names), else the
        // free-for-all kind.
        let strings = |vm: &mut Vm<World>, v: ScriptValue| -> Vec<String> {
            match v {
                ScriptValue::Array(list) => list
                    .read()
                    .values_in_order()
                    .filter_map(|val| match val {
                        ScriptValue::Str(s) => Some(vm.str(*s).to_ascii_lowercase()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        let team_kinds = {
            let fl = vm.intern("spawn_point_team_class_names");
            let key = gsc_t6::Key::Str(vm.intern(&team));
            let v = match vm.raw_field(level, fl) {
                ScriptValue::Array(arr) => arr.get(&key).unwrap_or(ScriptValue::Undefined),
                _ => ScriptValue::Undefined,
            };
            strings(vm, v)
        };
        let kinds = if team_kinds.is_empty() {
            let fl = vm.intern("spawn_point_class_names");
            let all = vm.raw_field(level, fl);
            let v = strings(vm, all);
            if v.is_empty() { vec!["mp_dm_spawn".to_owned()] } else { v }
        } else {
            team_kinds
        };
        // Every map entity of those kinds.
        let points: Vec<ScriptValue> = world
            .resource::<Zm>()
            .ents
            .values()
            .filter(|e| kinds.iter().any(|k| e.class_tag.eq_ignore_ascii_case(k)))
            .filter_map(|e| e.obj.map(ScriptValue::Object))
            .collect();
        let me = super::entnum(vm, &player);
        let my_team = me.and_then(|n| {
            let p = world.resource::<Zm>().players.get(&n)?.obj;
            let fl = vm.intern("team");
            Some(vm.to_text(&vm.raw_field(p, fl)))
        });
        // Living enemies' eyes.
        let others: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
        let mut enemies: Vec<[f32; 3]> = Vec::new();
        for n in others {
            if Some(n) == me {
                continue;
            }
            let Some(obj) = world.resource::<Zm>().players.get(&n).map(|p| p.obj) else {
                continue;
            };
            let fl = vm.intern("team");
            let t = vm.to_text(&vm.raw_field(obj, fl));
            if my_team.as_deref().is_some_and(|m| m != "free" && m == t) {
                continue;
            }
            if let Some(pstate) = super::frame(world).avatar(crate::sim_parts::ClientId(n))
                && pstate.health > 0
            {
                enemies.push([pstate.origin[0], pstate.origin[1], pstate.origin[2] + SIGHT_HEIGHT]);
            }
        }
        // spawnsystem_sight_check_max_distance, spawnsystem_sight_height_offset.
        const SIGHT_RANGE: f32 = 2600.0;
        const SIGHT_HEIGHT: f32 = 65.0;
        let mut rated: Vec<(f32, [f32; 3], ScriptValue)> = Vec::new();
        for p in points {
            let Some(o) = super::origin_of(vm, world, &p).or_else(|| {
                let ScriptValue::Object(obj) = &p else { return None };
                let fl = vm.intern("origin");
                vm.raw_field(*obj, fl).as_vec3()
            }) else {
                continue;
            };
            let score = enemies
                .iter()
                .map(|e| ((e[0] - o[0]).powi(2) + (e[1] - o[1]).powi(2)).sqrt())
                .fold(4096.0f32, f32::min);
            rated.push((score, o, p));
        }
        if rated.is_empty() {
            return Ok(super::list(Vec::new()));
        }
        rated.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
        let fl = super::frame(world);
        let alive: Vec<[f32; 3]> = fl.ordered_player_ids().into_iter()
            .filter_map(|id| fl.avatar(id).filter(|pstate| pstate.health > 0).map(|pstate| pstate.origin))
            .collect();
        // Never on top of anyone alive (the spawn would telefrag him).
        let telefrag = |o: [f32; 3]| {
            alive.iter().any(|a| {
                ((a[0] - o[0]).powi(2) + (a[1] - o[1]).powi(2)).sqrt() < 48.0 && (a[2] - o[2]).abs() < 72.0
            })
        };
        let mut safe: Vec<usize> = Vec::new();
        let (tracing, traces) = (std::time::Instant::now(), std::cell::Cell::new(0u32));
        for (i, (_, o, _)) in rated.iter().enumerate().take(if predicted { 0 } else { 20 }) {
            if telefrag(*o) {
                continue;
            }
            let eye = [o[0], o[1], o[2] + SIGHT_HEIGHT];
            let seen = enemies.iter().any(|e| {
                (e[0] - o[0]).powi(2) + (e[1] - o[1]).powi(2) <= SIGHT_RANGE * SIGHT_RANGE
                    && {
                        traces.set(traces.get() + 1);
                        true
                    }
                    && fl.trace_static_world_models(*e, eye, crate::shot_trace::MASK_SHOT)
                        .fraction
                        >= 1.0
            });
            if !seen {
                safe.push(i);
                if safe.len() == 3 {
                    break;
                }
            }
        }
        let ms = tracing.elapsed().as_secs_f64() * 1000.0;
        if ms > 10.0 {
            diag::warn!(Sim, "bo2mp spawn pick: {} sight traces took {ms:.1} ms", traces.get());
        }
        // All dangerous: the best that would not telefrag, else the best.
        let fallback = rated.iter().position(|(_, o, _)| !telefrag(*o)).unwrap_or(0);
        drop(fl);
        let pick = if safe.is_empty() { fallback } else { safe[(vm.rand_u32() as usize) % safe.len()] };
        Ok(super::list(vec![rated[pick].2.clone()]))
    });
    // The death path's small answers: no revive, rank XP waits for the
    // profile (`cloneplayer` is below).
    // Grenades: the lethal goes in the primary offhand slot (G), the
    // tactical in the secondary; each slot's class comes from the weapon's
    // own equipment row (frag, smoke, flash...).
    fn secondary_hand_category(game: &mut World, ident: &str) -> i32 {
        let Ok(w) = super::weapon(game, ident) else {
            return 0;
        };
        super::frame(game)
            .side_slot_row(w)
            .map_or(0, |fl| fl.secondary_hand_category)
    }
    m!("setoffhandsecondaryclass", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let ident = text(vm, a, 0);
        let class = match secondary_hand_category(world, &ident) {
            0 => 3,
            c => c,
        };
        if let Some(pstate) = super::frame(world).occupant_mut(id) {
            pstate.secondary_gear_alt = class;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("switchtooffhand", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let ident = text(vm, a, 0);
        let class = secondary_hand_category(world, &ident);
        if class != 0
            && let Some(pstate) = super::frame(world).occupant_mut(id)
        {
            pstate.secondary_gear_main = class;
        }
        Ok(ScriptValue::Undefined)
    });
    // Every perk of a class at once (`setperks(array)`), as setperk each.
    m!("setperks", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let ScriptValue::Array(list) = arg(a, 0) else {
            return Ok(ScriptValue::Undefined);
        };
        let names: Vec<String> = list
            .read()
            .values_in_order()
            .map(|v| vm.to_text(v))
            .collect();
        for p in names {
            crate::client_script::set_perk(&mut super::frame(world), id, &p, true);
            if let Some(pl) = world.resource_mut::<Zm>().players.get_mut(&id.0) {
                pl.perks.insert(p);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // A gun's attachment by ident (`weaponhasattachment("mk48_mp+fmj", "fmj")`).
    f!("weaponhasattachment", |vm, _, _, a| {
        let w = text(vm, a, 0);
        let att = text(vm, a, 1);
        Ok(ScriptValue::bool(w.split('+').skip(1).any(|x| x == att)))
    });
    // A gun that forbids going prone while held (`weaponblocksprone("minigun_mp")`,
    // its WeaponDef blocksProne; BG_WeaponBlocksProne).
    f!("weaponblocksprone", |vm, world, _, a| {
        let w = super::weapon(world, &text(vm, a, 0)).unwrap_or(0);
        Ok(ScriptValue::bool(w != 0 && super::frame(world).gun_facts().get(w).moves.blocks_prone))
    });
    m!("getstowedweapon", |vm, world, s, _| {
        let ident = super::entnum(vm, s)
            .and_then(|n| super::playeranim::stowed_of(world, n))
            .unwrap_or_else(|| "none".to_owned());
        Ok(vm.string(&ident))
    });
    // His held gun's alternate (an underbarrel's launcher, a Hybrid's other
    // mode), "none" without one.
    m!("getcurrentweaponaltweapon", |vm, world, s, _| {
        let Some(n) = super::entnum(vm, s) else {
            return Ok(vm.string("none"));
        };
        let held = super::frame(world).avatar(crate::sim_parts::ClientId(n)).map_or(0, |pstate| pstate.weapon);
        let alt = super::frame(world).damage_profile_of(held).map_or(0, |fl| fl.alternate_weapon);
        let ident = if alt == 0 { "none".to_owned() } else { super::weapon_text(world, alt) };
        Ok(vm.string(&ident))
    });
    m!("getvehicleoccupied", |_, _, _, _| Ok(ScriptValue::Undefined));
    f!("doesweaponreplacespawnweapon", |_, _, _, _| Ok(ScriptValue::Int(0)));
    for ident in ["gpsjammeractive", "gpsjammerinactive", "recordmultikill", "trackweaponfirenative"] {
        vm.bind(ident, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // The bomb timers (objectives are `objectives.rs`).
    vm.bind("setbombtimer", false, |_, _, _, _| Ok(ScriptValue::Undefined));
    // A spawn point's base weight, an influencer's teams (`spawnsystem.rs`).
    f!("setspawnpointsbaseweight", |_, world, _, a| super::spawnsystem::set_base_weight(world, a));
    f!("setinfluencerteammask", |_, world, _, a| super::spawnsystem::set_team_mask(world, a));
    let telemetry_stubs: &[&str] = &[
        "recordgameevent",
        "clientclaimtrigger",
        "clientreleasetrigger",
        "releaseclaimedtrigger",
        "showtoplayer",
        // Spawn logging flags (BO2's spawn telemetry).
        "setspawnclientflag",
    ];
    for &ident in telemetry_stubs {
        vm.bind(ident, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    m!("isempjammed", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // The lethal slot by weapon (One in the Chamber, Sticks and Stones give
    // the tomahawk).
    m!("setoffhandprimaryclass", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let ident = text(vm, a, 0);
        let class = match secondary_hand_category(world, &ident) {
            0 => 1,
            c => c,
        };
        if let Some(pstate) = super::frame(world).occupant_mut(id) {
            pstate.secondary_gear_main = class;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: the engine's corpse of him, holding his death animation, and
    // an entity for the scripts (`self.body`).
    m!("cloneplayer", |vm, world, s, _| {
        let Some(n) = super::entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        Ok(super::playeranim::clone_player(vm, world, n))
    });
    m!("needsrevive", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("getentitytype", |_, _, _, _| Ok(ScriptValue::Int(1)));
    m!("anyammoforweaponmodes", |_, _, _, _| Ok(ScriptValue::Int(1)));
    for ident in ["enabledeathstreak", "luinotifyeventtospectators"] {
        vm.bind(ident, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    f!("registerxp", |_, _, _, _| Ok(ScriptValue::Undefined));
    // A sound's length in ms, its longest variant's clip in his banks (the
    // announcer waits for one line before the next); -1 for one with no
    // known length, as BO2's (the announcer then waits 3.1 s, not 0.1).
    f!("soundgetplaybacktime", |vm, world, _, a| {
        let ident = text(vm, a, 0).to_ascii_lowercase();
        Ok(ScriptValue::Int(
            world.resource::<Zm>().sound_lengths.get(&ident).map_or(-1, |ms| *ms as i32),
        ))
    });
    // A weapon's blast radius (its weapon file's explosionRadius).
    f!("getweaponexplosionradius", |vm, world, _, a| {
        let w = super::weapon(world, &text(vm, a, 0)).unwrap_or(0);
        let r = if w == 0 {
            0
        } else {
            super::frame(world)
                .gadget_stats_lookup(w)
                .map_or(0, |fl| fl.blast_outer_reach.max(0))
        };
        Ok(ScriptValue::Float(r as f32))
    });
    bind_screen(vm);
}

/// A player's last LUI notifies, oldest first.
#[derive(Clone, Default)]
struct LuiLog(Vec<String>);

impl LuiLog {
    /// How many it keeps.
    const KEPT: usize = 12;

    fn push(&mut self, line: String) {
        self.0.push(line);
        let over = self.0.len().saturating_sub(Self::KEPT);
        self.0.drain(..over);
    }

    /// The `bo2mp_lui` value.
    fn joined(&self) -> String {
        self.0.join(";")
    }
}

/// bo2mp lane C: what each player's own screen shows, for his client: the
/// menus the scripts open (BO2's class menu at spawn), the LUI notifies
/// (score popups, medals, rank), the kill feed. `publish_screen` sends them
/// each tick as client dvars: `bo2mp_menu` ("<n>:<menu>", "" menu =
/// closed), `bo2mp_lui` ("<n>|<event>|<arg>|..." by `;`, the last 12),
/// `bo2mp_feed` ("<n>|<attacker>|<victim>|<weapon>|<mod>|<self>|<attacker team>|<victim team>" by `;`, the last
/// 6), `bo2mp_scores` ("<allies>,<axis>"), `bo2mp_timeleft` (whole
/// seconds, "" no clock), `bo2mp_team`, `bo2mp_gametype`, `bo2mp_teams`
/// (everyone's: "<client>:<team>,..."), `bo2mp_cols` (the scoreboard's
/// columns). `<n>` counts up
/// so the client sends each once.
#[derive(bevy_ecs::prelude::Resource, Default)]
pub(crate) struct ScreenFeed {
    seq: u32,
    menus: std::collections::BTreeMap<u32, String>,
    lui: std::collections::BTreeMap<u32, LuiLog>,
    feed: Vec<String>,
    /// The game type's scoreboard columns (`setscoreboardcolumns`).
    cols: Vec<String>,
    /// The minimap (`setminimap`): "<material>,<ul x>,<ul y>,<lr x>,<lr y>,
    /// <north yaw>".
    minimap: String,
    /// Whoever last killed each client (victim -> killer), until he is back
    /// in play: the card BO2 shows at his death, before the killcam.
    killers: std::collections::HashMap<u32, (i32, bool)>,
}

impl ScreenFeed {
    fn next(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    fn obituary(&mut self, line: String) {
        let n = self.next();
        self.feed.push(format!("{n}|{line}"));
        let len = self.feed.len();
        self.feed.drain(..len.saturating_sub(6));
    }
}

fn feed(world: &mut World) -> bevy_ecs::prelude::Mut<'_, ScreenFeed> {
    if !world.contains_resource::<ScreenFeed>() {
        world.insert_resource(ScreenFeed::default());
    }
    world.resource_mut::<ScreenFeed>()
}

/// A player's team (`self.team`: "allies", "axis", ...), "" for no player.
fn player_team(vm: &mut Vm<World>, v: &ScriptValue) -> String {
    match v {
        ScriptValue::Object(o) if super::entnum(vm, v).is_some() => {
            let f = vm.intern("team");
            vm.to_text(&vm.raw_field(*o, f))
        }
        _ => String::new(),
    }
    .replace(['|', ';'], "")
}

/// A player's name (`self.name`), else the value as text.
fn player_name(vm: &mut Vm<World>, game: &mut World, v: &ScriptValue) -> String {
    match v {
        // (`name` is the engine's field on a player, not a script one.)
        ScriptValue::Object(o) if super::entnum(vm, v).is_some() => {
            let f = vm.intern("name");
            let n = vm.get_field(game, *o, f);
            vm.to_text(&n)
        }
        other => vm.to_text(other),
    }
    .replace(['|', ';'], " ")
}

fn bind_screen(vm: &mut Vm<World>) {
    // The text effects (decode, pulse, typewriter):
    // elem setpulsefx(ms per letter, decay start ms, decay ms). The letters
    // come in, then the text decays away: BO2 takes down its objective
    // text and outcome title this way, not by alpha. (The decode's
    // scrambled letters come in as plain ones; redact is not drawn.)
    for name in ["setcod7decodefx", "setpulsefx", "settypewriterfx"] {
        vm.bind(name, true, |_, world, s, a| {
            let n = |i: usize| arg(a, i).as_float().unwrap_or(0.0) as i64;
            let fx = [world.resource::<Zm>().clock_millis, n(0), n(1), n(2)];
            if let ScriptValue::Object(o) = s
                && let Some(e) = world.resource_mut::<Zm>().huds.elems.get_mut(&o.index)
            {
                e.fx = Some(fx);
            }
            Ok(ScriptValue::Undefined)
        });
    }
    vm.bind("setredactfx", true, |_, _, _, _| Ok(ScriptValue::Undefined));
    // elem setperks(player): the engine draws that player's perk icons in
    // the element (the killcam's killer card). Not drawn yet: the element
    // shows nothing (it was the "white" square createloadouticon starts
    // it as).
    vm.bind("setperks", true, |vm, world, s, a| {
        // The list rides in the element's text as `#perks:icon|name;...`
        // (the names localized here), drawn by the client.
        let who = arg(a, 0);
        let class = match &who {
            ScriptValue::Object(p) => {
                let f = vm.intern("class_num");
                vm.raw_field(*p, f).as_int().unwrap_or(0)
            }
            _ => 0,
        };
        let card = super::loadout::perk_card(vm, world, &who, class);
        let mut table_rows = Vec::new();
        for row in card.split(';').filter(|r| !r.is_empty()) {
            let (icon, name) = row.split_once('|').unwrap_or(("", row));
            let key = vm.string(name);
            table_rows.push(format!("{icon}|{}", super::localize(vm, world, &key, &[])));
        }
        if let ScriptValue::Object(o) = s
            && let Some(e) = world.resource_mut::<Zm>().huds.elems.get_mut(&o.index)
        {
            e.shader = None;
            e.caption = if table_rows.is_empty() { String::new() } else { format!("#perks:{}", table_rows.join(";")) };
        }
        Ok(ScriptValue::Undefined)
    });
    // elem setplayernamestring(player): his name.
    vm.bind("setplayernamestring", true, |vm, world, s, a| {
        let name = player_name(vm, world, arg(a, 0));
        if let ScriptValue::Object(o) = s
            && let Some(e) = world.resource_mut::<Zm>().huds.elems.get_mut(&o.index)
        {
            e.caption = name;
        }
        Ok(ScriptValue::Undefined)
    });
    // getnorthyaw(): the map's north (its worldspawn `northyaw`).
    vm.bind("getnorthyaw", false, |vm, _, _, _| {
        Ok(ScriptValue::Float(vm.dvars.get("northyaw").and_then(|v| v.parse().ok()).unwrap_or(0.0)))
    });
    // setminimap(material, ul x, ul y, lr x, lr y): his minimap's picture
    // and the world corners it covers.
    vm.bind("setminimap", false, |vm, world, _, a| {
        let material = text(vm, a, 0);
        let n = |i: usize| arg(a, i).as_float().unwrap_or(0.0);
        let north = vm.dvars.get("northyaw").cloned().unwrap_or_else(|| "0".to_owned());
        let line = format!("{material},{},{},{},{},{north}", n(1), n(2), n(3), n(4));
        diag::info!(Sim, "bo2mp minimap: {line}");
        // Lane D: where a scorestreak's map pick lands.
        super::killstreaks::set_minimap(world, [n(1), n(2)], [n(3), n(4)], north.parse().unwrap_or(0.0));
        feed(world).minimap = line;
        Ok(ScriptValue::Undefined)
    });
    // setscoreboardcolumns(col, ...): his scoreboard's columns.
    vm.bind("setscoreboardcolumns", false, |vm, world, _, a| {
        let cols: Vec<String> = a.iter().map(|v| vm.to_text(v)).filter(|c| !c.is_empty()).collect();
        feed(world).cols = cols;
        Ok(ScriptValue::Undefined)
    });
    // openmenu(menu): BO2's own menu opens on his screen (lane C's client);
    // BO2MP_AUTOCLASS=<class> still answers the class menu a second later
    // (a test aid: hidden test matches have no one at the keyboard).
    vm.bind("openmenu", true, |vm, world, s, a| {
        let menu = text(vm, a, 0);
        let client = super::entnum(vm, s).unwrap_or(u32::MAX);
        diag::info!(Sim, "bo2mp menu open: player {client} {menu}");
        let mut f = feed(world);
        let n = f.next();
        f.menus.insert(client, format!("{n}:{menu}"));
        if let (ScriptValue::Object(o), Ok(class)) = (s, std::env::var("BO2MP_AUTOCLASS"))
            && menu.contains("changeclass")
        {
            let mut zm = world.resource_mut::<Zm>();
            let due = zm.clock_millis + 1000;
            zm.menu_replies.push((due, *o, menu, class));
        }
        Ok(ScriptValue::Undefined)
    });
    for name in ["closemenu", "closeingamemenu"] {
        vm.bind(name, true, |vm, world, s, _| {
            let client = super::entnum(vm, s).unwrap_or(u32::MAX);
            let mut f = feed(world);
            let n = f.next();
            f.menus.insert(client, format!("{n}:"));
            Ok(ScriptValue::Undefined)
        });
    }
    // self luinotifyevent(&"event", argc, args...): an event his HUD's
    // scripts handle (`score_event`, medals, rank up).
    for name in ["luinotifyevent", "luinotifyeventtoplayer"] {
        vm.bind(name, true, |vm, world, s, a| {
            let client = super::entnum(vm, s).unwrap_or(u32::MAX);
            let mut parts = vec![text(vm, a, 0)];
            parts.extend(a.iter().skip(2).map(|v| vm.to_text(v).replace(['|', ';'], " ")));
            let mut f = feed(world);
            let n = f.next();
            f.lui.entry(client).or_default().push(format!("{n}|{}", parts.join("|")));
            Ok(ScriptValue::Undefined)
        });
    }
}

/// Each tick: every player's screen values (see `ScreenFeed`) as client
/// dvars.
/// The BO2MP_SETTING_<name> test overrides, read once (not each tick).
fn setting_overrides() -> &'static [(String, String)] {
    static OVERRIDES: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    OVERRIDES.get_or_init(|| {
        std::env::vars()
            .filter_map(|(k, v)| {
                let name = k.get(..14).filter(|p| p.eq_ignore_ascii_case("BO2MP_SETTING_")).map(|_| k[14..].to_owned())?;
                Some((name, v))
            })
            .collect()
    })
}

pub(super) fn publish_screen(world: &mut World) {
    let (scores, timeleft, clients) = {
        let zm = world.resource::<Zm>();
        let score = |t: &str| zm.team_scores.get(t).copied().unwrap_or(0);
        let left = (zm.game_end_time > 0).then(|| ((i64::from(zm.game_end_time) - zm.clock_millis).max(0) / 1000).to_string());
        let clients: Vec<(u32, gsc_t6::ObjRef)> = zm.players.iter().map(|(c, p)| (*c, p.obj)).collect();
        (format!("{},{}", score("allies"), score("axis")), left.unwrap_or_default(), clients)
    };
    // The game type's settings the HUD reads (score limit, rounds), with
    // the BO2MP_SETTING_<name> test overrides the server uses.
    let settings = {
        let zm = world.resource::<Zm>();
        let overrides = setting_overrides();
        zm.gamesettings
            .iter()
            .map(|(k, v)| {
                let v = if zm.gamesettings_set.contains(k) { v } else { overrides.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map_or(v, |(_, o)| o) };
                format!("{k}={v}")
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    let teams: Vec<(u32, String, String)> = super::with_vm(world, |vm, _| {
        let f = vm.intern("team");
        let gametype = vm.dvars.get("g_gametype").cloned().unwrap_or_default();
        clients.iter().map(|(c, o)| (*c, vm.to_text(&vm.raw_field(*o, f)), gametype.clone())).collect()
    })
    .unwrap_or_default();
    let (menus, lui, feed_line, cols, minimap) = {
        let f = feed(world);
        (f.menus.clone(), f.lui.clone(), f.feed.join(";"), f.cols.join(","), f.minimap.clone())
    };
    // The minimap's range: the world distance from its middle to its edge
    // (the map's compassmaxrange, the distance across the box; none set: 2048, measured against real BO2 on Aftermath).
    let range = super::with_vm(world, |vm, _| {
        vm.dvars.iter().find(|(k, _)| k.eq_ignore_ascii_case("compassmaxrange")).map(|(_, v)| v.clone())
    })
    .flatten()
    .unwrap_or_else(|| "2048".to_owned());
    let minimap = if minimap.is_empty() { minimap } else { format!("{minimap},{range}") };
    // The killcam (`_killcam.gsc` sets `self.killcam` while one plays, and
    // the match flag final_killcam or round_end_killcam for the last one):
    // "<kind>|<killer>" with kind 1 (killcam), 2 (final killcam) or 4 (the
    // round's end killcam), "" for none. BO2's engine sets BIT_IN_KILLCAM,
    // and BIT_FINAL_KILLCAM / BIT_ROUND_END_KILLCAM from the match flags
    // (CL_UpdateUIVisibilityBits); the HUD's killcam widget shows the card
    // of `GetPredictedClientNum` (the killer).
    // Kind 3 is the card at his death before the killcam opens: he is out
    // of play, his killer known, no killcam yet.
    let killers = feed(world).killers.clone();
    let spectating: std::collections::HashSet<u32> =
        world.resource::<Zm>().players.iter().filter(|(_, p)| p.sessionstate == "spectator").map(|(c, _)| *c).collect();
    // `sessionstate` is a built-in player field the sim keeps in `Zm`, not a raw script field
    // (reading it off the object gave "undefined", so the death card never ended at the respawn).
    let states: std::collections::HashMap<u32, String> =
        world.resource::<Zm>().players.iter().map(|(c, p)| (*c, p.sessionstate.clone())).collect();
    let flags = world.resource::<Zm>().match_flags;
    let (final_on, round_end_on) = (flags & 1 << 4 != 0, flags & 1 << 5 != 0);
    let killcams: std::collections::HashMap<u32, String> = super::with_vm(world, |vm, _| {
        let (kc, sc) = (vm.intern("killcam"), vm.intern("spectatorclient"));
        let (at, ge) = (vm.intern("archivetime"), vm.intern("gameended"));
        clients
            .iter()
            .filter_map(|(c, o)| {
                if gsc_t6::truthy(&vm.raw_field(*o, kc)) {
                    let killer = vm.raw_field(*o, sc).as_float().map_or(-1, |v| v as i32);
                    return Some((*c, format!("{}|{killer}", if final_on { 2 } else if round_end_on { 4 } else { 1 })));
                }
                // The engine's own replay (a spectator with an archive time and
                // someone to watch) is a killcam even where the script left
                // `killcam` unset: the final one at the end of the match.
                let wc = vm.raw_field(*o, sc).as_float().map_or(-1, |v| v as i32);
                if spectating.contains(c) && wc >= 0 && vm.raw_field(*o, at).as_float().unwrap_or(0.0) > 0.0 {
                    let ended = gsc_t6::truthy(&vm.raw_field(vm.level, ge));
                    return Some((*c, format!("{}|{wc}", if final_on { 2 } else if round_end_on { 4 } else if ended { 2 } else { 1 })));
                }
                // (The intermission is no card: his death card goes with the match.)
                let out = !matches!(states.get(c).map_or("playing", String::as_str), "playing" | "intermission");
                killers.get(c).filter(|_| out).map(|(k, _)| (*c, format!("3|{k}")))
            })
            .collect()
    })
    .unwrap_or_default();
    // Once he was seen out of play and is back, the death card is over.
    {
        let mut f = feed(world);
        for (c, (_, seen)) in f.killers.iter_mut() {
            if killcams.contains_key(c) {
                *seen = true;
            }
        }
        f.killers.retain(|c, (_, seen)| killcams.contains_key(c) || !*seen);
    }
    // The objectives each player's HUD marks (bots have no HUD).
    let views: Vec<(u32, String)> =
        teams.iter().filter(|(c, _, _)| !super::is_bot(world, *c)).map(|(c, t, _)| (*c, t.clone())).collect();
    super::goals::post_goals(world, &views);
    // Everyone's team (the scoreboard's sides): "<client>:<team>,...".
    // A game that is not team based has one side, TEAM_FREE, as in BO2: the
    // script still hands each player "allies" or "axis", and the board would
    // then list only the players on one of them.
    let teambased = super::with_vm(world, |vm, _| {
        let f = vm.intern("teambased");
        gsc_t6::truthy(&vm.raw_field(vm.level, f))
    })
    .is_none_or(|on| on);
    let all_teams = teams
        .iter()
        .map(|(c, t, _)| if teambased { format!("{c}:{t}") } else { format!("{c}:free") })
        .collect::<Vec<_>>()
        .join(",");
    for (c, team, gametype) in teams {
        if super::is_bot(world, c) {
            continue;
        }
        for (k, v) in [
            ("bo2mp_scores", scores.clone()),
            ("bo2mp_timeleft", timeleft.clone()),
            ("bo2mp_team", team),
            ("bo2mp_gametype", gametype),
            ("bo2mp_menu", menus.get(&c).cloned().unwrap_or_default()),
            ("bo2mp_lui", lui.get(&c).map(LuiLog::joined).unwrap_or_default()),
            ("bo2mp_feed", feed_line.clone()),
            ("bo2mp_teams", all_teams.clone()),
            ("bo2mp_cols", cols.clone()),
            ("bo2mp_settings", settings.clone()),
            ("bo2mp_minimap", minimap.clone()),
            ("bo2mp_killcam", killcams.get(&c).cloned().unwrap_or_default()),
        ] {
            super::set_client_dvar(world, c, k, &v);
        }
    }
}
