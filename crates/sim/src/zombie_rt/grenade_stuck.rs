//! bo2mp: `grenade_stuck` and `grenade_bounce` (BO2's AttachBoltGrenade and
//! MissileImpact): what a grenade, bolt or rocket sticks to hears it with the
//! missile, and so does a breakable it bounces off, for its first five such
//! bounces. The cars' and tanks' scripts wait for it (_destructible.gsc): a
//! bolt, Semtex or C4 stuck to one blows it up when it goes off. A player it
//! sticks to hears it too (the battle chatter's "sticky" warning). Each bounce
//! the missile itself hears as `grenade_bounce` (where, the surface's normal,
//! what it hit and its surface): a Shock Charge off an enemy goes off
//! (_proximity_grenade.gsc), a decoy turns from a wall (_decoy.gsc).

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ObjRef, ScriptValue};

use super::{Zm, killstreaks, with_vm};
use crate::{FlyingProjectile, ScriptModelId};

/// BO2's cap on the breakable bounces one missile tells (destructibleBounceCount).
const BOUNCES: u8 = 5;

/// What a bouncing missile hit.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BounceHit {
    /// The map, or an entity the scripts have no object for (undefined).
    World,
    Player(u32),
    Model(ScriptModelId),
}

#[derive(Clone, Copy)]
enum Struck {
    Stuck(ScriptModelId),
    Bounced {
        pos: [f32; 3],
        normal: [f32; 3],
        hit: BounceHit,
        surface: u8,
    },
}

enum Tell {
    Stuck {
        to: ObjRef,
        missile: ObjRef,
    },
    Bounced {
        missile: ObjRef,
        pos: [f32; 3],
        normal: [f32; 3],
        ent: Option<ObjRef>,
        surface: u8,
    },
}

#[derive(Resource, Default)]
pub(crate) struct GrenadeStuck {
    /// What the grenade code saw since the last pass: the projectile, what
    /// happened, and the passes it has waited.
    struck: Vec<(u32, Struck, u8)>,
    /// Each live missile's breakable bounces told so far.
    bounces: BTreeMap<u32, u8>,
    /// The missiles whose stick to a player was told.
    rode: BTreeSet<u32>,
}

fn queue(world: &mut World, id: u32, what: Struck) {
    if !world.contains_resource::<super::T6Runtime>() {
        return;
    }
    world
        .get_resource_or_insert_with(GrenadeStuck::default)
        .struck
        .push((id, what, 0));
}

/// The grenade code: missile `id` stuck to script model `model`.
pub(crate) fn struck(world: &mut World, id: u32, model: ScriptModelId) {
    queue(world, id, Struck::Stuck(model));
}

/// The grenade code: `projectile` bounced off `hit` (BO2's BounceMissile
/// counted it: a hard bounce, or the one it came to rest on).
pub(crate) fn bounced(
    game: &mut World, proj: &FlyingProjectile,
    norm: [f32; 3],
    hit: BounceHit,
    surface: u8,
) {
    let what = Struck::Bounced {
        pos: proj.origin,
        normal: norm,
        hit,
        surface,
    };
    queue(game, proj.id.0, what);
}

