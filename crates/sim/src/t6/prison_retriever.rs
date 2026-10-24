//! Mob of the Dead (zm_prison) Hell's Retriever test aid.
//!
//! IW4L_T6_RETRIEVER=1: the tomahawk the way a player gets it
//! (zm_alcatraz_weap_quest, _zm_weap_tomahawk). Once he holds the lightning
//! hands (the Afterlife start) he is put by his body and revived (6-16 s),
//! and at 17 s every zone is opened. Then each wolf head in turn
//! (`level.soul_catchers`): he stands in the middle of its area
//! (`level.soul_catchers_vol`) and the autoplay player kills what comes
//! (best with IW4L_T6_AUTOPLAY_KNIFE=1, so they die inside the area); every
//! 5 s the head's `souls_received` (6 charge it) is logged, and once it is
//! charged he moves to the next. When all three are fed
//! (`soul_catchers_charged`) he is put by the spinning tomahawk
//! (`retriever_pickup_trigger`) and presses Use; his weapons and tactical are
//! logged. Then for 20 s he throws it (the tactical button) at the nearest
//! zombie every 3 s, and his `tomahawk_upgrade_kills` (each
//! `got_a_tomahawk_kill`) is logged.

use std::sync::atomic::{AtomicI64, Ordering};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Value, Vm};

use super::autoplay_prison::revive_at_body;
use super::tomb_test::{look_and_use, place_near};
use super::{Zm, frame, with_vm};
use crate::world::ClientId;

const OPEN_AT: i64 = 17_000;
const FEED_AT: i64 = 19_000;
/// The longest he waits at one head before going on to the next.
const FEED_MS: i64 = 240_000;

fn field(vm: &mut Vm<World>, o: ObjRef, s: &str) -> Value {
    let k = vm.intern(s);
    vm.raw_field(o, k)
}

fn level_flag(vm: &mut Vm<World>, name: &str) -> bool {
    let lvl = vm.level;
    match field(vm, lvl, "flag") {
        Value::Array(a) => a
            .get(&gsc_t6::Key::Str(vm.intern(name)))
            .is_some_and(|v| gsc_t6::truthy(&v)),
        _ => false,
    }
}

/// Wolf head `i`: (its name, souls received, charged, eating, the entity of
/// its area).
fn head(world: &mut World, i: i32) -> Option<(String, i32, bool, bool, Option<u32>)> {
    with_vm(world, |vm, _| {
        let lvl = vm.level;
        let Value::Array(a) = field(vm, lvl, "soul_catchers") else {
            return None;
        };
        let Some(Value::Object(s)) = a.get(&gsc_t6::Key::Int(i)) else {
            return None;
        };
        let noteworthy = field(vm, s, "script_noteworthy");
        let name = vm.to_text(&noteworthy);
        let souls = match field(vm, s, "souls_received") {
            Value::Int(n) => n,
            _ => -1,
        };
        let charged = gsc_t6::truthy(&field(vm, s, "is_charged"));
        let eating = gsc_t6::truthy(&field(vm, s, "is_eating"));
        let vol = match field(vm, lvl, "soul_catchers_vol") {
            Value::Array(v) => match v.get(&gsc_t6::Key::Int(i)) {
                Some(Value::Object(o)) => match vm.kind(o) {
                    Some(ObjKind::Entity(n)) => Some(n),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        Some((name, souls, charged, eating, vol))
    })
    .flatten()
}

/// The middle of an entity's brush box, else its origin.
fn middle(world: &mut World, n: u32) -> Option<[f32; 3]> {
    let e = world.resource::<Zm>().ents.get(&n).cloned()?;
    Some(
        super::brushes::world_box(world, &e)
            .map_or(e.origin, |(lo, hi)| std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5)),
    )
}

/// A spawned entity whose `name` field reads `value`: (entnum, origin).
fn ent_by(world: &mut World, name: &str, value: &str) -> Option<(u32, [f32; 3])> {
    with_vm(world, |vm, world| {
        let k = vm.intern(name);
        world.resource::<Zm>().ents.iter().find_map(|(n, e)| {
            let o = e.obj?;
            (vm.to_text(&vm.raw_field(o, k)) == value).then_some((*n, e.origin))
        })
    })
    .flatten()
}

fn player_field(world: &mut World, name: &str) -> String {
    with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&0).map(|p| p.obj)?;
        let v = field(vm, p, name);
        Some(vm.to_text(&v))
    })
    .flatten()
    .unwrap_or_default()
}

