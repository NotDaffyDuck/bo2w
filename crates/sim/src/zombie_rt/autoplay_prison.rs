//! Mob of the Dead (zm_prison) for the autoplay test player: every player
//! starts in Afterlife and goes back to it when downed, and only walking to
//! his own body and holding Use brings him back. Without this the test
//! player would stand as a ghost forever.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use std::sync::atomic::{AtomicI64, Ordering};

use gsc_t6::{ObjKind, ScriptValue};
use playerstate_t6::game::buttons;

use super::{Zm, with_vm};
use crate::sim_parts::ClientId;

/// Where the player's own body lies while he is in Afterlife
/// (`self.afterlife` and `self.e_afterlife_corpse`, _zm_afterlife).
pub(super) fn afterlife_corpse(world: &mut World, client: ClientId) -> Option<[f32; 3]> {
    let corpse = with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&client.0)?.obj;
        let f = vm.intern("afterlife");
        if !gsc_t6::truthy(&vm.get_field(world, p, f)) {
            return None;
        }
        let f = vm.intern("e_afterlife_corpse");
        match vm.get_field(world, p, f) {
            ScriptValue::Object(o) => match vm.kind(o) {
                Some(ObjKind::Entity(n)) => Some(n),
                _ => None,
            },
            _ => None,
        }
    })??;
    world.resource::<Zm>().ents.get(&corpse).map(|e| e.origin)
}

/// The keys that walk to the body and hold Use on it:
/// (buttons, forward, right, view angles).
pub(super) fn revive_keys(
    spot: [f32; 3],
    eye: [f32; 3],
    corpse: [f32; 3],
) -> (u32, i8, i8, [f32; 3]) {
    revive_keys_via(spot, eye, corpse, corpse)
}

/// `revive_keys` walking at `toward` (the next path node) until the body
/// is in reach.
pub(super) fn revive_keys_via(
    spot: [f32; 3],
    eye: [f32; 3],
    corpse: [f32; 3],
    toward: [f32; 3],
) -> (u32, i8, i8, [f32; 3]) {
    let flat = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    if flat(corpse, spot) > 40.0 {
        (0, 127, 0, super::autoplay::look(eye, toward))
    } else {
        (buttons::INTERACT_KEY, 0, 0, super::autoplay::look(eye, corpse))
    }
}

/// BO2MP_T6_ZAPBRUTUS=1: for the first 6 s of each Afterlife a ghost with a
/// Brutus within 900 units zaps him with the lightning hands (fire tapped,
/// aimed at his chest). BO2 sends him away for it (_zm_ai_brutus
/// brutus_damage_override: lightning_hands_zm -> brutus_afterlife_teleport,
/// hidden, deleted, respawned elsewhere). The view to aim with, or None.
pub(super) fn ghost_zap_brutus(game: &mut World, now_ms: i64, spot: [f32; 3], eye_at: [f32; 3]) -> Option<[f32; 3]> {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_ZAPBRUTUS").is_some()) {
        return None;
    }
    // A gap of over 1 s since the last ghost tick starts a new Afterlife.
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    static LAST: AtomicI64 = AtomicI64::new(i64::MIN);
    if now_ms - LAST.swap(now_ms, Ordering::Relaxed) > 1000 {
        START.store(now_ms, Ordering::Relaxed);
    }
    if now_ms - START.load(Ordering::Relaxed) > 6000 {
        return None;
    }
    let (n, at) = brutus_pos(game)?;
    let d = ((at[0] - spot[0]).powi(2) + (at[1] - spot[1]).powi(2)).sqrt();
    if d > 900.0 {
        return None;
    }
    if now_ms % 1000 < i64::from(crate::SIM_STEP_MILLIS) {
        diag::info!(
            Sim,
            "bo2zm t6 ghost zaps brutus ent{n} at {:?} ({d:.0} away)",
            at.map(f32::round)
        );
    }
    Some(super::autoplay::look(eye_at, [at[0], at[1], at[2] + 56.0]))
}

/// Where a ghost walks to reach his body: the next node on the zombies'
/// path graph (the ghost respawns at a set point and a straight line to
/// the body can run into a wall: bhit8 stood 30 s against one till the
/// meter ran out). The body itself when the path is short or missing.
pub(super) fn body_waypoint(game: &World, spot: [f32; 3], corpse: [f32; 3]) -> [f32; 3] {
    let nav = &game.resource::<Zm>().nav;
    let (Some(a), Some(b)) = (nav.nearest(spot), nav.nearest(corpse)) else {
        return corpse;
    };
    let Some(mut path) = nav.path(a, b) else {
        return corpse;
    };
    let flat = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let at = |n: u32| nav.graph[n as usize].origin;
    // Already between the first two nodes: no turning back for the first.
    if path.len() >= 2 && flat(spot, at(path[1])) < flat(at(path[0]), at(path[1])) {
        path.remove(0);
    }
    path.iter()
        .map(|&n| at(n))
        .find(|&o| flat(o, spot) > 32.0)
        .filter(|_| path.len() > 1)
        .unwrap_or(corpse)
}

