//! Game-wide builtins: tables, session and game-mode queries, precache,
//! players lists, HUD element stand-ins, traces, and quiet stubs for what
//! has no engine side yet (client fields, effects, sounds) - those are
//! listed once in the log by `unbound` when a script reaches one not here.

use super::pairs::KeepByValue;
use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, ScriptValue, Vm};

use super::{Zm, arg, entnum, frame, int, is_player, list, text, vec3};
use crate::sim_parts::ClientId;

/// Notify `name` on `obj` after `ms` (server time).
pub(super) fn notify_later(
    _vm: &mut Vm<World>,
    world: &mut World,
    obj: ObjRef,
    ident: &str,
    ms: i64,
) {
    let mut zm = world.resource_mut::<Zm>();
    let at = zm.clock_millis + ms;
    zm.timers.push((at, obj, ident.to_owned()));
}

/// `notify_later`, unless the same notify is already on its way to `obj`.
/// BO2's engine raises `bad_path` from the actor's path state, at most once a
/// frame; one per failed `setgoalpos` call multiplied on Origins: a zombie on
/// the tank runs several breadcrumb threads (`set_zombie_on_tank` threads
/// `find_flesh` again), each woken by every `bad_path`, each queueing another,
/// till one tick ran 85 s of script.
pub(super) fn notify_later_once(
    vm: &mut Vm<World>,
    world: &mut World,
    obj: ObjRef,
    name: &str,
    ms: i64,
) {
    let pending = world
        .resource::<Zm>()
        .timers
        .iter()
        .any(|t| t.1 == obj && t.2 == name);
    if !pending {
        notify_later(vm, world, obj, name, ms);
    }
}

/// Deliver due `notify_later`s.
pub(super) fn deliver_timers(vm: &mut Vm<World>, game: &mut World, now_ms: i64) {
    let due: Vec<(i64, ObjRef, String)> = {
        let mut zm = game.resource_mut::<Zm>();
        let (due, keep): (Vec<_>, Vec<_>) = zm.timers.drain(..).partition(|t| t.0 <= now_ms);
        zm.timers = keep;
        due
    };
    for (_, o, n) in due {
        if vm.alive(o) {
            vm.notify_str(game, o, &n, &[]);
        }
    }
    let replies: Vec<(i64, ObjRef, String, String)> = {
        let mut zm = game.resource_mut::<Zm>();
        let (due, keep): (Vec<_>, Vec<_>) = zm.menu_replies.drain(..).partition(|t| t.0 <= now_ms);
        zm.menu_replies = keep;
        due
    };
    for (_, o, menu, response) in replies {
        if vm.alive(o) {
            let (m, r) = (vm.string(&menu), vm.string(&response));
            diag::info!(Sim, "bo2mp menu response (test aid): {menu} {response}");
            vm.notify_str(game, o, "menuresponse", &[m, r]);
        }
    }
}

/// Script argument `ix` read as a table row or column number.
/// BO2's GScr_SetMatchFlag names and their bits (case is ignored).
pub(super) fn match_flag_index(name: &str) -> Option<u32> {
    const FLAGS: [(&str, u32); 16] = [
        ("radar_allies", 6),
        ("radar_axis", 7),
        ("bomb_timer", 9),
        ("bomb_timer_a", 10),
        ("bomb_timer_b", 0xb),
        ("ammocounterhide", 0xc),
        ("enable_popups", 8),
        ("hud_hardcore", 0xd),
        ("pregame", 0xe),
        ("final_killcam", 4),
        ("round_end_killcam", 5),
        ("cg_drawSpectatorMessages", 0xf),
        ("disableIngameMenu", 0x10),
        ("hud_zombie", 0x36),
        ("game_ended", 0x11),
        ("overtime", 0x12),
    ];
    FLAGS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, b)| *b)
}

fn column(lhs: &[ScriptValue], ix: usize) -> Result<usize, String> {
    int(lhs, ix).map(|n| n as usize)
}

fn table_cell(game: &World, tbl: &str, row_no: usize, col: usize) -> Option<String> {
    let zm = game.resource::<Zm>();
    let t = zm.tables.get(&tbl.to_ascii_lowercase())?;
    if row_no >= t.row_total || col >= t.col_total {
        return None;
    }
    t.entries.get(row_no * t.col_total + col).cloned()
}

fn table_find(game: &World, tbl: &str, col: usize, lookup_key: &str) -> Option<usize> {
    let zm = game.resource::<Zm>();
    let t = zm.tables.get(&tbl.to_ascii_lowercase())?;
    (0..t.row_total).find(|&r| {
        t.entries
            .get(r * t.col_total + col)
            .is_some_and(|c| c.eq_ignore_ascii_case(lookup_key))
    })
}

/// bo2mp: a multiplayer game type setting from the settings files; one
/// they do not name is 0 (as the engine's default for an unset setting).
/// BO2MP_SETTING_<name>=<value> overrides one (a test aid: a short time
/// limit, a low score limit).
pub(super) fn mp_setting(settings: &BTreeMap<String, String>, set: &BTreeSet<String>, name: &str) -> ScriptValue {
    let key = name.to_ascii_lowercase();
    // A value a script set beats the test override.
    let over = (!set.contains(&key)).then(|| std::env::var(format!("BO2MP_SETTING_{key}")).ok()).flatten();
    let raw = over.as_deref().or(settings.get(&key).map(String::as_str));
    match raw {
        Some(v) if v.contains('.') => ScriptValue::Float(v.parse().unwrap_or(0.0)),
        Some(v) => ScriptValue::Int(v.parse().unwrap_or(0)),
        // BO2 plays its announcer unless a game type turns it off:
        // `_globallogic_audio.gsc:496,525` skip every line when
        // `level.allowannouncer` (this setting) is 0, so an unnamed one must
        // not read as off.
        None if key == "allowannouncer" => ScriptValue::Int(1),
        None => ScriptValue::Int(0),
    }
}

/// bo2zm M4: the game settings BO2's front end picked for this game (Custom
/// Games: starting round, difficulty, magic, headshots only), by lower-case
/// name; they win over the defaults below.
static FRONT_END_SETTINGS: std::sync::Mutex<Vec<(String, i32)>> = std::sync::Mutex::new(Vec::new());

/// bo2zm M4: the front end's settings for the next game (all of them; an
/// empty list = the defaults).
pub fn set_t6_game_settings(settings: Vec<(String, i32)>) {
    if let Ok(mut s) = FRONT_END_SETTINGS.lock() {
        *s = settings
            .into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect();
    }
}

/// Game type settings Zombies reads (`getgametypesetting`); others are 0.
fn gametype_setting(name: &str) -> ScriptValue {
    let key = name.to_ascii_lowercase();
    // (The test aid's starting round still wins.)
    if !(key == "startround" && std::env::var_os("BO2MP_T6_START_ROUND").is_some())
        && let Some(v) = FRONT_END_SETTINGS
            .lock()
            .ok()
            .and_then(|s| s.iter().find(|(k, _)| *k == key).map(|(_, v)| *v))
    {
        return ScriptValue::Int(v);
    }
    match key.as_str() {
        "teamcount" => ScriptValue::Int(1),
        "roundlimit" => ScriptValue::Int(1),
        "playernumlives" => ScriptValue::Int(0),
        "allowhitmarkers" => ScriptValue::Int(1),
        "allowfinalkillcam" => ScriptValue::Int(0),
        // BO2MP_T6_START_ROUND=<n>: start there (a test aid: runners and
        // sprinters at once).
        "startround" => ScriptValue::Int(
            std::env::var("BO2MP_T6_START_ROUND")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
        ),
        "zmdifficulty" => ScriptValue::Int(1),
        "magic" => ScriptValue::Int(1),
        // His zm/gamesettings_zstandard.cfg (and _zclassic): no hellhound
        // rounds in Nuketown (the scripts' dog AI never ran here: dog rounds
        // stood two still dogs far off for minutes).
        "allowdogs" => ScriptValue::Int(0),
        "headshotsonly" => ScriptValue::Int(0),
        "allowpowerups" => ScriptValue::Int(1),
        "cleansedloadout" => ScriptValue::Int(0),
        _ => ScriptValue::Int(0),
    }
}

