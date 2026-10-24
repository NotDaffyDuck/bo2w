//! bo2mp: a ballistic knife's blade lands (Black Ops II's
//! createRetrieveableProjectile). Its missile hears `stationary` with where
//! it stopped, the surface's normal, its facing, who fired it, what it struck
//! and the bone it struck, then goes with no blast. _ballistic_knife.gsc
//! leaves a blade model there for its owner to pick up: stuck in the body of
//! the player it killed, knocked off a teammate.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::Value;

use super::{Zm, frame, killstreaks, with_vm};
use crate::ScriptModelId;
use crate::world::ClientId;

/// What a blade struck.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Prey {
    /// The map, or nothing the scripts know.
    World,
    /// A player, at Black Ops II hit location `hitloc`.
    Player { client: u32, hitloc: u8 },
    /// A script model (a car, a crate).
    Model(ScriptModelId),
}

/// Where and how a blade stopped.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Landing {
    pub endpos: [f32; 3],
    pub normal: [f32; 3],
    /// Which way it flew.
    pub dir: [f32; 3],
    pub prey: Prey,
}

/// What the streaks' projectile pass tells a missile at rest.
pub(crate) enum Landed {
    /// No blade landed (any other missile): the plain `stationary`.
    No,
    /// The player it killed has no body yet: ask again next pass.
    Wait,
    /// A blade landed: `stationary` with these six, then it goes quietly.
    Now(Vec<Value>),
}

/// Passes a blade waits for the body of the player it killed (his killed
/// callback clones it).
const BODY_WAIT: u8 = 3;

/// The most landings kept for missiles the scripts never saw come to rest.
const KEPT: usize = 64;

#[derive(Resource, Default)]
struct Landings(BTreeMap<u32, (Landing, u8)>);

/// The grenade code: blade missile `id` stopped.
pub(crate) fn record(world: &mut World, id: u32, landing: Landing) {
    if !world.get_resource::<Zm>().is_some_and(|z| z.mp) {
        return;
    }
    let mut l = world.get_resource_or_insert_with(Landings::default);
    while l.0.len() >= KEPT {
        l.0.pop_first();
    }
    l.0.insert(id, (landing, 0));
}

/// Missile `id`, fired by player `owner`, is at rest: is it a blade, and
/// what does it tell?
pub(crate) fn landed(world: &mut World, id: u32, owner: u32) -> Landed {
    let Some((landing, waited)) = world
        .get_resource::<Landings>()
        .and_then(|l| l.0.get(&id).copied())
    else {
        return Landed::No;
    };
    let alive = match landing.prey {
        Prey::Player { client, .. } => {
            let playing = world
                .resource::<Zm>()
                .players
                .get(&client)
                .is_some_and(|p| p.sessionstate == "playing");
            playing && frame(world).player(ClientId(client)).is_some_and(|ps| ps.health > 0)
        }
        _ => false,
    };
    let told = with_vm(world, |vm, world| {
        let zm = world.resource::<Zm>();
        let attacker = zm
            .players
            .get(&owner)
            .map_or(Value::Undefined, |p| Value::Object(p.obj));
        let (prey, bone) = match landing.prey {
            Prey::World => (Value::Undefined, ""),
            Prey::Player { client, hitloc } => match zm.players.get(&client).map(|p| p.obj) {
                None => (Value::Undefined, ""),
                Some(him) if alive => (Value::Object(him), ragdoll_bone(hitloc)),
                // Dead: his body, once his killed callback has made it.
                Some(him) => {
                    let f = vm.intern("body");
                    match vm.raw_field(him, f) {
                        Value::Object(body) if vm.alive(body) => {
                            (Value::Object(body), ragdoll_bone(hitloc))
                        }
                        _ if waited < BODY_WAIT => return None,
                        _ => (Value::Undefined, ""),
                    }
                }
            },
            Prey::Model(model) => {
                let obj = zm
                    .presences
                    .by_ent
                    .iter()
                    .find(|(_, p)| p.id == model)
                    .and_then(|(n, _)| zm.ents.get(n))
                    .and_then(|e| e.obj);
                (obj.map_or(Value::Undefined, Value::Object), "")
            }
        };
        let bone = vm.string(bone);
        Some(vec![
            Value::Vec3(landing.endpos),
            Value::Vec3(landing.normal),
            Value::Vec3(killstreaks::aim_angles(landing.dir)),
            attacker,
            prey,
            bone,
        ])
    });
    let mut l = world.resource_mut::<Landings>();
    match told {
        Some(Some(args)) => {
            l.0.remove(&id);
            Landed::Now(args)
        }
        Some(None) => {
            if let Some(w) = l.0.get_mut(&id) {
                w.1 += 1;
            }
            Landed::Wait
        }
        None => {
            l.0.remove(&id);
            Landed::No
        }
    }
}

/// Black Ops II's MapHitLocationToRagdollBoneName: the bone a blade that
/// struck hit location `hitloc` sticks to ("" for the mid torso and none).
fn ragdoll_bone(hitloc: u8) -> &'static str {
    match hitloc {
        1 | 2 => "j_head",
        3 => "j_neck",
        4 => "j_spineupper",
        6 => "j_spinelower",
        7 => "j_elbow_ri",
        8 => "j_elbow_le",
        9 | 11 => "j_wrist_ri",
        10 | 12 => "j_wrist_le",
        13 => "j_knee_ri",
        14 => "j_knee_le",
        15 | 17 => "j_ankle_ri",
        16 | 18 => "j_ankle_le",
        // The riot shield: this engine numbers it 19, Black Ops II 20.
        19 | 20 => "tag_weapon_left",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::ragdoll_bone;

    #[test]
    fn blade_bones_follow_black_ops_ii() {
        assert_eq!(ragdoll_bone(1), "j_head");
        assert_eq!(ragdoll_bone(5), "");
        assert_eq!(ragdoll_bone(12), "j_wrist_le");
        assert_eq!(ragdoll_bone(17), "j_ankle_ri");
        assert_eq!(ragdoll_bone(19), "tag_weapon_left");
        assert_eq!(ragdoll_bone(0), "");
    }
}
