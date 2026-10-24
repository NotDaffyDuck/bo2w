//! Origins' G-Strike (beacon_zm): the client half of
//! clientscripts/mp/zombies/_zm_weap_beacon.csc. The server script
//! (_zm_weap_beacon.gsc) does the damage; these are what each client sees:
//! the beacon's glow and beep, the launch flash on the robot's rocket pod,
//! and each shell falling from the sky onto its landing spot in 3 s.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Value, Vm};

use super::{Zm, entnum, frame};
use crate::EventAudience;

const GLOW: &str = "maps/zombie_tomb/fx_tomb_beacon_glow";
const LAUNCH: &str = "maps/zombie_tomb/fx_tomb_beacon_launch";
const EXPLOSION: &str = "maps/zombie_tomb/fx_tomb_beacon_exp";
const TRAIL: &str = "maps/zombie_tomb/fx_tomb_beacon_trail";
/// shell_logic: `self moveto(v_land, 3)`.
const FALL_MS: i64 = 3000;

#[derive(Resource, Default)]
struct Beacons {
    /// Each beacon model's shells fired (`num_rockets_fired`).
    fired: std::collections::HashMap<u32, usize>,
    /// Beacons glowing: model, next beep.
    glowing: Vec<(u32, i64)>,
    /// Shells falling: shell entity, beacon model, its index, lands at.
    shells: Vec<(u32, u32, usize, i64)>,
    /// sndfakelaunchaudio: the five fire sounds still to play.
    launch_sounds: Vec<([f32; 3], i64)>,
}

/// build_weap_beacon_landing_offsets(_ee): the first on the beacon, the
/// rest 72 out on the diagonals, round and round.
fn land_offset(i: usize) -> [f32; 3] {
    match i {
        0 => [0.0, 0.0, 0.0],
        _ => [[-72.0, 72.0, 0.0], [72.0, 72.0, 0.0], [72.0, -72.0, 0.0], [-72.0, -72.0, 0.0]][(i - 1) % 4],
    }
}

/// build_weap_beacon_start_offsets(_ee): 8500 up, the rest 6500 out too.
fn start_offset(i: usize) -> [f32; 3] {
    match i {
        0 => [0.0, 0.0, 8500.0],
        _ => [
            [-6500.0, 6500.0, 8500.0],
            [6500.0, 6500.0, 8500.0],
            [6500.0, -6500.0, 8500.0],
            [-6500.0, -6500.0, 8500.0],
        ][(i - 1) % 4],
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// An entity field of the beacon's script: `play_beacon_fx` (its glow)
/// and `play_artillery_barrage` (1, 2 for the easter egg's 15: one shell).
pub(super) fn entity_field(vm: &mut Vm<World>, world: &mut World, n: u32, name: &str, value: i64) {
    match name {
        "play_beacon_fx" if value != 0 => {
            let now = world.resource::<Zm>().now_ms;
            let b = &mut world.get_resource_or_insert_with(Beacons::default).glowing;
            if !b.iter().any(|(m, _)| *m == n) {
                b.push((n, now));
            }
        }
        "play_artillery_barrage" if value != 0 => shell(vm, world, n),
        _ => {}
    }
}

/// play_launch_artillery_fx_robot_<id> = 1: the launch flash on that
/// robot's rocket pod, the alarm and five fire sounds 0.15 s apart.
pub(super) fn world_field(vm: &mut Vm<World>, world: &mut World, name: &str, value: i64) {
    let Some(id) = name.strip_prefix("play_launch_artillery_fx_robot_") else {
        return;
    };
    if value != 1 {
        return;
    }
    let Ok(id) = id.parse::<i32>() else { return };
    let lvl = vm.level;
    let k = vm.intern("a_giant_robots");
    let Value::Array(a) = vm.raw_field(lvl, k) else {
        return;
    };
    let Some(robot) = a.get(&gsc_t6::Key::Int(id)).and_then(|r| entnum(vm, &r)) else {
        return;
    };
    let pod = super::tags::tag_world(world, robot, "tag_rocketpod")
        .map(|(o, _)| o)
        .or_else(|| world.resource::<Zm>().ents.get(&robot).map(|e| e.origin));
    let Some(pod) = pod else { return };
    super::natives_fx::effect(world, LAUNCH, pod, [0.0, 0.0, 1.0]);
    super::natives_fx::sound(world, EventAudience::All, "zmb_homingbeacon_missiile_alarm", pod);
    let now = world.resource::<Zm>().now_ms;
    let b = &mut world.get_resource_or_insert_with(Beacons::default).launch_sounds;
    for i in 0..5 {
        b.push((pod, now + i * 150));
    }
    diag::info!(Sim, "bo2zm t6 beacon: robot {id} launches at {pod:?}");
}

/// play_artillery_barrage: the next shell, from high above the beacon
/// down onto its landing spot (traced down from 3500 above, as the csc).
fn shell(vm: &mut Vm<World>, world: &mut World, model: u32) {
    let Some(at) = world.resource::<Zm>().ents.get(&model).map(|e| e.origin) else {
        return;
    };
    let i = {
        let b = world.get_resource_or_insert_with(Beacons::default);
        let f = b.into_inner().fired.entry(model).or_insert(0);
        *f += 1;
        *f - 1
    };
    let start = add(at, start_offset(i));
    let aim = add(at, land_offset(i));
    let from = [start[0], start[1], start[2] - 5000.0];
    let t = frame(world).trace_static_world(from, aim, [0.0; 3], [0.0; 3], crate::bullet_collision::MASK_SHOT);
    let f = t.fraction.clamp(0.0, 1.0);
    let land: [f32; 3] = std::array::from_fn(|k| from[k] + (aim[k] - from[k]) * f);
    let n = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(n));
    let now = {
        let mut zm = world.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                classname: "script_model".into(),
                origin: start,
                angles: [-90.0, 0.0, 0.0],
                model: "jun_missile".into(),
                ..Default::default()
            },
        );
        let now = zm.now_ms;
        zm.movers.move_to(n, start, land, now, FALL_MS as f32 / 1000.0, 0.0, 0.0);
        now
    };
    super::clientfields::bolt_named_queued(world, n, TRAIL, "tag_origin");
    super::natives_fx::sound(world, EventAudience::All, "zmb_homingbeacon_missile_boom", start);
    world
        .get_resource_or_insert_with(Beacons::default)
        .shells
        .push((n, model, i, now + FALL_MS));
    diag::info!(Sim, "bo2zm t6 beacon: shell {i} from {start:?} lands {land:?}");
}