/// BO2MP_T6_SHOCK=<script_string> (e.g. `revive_on`; `list` only logs them):
/// the shock boxes (`afterlife_interact`; an Afterlife door's box by its
/// targetname). At 1 s every box is logged. Once
/// the player holds the lightning hands (the Afterlife start, 10 to 40 s in)
/// he is put in front of the box that powers that, looking at it, and taps
/// fire for 20 s, his weapon and clip logged each second; then the boxes are
/// logged again (`p6_zm_al_shock_box_on` once zapped). For the next 10 s he
/// is put by his body and holds Use, so he is alive again; then BO2MP_T6_PERK
/// runs (its 42 s step comes 32 s after the hands).
pub(crate) fn shock_test(world: &mut World, now_ms: i64) {
    use std::sync::OnceLock;
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("BO2MP_T6_SHOCK").ok())
        .clone()
    else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    // When he first held the hands (and was put at the box).
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    let start = START.load(Ordering::Relaxed);
    let rel = if start == i64::MIN { -1 } else { now_ms - start };
    let at = |t: i64| (t..t + tick).contains(&now_ms);
    let at_rel = |t: i64| (t..t + tick).contains(&rel);
    if (21000 + tick..31000).contains(&rel) {
        revive_at_body(world, rel, 21000);
        return;
    }
    if at_rel(32000) {
        super::autoplay::PERK_BASE.store(now_ms - 42000, Ordering::Relaxed);
        return;
    }
    // BO2 sets the gondola and the traps up once the round logic starts
    // (zm_alcatraz_classic: flag_wait start_zombie_round_logic, then
    // init_alcatraz_zipline waits for its box's notify); a zap before that
    // is missed, and no one can reach a box sooner. Its main() waits for
    // initial_blackscreen_passed first (gondola9 zapped at 5 s, before
    // both had passed, and the gondola never heard it). 2 s after both.
    static ROUNDS: AtomicI64 = AtomicI64::new(i64::MIN);
    if ROUNDS.load(Ordering::Relaxed) == i64::MIN
        && with_vm(world, |vm, _| {
            super::prison_plane::level_flag(vm, "initial_blackscreen_passed")
                && super::prison_plane::level_flag(vm, "start_zombie_round_logic")
        })
        .unwrap_or(false)
    {
        ROUNDS.store(now_ms, Ordering::Relaxed);
        diag::info!(
            Sim,
            "bo2zm t6 shock test: blackscreen + round logic passed at {now_ms} ms"
        );
    }
    let rounds = ROUNDS.load(Ordering::Relaxed);
    let placing = start == i64::MIN
        && rounds != i64::MIN
        && now_ms >= rounds + 2000
        && (5000..120000).contains(&now_ms);
    if !(at(1000) || placing || (0..20000).contains(&rel) || at_rel(21000)) {
        return;
    }
    // (entnum, script_string, origin, model)
    let boxes: Vec<(u32, String, [f32; 3], String)> = with_vm(world, |vm, world| {
        let (tn, ss) = (vm.intern("targetname"), vm.intern("script_string"));
        world
            .resource::<Zm>()
            .ents
            .iter()
            .filter_map(|(n, e)| {
                let o = e.obj?;
                let name = vm.to_text(&vm.raw_field(o, tn));
                // An Afterlife door's box (zm_prison alcatraz_afterlife_doors)
                // goes by its own targetname.
                let label = if name == "afterlife_interact" {
                    vm.to_text(&vm.raw_field(o, ss))
                } else if e.mesh.starts_with("p6_zm_al_shock_box") && !name.is_empty() {
                    name
                } else {
                    return None;
                };
                Some((*n, label, e.origin, e.mesh.clone()))
            })
            .collect()
    })
    .unwrap_or_default();
    if at(1000) || at_rel(21000) {
        for (n, s, o, m) in &boxes {
            diag::info!(
                Sim,
                "bo2zm t6 shock box ent{n} {s:?} {m} at ({:.0} {:.0} {:.0})",
                o[0],
                o[1],
                o[2]
            );
        }
        // The perk machines: a powered one has its "_on" model.
        for (n, e) in &world.resource::<Zm>().ents {
            if e.mesh.contains("vending") {
                diag::info!(Sim, "bo2zm t6 shock perk machine ent{n} {}", e.mesh);
            }
        }
        return;
    }
    let Some(e) = boxes
        .iter()
        .find(|b| b.1 == want)
        .and_then(|b| world.resource::<Zm>().ents.get(&b.0).cloned())
    else {
        return;
    };
    let peer = ClientId(0);
    let target = super::brushes::model_box(world, &e).map_or(e.origin, |(lo, hi)| {
        std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5)
    });
    // Placed once he holds the lightning hands: the Afterlife start moves
    // him to its own spot first (its time varies with the load).
    let held = super::frame(world).avatar(peer).map_or(0, |p| p.weapon);
    let hands = held != 0 && super::weapon_text(world, held) == "lightning_hands_zm";
    if start == i64::MIN {
        if !hands {
            return;
        }
        START.store(now_ms, Ordering::Relaxed);
        let Some(feet) = zap_spot(world, target, e.rot[1]) else {
            diag::warn!(Sim, "bo2zm t6 shock test: no open side at the {want} box");
            return;
        };
        super::teleport_player(world, peer, feet);
        diag::info!(
            Sim,
            "bo2zm t6 shock test: {want} box at ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0})",
            target[0],
            target[1],
            target[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    // Look at it every tick of the zap (his eye is 60 up from his feet).
    let Some(origin) = super::frame(world).avatar(peer).map(|p| p.origin) else {
        return;
    };
    let view = super::autoplay::look([origin[0], origin[1], origin[2] + 60.0], target);
    super::set_player_view(world, peer, view);
    if now_ms % 1000 < tick {
        let fl = super::frame(world);
        if let Some(pstate) = fl.avatar(peer) {
            let clip_no = crate::client_script::ammo_clip(&fl, peer, pstate.weapon);
            let facts = fl.damage_profile_of(pstate.weapon).map(|c| {
                (
                    c.magazine_capacity,
                    c.spawn_rounds,
                    c.reserve_cap,
                    c.gun_family_code,
                    c.gun_kind,
                    c.trigger_mode,
                )
            });
            diag::info!(
                Sim,
                "bo2zm t6 shock test: weapon {} state {} clip {clip_no} (clip size, start, max, type, class, fire) {facts:?}",
                pstate.weapon,
                pstate.weaponstate_primary
            );
        }
    }
    if now_ms % 500 < 250 {
        let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
        let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == peer);
        for (_, command) in mine {
            command.buttons |= buttons::ATTACK;
        }
    }
}

/// Where he stands to zap something at `target` (a shock box facing `yaw`):
/// the first spot on its four sides (80 to 160 units out), else its corners,
/// where he stands clear and his eye sees it. His feet there.
pub(super) fn zap_spot(game: &mut World, tgt: [f32; 3], yw: f32) -> Option<[f32; 3]> {
    zap_spot_within(game, tgt, yw, None)
}

/// As [`zap_spot`], but with `origin` his feet stay within 240 of it: BO2's
/// `afterlife_interact_object_think` only counts a zap from a ghost within
/// 256 of the object's origin (the key pulley's panel hangs high, and a spot
/// under its middle was 300 out). Spots round the origin are tried too.
pub(super) fn zap_spot_within(
    world: &mut World,
    tgt: [f32; 3],
    yaw: f32,
    spot: Option<[f32; 3]>,
) -> Option<[f32; 3]> {
    let (fw, rt, _) = gsc_t6::math::facing_axes([0.0, yaw, 0.0]);
    let sides = [
        [-rt[0], -rt[1], 0.0],
        fw,
        [rt[0], rt[1], 0.0],
        [-fw[0], -fw[1], 0.0],
    ];
    let f = super::frame(world);
    let reach = |feet: &[f32; 3]| {
        spot.is_none_or(|o| (0..3).map(|i| (feet[i] - o[i]).powi(2)).sum::<f32>() < 240.0 * 240.0)
    };
    let spot_from = |c: [f32; 3], d: &[f32; 3], dist: f32| -> Option<[f32; 3]> {
        let stand: [f32; 3] = std::array::from_fn(|i| c[i] + d[i] * dist);
        let down = [stand[0], stand[1], stand[2] - 200.0];
        let t = f.trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        if t.starts_inside || t.fraction >= 1.0 {
            return None;
        }
        let feet: [f32; 3] =
            std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        if !reach(&feet) {
            return None;
        }
        let eye = [feet[0], feet[1], feet[2] + 60.0];
        let sight = f.trace_static_world(
            eye,
            tgt,
            [0.0; 3],
            [0.0; 3],
            crate::shot_trace::MASK_SHOT,
        );
        (!sight.starts_inside && sight.fraction > 0.9).then_some(feet)
    };
    // Else the corners too, 40 to 240 out (the zap reaches 256).
    let corners: Vec<[f32; 3]> = (0..4)
        .map(|k| {
            let (a, b) = (sides[k], sides[(k + 1) % 4]);
            let h = std::f32::consts::FRAC_1_SQRT_2;
            [(a[0] + b[0]) * h, (a[1] + b[1]) * h, 0.0]
        })
        .collect();
    let around = |c: [f32; 3]| {
        [80.0, 110.0, 140.0, 160.0]
            .iter()
            .find_map(|&dist| sides.iter().find_map(|d| spot_from(c, d, dist)))
            .or_else(|| {
                [40.0, 60.0, 80.0, 110.0, 140.0, 160.0, 200.0, 240.0]
                    .iter()
                    .find_map(|&dist| sides.iter().chain(&corners).find_map(|d| spot_from(c, d, dist)))
            })
    };
    around(tgt).or_else(|| spot.and_then(|o| around([o[0], o[1], tgt[2]])))
}

/// The shock test's way back from Afterlife: put him beside his body once,
/// then look at it and hold Use till he is up.
pub(super) fn revive_at_body(world: &mut World, rel: i64, t0: i64) {
    let client = ClientId(0);
    let Some(corpse) = afterlife_corpse(world, client) else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if (t0 + tick..t0 + 2 * tick).contains(&rel) {
        super::teleport_player(world, client, [corpse[0] + 30.0, corpse[1], corpse[2]]);
        diag::info!(
            Sim,
            "bo2zm t6 shock test: back to the body at ({:.0} {:.0} {:.0})",
            corpse[0],
            corpse[1],
            corpse[2]
        );
    }
    let Some(spot) = super::frame(world).avatar(client).map(|p| p.origin) else {
        return;
    };
    let eye_at = [spot[0], spot[1], spot[2] + 60.0];
    let (press, _, _, view) = revive_keys(spot, eye_at, corpse);
    super::set_player_view(world, client, view);
    let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
    let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
    for (_, cmd) in mine {
        cmd.buttons |= press;
    }
}

/// One door or debris pile the door sweep buys: its use trigger, its targetname,
/// `script_flag`, `script_noteworthy` and `zombie_cost`, and where the thing it opens is.
#[derive(Clone)]
struct SweptDoor {
    trig_ent: u32,
    tname: String,
    flags: String,
    noteworthy: String,
    price: i32,
    opens_at: Option<[f32; 3]>,
}

/// BO2MP_T6_DOORS=1 (or a comma list of script_flags): every door and debris
/// pile bought by hand, one after the other. Once the player holds the
/// lightning hands he is put by his body and holds Use for 10 s (revived);
/// from 13 s after the hands, every 4 s: 10000 points, put in the next
/// door's use trigger looking at what it opens, Use held 0.5-1.5 s; at 3.5 s
/// its flag, `_door_open`, his points and the hint are logged. With
/// BO2MP_T6_SHOCK it starts once that test has him up again. Then
/// BO2MP_T6_BOX runs (its clock starts there).
pub(crate) fn door_sweep(world: &mut World, now_ms: i64) {
    use std::sync::{Mutex, OnceLock};
    static WANT: OnceLock<Option<Vec<String>>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| {
            std::env::var("BO2MP_T6_DOORS").ok().map(|v| {
                if v == "1" {
                    Vec::new()
                } else {
                    v.split(',').map(str::to_owned).collect()
                }
            })
        })
        .clone()
    else {
        return;
    };
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    static DOORS: Mutex<Vec<SweptDoor>> = Mutex::new(Vec::new());
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let mut from_pt = START.load(Ordering::Relaxed);
    // With the shock test it starts once that is done and he is up again.
    if from_pt == i64::MIN && std::env::var_os("BO2MP_T6_SHOCK").is_some() {
        let base = super::autoplay::PERK_BASE.load(Ordering::Relaxed);
        if base == i64::MIN {
            return;
        }
        // The perk test (BO2MP_T6_PERK) has the player from then until its
        // "50 s" log; a door before that took his Use and points (bl11).
        if std::env::var_os("BO2MP_T6_PERK").is_some() && now_ms < base + 51000 {
            return;
        }
        from_pt = now_ms - 12000;
        START.store(from_pt, Ordering::Relaxed);
    }
    if from_pt == i64::MIN {
        let held = super::frame(world).avatar(client).map_or(0, |p| p.weapon);
        if held == 0 || super::weapon_text(world, held) != "lightning_hands_zm" {
            return;
        }
        from_pt = now_ms;
        START.store(now_ms, Ordering::Relaxed);
    }
    let rel = now_ms - from_pt;
    if (1000 + tick..11000).contains(&rel) {
        revive_at_body(world, rel, 1000);
        return;
    }
    if rel < 13000 {
        return;
    }
    let mut doors = DOORS.lock().unwrap();
    if (13000..13000 + tick).contains(&rel) {
        *doors = with_vm(world, |vm, world| {
            let (tn, flag, note, cost, target) = (
                vm.intern("targetname"),
                vm.intern("script_flag"),
                vm.intern("script_noteworthy"),
                vm.intern("zombie_cost"),
                vm.intern("target"),
            );
            let zm = world.resource::<Zm>();
            let mut v: Vec<SweptDoor> = zm
                .ents
                .iter()
                .where_value(|en| en.class_tag.starts_with("trigger_use"))
                .filter_map(|(nn, en)| {
                    let o = en.obj?;
                    let name = vm.to_text(&vm.raw_field(o, tn));
                    if name != "zombie_door" && name != "zombie_debris" {
                        return None;
                    }
                    let fl = vm.to_text(&vm.raw_field(o, flag));
                    if !want.is_empty() && !want.contains(&fl) {
                        return None;
                    }
                    let t = vm.to_text(&vm.raw_field(o, target));
                    let aim = zm
                        .ents
                        .values()
                        .find(|en| {
                            en.obj
                                .is_some_and(|eo| vm.to_text(&vm.raw_field(eo, tn)) == t)
                        })
                        .map(|en| en.origin);
                    let c = match vm.raw_field(o, cost) {
                        ScriptValue::Int(c) => c as i32,
                        _ => -1,
                    };
                    Some(SweptDoor {
                        trig_ent: *nn,
                        tname: name,
                        flags: fl,
                        noteworthy: vm.to_text(&vm.raw_field(o, note)),
                        price: c,
                        opens_at: aim,
                    })
                })
                .collect();
            v.sort_by_key(|d| d.trig_ent);
            v
        })
        .unwrap_or_default();
        for d in doors.iter() {
            diag::info!(
                Sim,
                "bo2zm t6 doors: ent{} {} flag {:?} {:?} cost {} opens {:?}",
                d.trig_ent,
                d.tname,
                d.flags,
                d.noteworthy,
                d.price,
                d.opens_at.map(|a| a.map(f32::round))
            );
        }
    }
    let step = rel - 13000;
    let (i, t) = ((step / 4000) as usize, step % 4000);
    let Some(door) = doors.get(i).cloned() else {
        if t < tick && i == doors.len() {
            diag::info!(Sim, "bo2zm t6 doors: sweep done, {} doors", doors.len());
            super::autoplay::BOX_BASE.store(now_ms - 11500, Ordering::Relaxed);
        }
        if t < tick && i >= doors.len() && (i - doors.len()) % 2 == 0 {
            drop(doors);
            box_zone_report(world);
        }
        return;
    };
    drop(doors);
    let trig = world.resource::<Zm>().ents.get(&door.trig_ent).cloned();
    if t < tick {
        let Some(trig) = trig else {
            diag::info!(Sim, "bo2zm t6 doors: ent{} gone before its turn", door.trig_ent);
            return;
        };
        let c = super::triggers::center(world, &trig);
        // trigger_off parks a trigger 10000 under the map: that door is
        // open already (its twin was bought, or a shock box opened it).
        if c[2] < -5000.0 {
            diag::info!(Sim, "bo2zm t6 doors: ent{} {:?} switched off (open already), skipped", door.trig_ent, door.flags);
            return;
        }
        super::autoplay::give_points(world, 10000);
        let stand = [c[0], c[1], c[2] + 40.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let tr = super::frame(world).trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|k| stand[k] + (down[k] - stand[k]) * tr.fraction);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 doors: ent{} {:?} trigger at ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0})",
            door.trig_ent,
            door.flags,
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
        return;
    }
    if t < 1500 {
        let Some(trig) = trig else { return };
        let Some(spot) = super::frame(world).avatar(client).map(|p| p.origin) else {
            return;
        };
        let to = door
            .opens_at
            .unwrap_or_else(|| super::triggers::center(world, &trig));
        let view = super::autoplay::look(
            [spot[0], spot[1], spot[2] + 60.0],
            [to[0], to[1], to[2] + 40.0],
        );
        super::set_player_view(world, client, view);
        if t >= 500 {
            let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
            let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
            for (_, cmd) in mine {
                cmd.buttons |= buttons::INTERACT_KEY;
            }
        }
        return;
    }
    if (3500..3500 + tick).contains(&t) {
        let score = super::frame(world)
            .client_meta(client)
            .map_or(0, |m| m.score);
        let (flag_on, open) = with_vm(world, |vm, world| {
            let first = door.flags.split(',').next().unwrap_or("").to_owned();
            let f = vm.intern("flag");
            let flag_on = match vm.raw_field(vm.level, f) {
                ScriptValue::Array(a) => a
                    .get(&gsc_t6::Key::Str(vm.intern(&first)))
                    .is_some_and(|v| gsc_t6::truthy(&v)),
                _ => false,
            };
            let f = vm.intern("_door_open");
            let open = world
                .resource::<Zm>()
                .ents
                .get(&door.trig_ent)
                .and_then(|en| en.obj)
                .map(|o| vm.to_text(&vm.raw_field(o, f)));
            (flag_on, open)
        })
        .unwrap_or((false, None));
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 doors: ent{} {:?} flag {flag_on}, _door_open {:?}, points {score}, hint {hint:?}",
            door.trig_ent,
            door.flags,
            open.unwrap_or_else(|| "gone".into())
        );
    }
}

