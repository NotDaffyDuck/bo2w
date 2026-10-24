//! bo2zm: the solo cheat menu's side in the game (Num 1, for recording a
//! map's features): the lines he picks are queued by the menu and land at
//! the next tick, each through the scripts' own functions; the toggles are
//! flags the game reads while they are on. Only a local game of one player
//! (no Public Match lobby, one player expected) takes them.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use bevy_ecs::prelude::World;
use gsc_t6::Value;

use super::{Zm, frame, with_vm};

/// One line of the menu, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cheat {
    Points,
    GodMode,
    NoClip,
    IgnoreMe,
    InfiniteAmmo,
    EndRound,
    DropPowerUp,
    PowerOn,
    OpenDoors,
    PackAPunch,
    AllPerks,
    SpecialWeapon,
}

impl Cheat {
    pub const ALL: [Cheat; 12] = [
        Cheat::Points,
        Cheat::GodMode,
        Cheat::NoClip,
        Cheat::IgnoreMe,
        Cheat::InfiniteAmmo,
        Cheat::EndRound,
        Cheat::DropPowerUp,
        Cheat::PowerOn,
        Cheat::OpenDoors,
        Cheat::PackAPunch,
        Cheat::AllPerks,
        Cheat::SpecialWeapon,
    ];

    /// The line as the menu shows it.
    pub fn label(self) -> &'static str {
        match self {
            Cheat::Points => "+1000 Points",
            Cheat::GodMode => "God Mode",
            Cheat::NoClip => "No Clip",
            Cheat::IgnoreMe => "Zombies Ignore You",
            Cheat::InfiniteAmmo => "Infinite Ammo",
            Cheat::EndRound => "End Round",
            Cheat::DropPowerUp => "Drop Power-Up",
            Cheat::PowerOn => "Power On",
            Cheat::OpenDoors => "Open All Doors",
            Cheat::PackAPunch => "Pack-a-Punch Gun",
            Cheat::AllPerks => "All Perks",
            Cheat::SpecialWeapon => "Special Weapons",
        }
    }

    /// ON / OFF at the end of its line (else an action, picked again and
    /// again).
    pub fn is_toggle(self) -> bool {
        matches!(
            self,
            Cheat::GodMode | Cheat::NoClip | Cheat::IgnoreMe | Cheat::InfiniteAmmo
        )
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

static PICKS: Mutex<Vec<Cheat>> = Mutex::new(Vec::new());
static ON: AtomicU32 = AtomicU32::new(0);
/// Zombies Ignore You set the player's `ignoreme` (cleared once when it
/// goes off, so the scripts' own use of it is left alone).
static IGNORING: AtomicBool = AtomicBool::new(false);

/// A local game of one player: no Public Match lobby and one player.
pub fn cheats_allowed() -> bool {
    super::PARTY.load(Ordering::Relaxed) == 0 && super::expected_players() == 1
}

/// He picked `cheat`: a toggle flips (its new state is returned), an action
/// is queued for the next tick (true).
pub fn cheat_pick(cheat: Cheat) -> bool {
    if cheat.is_toggle() {
        return ON.fetch_xor(cheat.bit(), Ordering::Relaxed) & cheat.bit() == 0;
    }
    if let Ok(mut picks) = PICKS.lock() {
        picks.push(cheat);
    }
    true
}

/// Whether a toggle is on.
pub fn cheat_on(cheat: Cheat) -> bool {
    ON.load(Ordering::Relaxed) & cheat.bit() != 0
}

/// A new game starts with every cheat off and nothing queued.
pub(crate) fn reset() {
    ON.store(0, Ordering::Relaxed);
    OPENING.store(false, Ordering::Relaxed);
    if let Ok(mut have) = SPECIAL_HAVE.lock() {
        have.clear();
    }
    SPECIAL_LISTED.store(false, Ordering::Relaxed);
    if let Ok(mut draw) = SPECIAL_DRAW.lock() {
        *draw = None;
    }
    IGNORING.store(false, Ordering::Relaxed);
    if let Ok(mut picks) = PICKS.lock() {
        picks.clear();
    }
}

/// The queued picks, applied (each tick, before the scripts run).
pub(crate) fn apply(world: &mut World) {
    list_special(world);
    draw_picked_special(world);
    give_picked_special(world);
    let picks = PICKS
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default();
    if picks.is_empty() {
        return;
    }
    if !cheats_allowed() || world.resource::<Zm>().players.len() != 1 {
        diag::info!(
            Sim,
            "bo2zm cheats: {} picks dropped (not a solo game)",
            picks.len()
        );
        return;
    }
    for cheat in picks {
        let done = match cheat {
            Cheat::Points => player_call(
                world,
                "maps/mp/zombies/_zm_score",
                "add_to_player_score",
                vec![Value::Int(1000)],
            ),
            Cheat::EndRound => end_round(world),
            Cheat::DropPowerUp => drop_powerup(world),
            Cheat::PowerOn => power_on(world),
            Cheat::OpenDoors => {
                NEXT_DOOR.store(0, Ordering::Relaxed);
                OPENING.store(true, Ordering::Relaxed);
                true
            }
            Cheat::PackAPunch => pack_a_punch(world),
            Cheat::AllPerks => all_perks(world),
            // A tab of its own: the menu lists them and he picks one.
            Cheat::SpecialWeapon => true,
            // The toggles are flags, never queued.
            Cheat::GodMode | Cheat::NoClip | Cheat::IgnoreMe | Cheat::InfiniteAmmo => true,
        };
        diag::info!(
            Sim,
            "bo2zm cheats: {} {}",
            cheat.label(),
            if done { "applied" } else { "not found" }
        );
    }
}

/// No Clip held each tick: a walking player flies (the movement code's
/// pm_type 2, which his client predicts too); off, he walks and falls
/// from where he is. Down, linked or dead he is left as he is.
pub(crate) fn hold(world: &mut World) {
    use movement_iw4::noclip::PM_TYPE_NOCLIP;
    let fly = cheat_on(Cheat::NoClip);
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        if let Some(ps) = frame(world).player_mut(crate::world::ClientId(c)) {
            if fly && ps.pm_type == 0 {
                ps.pm_type = PM_TYPE_NOCLIP;
                diag::info!(Sim, "bo2zm cheats: no clip on (client {c})");
            } else if !fly && ps.pm_type == PM_TYPE_NOCLIP {
                ps.pm_type = 0;
                diag::info!(Sim, "bo2zm cheats: no clip off (client {c})");
            }
        }
        // Infinite Ammo: the gun in his hands, clip and stock full (each
        // clamps to the weapon's own size).
        if cheat_on(Cheat::InfiniteAmmo) {
            let id = crate::world::ClientId(c);
            let mut f = frame(world);
            if let Some(w) = f.player(id).map(|p| p.weapon as u32).filter(|w| *w != 0) {
                crate::script_player::set_ammo_clip(&mut f, id, w, i32::MAX);
                crate::script_player::set_ammo_stock(&mut f, id, w, i32::MAX);
            }
        }
    }
    open_next_door(world);
    // Zombies Ignore You: the scripts' own `ignoreme`, which their target
    // pick (is_player_valid) skips.
    let ignore = cheat_on(Cheat::IgnoreMe);
    if ignore || IGNORING.load(Ordering::Relaxed) {
        if let Some(p) = player(world) {
            with_vm(world, |vm, world| {
                let k = vm.intern("ignoreme");
                vm.set_field(world, p, k, Value::Int(i32::from(ignore)));
            });
        }
        if IGNORING.swap(ignore, Ordering::Relaxed) != ignore {
            let state = if ignore { "on" } else { "off" };
            diag::info!(Sim, "bo2zm cheats: zombies ignore you {state}");
        }
    }
}

