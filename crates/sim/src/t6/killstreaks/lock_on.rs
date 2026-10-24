//! Lock-on launchers (the FHJ-18 AA, the SMAW): BO2's scripts
//! (`_heatseekingmissile`) pick the target from `target_getarray`, count the
//! lock up and hand it to the engine (`weaponlockfinalize`); the engine keeps
//! it on the player. His HUD draws its box on the target, and a missile he
//! fires while locked homes on it, at the offset the target was set with
//! (BO2's FireWeapon: the locked entity, `G_TargetGetOffset`). A flare
//! retargets that missile afterwards (`missile_settarget`).

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::super::{Zm, arg, entnum, frame};
use super::streaks;
use crate::WeaponLock;
use crate::world::ClientId;

/// BO2's lock flags: counting up, locked, top attack, direct attack, the
/// target too close, no clear line to it.
const LOCKING: u8 = 1;
const LOCKED: u8 = 2;
const TOP: u8 = 4;
const DIRECT: u8 = 8;
const TOO_CLOSE: u8 = 16;
const NO_CLEARANCE: u8 = 32;
/// The legacy half's point lock, which BO2 has none of.
const POINT_LOCK: u8 = 0x40;

/// BO2's finalize on a lock: counting done, locked, direct attack, its box
/// (and a missile fired now) on `at`.
fn finalize(lock: &mut WeaponLock, weapon: u32, at: Option<[f32; 3]>) {
    lock.weapon = weapon;
    lock.flags |= LOCKING | LOCKED | DIRECT;
    if let Some(p) = at {
        follow(lock, p);
    }
}

/// BO2's free: no lock and no attack mode. Too close and no clearance stay
/// as the scripts last set them.
fn release(lock: &mut WeaponLock) {
    lock.flags &= !(LOCKING | LOCKED | TOP | DIRECT | POINT_LOCK);
    lock.aim = None;
}

/// The lock's box and aim on `p`.
fn follow(lock: &mut WeaponLock, p: [f32; 3]) {
    lock.target = p;
    lock.aim = Some(crate::MissileTarget::Point(p));
}

/// He holds this lock in his life now.
fn holds(lock: WeaponLock, life: u32) -> bool {
    lock.locked() && lock.life == life
}

/// Edit his lock (a new life starts with none).
fn edit(world: &mut World, id: ClientId, f: impl FnOnce(&mut WeaponLock, u32)) {
    let mut fr = frame(world);
    let Some(weapon) = fr.player(id).map(|ps| ps.weapon) else {
        return;
    };
    let meta = fr.client_meta_mut(id);
    let life = meta.life_sequence.0;
    if meta.weapon_lock.life != life {
        meta.weapon_lock = WeaponLock {
            weapon,
            life,
            ..Default::default()
        };
    }
    f(&mut meta.weapon_lock, weapon);
}

/// Let go of his lock.
fn free(world: &mut World, id: ClientId) {
    streaks(world).locks.remove(&id.0);
    edit(world, id, |lock, _| release(lock));
}

/// The locked entity is still in the world.
fn stands(world: &World, n: u32) -> bool {
    let zm = world.resource::<Zm>();
    zm.ents.contains_key(&n) || zm.players.contains_key(&n)
}

/// Where his lock sits now: the target and the offset it was set with.
fn lock_point(world: &mut World, target: u32) -> Option<[f32; 3]> {
    let offset = streaks(world)
        .targets
        .get(&target)
        .copied()
        .unwrap_or([0.0; 3]);
    super::aim_point(world, target, offset)
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // `player weaponlockfinalize(target)`: he has it. Top attack only for a
    // target the scripts set so (`target_setattackmode`, which no MP script
    // calls): direct.
    vm.bind("weaponlockfinalize", true, |vm, world, s, a| {
        let id = super::super::client(vm, world, s)?;
        let target = entnum(vm, arg(a, 0)).ok_or("not an entity")?;
        streaks(world).locks.insert(id.0, target);
        let at = lock_point(world, target);
        edit(world, id, |lock, weapon| finalize(lock, weapon, at));
        Ok(Value::Undefined)
    });
    vm.bind("weaponlockfree", true, |vm, world, s, _| {
        let id = super::super::client(vm, world, s)?;
        free(world, id);
        Ok(Value::Undefined)
    });
    fn lock_bit(vm: &Vm<World>, world: &mut World, s: &Value, a: &[Value], bit: u8) -> Result<Value, String> {
        let id = super::super::client(vm, world, s)?;
        let on = gsc_t6::truthy(arg(a, 0));
        edit(world, id, |lock, _| {
            if on {
                lock.flags |= bit;
            } else {
                lock.flags &= !bit;
            }
        });
        Ok(Value::Undefined)
    }
    vm.bind("weaponlocktargettooclose", true, |vm, world, s, a| {
        lock_bit(vm, world, s, a, TOO_CLOSE)
    });
    vm.bind("weaponlocknoclearance", true, |vm, world, s, a| {
        lock_bit(vm, world, s, a, NO_CLEARANCE)
    });
}