/// Path nodes whose `key` (targetname by default) is `value`.
fn nodes_matching(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue]) -> Vec<ScriptValue> {
    let want = text(vm, lhs, 0);
    let key = match arg(lhs, 1) {
        ScriptValue::Undefined => "targetname".to_owned(),
        v => vm.to_text(v).to_ascii_lowercase(),
    };
    let k = vm.intern(&key);
    let objs: Vec<ObjRef> = game.resource::<Zm>().node_objs.clone();
    objs.into_iter()
        .filter(|o| matches!(vm.raw_field(*o, k), ScriptValue::Str(s) if vm.str(s) == want))
        .map(ScriptValue::Object)
        .collect()
}

/// Vehicle path nodes (`info_vehicle_node`) whose `key` is `value`.
fn vehicle_nodes(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue]) -> Vec<ScriptValue> {
    let want = text(vm, lhs, 0);
    let key = match arg(lhs, 1) {
        ScriptValue::Undefined => "targetname".to_owned(),
        v => vm.to_text(v).to_ascii_lowercase(),
    };
    let k = vm.intern(&key);
    let nodes: Vec<ObjRef> = game
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| e.class_tag.starts_with("info_vehicle_node"))
        .filter_map(|e| e.obj)
        .collect();
    nodes
        .into_iter()
        .filter(|o| matches!(vm.raw_field(*o, k), ScriptValue::Str(s) if vm.str(s) == want))
        .map(ScriptValue::Object)
        .collect()
}