/// The magic box's use trigger only comes to a player in an active zone
/// (_zm_unitrigger): the showing box's place, its stub's zone and whether
/// that zone is enabled and active, and every active zone.
pub(super) fn box_zone_report(world: &mut World) {
    let line = with_vm(world, |vm, world| {
        let f = |vm: &mut gsc_t6::Vm<World>, o: gsc_t6::ObjRef, s: &str| {
            let k = vm.intern(s);
            vm.raw_field(o, k)
        };
        let t = |vm: &mut gsc_t6::Vm<World>, o: gsc_t6::ObjRef, s: &str| {
            let v = f(vm, o, s);
            vm.to_text(&v)
        };
        let lvl = vm.level;
        let zones = f(vm, lvl, "zones");
        let zone_flags = |vm: &mut gsc_t6::Vm<World>, name: &str| -> String {
            let ScriptValue::Array(a) = &zones else {
                return "no zones".into();
            };
            match a.get(&gsc_t6::Key::Str(vm.intern(name))) {
                Some(ScriptValue::Object(z)) => format!(
                    "enabled {} active {}",
                    t(vm, z, "is_enabled"),
                    t(vm, z, "is_active")
                ),
                _ => "unknown".into(),
            }
        };
        let zm = world.resource::<Zm>();
        let mut boxes = Vec::new();
        let showing: Vec<([f32; 3], gsc_t6::ObjRef)> = zm
            .ents
            .values()
            .filter(|e| e.zbarrier.is_some())
            .filter_map(|e| Some((e.origin, e.obj?)))
            .collect();
        for (o, z) in showing {
            let state = t(vm, z, "state");
            if matches!(state.as_str(), "" | "undefined" | "away") {
                continue;
            }
            let zone = match f(vm, z, "owner") {
                ScriptValue::Object(owner) => match f(vm, owner, "unitrigger_stub") {
                    ScriptValue::Object(stub) => t(vm, stub, "in_zone"),
                    _ => "no stub".into(),
                },
                _ => "no owner".into(),
            };
            let flags = zone_flags(vm, &zone);
            boxes.push(format!(
                "box {state} at ({:.0} {:.0} {:.0}) zone {zone:?} {flags}",
                o[0], o[1], o[2]
            ));
        }
        let active = match f(vm, lvl, "active_zone_names") {
            ScriptValue::Array(a) => a
                .snapshot()
                .values_in_order()
                .map(|v| vm.to_text(v))
                .collect::<Vec<_>>()
                .join(" "),
            _ => "none".into(),
        };
        let lives = match zm.players.get(&0).map(|p| p.obj) {
            Some(p) => format!(
                "lives {} afterlife {} afterliferound {} solo {} intermission {} start_over {}",
                t(vm, p, "lives"),
                t(vm, p, "afterlife"),
                t(vm, p, "afterliferound"),
                t(vm, lvl, "is_forever_solo_game"),
                t(vm, lvl, "intermission"),
                match f(vm, lvl, "flag") {
                    ScriptValue::Array(a) => a
                        .get(&gsc_t6::Key::Str(vm.intern("afterlife_start_over")))
                        .map_or("undefined".into(), |v| vm.to_text(&v)),
                    _ => "no flags".into(),
                }
            ),
            None => "no player".into(),
        };
        format!("{}; active zones: {active}; {lives}", boxes.join("; "))
    })
    .unwrap_or_default();
    diag::info!(Sim, "bo2zm t6 box zones: {line}");
}

