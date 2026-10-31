//! Another map's special weapons on any map (his 10-14 "Can't I test the
//! thunder gun or hacker on any map?"): the Special Weapons tab's Hell's
//! Retriever on Nuketown. The assets bring the weapons, the scripts that run
//! them and their effects (`assets` t6_m2 `DONOR_ZONES`); this does what
//! that map's own scripts would have: its `level._effect` rows at the level
//! start, and each weapon's setup (`_zm_weap_tomahawk::init`,
//! `_zm_weap_blundersplat::init`, zm_prison's melee list) at its first give.
//! Their init is not run whole: the tomahawk's threads Mob's pickup, which
//! waits on Mob's wolf heads and empties `level.zombie_weapons_no_max_ammo`.

use bevy_ecs::prelude::World;
use gsc_t6::{Array, Key, ObjRef, ScriptValue, Vm};

use super::Zm;

/// At the level start, after the map's main: each donor effect row the map
/// left unset, loaded as `loadfx` would.
pub(crate) fn set_effects(vm: &mut Vm<World>, world: &mut World) {
    let table_rows = world.resource::<Zm>().donor_effects.clone();
    if table_rows.is_empty() {
        return;
    }
    let k = vm.intern("_effect");
    let table = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a,
        _ => {
            let v = ScriptValue::array(Array::new());
            vm.set_raw_field(vm.level, k, v.clone());
            let ScriptValue::Array(a) = v else {
                return;
            };
            a
        }
    };
    let mut set = 0usize;
    for (key, fx) in &table_rows {
        let key = Key::Str(vm.intern(key));
        if table.get(&key).is_some() {
            continue;
        }
        let mut zm = world.resource_mut::<Zm>();
        let id = match zm.fx.iter().position(|f| f == fx) {
            Some(i) => i + 1,
            None => {
                zm.fx.push(fx.clone());
                zm.fx.len()
            }
        };
        table.write().set(key, ScriptValue::Int(id as i32));
        set += 1;
    }
    diag::info!(
        Sim,
        "bo2zm t6: {set} of {} other maps' weapon effects set",
        table_rows.len()
    );
}