/// End Round: no more zombies to spawn this round and every one alive
/// killed (as BO2's kill_all_zombies does: its own damage, MOD_SUICIDE), so
/// the scripts end the round and start the next as normal.
fn end_round(world: &mut World) -> bool {
    let alive: Vec<(u32, gsc_t6::ObjRef)> = world
        .resource::<Zm>()
        .actors
        .iter()
        .filter(|(_, a)| a.alive)
        .filter_map(|(n, a)| a.obj.map(|o| (*n, o)))
        .collect();
    with_vm(world, |vm, world| {
        let total = vm.intern("zombie_total");
        vm.set_field(world, vm.level, total, Value::Int(0));
        let origin = vm.intern("origin");
        for (n, obj) in &alive {
            let at = vm
                .get_field(world, *obj, origin)
                .as_vec3()
                .unwrap_or([0.0; 3]);
            let me = Value::Object(*obj);
            super::actors::damage(
                vm,
                world,
                *n,
                me.clone(),
                me,
                100_000_000,
                0,
                "MOD_SUICIDE",
                "none",
                at,
                [0.0; 3],
                "none",
            );
        }
        diag::info!(
            Sim,
            "bo2zm cheats: end round: {} zombies killed",
            alive.len()
        );
    })
    .is_some()
}

/// Which of the map's power-ups the next Drop Power-Up drops.
static NEXT_POWERUP: AtomicU32 = AtomicU32::new(0);