pub(super) fn advance(world: &mut World) {
    let projectiles: Vec<crate::FlyingProjectile> = crate::step_view::gather_shells(world)
        .into_iter()
        .filter(|p| p.in_flight)
        .collect();
    let riders: Vec<(u32, u32)> = projectiles
        .iter()
        .filter_map(|p| Some((p.id.0, p.rider?.client.0)))
        .collect();
    if riders.is_empty() && !world.contains_resource::<GrenadeStuck>() {
        return;
    }
    let mut s = world.remove_resource::<GrenadeStuck>().unwrap_or_default();
    let live: BTreeSet<u32> = projectiles.iter().map(|p| p.id.0).collect();
    s.bounces.retain(|id, _| live.contains(id));
    s.rode.retain(|id| live.contains(id));
    let mut told: Vec<Tell> = Vec::new();
    let mut later = Vec::new();
    for (id, what, waited) in std::mem::take(&mut s.struck) {
        // The scripts' stand-in for it is made the pass after it is thrown:
        // one that strikes at once waits a pass for it.
        let Some(missile) = killstreaks::thrown_ent(world, id) else {
            if waited < 2 && live.contains(&id) {
                later.push((id, what, waited + 1));
            }
            continue;
        };
        let zm = world.resource::<Zm>();
        let Some(m) = zm.ents.get(&missile).and_then(|m| m.obj) else {
            continue;
        };
        let model_ent = |model: ScriptModelId| {
            zm.presences
                .by_ent
                .iter()
                .find(|(_, p)| p.id == model)
                .and_then(|(n, _)| zm.ents.get(n))
        };
        match what {
            Struck::Stuck(model) => {
                if let Some(to) = model_ent(model).and_then(|e| e.obj) {
                    told.push(Tell::Stuck { to, missile: m });
                }
            }
            Struck::Bounced {
                pos,
                normal,
                hit,
                surface,
            } => {
                let (ent, breakable) = match hit {
                    BounceHit::World => (None, None),
                    BounceHit::Player(client) => (zm.players.get(&client).map(|p| p.obj), None),
                    BounceHit::Model(model) => {
                        let e = model_ent(model);
                        let obj = e.and_then(|e| e.obj);
                        (
                            obj,
                            obj.filter(|_| e.is_some_and(|e| e.destructible.is_some())),
                        )
                    }
                };
                // A breakable past its five told bounces tells nothing more.
                if breakable.is_some() && s.bounces.get(&id).copied().unwrap_or(0) >= BOUNCES {
                    continue;
                }
                told.push(Tell::Bounced {
                    missile: m,
                    pos,
                    normal,
                    ent,
                    surface,
                });
                if let Some(to) = breakable {
                    *s.bounces.entry(id).or_default() += 1;
                    told.push(Tell::Stuck { to, missile: m });
                }
            }
        }
    }
    s.struck = later;
    for (id, client) in riders {
        if s.rode.contains(&id) {
            continue;
        }
        let Some(missile) = killstreaks::thrown_ent(world, id) else {
            continue;
        };
        s.rode.insert(id);
        let zm = world.resource::<Zm>();
        if let (Some(to), Some(m)) = (
            zm.players.get(&client).map(|p| p.obj),
            zm.ents.get(&missile).and_then(|m| m.obj),
        ) {
            told.push(Tell::Stuck { to, missile: m });
        }
    }
    world.insert_resource(s);
    if told.is_empty() {
        return;
    }
    with_vm(world, |vm, world| {
        for tell in told {
            match tell {
                Tell::Stuck { to, missile } => {
                    if vm.alive(to) && vm.alive(missile) {
                        diag::info!(Sim, "bo2mp grenade_stuck: missile {missile:?} on {to:?}");
                        vm.notify_str(world, to, "grenade_stuck", &[ScriptValue::Object(missile)]);
                    }
                }
                Tell::Bounced {
                    missile,
                    pos,
                    normal,
                    ent,
                    surface,
                } => {
                    if !vm.alive(missile) {
                        continue;
                    }
                    let ent = ent
                        .filter(|&e| vm.alive(e))
                        .map_or(ScriptValue::Undefined, ScriptValue::Object);
                    let name = weapon_t6::surface::T6_SURFACE_TYPE_NAMES
                        .get(usize::from(surface))
                        .copied()
                        .unwrap_or("default");
                    let surface = vm.string(name);
                    vm.notify_str(
                        world,
                        missile,
                        "grenade_bounce",
                        &[ScriptValue::Vec3(pos), ScriptValue::Vec3(normal), ent, surface],
                    );
                }
            }
        }
    });
}