/// Before the Special Weapons tab gives `name` to `p`: what its own map's
/// scripts would have set up, on a map whose scripts never did. Once a game.
pub(crate) fn before_give(vm: &mut Vm<World>, game: &mut World, pt: ObjRef, label: &str) {
    let level = ScriptValue::Object(vm.level);
    // zm_prison offhand_weapon_overrride: the spoon and spork are melee
    // weapons there, so weapon_give swaps the knife for them.
    if matches!(label, "spoon_zm_alcatraz" | "spork_zm_alcatraz") {
        let n = vm.string(label);
        vm.spawn_named(
            game,
            "maps/mp/zombies/_zm_utility",
            "register_melee_weapon_for_level",
            level,
            vec![n],
        );
        return;
    }
    if label.starts_with("thundergun") {
        thundergun(vm, game, pt);
        return;
    }
    if label == "equip_hacker_zm" {
        hacker(vm, game, pt);
        return;
    }
    if label == "jetgun_zm" {
        jetgun(vm, game, pt);
        return;
    }
    if label == "emp_grenade_zm" {
        emp(vm, game, pt);
        return;
    }
    if label == "time_bomb_zm" {
        time_bomb(vm, game);
        return;
    }
    if label == "riotshield_zm" {
        riotshield(vm, game, pt);
        return;
    }
    if label == "beacon_zm" {
        beacon(vm, game);
        return;
    }
    if label == "one_inch_punch_zm" {
        if first_time(vm, "bo2zm_foreign_one_inch_punch") && game.resource::<Zm>().map != "zm_tomb" {
            let script = "maps/mp/zombies/_zm_weap_one_inch_punch";
            vm.spawn_named(game, script, "one_inch_precache", level, vec![]);
            let map = game.resource::<Zm>().map.clone();
            diag::info!(Sim, "bo2zm t6: {script} set up for one_inch_punch_zm on {map}");
        }
        return;
    }
    if let Some(row) = BO1.iter().find(|r| r.weapon == label) {
        bo1(vm, game, pt, row);
        return;
    }
    if PLACED.iter().any(|e| e.0 == label) {
        placed(vm, game, pt, label);
        return;
    }
    if label.starts_with("slipgun") {
        slipgun(vm, game, pt);
        return;
    }
    if label.starts_with("slowgun") || label.starts_with("raygun_mark2") {
        buried_guns(vm, game, pt, label);
        return;
    }
    if label.starts_with("staff_") {
        staffs(vm, game, pt);
        return;
    }
    let (script, once) = if label.ends_with("tomahawk_zm") {
        ("maps/mp/zombies/_zm_weap_tomahawk", "bo2zm_foreign_tomahawk")
    } else if label.starts_with("blunder") {
        ("maps/mp/zombies/_zm_weap_blundersplat", "bo2zm_foreign_blundergat")
    } else {
        return;
    };
    let map = game.resource::<Zm>().map.clone();
    // The map's own scripts set it up (add_zombie_weapon): nothing to do.
    let first = if script.ends_with("tomahawk") { "bouncing_tomahawk_zm" } else { "blundergat_zm" };
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern(first))).is_some(),
        _ => false,
    };
    let once = vm.intern(once);
    if ran || gsc_t6::truthy(&vm.raw_field(vm.level, once)) {
        return;
    }
    vm.set_raw_field(vm.level, once, ScriptValue::Int(1));
    let weapons = "maps/mp/zombies/_zm_weapons";
    let call = |vm: &mut Vm<World>, game: &mut World, script: &str, func: &str, args: Vec<ScriptValue>| {
        vm.spawn_named(game, script, func, level.clone(), args);
    };
    if script.ends_with("tomahawk") {
        // _zm_weap_tomahawk::init, without Mob's pickup.
        for w in ["bouncing_tomahawk_zm", "upgraded_tomahawk_zm", "zombie_tomahawk_flourish"] {
            let w = vm.string(w);
            call(vm, game, weapons, "include_zombie_weapon", vec![w, ScriptValue::Int(0)]);
        }
        for w in ["bouncing_tomahawk_zm", "upgraded_tomahawk_zm"] {
            let args = vec![
                vm.string(w),
                vm.string("zombie_tomahawk_flourish"),
                ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_SATCHEL_2000")),
                ScriptValue::Int(2000),
                vm.string("wpck_monkey"),
                vm.string(""),
                ScriptValue::Undefined,
                ScriptValue::Int(1),
            ];
            call(vm, game, weapons, "add_zombie_weapon", args);
        }
        let k = vm.intern("zombie_weapons_no_max_ammo");
        let no_max = match vm.raw_field(vm.level, k) {
            ScriptValue::Array(a) => a,
            _ => {
                let v = ScriptValue::array(Array::new());
                vm.set_raw_field(vm.level, k, v.clone());
                let ScriptValue::Array(a) = v else {
                    return;
                };
                a
            }
        };
        for w in ["bouncing_tomahawk_zm", "upgraded_tomahawk_zm"] {
            let key = Key::Str(vm.intern(w));
            no_max.write().set(key, ScriptValue::Int(1));
        }
        let k = vm.intern("a_tomahawk_pickup_funcs");
        if vm.raw_field(vm.level, k).is_undefined() {
            vm.set_raw_field(vm.level, k, ScriptValue::array(Array::new()));
        }
        vm.spawn_named(game, script, "tomahawk_on_player_connect", ScriptValue::Object(pt), vec![]);
    } else {
        // zm_prison's include_weapons and add_zombie_weapon_prison rows (out
        // of the box here), then _zm_weap_blundersplat::init.
        for w in [
            "blundergat_zm",
            "blundergat_upgraded_zm",
            "blundersplat_zm",
            "blundersplat_upgraded_zm",
        ] {
            let w = vm.string(w);
            call(vm, game, weapons, "include_zombie_weapon", vec![w, ScriptValue::Int(0)]);
        }
        for (w, up, vox) in [
            ("blundergat_zm", "blundergat_upgraded_zm", ScriptValue::Int(1)),
            ("blundersplat_zm", "blundersplat_upgraded_zm", ScriptValue::Undefined),
        ] {
            let args = vec![
                vm.string(w),
                vm.string(up),
                ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_BLUNDERGAT")),
                ScriptValue::Int(500),
                vm.string("wpck_shot"),
                vm.string(""),
                ScriptValue::Undefined,
                vox,
            ];
            call(vm, game, weapons, "add_zombie_weapon", args);
        }
        call(vm, game, script, "init", vec![]);
        // blundersplat_on_player_connect's two watchers, past the spawn they
        // wait for (he spawned long ago).
        for watch in ["wait_for_blundersplat_fired", "wait_for_blundersplat_upgraded_fired"] {
            if let Some(t) = vm.spawn_named(game, script, watch, ScriptValue::Object(pt), vec![]) {
                vm.notify_thread(game, t, pt, "spawned_player", &[]);
            }
        }
        // init arms the zombies spawned from now on; these were up already.
        let alive: Vec<ObjRef> = game
            .resource::<Zm>()
            .actors
            .values()
            .filter(|a| a.living)
            .filter_map(|a| a.obj)
            .collect();
        for z in alive {
            if vm.alive(z) {
                vm.spawn_named(game, script, "zombie_wait_for_blundersplat_hit", ScriptValue::Object(z), vec![]);
            }
        }
    }
    diag::info!(Sim, "bo2zm t6: {script} set up for {label} on {map}");
}

