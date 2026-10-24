//! bo2mp: the Sensor Grenade (BO2's CG_SensorGrenade and
//! G_SensorGrenadeDetonate). While it is live it scans: each pulse
//! (cg_sensorGrenadePulsePeriodMs, 800) sends a runner out from it, one look
//! to its owner's side and another to the rest, and for the first
//! cg_sensorGrenadeLaserActiveTimeMs (400) of each pulse its laser sweeps its
//! owner's enemies within cg_sensorGrenadeRange (1000), one after another in
//! client order. An enemy the laser reaches with nothing in the way shows a
//! beam to him, and each new one sounds an alert at the grenade, plays the
//! targeting effect toward him and pings him on the owner's minimap. When it
//! goes off, each enemy within its explosion radius it can see is tracked on
//! the owner's minimap for cg_sensorGrenadeExplosionTrackTimeMs (6000). A
//! player with specialty_nomotionsensor is never pinged or tracked.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use playerstate_t6::game::{PlayerState, pm_flags};

use super::{Zm, frame, is_bot, killstreaks, natives_fx, set_client_dvar, with_vm};
use crate::world::ClientId;
use crate::{EventAudience, ProjectileState};

/// cg_sensorGrenadePulsePeriodMs.
const PULSE_MS: i64 = 800;
/// cg_sensorGrenadeLaserActiveTimeMs.
const LASER_MS: i64 = 400;
/// cg_sensorGrenadeRange.
const RANGE: f32 = 1000.0;
/// cg_sensorGrenadeExplosionTrackTimeMs.
const TRACK_MS: i64 = 6000;
/// What blocks its laser.
const MASK: u32 = 0x0280_e833;
/// cg_sensorGrenadeZOffset, cg_sensorGrenadeCrouchZOffset and
/// cg_sensorGrenadeProneZOffset: where on an enemy the laser aims.
const Z_STAND: f32 = 35.0;
const Z_CROUCH: f32 = 25.0;
const Z_PRONE: f32 = 5.0;
const NO_SENSOR: &str = "specialty_nomotionsensor";
const RUNNER_FRIENDLY: &str = "weapon/sensor_grenade/fx_sensor_scan_friendly_runner";
const RUNNER_ENEMY: &str = "weapon/sensor_grenade/fx_sensor_scan_enemy_runner";
const TARGETING: &str = "weapon/sensor_grenade/fx_sensor_targeting";
const ALERT: &str = "fly_sensor_alert";
/// The client dvar each owner's minimap reads.
const DVAR: &str = "bo2mp_sensor";

#[derive(Resource, Default)]
pub(crate) struct SensorGrenades {
    /// Each live sensor's time into its pulse last pass.
    pulse: BTreeMap<u32, i64>,
    /// Each owner's enemies a sensor that went off tracks, and until when.
    tracked: BTreeMap<u32, BTreeMap<u32, i64>>,
    /// Each owner's pings so far, per enemy: his minimap marks each new one.
    pings: BTreeMap<u32, BTreeMap<u32, u32>>,
    /// What each owner's client was last sent.
    sent: BTreeMap<u32, String>,
}

/// Whether the game is team based, and each player's team.
fn sides(world: &mut World) -> (bool, BTreeMap<u32, String>) {
    with_vm(world, |vm, world| {
        let tb = vm.intern("teambased");
        let teambased = gsc_t6::truthy(&vm.raw_field(vm.level, tb));
        let team_s = vm.intern("team");
        let players: Vec<(u32, gsc_t6::ObjRef)> = world
            .resource::<Zm>()
            .players
            .iter()
            .map(|(n, p)| (*n, p.obj))
            .collect();
        let teams = players
            .into_iter()
            .map(|(n, o)| (n, vm.to_text(&vm.raw_field(o, team_s))))
            .collect();
        (teambased, teams)
    })
    .unwrap_or_default()
}

fn friend(teambased: bool, teams: &BTreeMap<u32, String>, owner: u32, other: u32) -> bool {
    other == owner || (teambased && teams.get(&other) == teams.get(&owner))
}

/// The players in play, alive, with where they stand.
fn alive(world: &World) -> Vec<(u32, PlayerState)> {
    let zm = world.resource::<Zm>();
    crate::frame::collect_players(world)
        .into_iter()
        .filter(|(c, ps)| {
            ps.health > 0
                && zm
                    .players
                    .get(&c.0)
                    .is_some_and(|p| p.sessionstate == "playing")
        })
        .map(|(c, ps)| (c.0, ps))
        .collect()
}