/// BO2MP_T6_LIVES=1: logs the player's Afterlife state each time it changes
/// (`self.lives`, `self.afterlife`, health, `sessionstate`, laststand), with
/// the match time, so the step that takes his lives away shows in the log.
pub(crate) fn lives_watch(world: &mut World, now: i64) {
    use std::sync::{Mutex, OnceLock};
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_LIVES").is_some()) {
        return;
    }
    static LAST: Mutex<String> = Mutex::new(String::new());
    let line = with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&0)?.obj;
        let mut out = Vec::new();
        for name in [
            "lives",
            "afterlife",
            "health",
            "sessionstate",
            "laststand",
            "afterlifedeaths",
            // What else BO2's is_player_valid reads (zombies and Brutus
            // skip a player it fails).
            "ignoreme",
            "is_zombie",
            "no_revive_trigger",
        ] {
            let k = vm.intern(name);
            let v = vm.get_field(world, p, k);
            out.push(format!("{name} {}", vm.to_text(&v)));
        }
        let k = vm.intern("revivetrigger");
        let rt = !matches!(vm.get_field(world, p, k), ScriptValue::Undefined);
        out.push(format!("revivetrigger {}", if rt { "set" } else { "none" }));
        Some(out.join(" "))
    })
    .flatten()
    .unwrap_or_default();
    let mut last = LAST.lock().unwrap();
    if *last != line {
        diag::info!(Sim, "bo2zm t6 lives at {:.1}s: {line}", now as f64 / 1000.0);
        *last = line;
    }
}