/// Once a game, true the first time: `once` set on the level.
fn first_time(vm: &mut Vm<World>, once: &str) -> bool {
    let once = vm.intern(once);
    if gsc_t6::truthy(&vm.raw_field(vm.level, once)) {
        return false;
    }
    vm.set_raw_field(vm.level, once, ScriptValue::Int(1));
    true
}

/// The Thunder Gun off Kino (BO1): Kino's include_weapons and
/// add_zombie_weapon rows, then BO2's own `_zm_weap_thundergun::init`
/// (its effects, ranges and gib list) and its fire watcher, past the spawn
/// it waits for.
fn thundergun(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("thundergun_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_thundergun") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let weapons = "maps/mp/zombies/_zm_weapons";
    for w in ["thundergun_zm", "thundergun_upgraded_zm"] {
        let w = vm.string(w);
        vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w, ScriptValue::Int(0)]);
    }
    let args = vec![
        vm.string("thundergun_zm"),
        vm.string("thundergun_upgraded_zm"),
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_THUNDERGUN")),
        ScriptValue::Int(10),
        vm.string("wunder"),
        vm.string(""),
        ScriptValue::Undefined,
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    let script = "maps/mp/zombies/_zm_weap_thundergun";
    vm.spawn_named(game, script, "init", level, vec![]);
    if let Some(t) = vm.spawn_named(game, script, "wait_for_thundergun_fired", ScriptValue::Object(pt), vec![]) {
        vm.notify_thread(game, t, pt, "spawned_player", &[]);
    }
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for thundergun_zm on {map}");
}

/// The Hacker off Moon (BO1): the equipment included, BO2's own
/// `_zm_equip_hacker::init`, and Moon's hackables that every map has (wall
/// guns, perks, Pack-a-Punch, boards, doors, power-ups, the box).
fn hacker(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("_hackable_objects");
    if !vm.raw_field(vm.level, k).is_undefined() || !first_time(vm, "bo2zm_foreign_hacker") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string("equip_hacker_zm");
    // Moon's offhand list: without it weapon_give hands it over as a plain
    // gun, equipment_give never runs and it is never switched on to hack.
    vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_equipment_for_level", level.clone(), vec![w.clone()]);
    vm.spawn_named(game, "maps/mp/zombies/_zm_equipment", "include_zombie_equipment", level.clone(), vec![w]);
    vm.spawn_named(game, "maps/mp/zombies/_zm_equip_hacker", "init", level.clone(), vec![]);
    for (script, func) in [
        ("_zm_hackables_wallbuys", "hack_wallbuys"),
        ("_zm_hackables_perks", "hack_perks"),
        ("_zm_hackables_packapunch", "hack_packapunch"),
        ("_zm_hackables_boards", "hack_boards"),
        ("_zm_hackables_doors", "hack_doors"),
        ("_zm_hackables_powerups", "hack_powerups"),
        ("_zm_hackables_box", "box_hacks"),
    ] {
        vm.spawn_named(game, &format!("maps/mp/zombies/{script}"), func, level.clone(), vec![]);
    }
    vm.spawn_named(game, "maps/mp/zombies/_zm_equip_hacker", "hacker_on_player_connect", ScriptValue::Object(pt), vec![]);
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: _zm_equip_hacker set up for equip_hacker_zm on {map}");
}

/// The Sliquifier off Die Rise: zm_highrise's include_weapons and
/// add_zombie_weapon rows, then `_zm_weap_slipgun::init` (its goo, chain
/// and damage callbacks) and its bolt watcher for the player already in.
fn slipgun(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("slipgun_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_slipgun") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let weapons = "maps/mp/zombies/_zm_weapons";
    for w in ["slipgun_zm", "slipgun_upgraded_zm"] {
        let w = vm.string(w);
        vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w, ScriptValue::Int(0)]);
    }
    let args = vec![
        vm.string("slipgun_zm"),
        ScriptValue::Undefined,
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_SLIPGUN")),
        ScriptValue::Int(10),
        vm.string("slip"),
        vm.string(""),
        ScriptValue::Undefined,
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    let script = "maps/mp/zombies/_zm_weap_slipgun";
    vm.spawn_named(game, script, "init", level, vec![]);
    vm.spawn_named(game, script, "watch_for_slip_bolt", ScriptValue::Object(pt), vec![]);
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for slipgun_zm on {map}");
}