/// Drop Power-Up: the next of the map's own power-ups (its
/// `level.zombie_powerup_array`, by name) at his feet, by the scripts'
/// specific_powerup_drop.
fn drop_powerup(world: &mut World) -> bool {
    let Some(at) = player_origin(world) else {
        return false;
    };
    with_vm(world, |vm, world| {
        let k = vm.intern("zombie_powerup_array");
        let mut names: Vec<String> = match vm.raw_field(vm.level, k) {
            Value::Array(a) => a.read().values_in_order().map(|v| vm.to_text(v)).collect(),
            _ => Vec::new(),
        };
        names.sort();
        names.dedup();
        if names.is_empty() {
            return false;
        }
        let i = NEXT_POWERUP.fetch_add(1, Ordering::Relaxed) as usize % names.len();
        diag::info!(
            Sim,
            "bo2zm cheats: drop power-up {} ({} of {})",
            names[i],
            i + 1,
            names.len()
        );
        let name = vm.string(&names[i]);
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm_powerups",
            "specific_powerup_drop",
            Value::Object(vm.level),
            vec![name, Value::Vec3(at)],
        )
        .is_some()
    })
    .unwrap_or(false)
}

/// The map's script triggers named `targetname` (entity number, object,
/// its `script_noteworthy`).
fn triggers_named(
    vm: &mut gsc_t6::Vm<World>,
    world: &World,
    targetname: &[&str],
) -> Vec<(u32, gsc_t6::ObjRef, String)> {
    let (tn, sn) = (vm.intern("targetname"), vm.intern("script_noteworthy"));
    let mut out: Vec<_> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter(|(_, e)| e.classname.starts_with("trigger"))
        .filter_map(|(n, e)| {
            let o = e.obj?;
            let name = vm.to_text(&vm.raw_field(o, tn));
            targetname
                .contains(&name.as_str())
                .then(|| (*n, o, vm.to_text(&vm.raw_field(o, sn))))
        })
        .collect();
    out.sort_by_key(|t| t.0);
    out
}

/// Power On: the power switch's own trigger fired by him (its lever and
/// lights) and the scripts' `power_on` flag set, as the switch sets it.
fn power_on(world: &mut World) -> bool {
    let Some(p) = player(world) else {
        return false;
    };
    with_vm(world, |vm, world| {
        let switches = triggers_named(vm, world, &["use_elec_switch"]);
        for (_, t, _) in &switches {
            vm.notify_str(world, *t, "trigger", &[Value::Object(p)]);
        }
        let f = vm.string("power_on");
        diag::info!(
            Sim,
            "bo2zm cheats: power on ({} switches thrown)",
            switches.len()
        );
        vm.spawn_named(
            world,
            "common_scripts/utility",
            "flag_set",
            Value::Object(vm.level),
            vec![f],
        )
        .is_some()
    })
    .unwrap_or(false)
}

/// Open All Doors runs one door or debris pile a tick (a bought debris
/// pile deletes its sibling triggers): the next one by entity number.
static OPENING: AtomicBool = AtomicBool::new(false);
static NEXT_DOOR: AtomicU32 = AtomicU32::new(0);

/// The next door or debris pile not yet opened, bought by him for free
/// (its trigger with `force`, as the scripts' own open-all does).
fn open_next_door(world: &mut World) {
    if !OPENING.load(Ordering::Relaxed) {
        return;
    }
    let Some(p) = player(world) else {
        return;
    };
    let next = NEXT_DOOR.load(Ordering::Relaxed);
    with_vm(world, |vm, world| {
        let opened = vm.intern("has_been_opened");
        let door = triggers_named(vm, world, &["zombie_door", "zombie_debris"])
            .into_iter()
            .find(|(n, o, _)| *n >= next && !gsc_t6::truthy(&vm.raw_field(*o, opened)));
        match door {
            Some((n, t, _)) => {
                NEXT_DOOR.store(n + 1, Ordering::Relaxed);
                diag::info!(Sim, "bo2zm cheats: open door ent{n}");
                vm.notify_str(world, t, "trigger", &[Value::Object(p), Value::Int(1)]);
            }
            None => {
                OPENING.store(false, Ordering::Relaxed);
                diag::info!(Sim, "bo2zm cheats: open all doors: done");
            }
        }
    });
}