/// The target a missile he fires with `weapon` homes on: his lock's, while
/// he holds it on that weapon and the target still stands.
pub(super) fn locked_target(world: &mut World, client: u32, weapon: u32) -> Option<u32> {
    let target = *streaks(world).locks.get(&client)?;
    let held = frame(world)
        .client_meta(ClientId(client))
        .is_some_and(|m| holds(m.weapon_lock, m.life_sequence.0) && m.weapon_lock.weapon == weapon);
    (held && stands(world, target)).then_some(target)
}

/// Every tick: his lock's box follows its target; a lock on something gone,
/// or from a life before, is let go.
pub(super) fn advance(world: &mut World) {
    let locks: Vec<(u32, u32)> = streaks(world).locks.iter().map(|(c, t)| (*c, *t)).collect();
    for (client, target) in locks {
        let id = ClientId(client);
        let held = frame(world)
            .client_meta(id)
            .is_some_and(|m| holds(m.weapon_lock, m.life_sequence.0));
        let at = if stands(world, target) {
            lock_point(world, target)
        } else {
            None
        };
        match at {
            Some(p) if held => edit(world, id, |lock, _| follow(lock, p)),
            _ if held => free(world, id),
            _ => {
                streaks(world).locks.remove(&client);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finalized_lock_fires_a_direct_missile_at_its_point() {
        let mut lock = WeaponLock { life: 3, ..Default::default() };
        finalize(&mut lock, 77, Some([10.0, 20.0, 30.0]));
        assert_eq!(lock.flags, LOCKING | LOCKED | DIRECT);
        assert_eq!(lock.target, [10.0, 20.0, 30.0]);
        assert!(lock.can_fire(77, 3));
        // Another weapon in his hands, or a life after, fires nothing locked.
        assert!(!lock.can_fire(78, 3));
        assert!(!lock.can_fire(77, 4));
        assert!(holds(lock, 3) && !holds(lock, 4));
    }

    #[test]
    fn too_close_or_no_clearance_hold_the_lock_but_not_the_shot() {
        let mut lock = WeaponLock::default();
        finalize(&mut lock, 5, Some([0.0; 3]));
        lock.flags |= TOO_CLOSE;
        assert!(holds(lock, 0) && !lock.can_fire(5, 0));
        lock.flags &= !TOO_CLOSE;
        lock.flags |= NO_CLEARANCE;
        assert!(!lock.can_fire(5, 0));
    }

    #[test]
    fn a_freed_lock_keeps_only_the_script_bits() {
        let mut lock = WeaponLock::default();
        finalize(&mut lock, 5, Some([1.0; 3]));
        lock.flags |= TOP | NO_CLEARANCE | POINT_LOCK;
        release(&mut lock);
        assert_eq!(lock.flags, NO_CLEARANCE);
        assert_eq!(lock.aim, None);
        assert!(!holds(lock, 0));
    }

    #[test]
    fn the_box_follows_the_target() {
        let mut lock = WeaponLock::default();
        finalize(&mut lock, 5, None);
        assert_eq!(lock.aim, None);
        follow(&mut lock, [4.0, 5.0, 6.0]);
        assert_eq!(lock.aim, Some(crate::MissileTarget::Point([4.0, 5.0, 6.0])));
        assert!(lock.can_fire(5, 0));
    }
}