/// Each tick: the beeps, the launch sounds, the incoming whistle 2 s into
/// each fall, and each shell's blast where it lands.
pub(super) fn advance(world: &mut World, now: i64) {
    let Some(mut b) = world.get_resource_mut::<Beacons>().map(|mut b| std::mem::take(&mut *b)) else {
        return;
    };
    b.launch_sounds.retain(|&(at, due)| {
        if due > now {
            return true;
        }
        super::natives_fx::sound(world, EventAudience::All, "zmb_homingbeacon_missile_fire", at);
        false
    });
    // play_beacon_glow: a beep and a glow every 1.5 s while it stands.
    b.glowing.retain_mut(|(m, next)| {
        if !world.resource::<Zm>().ents.contains_key(m) {
            return false;
        }
        if *next <= now {
            let at = world.resource::<Zm>().ents[m].origin;
            super::natives_fx::sound(world, EventAudience::All, "evt_beacon_beep", at);
            super::natives_fx::bolt_effect(world, *m, GLOW, "origin_animate_jnt", false);
            *next = now + 1500;
        }
        true
    });
    let mut stop_glow = Vec::new();
    b.shells.retain(|&(n, model, i, lands)| {
        let tick = i64::from(crate::MATCH_TICK_MS);
        if (lands - 1000..lands - 1000 + tick).contains(&now) {
            if let Some(at) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) {
                super::natives_fx::sound(world, EventAudience::All, "zmb_homingbeacon_missile_incoming", at);
            }
        }
        if now < lands {
            return true;
        }
        // shell_logic after movedone: the second shell's landing ends the
        // beacon's glow (weapon_beacon_destroyed).
        if i == 1 {
            stop_glow.push(model);
        }
        let gone = world.resource_mut::<Zm>().ents.remove(&n);
        world.resource_mut::<Zm>().movers.list.remove(&n);
        if let Some(e) = gone {
            super::natives_fx::effect(world, EXPLOSION, e.origin, [0.0, 0.0, 1.0]);
            super::natives_fx::sound(world, EventAudience::All, "zmb_homingbeacon_missile_impact", e.origin);
            if let Some(o) = e.obj {
                super::with_vm(world, |vm, world| vm.free_object(world, o));
            }
        }
        false
    });
    b.glowing.retain(|(m, _)| !stop_glow.contains(m));
    b.fired.retain(|m, _| world.resource::<Zm>().ents.contains_key(m));
    *world.resource_mut::<Beacons>() = b;
}
