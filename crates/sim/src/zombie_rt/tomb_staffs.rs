//! Origins (zm_tomb) staff puzzle test aid: the upgrade puzzles done with
//! the staff's own shots.
//!
//! BO2MP_T6_STAFFPUZ=<step>[,<step>...][@<seconds>] (from 30 s unless
//! given). BO2MP_T6_STAFFPUZ_GIVE=1 gives him the step's staff
//! (`weapon_give`) when he has none, so a puzzle can be tried without
//! crafting it first. One action every 1.5 s (clip refilled), each logged;
//! a step ends when its flag is set, then the next one starts.
//! - air1: the four rings on the wind chamber's ceiling
//!   (`ceiling_ring_01..04`, one inside the other) turn a quarter for each
//!   wind staff hit until each is at its `script_int`. He stands under the
//!   rings and shoots the band of the first one still wrong
//!   (`air_puzzle_1_complete`).
//! - air2: each smoke (`puzzle_smoke_origin`) turns to the way a wind
//!   staff shot through its ghosted detector brush went; all must point at
//!   `puzzle_smoke_dest`. He stands on the far side of a smoke that is not
//!   solved and shoots through it toward the write_target (`air_puzzle_2_complete`).
//! - ice1: the ice chamber's ceiling tiles (`ice_ceiling_tile`): he shoots
//!   the tile whose `value` is the gem's (`ice_chamber_gem`, shown by the
//!   digits) while it still shows its tile side (`ice_puzzle_1_complete`).
//! - ice2: the rocks (`puzzle_stone_water`): a water staff hit freezes one
//!   and a bullet breaks it, so he shoots the first rock left with the staff
//!   and his pistol in turn (`ice_puzzle_2_complete`).
//! - fire1: the fire pits in the chamber (`fire_sacrifice_volume`): a
//!   zombie killed on a pit leaves a body there that burns when a fire
//!   staff shot lands in the pit (lit 6 s); 32 bodies please each pit. He
//!   stands on the first pit left and shoots the feet of the zombie on it
//!   nearest its fire, or the pit floor when none is on it
//!   (`fire_puzzle_1_complete`). With STAFFPUZ_GIVE the fire staff is also
//!   marked crafted, which the pits wait for.
//! - fire2: the church torches (`church_torch_target`): he lights each one
//!   whose number shows (`b_correct_torch`) and is not burning
//!   (`fire_puzzle_2_complete`).
//! - elec1: the piano keys in the chamber (`piano_key`): the three notes of
//!   a key_combo, a note not ringing yet, from the chamber side. The script asks
//!   for its key_combos in its own order (ours walks arrays newest first, so d
//!   minor comes first): three ringing notes it does not take in 3 s mean
//!   it wants another key_combo, and he plays the next one not heard yet
//!   (`electric_puzzle_1_complete`).
//! - elec2: the relay switches (`puzzle_relay_switch`): he turns each one
//!   (use) until its `position` is the one its connection is on, which runs
//!   the power from the lightning relay to the chamber
//!   (`electric_puzzle_2_complete`).
//! - air3, ice3, fire3, elec3: the crypt. Once puzzle 2 is done the orb
//!   flies from its chamber to its gem's place on the crypt's middle disc
//!   (he waits in the crypt, so no one is in the chamber to hold it). Shot
//!   with its staff, the gem goes up once every lever disc is turned to the
//!   gem's place (`(main disc + gem_start_pos[gem]) % 4`); he pulls the
//!   lever that gets a wrong disc there in fewest turns, then shoots the
//!   gem (`staff_<x>_zm_upgrade_unlocked`).
//! - air4, ice4, fire4, elec4: the charger. He puts the staff in its
//!   charger (use), kills 20 zombies with his pistol near it to charge it,
//!   then picks up the upgraded staff from it (use; done when it has an
//!   owner).

use super::pairs::KeepByValue;
use std::sync::{Mutex, OnceLock};

use bevy_ecs::prelude::World;
use gsc_t6::{Key, ObjRef, ScriptValue, Vm};
use playerstate_t6::game::buttons;

use super::tomb_test::level_flag;
use super::{Closest, frame, with_vm};
use crate::sim_parts::ClientId;

const SHOT: i64 = 1_500;
/// His start pistol (BO2's `level.start_weapon` on Origins).
const PISTOL: &str = "c96_zm";
const CHORDS: [&str; 3] = ["a_minor", "e_minor", "d_minor"];

/// The first key_combo after `from` (round the three) not heard yet; 3 when
/// all three have been.
fn next_chord(solved: u8, from_ix: usize) -> usize {
    (1..=3).map(|d| (from_ix + d) % 3).find(|c| solved & (1 << c) == 0).unwrap_or(3)
}

fn want() -> Option<&'static (Vec<String>, i64)> {
    static WANT: OnceLock<Option<(Vec<String>, i64)>> = OnceLock::new();
    WANT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_STAFFPUZ").ok()?;
        let (steps, at) = match v.split_once('@') {
            Some((n, s)) => (n.to_owned(), s.parse::<f32>().map_or(30_000, |s| (s * 1000.0) as i64)),
            None => (v, 30_000),
        };
        let list: Vec<String> = steps.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        (!list.is_empty()).then_some((list, at))
    })
    .as_ref()
}

struct State {
    stage_no: usize, given: bool,
    prior_shot: i64,
    shots: u32,
    /// elec1: the key_combo being played, whether it was heard ringing (it
    /// moves on when the ringing stops), the key_combos heard (bits), and since
    /// when all three of its notes have rung without the key_combo taking.
    key_combo: usize,
    ringing: bool,
    solved: u8,
    full_since: Option<i64>,
    /// ice2: the next shot is the pistol's.
    pistol: bool,
    /// The crypt: where each gem waits (`crypt_gem_<x>`: name, origin,
    /// script_int), seen before its orb arrives and the placeholder goes.
    homes: Option<Vec<(String, [f32; 3], i32)>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    stage_no: 0,
    given: false,
    prior_shot: 0,
    shots: 0,
    key_combo: 0,
    ringing: false,
    solved: 0,
    full_since: None,
    pistol: false,
    homes: None,
});

/// Time between the charger step's actions (it kills zombies with the
/// pistol, which wants more than one shot each).
const CHARGE_SHOT: i64 = 400;

fn staff(step: &str) -> &'static str {
    match step.trim_end_matches(char::is_numeric) {
        "fire" => "staff_fire_zm",
        "ice" => "staff_water_zm",
        "elec" => "staff_lightning_zm",
        _ => "staff_air_zm",
    }
}

