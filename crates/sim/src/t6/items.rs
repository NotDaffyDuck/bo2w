//! bo2mp: guns and scavenger bags on the ground. BO2's `_weapons.gsc`
//! drops a dead player's gun (`dropitem`) and, for a killer with Scavenger,
//! a bag (`dropscavengeritem`); the engine's own dropped items
//! (`crate::item`: fall, touch for ammo, hold use to swap, bags for
//! Scavenger only) carry them. An item's script entity hears `trigger`
//! (player, the gun he swapped out) or `scavenger` (player) when picked up,
//! then dies, as BO2's `watchpickup` and `scavenger_think` wait for.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Value, Vm};
use playerstate_t6::game::ENTITYNUM_NONE;

use super::{Ent, Zm, arg, client, entnum, frame, text, tick, weapon, with_vm};

const WEAPON_ITEM: &str = "weapon_item";
const SCAVENGER_ITEM: &str = "scavenger_item";

/// A script entity for the engine's dropped item `number`.
fn item_entity(vm: &mut Vm<World>, world: &mut World, number: i32, classname: &str) -> Value {
    let origin = frame(world)
        .dropped_item_by_number(number)
        .map_or([0.0; 3], |i| i.origin);
    let n = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(ObjKind::Entity(n));
    world.resource_mut::<Zm>().ents.insert(
        n,
        Ent {
            obj: Some(obj),
            classname: classname.to_owned(),
            origin,
            item: Some(number),
            ..Default::default()
        },
    );
    Value::Object(obj)
}

/// The engine's item behind a script entity.
fn item_number(vm: &Vm<World>, world: &World, v: &Value) -> Option<i32> {
    let n = entnum(vm, v)?;
    world.resource::<Zm>().ents.get(&n)?.item
}