/// Pack-a-Punch Gun: the gun in his hands swapped for its upgrade (the
/// map's `level.zombie_weapons[gun].upgrade_name`), given by the scripts'
/// weapon_give as an upgrade.
fn pack_a_punch(world: &mut World) -> bool {
    let (Some(p), Some(c)) = (
        player(world),
        world.resource::<Zm>().players.keys().next().copied(),
    ) else {
        return false;
    };
    let id = crate::world::ClientId(c);
    let Some(w) = frame(world)
        .player(id)
        .map(|ps| ps.weapon as u32)
        .filter(|w| *w != 0)
    else {
        return false;
    };
    let gun = super::weapon_text(world, w);
    with_vm(world, |vm, world| {
        let (zw, up) = (vm.intern("zombie_weapons"), vm.intern("upgrade_name"));
        let upgrade = match vm.raw_field(vm.level, zw) {
            Value::Array(a) => a.read().get(&gsc_t6::Key::Str(vm.intern(&gun))).cloned(),
            _ => None,
        }
        .and_then(|v| v.as_obj())
        .map(|o| vm.to_text(&vm.raw_field(o, up)))
        .filter(|u| !u.is_empty() && u != "undefined" && *u != gun);
        let Some(upgrade) = upgrade else {
            diag::info!(Sim, "bo2zm cheats: pack-a-punch: {gun} has no upgrade");
            return false;
        };
        diag::info!(Sim, "bo2zm cheats: pack-a-punch: {gun} -> {upgrade}");
        crate::script_player::take_weapon(&mut frame(world), id, w);
        let u = vm.string(&upgrade);
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm_weapons",
            "weapon_give",
            Value::Object(p),
            vec![u, Value::Int(1)],
        )
        .is_some()
    })
    .unwrap_or(false)
}

/// All Perks: every perk machine's perk the map has (its vending triggers'
/// `script_noteworthy`) that he lacks, by the scripts' give_perk.
fn all_perks(world: &mut World) -> bool {
    let Some(p) = player(world) else {
        return false;
    };
    let has: Vec<String> = world
        .resource::<Zm>()
        .players
        .values()
        .next()
        .map(|pl| pl.perks.iter().cloned().collect())
        .unwrap_or_default();
    with_vm(world, |vm, world| {
        let mut perks: Vec<String> = triggers_named(vm, world, &["zombie_vending"])
            .into_iter()
            .map(|(_, _, perk)| perk)
            .filter(|perk| {
                perk.starts_with("specialty_")
                    && perk != "specialty_weapupgrade"
                    && !has.contains(perk)
            })
            .collect();
        perks.sort();
        perks.dedup();
        diag::info!(Sim, "bo2zm cheats: all perks: {}", perks.join(" "));
        for perk in &perks {
            let s = vm.string(perk);
            vm.spawn_named(
                world,
                "maps/mp/zombies/_zm_perks",
                "give_perk",
                Value::Object(p),
                vec![s, Value::Int(0)],
            );
        }
        true
    })
    .unwrap_or(false)
}

