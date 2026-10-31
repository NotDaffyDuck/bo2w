//! bo2mp: guns and scavenger bags on the ground. BO2's `_weapons.gsc`
//! drops a dead player's gun (`dropitem`) and, for a killer with Scavenger,
//! a bag (`dropscavengeritem`); the engine's own dropped items
//! (`crate::floor_guns`: fall, touch for ammo, hold use to swap, bags for
//! Scavenger only) carry them. An item's script entity hears `trigger`
//! (player, the gun he swapped out) or `scavenger` (player) when picked up,
//! then dies, as BO2's `watchpickup` and `scavenger_think` wait for.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, ScriptValue, Vm};
use playerstate_t6::game::NO_ENTITY_SLOT;

use super::{Ent, Zm, arg, client, entnum, frame, text, tick, weapon, with_vm};

const WEAPON_ITEM: &str = "weapon_item";
const SCAVENGER_ITEM: &str = "scavenger_item";

/// A script entity for the engine's dropped item `number`.
fn item_entity(vm: &mut Vm<World>, game: &mut World, num_no: i32, class_name: &str) -> ScriptValue {
    let origin = frame(game)
        .loot_at_slot(num_no)
        .map_or([0.0; 3], |i| i.origin);
    let n = game.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(ObjKind::Entity(n));
    game.resource_mut::<Zm>().ents.insert(
        n,
        Ent {
            obj: Some(obj),
            class_tag: String::from(class_name),
            origin,
            item: Some(num_no),
            ..Default::default()
        },
    );
    ScriptValue::Object(obj)
}

/// The engine's item behind a script entity.
fn item_number(vm: &Vm<World>, world: &World, v: &ScriptValue) -> Option<i32> {
    let n = entnum(vm, v)?;
    world.resource::<Zm>().ents.get(&n)?.item
}

/// The engine's item goes with its script entity (`delete`).
pub(super) fn forget(game: &mut World, num_no: i32) {
    game.resource_mut::<Zm>().item_names.remove(&num_no);
    let mut f = frame(game);
    if f.take_loot_at_slot(num_no).is_some() {
        f.free_dynamic_entity_number(num_no);
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    m!("dropitem", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name)?;
        if w == 0 {
            return Ok(ScriptValue::Undefined);
        }
        let t = tick(world);
        // BO2's ThrowDownWeapon when its weapon facts are loaded: an alternate mode or a
        // left hand throws down its gun, which lands with what the rest of his guns cannot
        // carry (or is only taken).
        let (thrown, dropped) = if frame(world).gun_facts().is_empty() {
            (w, crate::floor_guns::drop_weapon(&mut frame(world), t, cid, w))
        } else {
            crate::floor_gun_facts::drop_weapon(&mut frame(world), t, cid, w)
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
                .get(&cid.0)
                .and_then(|p| p.given_names.get(&thrown).cloned())
        };
        let held: Vec<i32> = frame(world)
            .avatar(cid)
            .map(|ps| ps.weapons.to_vec())
            .unwrap_or_default();
        if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&cid.0) {
            for g in [w, thrown] {
                if !held.contains(&(g as i32)) {
                    p.given_names.remove(&g);
                }
            }
        }
        match dropped {
            Some(number) => {
                diag::info!(Sim, "bo2mp item {number}: player {} dropped {name}", cid.0);
                let catalog = super::weapon_text(world, thrown);
                let mut zm = world.resource_mut::<Zm>();
                zm.item_names.remove(&number);
                if let Some(full) = full.filter(|f| f.contains('+') && *f != catalog) {
                    zm.item_names.insert(number, full);
                }
                Ok(item_entity(vm, world, number, WEAPON_ITEM))
            }
            None => Ok(ScriptValue::Undefined),
        }
    });
    // The bag is BO2's `scavenger_item_mp` (its world model is the bag).
    m!("dropscavengeritem", |vm, world, s, a| {
        let cid = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name).unwrap_or(0);
        if w == 0 {
            diag::warn!(Sim, "bo2mp: no weapon {name} for the scavenger bag");
            return Ok(ScriptValue::Undefined);
        }
        let t = tick(world);
        match crate::floor_guns::drop_scavenger_item(&mut frame(world), t, cid, w) {
            Some(number) => {
                diag::info!(
                    Sim,
                    "bo2mp item {number}: player {} dropped a scavenger bag",
                    cid.0
                );
                Ok(item_entity(vm, world, number, SCAVENGER_ITEM))
            }
            None => Ok(ScriptValue::Undefined),
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
            .filter(|e| e.class_tag == WEAPON_ITEM && e.item.is_some())
            .filter_map(|e| e.obj)
            .collect();
        Ok(super::list(
            guns.into_iter()
                .filter(|obj| vm.alive(*obj))
                .map(ScriptValue::Object)
                .collect(),
        ))
    });
    m!("itemweaponsetammo", |vm, world, s, a| {
        let Some(number) = item_number(vm, world, s) else {
            return Ok(ScriptValue::Undefined);
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
        let Some(item) = f.loot_at_slot(number) else {
            return Ok(ScriptValue::Undefined);
        };
        let w = u32::try_from(item.state.index).unwrap_or(0);
        let facts = f.gun_facts_shared();
        let (sw, size) = if facts.is_empty() {
            (w, f.damage_profile_of(w).map_or(clip, |c| c.magazine_capacity))
        } else {
            let sw = if slot == 0 {
                w
            } else {
                weapon_t6::pickup::slot2_weapon(&facts, w)
            };
            (sw, facts.get(sw).core.magazine_capacity)
        };
        if sw == 0 {
            return Ok(ScriptValue::Undefined);
        }
        let clip = clip.min(size.max(0));
        if let Some(item) = f.loot_at_slot_mut(number) {
            let (clip_slot, stock_slot) = match slot {
                0 => (&mut item.right_mag, &mut item.spare),
                _ => (&mut item.left_mag, &mut item.stock2),
            };
            *clip_slot = clip;
            *stock_slot = stock;
        }
        Ok(ScriptValue::Undefined)
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
            .loot_at_slot(number)
            .map_or(0, |i| u32::try_from(i.state.index).unwrap_or(0));
        let name = crate::client_script::weapon_name(&f, w);
        Ok(vm.string(&name))
    });
}