fn round(world: &mut World) -> String {
    with_vm(world, |vm, _| {
        let lvl = vm.level;
        let v = field(vm, lvl, "round_number");
        vm.to_text(&v)
    })
    .unwrap_or_default()
}

fn weapons(world: &mut World, client: ClientId) -> Vec<String> {
    let f = frame(world);
    crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
        .into_iter()
        .map(|w| crate::script_player::weapon_name(&f, w))
        .collect()
}

fn nearest_zombie(world: &mut World, from: [f32; 3]) -> Option<[f32; 3]> {
    let zm = world.resource::<Zm>();
    zm.actors
        .iter()
        .filter(|(_, a)| a.alive && a.scripted.is_none())
        .filter_map(|(n, _)| zm.ents.get(n).map(|e| e.origin))
        .min_by(|a, b| {
            let d = |p: &[f32; 3]| (p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2);
            d(a).total_cmp(&d(b))
        })
}

pub(crate) fn retriever_test(world: &mut World, now: i64) {
    if std::env::var_os("IW4L_T6_RETRIEVER").is_none() {
        return;
    }
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    // Which head he is at (0-2, 3 = all fed) and since when.
    static AT: AtomicI64 = AtomicI64::new(0);
    static SINCE: AtomicI64 = AtomicI64::new(i64::MIN);
    static GOT: AtomicI64 = AtomicI64::new(i64::MIN);
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    // Off Mob (the axe on any map) there is no afterlife to wait out.
    let mob = world.resource::<Zm>().map == "zm_prison";
    let mut start = START.load(Ordering::Relaxed);
    if start == i64::MIN {
        let held = frame(world).player(client).map_or(0, |p| p.weapon);
        if held == 0 || (mob && super::weapon_text(world, held) != "lightning_hands_zm") {
            return;
        }
        start = now;
        START.store(now, Ordering::Relaxed);
    }
    let rel = now - start;
    if mob && (6000 + tick..16000).contains(&rel) {
        revive_at_body(world, rel, 6000);
        return;
    }
    // IW4L_T6_RETRIEVER=throw: no wolf heads; at 20 s he is given the
    // tomahawk (weapon_give puts it on his tactical slot) and throws it.
    // =throw_ug: the same with the Hell's Redeemer.
    let mode = std::env::var("IW4L_T6_RETRIEVER").unwrap_or_default();
    let throw_only = mode.starts_with("throw");
    let axe = if mode == "throw_ug" { "upgraded_tomahawk_zm" } else { "bouncing_tomahawk_zm" };
    if throw_only && GOT.load(Ordering::Relaxed) == i64::MIN {
        if rel >= 20000 {
            with_vm(world, |vm, world| {
                if let Some(p) = world.resource::<Zm>().players.get(&client.0).map(|p| p.obj) {
                    // As the Special Weapons menu gives it (Mob's pickup).
                    super::cheats::give_special(vm, world, p, axe);
                }
            });
            let w = super::weapon(world, axe).ok();
            let f = frame(world);
            let class = w.and_then(|w| f.equipment_facts_for(w)).map(|eq| eq.offhand_class);
            let slots = f.player(client).map(|ps| (ps.offhand_primary, ps.offhand_secondary));
            diag::info!(
                Sim,
                "bo2zm t6 retriever: given {axe} (throw test), offhand class {class:?}, slots (lethal, tactical) {slots:?}"
            );
            GOT.store(rel - 3000, Ordering::Relaxed);
        }
        return;
    }
    // The throw test skips the wolf heads and the pickup.
    'quest: {
    if throw_only {
        break 'quest;
    }
    if (OPEN_AT..OPEN_AT + tick).contains(&rel) {
        super::autoplay::open_all_zones(world);
        return;
    }
    if rel < FEED_AT {
        return;
    }
    let at = AT.load(Ordering::Relaxed);
    if at < 3 {
        let Some((name, souls, charged, eating, vol)) = head(world, at as i32) else {
            if rel % 5000 < tick {
                diag::info!(Sim, "bo2zm t6 retriever: no wolf head {at}");
            }
            return;
        };
        let mut since = SINCE.load(Ordering::Relaxed);
        if since == i64::MIN {
            since = rel;
            SINCE.store(rel, Ordering::Relaxed);
            let c = vol.and_then(|n| middle(world, n));
            // The area's middle may hang well above the floor (the
            // infirmary's is 57 up): try 55 and 110 under it too, and
            // further out when the middle is inside something.
            let spot = c.and_then(|c| {
                [0.0, 55.0, 110.0].iter().find_map(|dz| {
                    [0.0, 60.0, 100.0, 150.0]
                        .iter()
                        .find_map(|d| place_near(world, [c[0], c[1], c[2] - dz], 0.0, *d))
                })
            });
            diag::info!(
                Sim,
                "bo2zm t6 retriever: wolf {at} {name:?} area ent{vol:?} middle {c:?}, he stands at {spot:?}"
            );
            if let Some(p) = spot {
                super::teleport_player(world, client, p);
            }
        }
        let held = rel - since;
        if held % 5000 < tick || charged {
            diag::info!(
                Sim,
                "bo2zm t6 retriever: wolf {at} {name:?} at {}s: souls {souls}, eating {eating}, charged {charged}, round {}",
                held / 1000,
                round(world)
            );
        }
        if charged || held > FEED_MS {
            AT.store(at + 1, Ordering::Relaxed);
            SINCE.store(i64::MIN, Ordering::Relaxed);
        }
        return;
    }
    let got = GOT.load(Ordering::Relaxed);
    if got == i64::MIN {
        let all = with_vm(world, |vm, _| level_flag(vm, "soul_catchers_charged")).unwrap_or(false);
        let trig = ent_by(world, "script_noteworthy", "retriever_pickup_trigger");
        if !all || trig.is_none() {
            if rel % 5000 < tick {
                diag::info!(Sim, "bo2zm t6 retriever: all fed {all}, pickup trigger {trig:?}");
            }
            return;
        }
        GOT.store(rel, Ordering::Relaxed);
        let (_, o) = trig.unwrap_or_default();
        let spin = ent_by(world, "targetname", "spinning_tomahawk_pickup").map(|(_, o)| o);
        let door = ent_by(world, "targetname", "tomahawk_room_door");
        diag::info!(
            Sim,
            "bo2zm t6 retriever: all fed; pickup trigger at ({:.0} {:.0} {:.0}), tomahawk {spin:?}, room door {door:?}",
            o[0],
            o[1],
            o[2]
        );
        if let Some(p) = place_near(world, o, 0.0, 50.0) {
            super::teleport_player(world, client, p);
        }
        return;
    }
    }
    let after = rel - GOT.load(Ordering::Relaxed);
    let trig = ent_by(world, "script_noteworthy", "retriever_pickup_trigger").map(|(_, o)| o);
    if (500..3500).contains(&after) {
        let at = ent_by(world, "targetname", "spinning_tomahawk_pickup")
            .map(|(_, o)| o)
            .or(trig);
        if let Some(at) = at {
            if after % 500 < 250 {
                look_and_use(world, client, at);
            }
        }
        return;
    }
    if (4000..4000 + tick).contains(&after) {
        let names = weapons(world, client);
        let cur = player_field(world, "current_tomahawk_weapon");
        let tac = player_field(world, "current_tactical_grenade");
        diag::info!(
            Sim,
            "bo2zm t6 retriever: after the pickup weapons {names:?}, tomahawk {cur:?}, tactical {tac:?}"
        );
        return;
    }
    // The throw test cooks each throw longer than the last, timed from
    // BO2's own charge clock (n_tomahawk_cooking_time, set on
    // grenade_pullback): 0.4, 1.3, 2.3, 3.3 s held, ~0.3 s more to the
    // throw = n_cookedtime ~0.7/1.6/2.6/3.6 s = power 1, 1.5, 2, 2
    // (Redeemer 1, 4.5, 6, 3: health + 1 above 2).
    static PULLED: AtomicI64 = AtomicI64::new(-1);
    let (span, every) = if throw_only { (120000, 6000) } else { (20000, 3000) };
    if (6000..6000 + span).contains(&after) {
        let step = (after - 6000) % every;
        let k = (after - 6000) / every;
        let hold = if throw_only { [400, 1300, 2300, 3300][(k % 4) as usize] } else { 400 };
        let o = frame(world).player(client).map_or([0.0; 3], |p| p.origin);
        // weapon_give put the tomahawk in his hands; BO2's own pickup
        // (tomahawk_pickup) only gives it, so he keeps his pistol out.
        if throw_only
            && let (Ok(t), Ok(gun)) = (super::weapon(world, axe), super::weapon(world, "m1911_zm"))
            && frame(world).player(client).is_some_and(|p| p.weapon as u32 == t)
        {
            crate::script_player::switch_to_weapon(&mut frame(world), client, gun);
        }
        if step < tick {
            let hand = frame(world).player(client).map(|ps| {
                (ps.weapon, ps.weaponstate_primary, ps.off_hand_index, ps.weap_flags, ps.offhand_secondary)
            });
            let w = super::weapon(world, axe).ok();
            let ammo = w.map(|w| {
                let f = frame(world);
                (crate::script_player::ammo_clip(&f, client, w), crate::script_player::ammo_stock(&f, client, w))
            });
            diag::info!(
                Sim,
                "bo2zm t6 retriever: throw at {}s: tomahawk kills {}, zombie near {}, hand (weapon, state, offhand, flags, tactical) {hand:?}, tomahawk {w:?} ammo {ammo:?}",
                after / 1000,
                player_field(world, "tomahawk_upgrade_kills"),
                nearest_zombie(world, o).is_some()
            );
        }
        // The throw test waits for a zombie within 350 (one behind a wall
        // takes the throw into the wall).
        let near = |z: &[f32; 3]| !throw_only || (z[0] - o[0]).hypot(z[1] - o[1]) < 350.0;
        if let Some(z) = nearest_zombie(world, o).filter(near) {
            let eye = [o[0], o[1], o[2] + 60.0];
            super::set_player_view(world, client, super::autoplay::look(eye, [z[0], z[1], z[2] + 40.0]));
            // Throw test: the charge clock BO2 keeps while he holds it
            // (n_tomahawk_cooking_time, set on grenade_pullback).
            let now_ms = with_vm(world, |vm, _| i64::from(vm.time_ms)).unwrap_or_default();
            let cooking = player_field(world, "n_tomahawk_cooking_time").parse::<i64>().ok();
            if throw_only && step % 250 < tick && step < 5000 {
                let st = frame(world).player(client).map_or(-1, |ps| ps.weaponstate_primary);
                diag::info!(
                    Sim,
                    "bo2zm t6 retriever: throw {k} cook {hold} ms: step {step}, time {now_ms}, cooking {cooking:?}, state {st}"
                );
            }
            let press = if throw_only {
                match cooking {
                    Some(t) => {
                        PULLED.store(k, Ordering::Relaxed);
                        now_ms - t < hold
                    }
                    None => PULLED.load(Ordering::Relaxed) != k && step < 3000,
                }
            } else {
                step < hold
            };
            if press {
                let mut req = world.resource_mut::<crate::step::StepRequest>();
                for (id, cmd) in &mut req.input.cmds {
                    if *id == client {
                        cmd.buttons |= playerstate_iw4::buttons::SMOKE;
                    }
                }
            }
        }
    }
}