/// BO2MP_T6_BRUTUS=<seconds>: at that match time `level notify("spawn_brutus",
/// 1)`, as Mob's own devgui switch does (`watch_devgui_brutus`); with any
/// value, every 2 s each living Brutus (`is_brutus`) is logged: place,
/// distance to the player, health, lockdown state, script and animation.
pub(crate) fn brutus_test(world: &mut World, now: i64) {
    use std::sync::OnceLock;
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_BRUTUS")
            .ok()
            .map(|v| v.parse::<i64>().unwrap_or(0) * 1000)
    }) else {
        return;
    };
    brutus_keepaway(world, now, at);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    if at > 0 && (at..at + tick).contains(&now) {
        with_vm(world, |vm, world| {
            let level = vm.level;
            vm.notify_str(world, level, "spawn_brutus", &[ScriptValue::Int(1)]);
        });
        diag::info!(
            Sim,
            "bo2zm t6 brutus: spawn_brutus notified at {}s",
            now / 1000
        );
    }
    if now % 2000 >= tick {
        return;
    }
    let me = super::frame(world)
        .avatar(ClientId(0))
        .map_or([0.0; 3], |p| p.origin);
    let rows: Vec<String> = with_vm(world, |vm, world| {
        let objs: Vec<(u32, gsc_t6::ObjRef)> = {
            let zm = world.resource::<Zm>();
            zm.actors
                .iter()
                .where_value(|lhs| lhs.living)
                .filter_map(|(nn, _)| Some((*nn, zm.ents.get(nn)?.obj?)))
                .collect()
        };
        let mut out = Vec::new();
        for (nn, o) in objs {
            let k = vm.intern("is_brutus");
            if !gsc_t6::truthy(&vm.raw_field(o, k)) {
                continue;
            }
            let mut fields = Vec::new();
            for name in [
                "health",
                "has_helmet",
                "ai_state",
                "priority_item",
                "goal_pos",
                "not_interruptable",
            ] {
                let k = vm.intern(name);
                let mut v = vm.get_field(world, o, k);
                // An item (box, perk machine, table...): where it is.
                if let ScriptValue::Object(item) = v
                    && name == "priority_item"
                {
                    let k = vm.intern("origin");
                    v = vm.get_field(world, item, k);
                }
                fields.push(format!("{name} {}", vm.to_text(&v)));
            }
            // His way to the player: BO2's get_closest_player_using_paths
            // reads lhs 0 `calcpathlength` past 36 units as no way there and
            // skips the player, and with none he stands idle.
            let from = world.resource::<Zm>().ents.get(&nn).map_or([0.0; 3], |e| e.origin);
            let (r, h) = world.resource::<Zm>().actors.get(&nn).map_or((15.0, 72.0), |lhs| (lhs.reach, lhs.height));
            let path = super::actors::path_length(world, from, me, r, h);
            // The old way (each end the single nearest node): would it have
            // found one? When not: why (the cut links, the step limit, or no
            // link at all), the first 3 times.
            let old = {
                let nav = &world.resource::<Zm>().nav;
                nav.nearest(from).zip(nav.nearest(me)).and_then(|(lhs, b)| nav.path(lhs, b)).is_some()
            };
            fields.push(format!("oldway {}", if old { "yes" } else { "none" }));
            static NONE: AtomicI64 = AtomicI64::new(0);
            if (path.is_none() || !old) && NONE.fetch_add(1, Ordering::Relaxed) < 3 {
                let p = path_probe(world, from, me);
                diag::info!(Sim, "bo2zm t6 brutus: no path, probe {p}");
            }
            let nav = &world.resource::<Zm>().nav;
            let node = |p: [f32; 3]| {
                nav.nearest(p).map_or("none".to_owned(), |i| {
                    let o = nav.graph[i as usize].origin;
                    format!("{i} ({:.0} {:.0} {:.0})", o[0], o[1], o[2])
                })
            };
            fields.push(format!(
                "path {} nodes {} -> {}",
                path.map_or("none".to_owned(), |l| format!("{l:.0}")),
                node(from),
                node(me)
            ));
            let zm = world.resource::<Zm>();
            let (Some(e), Some(lhs)) = (zm.ents.get(&nn), zm.actors.get(&nn)) else {
                continue;
            };
            let d = ((e.origin[0] - me[0]).powi(2) + (e.origin[1] - me[1]).powi(2)).sqrt();
            out.push(format!(
                "ent{nn} at ({:.0} {:.0} {:.0}) dist {d:.0} {} {} {}",
                e.origin[0],
                e.origin[1],
                e.origin[2],
                lhs.script,
                lhs.playing.as_ref().map_or("-", |p| p.anim.as_str()),
                fields.join(" ")
            ));
        }
        out
    })
    .unwrap_or_default();
    // When lhs Brutus goes: his points (BO2 brutus_death: 500 to the team,
    // 250 more to the killer) and the power-up he drops.
    static SEEN: AtomicI64 = AtomicI64::new(-1);
    static SCORE: AtomicI64 = AtomicI64::new(0);
    let score = i64::from(super::frame(world).client_meta(ClientId(0)).map_or(0, |m| m.score));
    let seen = SEEN.load(Ordering::Relaxed);
    if rows.is_empty() && seen >= 0 {
        let drops = with_vm(world, |vm, world| {
            let k = vm.intern("active_powerups");
            let ScriptValue::Array(lhs) = vm.raw_field(vm.level, k) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for v in lhs.snapshot().values_in_order() {
                let ScriptValue::Object(o) = *v else { continue };
                let k = vm.intern("powerup_name");
                let name = vm.get_field(world, o, k);
                let k = vm.intern("origin");
                let at = vm.get_field(world, o, k);
                out.push(format!("{} at {}", vm.to_text(&name), vm.to_text(&at)));
            }
            out
        })
        .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 brutus: ent{seen} gone by {}s: points {} -> {score}, power-ups on the ground {drops:?}",
            now / 1000,
            SCORE.load(Ordering::Relaxed)
        );
        SEEN.store(-1, Ordering::Relaxed);
    }
    if let Some(nn) = rows.first().and_then(|r| r.split_whitespace().next()) {
        SEEN.store(nn.trim_start_matches("ent").parse().unwrap_or(0), Ordering::Relaxed);
        SCORE.store(score, Ordering::Relaxed);
    }
    for r in rows {
        diag::info!(Sim, "bo2zm t6 brutus at {}s: {r}", now / 1000);
    }
}

/// BO2MP_T6_ZONEPROBE=1: every 5 s, the zone volumes (`level.zones[*].volumes`)
/// within 1500 units of the player: entity, brush model, origin, the brush
/// model's bounds and whether `istouching` would see the player in it.
pub(crate) fn zone_probe(world: &mut World, now: i64) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_ZONEPROBE").is_some()) {
        return;
    }
    if now % 5000 >= i64::from(crate::SIM_STEP_MILLIS) {
        return;
    }
    let Some((pmins, pmaxs)) = super::triggers::player_box(world, 0) else {
        return;
    };
    let me: [f32; 3] = std::array::from_fn(|i| (pmins[i] + pmaxs[i]) * 0.5);
    let ents = zone_volumes(world);
    let mut table_rows = Vec::new();
    for (zone, n, e) in ents {
        let bounds = super::triggers::brush_index(&e).and_then(|b| {
            super::frame(world)
                .collision_submodel_set()
                .models
                .get(b as usize)
                .map(|m| (m.lo, m.hi, m.solid_volume_total))
        });
        let near = match bounds {
            Some((lo, hi, _)) => (0..3).all(|i| {
                me[i] > lo[i].min(e.origin[i] + lo[i]) - 1500.0
                    && me[i] < hi[i].max(e.origin[i] + hi[i]) + 1500.0
            }),
            None => true,
        };
        if !near {
            continue;
        }
        let hit = super::triggers::touches(world, &e, pmins, pmaxs);
        table_rows.push(format!(
            "{zone} ent{n} {} brush {:?} origin ({:.0} {:.0} {:.0}) bounds {:?} touch {hit}",
            e.class_tag,
            e.brush,
            e.origin[0],
            e.origin[1],
            e.origin[2],
            bounds.map(|(lo, hi, nb)| format!(
                "({:.0} {:.0} {:.0})..({:.0} {:.0} {:.0}) brushes {nb}",
                lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
            )),
        ));
    }
    diag::info!(
        Sim,
        "bo2zm t6 zoneprobe at {}s: player ({:.0} {:.0} {:.0}), {} volumes near",
        now / 1000,
        me[0],
        me[1],
        me[2],
        table_rows.len()
    );
    for r in table_rows {
        diag::info!(Sim, "bo2zm t6 zoneprobe: {r}");
    }
}

/// Every zone volume (`level.zones[*].volumes`): (zone, entnum, entity).
fn zone_volumes(world: &mut World) -> Vec<(String, u32, super::Ent)> {
    let vols: Vec<(String, gsc_t6::ObjRef)> = with_vm(world, |vm, _| {
        let k = vm.intern("zones");
        let ScriptValue::Array(zones) = vm.raw_field(vm.level, k) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let zones = zones.snapshot();
        for key in zones.keys() {
            let Some(ScriptValue::Object(z)) = zones.get(&key) else {
                continue;
            };
            let name = vm.to_text(&key.value());
            let k = vm.intern("volumes");
            if let ScriptValue::Array(v) = vm.raw_field(*z, k) {
                for e in v.snapshot().values_in_order() {
                    if let ScriptValue::Object(e) = e {
                        out.push((name.clone(), *e));
                    }
                }
            }
        }
        out
    })
    .unwrap_or_default();
    {
        let zm = world.resource::<Zm>();
        vols.iter()
            .filter_map(|(z, o)| {
                let (n, e) = zm.ents.iter().find(|(_, e)| e.obj == Some(*o))?;
                Some((z.clone(), *n, e.clone()))
            })
            .collect()
    }
}