/// The engine's item goes with its script entity (`delete`).
pub(super) fn forget(world: &mut World, number: i32) {
    world.resource_mut::<Zm>().item_names.remove(&number);
    let mut f = frame(world);
    if f.remove_dropped_item_by_number(number).is_some() {
        f.free_dynamic_entity_number(number);
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    m!("dropitem", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name)?;
        if w == 0 {
            return Ok(Value::Undefined);
        }
        let t = tick(world);
        // BO2's ThrowDownWeapon when its weapon facts are loaded: an alternate mode or a
        // left hand throws down its gun, which lands with what the rest of his guns cannot
        // carry (or is only taken).
        let (thrown, dropped) = if frame(world).gun_facts().is_empty() {
            (w, crate::item::drop_weapon(&mut frame(world), t, id, w))
        } else {
            crate::item_t6::drop_weapon(&mut frame(world), t, id, w)
        };
        // bo2mp: the gun keeps its full name, attachments and all (the scripts drop it by
        // the name it was given by; a gun thrown down for its alternate mode keeps the name
        // it was given by). Item numbers come back: an old one's name never stays.
        let full = if thrown == w {
            Some(name.clone())
        } else {
            world
                .resource::<Zm>()
                .players
                .get(&id.0)
                .and_then(|p| p.given_names.get(&thrown).cloned())
        };
        let held: Vec<i32> = frame(world)
            .player(id)
            .map(|ps| ps.weapons.to_vec())
            .unwrap_or_default();
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&id.0) {
            for g in [w, thrown] {
                if !held.contains(&(g as i32)) {
                    p.given_names.remove(&g);
                }
            }
        }
        match dropped {
            Some(number) => {
                diag::info!(Sim, "bo2mp item {number}: player {} dropped {name}", id.0);
                let catalog = super::weapon_text(world, thrown);
                let mut zm = world.resource_mut::<Zm>();
                zm.item_names.remove(&number);
                if let Some(full) = full.filter(|f| f.contains('+') && *f != catalog) {
                    zm.item_names.insert(number, full);
                }
                Ok(item_entity(vm, world, number, WEAPON_ITEM))
            }
            None => Ok(Value::Undefined),
        }
    });
    // The bag is BO2's `scavenger_item_mp` (its world model is the bag).
    m!("dropscavengeritem", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name).unwrap_or(0);
        if w == 0 {
            diag::warn!(Sim, "bo2mp: no weapon {name} for the scavenger bag");
            return Ok(Value::Undefined);
        }
        let t = tick(world);
        match crate::item::drop_scavenger_item(&mut frame(world), t, id, w) {
            Some(number) => {
                diag::info!(
                    Sim,
                    "bo2mp item {number}: player {} dropped a scavenger bag",
                    id.0
                );
                Ok(item_entity(vm, world, number, SCAVENGER_ITEM))
            }
            None => Ok(Value::Undefined),
        }
    });
    // Every gun on the ground (the engine's dropped-weapon queue, not the
    // Scavenger bags): an exploding car's script lifts the car, deletes the
    // guns resting on it, then sets it down again.
    vm.bind("getdroppedweapons", false, |vm, world, _, _| {
        let guns: Vec<ObjRef> = world
            .resource::<Zm>()
            .ents
            .values()
            .filter(|e| e.classname == WEAPON_ITEM && e.item.is_some())
            .filter_map(|e| e.obj)
            .collect();
        Ok(super::list(
            guns.into_iter()
                .filter(|obj| vm.alive(*obj))
                .map(Value::Object)
                .collect(),
        ))
    });
    m!("itemweaponsetammo", |vm, world, s, a| {
        let Some(number) = item_number(vm, world, s) else {
            return Ok(Value::Undefined);
        };
        // BO2's ScrCmd_ItemWeaponSetAmmo(clip, stock[, slot]): slot 0 is the gun, slot 1 its
        // left hand or alternate mode; the magazine is cut to that weapon's clip size, and a
        // slot with no weapon behind it is left alone.
        let clip = arg(a, 0).as_int().unwrap_or(0);
        let stock = arg(a, 1).as_int().unwrap_or(0);
        if clip < 0 || stock < 0 {
            return Err("Ammo count must not be negative".into());
        }
        let slot = match arg(a, 2).as_int() {
            None => 0,
            Some(s @ 0..=1) => s,
            Some(_) => return Err("Value out of range.  Allowed values: 0 to 1".into()),
        };
        let mut f = frame(world);
        let Some(item) = f.dropped_item_by_number(number) else {
            return Ok(Value::Undefined);
        };
        let w = u32::try_from(item.state.index).unwrap_or(0);
        let facts = f.gun_facts_shared();
        let (sw, size) = if facts.is_empty() {
            (w, f.combat_facts_for(w).map_or(clip, |c| c.clip_size))
        } else {
            let sw = if slot == 0 {
                w
            } else {
                weapon_t6::pickup::slot2_weapon(&facts, w)
            };
            (sw, facts.get(sw).core.clip_size)
        };
        if sw == 0 {
            return Ok(Value::Undefined);
        }
        let clip = clip.min(size.max(0));
        if let Some(item) = f.dropped_item_mut_by_number(number) {
            if slot == 0 {
                item.clip_r = clip;
                item.stock = stock;
            } else {
                item.clip_l = clip;
                item.stock2 = stock;
            }
        }
        Ok(Value::Undefined)
    });
    m!("getitemweaponname", |vm, world, s, _| {
        let Some(number) = item_number(vm, world, s) else {
            return Ok(vm.string("none"));
        };
        if let Some(name) = world.resource::<Zm>().item_names.get(&number) {
            return Ok(vm.string(name));
        }
        let f = frame(world);
        let w = f
            .dropped_item_by_number(number)
            .map_or(0, |i| u32::try_from(i.state.index).unwrap_or(0));
        let name = crate::script_player::weapon_name(&f, w);
        Ok(vm.string(&name))
    });
}