/// Every Black Ops and Black Ops II wonder weapon, craftable and special
/// item: its script name and the name the Special Weapons tab shows. The tab
/// lists the ones this map loaded, in this order.
const SPECIAL_WEAPONS: &[(&str, &str)] = &[
    ("ray_gun_zm", "Ray Gun"),
    ("raygun_mark2_zm", "Ray Gun Mark II"),
    ("thundergun_zm", "Thunder Gun"),
    ("tesla_gun_zm", "Wunderwaffe DG-2"),
    ("freezegun_zm", "Winter's Howl"),
    ("microwavegundw_zm", "Wave Gun"),
    ("shrink_ray_zm", "Baby Gun (31-79 JGb215)"),
    ("humangun_zm", "V-R11"),
    ("sniper_explosive_zm", "Scavenger"),
    ("crossbow_explosive_zm", "Crossbow"),
    ("jetgun_zm", "Jet Gun"),
    ("slipgun_zm", "Sliquifier"),
    ("slowgun_zm", "Paralyzer"),
    ("blundergat_zm", "Blundergat"),
    ("blundersplat_zm", "Acid Gat"),
    ("staff_fire_zm", "Fire Staff"),
    ("staff_water_zm", "Ice Staff"),
    ("staff_air_zm", "Wind Staff"),
    ("staff_lightning_zm", "Lightning Staff"),
    ("staff_revive_zm", "Revive Staff"),
    ("bouncing_tomahawk_zm", "Hell's Retriever"),
    ("upgraded_tomahawk_zm", "Hell's Redeemer"),
    ("knife_ballistic_zm", "Ballistic Knife"),
    ("one_inch_punch_zm", "One Inch Punch"),
    ("tazer_knuckles_zm", "Galvaknuckles"),
    ("bowie_knife_zm", "Bowie Knife"),
    ("sickle_knife_zm", "Sickle"),
    ("spoon_zm_alcatraz", "Golden Spoon"),
    ("spork_zm_alcatraz", "Golden Spork"),
    ("cymbal_monkey_zm", "Monkey Bomb"),
    ("zombie_cymbal_monkey", "Monkey Bomb"),
    ("zombie_black_hole_bomb", "Gersh Device"),
    ("zombie_nesting_dolls", "Matryoshka Dolls"),
    ("zombie_quantum_bomb", "QED"),
    ("emp_grenade_zm", "EMP Grenade"),
    ("time_bomb_zm", "Time Bomb"),
    ("beacon_zm", "G-Strike"),
    ("sticky_grenade_zm", "Semtex"),
    ("claymore_zm", "Claymore"),
    ("riotshield_zm", "Zombie Shield"),
    ("alcatraz_shield_zm", "Zombie Shield"),
    ("tomb_shield_zm", "Zombie Shield"),
    ("equip_turbine_zm", "Turbine"),
    ("equip_turret_zm", "Turret"),
    ("equip_electrictrap_zm", "Electric Trap"),
    ("equip_springpad_zm", "Trample Steam"),
    ("equip_headchopper_zm", "Head Chopper"),
    ("equip_subwoofer_zm", "Subwoofer"),
    ("equip_hacker_zm", "Hacker"),
    ("equip_gasmask_zm", "Gas Mask (P.E.S.)"),
];

/// Entries another game mode adds to the tab (Minecraft Zombies' own
/// weapons), after the list above.
static SPECIAL_EXTRA: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
/// The ones this map loaded (script name, shown name), worked out once a
/// game at its first tick; and the one he picked, given at the next tick.
static SPECIAL_HAVE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static SPECIAL_LISTED: AtomicBool = AtomicBool::new(false);
static SPECIAL_PICK: Mutex<Option<String>> = Mutex::new(None);
/// The picked weapon to pull out once weapon_give has put it in his hands'
/// list, and the ticks left to wait for that.
static SPECIAL_DRAW: Mutex<Option<(String, u32)>> = Mutex::new(None);

/// Melee weapons swap his knife rather than being held; grenades and
/// tacticals (the offhand kind) are thrown from their own button.
const SPECIAL_MELEE: &[&str] = &[
    "one_inch_punch_zm",
    "tazer_knuckles_zm",
    "bowie_knife_zm",
    "sickle_knife_zm",
    "spoon_zm_alcatraz",
    "spork_zm_alcatraz",
];

/// Adds `(script name, shown name)` entries to the Special Weapons tab
/// (each listed only on a map that loaded it). Call before the map loads.
pub fn add_special_weapons(items: &[(&str, &str)]) {
    if let Ok(mut extra) = SPECIAL_EXTRA.lock() {
        for (name, label) in items {
            if !extra.iter().any(|(n, _)| n == name) {
                extra.push(((*name).to_owned(), (*label).to_owned()));
            }
        }
    }
}

/// Items that are not Black Ops II weapons (Minecraft Zombies' bow and
/// stick): shown on every map, and picking one calls its function.
static SPECIAL_ITEMS: Mutex<Vec<(String, fn())>> = Mutex::new(Vec::new());
/// The tab's name for an item from `add_special_items`.
const ITEM: &str = "item:";

/// Adds `(shown name, give)` items to the Special Weapons tab, after the
/// weapons: always listed, and picking one calls `give` at the next tick.
pub fn add_special_items(items: &[(&str, fn())]) {
    if let Ok(mut have) = SPECIAL_ITEMS.lock() {
        for (label, give) in items {
            if !have.iter().any(|(l, _)| l == label) {
                have.push(((*label).to_owned(), *give));
            }
        }
    }
}