/// Buried's guns: zm_buried's include_weapons and add_zombie_weapon rows for
/// the Ray Gun Mark II and the Paralyzer, then `_zm_weap_slowgun::init` (its
/// clientfields, damage callbacks and anim-rate watchers) for the player
/// already in and the zombies already up.
fn buried_guns(vm: &mut Vm<World>, game: &mut World, pt: ObjRef, label: &str) {
    let first = if label.starts_with("slowgun") { "slowgun_zm" } else { "raygun_mark2_zm" };
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern(first))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, &format!("bo2zm_foreign_{first}")) {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let weapons = "maps/mp/zombies/_zm_weapons";
    let (up, hint, cost, vo) = if first == "slowgun_zm" {
        ("slowgun_upgraded_zm", "ZOMBIE_WEAPON_SLOWGUN", 10, "wpck_paralyzer")
    } else {
        ("raygun_mark2_upgraded_zm", "ZOMBIE_WEAPON_RAYGUN_MARK2", 10000, "pickup_raymk2")
    };
    for w in [first, up] {
        let w = vm.string(w);
        vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w, ScriptValue::Int(0)]);
    }
    let args = vec![
        vm.string(first),
        vm.string(up),
        ScriptValue::IStr(vm.intern(hint)),
        ScriptValue::Int(cost),
        vm.string(vo),
        vm.string(""),
        ScriptValue::Undefined,
        ScriptValue::Int(1),
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    let map = game.resource::<Zm>().map.clone();
    if first != "slowgun_zm" {
        diag::info!(Sim, "bo2zm t6: zm_buried rows set up for {first} on {map}");
        return;
    }
    let script = "maps/mp/zombies/_zm_weap_slowgun";
    vm.spawn_named(game, script, "init", level, vec![]);
    // slowgun_player_connect's three watchers, the fire one past the spawn it waits for.
    if let Some(t) = vm.spawn_named(game, script, "watch_slowgun_fired", ScriptValue::Object(pt), vec![]) {
        vm.notify_thread(game, t, pt, "spawned_player", &[]);
    }
    for watch in ["watch_reset_anim_rate", "sndwatchforweapswitch"] {
        vm.spawn_named(game, script, watch, ScriptValue::Object(pt), vec![]);
    }
    let alive: Vec<ObjRef> = game
        .resource::<Zm>()
        .actors
        .values()
        .filter(|a| a.living)
        .filter_map(|a| a.obj)
        .collect();
    for z in alive {
        if vm.alive(z) {
            vm.spawn_named(game, script, "slowgun_on_zombie_spawned", ScriptValue::Object(z), vec![]);
        }
    }
    diag::info!(Sim, "bo2zm t6: {script} set up for slowgun_zm on {map}");
}

/// Origins' staffs: zm_tomb's include_weapons and add_zombie_weapon rows, its
/// staff melee weapons and charge-shot ammo setting, then each staff's init
/// and its watchers for the player already in (the fire and revive ones past
/// the spawn they wait for). zm_tomb's own damage callback (no hurt from a
/// staff) is player_damage's `bo2zm_foreign_staffs` check.
fn staffs(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("staff_air_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_staffs") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let weapons = "maps/mp/zombies/_zm_weapons";
    for (w, hint) in [
        ("staff_air_zm", "AIR_STAFF"),
        ("staff_air_upgraded_zm", "AIR_STAFF_CHARGED"),
        ("staff_fire_zm", "FIRE_STAFF"),
        ("staff_fire_upgraded_zm", "FIRE_STAFF_CHARGED"),
        ("staff_lightning_zm", "LIGHTNING_STAFF"),
        ("staff_lightning_upgraded_zm", "LIGHTNING_STAFF_CHARGED"),
        ("staff_water_zm", "WATER_STAFF"),
        ("staff_water_zm_cheap", "WATER_STAFF"),
        ("staff_water_upgraded_zm", "WATER_STAFF_CHARGED"),
        ("staff_revive_zm", "ZM_TOMB_WEAP_STAFF_REVIVE"),
    ] {
        let n = vm.string(w);
        vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![n, ScriptValue::Int(0)]);
        let args = vec![
            vm.string(w),
            ScriptValue::Undefined,
            ScriptValue::IStr(vm.intern(hint)),
            ScriptValue::Int(50),
            vm.string("wpck_rpg"),
            vm.string(""),
            ScriptValue::Undefined,
            ScriptValue::Int(1),
        ];
        vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    }
    for m in ["staff_air_melee_zm", "staff_fire_melee_zm", "staff_lightning_melee_zm", "staff_water_melee_zm"] {
        let m = vm.string(m);
        vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_melee_weapon_for_level", level.clone(), vec![m]);
    }
    // zm_tomb.gsc: a charge level costs more than one round (charge_shot::sync_dvars).
    vm.dvars.insert("tu14_bg_chargeshotexponentialammoperchargelevel".to_owned(), "1".to_owned());
    for s in ["air", "fire", "lightning", "water", "revive"] {
        let script = format!("maps/mp/zombies/_zm_weap_staff_{s}");
        vm.spawn_named(game, &script, "init", level.clone(), vec![]);
        if let Some(t) = vm.spawn_named(game, &script, "onplayerspawned", ScriptValue::Object(pt), vec![])
            && matches!(s, "fire" | "revive")
        {
            vm.notify_thread(game, t, pt, "spawned_player", &[]);
        }
    }
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: zm_tomb staffs set up on {map}");
}