fn flag(step: &str) -> String {
    let elem = step.trim_end_matches(char::is_numeric);
    let n = &step[elem.len()..];
    match n {
        "3" => format!("{}_upgrade_unlocked", staff(step)),
        "4" => "the upgraded staff picked up".to_owned(),
        _ => {
            let elem = if elem == "elec" { "electric" } else { elem };
            format!("{elem}_puzzle_{n}_complete")
        }
    }
}

/// The step's end: its flag set, or for the charger the upgraded staff
/// picked up (`upgrade.owner`).
fn step_done(world: &mut World, step: &str) -> bool {
    with_vm(world, |vm, _| {
        if step.ends_with('4') {
            let Some(s) = staff_obj(vm, staff(step)) else {
                return false;
            };
            let up = vm.intern("upgrade");
            let owner = vm.intern("owner");
            match vm.raw_field(s, up) {
                ScriptValue::Object(u) => !matches!(vm.raw_field(u, owner), ScriptValue::Undefined),
                _ => false,
            }
        } else {
            level_flag(vm, &flag(step))
        }
    })
    .unwrap_or_default()
}

/// The staff's entry in `level.a_elemental_staffs` (by `weapname`).
fn staff_obj(vm: &mut Vm<World>, weap: &str) -> Option<ObjRef> {
    let f = vm.intern("a_elemental_staffs");
    let ScriptValue::Array(a) = vm.raw_field(vm.level, f) else {
        return None;
    };
    let list: Vec<ObjRef> = a.snapshot().values_in_order().filter_map(ScriptValue::as_obj).collect();
    list.into_iter().find(|s| text(vm, *s, "weapname") == weap)
}

/// One aimed shot this tick: look from his eye at `at`, refill the clip,
/// press fire.
fn shoot(game: &mut World, peer: ClientId, at: [f32; 3]) {
    let Some(eye) = frame(game)
        .avatar(peer)
        .map(|p| [p.origin[0], p.origin[1], p.origin[2] + p.eye_height_now])
    else {
        return;
    };
    super::set_player_view(game, peer, super::autoplay::look(eye, at));
    {
        let mut f = frame(game);
        if let Some(w) = f.avatar(peer).map(|p| p.weapon) {
            crate::client_script::set_ammo_clip(&mut f, peer, w, 8);
        }
    }
    let mut req = game.resource_mut::<crate::frame_step::FrameTicket>();
    for (id, cmd) in &mut req.input.cmds {
        if peer == *id {
            cmd.buttons = cmd.buttons | buttons::ATTACK;
        }
    }
}

/// The floor under `c` (the first floor his box lands on going down).
pub(super) fn floor_under(game: &mut World, ch: [f32; 3]) -> Option<[f32; 3]> {
    let down = [ch[0], ch[1], ch[2] - 2000.0];
    let t = frame(game).trace_static_world(
        ch,
        down,
        crate::shot_trace::BODY_BOX_LOWER,
        crate::shot_trace::BODY_BOX_UPPER,
        crate::shot_trace::PLAYER_BODY_MASK,
    );
    (!t.starts_inside && t.fraction < 1.0)
        .then(|| std::array::from_fn(|i| ch[i] + (down[i] - ch[i]) * t.fraction))
}

/// The floor under something up on a ceiling (the wind rings, the ice
/// tiles): the first start under it that is in open air (60 under can still
/// be inside the ceiling).
fn floor_below(game: &mut World, at: [f32; 3]) -> Option<[f32; 3]> {
    [60.0, 120.0, 200.0, 300.0]
        .into_iter()
        .find_map(|dz| floor_under(game, [at[0], at[1], at[2] - dz]))
}

/// A floor spot about `dist` out from `at` with open air between (the first
/// of eight ways that is clear), to shoot at `at` from.
fn stand_off(game: &mut World, at: [f32; 3], dist: f32) -> Option<[f32; 3]> {
    (0..8).find_map(|k| {
        let yaw = (45.0 * k as f32).to_radians();
        let goal = [at[0] + dist * yaw.cos(), at[1] + dist * yaw.sin(), at[2]];
        let tt = frame(game).trace_static_world(at, goal, [0.0; 3], [0.0; 3], crate::shot_trace::PLAYER_BODY_MASK);
        if tt.starts_inside || tt.fraction < 0.95 {
            return None;
        }
        floor_under(game, goal).filter(|f| at[2] - f[2] < 400.0)
    })
}