/// After the engine's touch phase: keep this tick's pickups for the
/// scripts' next frame ([`settle`]).
pub(crate) fn collect_pickups(world: &mut World) {
    if !world.contains_resource::<Zm>() {
        return;
    }
    let pickups = frame(world).item_pickups_mut().clone();
    if !pickups.is_empty() {
        world.resource_mut::<Zm>().item_pickups.extend(pickups);
    }
}

/// Items follow the engine's: picked-up ones tell the scripts and die,
/// the rest keep its origin (they fall).
pub(super) fn settle(world: &mut World) {
    let pickups = std::mem::take(&mut world.resource_mut::<Zm>().item_pickups);
    let items: Vec<(u32, i32, ObjRef, bool)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(n, e)| Some((*n, e.item?, e.obj?, e.classname == SCAVENGER_ITEM)))
        .collect();
    if items.is_empty() {
        return;
    }
    // bo2mp: a picked-up gun is his by its full name (attachments and all);
    // the gun he swapped out for it lies there by his.
    for p in &pickups {
        if p.ammo_only {
            // A touch took only ammo: the gun's name stays on the item while it lies there.
            if frame(world).dropped_item_by_number(p.from_entnum).is_none() {
                world.resource_mut::<Zm>().item_names.remove(&p.from_entnum);
            }
            continue;
        }
        let swapped = (p.swapped_entnum != ENTITYNUM_NONE)
            .then(|| {
                frame(world)
                    .dropped_item_by_number(p.swapped_entnum)
                    .map(|i| u32::try_from(i.state.index).unwrap_or(0))
            })
            .flatten();
        let mut zm = world.resource_mut::<Zm>();
        let name = zm.item_names.remove(&p.from_entnum);
        let Some(player) = zm.players.get_mut(&(p.picker as u32)) else {
            continue;
        };
        let out = swapped.and_then(|w| player.given_names.remove(&w));
        match name {
            Some(name) => {
                player.given_names.insert(p.weapon, name);
            }
            None => {
                player.given_names.remove(&p.weapon);
            }
        }
        match out {
            Some(out) => {
                zm.item_names.insert(p.swapped_entnum, out);
            }
            None => {
                zm.item_names.remove(&p.swapped_entnum);
            }
        }
    }
    let mut gone = Vec::new();
    {
        let mut moved = Vec::new();
        let f = frame(world);
        for (n, number, obj, _) in &items {
            match f.dropped_item_by_number(*number) {
                Some(i) => moved.push((*n, i.origin)),
                None => gone.push((*n, *obj)),
            }
        }
        let mut zm = world.resource_mut::<Zm>();
        for (n, o) in moved {
            if let Some(e) = zm.ents.get_mut(&n) {
                e.origin = o;
            }
        }
    }
    with_vm(world, |vm, world| {
        for p in pickups {
            let Some((_, _, obj, scavenger)) = items.iter().find(|i| i.1 == p.from_entnum) else {
                continue;
            };
            let Some(player) = world
                .resource::<Zm>()
                .players
                .get(&(p.picker as u32))
                .map(|x| x.obj)
            else {
                continue;
            };
            if !vm.alive(*obj) {
                continue;
            }
            diag::info!(
                Sim,
                "bo2mp item {}: player {} picked it up (swapped out item {})",
                p.from_entnum,
                p.picker,
                p.swapped_entnum
            );
            if *scavenger {
                vm.notify_str(world, *obj, "scavenger", &[Value::Object(player)]);
            } else {
                let swapped = if p.swapped_entnum == ENTITYNUM_NONE {
                    Value::Undefined
                } else {
                    item_entity(vm, world, p.swapped_entnum, WEAPON_ITEM)
                };
                vm.notify_str(world, *obj, "trigger", &[Value::Object(player), swapped]);
            }
        }
        for (n, obj) in gone {
            world.resource_mut::<Zm>().ents.remove(&n);
            vm.free_object(world, obj);
        }
    });
}
