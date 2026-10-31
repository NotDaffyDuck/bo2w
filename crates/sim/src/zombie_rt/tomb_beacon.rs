//! Origins' G-Strike (beacon_zm): the client half of
//! clientscripts/mp/zombies/_zm_weap_beacon.csc. The server script
//! (_zm_weap_beacon.gsc) does the damage; these are what each client sees:
//! the beacon's glow and beep, the launch flash on the robot's rocket pod,
//! and each shell falling from the sky onto its landing spot in 3 s.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, entnum, frame};
use crate::EventRecipients;

const GLOW: &str = "maps/zombie_tomb/fx_tomb_beacon_glow";
const LAUNCH: &str = "maps/zombie_tomb/fx_tomb_beacon_launch";
const EXPLOSION: &str = "maps/zombie_tomb/fx_tomb_beacon_exp";
const STREAK_BODY: &str = "maps/zombie_tomb/fx_tomb_beacon_trail";
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

/// `at` moved by `d`.
fn offset_by(at: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| at[k] + d[k])
}

/// An entity field of the beacon's script: `play_beacon_fx` (its glow)
/// and `play_artillery_barrage` (1, 2 for the easter egg's 15: one shell).
pub(super) fn entity_field(vm: &mut Vm<World>, game: &mut World, nn: u32, ident: &str, val: i64) {
    match ident {
        "play_beacon_fx" if val != 0 => {
            let now = game.resource::<Zm>().clock_millis;
            let b = &mut game.get_resource_or_insert_with(Beacons::default).glowing;
            if !b.iter().any(|(m, _)| *m == nn) {
                b.push((nn, now));
            }
        }
        "play_artillery_barrage" if val != 0 => shell(vm, game, nn),
        _ => {}
    }
}

/// play_launch_artillery_fx_robot_<id> = 1: the launch flash on that
/// robot's rocket pod, the alarm and five fire sounds 0.15 s apart.
pub(super) fn world_field(vm: &mut Vm<World>, game: &mut World, ident: &str, val: i64) {
    let Some(id) = ident.strip_prefix("play_launch_artillery_fx_robot_") else {
        return;
    };
    if val != 1 {
        return;
    }
    let Ok(id) = id.parse::<i32>() else { return };
    let lvl = vm.level;
    let k = vm.intern("a_giant_robots");
    let ScriptValue::Array(a) = vm.raw_field(lvl, k) else {
        return;
    };
    let Some(robot) = a.get(&gsc_t6::Key::Int(id)).and_then(|r| entnum(vm, &r)) else {
        return;
    };
    let pod = super::tags::tag_world(game, robot, "tag_rocketpod")
        .map(|(o, _)| o)
        .or_else(|| game.resource::<Zm>().ents.get(&robot).map(|e| e.origin));
    let Some(pod) = pod else { return };
    super::natives_fx::effect(game, LAUNCH, pod, [0.0, 0.0, 1.0]);
    super::natives_fx::sound(game, EventRecipients::All, "zmb_homingbeacon_missiile_alarm", pod);
    let now = game.resource::<Zm>().clock_millis;
    let b = &mut game.get_resource_or_insert_with(Beacons::default).launch_sounds;
    // Five launch sounds, 150 ms apart.
    b.extend((0..5).map(|k| (pod, now + k * 150)));
    diag::info!(Sim, "bo2zm t6 beacon: robot {id} launches at {pod:?}");
}

/// play_artillery_barrage: the next shell, from high above the beacon
/// down onto its landing spot (traced down from 3500 above, as the csc).
fn shell(vm: &mut Vm<World>, game: &mut World, mdl: u32) {
    let Some(at) = game.resource::<Zm>().ents.get(&mdl).map(|e| e.origin) else {
        return;
    };
    let i = {
        let b = game.get_resource_or_insert_with(Beacons::default);
        let f = b.into_inner().fired.entry(mdl).or_insert(0);
        *f += 1;
        *f - 1
    };
    let start = offset_by(at, start_offset(i));
    let aim = offset_by(at, land_offset(i));
    let from = [start[0], start[1], start[2] - 5000.0];
    let t = frame(game).trace_static_world(from, aim, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
    let f = t.fraction.clamp(0.0, 1.0);
    let land: [f32; 3] = std::array::from_fn(|k| from[k] + (aim[k] - from[k]) * f);
    let n = game.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(n));
    let now = {
        let mut zm = game.resource_mut::<Zm>();
        zm.ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                class_tag: "script_model".into(),
                origin: start,
                rot: [-90.0, 0.0, 0.0],
                mesh: "jun_missile".into(),
                ..Default::default()
            },
        );
        let now = zm.clock_millis;
        zm.movers.move_to(n, start, land, now, FALL_MS as f32 / 1000.0, 0.0, 0.0);
        now
    };
    super::clientfields::bolt_named_queued(game, n, STREAK_BODY, "tag_origin");
    super::natives_fx::sound(game, EventRecipients::All, "zmb_homingbeacon_missile_boom", start);
    game
        .get_resource_or_insert_with(Beacons::default)
        .shells
        .push((n, mdl, i, now + FALL_MS));
    diag::info!(Sim, "bo2zm t6 beacon: shell {i} from {start:?} lands {land:?}");
}

/// Each tick: the beeps, the launch sounds, the incoming whistle 2 s into
/// each fall, and each shell's blast where it lands.
pub(super) fn advance(game: &mut World, now: i64) {
    let Some(mut b) = game.get_resource_mut::<Beacons>().map(|mut b| std::mem::take(&mut *b)) else {
        return;
    };
    b.launch_sounds.retain(|&(at, due)| {
        if due > now {
            return true;
        }
        super::natives_fx::sound(game, EventRecipients::All, "zmb_homingbeacon_missile_fire", at);
        false
    });
    // play_beacon_glow: a beep and a glow every 1.5 s while it stands.
    b.glowing.retain_mut(|(m, next)| {
        if !game.resource::<Zm>().ents.contains_key(m) {
            return false;
        }
        if *next <= now {
            let at = game.resource::<Zm>().ents[m].origin;
            super::natives_fx::sound(game, EventRecipients::All, "evt_beacon_beep", at);
            super::natives_fx::bolt_effect(game, *m, GLOW, "origin_animate_jnt", false);
            *next = now + 1500;
        }
        true
    });
    let mut stop_glow = Vec::new();
    b.shells.retain(|&(n, model, i, lands)| {
        let tick = i64::from(crate::SIM_STEP_MILLIS);
        if (lands - 1000..lands - 1000 + tick).contains(&now) {
            if let Some(at) = game.resource::<Zm>().ents.get(&n).map(|en| en.origin) {
                super::natives_fx::sound(game, EventRecipients::All, "zmb_homingbeacon_missile_incoming", at);
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
        let gone = game.resource_mut::<Zm>().ents.remove(&n);
        game.resource_mut::<Zm>().movers.list.remove(&n);
        if let Some(en) = gone {
            super::natives_fx::effect(game, EXPLOSION, en.origin, [0.0, 0.0, 1.0]);
            super::natives_fx::sound(game, EventRecipients::All, "zmb_homingbeacon_missile_impact", en.origin);
            if let Some(o) = en.obj {
                super::with_vm(game, |vm, game| vm.free_object(game, o));
            }
        }
        false
    });
    b.glowing.retain(|(m, _)| !stop_glow.contains(m));
    b.fired.retain(|m, _| game.resource::<Zm>().ents.contains_key(m));
    *game.resource_mut::<Beacons>() = b;
}