/// The Special Weapons tab: what this map has, (script name, shown name).
pub fn special_weapons() -> Vec<(String, String)> {
    let mut all = SPECIAL_HAVE.lock().map(|h| h.clone()).unwrap_or_default();
    if let Ok(items) = SPECIAL_ITEMS.lock() {
        all.extend(items.iter().map(|(l, _)| (format!("{ITEM}{l}"), l.clone())));
    }
    all
}

/// He picked a special weapon on the tab: given at the next tick.
pub fn special_pick(name: &str) {
    if let Ok(mut pick) = SPECIAL_PICK.lock() {
        *pick = Some(name.to_owned());
    }
}

/// Once a game: which special weapons the map loaded.
fn list_special(world: &mut World) {
    if SPECIAL_LISTED.load(Ordering::Relaxed) || world.resource::<Zm>().players.is_empty() {
        return;
    }
    SPECIAL_LISTED.store(true, Ordering::Relaxed);
    let extra = SPECIAL_EXTRA.lock().map(|e| e.clone()).unwrap_or_default();
    let all: Vec<(String, String)> = SPECIAL_WEAPONS
        .iter()
        .map(|(n, l)| ((*n).to_owned(), (*l).to_owned()))
        .chain(extra)
        .collect();
    let mut have: Vec<(String, String)> = all
        .into_iter()
        .filter(|(n, _)| super::weapon(world, n).is_ok_and(|w| w != 0))
        .collect();
    // TranZit's shield comes on every map; Mob's and Origins' own is theirs.
    if have.iter().any(|h| matches!(h.0.as_str(), "alcatraz_shield_zm" | "tomb_shield_zm")) {
        have.retain(|h| h.0 != "riotshield_zm");
    }
    diag::info!(
        Sim,
        "bo2zm cheats: special weapons this map: {}",
        have.iter()
            .map(|h| h.0.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Ok(mut h) = SPECIAL_HAVE.lock() {
        *h = have;
    }
}

/// The special weapon he picked, given by the scripts' weapon_give the way
/// the box or a craft table gives it (equipment, melee weapons and grenades
/// go to their own slots).
fn give_picked_special(world: &mut World) {
    let Some(name) = SPECIAL_PICK.lock().ok().and_then(|mut p| p.take()) else {
        return;
    };
    let Some(p) = player(world).filter(|_| cheats_allowed()) else {
        diag::info!(
            Sim,
            "bo2zm cheats: special weapon {name} dropped (not a solo game)"
        );
        return;
    };
    if let Some(label) = name.strip_prefix(ITEM) {
        let give = SPECIAL_ITEMS
            .lock()
            .ok()
            .and_then(|items| items.iter().find(|(l, _)| l == label).map(|(_, g)| *g));
        if let Some(give) = give {
            give();
        }
        diag::info!(
            Sim,
            "bo2zm cheats: special item {label} {}",
            if give.is_some() { "given" } else { "not found" }
        );
        return;
    }
    let done = with_vm(world, |vm, world| give_special(vm, world, p, &name)).unwrap_or(false);
    diag::info!(
        Sim,
        "bo2zm cheats: special weapon {name} {}",
        if done { "given" } else { "not found" }
    );
    if done && !SPECIAL_MELEE.contains(&name.as_str()) {
        if let Ok(mut draw) = SPECIAL_DRAW.lock() {
            *draw = Some((name, 40));
        }
    }
}

/// Pulls out the special weapon he picked (as pressing its button would,
/// for equipment like the Jet Gun or a shield), once he has it.
fn draw_picked_special(world: &mut World) {
    let Some((name, left)) = SPECIAL_DRAW.lock().ok().and_then(|mut d| d.take()) else {
        return;
    };
    let (Some(c), Ok(w)) = (
        world.resource::<Zm>().players.keys().next().copied(),
        super::weapon(world, &name),
    ) else {
        return;
    };
    let id = crate::world::ClientId(c);
    let mut f = frame(world);
    let offhand = f
        .combat_facts_for(w)
        .is_some_and(|facts| facts.inventory_type == 1);
    if offhand {
        return;
    }
    if crate::script_player::has_weapon(&f, id, w) {
        crate::script_player::switch_to_weapon(&mut f, id, w);
        diag::info!(Sim, "bo2zm cheats: special weapon {name} pulled out");
    } else if left > 0 {
        drop(f);
        if let Ok(mut draw) = SPECIAL_DRAW.lock() {
            *draw = Some((name, left - 1));
        }
    }
}

/// Gives a special weapon the way its map does: weapon_give, except the One
/// Inch Punch, which Origins grants by threading one_inch_punch_melee_attack
/// (zm_tomb.gsc; it swaps the melee weapon and watches every swing).
pub(crate) fn give_special(
    vm: &mut gsc_t6::Vm<World>,
    world: &mut World,
    p: gsc_t6::ObjRef,
    name: &str,
) -> bool {
    if name == "one_inch_punch_zm" {
        super::foreign_weapons::before_give(vm, world, p, name);
        return vm
            .spawn_named(
                world,
                "maps/mp/zombies/_zm_weap_one_inch_punch",
                "one_inch_punch_melee_attack",
                Value::Object(p),
                vec![],
            )
            .is_some();
    }
    // Another map's weapon (the Hell's Retriever on Nuketown): what its own
    // map's scripts would have set up first.
    super::foreign_weapons::before_give(vm, world, p, name);
    let n = vm.string(name);
    let given = vm
        .spawn_named(
            world,
            "maps/mp/zombies/_zm_weapons",
            "weapon_give",
            Value::Object(p),
            vec![n, Value::Int(0)],
        )
        .is_some();
    // The Hell's Retriever / Redeemer as Mob's pickup hands it over
    // (_zm_weap_tomahawk tomahawk_pickup_trigger): the axe he has earned,
    // its HUD icon (red for the Redeemer), and the notes its scripts wait on.
    if given && name.ends_with("tomahawk_zm") {
        let k = vm.intern("current_tomahawk_weapon");
        let g = vm.string(name);
        vm.set_field(world, p, k, g.clone());
        // The pickup's set_player_tactical_grenade: Mob never registers the
        // axe as a tactical, so weapon_give skips it, and without it the
        // Redeemer's glow, trail and hits play the plain axe's effects.
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm_utility",
            "set_player_tactical_grenade",
            Value::Object(p),
            vec![g],
        );
        let mut fields = vec!["tomahawk_in_use"];
        if name == "upgraded_tomahawk_zm" {
            fields.push("upgraded_tomahawk_in_use");
        }
        for f in fields {
            let f = vm.string(f);
            vm.spawn_named(
                world,
                "maps/mp/_utility",
                "setclientfieldtoplayer",
                Value::Object(p),
                vec![f, Value::Int(1)],
            );
        }
        vm.notify_str(world, p, "tomahawk_picked_up", &[]);
        vm.notify_str(world, p, "player_obtained_tomahawk", &[]);
    }
    given
}

/// Where the one player stands.
fn player_origin(world: &mut World) -> Option<[f32; 3]> {
    let c = *world.resource::<Zm>().players.keys().next()?;
    frame(world)
        .player(crate::world::ClientId(c))
        .map(|p| p.origin)
}

/// The one player's script object.
fn player(world: &World) -> Option<gsc_t6::ObjRef> {
    world
        .resource::<Zm>()
        .players
        .values()
        .next()
        .map(|p| p.obj)
}

/// A script function run on the player (false when it or he is missing).
fn player_call(world: &mut World, file: &str, func: &str, args: Vec<Value>) -> bool {
    let Some(p) = player(world) else {
        return false;
    };
    with_vm(world, |vm, world| {
        vm.spawn_named(world, file, func, Value::Object(p), args)
            .is_some()
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_is_queued_each_pick_a_toggle_flips() {
        reset();
        assert!(cheat_pick(Cheat::Points));
        assert!(cheat_pick(Cheat::Points));
        assert_eq!(PICKS.lock().unwrap().len(), 2);
        assert!(!cheat_on(Cheat::Points));
        reset();
        assert!(PICKS.lock().unwrap().is_empty());
        // A toggle flips and is not queued (one test: the state is global).
        assert!(cheat_pick(Cheat::GodMode));
        assert!(cheat_on(Cheat::GodMode));
        assert!(!cheat_pick(Cheat::GodMode));
        assert!(!cheat_on(Cheat::GodMode));
        assert!(PICKS.lock().unwrap().is_empty());
        cheat_pick(Cheat::GodMode);
        reset();
        assert!(!cheat_on(Cheat::GodMode));
        assert!(PICKS.lock().unwrap().is_empty());
    }
}