/// Why `findpath(from, to)` fails, for the log: the nearest node at each end,
/// the path with the links doors and debris cut now, without them, and
/// which entity's cut links block it (one `connect` at a time).
fn path_probe(game: &World, from: [f32; 3], dest: [f32; 3]) -> String {
    let zm = game.resource::<Zm>();
    let nav = &zm.nav;
    let (Some(a), Some(b)) = (nav.nearest(from), nav.nearest(dest)) else {
        return "no nearest node".to_owned();
    };
    let at = |n: u32| {
        let o = nav.graph[n as usize].origin;
        format!("node{n} ({:.0} {:.0} {:.0})", o[0], o[1], o[2])
    };
    let cut = nav.path(a, b).map(|p| p.len());
    let mut open = nav.clone();
    let ents: Vec<u32> = zm.ents.keys().copied().collect();
    for &e in &ents {
        open.connect(e);
    }
    let uncut = open.path(a, b).map(|p| p.len());
    // A full walk with no step limit (the search above gives up after 4096
    // steps): can Brutus's side reach the player's side at all, and if not,
    // the reached spot nearest the player (where the route breaks).
    let mut seen = vec![false; open.graph.len()];
    let mut stack = vec![a];
    seen[a as usize] = true;
    while let Some(n) = stack.pop() {
        for &(next, _, _) in &open.graph[n as usize].links {
            let next = usize::from(next);
            if next < seen.len() && !seen[next] {
                seen[next] = true;
                stack.push(next as u32);
            }
        }
    }
    let reached = seen.iter().filter(|s| **s).count();
    let goal = open.graph[b as usize].origin;
    let near = (0..seen.len())
        .filter(|&i| seen[i])
        .map(|i| {
            let o = open.graph[i].origin;
            let d = ((o[0] - goal[0]).powi(2) + (o[1] - goal[1]).powi(2) + (o[2] - goal[2]).powi(2)).sqrt();
            (i, d)
        })
        .min_by(|x, y| x.1.total_cmp(&y.1));
    let walk = format!(
        "full walk reaches {reached} of {} nodes, player's node {}, nearest reached {}",
        open.graph.len(),
        if seen[b as usize] { "reached" } else { "NOT reached" },
        near.map_or("none".to_owned(), |(i, d)| format!("{} at {d:.0}", at(i as u32)))
    );
    let mut blockers = Vec::new();
    if cut.is_none() && uncut.is_some() {
        for &e in &ents {
            let mut one = nav.clone();
            let before = one.cut_count();
            one.connect(e);
            if one.cut_count() == before || one.path(a, b).is_none() {
                continue;
            }
            let Some(en) = zm.ents.get(&e) else {
                continue;
            };
            blockers.push(format!(
                "ent{e} {} {:?} at ({:.0} {:.0} {:.0}) cuts {}",
                en.class_tag,
                en.brush,
                en.origin[0],
                en.origin[1],
                en.origin[2],
                before - one.cut_count()
            ));
        }
    }
    format!(
        "from {} to {}: path {cut:?} nodes, with no cuts {uncut:?}, {} links cut, opened by {blockers:?}; {walk}",
        at(a),
        at(b),
        nav.cut_count()
    )
}

/// The first living Brutus: (entnum, origin).
fn brutus_pos(world: &mut World) -> Option<(u32, [f32; 3])> {
    with_vm(world, |vm, world| {
        let k = vm.intern("is_brutus");
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, _)| Some((*nn, zm.ents.get(nn)?)))
            .find(|(_, en)| en.obj.is_some_and(|o| gsc_t6::truthy(&vm.raw_field(o, k))))
            .map(|(nn, en)| (nn, en.origin))
    })
    .flatten()
}

/// Something Brutus locked: a map entity (perk machine trigger...) or a
/// craftable table's use trigger, a struct in `level.a_uts_craftables`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Locked {
    Ent(u32),
    Uts(usize),
}

/// One thing locked now: what it is, its name, where it is and its `locked_cost`.
struct LockedThing {
    what: Locked,
    desc: String,
    at: [f32; 3],
    locked_cost: String,
}

/// Everything locked now (`is_locked`).
fn locked_things(game: &mut World) -> Vec<LockedThing> {
    let (ents, uts) = with_vm(game, |vm, game| {
        let (il, lc, tg) = (
            vm.intern("is_locked"),
            vm.intern("locked_cost"),
            vm.intern("target"),
        );
        let zm = game.resource::<Zm>();
        let mut ents: Vec<(u32, String, String)> = zm
            .ents
            .iter()
            .filter_map(|(n, e)| {
                let o = e.obj?;
                gsc_t6::truthy(&vm.raw_field(o, il)).then(|| {
                    (
                        *n,
                        vm.to_text(&vm.raw_field(o, tg)),
                        vm.to_text(&vm.raw_field(o, lc)),
                    )
                })
            })
            .collect();
        ents.sort();
        let mut uts = Vec::new();
        let k = vm.intern("a_uts_craftables");
        if let ScriptValue::Array(a) = vm.raw_field(vm.level, k) {
            let (org, eq) = (vm.intern("origin"), vm.intern("equipname"));
            for (i, v) in a.snapshot().values_in_order().enumerate() {
                let ScriptValue::Object(o) = v else {
                    continue;
                };
                if !gsc_t6::truthy(&vm.raw_field(*o, il)) {
                    continue;
                }
                let at = match vm.raw_field(*o, org) {
                    ScriptValue::Vec3(p) => p,
                    _ => [0.0; 3],
                };
                uts.push((
                    i,
                    format!(
                        "table {} {}",
                        vm.to_text(&vm.raw_field(*o, tg)),
                        vm.to_text(&vm.raw_field(*o, eq))
                    ),
                    at,
                    vm.to_text(&vm.raw_field(*o, lc)),
                ));
            }
        }
        (ents, uts)
    })
    .unwrap_or_default();
    let mut out = Vec::new();
    for (n, t, c) in ents {
        let Some(e) = game.resource::<Zm>().ents.get(&n).cloned() else {
            continue;
        };
        let at = super::triggers::center(game, &e);
        out.push(LockedThing { what: Locked::Ent(n), desc: t, at, locked_cost: c });
    }
    for (i, t, at, c) in uts {
        out.push(LockedThing { what: Locked::Uts(i), desc: t, at, locked_cost: c });
    }
    out
}