/// The EMP grenade off TranZit: zm_transit's tactical and weapon rows, its
/// inert-zombie setup (`_zm_ai_basic::init_inert_zombies`), then
/// `_zm_weap_emp_bomb::init` and its throw watcher for the player already in.
fn emp(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("emp_grenade_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_emp") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string("emp_grenade_zm");
    vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_tactical_grenade_for_level", level.clone(), vec![w.clone()]);
    let weapons = "maps/mp/zombies/_zm_weapons";
    vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w.clone(), ScriptValue::Int(0)]);
    let args = vec![
        w,
        ScriptValue::Undefined,
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_EMP_GRENADE")),
        ScriptValue::Int(2000),
        vm.string("wpck_emp"),
        vm.string(""),
        ScriptValue::Undefined,
        ScriptValue::Int(1),
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    let k = vm.intern("inert_substates");
    if matches!(vm.raw_field(vm.level, k), ScriptValue::Undefined) {
        vm.spawn_named(game, "maps/mp/zombies/_zm_ai_basic", "init_inert_zombies", level.clone(), vec![]);
    }
    let script = "maps/mp/zombies/_zm_weap_emp_bomb";
    vm.spawn_named(game, script, "init", level, vec![]);
    vm.spawn_named(game, script, "watch_for_grenade_throw", ScriptValue::Object(pt), vec![]);
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for emp_grenade_zm on {map}");
}

/// Placed equipment off TranZit and Buried: the weapon, its script, the
/// pickup and how-to strings its home map passes its init, and the home map's
/// power rule (TranZit's turret and trap and Buried's subwoofer need a turbine).
const PLACED: &[(&str, &str, Option<(&str, &str)>, Option<&str>)] = &[
    ("equip_turbine_zm", "_zm_equip_turbine", None, None),
    ("equip_turret_zm", "_zm_equip_turret", None, Some("equipment_turret_needs_power")),
    ("equip_electrictrap_zm", "_zm_equip_electrictrap", None, Some("equipment_etrap_needs_power")),
    ("equip_springpad_zm", "_zm_equip_springpad", Some(("ZM_BURIED_EQ_SP_PHS", "ZM_BURIED_EQ_SP_HTS")), None),
    (
        "equip_subwoofer_zm",
        "_zm_equip_subwoofer",
        Some(("ZM_BURIED_EQ_SW_PHS", "ZM_BURIED_EQ_SW_HTS")),
        Some("equipment_subwoofer_needs_power"),
    ),
    ("equip_headchopper_zm", "_zm_equip_headchopper", Some(("ZM_BURIED_EQ_HC_PHS", "ZM_BURIED_EQ_HC_HTS")), None),
];

/// A placed piece of equipment off TranZit or Buried: the home map's
/// equipment rows and power rule, the script's own init, then its player
/// watchers for the player already in: past the spawn they wait on, and the
/// weapon watchers (`weapon_watchers_created`, sent once at the spawn) that
/// make it pick-up-able once placed.
fn placed(vm: &mut Vm<World>, game: &mut World, pt: ObjRef, label: &str) {
    let Some(&(_, script, strings, power)) = PLACED.iter().find(|e| e.0 == label) else {
        return;
    };
    let k = vm.intern("zombie_include_equipment");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern(label))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, &format!("bo2zm_foreign_{label}")) {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string(label);
    vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_equipment_for_level", level.clone(), vec![w.clone()]);
    vm.spawn_named(game, "maps/mp/zombies/_zm_equipment", "include_zombie_equipment", level.clone(), vec![w]);
    if let Some(power) = power {
        let k = vm.intern(power);
        vm.set_raw_field(vm.level, k, ScriptValue::Int(1));
    }
    let args = match strings {
        Some((pick, howto)) => vec![ScriptValue::IStr(vm.intern(pick)), ScriptValue::IStr(vm.intern(howto))],
        None => vec![],
    };
    let path = format!("maps/mp/zombies/{script}");
    vm.spawn_named(game, &path, "init", level.clone(), args);
    if label == "equip_turbine_zm" {
        vm.spawn_named(game, &path, "init_animtree", level, vec![]);
    }
    if let Some(t) = vm.spawn_named(game, &path, "onplayerspawned", ScriptValue::Object(pt), vec![]) {
        vm.notify_thread(game, t, pt, "spawned_player", &[]);
    }
    vm.notify_str(game, pt, "weapon_watchers_created", &[]);
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for {label} on {map}");
}