/// After the engine's touch phase: keep this tick's pickups for the
/// scripts' next frame ([`settle`]).
pub(crate) fn collect_item_pickups(world: &mut World) {
    if !world.contains_resource::<Zm>() {
        return;
    }
    let pickups = frame(world).item_pickups_mut().clone();
    if !pickups.is_empty() {
        world.resource_mut::<Zm>().pickups_log.extend(pickups);
    }
}

/// Items follow the engine's: picked-up ones tell the scripts and die,
/// the rest keep its origin (they fall).
pub(super) fn settle(game: &mut World) {
    let pickups = std::mem::take(&mut game.resource_mut::<Zm>().pickups_log);
    let items: Vec<(u32, i32, ObjRef, bool)> = game
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(nn, en)| Some((*nn, en.item?, en.obj?, en.class_tag == SCAVENGER_ITEM)))
        .collect();
    if items.first().is_none() {
        return;
    }
    // bo2mp: a picked-up gun is his by its full name (attachments and all);
    // the gun he swapped out for it lies there by his.
    for pt in &pickups {
        if pt.ammo_only {
            // A touch took only ammo: the gun's name stays on the item while it lies there.
            if frame(game).loot_at_slot(pt.from_entnum).is_none() {
                game.resource_mut::<Zm>().item_names.remove(&pt.from_entnum);
            }
            continue;
        }
        let swapped = (pt.swapped_entnum != NO_ENTITY_SLOT)
            .then(|| {
                frame(game)
                    .loot_at_slot(pt.swapped_entnum)
                    .map(|ix| u32::try_from(ix.state.index).unwrap_or(0))
            })
            .flatten();
        let mut zm = game.resource_mut::<Zm>();
        let name = zm.item_names.remove(&pt.from_entnum);
        let Some(pl) = zm.players.get_mut(&(pt.picker as u32)) else {
            continue;
        };
        let out = swapped.and_then(|w| pl.given_names.remove(&w));
        if let Some(given) = name {
            pl.given_names.insert(pt.weapon, given);
        } else {
            pl.given_names.remove(&pt.weapon);
        }
        match out {
            Some(out) => {
                zm.item_names.insert(pt.swapped_entnum, out);
            }
            None => {
                zm.item_names.remove(&pt.swapped_entnum);
            }
        }
    }
    let mut gone = Vec::new();
    {
        let mut moved = Vec::new();
        let f = frame(game);
        for (nn, number, obj, _) in &items {
            match f.loot_at_slot(*number) {
                Some(ix) => moved.push((*nn, ix.origin)),
                None => gone.push((*nn, *obj)),
            }
        }
        let mut zm = game.resource_mut::<Zm>();
        for (nn, o) in moved {
            if let Some(en) = zm.ents.get_mut(&nn) {
                en.origin = o;
            }
        }
    }
    with_vm(game, |vm, game| {
        for pt in pickups {
            let Some((obj, scavenger)) = items.iter().find(|ix| ix.1 == pt.from_entnum).map(|ix| (&ix.2, &ix.3)) else {
                continue;
            };
            let Some(pl) = game
                .resource::<Zm>()
                .players
                .get(&(pt.picker as u32))
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
                pt.from_entnum,
                pt.picker,
                pt.swapped_entnum
            );
            if *scavenger {
                vm.notify_str(game, *obj, "scavenger", &[ScriptValue::Object(pl)]);
            } else {
                let swapped = if pt.swapped_entnum == NO_ENTITY_SLOT {
                    ScriptValue::Undefined
                } else {
                    item_entity(vm, game, pt.swapped_entnum, WEAPON_ITEM)
                };
                vm.notify_str(game, *obj, "trigger", &[ScriptValue::Object(pl), swapped]);
            }
        }
        for (nn, obj) in gone {
            game.resource_mut::<Zm>().ents.remove(&nn);
            vm.free_object(game, obj);
        }
    });
}
