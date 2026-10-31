//! bo2mp: a flash grenade going off (BO2's G_FlashbangBlast and
//! FlashbangBlastEnt). Each player in play within its explosion radius, and
//! each scorestreak vehicle driven from afar with someone in it (the RC-XD),
//! whose eye can see where it went off is sent "flashbang": how near it was
//! (1 inside the inner radius, falling to 0 at the outer), how squarely it
//! was faced (1 straight at it, 0 straight away), who threw it and the
//! thrower's team. The scripts (_flashgrenades) blind and deafen a player;
//! a vehicle passes it on to its driver.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue};
use playerstate_t6::game::AvatarState;

use super::{T6Runtime, Zm, frame, killstreaks, with_vm};
use crate::sim_parts::ClientId;

/// What stands between the flash and an eye.
const MASK: u32 = 0x0080_7823;

/// How near it went off and how squarely it was faced (amount_distance,
/// amount_angle), for an entity `dist` from it whose eye at `eye` looks
/// along `facing`.
fn amounts(
    dist: f32,
    min_r: f32,
    max_r: f32,
    eye: [f32; 3],
    heading: [f32; 3],
    spot: [f32; 3],
) -> (f32, f32) {
    let near = if dist <= min_r {
        1.0
    } else {
        let span = max_r - min_r;
        1.0 - (dist - min_r) / span
    };
    let to: [f32; 3] = std::array::from_fn(|i| spot[i] - eye[i]);
    let len = to.iter().map(|v| v * v).sum::<f32>().sqrt();
    let dot = if len > 0.0 {
        (0..3).map(|i| heading[i] * to[i]).sum::<f32>() / len
    } else {
        0.0
    };
    (near, (dot + 1.0) * 0.5)
}

/// BO2 looks with a sight trace, which has no surface epsilon: a flash
/// lying on the floor is seen. Our clip trace stops 0.125 short of every
/// surface, so a hit counts only when the line truly reaches the plane
/// before its end. The sight trace also meets the static models (fences,
/// cars, crates): a flash behind one is not seen.
fn sees(game: &mut World, eye_at: [f32; 3], spot: [f32; 3]) -> bool {
    let t = frame(game).trace_static_world_models(eye_at, spot, MASK);
    let f = t.fraction.clamp(0.0, 1.0);
    if f >= 1.0 {
        return true;
    }
    let into: f32 = (0..3).map(|i| t.normal[i] * (eye_at[i] - spot[i])).sum();
    into > 1e-6 && f + 0.125 / into >= 1.0
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

/// The thrower's team, by the engine's name for it.
fn team_name(team: i32) -> &'static str {
    match team {
        entity_t6::team::AXIS => "axis",
        entity_t6::team::ALLIES => "allies",
        entity_t6::team::SPECTATOR => "spectator",
        _ => "free",
    }
}

/// A flash went off at `origin`. False without BO2's scripts running (the
/// legacy game handles it itself).
pub(crate) fn blast(
    sim_world: &mut World,
    spot: [f32; 3],
    outer_in: f32,
    inner_in: f32,
    hitter: ClientId,
) -> bool {
    if !sim_world.contains_resource::<T6Runtime>() {
        return false;
    }
    let inner_r = inner_in.max(1.0);
    let outer_r = outer_in.max(inner_r);
    let states: BTreeMap<u32, AvatarState> = crate::step_view::gather_avatars(sim_world)
        .into_iter()
        .map(|(c, pstate)| (c.0, pstate))
        .collect();
    // Each one it may reach: its object, where it stands, its eye_at and where
    // it looks.
    let mut near: Vec<(ObjRef, [f32; 3], [f32; 3], [f32; 3])> = Vec::new();
    {
        let zm = sim_world.resource::<Zm>();
        for (c, pstate) in &states {
            let Some(p) = zm.players.get(c) else {
                continue;
            };
            if p.sessionstate != "playing" {
                continue;
            }
            let eye_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
            near.push((p.obj, pstate.origin, eye_at, pstate.look_angles));
        }
        for d in killstreaks::remote_driven(sim_world) {
            let Some(e) = zm.ents.get(&d.craft) else {
                continue;
            };
            let Some(obj) = e.obj else {
                continue;
            };
            let look = d
                .driver
                .and_then(|d| states.get(&d))
                .map_or(e.rot, |pstate| pstate.look_angles);
            near.push((obj, e.origin, e.origin, look));
        }
    }
    let mut hits = Vec::default();
    for (obj, at, eye_at, look) in near {
        let d = dist(at, spot);
        if d > outer_r || !sees(sim_world, eye_at, spot) {
            continue;
        }
        let (facing, _, _) = game_math::euler_basis(look);
        hits.push((obj, amounts(d, inner_r, outer_r, eye_at, facing, spot)));
    }
    if hits.first().is_none() {
        return true;
    }
    let thrower = sim_world
        .resource::<Zm>()
        .players
        .get(&hitter.0)
        .map(|p| p.obj);
    let team_id = frame(sim_world).client_meta(hitter).map_or(entity_t6::team::FREE, |m| m.seat_side);
    let team = team_name(team_id);
    with_vm(sim_world, |vm, sim_world| {
        let by = thrower.map_or(ScriptValue::Undefined, ScriptValue::Object);
        let team = vm.string(team);
        for (obj, (near, angle)) in hits {
            vm.notify_str(
                sim_world,
                obj,
                "flashbang",
                &[ScriptValue::Float(near), ScriptValue::Float(angle), by.clone(), team.clone()],
            );
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_follow_distance_and_facing() {
        let eye = [0.0, 0.0, 60.0];
        let flash = [100.0, 0.0, 60.0];
        // Inside the inner radius, looking straight at it.
        let (n, a) = amounts(100.0, 150.0, 650.0, eye, [1.0, 0.0, 0.0], flash);
        assert_eq!((n, a), (1.0, 1.0));
        // Halfway between the radii, looking straight away.
        let (n, a) = amounts(400.0, 150.0, 650.0, eye, [-1.0, 0.0, 0.0], flash);
        assert!((n - 0.5).abs() < 1e-6 && a.abs() < 1e-6);
        // At the outer radius, looking across it.
        let (n, a) = amounts(650.0, 150.0, 650.0, eye, [0.0, 1.0, 0.0], flash);
        assert!(n.abs() < 1e-6 && (a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn team_names_are_the_engines() {
        assert_eq!(team_name(entity_t6::team::AXIS), "axis");
        assert_eq!(team_name(entity_t6::team::ALLIES), "allies");
        assert_eq!(team_name(entity_t6::team::SPECTATOR), "spectator");
        assert_eq!(team_name(entity_t6::team::FREE), "free");
    }

    #[test]
    fn without_bo2s_scripts_the_stub_game_handles_it() {
        let mut world = World::new();
        assert!(!blast(&mut world, [0.0; 3], 650.0, 150.0, ClientId(0)));
    }
}