/// Registers each of `names` as a script function (or, with `method`, a method) that does nothing and returns
/// undefined: calls the scripts make that have no effect on this server.
pub(super) fn bind_inert(vm: &mut Vm<World>, method: bool, names: &[&str]) {
    for &name in names {
        vm.bind(name, method, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
}

/// The script-facing name at position `code` of `names`, or "none" when the code is outside the table.
/// The array a trace native hands back: how far it got, where it stopped, the
/// surface normal there and the surface type.
pub(super) fn trace_result(vm: &mut Vm<World>, fraction: f32, position: [f32; 3], normal: [f32; 3], surface: &str) -> ScriptValue {
    let surface = vm.string(surface);
    let fields = [
        ("fraction", ScriptValue::Float(fraction)),
        ("position", ScriptValue::Vec3(position)),
        ("normal", ScriptValue::Vec3(normal)),
        ("surfacetype", surface),
    ];
    let mut arr = gsc_t6::Array::new();
    for (name, v) in fields {
        let key = gsc_t6::Key::Str(vm.intern(name));
        arr.set(key, v);
    }
    ScriptValue::array(arr)
}
fn code_name(code: i32, names: &[&'static str]) -> &'static str {
    usize::try_from(code).ok().and_then(|at| names.get(at)).copied().unwrap_or("none")
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    f!("tablelookup", |vm, world, _, lhs| {
        let table = text(vm, lhs, 0);
        let col = column(lhs, 1)?;
        let key = text(vm, lhs, 2);
        let out = column(lhs, 3)?;
        let v = table_find(world, &table, col, &key)
            .and_then(|r| table_cell(world, &table, r, out))
            .unwrap_or_default();
        Ok(vm.string(&v))
    });
    f!("tablelookupistring", |vm, world, _, lhs| {
        let table = text(vm, lhs, 0);
        let col = column(lhs, 1)?;
        let key = text(vm, lhs, 2);
        let out = column(lhs, 3)?;
        let v = table_find(world, &table, col, &key)
            .and_then(|r| table_cell(world, &table, r, out))
            .unwrap_or_default();
        Ok(ScriptValue::IStr(vm.intern(&v)))
    });
    f!("tablelookuprownum", |vm, world, _, lhs| {
        let table = text(vm, lhs, 0);
        let col = column(lhs, 1)?;
        let key = text(vm, lhs, 2);
        Ok(ScriptValue::Int(
            table_find(world, &table, col, &key).map_or(-1, |r| r as i32),
        ))
    });
    f!("tablelookupcolumnforrow", |vm, world, _, lhs| {
        let table = text(vm, lhs, 0);
        let row_no = column(lhs, 1)?;
        let col = column(lhs, 2)?;
        let v = table_cell(world, &table, row_no, col).unwrap_or_default();
        Ok(vm.string(&v))
    });
    f!("tablelookuprowcount", |vm, world, _, lhs| {
        let table = text(vm, lhs, 0).to_ascii_lowercase();
        Ok(ScriptValue::Int(
            world
                .resource::<Zm>()
                .tables
                .get(&table)
                .map_or(0, |t| t.row_total as i32),
        ))
    });
    f!("getgametypesetting", |vm, world, _, lhs| {
        let name = text(vm, lhs, 0);
        // bo2mp: lhs multiplayer game type's own settings (his files'
        // mp/gamesettings_default.cfg then mp/gamesettings_<type>.cfg).
        let zm = world.resource::<Zm>();
        if zm.mp {
            return Ok(mp_setting(&zm.gamesettings, &zm.gamesettings_set, &name));
        }
        Ok(gametype_setting(&name))
    });
    // setgametypesetting( name, value ): lhs multiplayer setting changes for
    // the rest of the match (BO2's _globallogic re-reads the score and time
    // limits from it every second).
    f!("setgametypesetting", |vm, world, _, lhs| {
        let key = text(vm, lhs, 0).to_ascii_lowercase();
        let value = match arg(lhs, 1) {
            ScriptValue::Int(n) => n.to_string(),
            ScriptValue::Float(fl) => fl.to_string(),
            v => vm.to_text(v),
        };
        let mut zm = world.resource_mut::<Zm>();
        if zm.mp && !key.is_empty() {
            zm.gamesettings.insert(key.clone(), value);
            zm.gamesettings_set.insert(key);
        }
        Ok(ScriptValue::Undefined)
    });
    // An online match: BO2's Combat Training (lhs ranked match) is lhs public one,
    // so its game types' scoreboards and allow_teamchange follow it.
    f!("sessionmodeisonlinegame", |_, world, _, _| Ok(ScriptValue::Int(i32::from(
        world.resource::<Zm>().ranked_match
    ))));
    f!("sessionmodeisprivate", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("sessionmodeisprivateonlinegame", |_, _, _, _| Ok(
        ScriptValue::Int(0)
    ));
    f!("sessionmodeissystemlink", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("sessionmodeiszombiesgame", |_, world, _, _| Ok(ScriptValue::Int(
        i32::from(!world.resource::<Zm>().mp)
    )));
    f!("sessionmodeiscampaigngame", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("gamemodeisusingxp", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("gamemodeisusingstats", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("gamemodeismode", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("ispregame", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // Zombies runs on the multiplayer side (_createfx's fx_init sets up
    // the exploders only there).
    f!("ismp", |_, _, _, _| Ok(ScriptValue::Int(1)));
    f!("issplitscreen", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("isonlinegame", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // The party size, or everyone already in (onallplayersready waits until
    // this many are playing; one means solo).
    f!("getnumexpectedplayers", |_, world, _, _| Ok(ScriptValue::Int(
        super::expected_players().max(world.resource::<Zm>().players.len() as i32)
    )));
    f!("getnumconnectedplayers", |_, world, _, _| Ok(ScriptValue::Int(
        world.resource::<Zm>().players.len() as i32
    )));
    // His profile: mature content (BO2's default on; is_mature() gates
    // heads popping off on headshots and limbs flying off).
    f!("getlocalprofileint", |vm, _, _, lhs| Ok(ScriptValue::Int(
        i32::from(
            matches!(vm.to_text(arg(lhs, 0)).as_str(), "cg_mature" | "cg_blood")
                && super::profile_mature()
        )
    )));
    f!("hasdlcavailable", |_, _, _, _| Ok(ScriptValue::Int(1)));
    // _zm_weapons::is_weapon_included asks the player too.
    m!("hasdlcavailable", |_, _, _, _| Ok(ScriptValue::Int(1)));
    // No god mode (TranZit's bus checks for it before hurting him).
    f!("isgodmode", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("getnorthyaw", |_, _, _, _| Ok(ScriptValue::Float(0.0)));
    f!("getaitriggerflags", |_, _, _, _| Ok(ScriptValue::Int(2)));
    f!("getvehicletriggerflags", |_, _, _, _| Ok(ScriptValue::Int(4)));
    f!("getfreeactorcount", |_, _, _, _| Ok(ScriptValue::Int(32)));
    f!("aretexturesloaded", |_, _, _, _| Ok(ScriptValue::Int(1)));
    f!("getutc", |_, _, _, _| {
        let s = std::time::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_secs());
        Ok(ScriptValue::Int(s as i32))
    });
    f!("getminbitcountfornum", |_, _, _, lhs| {
        let n = int(lhs, 0)?.max(0) as u32;
        Ok(ScriptValue::Int((32 - n.leading_zeros()) as i32))
    });
    f!("loadfx", |vm, world, _, lhs| {
        let name = text(vm, lhs, 0);
        let mut zm = world.resource_mut::<Zm>();
        if let Some(i) = zm.fx.iter().position(|fl| *fl == name) {
            return Ok(ScriptValue::Int(i as i32 + 1));
        }
        zm.fx.push(name);
        Ok(ScriptValue::Int(zm.fx.len() as i32))
    });
    // Only players in the game: one still connecting (before `begin`) is not
    // listed, as in the engine (_visionset_mgr::monitor reads every listed
    // player's _player_entnum, set on "connected", which follows `begin`).
    f!("getplayers", |_, world, _, _| {
        let pstate: Vec<ScriptValue> = world
            .resource::<Zm>()
            .players
            .values()
            .filter(|pt| pt.begun)
            .map(|pt| ScriptValue::Object(pt.obj))
            .collect();
        Ok(list(pstate))
    });
    // bo2mp: Scr_GetDamageableEntArray: every entity that takes damage (lhs
    // live player, lhs breakable, lhs live AI, what lhs script let take damage),
    // by entity number; given lhs centre and radius, only those within the
    // radius on X and Y (lhs radius at or under 0: all of them). The EMP
    // grenade's blast and the maps' train, crane and cable car kills.
    f!("getdamageableentarray", |_, world, _, lhs| {
        let c = arg(lhs, 0).as_vec3();
        let r = super::num(lhs, 1).unwrap_or(0.0);
        let r2 = if c.is_some() { r * r } else { 0.0 };
        let c = c.unwrap_or([0.0; 3]);
        let near = |o: [f32; 3]| {
            let (step_along_x, dy) = (c[0] - o[0], c[1] - o[1]);
            r2 <= 0.0 || step_along_x * step_along_x + dy * dy <= r2
        };
        let players: Vec<(u32, ObjRef)> = {
            let zm = world.resource::<Zm>();
            zm.players
                .iter()
                .where_value(|pt| pt.sessionstate == "playing")
                .map(|(n, pt)| (*n, pt.obj))
                .collect()
        };
        let mut found: Vec<(u32, ObjRef)> = Vec::new();
        for (n, obj) in players {
            let pstate = frame(world)
                .avatar(ClientId(n))
                .map(|pstate| (pstate.health, pstate.origin));
            if let Some((h, o)) = pstate
                && h > 0
                && near(o)
            {
                found.push((n, obj));
            }
        }
        {
            let zm = world.resource::<Zm>();
            found.extend(zm.ents.iter().filter_map(|(n, e)| {
                // A scorestreak's craft or turret takes damage too.
                let takes = e.can_damage
                    || e.destructible.is_some()
                    || zm.actors.get(n).is_some_and(|lhs| lhs.living)
                    || super::killstreaks::damage_target(world, *n);
                (takes && near(e.origin)).then_some((*n, e.obj?))
            }));
        }
        found.sort_by_key(|(n, _)| *n);
        Ok(list(
            found.into_iter().map(|(_, o)| ScriptValue::Object(o)).collect(),
        ))
    });
    f!("isplayer", |vm, world, _, lhs| Ok(ScriptValue::bool(is_player(
        vm,
        world,
        arg(lhs, 0)
    ))));
    f!("isai", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("isalive", |vm, world, _, lhs| {
        let v = arg(lhs, 0);
        if !vm.is_defined(v) {
            return Ok(ScriptValue::Int(0));
        }
        if let Some(n) = entnum(vm, v)
            && let Some(pt) = world.resource::<Zm>().players.get(&n).cloned()
        {
            let h = frame(world).avatar(ClientId(n)).map_or(0, |pstate| pstate.health);
            return Ok(ScriptValue::bool(pt.sessionstate == "playing" && h > 0));
        }
        Ok(ScriptValue::bool(entnum(vm, v).is_some()))
    });
    // An animation's length in seconds, and the times (0..1 of its length)
    // lhs notetrack of it fires (Buried's head chopper slices by them).
    f!("getanimlength", |vm, world, _, lhs| {
        let len = anim_of(vm, world, arg(lhs, 0)).map_or(0.0, |xs| {
            if xs.fps > 0.0 {
                f32::from(xs.frame_total) / xs.fps
            } else {
                0.0
            }
        });
        Ok(ScriptValue::Float(len))
    });
    f!("getnotetracktimes", |vm, world, _, lhs| {
        let note = text(vm, lhs, 1);
        let times = anim_of(vm, world, arg(lhs, 0))
            .map(|xs| {
                xs.notifies
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(&note))
                    .map(|(_, t)| ScriptValue::Float(*t))
                    .collect()
            })
            .unwrap_or_default();
        Ok(list(times))
    });
    f!("getwatcherweapons", |_, _, _, _| Ok(list(Vec::new())));
    // Weapons lhs player picks back up (their weapon file's bRetrievable). In
    // multiplayer as `<name>_mp` from the match's weapon list (BO2's
    // _weaponobjects strips the suffix itself); in Zombies the weapons BO2
    // marks retrievable (placed equipment, knives): their names, so
    // _weaponobjects makes each lhs watcher ("equip_turbine").
    f!("getretrievableweapons", |vm, world, _, _| {
        if world.resource::<Zm>().mp {
            let names = world.resource::<Zm>().retrievable_weapons.clone();
            let v = names.iter().map(|n| vm.string(n)).collect();
            return Ok(list(v));
        }
        let fl = frame(world);
        let ws: Vec<u32> = (1..fl.weapon_stats_count() as u32)
            .filter(|&w| fl.weapon_stats_entry(w).is_some_and(|r| r.retrievable))
            .collect();
        drop(fl);
        let names = ws
            .into_iter()
            .map(|w| vm.string(&super::weapon_text(world, w)))
            .collect();
        Ok(list(names))
    });
    f!("getplayerspawnid", |vm, _, _, lhs| Ok(
        entnum(vm, arg(lhs, 0)).map_or(ScriptValue::Int(0), |n| ScriptValue::Int(n as i32))
    ));
    f!("getweaponindexfromname", |vm, world, _, lhs| {
        let n = text(vm, lhs, 0);
        Ok(ScriptValue::Int(super::weapon(world, &n).map_or(0, |w| w as i32)))
    });
    for name in [
        "getweaponfiresound",
        "getweaponfiresoundplayer",
        "getweaponpickupsound",
        "getweaponpickupsoundplayer",
    ] {
        vm.bind(name, false, |vm, _, _, _| Ok(vm.string("")));
    }
    f!("iprintln", |vm, _, _, lhs| {
        let t = text(vm, lhs, 0);
        diag::info!(Sim, "bo2zm t6 iprintln: {t}");
        Ok(ScriptValue::Undefined)
    });
    f!("iprintlnbold", |vm, _, _, lhs| {
        let t = text(vm, lhs, 0);
        diag::info!(Sim, "bo2zm t6 iprintlnbold: {t}");
        Ok(ScriptValue::Undefined)
    });
    for method in [false, true] {
        vm.bind("blank", method, |_, _, _, _| Ok(ScriptValue::Undefined));
        vm.bind("issplitscreen", method, |_, _, _, _| Ok(ScriptValue::Int(0)));
    }
    for method in [false, true] {
        bind_inert(vm, method, &[
            "setspawnpointrandomvariation",
            "setentertime",
            "setclientcompass",
            "setclienthudhardcore",
            "setclientplayerpushamount",
            "setclientaimlockonpitchstrength",
            "setclientplayersprinttime",
            "disabledeathstreak",
        ]);
    }
    // setclientnumlives(n): BO2 sets his client dvar scr_numLives (the lives
    // his HUD counts down from, UIExpression.GetNumLives).
    m!("setclientnumlives", |vm, world, s, lhs| {
        if let Ok(id) = super::client(vm, world, s) {
            let n = arg(lhs, 0).as_int().unwrap_or(0);
            super::set_client_dvar(world, id.0, "scr_numLives", &n.to_string());
        }
        Ok(ScriptValue::Undefined)
    });
    f!("getvehiclenode", |vm, world, _, lhs| {
        let v = vehicle_nodes(vm, world, lhs);
        Ok(v.into_iter().next().unwrap_or_default())
    });
    f!("getvehiclenodearray", |vm, world, _, lhs| {
        let v = vehicle_nodes(vm, world, lhs);
        Ok(list(v))
    });
    f!("weaponfuellife", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("numremoteclients", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // bo2mp: BO2's spawn influencers and points, multiplayer's
    // (`spawnsystem.rs`); zombies keeps them inert.
    f!("addsphereinfluencer", |vm, world, _, lhs| {
        if !world.resource::<Zm>().mp {
            return Ok(ScriptValue::Int(1));
        }
        super::spawnsystem::add_sphere(vm, world, lhs)
    });
    f!("addcylinderinfluencer", |vm, world, _, lhs| {
        if !world.resource::<Zm>().mp {
            return Ok(ScriptValue::Int(1));
        }
        super::spawnsystem::add_cylinder(vm, world, lhs)
    });
    f!("addboxinfluencer", |_, _, _, _| Ok(ScriptValue::Int(1)));
    f!("removeinfluencer", |_, world, _, lhs| {
        if !world.resource::<Zm>().mp {
            return Ok(ScriptValue::Undefined);
        }
        super::spawnsystem::remove(world, lhs)
    });
    f!("enableinfluencer", |_, world, _, lhs| {
        if !world.resource::<Zm>().mp {
            return Ok(ScriptValue::Undefined);
        }
        super::spawnsystem::enable(world, lhs)
    });
    f!("addspawnpoints", |vm, world, _, lhs| {
        if !world.resource::<Zm>().mp {
            return Ok(ScriptValue::Undefined);
        }
        super::spawnsystem::add_points(vm, world, lhs)
    });
    f!("clearspawnpoints", |_, world, _, _| super::spawnsystem::clear_points(world));
    bind_inert(vm, false, &[
        "adddemobookmark",
        "setinitialplayersconnected",
        "addinfluencerspawnpoints",
        "setenableinfluencer",
        "spawnpointsetrandomvariation",
    ]);
    bind_inert(vm, true, &[
        "stopburning",
        "cameraactivate",
        "camerasetposition",
        "camerasetlookat",
        "logstring",
    ]);
    vm.bind("enableusability", true, |vm, world, s, _| {
        if let Ok(id) = super::client(vm, world, s) {
            let mut fl = frame(world);
            if fl.client_meta(id).is_some() {
                fl.client_meta_mut(id).controls.usability_disabled = false;
            }
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("disableusability", true, |vm, world, s, _| {
        if let Ok(id) = super::client(vm, world, s) {
            let mut fl = frame(world);
            if fl.client_meta(id).is_some() {
                fl.client_meta_mut(id).controls.usability_disabled = true;
            }
        }
        Ok(ScriptValue::Undefined)
    });
    f!("getnode", |vm, world, _, lhs| {
        let v = nodes_matching(vm, world, lhs);
        Ok(v.into_iter().next().unwrap_or_default())
    });
    f!("getnodearray", |vm, world, _, lhs| {
        let v = nodes_matching(vm, world, lhs);
        Ok(list(v))
    });
    // Path node links the scripts change (Die Rise's escape pod, ffotd fixes).
    f!("linknodes", |_, world, _, lhs| {
        if let Some((xs, y)) = node_pair(world, lhs) {
            world.resource_mut::<Zm>().nav.link(xs, y);
        }
        Ok(ScriptValue::Undefined)
    });
    f!("unlinknodes", |_, world, _, lhs| {
        if let Some((xs, y)) = node_pair(world, lhs) {
            world.resource_mut::<Zm>().nav.unlink(xs, y);
        }
        Ok(ScriptValue::Undefined)
    });
    f!("nodesarelinked", |_, world, _, lhs| {
        let linked = node_pair(world, lhs).is_some_and(|(xs, y)| world.resource::<Zm>().nav.linked(xs, y));
        Ok(ScriptValue::Int(linked as i32))
    });
    f!("deletepathnode", |_, world, _, lhs| {
        if let Some(n) = node_index(world, arg(lhs, 0)) {
            world.resource_mut::<Zm>().nav.delete(n);
        }
        Ok(ScriptValue::Undefined)
    });
    f!("getallnodes", |_, world, _, _| {
        let v = world
            .resource::<Zm>()
            .node_objs
            .iter()
            .map(|o| ScriptValue::Object(*o))
            .collect();
        Ok(list(v))
    });
    // The node nearest lhs point (Buried's ghosts and Leroy path from it).
    f!("getnearestnode", |_, world, _, lhs| {
        let pt = arg(lhs, 0).as_vec3().unwrap_or([0.0; 3]);
        let zm = world.resource::<Zm>();
        Ok(zm
            .nav
            .nearest(pt)
            .and_then(|i| zm.node_objs.get(i as usize).copied())
            .map_or(ScriptValue::Undefined, ScriptValue::Object))
    });
    // BO2's GetNodesInRadius: (origin, radius, min radius, height = 512,
    // kind, at most = 256). The nodes within the radius across and the
    // height up or down, of the kind ("Cover": every cover kind; none:
    // every kind but lhs bad node); at most that many (BO2 stops in its node
    // tree's order, ours in node order); for the sorted one nearest first
    // (3D); only then are those inside the min radius (across), without
    // links or bad dropped.
    vm.bind("getnodesinradius", false, |vm, world, _, lhs| nodes_in_radius(vm, world, lhs, false));
    vm.bind("getnodesinradiussorted", false, |vm, world, _, lhs| nodes_in_radius(vm, world, lhs, true));
    fn nodes_in_radius(vm: &mut Vm<World>, game: &mut World, lhs: &[ScriptValue], sorted: bool) -> Result<ScriptValue, String> {
        use super::nav::{NODE_COVER_KINDS, NODE_TYPE_NAMES};
        let center = arg(lhs, 0).as_vec3().unwrap_or([0.0; 3]);
        let radius = arg(lhs, 1).as_float().unwrap_or(0.0);
        let min = arg(lhs, 2).as_float().unwrap_or(0.0);
        let height = arg(lhs, 3).as_float().unwrap_or(512.0);
        let kinds: u32 = match arg(lhs, 4) {
            ScriptValue::Undefined => !1,
            _ => {
                let k = text(vm, lhs, 4);
                if k.eq_ignore_ascii_case("Cover") {
                    NODE_COVER_KINDS
                } else {
                    NODE_TYPE_NAMES
                        .iter()
                        .position(|n| n.eq_ignore_ascii_case(&k))
                        .map_or(!1, |ix| 1 << ix)
                }
            }
        };
        let most = match arg(lhs, 5) {
            ScriptValue::Undefined => 256,
            v => v.as_int().unwrap_or(256).clamp(1, 256) as usize,
        };
        let zm = game.resource::<Zm>();
        let mut hits: Vec<(f32, u32)> = zm
            .nav
            .graph
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                let d = gsc_t6::math::sub(n.origin, center);
                d[0] * d[0] + d[1] * d[1] <= radius * radius
                    && kinds & (1 << (n.ty & 31)) != 0
                    && (n.spawnflags & 1 == 0 || kinds & 1 != 0)
                    && d[2] * d[2] <= height * height
            })
            .take(most)
            .map(|(ix, n)| {
                let d = gsc_t6::math::sub(n.origin, center);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], ix as u32)
            })
            .collect();
        if sorted {
            hits.sort_by(|xs, y| xs.0.total_cmp(&y.0));
        }
        let out = hits
            .into_iter()
            .filter(|&(_, ix)| {
                let n = &zm.nav.graph[ix as usize];
                let d = gsc_t6::math::sub(n.origin, center);
                (min * min == 0.0 || min * min <= d[0] * d[0] + d[1] * d[1]) && !n.links.is_empty() && n.ty != 0
            })
            .filter_map(|hit| zm.node_objs.get(hit.1 as usize).map(|o| ScriptValue::Object(*o)))
            .collect();
        Ok(list(out))
    }
    // Every path node within lhs sphere (zm_prison::title_update_main_end
    // marks the ones by the docks `no_teleport`).
    f!("getanynodearray", |_, world, _, lhs| {
        let center = arg(lhs, 0).as_vec3().unwrap_or([0.0; 3]);
        let radius = arg(lhs, 1).as_float().unwrap_or(0.0);
        let zm = world.resource::<Zm>();
        let v = zm
            .nav
            .graph
            .iter()
            .zip(&zm.node_objs)
            .filter(|(n, _)| gsc_t6::math::length(gsc_t6::math::sub(n.origin, center)) <= radius)
            .map(|(_, o)| ScriptValue::Object(*o))
            .collect();
        Ok(list(v))
    });
    // bo2zm M3: weapon facts the scripts ask (class, fire type), by BO2's own numbers.
    f!("weaponclass", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let class = super::frame(world).weapon_raw_kinds(w).map_or(-1, |k| k.gun_kind);
        Ok(vm.string(weapon_t6::facts::gun_kind_ids::name(class)))
    });
    // A weapon's world model (the box's floating gun, lhs wall buy's).
    f!("getweaponmodel", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let model = super::frame(world)
            .weapon_world_model(w)
            .map(|(m, _)| m.to_owned())
            .unwrap_or_default();
        Ok(vm.string(&model))
    });
    // A weapon's localized name (lhs dug-up gun's pickup hint).
    f!("getweapondisplayname", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).ok();
        let key = w.and_then(|w| {
            super::frame(world)
                .weapon_display_name(w)
                .map(str::to_owned)
        });
        Ok(match key {
            Some(k) => ScriptValue::IStr(vm.intern(&k)),
            None => vm.string(""),
        })
    });
    // Weapon facts the scripts ask about lhs gun by name.
    fn facts(
        vm: &Vm<World>,
        world: &mut World,
        lhs: &[ScriptValue],
    ) -> Option<weapon_t6::combat_facts::GunBattleProfile> {
        let w = super::weapon(world, &text(vm, lhs, 0)).ok()?;
        super::frame(world).damage_profile_of(w)
    }
    f!("isweaponprimary", |vm, world, _, lhs| {
        Ok(ScriptValue::bool(
            facts(vm, world, lhs).is_some_and(|fl| fl.carry_slot_kind == 0),
        ))
    });
    f!("isweaponequipment", |vm, world, _, lhs| {
        Ok(ScriptValue::bool(
            facts(vm, world, lhs).is_some_and(|fl| fl.carry_slot_kind == 2),
        ))
    });
    f!("weaponclipsize", |vm, world, _, lhs| {
        Ok(ScriptValue::Int(facts(vm, world, lhs).map_or(0, |fl| fl.magazine_capacity)))
    });
    f!("weaponmaxammo", |vm, world, _, lhs| {
        Ok(ScriptValue::Int(facts(vm, world, lhs).map_or(0, |fl| fl.reserve_cap)))
    });
    f!("weaponstartammo", |vm, world, _, lhs| {
        Ok(ScriptValue::Int(facts(vm, world, lhs).map_or(0, |fl| fl.spawn_rounds)))
    });
    f!("weaponfiretime", |vm, world, _, lhs| {
        Ok(ScriptValue::Float(
            facts(vm, world, lhs).map_or(0.0, |fl| fl.shot_gap_ms as f32 / 1000.0),
        ))
    });
    f!("weaponreloadtime", |vm, world, _, lhs| {
        Ok(ScriptValue::Float(
            facts(vm, world, lhs).map_or(0.0, |fl| fl.reload_duration_millis as f32 / 1000.0),
        ))
    });
    f!("weaponisboltaction", |vm, world, _, lhs| {
        Ok(ScriptValue::bool(
            facts(vm, world, lhs).is_some_and(|fl| fl.manual_cycle),
        ))
    });
    f!("weaponaltweaponname", |vm, world, _, lhs| {
        let alt = facts(vm, world, lhs).map_or(0, |fl| fl.alternate_weapon);
        let name = if alt == 0 {
            "none".to_owned()
        } else {
            super::weapon_text(world, alt)
        };
        Ok(vm.string(&name))
    });
    // A Black Ops II dual-wield weapon's left-hand weapon ("none").
    f!("weapondualwieldweaponname", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let left = frame(world)
            .damage_profile_of(w)
            .map_or(0, |fl| fl.dual_wield_weapon);
        if left == 0 {
            return Ok(vm.string("none"));
        }
        let name = super::weapon_text(world, left);
        Ok(vm.string(&name))
    });
    f!("weaponisdualwield", |vm, world, _, lhs| {
        Ok(ScriptValue::bool(
            facts(vm, world, lhs).is_some_and(|fl| fl.dual_wield_weapon != 0),
        ))
    });
    f!("weaponisgasweapon", |_, _, _, _| Ok(ScriptValue::Int(0)));
    f!("weapontype", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let family = super::frame(world).damage_profile_of(w).map_or(-1, |profile| profile.gun_family_code);
        Ok(vm.string(code_name(family, &["bullet", "grenade", "projectile", "riotshield"])))
    });
    f!("weaponinventorytype", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let slot = super::frame(world).damage_profile_of(w).map_or(-1, |profile| profile.carry_slot_kind);
        Ok(vm.string(code_name(slot, &["primary", "offhand", "item", "altmode"])))
    });
    f!("weaponissemiauto", |vm, world, _, lhs| {
        let w = super::weapon(world, &text(vm, lhs, 0)).unwrap_or(0);
        let semi = super::frame(world)
            .weapon_raw_kinds(w)
            .is_some_and(|k| k.fire_mode == i32::from(weapon_t6::facts::fire_type::SINGLESHOT));
        Ok(ScriptValue::bool(semi))
    });
    f!("setroundsplayed", |vm, world, _, lhs| {
        let n = arg(lhs, 0).as_int().unwrap_or(0);
        world.resource_mut::<Zm>().games_finished = n;
        let _ = vm;
        Ok(ScriptValue::Undefined)
    });
    bind_inert(vm, false, &[
        "reportmtu",
        "recordzombieroundstart",
        "recordzombieroundend",
        "recordmatchsummaryzombieendgamedata",
        "recordplayermatchend",
        "recordzombiezone",
        "uploadstats",
        "setmatchtalkflag",
    ]);
    // HUD elements start with the engine's defaults (scripts do
    // `elem.y = elem.y - 130`).
    fn hud(vm: &mut Vm<World>) -> ScriptValue {
        let o = vm.alloc_object(ObjKind::HudElem(0));
        for (k, v) in [
            ("x", ScriptValue::Int(0)),
            ("y", ScriptValue::Int(0)),
            ("z", ScriptValue::Int(0)),
            ("alpha", ScriptValue::Float(1.0)),
            ("fontscale", ScriptValue::Float(1.0)),
            ("sort", ScriptValue::Int(0)),
            ("color", ScriptValue::Vec3([1.0, 1.0, 1.0])),
            ("glowalpha", ScriptValue::Float(0.0)),
            ("glowcolor", ScriptValue::Vec3([0.0, 0.0, 0.0])),
            ("width", ScriptValue::Int(0)),
            ("height", ScriptValue::Int(0)),
        ] {
            let fl = vm.intern(k);
            vm.set_raw_field(o, fl, v);
        }
        ScriptValue::Object(o)
    }
    for name in [
        "newhudelem",
        "newclienthudelem",
        "newteamhudelem",
        "newscorehudelem",
        "newdebughudelem",
        "newdamageindicatorhudelem",
    ] {
        vm.bind(name, false, |vm, _, _, _| Ok(hud(vm)));
    }
    m!("destroy", |vm, world, s, _| {
        if let Some(o) = s.as_obj()
            && matches!(vm.kind(o), Some(ObjKind::HudElem(_)))
        {
            vm.free_object(world, o);
        }
        Ok(ScriptValue::Undefined)
    });
    bind_inert(vm, true, &[
        "settext",
        "setvalue",
        "setshader",
        "settimer",
        "settimerup",
        "settenthstimer",
        "settenthstimerup",
        "setclock",
        "setclockup",
        "fadeovertime",
        "moveovertime",
        "changefontscaleovertime",
        "setwaypoint",
        "settargetent",
        "cleartargetent",
        "setpulsefx",
        "setcod7decodefx",
        "setredeemfx",
        "reset",
    ]);
    // Traces against the map (entities aside): bullettrace and
    // groundtrace give the engine's result array, the physics traces the
    // stopping point, the "passed" ones whether nothing was in the way.
    // A line's result: how far along it stopped, where, and the surface's normal.
    struct LineHit {
        frac: f32,
        pos: [f32; 3],
        hit_normal: [f32; 3],
    }
    fn world_trace(
        world: &mut World,
        from_pt: [f32; 3],
        end: [f32; 3],
        mask: u32,
    ) -> LineHit {
        let t = frame(world).trace_static_world(from_pt, end, [0.0; 3], [0.0; 3], mask);
        let fl = t.fraction.clamp(0.0, 1.0);
        // A clear trace stops on `end` itself: the Head Chopper only cuts
        // when the position equals its aim point exactly.
        let pos = if fl >= 1.0 { end } else { std::array::from_fn(|i| from_pt[i] + (end[i] - from_pt[i]) * fl) };
        LineHit { frac: fl, pos, hit_normal: t.normal }
    }
    // A bullet's line: the static models (Nuketown's concrete fences, the
    // cars) stop it as well, as they stop BO2's bullettrace and sight traces.
    fn shot_trace(game: &mut World, from_pt: [f32; 3], finish: [f32; 3]) -> LineHit {
        let t = frame(game).trace_static_world_models(from_pt, finish, SHOT);
        let fl = t.fraction.clamp(0.0, 1.0);
        let pos = std::array::from_fn(|i| from_pt[i] + (finish[i] - from_pt[i]) * fl);
        LineHit { frac: fl, pos, hit_normal: t.normal }
    }
    fn trace_array(
        vm: &mut Vm<World>,
        LineHit { frac: part_frac, pos: position, hit_normal: normal }: LineHit,
    ) -> ScriptValue {
        let hit = part_frac < 1.0;
        let surface = if hit { "default" } else { "none" };
        trace_result(vm, part_frac, position, if hit { normal } else { [0.0; 3] }, surface)
    }
    const SHOT: u32 = crate::shot_trace::MASK_SHOT;
    const SOLID: u32 = crate::shot_trace::PLAYER_BODY_MASK;
    f!("bullettrace", |vm, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        let r = shot_trace(world, s, e);
        Ok(trace_array(vm, r))
    });
    // BO2's Scr_GroundTrace (G_LocationalTrace; the map alone here, the
    // entities aside): contents 0x806cb3, or 0x280ecb3 when the third
    // argument says it hits characters; lhs fifth argument (of five) takes
    // out water (0x20), lhs sixth (of six) 0x10. A hit gives its surface's
    // name (Com_SurfaceTypeToName, "default" past the list); lhs miss the
    // trace's own direction as the normal and the surface "none". Detour's
    // falling deaths splash where it reads "water".
    f!("groundtrace", |vm, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        let mut mask: u32 = if int(lhs, 2)? != 0 { 0x0280_ecb3 } else { 0x0080_6cb3 };
        if lhs.len() == 5 && int(lhs, 4)? != 0 {
            mask &= !0x20;
        }
        if lhs.len() == 6 && int(lhs, 5)? != 0 {
            mask &= !0x10;
        }
        let t = frame(world).trace_static_world(s, e, [0.0; 3], [0.0; 3], mask);
        let fl = t.fraction.clamp(0.0, 1.0);
        let pos: [f32; 3] = std::array::from_fn(|i| s[i] + (e[i] - s[i]) * fl);
        let (normal, surface) = if fl < 1.0 {
            let i = ((t.sflags >> 20) & 0x3f) as usize;
            let name = if (1..=31).contains(&i) {
                weapon_t6::surface::T6_SURFACE_TYPE_NAMES.get(i).copied().unwrap_or("default")
            } else {
                "default"
            };
            (t.normal, name)
        } else {
            let d: [f32; 3] = std::array::from_fn(|i| e[i] - s[i]);
            // A zero-length trace keeps its (zero) direction as is.
            let span = d.iter().map(|c| c * c).sum::<f32>().sqrt();
            (d.map(|c| if span <= 0.0 { c } else { c / span }), "none")
        };
        Ok(trace_result(vm, fl, pos, normal, surface))
    });
    // BO2 answers the "passed" natives with lhs sight trace (SV_SightTracePoint),
    // which has no surface epsilon: lhs line that ends lhs hair above lhs floor
    // passes. Our clip trace stops 0.125 short of every surface, so lhs hit
    // counts only when the line truly reaches the plane before its end.
    fn sight_passed(game: &mut World, from_pt: [f32; 3], finish: [f32; 3]) -> bool {
        let LineHit { frac: fl, hit_normal: n, .. } = shot_trace(game, from_pt, finish);
        if fl >= 1.0 {
            return true;
        }
        let into: f32 = (0..3).map(|i| n[i] * (from_pt[i] - finish[i])).sum();
        into > 1e-6 && fl + 0.125 / into >= 1.0
    }
    // bo2mp: gettouchingvolume(origin, mins, maxs) (BO2's
    // `ScrCmd_GetTouchingVolume`): by entity number, the entities whose
    // world boxes (which reach lhs unit past their bounds) meet the box about
    // the origin, made at least as tall as it is wide: players in the game
    // and the things with contents (models, brushes, vehicles, actors,
    // thrown grenades and rockets), not triggers or script origins.
    // Drone's and Vertigo's doors ask it of themselves to set off grenades
    // and crush turrets in their way.
    f!("gettouchingvolume", |vm, world, _, lhs| {
        let o = vec3(lhs, 0)?;
        let (mins, maxs) = (vec3(lhs, 1)?, vec3(lhs, 2)?);
        let mut lo: [f32; 3] = std::array::from_fn(|i| o[i] + mins[i]);
        let mut hi: [f32; 3] = std::array::from_fn(|i| o[i] + maxs[i]);
        super::natives_ent::expand_to_width(&mut lo, &mut hi);
        let meets = |(bl, bh): ([f32; 3], [f32; 3])| {
            (0..3).all(|i| lo[i] <= bh[i] + 1.0 && bl[i] - 1.0 <= hi[i])
        };
        let (players, ents): (Vec<(u32, ObjRef)>, Vec<(u32, ObjRef)>) = {
            let zm = world.resource::<Zm>();
            let players = zm
                .players
                .iter()
                .where_value(|pt| pt.sessionstate == "playing")
                .map(|(&n, pt)| (n, pt.obj))
                .collect();
            let ents = zm
                .ents
                .iter()
                .filter(|(n, e)| {
                    let thrown = e.class_tag == "grenade" || e.class_tag == "rocket";
                    let solid = e.solid
                        && (matches!(
                            e.class_tag.as_str(),
                            "script_model" | "script_brushmodel" | "auto_turret" | "misc_turret"
                        ) || e.class_tag.starts_with("script_vehicle")
                            || zm.actors.contains_key(*n));
                    thrown || solid
                })
                .filter_map(|(&n, e)| e.obj.map(|o| (n, o)))
                .collect();
            (players, ents)
        };
        let mut hits: BTreeMap<u32, ScriptValue> = BTreeMap::new();
        for (n, obj) in players {
            if super::triggers::player_box(world, n).is_some_and(&meets) {
                hits.insert(n, ScriptValue::Object(obj));
            }
        }
        for (n, obj) in ents {
            let s = ScriptValue::Object(obj);
            if meets(link_box(vm, world, n, &s)) {
                hits.insert(n, s);
            }
        }
        Ok(list(hits.into_values().collect()))
    });
    // bo2mp: an entity's box as BO2's `SV_LinkEntity` links it (before the
    // unit it adds all round): lhs brush model's turned box; an actor's own
    // box, unturned; anything else (models, vehicles, thrown grenades,
    // rockets, turrets) lhs cube about its origin out to its box's farthest
    // corner (`RadiusFromBounds`), turned or not. A thrown grenade's box is
    // 1.5 each way (`G_InitGrenadeEntity`), lhs rocket's lhs point, lhs turret's
    // -32 -32 0 to 32 32 56 (`G_SpawnTurret`).
    fn link_box(vm: &Vm<World>, game: &mut World, nn: u32, st: &ScriptValue) -> ([f32; 3], [f32; 3]) {
        let (class, brush) = game.resource::<Zm>().ents.get(&nn).map_or((String::new(), false), |e| {
            (
                e.class_tag.clone(),
                e.class_tag == "script_brushmodel" || e.mesh.starts_with('*'),
            )
        });
        if brush {
            return super::natives_ent::abs_bounds(vm, game, st);
        }
        let o = super::origin_of(vm, game, st).unwrap_or([0.0; 3]);
        let (lo, hi) = match class.as_str() {
            "grenade" => ([-1.5; 3], [1.5; 3]),
            "rocket" => ([0.0; 3], [0.0; 3]),
            "auto_turret" | "misc_turret" => ([-32.0, -32.0, 0.0], [32.0, 32.0, 56.0]),
            _ => super::natives_ent::local_bounds(vm, game, st),
        };
        if game.resource::<Zm>().actors.contains_key(&nn) {
            return (
                std::array::from_fn(|i| o[i] + lo[i]),
                std::array::from_fn(|i| o[i] + hi[i]),
            );
        }
        let r = (0..3)
            .map(|i| lo[i].abs().max(hi[i].abs()).powi(2))
            .sum::<f32>()
            .sqrt();
        (o.map(|v| v - r), o.map(|v| v + r))
    }
    // bo2mp: absangleclamp360(lhs) (BO2's `Scr_AbsAngleClamp360`): the angle
    // brought into 0..360 (`AngleNormalize360`). Cargo's crane turns by it.
    f!("absangleclamp360", |_, _, _, lhs| {
        let fl = super::num(lhs, 0)? / 360.0;
        let r = (fl - fl.floor()) * 360.0;
        Ok(ScriptValue::Float(if r - 360.0 >= 0.0 { r - 360.0 } else { r }))
    });
    // bo2mp: getassignedteam(player) (BO2's `GScr_GetAssignedTeam`): the
    // team the party lobby put him on. There is no lobby (and BO2 gives lhs
    // bot none), so it is BO2's answer for no team, "free".
    f!("getassignedteam", |vm, _, _, lhs| {
        if entnum(vm, arg(lhs, 0)).is_none() {
            return Err("getAssignedTeam Error: param 1 is not an entity.".into());
        }
        Ok(vm.string("free"))
    });
    f!("bullettracepassed", |_, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        // bo2mp: lhs sixth argument that is true: smoke blocks it too (BO2's
        // `Scr_BulletTracePassed`).
        if lhs.get(5).is_some_and(gsc_t6::truthy)
            && super::natives_fx::fx_visibility(world, s, e) < 1.0
        {
            return Ok(ScriptValue::bool(false));
        }
        Ok(ScriptValue::bool(sight_passed(world, s, e)))
    });
    f!("sighttracepassed", |_, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        Ok(ScriptValue::bool(sight_passed(world, s, e)))
    });
    // ent damageconetrace(from[, ignore, dir, cone_deg]): 1 when lhs damage
    // trace from `from` reaches the body's middle or lhs point 15 units off
    // it (side to side, then up and down), else 0. With `dir` lhs point only
    // counts inside the cone (65 degrees unless given). The Paralyzer and
    // claymores ask this before they hurt anything.
    m!("damageconetrace", |vm, world, s, lhs| {
        let from = vec3(lhs, 0)?;
        let Some(o) = super::origin_of(vm, world, s) else {
            return Ok(ScriptValue::Float(0.0));
        };
        let body = entnum(vm, s).is_some_and(|n| {
            let zm = world.resource::<Zm>();
            zm.players.contains_key(&n) || zm.actors.contains_key(&n)
        });
        let mid = [o[0], o[1], o[2] + if body { 36.0 } else { 0.0 }];
        let (step_along_x, dy) = (from[0] - mid[0], from[1] - mid[1]);
        let len = (step_along_x * step_along_x + dy * dy).sqrt();
        let side = if len < 0.001 { [0.0, 1.0] } else { [-dy / len, step_along_x / len] };
        let points = [
            mid,
            [mid[0] + side[0] * 15.0, mid[1] + side[1] * 15.0, mid[2]],
            [mid[0] - side[0] * 15.0, mid[1] - side[1] * 15.0, mid[2]],
            [mid[0], mid[1], mid[2] + 15.0],
            [mid[0], mid[1], mid[2] - 15.0],
        ];
        let cone = vec3(lhs, 2).ok().map(|d| {
            let deg = match lhs.get(3) {
                Some(ScriptValue::Float(fl)) => *fl,
                Some(ScriptValue::Int(i)) => *i as f32,
                _ => 65.0,
            };
            (d, deg.to_radians().cos())
        });
        for pt in points {
            if let Some((d, cos)) = cone {
                let v = [pt[0] - from[0], pt[1] - from[1], pt[2] - from[2]];
                let (vl, dl) = (
                    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(),
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(),
                );
                if vl > 0.001 && dl > 0.001 && (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / (vl * dl) < cos {
                    continue;
                }
            }
            if world_trace(world, from, pt, SHOT).frac >= 1.0 {
                return Ok(ScriptValue::Float(1.0));
            }
        }
        Ok(ScriptValue::Float(0.0))
    });
    // physicstrace(start, end[, mins, maxs, ignore]): lhs trace array like
    // bullettrace's (BO2's scripts read trace["position"]), swept with the
    // box when one is given.
    f!("physicstrace", |vm, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        let (mins, maxs) = (
            vec3(lhs, 2).unwrap_or([0.0; 3]),
            vec3(lhs, 3).unwrap_or([0.0; 3]),
        );
        let t = frame(world).trace_static_world(s, e, mins, maxs, SOLID);
        let fl = if t.starts_inside {
            0.0
        } else {
            t.fraction.clamp(0.0, 1.0)
        };
        let pos = std::array::from_fn(|i| s[i] + (e[i] - s[i]) * fl);
        Ok(trace_array(vm, LineHit { frac: fl, pos, hit_normal: t.normal }))
    });
    f!("playerphysicstrace", |_, world, _, lhs| {
        let (s, e) = (vec3(lhs, 0)?, vec3(lhs, 1)?);
        let t = frame(world).trace_static_world(
            s,
            e,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            SOLID,
        );
        let fl = if t.starts_inside {
            0.0
        } else {
            t.fraction.clamp(0.0, 1.0)
        };
        Ok(ScriptValue::Vec3(std::array::from_fn(|i| {
            s[i] + (e[i] - s[i]) * fl
        })))
    });
    // bo2mp: lhs spawn on top of lhs living player (within lhs player's width,
    // lhs player's height) would telefrag him: the spawn scripts pick another.
    f!("positionwouldtelefrag", |_, world, _, lhs| {
        let pt = arg(lhs, 0).as_vec3().unwrap_or([0.0; 3]);
        Ok(ScriptValue::bool(telefrag(world, pt)))
    });
    f!("boundswouldtelefrag", |_, world, _, lhs| {
        let pt = arg(lhs, 0).as_vec3().unwrap_or([0.0; 3]);
        Ok(ScriptValue::bool(telefrag(world, pt)))
    });
    fn telefrag(world: &mut World, pt: [f32; 3]) -> bool {
        let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
        let fl = frame(world);
        ids.into_iter().any(|n| {
            fl.avatar(ClientId(n)).is_some_and(|pstate| {
                pstate.health > 0
                    && ((pstate.origin[0] - pt[0]).powi(2) + (pstate.origin[1] - pt[1]).powi(2)).sqrt() < 32.0
                    && (pstate.origin[2] - pt[2]).abs() < 72.0
            })
        })
    }
    // bo2mp: `setmatchflag(name, on)`, BO2's GScr_SetMatchFlag: the flag's
    // bit in `Zm.match_flags` (its names and indexes; an unknown name is a
    // script error). The HUD reads final_killcam and round_end_killcam
    // (BIT_FINAL_KILLCAM, BIT_ROUND_END_KILLCAM). `game_ended` also sets
    // every client's `bo2mp_game_ended` dvar, which opens the scoreboard
    // once his HUD is back (the intermission); `disableIngameMenu` (the
    // host ended the game, forceend) every client's
    // `bo2mp_disable_ingame_menu`, and Esc opens no in-game menu (BO2's
    // CL_ShouldAllowInGameMenu). BO2 matches the names in any case.
    f!("setmatchflag", |vm, world, _, lhs| {
        let flag = text(vm, lhs, 0);
        let Some(bit) = match_flag_index(&flag) else {
            return Err(format!("GScr_SetMatchFlag: Couldn't find a matching bit for flag name : {flag} "));
        };
        let on = int(lhs, 1).unwrap_or(0) != 0;
        {
            let mut zm = world.resource_mut::<Zm>();
            if on { zm.match_flags |= 1 << bit } else { zm.match_flags &= !(1 << bit) }
        }
        let dvar = match bit {
            0x11 => Some("bo2mp_game_ended"),
            0x10 => Some("bo2mp_disable_ingame_menu"),
            _ => None,
        };
        if let Some(dvar) = dvar {
            let on = if on { "1" } else { "" };
            let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
            for n in ids {
                super::set_client_dvar(world, n, dvar, on);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2mp: lhs script's vision (mpOutro at the match's end, then the map's
    // own; the EMP's flash): the frame's grade fades to it over its time.
    f!("visionsetnaked", |vm, world, _, lhs| {
        let name = text(vm, lhs, 0);
        let secs = arg(lhs, 1).as_float().unwrap_or(0.0) as f32;
        let found = super::clientfields::vision::naked(world, &name, secs);
        diag::info!(Sim, "bo2mp: visionsetnaked {name} (vision found: {found})");
        Ok(ScriptValue::Undefined)
    });
    // The game's end (end_game): the session takes it from here.
    f!("exitlevel", |_, world, _, _| {
        world.resource_mut::<crate::gsc_host::Runtime>().exit_level = true;
        Ok(ScriptValue::Undefined)
    });
    bind_inert(vm, false, &[
        "precachemodel",
        "precacheshader",
        "precachestring",
        "precacheitem",
        "precacherumble",
        "precachemenu",
        "precacheshellshock",
        "precacheanimstatedef",
        "precachevehicle",
        "precacheleaderboards",
        "precachestatusicon",
        "precacheheadicon",
        "precachelocationselector",
        "precacheturret",
        "registerclientfield",
        "codesetclientfield",
        "codesetworldclientfield",
        "codesetplayerstateclientfield",
        "makedvarserverinfo",
        "setmatchtalkflag",
        "setscoreboardcolumns",
        "setdemointermissionpoint",
        "setminimap",
        "visionsetnight",
        "setvisionsetnaked",
        "setsunlight",
        "resetsunlight",
        "setexpfog",
        "setvolfog",
        "logstring",
        "logprint",
        "recordmatchbegin",
        "gamehistorystartmatch",
        "gamehistoryfinishmatch",
        "bbpostdemostreamstatsforround",
        "uploadstats",
        "recordplayerstats",
        "resettimeout",
        "setslowmotion",
        "objective_add",
        "objective_state",
        "objective_position",
        "objective_onentity",
        "objective_delete",
        "objective_visibleteams",
        "objective_icon",
        "objective_team",
        // the capture bar's progress and who is capturing (_gameobjects,
        // Origins' capture zones): the compass draws no objectives.
        "objective_setprogress",
        "objective_setplayerusing",
        "objective_clearplayerusing",
        "objective_setgamemodeflags",
        // the MP match clock (_globallogic, Turned's zcleansed): zombies
        // shows none.
        "setgameendtime",
        "playfx",
        "playfxontag",
        "playfxontagforclients",
        "stopfx",
        "playsoundatposition",
        "playsound",
        "physicsexplosioncylinder",
        "setclientnamemode",
        "setvoipattachment",
        "deactivateclientexploder",
        "scriptmodelsuseanimtree",
        "enablezombies",
        "disablezombies",
        "setdvarbool",
        "sethideonclientwhendead",
        // bo2mp: lhs debug readout, the pregame lobby's start, the anti-cheat
        // report, the developer's menus and its log files (closed).
        "setdebugsideswitch",
        "pregamestartgame",
        "gamerepthresholdexceeded",
        "execdevgui",
        "closefile",
        "fprintfields",
        "fprintln",
        // lhs client draw distance (map mains, e.g. Tranzit)
        "setculldist",
    ]);
    // bo2mp: only league matches restrict items.
    f!("isitemrestricted", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // bo2mp: the developer's files (spawn data, createfx) never open.
    f!("openfile", |_, _, _, _| Ok(ScriptValue::Int(-1)));
    f!("freadln", |_, _, _, _| Ok(ScriptValue::Int(-1)));
    f!("fgetarg", |vm, _, _, _| Ok(vm.string("")));
    // bo2mp: _art.gsc's fog and reflection readouts: lhs server draws no
    // fog (its 17 settings read 0) and has no reflection probes.
    f!("getfogsettings", |_, _, _, _| {
        Ok(list(vec![ScriptValue::Float(0.0); 17]))
    });
    f!("getreflectionlocs", |_, _, _, _| Ok(list(Vec::new())));
    f!("getreflectionorigin", |_, _, _, _| {
        Ok(ScriptValue::Vec3([0.0; 3]))
    });
    // BO2's level switch for lhs grenade held past its fuse: off (Zombies), it goes off in
    // the hand (t6_gun's EV_GRENADE_SUICIDE) instead of killing the holder.
    vm.bind("disablegrenadesuicide", false, |_, world, _, _| {
        world.resource_mut::<Zm>().grenade_suicide_off = true;
        Ok(ScriptValue::Undefined)
    });
    vm.bind("enablegrenadesuicide", false, |_, world, _, _| {
        world.resource_mut::<Zm>().grenade_suicide_off = false;
        Ok(ScriptValue::Undefined)
    });
}

/// The animation a script `%name` value names (its xanim's timing).
fn anim_of(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Option<super::T6Anim> {
    let ScriptValue::Anim(i) = v else {
        return None;
    };
    let (_, name) = *vm.program.anims.get(*i as usize)?;
    world.resource::<Zm>().anims.get(vm.str(name)).cloned()
}

/// The path node index a script node object stands for.
fn node_index(world: &World, v: &ScriptValue) -> Option<u32> {
    let o = v.as_obj()?;
    let zm = world.resource::<Zm>();
    zm.node_objs.iter().position(|n| *n == o).map(|i| i as u32)
}

/// The two path nodes a link builtin names.
fn node_pair(world: &World, a: &[ScriptValue]) -> Option<(u32, u32)> {
    Some((node_index(world, arg(a, 0))?, node_index(world, arg(a, 1))?))
}

#[cfg(test)]
mod match_flag_tests {
    use super::match_flag_index;

    /// GScr_SetMatchFlag's bits, any case; an unknown name has none.
    #[test]
    fn match_flags_take_bo2s_bits() {
        assert_eq!(match_flag_index("final_killcam"), Some(4));
        assert_eq!(match_flag_index("ROUND_END_KILLCAM"), Some(5));
        assert_eq!(match_flag_index("game_ended"), Some(0x11));
        assert_eq!(match_flag_index("disableingamemenu"), Some(0x10));
        assert_eq!(match_flag_index("hud_zombie"), Some(0x36));
        assert_eq!(match_flag_index("killcam"), None);
    }
}
