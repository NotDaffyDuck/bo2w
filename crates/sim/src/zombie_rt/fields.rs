//! Engine-owned entity fields (`self.origin`, a player's `health`, ...).

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Str, ScriptValue, Vm};

use super::{Zm, frame};
use crate::sim_parts::ClientId;

/// A player's scoreboard counts: engine fields, 0 until set (the scripts
/// count `attacker.headshots++` from nothing).
const STATS: &[&str] = &[
    "headshots",
    "assists",
    "downs",
    "revives",
    "plants",
    "defuses",
    "returns",
    "captures",
    "destructions",
    "survived",
    "stabs",
    "tomahawks",
    "humiliated",
    "x2kills",
    "agrkills",
    "hacks",
    "momentum",
    "objtime",
];

pub(super) fn get(
    vm: &mut Vm<World>,
    game: &mut World,
    obj: ObjRef,
    kind: ObjKind,
    field: Str,
) -> Option<ScriptValue> {
    let ObjKind::Entity(nn) = kind else {
        return None;
    };
    let name = vm.str(field);
    let player = game
        .resource::<Zm>()
        .players
        .get(&nn)
        .map(|p| p.sessionstate.clone());
    if let Some(sessionstate) = player {
        let cid = ClientId(nn);
        return match name {
            "origin" => Some(ScriptValue::Vec3(
                frame(game).avatar(cid).map_or([0.0; 3], |ps| ps.origin),
            )),
            "angles" => {
                // bo2mp: BO2 sets a player's angles to its view's yaw alone
                // every think (ClientThink_real), unless the view rides a
                // link (PLF_WEAPONVIEW_ONLY): a script's
                // anglestoforward(self.angles) runs level. With the pitch
                // in, a bot looking down off a ledge saw the window in
                // front of it as behind it and never knifed the glass
                // (bot_update_glass). Zombies keep the view angles.
                let mp = game.resource::<Zm>().mp;
                let f = frame(game);
                let ps = f.avatar(cid);
                let view = ps.map_or([0.0; 3], |ps| ps.look_angles);
                let linked = ps.is_some_and(|ps| ps.attach_bits & 4 != 0);
                Some(ScriptValue::Vec3(if mp && !linked { [0.0, view[1], 0.0] } else { view }))
            }
            "health" => Some(ScriptValue::Int(
                frame(game).avatar(cid).map_or(0, |ps| ps.health),
            )),
            "maxhealth" => {
                let f = frame(game);
                let meta = f.client_meta(cid).map_or(0, |m| m.hit_point_ceiling);
                Some(ScriptValue::Int(if meta > 0 {
                    meta
                } else {
                    f.avatar(cid).map_or(100, |ps| ps.hit_point_ceiling)
                }))
            }
            "score" => Some(ScriptValue::Int(
                frame(game).client_meta(cid).map_or(0, |m| m.score),
            )),
            "kills" => Some(ScriptValue::Int(
                frame(game).client_meta(cid).map_or(0, |m| m.frags),
            )),
            "deaths" => Some(ScriptValue::Int(
                frame(game).client_meta(cid).map_or(0, |m| m.died),
            )),
            "sessionstate" => Some(vm.string(&sessionstate)),
            "classname" => Some(vm.string("player")),
            "model" => {
                let m = game.resource::<Zm>().players.get(&nn).map(|p| p.mesh.clone());
                Some(vm.string(&m.unwrap_or_default()))
            }
            "name" => {
                let nn = frame(game)
                    .client_meta(cid)
                    .map(|m| {
                        let end = m.name.iter().position(|&b| b == 0).unwrap_or(m.name.len());
                        String::from_utf8_lossy(&m.name[..end]).into_owned()
                    })
                    .unwrap_or_else(|| "Player".to_owned());
                Some(vm.string(&nn))
            }
            "velocity" => Some(ScriptValue::Vec3(
                frame(game).avatar(cid).map_or([0.0; 3], |ps| ps.velocity),
            )),
            // bo2mp: the scorestreak vehicle his view is locked to (the
            // VTOL's and the Dragonfire's ends unlink him only while set).
            "viewlockedentity" => Some(
                super::killstreaks::view_locked(game, nn)
                    .and_then(|v| game.resource::<Zm>().ents.get(&v).and_then(|en| en.obj))
                    .map_or(ScriptValue::Undefined, ScriptValue::Object),
            ),
            // An upgraded staff's charge (t6::charge_shot).
            "chargeshotlevel" => Some(ScriptValue::Int(super::charge_shot::level(game, cid))),
            // 1 from a dive's take-off until its slide ends.
            "divetoprone" => Some(ScriptValue::Int(
                frame(game)
                    .avatar(cid)
                    .map_or(0, |ps| movement_t6::bridge::dive_to_prone(ps) as i32),
            )),
            s if STATS.contains(&s) => Some(ScriptValue::Int(
                game
                    .resource::<Zm>()
                    .players
                    .get(&nn)
                    .and_then(|p| p.stats.get(s).copied())
                    .unwrap_or(0),
            )),
            // bo2mp: engine fields, 0 until a script sets them (a
            // free-for-all's spy plane does); the Lightning Strike's pick
            // shows his enemies only while `hasspyplane` is 0.
            "hasspyplane" | "hassatellite" => match vm.raw_field(obj, field) {
                ScriptValue::Undefined => Some(ScriptValue::Int(0)),
                v => Some(v),
            },
            _ => None,
        };
    }
    // bo2mp: a thrown grenade's or rocket's stand-in has no mdl of its own
    // (one would make it solid); its `mdl` is the projectile's (the cars'
    // `grenade_stuck` check reads it).
    if name == "model"
        && game
            .resource::<Zm>()
            .ents
            .get(&nn)
            .is_some_and(|en| en.mesh.is_empty())
        && let Some(mdl) = super::killstreaks::thrown_shown(game, nn).map(|t| t.1)
    {
        return Some(vm.string(&mdl));
    }
    if (name == "health" || name == "maxhealth") && super::killstreaks::is_vehicle(game, nn) {
        let wants_health = name == "health";
        let (h, m) = super::killstreaks::vehicle_health(vm, obj);
        return Some(ScriptValue::Int(if wants_health { h } else { m }));
    }
    // bo2mp: an entity's box: `mins` and `maxs` about its origin, and
    // `absmin` and `absmax` placed in the game. BO2 has no such entity
    // fields (scripts ask `getmins()`), so once a script sets one it is the
    // script's: Vertigo's doors set theirs to the turned box about the
    // closed door, which their `gettouchingvolume` sweeps.
    if matches!(name, "mins" | "maxs" | "absmin" | "absmax")
        && game.resource::<Zm>().ents.contains_key(&nn)
        && matches!(vm.raw_field(obj, field), ScriptValue::Undefined)
    {
        let (abs, low) = (
            name.starts_with("abs"),
            name.ends_with("min") || name == "mins",
        );
        let me = ScriptValue::Object(obj);
        let (lo, hi) = if abs {
            super::natives_ent::abs_bounds(vm, game, &me)
        } else {
            super::natives_ent::local_bounds(vm, game, &me)
        };
        return Some(ScriptValue::Vec3(if low { lo } else { hi }));
    }
    let zm = game.resource::<Zm>();
    if let Some(a) = zm.actors.get(&nn) {
        match name {
            "health" => return Some(ScriptValue::Int(a.hp)),
            "maxhealth" => return Some(ScriptValue::Int(a.maxhealth)),
            "enemy" => return Some(a.enemy.map_or(ScriptValue::Undefined, ScriptValue::Object)),
            // bo2mp: where its path leads next (a dog runs only on a
            // straight stretch: `dog_move::shouldrun`).
            "lookaheaddist" | "lookaheaddir" => {
                let me = zm.ents.get(&nn).map_or([0.0; 3], |en| en.origin);
                let next = a.path.get(a.path_i).copied().unwrap_or(me);
                let d = [next[0] - me[0], next[1] - me[1], 0.0];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                return Some(if name == "lookaheaddist" {
                    ScriptValue::Float(len)
                } else if len > 1e-3 {
                    ScriptValue::Vec3([d[0] / len, d[1] / len, 0.0])
                } else {
                    let (s, c) = zm.yaw_of(nn)
                        .to_radians()
                        .sin_cos();
                    ScriptValue::Vec3([c, s, 0.0])
                });
            }
            _ => {}
        }
    }
    let en = zm.ents.get(&nn)?;
    match name {
        "origin" => Some(ScriptValue::Vec3(en.origin)),
        "angles" => Some(ScriptValue::Vec3(en.rot)),
        "model" => {
            let m = en.mesh.clone();
            Some(vm.string(&m))
        }
        "classname" => {
            let c = en.class_tag.clone();
            Some(vm.string(&c))
        }
        // Engine fields every entity has, 0 until the map or a script sets
        // them (`destructible_car_explosion` adds to a car's health).
        "health" | "maxhealth" => match vm.raw_field(obj, field) {
            ScriptValue::Undefined => Some(ScriptValue::Int(0)),
            v => Some(v),
        },
        _ => None,
    }
}