/// The Time Bomb off Buried: `_zm_weap_time_bomb::init_time_bomb` (its
/// flags, client fields, give callback, tactical and equipment rows and the
/// round rewind), then out of the box it adds itself to: here it is the
/// Special Weapons tab's only.
fn time_bomb(vm: &mut Vm<World>, world: &mut World) {
    let k = vm.intern("_time_bomb");
    if !vm.raw_field(vm.level, k).is_undefined() || !first_time(vm, "bo2zm_foreign_time_bomb") {
        return;
    }
    let script = "maps/mp/zombies/_zm_weap_time_bomb";
    vm.spawn_named(world, script, "init_time_bomb", ScriptValue::Object(vm.level), vec![]);
    let (k, w) = (vm.intern("zombie_weapons"), vm.intern("time_bomb_zm"));
    if let ScriptValue::Array(a) = vm.raw_field(vm.level, k) {
        if let Some(ScriptValue::Object(row)) = a.get(&Key::Str(w)) {
            let k = vm.intern("is_in_box");
            vm.set_raw_field(row, k, ScriptValue::Int(0));
        }
    }
    let map = world.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for time_bomb_zm on {map}");
}

/// The G-Strike off Origins: zm_tomb's tactical and weapon rows, then
/// `_zm_weap_beacon::init`. Its shells are fired by a walking giant
/// robot, which no other map has: three that always walk, as Origins
/// between its robot rounds, with the robot flags its throw reads.
fn beacon(vm: &mut Vm<World>, world: &mut World) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("beacon_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_beacon") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string("beacon_zm");
    let utility = "maps/mp/zombies/_zm_utility";
    vm.spawn_named(world, utility, "register_tactical_grenade_for_level", level.clone(), vec![w.clone()]);
    let weapons = "maps/mp/zombies/_zm_weapons";
    let args = vec![
        w.clone(),
        ScriptValue::Undefined,
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_BEACON")),
        ScriptValue::Int(2000),
        vm.string("wpck_explo"),
        vm.string(""),
        ScriptValue::Undefined,
        ScriptValue::Int(1),
    ];
    // Included first: add_zombie_weapon skips a weapon not yet included.
    vm.spawn_named(world, weapons, "include_zombie_weapon", level.clone(), vec![w, ScriptValue::Int(0)]);
    vm.spawn_named(world, weapons, "add_zombie_weapon", level.clone(), args);
    for flag in ["three_robot_round", "fire_link_enabled"] {
        let f = vm.string(flag);
        vm.spawn_named(world, "common_scripts/utility", "flag_init", level.clone(), vec![f]);
    }
    let k = vm.intern("a_giant_robots");
    if vm.raw_field(vm.level, k).is_undefined() {
        let (on_foot, cid) = (vm.intern("is_walking"), vm.intern("giant_robot_id"));
        let mut robots = Array::default();
        for i in 0..3 {
            let r = vm.alloc_object(gsc_t6::ObjKind::Struct);
            vm.set_raw_field(r, on_foot, ScriptValue::Int(1));
            vm.set_raw_field(r, cid, ScriptValue::Int(i));
            robots.push(ScriptValue::Object(r));
        }
        vm.set_raw_field(vm.level, k, ScriptValue::array(robots));
    }
    let script = "maps/mp/zombies/_zm_weap_beacon";
    vm.spawn_named(world, script, "init", level, vec![]);
    let map = world.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for beacon_zm on {map}");
}

/// A Black Ops wonder weapon off its Zombies Declassified map: that map's
/// weapon rows (zm_factory, zm_pentagon, zm_moon, zm_temple,
/// zm_cosmodrome), then the weapon's own init and the watchers its connect
/// callback threads, for the player already in.
struct Bo1 {
    weapon: &'static str,
    upgraded: Option<&'static str>,
    hint: &'static str,
    cost: i32,
    vo: &'static str,
    script: &'static str,
    /// Run before its init (the QED's results).
    pre: &'static [&'static str],
    /// A tactical's give, which its map's offhand give override calls.
    give: Option<&'static str>,
    /// Each run on the player, past the spawn it waits for.
    watchers: &'static [&'static str],
}