/// Where on him the laser aims.
fn aim(ps: &PlayerState) -> [f32; 3] {
    let z = if ps.pm_flags & pm_flags::CROUCH != 0 {
        Z_CROUCH
    } else if ps.pm_flags & pm_flags::PRONE != 0 {
        Z_PRONE
    } else {
        Z_STAND
    };
    [ps.origin[0], ps.origin[1], ps.origin[2] + z]
}

fn clear(world: &mut World, from: [f32; 3], to: [f32; 3]) -> bool {
    frame(world)
        .trace_static_world(from, to, [0.0; 3], [0.0; 3], MASK)
        .fraction
        >= 1.0
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn has_perk(world: &World, client: u32, perk: &str) -> bool {
    world
        .resource::<Zm>()
        .players
        .get(&client)
        .is_some_and(|p| p.perks.contains(perk))
}

fn is_sensor(world: &mut World, weapon: u32) -> bool {
    frame(world)
        .equipment_facts_for(weapon)
        .is_some_and(|f| f.laser_sensor)
}

/// A pulse's runner: lying still it plays where it lies, in flight it rides
/// the grenade.
fn runner(world: &mut World, p: &ProjectileState, teambased: bool, teams: &BTreeMap<u32, String>) {
    let owner = p.owner.0;
    let (friends, foes): (Vec<u32>, Vec<u32>) = teams
        .keys()
        .copied()
        .partition(|&n| friend(teambased, teams, owner, n));
    let looks = [(friends, RUNNER_FRIENDLY), (foes, RUNNER_ENEMY)];
    let still = p.grounded || p.pos.tr_type == entity_t6::trajectory::tr_type::STATIONARY;
    let ent = if still {
        None
    } else {
        killstreaks::thrown_ent(world, p.id.0)
    };
    for (who, name) in looks {
        if who.is_empty() {
            continue;
        }
        let audience = EventAudience::Clients(who.into_iter().map(ClientId).collect());
        match ent {
            Some(n) => {
                natives_fx::bolt_effect_to(world, audience, n, name, "tag_origin", false);
            }
            None => {
                natives_fx::effect_axis_to(
                    world,
                    audience,
                    name,
                    p.origin,
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                );
            }
        }
    }
}

pub(super) fn advance(world: &mut World) {
    let sensors: Vec<ProjectileState> = crate::frame::collect_projectiles(world)
        .into_iter()
        .filter(|p| p.live)
        .collect();
    let sensors: Vec<ProjectileState> = sensors
        .into_iter()
        .filter(|p| is_sensor(world, p.weapon))
        .collect();
    if sensors.is_empty() && !world.contains_resource::<SensorGrenades>() {
        return;
    }
    let mut s = world
        .remove_resource::<SensorGrenades>()
        .unwrap_or_default();
    let now = world.resource::<Zm>().now_ms;
    // (With none live there is only the tracking to age.)
    let (teambased, teams, alive) = if sensors.is_empty() {
        (false, BTreeMap::new(), Vec::new())
    } else {
        let (teambased, teams) = sides(world);
        (teambased, teams, alive(world))
    };
    s.pulse
        .retain(|id, _| sensors.iter().any(|p| p.id.0 == *id));
    for p in &sensors {
        let owner = p.owner.0;
        let t = (now - i64::from(p.launch_time)).rem_euclid(PULSE_MS);
        let prev = s.pulse.insert(p.id.0, t);
        let wrapped = prev.is_none_or(|prev| t < prev);
        if wrapped {
            runner(world, p, teambased, &teams);
        }
        if t >= LASER_MS {
            continue;
        }
        let targets: Vec<&(u32, PlayerState)> = alive
            .iter()
            .filter(|(c, ps)| {
                !friend(teambased, &teams, owner, *c) && dist(ps.origin, p.origin) <= RANGE
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        let slot = (LASER_MS / targets.len() as i64).max(1);
        let idx = usize::try_from(t / slot)
            .unwrap_or(0)
            .min(targets.len() - 1);
        let (target, ps) = *targets[idx];
        let to = aim(&ps);
        if !clear(world, p.origin, to) {
            continue;
        }
        let moved_on = prev.is_none_or(|prev| usize::try_from(prev / slot).unwrap_or(0) < idx);
        if wrapped || moved_on {
            natives_fx::sound(world, EventAudience::All, ALERT, p.origin);
            let d = [
                to[0] - p.origin[0],
                to[1] - p.origin[1],
                to[2] - p.origin[2],
            ];
            let len = dist(to, p.origin).max(f32::EPSILON);
            natives_fx::effect(
                world,
                TARGETING,
                p.origin,
                [d[0] / len, d[1] / len, d[2] / len],
            );
            if !has_perk(world, target, NO_SENSOR) {
                *s.pings.entry(owner).or_default().entry(target).or_default() += 1;
            }
        }
        let tick = super::tick(world);
        frame(world).push_entity_event(
            tick,
            EventAudience::All,
            entity_t6::EntityEventKind::SENSOR_LASER,
            crate::EntityEventPayload {
                number: p.entnum,
                event_parm: i32::try_from(target).unwrap_or(0),
                correlation: p.id.0,
                origin: p.origin,
                origin2: to,
                ..Default::default()
            },
        );
    }
    let players = &world.resource::<Zm>().players;
    for marks in s.tracked.values_mut() {
        marks.retain(|c, until| *until > now && players.contains_key(c));
    }
    s.tracked
        .retain(|o, marks| !marks.is_empty() && players.contains_key(o));
    for marks in s.pings.values_mut() {
        marks.retain(|c, _| players.contains_key(c));
    }
    s.pings
        .retain(|o, marks| !marks.is_empty() && players.contains_key(o));
    publish(world, &mut s);
    world.insert_resource(s);
}

/// Each owner's minimap: "<tracked enemies> <enemy:pings>", each list comma
/// separated or "-", sent when it changes.
fn publish(world: &mut World, s: &mut SensorGrenades) {
    let mut owners: Vec<u32> = s.tracked.keys().chain(s.pings.keys()).copied().collect();
    owners.extend(s.sent.keys().copied());
    owners.sort_unstable();
    owners.dedup();
    for owner in owners {
        if is_bot(world, owner) {
            continue;
        }
        let list = |items: Vec<String>| {
            if items.is_empty() {
                "-".to_string()
            } else {
                items.join(",")
            }
        };
        let tracked = list(
            s.tracked
                .get(&owner)
                .map(|m| m.keys().map(u32::to_string).collect())
                .unwrap_or_default(),
        );
        let pings = list(
            s.pings
                .get(&owner)
                .map(|m| m.iter().map(|(c, n)| format!("{c}:{n}")).collect())
                .unwrap_or_default(),
        );
        let value = format!("{tracked} {pings}");
        if s.sent.get(&owner) == Some(&value) {
            continue;
        }
        set_client_dvar(world, owner, DVAR, &value);
        if value == "- -" {
            s.sent.remove(&owner);
        } else {
            s.sent.insert(owner, value);
        }
    }
}

/// The grenade code: sensor `p` went off at `origin`. Each of its owner's
/// enemies within its explosion radius it can see is tracked.
pub(crate) fn detonated(world: &mut World, p: &ProjectileState, origin: [f32; 3]) {
    if !world.contains_resource::<super::T6Runtime>() || !world.resource::<Zm>().mp {
        return;
    }
    let Some(facts) = frame(world).equipment_facts_for(p.weapon) else {
        return;
    };
    let radius = facts.explosion_radius as f32;
    let owner = p.owner.0;
    let now = world.resource::<Zm>().now_ms;
    let (teambased, teams) = sides(world);
    let mut marks = Vec::new();
    for (target, ps) in alive(world) {
        if friend(teambased, &teams, owner, target)
            || dist(ps.origin, origin) > radius
            || has_perk(world, target, NO_SENSOR)
        {
            continue;
        }
        if clear(world, origin, aim(&ps)) {
            marks.push(target);
        }
    }
    if marks.is_empty() {
        return;
    }
    diag::info!(Sim, "bo2mp sensor grenade: {owner} tracks {marks:?}");
    let mut s = world.get_resource_or_insert_with(SensorGrenades::default);
    let tracked = s.tracked.entry(owner).or_default();
    for target in marks {
        tracked.insert(target, now + TRACK_MS);
    }
}