/// BO2MP_T6_KEEPAWAY=<zone> (with BO2MP_T6_BRUTUS): from the Brutus notify on,
/// every 0.5 s, when Brutus is nearer than 200 or farther than 480 units the
/// player is put on the path node in that zone's volumes nearest to 320
/// units from him: outside his 128 aggro reach but inside the 512 past which
/// `watch_for_player_dist` idles him, so he goes for an item in the player's
/// zone and locks it. Every 2 s each locked thing (`is_locked`: a perk
/// machine trigger, a craftable table's trigger) is logged with its
/// `locked_cost`; 5 s after the first lock the player stops keeping away,
/// gets 5000 points, stands 40 units from it facing it and holds Use for 1 s
/// (paying unlocks it); at +9 s its state and his points are logged.
fn brutus_keepaway(world: &mut World, now_ms: i64, at: i64) {
    use std::sync::{Mutex, OnceLock};
    static ZONE: OnceLock<Option<String>> = OnceLock::new();
    static NODES: Mutex<Option<Vec<[f32; 3]>>> = Mutex::new(None);
    static LOCKED: AtomicI64 = AtomicI64::new(i64::MIN);
    static TARGET: Mutex<Option<(Locked, [f32; 3])>> = Mutex::new(None);
    let Some(zone) = ZONE
        .get_or_init(|| std::env::var("BO2MP_T6_KEEPAWAY").ok())
        .clone()
    else {
        return;
    };
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let client = ClientId(0);
    if now_ms < at {
        return;
    }
    let d2 = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let locked_at = LOCKED.load(Ordering::Relaxed);
    if now_ms % 2000 < tick {
        let locks = locked_things(world);
        for LockedThing { what, desc: t, at: c, locked_cost: cost } in &locks {
            diag::info!(
                Sim,
                "bo2zm t6 keepaway at {}s: {what:?} {t:?} at ({:.0} {:.0} {:.0}) locked, cost {cost}",
                now_ms / 1000,
                c[0],
                c[1],
                c[2]
            );
        }
        if locked_at == i64::MIN
            && let Some(LockedThing { what, at: c, .. }) = locks.first()
        {
            LOCKED.store(now_ms, Ordering::Relaxed);
            *TARGET.lock().unwrap() = Some((*what, *c));
        }
    }
    if locked_at != i64::MIN {
        let rel = now_ms - locked_at;
        let Some((what, c)) = *TARGET.lock().unwrap() else {
            return;
        };
        if (5000..5000 + tick).contains(&rel) {
            super::autoplay::give_points(world, 5000);
            let points = super::frame(world)
                .client_meta(client)
                .map_or(0, |m| m.score);
            diag::info!(Sim, "bo2zm t6 keepaway: points before unlock {points}");
            // 40 units from it on the player's side, feet on the floor.
            let me = super::frame(world).avatar(client).map_or(c, |p| p.origin);
            let (step_along_x, dy) = (me[0] - c[0], me[1] - c[1]);
            let len = (step_along_x * step_along_x + dy * dy).sqrt().max(1.0);
            let stand = [c[0] + step_along_x / len * 40.0, c[1] + dy / len * 40.0, c[2] + 40.0];
            let down = [stand[0], stand[1], stand[2] - 300.0];
            let tr = super::frame(world).trace_static_world(
                stand,
                down,
                crate::shot_trace::BODY_BOX_LOWER,
                crate::shot_trace::BODY_BOX_UPPER,
                crate::shot_trace::PLAYER_BODY_MASK,
            );
            let feet: [f32; 3] =
                std::array::from_fn(|k| stand[k] + (down[k] - stand[k]) * tr.fraction);
            super::teleport_player(world, client, feet);
            diag::info!(
                Sim,
                "bo2zm t6 keepaway: unlock {what:?}, player at ({:.0} {:.0} {:.0})",
                feet[0],
                feet[1],
                feet[2]
            );
        }
        if (5000..7000).contains(&rel) {
            if let Some(origin) = super::frame(world).avatar(client).map(|p| p.origin) {
                let view = super::autoplay::look([origin[0], origin[1], origin[2] + 60.0], c);
                super::set_player_view(world, client, view);
            }
            if (5500..6500).contains(&rel) {
                let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
                let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
                for (_, cmd) in mine {
                    cmd.buttons |= buttons::INTERACT_KEY;
                }
            }
        }
        if (9000..9000 + tick).contains(&rel) {
            let still = locked_things(world).iter().any(|lt| lt.what == what);
            let points = super::frame(world)
                .client_meta(client)
                .map_or(0, |m| m.score);
            diag::info!(
                Sim,
                "bo2zm t6 keepaway: after unlock {what:?}: still locked {still}, points {points}"
            );
        }
        return;
    }
    if now_ms % 500 >= tick {
        return;
    }
    let Some((bn, b)) = brutus_pos(world) else {
        return;
    };
    let Some(me) = super::frame(world).avatar(client).map(|p| p.origin) else {
        return;
    };
    if (200.0..480.0).contains(&d2(me, b)) {
        return;
    }
    let mut nodes = NODES.lock().unwrap();
    if nodes.is_none() {
        let vols: Vec<_> = zone_volumes(world)
            .into_iter()
            .filter(|(z, _, _)| *z == zone)
            .collect();
        let all: Vec<[f32; 3]> = world
            .resource::<Zm>()
            .nav
            .graph
            .iter()
            .map(|n| n.origin)
            .collect();
        let (mins, maxs) = (
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
        );
        let inside: Vec<[f32; 3]> = all
            .into_iter()
            .filter(|o| {
                let lo = std::array::from_fn(|i| o[i] + mins[i]);
                let hi = std::array::from_fn(|i| o[i] + maxs[i]);
                vols.iter()
                    .any(|(_, _, e)| super::triggers::touches(world, e, lo, hi))
            })
            .collect();
        diag::info!(
            Sim,
            "bo2zm t6 keepaway: {} path nodes in {zone} ({} volumes)",
            inside.len(),
            vols.len()
        );
        *nodes = Some(inside);
    }
    let Some(best) = nodes.as_ref().and_then(|v| {
        v.iter().copied().min_by(|x, y| {
            (d2(*x, b) - 320.0)
                .abs()
                .total_cmp(&(d2(*y, b) - 320.0).abs())
        })
    }) else {
        return;
    };
    drop(nodes);
    let to = [best[0], best[1], best[2] + 2.0];
    super::teleport_player(world, client, to);
    // BO2's stuck watcher runs findpath(Brutus, his goal) once a second and
    // teleports him after 4 fails; why ours fails, the first 3 times.
    static PROBES: AtomicI64 = AtomicI64::new(0);
    if PROBES.fetch_add(1, Ordering::Relaxed) < 3 {
        let p = path_probe(world, b, to);
        diag::info!(Sim, "bo2zm t6 keepaway: path probe {p}");
    }
    diag::info!(
        Sim,
        "bo2zm t6 keepaway at {:.1}s: Brutus ent{bn} at ({:.0} {:.0} {:.0}) {:.0} away; player to ({:.0} {:.0} {:.0}), {:.0} away",
        now_ms as f64 / 1000.0,
        b[0],
        b[1],
        b[2],
        d2(me, b),
        to[0],
        to[1],
        to[2],
        d2(to, b)
    );
}