pub(super) fn set(
    vm: &mut Vm<World>,
    world: &mut World,
    obj: ObjRef,
    kind: ObjKind,
    field: Str,
    value: &ScriptValue,
) -> bool {
    let ObjKind::Entity(n) = kind else {
        return false;
    };
    let name = vm.str(field).to_owned();
    let is_player = world.resource::<Zm>().players.contains_key(&n);
    if is_player {
        let id = ClientId(n);
        let int = |v: &ScriptValue| v.as_int().unwrap_or(0);
        match name.as_str() {
            "origin" => {
                if let Some(v) = value.as_vec3() {
                    super::teleport_player(world, id, v);
                }
            }
            "angles" => {
                if let Some(v) = value.as_vec3() {
                    super::set_player_view(world, id, v);
                }
            }
            "health" => {
                if let Some(ps) = frame(world).occupant_mut(id) {
                    ps.health = int(value);
                }
            }
            "maxhealth" => {
                let m = int(value).max(1);
                let mut f = frame(world);
                f.client_meta_mut(id).hit_point_ceiling = m;
                if let Some(ps) = f.occupant_mut(id) {
                    ps.hit_point_ceiling = m;
                    ps.health = ps.health.min(m);
                }
            }
            "score" | "kills" | "deaths" => {
                let v = int(value);
                let mut f = frame(world);
                if f.client_meta(id).is_some() {
                    let meta = f.client_meta_mut(id);
                    match name.as_str() {
                        "score" => meta.score = v,
                        "kills" => meta.frags = v,
                        _ => meta.died = v,
                    }
                }
            }
            s if STATS.contains(&s) => {
                let v = int(value);
                if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n) {
                    p.stats.insert(name.clone(), v);
                }
            }
            "sessionstate" => {
                let s = vm.to_text(value);
                if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&n) {
                    // Out of play (game over): no longer down and crawling.
                    if s != "playing" {
                        p.laststand = false;
                    }
                    p.sessionstate = s;
                }
            }
            "sessionteam" => {
                let team = match vm.to_text(value).as_str() {
                    "axis" => entity_t6::team::AXIS,
                    "allies" => entity_t6::team::ALLIES,
                    "spectator" => entity_t6::team::SPECTATOR,
                    _ => entity_t6::team::FREE,
                };
                let mut f = frame(world);
                if f.client_meta(id).is_some() {
                    f.client_meta_mut(id).seat_side = team;
                }
                return false;
            }
            _ => return false,
        }
        return true;
    }
    let mut zm = world.resource_mut::<Zm>();
    if let Some(a) = zm.actors.get_mut(&n) {
        match name.as_str() {
            "health" => {
                a.hp = value.as_int().unwrap_or(0);
                return true;
            }
            "maxhealth" => {
                a.maxhealth = value.as_int().unwrap_or(0);
                return true;
            }
            // bo2mp: its side, for the model each player sees it as (a
            // dog's `aiteam`; the field keeps its value too).
            "aiteam" => a.team = vm.to_text(value),
            _ => {}
        }
    }
    let Some(e) = zm.ents.get_mut(&n) else {
        return false;
    };
    match name.as_str() {
        "origin" => {
            if let Some(v) = value.as_vec3() {
                e.origin = v;
            }
            zm.movers.stop(n, true, false);
            true
        }
        "angles" => {
            if let Some(v) = value.as_vec3() {
                e.rot = v;
            }
            zm.movers.stop(n, false, true);
            true
        }
        "model" | "classname" => true,
        // BO2's `Scr_SetHealth`: an entity's health set from a script is its
        // most health too.
        "health" => {
            let h = ScriptValue::Int(value.as_int().unwrap_or(0));
            let most = vm.intern("maxhealth");
            vm.set_raw_field(obj, field, h.clone());
            vm.set_raw_field(obj, most, h);
            true
        }
        _ => false,
    }
}