const BO1: &[Bo1] = &[
    Bo1 {
        weapon: "tesla_gun_zm",
        upgraded: Some("tesla_gun_upgraded_zm"),
        hint: "ZOMBIE_WEAPON_TESLA",
        cost: 10,
        vo: "tesla",
        script: "_zm_weap_tesla",
        pre: &[],
        give: None,
        watchers: &["tesla_sound_thread", "tesla_pvp_thread", "tesla_network_choke"],
    },
    Bo1 {
        weapon: "freezegun_zm",
        upgraded: Some("freezegun_upgraded_zm"),
        hint: "ZOMBIE_WEAPON_FREEZEGUN",
        cost: 10,
        vo: "freezegun",
        script: "_zm_weap_freezegun",
        pre: &[],
        give: None,
        watchers: &["wait_for_thundergun_fired"],
    },
    Bo1 {
        weapon: "microwavegundw_zm",
        upgraded: Some("microwavegundw_upgraded_zm"),
        hint: "ZOMBIE_WEAPON_MICROWAVEGUN_DW",
        cost: 10,
        vo: "microwave",
        script: "_zm_weap_microwavegun",
        pre: &[],
        give: None,
        watchers: &["wait_for_microwavegun_fired"],
    },
    Bo1 {
        weapon: "shrink_ray_zm",
        upgraded: Some("shrink_ray_upgraded_zm"),
        hint: "ZOMBIE_TEMPLE_SHRINK_RAY",
        cost: 2000,
        vo: "shrink",
        script: "_zm_weap_shrink_ray",
        pre: &[],
        give: None,
        watchers: &["wait_for_shrink_ray_fired"],
    },
    Bo1 {
        weapon: "zombie_black_hole_bomb",
        upgraded: None,
        hint: "ZOMBIE_WEAPON_SATCHEL_2000",
        cost: 2000,
        vo: "gersh",
        script: "_zm_weap_black_hole_bomb",
        pre: &[],
        give: Some("player_give_black_hole_bomb"),
        watchers: &[],
    },
    Bo1 {
        weapon: "zombie_quantum_bomb",
        upgraded: None,
        hint: "ZOMBIE_WEAPON_SATCHEL_2000",
        cost: 2000,
        vo: "quantum",
        script: "_zm_weap_quantum_bomb",
        pre: &["init_registration"],
        give: Some("player_give_quantum_bomb"),
        watchers: &[],
    },
    Bo1 {
        weapon: "zombie_nesting_dolls",
        upgraded: None,
        hint: "ZOMBIE_WEAPON_NESTING_DOLLS",
        cost: 2000,
        vo: "dolls",
        script: "_zm_weap_nesting_dolls",
        pre: &[],
        give: Some("player_give_nesting_dolls"),
        watchers: &[],
    },
];

/// `row` set up as its map does: a tactical registered, both weapons
/// included, its add_zombie_weapon row, its init, a tactical's give as
/// the weapon callback weapon_give calls (BO2's path for its own monkey;
/// weapon_give takes the old tactical first, as the map's override did),
/// then its watchers.
fn bo1(vm: &mut Vm<World>, game: &mut World, pt: ObjRef, row_no: &Bo1) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern(row_no.weapon))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, &format!("bo2zm_foreign_{}", row_no.weapon)) {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let script = format!("maps/mp/zombies/{}", row_no.script);
    let weapons = "maps/mp/zombies/_zm_weapons";
    let w = vm.string(row_no.weapon);
    if row_no.give.is_some() {
        let utility = "maps/mp/zombies/_zm_utility";
        vm.spawn_named(game, utility, "register_tactical_grenade_for_level", level.clone(), vec![w.clone()]);
    }
    for name in std::iter::once(row_no.weapon).chain(row_no.upgraded) {
        let n = vm.string(name);
        vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![n, ScriptValue::Int(0)]);
    }
    let up = row_no.upgraded.map_or(ScriptValue::Undefined, |u| vm.string(u));
    let args = vec![
        w.clone(),
        up,
        ScriptValue::IStr(vm.intern(row_no.hint)),
        ScriptValue::Int(row_no.cost),
        vm.string(row_no.vo),
        vm.string(""),
        ScriptValue::Undefined,
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    for f in row_no.pre.iter().chain(std::iter::once(&"init")) {
        vm.spawn_named(game, &script, f, level.clone(), vec![]);
    }
    if let Some(give) = row_no.give
        && let Some(f) = vm.program.find(&vm.strings, &script, give)
    {
        let func = ScriptValue::Func(gsc_t6::FuncRef::Script(f));
        vm.spawn_named(game, weapons, "register_zombie_weapon_callback", level, vec![w, func]);
    }
    for watch in row_no.watchers {
        if let Some(t) = vm.spawn_named(game, &script, watch, ScriptValue::Object(pt), vec![]) {
            vm.notify_thread(game, t, pt, "spawned_player", &[]);
        }
    }
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for {} on {map}", row_no.weapon);
}