/// A floor spot out from `at` (toward `toward` first, then the eight ways
/// round, 120-300 out) from whose eye a shot at `at` goes off on something
/// within 16 of it (the piano keys count a lightning ball that bursts within
/// 20). Falls back to a spot whose shot only gets clear to `at`. The spot,
/// and how far from `at` its shot goes off.
fn clear_shot(world: &mut World, at: [f32; 3], toward: Option<[f32; 3]>) -> Option<([f32; 3], f32)> {
    let yaw0 = toward.map_or(0.0, |c| (c[1] - at[1]).atan2(c[0] - at[0]).to_degrees());
    let mut fallback = None;
    for dist in [160.0, 120.0, 220.0, 300.0] {
        for k in 0..8 {
            let yaw = (yaw0 + 45.0 * k as f32).to_radians();
            let Some(f) = floor_under(world, [at[0] + dist * yaw.cos(), at[1] + dist * yaw.sin(), at[2]]) else {
                continue;
            };
            if at[2] - f[2] > 400.0 {
                continue;
            }
            // Nearer the middle than the key is: in the chamber, which the
            // puzzle wants him in (is_chamber_occupied).
            if toward.is_some_and(|c| flat(f, c) >= flat(at, c)) {
                continue;
            }
            let eye = [f[0], f[1], f[2] + 60.0];
            let d = gsc_t6::math::sub(at, eye);
            let len = gsc_t6::math::length(d);
            let way = gsc_t6::math::normalize(d);
            let end: [f32; 3] = std::array::from_fn(|i| at[i] + way[i] * 40.0);
            let t = frame(world).trace_static_world(eye, end, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
            if t.starts_inside {
                continue;
            }
            let hit: [f32; 3] = std::array::from_fn(|i| eye[i] + (end[i] - eye[i]) * t.fraction);
            let off = gsc_t6::math::length(gsc_t6::math::sub(hit, at));
            if off < 16.0 {
                return Some((f, off));
            }
            if fallback.is_none() && t.fraction * (len + 40.0) >= len - 4.0 {
                fallback = Some((f, off));
            }
        }
    }
    fallback
}

/// Where to shoot the crypt's gem from: a spot whose shot goes off on it,
/// else one out from it with open air between.
fn gem_spot(game: &mut World, at: [f32; 3]) -> Option<[f32; 3]> {
    clear_shot(game, at, None)
        .map(|s| s.0)
        .or_else(|| stand_off(game, at, 160.0))
}

/// A floor spot back along `way` from the smoke (160-840 out) whose eye has
/// a clear shot at it, the one whose shot runs closest to `way`: the spot
/// and that shot's dot with `way`.
fn smoke_spot(world: &mut World, smoke: [f32; 3], way: [f32; 3]) -> Option<([f32; 3], f32)> {
    let flat = gsc_t6::math::normalize([way[0], way[1], 0.0]);
    let mut pick = Closest::new();
    for dist in [160.0, 240.0, 320.0, 420.0, 540.0, 680.0, 840.0] {
        for up in [40.0, 160.0] {
            let back = [smoke[0] - flat[0] * dist, smoke[1] - flat[1] * dist, smoke[2] + up];
            let Some(f) = floor_under(world, back) else {
                continue;
            };
            let eye = [f[0], f[1], f[2] + 60.0];
            let d = gsc_t6::math::normalize(gsc_t6::math::sub(smoke, eye));
            // Clear to within 24 of it (spz8: the shot went off 11 short,
            // the smoke sits against rock).
            let t = frame(world).trace_static_world(eye, smoke, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
            let hit: [f32; 3] = std::array::from_fn(|i| eye[i] + (smoke[i] - eye[i]) * t.fraction);
            if t.starts_inside || gsc_t6::math::length(gsc_t6::math::sub(hit, smoke)) > 24.0 {
                continue;
            }
            let dot = gsc_t6::math::dot(d, way);
            pick.offer(-dot, f);
        }
    }
    pick.into_scored().map(|(less, f)| (f, -less))
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn structs(vm: &mut Vm<World>, key: &str, value: &str) -> Vec<ObjRef> {
    let f = vm.intern("struct");
    let ScriptValue::Array(arr) = vm.raw_field(vm.level, f) else {
        return Vec::new();
    };
    let k = vm.intern(key);
    let list: Vec<ObjRef> = arr.snapshot().values_in_order().filter_map(ScriptValue::as_obj).collect();
    list.into_iter()
        .filter(|s| vm.to_text(&vm.raw_field(*s, k)) == value)
        .collect()
}

/// Every entity whose `key` is `value`: (object, origin).
fn ents(world: &mut World, key: &str, value: &str) -> Vec<(ObjRef, [f32; 3])> {
    with_vm(world, |vm, world| ents_in(vm, world, key, value)).unwrap_or_default()
}

/// `ents`, inside a `with_vm`.
fn ents_in(vm: &mut Vm<World>, world: &World, key: &str, val: &str) -> Vec<(ObjRef, [f32; 3])> {
    let k = vm.intern(key);
    let zm = world.resource::<super::Zm>();
    zm.ents
        .values()
        .filter_map(|en| {
            let o = en.obj?;
            (vm.to_text(&vm.raw_field(o, k)) == val).then_some((o, en.origin))
        })
        .collect()
}

fn int(vm: &mut Vm<World>, o: ObjRef, field: &str) -> Option<i32> {
    let f = vm.intern(field);
    match vm.raw_field(o, f) {
        ScriptValue::Int(i) => Some(i),
        ScriptValue::Float(x) => Some(x as i32),
        _ => None,
    }
}

fn vec3(vm: &mut Vm<World>, o: ObjRef, fld: &str) -> Option<[f32; 3]> {
    let f = vm.intern(fld);
    vm.raw_field(o, f).as_vec3()
}

fn text(vm: &mut Vm<World>, o: ObjRef, field: &str) -> String {
    let f = vm.intern(field);
    vm.to_text(&vm.raw_field(o, f))
}

/// Where `o.field` (an entity) stands, when it is there and not deleted.
fn ent_field(vm: &mut Vm<World>, game: &mut World, o: ObjRef, fld: &str) -> Option<[f32; 3]> {
    let f = vm.intern(fld);
    let v = vm.raw_field(o, f);
    super::origin_of(vm, game, &v)
}

/// The ring still off its mark: (name, the middle of its band on the
/// ceiling, the middle of the rings), and every ring's "name pos/want" for
/// the log. The rings nest (each model's box holds the smaller ones), so
/// ring k's own band lies between ring k-1's edge and its own.
fn air1_target(game: &mut World) -> (Option<(String, [f32; 3], [f32; 3])>, String) {
    let mut next = None;
    let mut log = Vec::new();
    let mut inner = 0.0f32;
    for i in 1..=4 {
        let name = format!("ceiling_ring_0{i}");
        let Some(o) = super::autoplay::ent_by(game, "targetname", &name).map(|f| f.obj) else {
            log.push(format!("{name} missing"));
            continue;
        };
        let ent = game.resource::<super::Zm>().ents.values().find(|en| en.obj == Some(o)).cloned();
        let Some((low, hi)) = ent.and_then(|en| super::brushes::model_box(game, &en)) else {
            log.push(format!("{name} no box"));
            continue;
        };
        let centre: [f32; 3] = std::array::from_fn(|k| (low[k] + hi[k]) * 0.5);
        let half = (hi[0] - low[0]) * 0.5;
        let band = (inner + half) * 0.5;
        inner = half;
        let (pos, goal) = with_vm(game, |vm, _| (int(vm, o, "position"), int(vm, o, "script_int")))
            .unwrap_or((None, None));
        log.push(format!(
            "{name} {}/{}",
            pos.map_or("-".into(), |p| p.to_string()),
            goal.map_or("-".into(), |p| p.to_string())
        ));
        if next.is_none() && pos.is_some() && pos != goal {
            next = Some((name, [centre[0] + band, centre[1], centre[2]], centre));
        }
    }
    (next, log.join(", "))
}

/// The smoke still not pointing at the write_target: (its origin, the write_target's way
/// from it), and every smoke's "solved" for the log.
fn air2_target(world: &mut World) -> (Option<([f32; 3], [f32; 3])>, String) {
    with_vm(world, |vm, world| {
        let write_target = structs(vm, "targetname", "puzzle_smoke_dest")
            .first()
            .and_then(|d| vec3(vm, *d, "origin"));
        let mut next = None;
        let mut log = Vec::new();
        let solved_f = vm.intern("solved");
        for s in structs(vm, "targetname", "puzzle_smoke_origin") {
            let Some(o) = vec3(vm, s, "origin") else {
                continue;
            };
            let solved = gsc_t6::truthy(&vm.get_field(world, s, solved_f));
            let tol = int(vm, s, "script_int").unwrap_or(0);
            log.push(format!("({:.0} {:.0} {:.0}) tol {tol} solved {}", o[0], o[1], o[2], u8::from(solved)));
            if next.is_none()
                && !solved
                && let Some(d) = write_target
            {
                next = Some((o, gsc_t6::math::normalize(gsc_t6::math::sub(d, o))));
            }
        }
        (next, log.join(", "))
    })
    .unwrap_or((None, String::new()))
}

/// The tile to shoot (its origin) and the gem's value and tiles for the log.
fn ice1_target(world: &mut World) -> (Option<[f32; 3]>, String) {
    let tiles = ents(world, "script_noteworthy", "ice_ceiling_tile");
    let gem = super::autoplay::ent_by(world, "targetname", "ice_chamber_gem").map(|g| g.obj);
    with_vm(world, |vm, _| {
        let want = gem.and_then(|g| int(vm, g, "value"));
        let mut next = None;
        let mut showing = Vec::new();
        for (t, origin) in &tiles {
            if int(vm, *t, "showing_tile_side").unwrap_or(0) == 0 {
                continue;
            }
            let value = int(vm, *t, "value");
            showing.push(value.map_or("-".into(), |v| v.to_string()));
            if next.is_none() && value.is_some() && value == want {
                next = Some(*origin);
            }
        }
        (
            next,
            format!(
                "gem {}, {} tiles, showing [{}]",
                want.map_or("-".into(), |v| v.to_string()),
                tiles.len(),
                showing.join(" ")
            ),
        )
    })
    .unwrap_or((None, String::new()))
}

/// The first rock still there, and how many are left.
fn ice2_target(world: &mut World) -> (Option<[f32; 3]>, String) {
    with_vm(world, |vm, world| {
        let mut next = None;
        let mut left = 0;
        for s in structs(vm, "targetname", "puzzle_stone_water") {
            if let Some(o) = ent_field(vm, world, s, "e_model") {
                left += 1;
                next.get_or_insert(o);
            }
        }
        (next, format!("{left} rocks left"))
    })
    .unwrap_or((None, String::new()))
}

/// A fire pit to please: its fire's spot and the feet of the zombie on it.
struct FirePit {
    fire: [f32; 3],
    zombie: Option<[f32; 3]>,
}

/// The first fire pit (`level.sacrifice_volumes`) not yet pleased: its
/// fire's spot (`e_ignition_point`) and the feet of the living zombie on it
/// nearest that spot (touching the pit, as `istouching` tests it); and every
/// pit's count and the bodies waiting for the log.
fn fire1_target(game: &mut World) -> (Option<FirePit>, String) {
    let (pit, log) = with_vm(game, |vm, game| {
        let f = vm.intern("sacrifice_volumes");
        let ScriptValue::Array(arr) = vm.raw_field(vm.level, f) else {
            return logged(None, "no fire pits yet (sacrifice_volumes undefined)");
        };
        let pleased_f = vm.intern("b_gods_pleased");
        let point_f = vm.intern("e_ignition_point");
        let mut next = None;
        let mut counts = Vec::new();
        for v in arr.snapshot().values_in_order().filter_map(ScriptValue::as_obj) {
            let done = gsc_t6::truthy(&vm.raw_field(v, pleased_f));
            let nn = int(vm, v, "num_sacrifices_received").unwrap_or(0);
            counts.push(if done { "done".to_owned() } else { format!("{nn}/32") });
            if next.is_some() || done {
                continue;
            }
            let fire = match vm.raw_field(v, point_f) {
                ScriptValue::Object(p) => vec3(vm, p, "origin"),
                _ => None,
            };
            let num = game.resource::<super::Zm>().ents.iter().find(|(_, en)| en.obj == Some(v)).map(|(nn, _)| *nn);
            if let (Some(fire), Some(num)) = (fire, num) {
                next = Some((num, fire));
            }
        }
        let c = vm.intern("clone_list");
        let bodies = match vm.raw_field(vm.level, c) {
            ScriptValue::Array(lhs) => lhs.len(),
            _ => 0,
        };
        (next, format!("pits [{}], {bodies} bodies waiting", counts.join(" ")))
    })
    .unwrap_or((None, String::new()));
    let Some((num, fire)) = pit else {
        return (None, log);
    };
    let Some(vol) = game.resource::<super::Zm>().ents.get(&num).cloned() else {
        return (None, log);
    };
    let zombies: Vec<([f32; 3], f32, f32)> = {
        let zm = game.resource::<super::Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living && lhs.scripted.is_none())
            .filter_map(|(nn, lhs)| zm.ents.get(nn).map(|en| (en.origin, lhs.reach, lhs.height)))
            .collect()
    };
    let mut on_it: Option<(f32, [f32; 3])> = None;
    for (o, r, h) in zombies {
        let (mins, maxs) = ([o[0] - r, o[1] - r, o[2]], [o[0] + r, o[1] + r, o[2] + h]);
        if !super::triggers::touches(game, &vol, mins, maxs) {
            continue;
        }
        let d = (o[0] - fire[0]).hypot(o[1] - fire[1]);
        if on_it.is_none_or(|(best, _)| d < best) {
            on_it = Some((d, [o[0], o[1], o[2] + 10.0]));
        }
    }
    (Some(FirePit { fire, zombie: on_it.map(|(_, feet)| feet) }), log)
}

/// The first right torch not burning, and the torches for the log.
fn fire2_target(world: &mut World) -> (Option<[f32; 3]>, String) {
    with_vm(world, |vm, world| {
        let right_f = vm.intern("b_correct_torch");
        let mut next = None;
        let (mut right, mut burning, mut wrong) = (0, 0, 0);
        for s in structs(vm, "script_noteworthy", "church_torch_target") {
            let lit = ent_field(vm, world, s, "e_fx").is_some();
            let ok = gsc_t6::truthy(&vm.raw_field(s, right_f));
            right += u32::from(ok);
            burning += u32::from(lit && ok);
            wrong += u32::from(lit && !ok);
            if next.is_none() && ok && !lit {
                next = vec3(vm, s, "origin");
            }
        }
        (next, format!("{right} right torches, {burning} of them burning, {wrong} wrong ones burning"))
    })
    .unwrap_or((None, String::new()))
}

/// What the piano step sees of a chord: the next key to shoot (a note of it
/// not ringing), the chamber's middle, whether all its notes ring, and the log.
struct Chord {
    next: Option<[f32; 3]>,
    center: Option<[f32; 3]>,
    full: bool,
    log: String,
}

impl Chord {
    /// Nothing to shoot, with why.
    fn idle(log: &str) -> Self {
        Chord { next: None, center: None, full: false, log: log.to_owned() }
    }
}

/// A step and its log line.
fn logged<T>(step: T, log: &str) -> (T, String) {
    (step, log.to_owned())
}

/// The next key of this key_combo not ringing (its origin), and the chamber's
/// middle to stand toward.
/// The key_combo's next key to shoot (a note of it not ringing), the
/// chamber's middle, whether all its notes ring, and the log.
fn elec1_target(game: &mut World, key_combo: usize) -> Chord {
    let Some(name) = CHORDS.get(key_combo) else {
        return Chord::idle("all chords played");
    };
    with_vm(game, |vm, _| {
        let notes: Vec<String> = match structs(vm, "script_noteworthy", &format!("piano_chord_{name}")).first() {
            Some(c) => text(vm, *c, "script_string").split_whitespace().map(str::to_owned).collect(),
            _ => Vec::new(),
        };
        let pf = vm.intern("a_piano_keys_playing");
        let playing: Vec<String> = match vm.raw_field(vm.level, pf) {
            ScriptValue::Array(a) => a.snapshot().values_in_order().map(|v| vm.to_text(v)).collect(),
            _ => Vec::new(),
        };
        let center = structs(vm, "targetname", "chamber_center")
            .first()
            .and_then(|c| vec3(vm, *c, "origin"));
        let keys = structs(vm, "script_noteworthy", "piano_key");
        let mut next = None;
        for k in &keys {
            let note = text(vm, *k, "script_string");
            if notes.contains(&note) && !playing.contains(&note) {
                next = vec3(vm, *k, "origin");
                break;
            }
        }
        let full = !notes.is_empty() && notes.iter().all(|n| playing.contains(n));
        Chord {
            next,
            center,
            full,
            log: format!(
                "chord {name} [{}], ringing [{}], {} keys",
                notes.join(" "),
                playing.join(" "),
                keys.len()
            ),
        }
    })
    .unwrap_or_else(|| Chord::idle(""))
}

/// The first relay switch on the power's way that is turned wrong (its
/// origin), and every relay's "name position/goal" for the log.
fn elec2_target(world: &mut World) -> (Option<[f32; 3]>, String) {
    with_vm(world, |vm, world| {
        let rf = vm.intern("electric_relays");
        let ScriptValue::Array(relays) = vm.raw_field(vm.level, rf) else {
            return logged(None, "no relays yet");
        };
        let relays = relays.snapshot();
        let conn = vm.intern("connections");
        let mut next = None;
        let mut log = Vec::new();
        for key in relays.keys().collect::<Vec<_>>() {
            let Some(r) = relays.get(&key).and_then(ScriptValue::as_obj) else {
                continue;
            };
            let name = match key {
                Key::Str(s) => vm.to_text(&ScriptValue::Str(s)),
                Key::Int(i) => i.to_string(),
                Key::Obj(_) => "?".to_owned(),
            };
            let goal = match vm.raw_field(r, conn) {
                ScriptValue::Array(c) => c.snapshot().keys().find_map(|k| match k {
                    Key::Int(i) => Some(i),
                    _ => None,
                }),
                _ => None,
            };
            // A relay with no connection (the bunker's) is not on the way.
            let Some(goal) = goal else {
                continue;
            };
            let pos = int(vm, r, "position");
            log.push(format!("{name} {}/{goal}", pos.map_or("-".into(), |p| p.to_string())));
            if next.is_none() && pos != Some(goal) {
                next = ent_field(vm, world, r, "e_switch");
            }
        }
        (next, log.join(", "))
    })
    .unwrap_or((None, String::new()))
}

/// What the crypt step does next.
enum Crypt {
    /// Nothing this time (the orb on its way, the discs turning, the gem
    /// going up), standing here if given.
    Wait(Option<[f32; 3]>),
    /// Pull the lever here (a disc is off the gem's place).
    Lever([f32; 3]),
    /// Shoot the gem here with the staff.
    Gem([f32; 3]),
}

/// Where each crypt gem waits, read once while the placeholders stand.
fn gem_homes(world: &mut World) -> Vec<(String, [f32; 3], i32)> {
    with_vm(world, |vm, world| {
        ["air", "ice", "fire", "elec"]
            .iter()
            .filter_map(|e| {
                let label = format!("crypt_gem_{e}");
                let (o, at) = *ents_in(vm, world, "targetname", &label).first()?;
                Some((label, at, int(vm, o, "script_int").unwrap_or(-1)))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn crypt_target(world: &mut World, elem: &str, home: Option<([f32; 3], i32)>) -> (Crypt, String) {
    let gem_name = format!("crypt_gem_{elem}");
    with_vm(world, |vm, world| {
        // The placeholder still stands: the orb is not here yet.
        if let Some((g, at)) = ents_in(vm, world, "targetname", &gem_name).first().copied() {
            let flags: Vec<String> = text(vm, g, "script_flag").split_whitespace()
                .map(|f| format!("{f} {}", u8::from(level_flag(vm, f))))
                .collect();
            return (
                Crypt::Wait(Some(at)),
                format!("the orb is not in the crypt yet (it waits for [{}])", flags.join(", ")),
            );
        }
        let Some((home, gem_int)) = home else {
            return (Crypt::Wait(None), format!("{gem_name}'s place was never seen"));
        };
        // The gem: the model with its script_int at its place (not a disc).
        let sn = vm.intern("script_noteworthy");
        let si = vm.intern("script_int");
        let gem = {
            let zm = world.resource::<super::Zm>();
            zm.ents
                .values()
                .filter(|e| !e.mesh.is_empty() && (e.origin[0] - home[0]).hypot(e.origin[1] - home[1]) < 48.0)
                .filter_map(|e| {
                    let o = e.obj?;
                    let n = match vm.raw_field(o, si) {
                        ScriptValue::Int(i) => i,
                        _ => return None,
                    };
                    (n == gem_int && vm.to_text(&vm.raw_field(o, sn)) != "crypt_puzzle_disc").then_some(e.origin)
                })
                .next()
        };
        let Some(gem) = gem else {
            return (Crypt::Wait(None), "the gem has left for the chamber".to_owned());
        };
        if gem[2] > home[2] + 24.0 {
            return (Crypt::Wait(None), "the gem is going up".to_owned());
        }
        // The discs: each one with levers must stand at the gem's place.
        let discs = ents_in(vm, world, "script_noteworthy", "crypt_puzzle_disc");
        let main = discs
            .iter()
            .find(|(d, _)| text(vm, *d, "targetname") == "crypt_puzzle_disc_main")
            .and_then(|(d, _)| int(vm, *d, "position"))
            .unwrap_or(0);
        let start = {
            let f = vm.intern("gem_start_pos");
            match vm.raw_field(vm.level, f) {
                ScriptValue::Array(a) => match a.snapshot().get(&Key::Str(vm.intern(&gem_name))) {
                    Some(ScriptValue::Int(i)) => *i,
                    _ => 0,
                },
                _ => 0,
            }
        };
        let want = (main + start).rem_euclid(4);
        let turning = level_flag(vm, "disc_rotation_active");
        let mut log = Vec::new();
        let mut lever: Option<(i32, [f32; 3])> = None;
        let mut list: Vec<(i32, ObjRef)> = Vec::new();
        for (d, _) in &discs {
            if text(vm, *d, "targetname") != "crypt_puzzle_disc_main" {
                list.push((int(vm, *d, "script_int").unwrap_or(0), *d));
            }
        }
        list.sort_by_key(|(n, _)| *n);
        for (n, d) in list {
            let pos = int(vm, d, "position").unwrap_or(0);
            log.push(format!("disc {n} at {pos}"));
            if pos == want || lever.is_some() {
                continue;
            }
            let target = text(vm, d, "target");
            if target.is_empty() {
                log.push("(no lever)".to_owned());
                continue;
            }
            for (l, at) in ents_in(vm, world, "targetname", &target) {
                let cw = text(vm, l, "script_string") == "clockwise";
                let turns = if cw { (want - pos).rem_euclid(4) } else { (pos - want).rem_euclid(4) };
                if lever.is_none_or(|(best, _)| turns < best) {
                    lever = Some((turns, at));
                }
            }
        }
        let log = format!(
            "the gem's place {want} (main disc {main} + {start}), {}{}",
            log.join(", "),
            if turning { ", discs turning" } else { "" }
        );
        match lever {
            Some(_) if turning => (Crypt::Wait(None), log),
            Some((turns, at)) => (Crypt::Lever(at), format!("{log}; a lever {turns} turns away")),
            None => (Crypt::Gem(gem), format!("{log}; all discs at the gem's place")),
        }
    })
    .unwrap_or_else(|| logged(Crypt::Wait(None), ""))
}

/// What the charger step does next.
enum Charge {
    /// Put the staff in the charger (use, looking at it).
    Insert([f32; 3]),
    /// Kill zombies with the pistol near it.
    Kill,
    /// Pick up the upgraded staff (use, looking at it).
    Pick([f32; 3]),
    Wait,
}

/// The staff's charger: once its upgrade is unlocked it takes the staff
/// (`charger.is_inserted`), 20 kills by a player within 3000 of it charge
/// it (`charges_received`, `is_charged`), and the upgraded staff is picked
/// up from it.
fn charge_target(world: &mut World, weap: &str) -> (Charge, String) {
    with_vm(world, |vm, world| {
        let Some(s) = staff_obj(vm, weap) else {
            return (Charge::Wait, "no staff entry yet".to_owned());
        };
        let cf = vm.intern("charger");
        let ScriptValue::Object(c) = vm.raw_field(s, cf) else {
            return (Charge::Wait, "the staff has no charger".to_owned());
        };
        let at = vec3(vm, c, "origin").or_else(|| ent_field(vm, world, s, "charger"));
        let (fi, fc, ft) = (vm.intern("is_inserted"), vm.intern("is_charged"), vm.intern("charge_trigger"));
        let inserted = gsc_t6::truthy(&vm.raw_field(c, fi));
        let charged = gsc_t6::truthy(&vm.raw_field(c, fc));
        let n = int(vm, c, "charges_received").unwrap_or(0);
        let trig = !matches!(vm.raw_field(s, ft), ScriptValue::Undefined);
        let log = format!(
            "charger inserted {} charged {} charges {n}/20 trigger {}",
            u8::from(inserted),
            u8::from(charged),
            u8::from(trig)
        );
        let Some(at) = at else {
            return (Charge::Wait, format!("{log}; no charger origin"));
        };
        match (inserted, charged) {
            _ if !trig => (Charge::Wait, format!("{log}; waiting for the charger to open")),
            (false, _) => (Charge::Insert(at), log),
            (true, false) => (Charge::Kill, log),
            (true, true) => (Charge::Pick(at), log),
        }
    })
    .unwrap_or_else(|| logged(Charge::Wait, ""))
}

/// What a staff step does next: where it aims, where he stands for it, and
/// the log line.
struct StaffMove {
    aim: Option<[f32; 3]>,
    stand: Option<[f32; 3]>,
    log: String,
}

/// No target and no spot to stand on: only the log line.
fn nothing(log: String) -> StaffMove {
    StaffMove { aim: None, stand: None, log }
}

/// The nearest living zombie to `me` within `reach`: a spot on its chest.
fn nearest_zombie(world: &mut World, me: [f32; 3], reach: f32) -> Option<[f32; 3]> {
    let zm = world.resource::<super::Zm>();
    zm.actors
        .iter()
        .where_value(|lhs| lhs.living && lhs.scripted.is_none())
        .filter_map(|(nn, lhs)| zm.ents.get(nn).map(|en| (en.origin, lhs.height)))
        .map(|(o, h)| (gsc_t6::math::length(gsc_t6::math::sub(o, me)), [o[0], o[1], o[2] + h * 0.7]))
        .filter(|(d, _)| *d < reach)
        .min_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0))
        .map(|(_, at)| at)
}

pub(crate) fn staff_puzzle_test(world: &mut World, now: i64) {
    let Some((steps, at)) = want() else {
        return;
    };
    if now < *at {
        return;
    }
    let client = ClientId(0);
    let mut st = STATE.lock().unwrap();
    let Some(step) = steps.get(st.stage_no) else {
        return;
    };
    if st.homes.is_none() {
        let homes = gem_homes(world);
        diag::info!(Sim, "bo2zm t6 staff puzzle test {}s: crypt gems at {homes:?}", now / 1000);
        st.homes = Some(homes);
    }
    if step_done(world, step) {
        diag::info!(
            Sim,
            "bo2zm t6 staff puzzle test {}s: {step} done ({} set) after {} shots",
            now / 1000,
            flag(step),
            st.shots
        );
        st.stage_no += 1;
        st.shots = 0;
        st.given = false;
        return;
    }
    let want_staff = staff(step);
    if !st.given {
        st.given = true;
        // The first puzzles are in the Crazy Place. He gets there through a
        // portal, and the record placed at a portal sets
        // `activate_zone_chamber` (run_gramophone_teleporter), which turns
        // its zones and their spawn spots on; the aid puts him there
        // directly, so it sets the flag the record would.
        if step.ends_with('1') {
            let set = with_vm(world, |vm, world| {
                if level_flag(vm, "activate_zone_chamber") {
                    return false;
                }
                let label = vm.string("activate_zone_chamber");
                let level = ScriptValue::Object(vm.level);
                vm.spawn_named(world, "common_scripts/utility", "flag_set", level, vec![label]);
                true
            })
            .unwrap_or(false);
            if set {
                diag::info!(Sim, "bo2zm t6 staff puzzle test {}s: {step}: activate_zone_chamber set (as the record at a portal does)", now / 1000);
            }
        }
        if std::env::var_os("BO2MP_T6_STAFFPUZ_GIVE").is_some() {
            with_vm(world, |vm, world| {
                if let Some(p) = world.resource::<super::Zm>().players.get(&client.0).map(|p| p.obj) {
                    let g = vm.string(want_staff);
                    vm.spawn_named(world, "maps/mp/zombies/_zm_weapons", "weapon_give", ScriptValue::Object(p), vec![g]);
                }
            });
            diag::info!(Sim, "bo2zm t6 staff puzzle test {}s: {step}: given {want_staff}", now / 1000);
            // The fire pits only take bodies once the fire staff is crafted
            // (`level.craftables_crafted["elemental_staff_fire"]`): the given
            // staff stands in for the crafted one.
            if step == "fire1" {
                let marked = with_vm(world, |vm, _| {
                    let f = vm.intern("craftables_crafted");
                    let ScriptValue::Array(a) = vm.raw_field(vm.level, f) else {
                        return false;
                    };
                    let k = Key::Str(vm.intern("elemental_staff_fire"));
                    a.write().set(k, ScriptValue::Int(1));
                    true
                })
                .unwrap_or(false);
                diag::info!(Sim, "bo2zm t6 staff puzzle test {}s: fire1: fire staff marked crafted: {marked}", now / 1000);
            }
        }
        st.prior_shot = now + 3000 - SHOT;
        return;
    }
    let cadence = if step.ends_with('4') { CHARGE_SHOT } else { SHOT };
    if now - st.prior_shot < cadence {
        return;
    }
    st.prior_shot = now;
    let held = {
        let w = frame(world).avatar(client).map(|p| p.weapon);
        w.map(|w| super::weapon_text(world, w)).unwrap_or_default()
    };
    // The gun this shot wants (ice2 breaks a frozen rock with the pistol),
    // and whether the action is a use instead of a shot (elec2's switches).
    let mut staff_wpn = want_staff;
    let mut use_it = false;
    let StaffMove { aim, stand, log } = match step.as_str() {
        "air1" => {
            let (next, log) = air1_target(world);
            match next {
                Some((label, band, middle)) => {
                    // Under the rings, out along the side he shoots.
                    let floor = floor_below(world, middle);
                    let stand = floor.and_then(|f| super::tomb_test::place_near(world, f, 0.0, 120.0)).or(floor);
                    let none = if floor.is_none() { " (no floor under it)" } else { "" };
                    StaffMove {
                        aim: Some(band),
                        stand,
                        log: format!("{log}; shooting {label} at ({:.0} {:.0} {:.0}){none}", band[0], band[1], band[2]),
                    }
                }
                None => nothing(log),
            }
        }
        "air2" => {
            let (next, log) = air2_target(world);
            match next {
                Some((smoke, way)) => {
                    // Behind the smoke, so the shot goes through it toward the
                    // write_target: its way must be within the smoke's script_int (25)
                    // degrees of the way to the write_target, which climbs (spz8: from
                    // 160 back his shot dropped onto it, dot 0.796), so the
                    // spot furthest along that line with a clear shot.
                    let (stand, dot) = smoke_spot(world, smoke, way).map_or((None, 0.0), |(s, d)| (Some(s), d));
                    StaffMove { aim: Some(smoke), stand, log: format!("{log}; shooting the smoke at ({:.0} {:.0} {:.0}), shot dot {dot:.3}", smoke[0], smoke[1], smoke[2]) }
                }
                None => nothing(log),
            }
        }
        "ice1" => {
            let (next, log) = ice1_target(world);
            match next {
                Some(tile) => {
                    // Where his shot goes clear up to the tile (spz7: from
                    // under it the ball burst 45 from his eye).
                    let spot = clear_shot(world, tile, None);
                    let floor = floor_below(world, tile);
                    let stand = spot
                        .map(|s| s.0)
                        .or_else(|| floor.and_then(|f| super::tomb_test::place_near(world, f, 0.0, 120.0)))
                        .or(floor);
                    let how = match (spot, floor) {
                        (Some(s), _) => format!("goes off {:.0} from it", s.1),
                        (None, Some(_)) => "no clear shot".to_owned(),
                        (None, None) => "no clear shot (no floor under it)".to_owned(),
                    };
                    StaffMove { aim: Some(tile), stand, log: format!("{log}; shooting the tile at ({:.0} {:.0} {:.0}), {how}", tile[0], tile[1], tile[2]) }
                }
                None => nothing(log),
            }
        }
        "ice2" => {
            let (next, log) = ice2_target(world);
            match next {
                Some(rock) => {
                    if st.pistol {
                        staff_wpn = PISTOL;
                    }
                    let stand = floor_under(world, [rock[0], rock[1], rock[2] + 40.0])
                        .and_then(|f| super::tomb_test::place_near(world, f, 0.0, 150.0));
                    StaffMove { aim: Some(rock), stand, log: format!("{log}; shooting the rock at ({:.0} {:.0} {:.0}) with {staff_wpn}", rock[0], rock[1], rock[2]) }
                }
                None => nothing(log),
            }
        }
        "fire1" => {
            let (next, log) = fire1_target(world);
            match next {
                Some(FirePit { fire, zombie }) => {
                    let stand = floor_under(world, [fire[0], fire[1], fire[2] + 40.0]);
                    let floor = stand.map_or(fire[2], |s| s[2]);
                    match zombie {
                        Some(feet) => StaffMove { aim: Some(feet), stand, log: format!("{log}; a zombie on the pit") },
                        // No one on it: light it (a body burns while it is lit).
                        None => StaffMove { aim: Some([fire[0] + 40.0, fire[1], floor]), stand, log: format!("{log}; lighting the pit") },
                    }
                }
                None => nothing(log),
            }
        }
        "fire2" => {
            let (next, log) = fire2_target(world);
            match next {
                Some(torch) => {
                    // In the church where his shot goes off within 64 of the
                    // torch (`fire_puzzle_torch_run`); the torches sit in the
                    // wall, so every way out from one starts in it (fire3: he
                    // stayed in the chamber, 13000 away).
                    let spot = clear_shot(world, torch, None);
                    let stand = spot.map(|s| s.0).or_else(|| stand_off(world, torch, 160.0));
                    let how = spot.map_or("no clear shot".to_owned(), |s| format!("goes off {:.0} from it", s.1));
                    StaffMove { aim: Some(torch), stand, log: format!("{log}; shooting the torch at ({:.0} {:.0} {:.0}), {how}", torch[0], torch[1], torch[2]) }
                }
                None => nothing(log),
            }
        }
        "elec1" => {
            let ringing = with_vm(world, |vm, _| level_flag(vm, "piano_chord_ringing")).unwrap_or(false);
            if ringing {
                st.ringing = true;
                st.full_since = None;
                nothing(format!("chord {} ringing", CHORDS.get(st.key_combo).unwrap_or(&"?")))
            } else {
                if st.ringing {
                    st.ringing = false;
                    if st.key_combo < 3 {
                        st.solved |= 1 << st.key_combo;
                    }
                    st.key_combo = next_chord(st.solved, st.key_combo);
                }
                let Chord { mut next, mut center, full, mut log } = elec1_target(world, st.key_combo);
                // All three ring and the key_combo does not take: the script
                // wants another one (its order, not ours); a key of it
                // stops these (`piano_keys_stop`).
                if full {
                    let since = *st.full_since.get_or_insert(now);
                    if now - since >= 3000 && st.key_combo < 3 {
                        let was = st.key_combo;
                        st.key_combo = next_chord(st.solved, was);
                        st.full_since = None;
                        let Chord { next: n, center: c, log: l, .. } = elec1_target(world, st.key_combo);
                        (next, center) = (n, c);
                        log = format!("{} not taken, trying {}; {l}", CHORDS[was], CHORDS.get(st.key_combo).unwrap_or(&"none"));
                    }
                } else {
                    st.full_since = None;
                }
                match next {
                    Some(piano_key) => {
                        // Out from the key (toward the chamber's middle first)
                        // where his shot goes off on the key, not on a pillar
                        // on the way (spz4: every ball burst 24 out).
                        let spot = clear_shot(world, piano_key, center);
                        let stand = spot.map(|s| s.0).or_else(|| stand_off(world, piano_key, 160.0));
                        let how = spot.map_or("no clear shot".to_owned(), |s| format!("goes off {:.0} from it", s.1));
                        StaffMove { aim: Some(piano_key), stand, log: format!("{log}; shooting the key at ({:.0} {:.0} {:.0}), {how}", piano_key[0], piano_key[1], piano_key[2]) }
                    }
                    None => nothing(log),
                }
            }
        }
        "elec2" => {
            let (next, log) = elec2_target(world);
            match next {
                Some(switch) => {
                    use_it = true;
                    let stand = floor_under(world, [switch[0], switch[1], switch[2] + 20.0])
                        .and_then(|f| super::tomb_test::place_near(world, f, 0.0, 30.0));
                    StaffMove { aim: Some(switch), stand, log: format!("{log}; turning the switch at ({:.0} {:.0} {:.0})", switch[0], switch[1], switch[2]) }
                }
                None => nothing(log),
            }
        }
        s if s.ends_with('3') => {
            let elem = s.trim_end_matches(char::is_numeric);
            let home = st
                .homes
                .iter()
                .flatten()
                .find(|(n, ..)| *n == format!("crypt_gem_{elem}"))
                .map(|(_, at, i)| (*at, *i));
            let (next, log) = crypt_target(world, elem, home);
            match next {
                // In the crypt by its gem, so no one holds the orb in its chamber.
                Crypt::Wait(Some(at)) => StaffMove { aim: None, stand: gem_spot(world, at), log },
                Crypt::Wait(None) => nothing(log),
                Crypt::Lever(at) => {
                    use_it = true;
                    let stand = floor_under(world, [at[0], at[1], at[2] + 20.0])
                        .and_then(|f| super::tomb_test::place_near(world, f, 0.0, 50.0));
                    StaffMove { aim: Some(at), stand, log: format!("{log}; pulling the lever at ({:.0} {:.0} {:.0})", at[0], at[1], at[2]) }
                }
                Crypt::Gem(at) => {
                    let stand = gem_spot(world, at);
                    StaffMove { aim: Some(at), stand, log: format!("{log}; shooting the gem at ({:.0} {:.0} {:.0})", at[0], at[1], at[2]) }
                }
            }
        }
        s if s.ends_with('4') => {
            let (next, log) = charge_target(world, want_staff);
            match next {
                Charge::Insert(at) | Charge::Pick(at) => {
                    use_it = true;
                    let stand = floor_under(world, [at[0], at[1], at[2] + 20.0])
                        .and_then(|f| super::tomb_test::place_near(world, f, 0.0, 60.0));
                    StaffMove { aim: Some(at), stand, log: format!("{log}; using the charger at ({:.0} {:.0} {:.0})", at[0], at[1], at[2]) }
                }
                Charge::Kill => {
                    staff_wpn = PISTOL;
                    let me = frame(world).avatar(client).map_or([0.0; 3], |p| p.origin);
                    match nearest_zombie(world, me, 1500.0) {
                        Some(z) => StaffMove { aim: Some(z), stand: None, log: format!("{log}; shooting a zombie") },
                        None => nothing(format!("{log}; no zombie near")),
                    }
                }
                Charge::Wait => nothing(log),
            }
        }
        other => {
            diag::info!(Sim, "bo2zm t6 staff puzzle test: no step called {other}");
            st.stage_no += 1;
            return;
        }
    };
    let mut shifted = false;
    if let Some(s) = stand {
        let me = frame(world).avatar(client).map(|p| p.origin);
        if me.is_none_or(|m| gsc_t6::math::length(gsc_t6::math::sub(m, s)) > 24.0) {
            super::teleport_player(world, client, s);
            shifted = true;
        }
    }
    let action = match aim {
        // A switch's use trigger comes when he is near it: use it next time.
        Some(a) if use_it && !shifted => {
            super::tomb_test::look_and_use(world, client, a);
            st.shots += 1;
            "used"
        }
        Some(_) if use_it => "walked up",
        Some(_) if held != staff_wpn => {
            if let Ok(w) = super::weapon(world, staff_wpn) {
                crate::client_script::switch_to_weapon(&mut frame(world), client, w);
            }
            "switching guns"
        }
        Some(staff_wpn) => {
            shoot(world, client, staff_wpn);
            st.shots += 1;
            if step == "ice2" {
                st.pistol = !st.pistol;
            }
            "fired"
        }
        None => "held fire",
    };
    let me = frame(world).avatar(client).map_or([0.0; 3], |p| p.origin);
    diag::info!(
        Sim,
        "bo2zm t6 staff puzzle test {}s: {step} shot {} {action}; holding {held}; at ({:.0} {:.0} {:.0}); {log}",
        now / 1000,
        st.shots,
        me[0],
        me[1],
        me[2]
    );
}