/// BO2MP_T6_GONDOLA=1: every 2 s logs the gondola (`level.e_gondola`): where
/// it is, its flags (`gondola_at_roof`, `gondola_at_docks`,
/// `gondola_in_motion`, `gondola_initialized`) and where the player is.
/// Shocking `gondola_powered_on_docks` (BO2MP_T6_SHOCK) brings it down to the
/// docks by itself (`init_alcatraz_zipline`). 6 s after it first reads
/// `gondola_at_docks` the player gets 2000 points, stands in the docks'
/// `gondola_move_trigger` and holds Use for 1 s (750 to ride it back up);
/// the 2 s lines then show whether he rides up with it.
pub(crate) fn gondola_test(game: &mut World, now_ms: i64) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    static DOCKS: AtomicI64 = AtomicI64::new(i64::MIN);
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_T6_GONDOLA").is_some()) {
        return;
    }
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let client = ClientId(0);
    let flags = |game: &mut World| {
        with_vm(game, |vm, _| {
            let f = vm.intern("flag");
            let ScriptValue::Array(a) = vm.raw_field(vm.level, f) else {
                return Vec::new();
            };
            [
                "gondola_at_roof",
                "gondola_at_docks",
                "gondola_in_motion",
                "gondola_initialized",
            ]
            .iter()
            .filter(|nn| {
                a.get(&gsc_t6::Key::Str(vm.intern(nn)))
                    .is_some_and(|v| gsc_t6::truthy(&v))
            })
            .map(|nn| nn.to_string())
            .collect::<Vec<_>>()
        })
        .unwrap_or_default()
    };
    // The docks' move trigger: (entnum, entity).
    let move_trigger = |game: &mut World| {
        with_vm(game, |vm, game| {
            let (tn, ss) = (vm.intern("targetname"), vm.intern("script_string"));
            let zm = game.resource::<Zm>();
            zm.ents
                .iter()
                .find(|(_, e)| {
                    e.obj.is_some_and(|o| {
                        vm.to_text(&vm.raw_field(o, tn)) == "gondola_move_trigger"
                            && vm.to_text(&vm.raw_field(o, ss)) == "docks"
                    })
                })
                .map(|(nn, e)| (*nn, e.clone()))
        })
        .flatten()
    };
    let docks_at = DOCKS.load(Ordering::Relaxed);
    if now_ms % 2000 < tick {
        let on = flags(game);
        let gondola = with_vm(game, |vm, game| {
            let k = vm.intern("e_gondola");
            let ScriptValue::Object(g) = vm.raw_field(vm.level, k) else {
                return None;
            };
            let zm = game.resource::<Zm>();
            zm.ents
                .values()
                .find(|e| e.obj == Some(g))
                .map(|e| e.origin)
        })
        .flatten();
        let me = super::frame(game)
            .avatar(client)
            .map_or([0.0; 3], |p| p.origin);
        diag::info!(
            Sim,
            "bo2zm t6 gondola at {}s: {} flags [{}]; player at ({:.0} {:.0} {:.0})",
            now_ms / 1000,
            gondola.map_or("none".into(), |g| format!(
                "at ({:.0} {:.0} {:.0})",
                g[0], g[1], g[2]
            )),
            on.join(" "),
            me[0],
            me[1],
            me[2]
        );
        if docks_at == i64::MIN && on.iter().any(|f| f == "gondola_at_docks") {
            DOCKS.store(now_ms, Ordering::Relaxed);
        }
    }
    if docks_at == i64::MIN {
        return;
    }
    let rel = now_ms - docks_at;
    if (6000..6000 + tick).contains(&rel) {
        let Some((nn, trig)) = move_trigger(game) else {
            diag::info!(Sim, "bo2zm t6 gondola: no docks move trigger");
            return;
        };
        super::autoplay::give_points(game, 2000);
        let c = super::triggers::center(game, &trig);
        let stand = [c[0], c[1], c[2] + 20.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let tr = super::frame(game).trace_static_world(
            stand,
            down,
            crate::shot_trace::BODY_BOX_LOWER,
            crate::shot_trace::BODY_BOX_UPPER,
            crate::shot_trace::PLAYER_BODY_MASK,
        );
        let feet: [f32; 3] = std::array::from_fn(|k| stand[k] + (down[k] - stand[k]) * tr.fraction);
        super::teleport_player(game, client, feet);
        let hint = game
            .resource::<Zm>()
            .ents
            .get(&nn)
            .map(|e| format!("{:?}", e.hint))
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 gondola: ride, move trigger ent{nn} at ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0}), hint {hint}",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (6500..7500).contains(&rel) {
        let mut req = game.resource_mut::<crate::frame_step::FrameTicket>();
        let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
        for (_, cmd) in mine {
            cmd.buttons |= buttons::INTERACT_KEY;
        }
    }
}

/// BO2MP_T6_GHOSTDOOR=<brush model, e.g. *14 the isolation cell door; `any`>:
/// Afterlife's ghost doors (`afterlife_door`, hidden from a ghost by
/// `afterlife_doors_open`). Once he holds the lightning hands (a ghost) he
/// is put 60 units to one side of that door, facing across it, and walks
/// 3 s; then he is put by his body and revived (6-16 s after the hands); at
/// 20 s, alive, he is put at the same spot and walks 3 s again. Each walk
/// logs where it ended, whether he crossed the door and whether the door is
/// hidden from him (a ghost crosses, the living are stopped).
pub(crate) fn ghost_door_test(game: &mut World, now_ms: i64) {
    use std::sync::OnceLock;
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("BO2MP_T6_GHOSTDOOR").ok())
        .clone()
    else {
        return;
    };
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    let client = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let mut start = START.load(Ordering::Relaxed);
    if start == i64::MIN {
        let held = super::frame(game).avatar(client).map_or(0, |p| p.weapon);
        if held == 0 || super::weapon_text(game, held) != "lightning_hands_zm" {
            return;
        }
        start = now_ms;
        START.store(now_ms, Ordering::Relaxed);
    }
    let rel = now_ms - start;
    if (6000 + tick..16000).contains(&rel) {
        revive_at_body(game, rel, 6000);
        return;
    }
    let Some(w0) = [1000i64, 20000]
        .into_iter()
        .find(|t| (*t..*t + 3600 + tick).contains(&rel))
    else {
        return;
    };
    // BO2's ghost doors: targetname "afterlife_door" (_zm_afterlife
    // afterlife_doors_open hides them from a ghost).
    let doors: Vec<(u32, super::Ent)> = with_vm(game, |vm, game| {
        let tn = vm.intern("targetname");
        game
            .resource::<Zm>()
            .ents
            .iter()
            .where_value(|en| en.obj.is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)) == "afterlife_door"))
            .map(|(nn, en)| (*nn, en.clone()))
            .collect()
    })
    .unwrap_or_default();
    // A door's `*N` is its brush, not its model.
    let Some((nn, en)) = doors
        .iter()
        .find(|(_, en)| en.brush.as_deref() == Some(want.as_str()) || en.mesh == want || want == "any")
        .cloned()
    else {
        if (w0..w0 + tick).contains(&rel) {
            let all: Vec<String> = doors
                .iter()
                .map(|(nn, en)| {
                    format!(
                        "ent{nn} {} {} {} at ({:.0} {:.0} {:.0})",
                        en.class_tag,
                        en.brush.as_deref().unwrap_or(""),
                        en.mesh,
                        en.origin[0],
                        en.origin[1],
                        en.origin[2]
                    )
                })
                .collect();
            diag::info!(Sim, "bo2zm t6 ghost door: no {want} among the afterlife doors {all:?}");
        }
        return;
    };
    let Some((low, hi)) = super::brushes::world_box(game, &en) else {
        return;
    };
    let c: [f32; 3] = std::array::from_fn(|i| (low[i] + hi[i]) * 0.5);
    // Across the door: its thin side.
    let ax = usize::from(hi[1] - low[1] < hi[0] - low[0]);
    let mut spot = [c[0], c[1], low[2] + 2.0];
    spot[ax] -= 60.0;
    let yaw = if ax == 0 { 0.0 } else { 90.0 };
    if (w0..w0 + tick).contains(&rel) {
        super::teleport_player(game, client, spot);
        super::set_player_view(game, client, [0.0, yaw, 0.0]);
        return;
    }
    if (w0 + 300..w0 + 3300).contains(&rel) {
        super::set_player_view(game, client, [0.0, yaw, 0.0]);
        let mut req = game.resource_mut::<crate::frame_step::FrameTicket>();
        let mine = req.input.cmds.iter_mut().filter(|(who, _)| *who == client);
        for (_, cmd) in mine {
            cmd.thrust_ahead = 127;
        }
        return;
    }
    if (w0 + 3500..w0 + 3500 + tick).contains(&rel) {
        let o = super::frame(game).avatar(client).map_or([0.0; 3], |p| p.origin);
        let ghost = afterlife_corpse(game, client).is_some();
        let hidden = en.invisible_to.contains(&0);
        diag::info!(
            Sim,
            "bo2zm t6 ghost door ent{nn} {want} at ({:.0} {:.0} {:.0}): ghost {ghost}, hidden from him {hidden}, \
             from ({:.0} {:.0}) walked to ({:.0} {:.0} {:.0}), crossed {}",
            c[0],
            c[1],
            c[2],
            spot[0],
            spot[1],
            o[0],
            o[1],
            o[2],
            o[ax] > c[ax]
        );
    }
}