/// The riot shield off TranZit: zm_transit's equipment and weapon rows,
/// `_zm_weap_riotshield::init` (the models, its zombie_vars, fling and
/// knockdown damage callback, equipment row) and its onplayerconnect
/// watchers for the player already in. Not on a map with its own shield:
/// both set level.riotshield_name.
fn riotshield(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("riotshield_name");
    if !vm.raw_field(vm.level, k).is_undefined() || !first_time(vm, "bo2zm_foreign_riotshield") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string("riotshield_zm");
    vm.spawn_named(game, "maps/mp/zombies/_zm_equipment", "include_zombie_equipment", level.clone(), vec![w.clone()]);
    vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_equipment_for_level", level.clone(), vec![w.clone()]);
    let weapons = "maps/mp/zombies/_zm_weapons";
    vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w.clone(), ScriptValue::Int(0)]);
    let args = vec![
        w,
        ScriptValue::Undefined,
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_RIOTSHIELD")),
        ScriptValue::Int(2000),
        vm.string("riot"),
        vm.string(""),
        ScriptValue::Undefined,
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    let script = "maps/mp/zombies/_zm_weap_riotshield";
    vm.spawn_named(game, script, "init", level, vec![]);
    vm.spawn_named(game, script, "onplayerconnect", ScriptValue::Object(pt), vec![]);
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for riotshield_zm on {map}");
}

/// The Jet Gun off TranZit: zm_transit's offhand, equipment and weapon rows
/// and its overheat explosion, then `_zm_weap_jetgun::init` and
/// jetgun_on_player_connect's three watchers (the fire one past the spawn it
/// waits for).
fn jetgun(vm: &mut Vm<World>, game: &mut World, pt: ObjRef) {
    let k = vm.intern("zombie_weapons");
    let ran = match vm.raw_field(vm.level, k) {
        ScriptValue::Array(a) => a.get(&Key::Str(vm.intern("jetgun_zm"))).is_some(),
        _ => false,
    };
    if ran || !first_time(vm, "bo2zm_foreign_jetgun") {
        return;
    }
    let level = ScriptValue::Object(vm.level);
    let w = vm.string("jetgun_zm");
    vm.spawn_named(game, "maps/mp/zombies/_zm_utility", "register_equipment_for_level", level.clone(), vec![w.clone()]);
    vm.spawn_named(game, "maps/mp/zombies/_zm_equipment", "include_zombie_equipment", level.clone(), vec![w.clone()]);
    let weapons = "maps/mp/zombies/_zm_weapons";
    vm.spawn_named(game, weapons, "include_zombie_weapon", level.clone(), vec![w.clone(), ScriptValue::Int(0)]);
    let args = vec![
        w,
        ScriptValue::Undefined,
        ScriptValue::IStr(vm.intern("ZOMBIE_WEAPON_JETGUN")),
        ScriptValue::Int(2000),
        vm.string("jet"),
        vm.string(""),
        ScriptValue::Undefined,
    ];
    vm.spawn_named(game, weapons, "add_zombie_weapon", level.clone(), args);
    // zm_transit: an overheated Jet Gun explodes, burning as the lava does.
    let k = vm.intern("explode_overheated_jetgun");
    vm.set_raw_field(vm.level, k, ScriptValue::Int(1));
    let (effects, lava) = (vm.intern("_effect"), vm.intern("lava_burning"));
    let fx = match vm.raw_field(vm.level, effects) {
        ScriptValue::Array(a) => a.get(&Key::Str(lava)).unwrap_or_default(),
        _ => ScriptValue::Undefined,
    };
    let k = vm.intern("exploding_jetgun_fx");
    vm.set_raw_field(vm.level, k, fx);
    let script = "maps/mp/zombies/_zm_weap_jetgun";
    vm.spawn_named(game, script, "init", level, vec![]);
    if let Some(t) = vm.spawn_named(game, script, "wait_for_jetgun_fired", ScriptValue::Object(pt), vec![]) {
        vm.notify_thread(game, t, pt, "spawned_player", &[]);
    }
    for watch in ["watch_weapon_changes", "handle_overheated_jetgun"] {
        vm.spawn_named(game, script, watch, ScriptValue::Object(pt), vec![]);
    }
    let map = game.resource::<Zm>().map.clone();
    diag::info!(Sim, "bo2zm t6: {script} set up for jetgun_zm on {map}");
}
