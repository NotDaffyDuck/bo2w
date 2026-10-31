//! Triggers: touch triggers (`trigger_multiple`, `trigger_radius`) notify
//! `trigger` with the player every frame he is inside (and with zombies when
//! the spawnflags take AI); use triggers
//! (`trigger_use`, `trigger_use_touch`, `trigger_radius_use`) when he
//! presses use inside the one he faces best; hurt triggers (`trigger_hurt`)
//! hurt him while he is inside. A trigger hidden from a player
//! (`setinvisibletoplayer`) ignores him.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue};

use super::{Ent, Zm, frame, with_vm};
use crate::shot_trace::{BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::gsc_host::{Volume, t6_volume};
use crate::sim_parts::ClientId;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Touch,
    Use,
}

/// The triggers of one kind, in their order.
fn of_kind(triggers: &[(u32, Ent, Kind)], want: Kind) -> impl Iterator<Item = &(u32, Ent, Kind)> {
    triggers.iter().filter(move |t| t.2 == want)
}

pub(super) fn kind(classname: &str) -> Option<Kind> {
    match classname {
        "trigger_multiple" | "trigger_radius" | "trigger_once" | "trigger_box" => Some(Kind::Touch),
        "trigger_use" | "trigger_use_touch" | "trigger_radius_use" | "trigger_box_use" => {
            Some(Kind::Use)
        }
        _ => None,
    }
}

pub(super) fn brush_index(e: &Ent) -> Option<u32> {
    e.brush.as_deref()?.strip_prefix('*')?.parse().ok()
}

pub(super) fn cylinder(e: &Ent) -> Option<(f32, f32)> {
    (e.reach > 0.0).then_some((e.reach, e.height.max(1.0)))
}

/// Does `e`'s volume touch the box `mins..maxs`?
pub(super) fn touches(game: &mut World, en: &Ent, low: [f32; 3], high: [f32; 3]) -> bool {
    // A turned box: the other box's centre against it, grown by that box's
    // reach along each of its axes.
    if let Some([w, l, h]) = en.box_dims {
        let (f, r, u) = gsc_t6::math::facing_axes(en.rot);
        let c: [f32; 3] = std::array::from_fn(|ix| (low[ix] + high[ix]) * 0.5 - en.origin[ix]);
        let half_ext: [f32; 3] = std::array::from_fn(|ix| (high[ix] - low[ix]) * 0.5);
        let reach =
            |a: [f32; 3]| a[0].abs() * half_ext[0] + a[1].abs() * half_ext[1] + a[2].abs() * half_ext[2];
        let dot = |a: [f32; 3]| a[0] * c[0] + a[1] * c[1] + a[2] * c[2];
        return dot(f).abs() <= w * 0.5 + reach(f)
            && dot(r).abs() <= l * 0.5 + reach(r)
            && dot(u).abs() <= h * 0.5 + reach(u);
    }
    let f = frame(game);
    match t6_volume(&f, en.origin, cylinder(en), brush_index(en)) {
        // A brush trigger turned since the map placed it (linked to the
        // Tranzit bus) turns its brushes with it.
        Some(v) => match en.birth_orientation.filter(|a| *a != en.rot) {
            Some(spawn) => v.touches_turned(low, high, turn_between(spawn, en.rot)),
            None => v.touches(low, high),
        },
        None => false,
    }
}

/// Does a trigger faced at `score` (low priority or not) win over the best
/// so far (its score and priority)? A normal priority trigger beats a low
/// one whatever their scores; among equals the better faced wins.
fn hint_beats(score: f32, low: bool, best: Option<(f32, bool)>) -> bool {
    best.is_none_or(|(b_score, b_low)| (!low && b_low) || (low == b_low && score > b_score))
}

/// The rotation from angles `from` to angles `to` (row i = world axis i):
/// R(to) * R(from)^T, R's columns being forward, left and up.
fn turn_between(from: [f32; 3], goal: [f32; 3]) -> [[f32; 3]; 3] {
    let (f0, r0, u0) = gsc_t6::math::facing_axes(from);
    let (f1, r1, u1) = gsc_t6::math::facing_axes(goal);
    std::array::from_fn(|i| std::array::from_fn(|j| f1[i] * f0[j] + r1[i] * r0[j] + u1[i] * u0[j]))
}

/// A look-at box trigger (every wall buy, the box, the craft tables): BO2
/// offers it when the player's body touches the turned box and he faces its
/// centre (dot 0.76). No use radius and no line of sight. The body is the
/// upright line through the middle of his box, 15 in from top and bottom,
/// grown 15 all round; the box grows 15 each way to meet it. Our old test
/// asked his view to land on the gun's own box within reach, so a chalk
/// whose box came out thin never offered (the Bus Depot shotgun).
fn box_use_reach(en: &Ent, low: [f32; 3], high: [f32; 3], eye_at: [f32; 3], fwd: [f32; 3]) -> bool {
    let Some([w, l, h]) = en.box_dims else {
        return false;
    };
    let r = (high[0] - low[0]).min(high[1] - low[1]) * 0.5;
    let centre: [f32; 3] = std::array::from_fn(|ix| (low[ix] + high[ix]) * 0.5);
    let reach = ((high[2] - low[2]) * 0.5 - r).max(0.0);
    let (f, rt, u) = gsc_t6::math::facing_axes(en.rot);
    let rel = gsc_t6::math::sub(centre, en.origin);
    // The body line's middle and half length in the box's own axes.
    let m = [
        gsc_t6::math::dot(rel, f),
        gsc_t6::math::dot(rel, rt),
        gsc_t6::math::dot(rel, u),
    ];
    let d = [f[2] * reach, rt[2] * reach, u[2] * reach];
    let half = [w * 0.5 + r, l * 0.5 + r, h * 0.5 + r];
    for ix in 0..3 {
        if m[ix].abs() > half[ix] + d[ix].abs() {
            return false;
        }
    }
    for (a, b) in [(1, 2), (2, 0), (0, 1)] {
        if (m[a] * d[b] - m[b] * d[a]).abs() > half[a] * d[b].abs() + half[b] * d[a].abs() {
            return false;
        }
    }
    if !en.look_at {
        return true;
    }
    let v = gsc_t6::math::sub(en.origin, eye_at);
    let len = gsc_t6::math::dot(v, v).sqrt();
    len < 1e-3 || gsc_t6::math::dot(v, fwd) / len >= 0.76
}

/// Zombies use a `trigger_radius_use` like any use entity, without touching
/// it: within the use reach of its centre from the eye (BO1 SP
/// Player_FindUsableEntities: 72; BO2's _zm_unitrigger raises
/// player_useRadius_zm to its largest stub radius instead), and facing it
/// (dot 0.76) only when it asks for a look. Buried's head chopper blade on
/// the courthouse shelf: its trigger starts above the player's head.
fn radius_use_reach(world: &mut World, e: &Ent, eye: [f32; 3], fwd: [f32; 3], reach: f32) -> bool {
    if e.class_tag != "trigger_radius_use" {
        return false;
    }
    let v = gsc_t6::math::sub(center(world, e), eye);
    let d = gsc_t6::math::dot(v, v).sqrt();
    d <= reach && (!e.look_at || d < 1e-3 || gsc_t6::math::dot(v, fwd) / d >= 0.76)
}

pub(super) fn center(world: &mut World, e: &Ent) -> [f32; 3] {
    if e.box_dims.is_some() {
        return e.origin;
    }
    if cylinder(e).is_some() {
        return [e.origin[0], e.origin[1], e.origin[2] + e.height * 0.5];
    }
    let f = frame(world);
    let model = brush_index(e).and_then(|n| {
        f.collision_submodel_set()
            .models
            .get(n as usize)
            .map(|m| (m.lo, m.hi))
    });
    match model {
        Some((lo, hi)) => std::array::from_fn(|i| e.origin[i] + (lo[i] + hi[i]) * 0.5),
        None => e.origin,
    }
}

/// The world box an entity fills (the engine's absmin / absmax): its turned
/// box's, its cylinder's, or its brush model's bounds around its origin.
pub(super) fn abs_box(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    if let Some([w, l, h]) = e.box_dims {
        let half = [w * 0.5, l * 0.5, h * 0.5];
        let (f, r, u) = gsc_t6::math::facing_axes(e.rot);
        let reach: [f32; 3] =
            std::array::from_fn(|i| f[i].abs() * half[0] + r[i].abs() * half[1] + u[i].abs() * half[2]);
        return Some((
            std::array::from_fn(|i| e.origin[i] - reach[i]),
            std::array::from_fn(|i| e.origin[i] + reach[i]),
        ));
    }
    if let Some((r, h)) = cylinder(e) {
        let o = e.origin;
        return Some(([o[0] - r, o[1] - r, o[2]], [o[0] + r, o[1] + r, o[2] + h]));
    }
    let n = brush_index(e)?;
    let f = frame(world);
    let m = f.collision_submodel_set().models.get(n as usize)?;
    Some((
        std::array::from_fn(|i| e.origin[i] + m.lo[i]),
        std::array::from_fn(|i| e.origin[i] + m.hi[i]),
    ))
}

/// The boxes of the entities a script barred tactical insertions from
/// (`allowtacticalinsertion(0)`), for the gun step's
/// Player_EntityBlocksTacticalInsertion.
pub(super) fn publish_tac_insert_blockers(world: &mut World) {
    let barred: Vec<Ent> = world
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| e.no_tac_insert)
        .cloned()
        .collect();
    let boxes: Vec<([f32; 3], [f32; 3])> = barred.iter().filter_map(|e| abs_box(world, e)).collect();
    frame(world).set_tac_insert_blockers(boxes);
}

/// A brush entity's clip model bounds, as the map stores them (the
/// touch log: Die Rise's elevator volumes against Who's Who's body).
pub(super) fn brush_bounds(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    let f = frame(world);
    let n = brush_index(e)?;
    f.collision_submodel_set()
        .models
        .get(n as usize)
        .map(|m| (m.lo, m.hi))
}

/// His box for triggers and touch tests. Its top follows his stance, as BO2's
/// ClientThink_real copies the move's box onto him: 70 standing, 50 crouched,
/// 30 prone.
pub(super) fn player_box(world: &mut World, client: u32) -> Option<([f32; 3], [f32; 3])> {
    let frame = frame(world);
    let ps = frame.avatar(ClientId(client))?;
    let o = ps.origin;
    let mut maxs = BODY_BOX_UPPER;
    maxs[2] = crate::floor_guns::stance_maxs_z(ps);
    Some((
        std::array::from_fn(|i| o[i] + BODY_BOX_LOWER[i]),
        std::array::from_fn(|i| o[i] + maxs[i]),
    ))
}

/// The trigger a player faces best so far: how well he faces it, which it
/// is, its hint string and whether that hint is low priority.
struct Faced {
    points: f32,
    obj: ObjRef,
    prompt: Option<String>,
    low: bool,
}

/// Fire the frame's triggers.
pub(super) fn dispatch(world: &mut World) {
    let players: Vec<(u32, ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .where_value(|pt| pt.sessionstate == "playing")
        .map(|(c, pt)| (*c, pt.obj))
        .collect();
    let triggers: Vec<(u32, Ent, Kind)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(nn, en)| {
            kind(&en.class_tag)
                .filter(|_| en.obj.is_some() && !en.trigger_off)
                .map(|k| (*nn, en.clone(), k))
        })
        .collect();
    // The use reach: the scripts raise player_useRadius_zm to their
    // largest trigger's (64 on Nuketown).
    let use_radius = with_vm(world, |vm, _| {
        vm.dvars
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("player_useRadius_zm"))
            .and_then(|(_, v)| v.parse::<f32>().ok())
    })
    .flatten()
    .unwrap_or(64.0)
    .max(1.0);
    // bo2mp: a use trigger only its claimant (`clientclaimtrigger`) or its
    // team (`setteamfortrigger`) may use: a turret's pick-up is its owner's.
    // The claims, and each player's team.
    struct Claim {
        trigger: u32,
        by: Option<u32>,
        team: Option<String>,
    }
    let mp = world.resource::<Zm>().mp;
    let (claims, teams): (Vec<Claim>, BTreeMap<u32, String>) = if !mp {
        Default::default()
    } else {
        with_vm(world, |vm, world| {
            let (by, team) = (vm.intern("claimedby"), vm.intern("triggerteam"));
            let pteam = vm.intern("team");
            let claims = of_kind(&triggers, Kind::Use)
                .filter_map(|(nn, en, _)| {
                    let o = en.obj?;
                    let claimant = super::entnum(vm, &vm.raw_field(o, by));
                    let tt = match vm.raw_field(o, team) {
                        ScriptValue::Undefined => None,
                        v => Some(vm.to_text(&v)),
                    };
                    (claimant.is_some() || tt.is_some()).then_some(Claim { trigger: *nn, by: claimant, team: tt })
                })
                .collect();
            let teams = world
                .resource::<Zm>()
                .players
                .iter()
                .map(|(c, pt)| (*c, vm.to_text(&vm.raw_field(pt.obj, pteam))))
                .collect();
            (claims, teams)
        })
        .unwrap_or_default()
    };
    let playing: Vec<u32> = players.iter().map(|(c, _)| *c).collect();
    let mut fire: Vec<(ObjRef, ObjRef)> = Vec::new();
    for (peer, pobj) in players {
        let Some((mins, maxs)) = player_box(world, peer) else {
            continue;
        };
        let held = crate::client_script::buttons(&mut frame(world), ClientId(peer))
            & (playerstate_t6::game::buttons::INTERACT_KEY | playerstate_t6::game::buttons::INTERACT_RELOAD_KEY)
            != 0;
        let pressed = {
            let mut zm = world.resource_mut::<Zm>();
            let was = zm.use_held.contains(&peer);
            if held {
                zm.use_held.insert(peer);
            } else {
                zm.use_held.remove(&peer);
            }
            held && !was
        };
        let (eye, fwd) = {
            let f = frame(world);
            let ps = f.avatar(ClientId(peer));
            let eye = ps.map_or([0.0; 3], |pt| {
                [
                    pt.origin[0],
                    pt.origin[1],
                    pt.origin[2] + pt.eye_height_now,
                ]
            });
            let fwd = gsc_t6::math::facing_axes(ps.map_or([0.0; 3], |pt| pt.look_angles)).0;
            (eye, fwd)
        };
        // In a vehicle he has no use hint and uses nothing (BO2's
        // Player_UpdateCursorHints and the use scan both stop while
        // eFlags 0x4000 is set).
        let in_vehicle = super::killstreaks::riding(world, peer);
        // The faced trigger: (points, trigger, hint, low priority). BO2 offers a
        // low priority one (`sethintlowpriority`: every buyable door) only
        // when no other is there to use; among equals, the best faced.
        let mut best: Option<Faced> = None;
        let beats = |points: f32, low: bool, best: &Option<Faced>| {
            hint_beats(points, low, best.as_ref().map(|b| (b.points, b.low)))
        };
        let my_team = teams.get(&peer).map(String::as_str);
        for (nn, en, k) in &triggers {
            if in_vehicle && *k == Kind::Use {
                continue;
            }
            // (A claim by someone no longer playing holds nothing.)
            if let Some(Claim { by: claimant, team, .. }) = claims.iter().find(|cl| cl.trigger == *nn)
                && (claimant.is_some_and(|c| c != peer && playing.contains(&c))
                    || team.as_deref().is_some_and(|tt| Some(tt) != my_team))
            {
                continue;
            }
            if en.invisible_to.contains(&peer) || en.invisible_to_all {
                continue;
            }
            let looked = *k == Kind::Use && en.look_at && en.box_dims.is_some();
            if looked {
                if !box_use_reach(en, mins, maxs, eye, fwd) {
                    continue;
                }
            } else if !touches(world, en, mins, maxs)
                && !radius_use_reach(world, en, eye, fwd, use_radius)
            {
                continue;
            }
            let obj = en.obj.unwrap();
            match k {
                Kind::Touch => {
                    fire.push((obj, pobj));
                    // BO2 (zombies) puts a touched trigger_radius or
                    // trigger_box that has a hint string on the use list, so
                    // its hint shows (a window's "Rebuild Barrier"); with
                    // look-at required, only within a 0.76 view dot.
                    if matches!(en.class_tag.as_str(), "trigger_radius" | "trigger_box")
                        && en.hint.is_some()
                        && touches(world, en, mins, maxs)
                    {
                        let c = center(world, en);
                        let d = gsc_t6::math::normalize(gsc_t6::math::sub(c, eye));
                        let points = gsc_t6::math::dot(d, fwd);
                        if (!en.look_at || points >= 0.76) && beats(points, en.hint_low, &best) {
                            best = Some(Faced { points, obj, prompt: en.hint.clone(), low: en.hint_low });
                        }
                    }
                }
                Kind::Use => {
                    let c = center(world, en);
                    let d = gsc_t6::math::normalize(gsc_t6::math::sub(c, eye));
                    let points = gsc_t6::math::dot(d, fwd);
                    if beats(points, en.hint_low, &best) {
                        best = Some(Faced { points, obj, prompt: en.hint.clone(), low: en.hint_low });
                    }
                }
            }
        }
        {
            let mut zm = world.resource_mut::<Zm>();
            // BO2MP_T6_HINTLOG=1: each change of the faced trigger's hint and
            // the trigger it came from.
            if std::env::var_os("BO2MP_T6_HINTLOG").is_some() {
                let now_hint = best.as_ref().and_then(|b| b.prompt.clone());
                if zm.hints.get(&peer) != now_hint.as_ref() {
                    let from = best.as_ref().and_then(|b| {
                        triggers
                            .iter()
                            .find(|(_, en, _)| en.obj == Some(b.obj))
                            .map(|(nn, en, _)| {
                                format!(
                                    "ent{nn} {} at ({:.0} {:.0} {:.0}) angles {:?} box {:?}",
                                    en.class_tag,
                                    en.origin[0],
                                    en.origin[1],
                                    en.origin[2],
                                    en.rot,
                                    en.box_dims
                                )
                            })
                    });
                    diag::info!(
                        Sim,
                        "bo2zm t6 hint for {peer}: {now_hint:?} from {from:?}"
                    );
                }
            }
            match best.as_ref().and_then(|b| b.prompt.as_ref()) {
                Some(h) => {
                    zm.hints.insert(peer, h.clone());
                }
                None => {
                    zm.hints.remove(&peer);
                }
            }
        }
        // bo2mp: else a usable model's (a care package's capture prompt).
        if !in_vehicle
            && best.as_ref().is_none_or(|b| b.prompt.is_none())
            && let Some(origin) = frame(world).avatar(ClientId(peer)).map(|ps| ps.origin)
            && let Some(h) = super::killstreaks::usable_hint(world, origin)
        {
            world.resource_mut::<Zm>().hints.insert(peer, h);
        }
        if pressed && let Some(Faced { obj, .. }) = best {
            if std::env::var("BO2MP_T6_TRIGLOG").is_ok() {
                diag::info!(Sim, "bo2zm t6 use by {peer} on {obj:?}");
            }
            fire.push((obj, pobj));
        }
    }
    actor_touches(world, &triggers, &mut fire);
    if fire.is_empty() {
        return;
    }
    with_vm(world, |vm, world| {
        if let Ok(names) = std::env::var("BO2MP_T6_FIRELOG") {
            fire_log(vm, world, &triggers, &fire, &names);
        }
        for (tt, pt) in fire {
            if vm.alive(tt) && !super::declassified::cabinet_used(vm, world, tt, pt) {
                vm.notify_str(world, tt, "trigger", &[ScriptValue::Object(pt)]);
            }
        }
    });
}

/// BO2's trigger_hurt (SP_trigger_hurt, hurt_touch, G_DoTouchTriggers): a
/// playing player touching one it is not hidden from gets its `trigger`
/// notify and its `dmg` (5 when the map gives none) as MOD_TRIGGER_HURT,
/// the trigger his inflictor and attacker, with no point and no direction.
/// Then it rests 50 ms (1000 ms with spawnflags 16) on its one clock for
/// everyone, and with spawnflags 32 it goes off. One with spawnflags 1
/// starts off (BO2's hurt_use turns it on: unported). Players only: BO2's
/// actors and vehicles touch them too.
pub(super) fn hurt(world: &mut World) {
    let players: Vec<(u32, ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .where_value(|pt| pt.sessionstate == "playing")
        .map(|(c, pt)| (*c, pt.obj))
        .collect();
    if players.is_empty() {
        return;
    }
    let hurts: Vec<(u32, Ent)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.class_tag == "trigger_hurt" && en.obj.is_some() && !en.trigger_off && !en.hurt_off)
        .map(|(n, en)| (*n, en.clone()))
        .collect();
    if hurts.is_empty() {
        return;
    }
    for (peer, pobj) in players {
        if !frame(world).avatar(ClientId(peer)).is_some_and(|ps| ps.health > 0) {
            continue;
        }
        let Some((mins, maxs)) = player_box(world, peer) else {
            continue;
        };
        for (n, en) in &hurts {
            if en.invisible_to.contains(&peer) || en.invisible_to_all || !touches(world, en, mins, maxs) {
                continue;
            }
            let t = en.obj.unwrap();
            with_vm(world, |vm, world| {
                let now = vm.clock_stamp_ms;
                if !vm.alive(t)
                    || world
                        .resource::<Zm>()
                        .ents
                        .get(n)
                        .is_none_or(|en| en.trigger_off || en.hurt_off || en.hurt_at > now)
                {
                    return;
                }
                let (sf, dmg) = (vm.intern("spawnflags"), vm.intern("dmg"));
                let flags = vm.raw_field(t, sf).as_int().unwrap_or(0);
                if flags & 1 != 0 {
                    return;
                }
                let amount = vm.raw_field(t, dmg).as_int().filter(|d| *d != 0).unwrap_or(5);
                if let Some(en) = world.resource_mut::<Zm>().ents.get_mut(n) {
                    en.hurt_at = now + if flags & 0x10 != 0 { 1000 } else { 50 };
                    en.hurt_off = flags & 0x20 != 0;
                }
                vm.notify_str(world, t, "trigger", &[ScriptValue::Object(pobj)]);
                super::natives_ai::player_damage_from(
                    vm,
                    world,
                    peer,
                    ScriptValue::Object(t),
                    ScriptValue::Object(t),
                    amount,
                    0,
                    "MOD_TRIGGER_HURT",
                    "none",
                    ScriptValue::Undefined,
                    ScriptValue::Undefined,
                    "none",
                );
            });
        }
    }
}

/// BO2MP_T6_FIRELOG=<targetname>[,...]: each trigger with that name that
/// fires, which entity it is, its volume and who set it off (at most once a
/// second per pair; Die Rise's squash triggers refuse Who's Who's body).
fn fire_log(
    vm: &mut gsc_t6::Vm<World>,
    world: &mut World,
    triggers: &[(u32, Ent, Kind)],
    fire: &[(ObjRef, ObjRef)],
    names: &str,
) {
    static LAST: std::sync::Mutex<Vec<(String, std::time::Instant)>> =
        std::sync::Mutex::new(Vec::new());
    let (tn, org) = (vm.intern("targetname"), vm.intern("origin"));
    for (tt, p) in fire {
        let name = vm.to_text(&vm.raw_field(*tt, tn));
        if !names.split(',').any(|nn| nn == name) {
            continue;
        }
        let key = format!("{tt:?}{p:?}");
        {
            let Ok(mut last) = LAST.lock() else { return };
            match last.iter_mut().find(|(k, _)| *k == key) {
                Some((_, at)) if at.elapsed().as_secs_f32() < 1.0 => continue,
                Some((_, at)) => *at = std::time::Instant::now(),
                None => last.push((key, std::time::Instant::now())),
            }
        }
        let who = vm.get_field(world, *p, org);
        let Some((nn, en, _)) = triggers.iter().find(|(_, en, _)| en.obj == Some(*tt)) else {
            continue;
        };
        diag::info!(
            Sim,
            "bo2zm t6 fire {name}: ent{nn} {} {:?} at {:?} box {:?} radius {} by {p:?} at {}",
            en.class_tag,
            en.brush,
            en.origin,
            en.box_dims,
            en.reach,
            vm.to_text(&who)
        );
    }
}

/// Touch triggers whose spawnflags take AI (1 axis, 2 allies, 4 neutral)
/// fire for each living actor of that team inside (Verrückt's electric
/// traps: zombies walking through the live `trigger_multiple` die).
fn actor_touches(
    game: &mut World,
    triggers: &[(u32, Ent, Kind)],
    fire: &mut Vec<(ObjRef, ObjRef)>,
) {
    let ai: Vec<(Ent, i32)> = with_vm(game, |vm, game| {
        let sf = vm.intern("spawnflags");
        of_kind(triggers, Kind::Touch)
            .map(|t| &t.1)
            .filter_map(|en| match vm.get_field(game, en.obj?, sf) {
                // BO2 (G_DoTouchTriggers): any AI flag lets every zombie
                // in; lhs team the scripts set then picks (the claymore's
                // flag 4 trigger ignores the owner's side).
                ScriptValue::Int(f) if f & 7 != 0 && en.trigger_team.is_some() => Some((en.clone(), 7)),
                ScriptValue::Int(f) if f & 7 != 0 => Some((en.clone(), f)),
                _ => None,
            })
            .collect()
    })
    .unwrap_or_default();
    if ai.is_empty() {
        return;
    }
    let actors: Vec<(ObjRef, i32, String, [f32; 3], [f32; 3])> = {
        let zm = game.resource::<Zm>();
        zm.actors
            .iter()
            .where_value(|lhs| lhs.living)
            .filter_map(|(nn, lhs)| {
                let p = zm.ents.get(nn)?.origin;
                let team = match lhs.team.as_str() {
                    "axis" => 1,
                    "allies" => 2,
                    _ => 4,
                };
                Some((
                    lhs.obj?,
                    team,
                    lhs.team.clone(),
                    [p[0] - lhs.reach, p[1] - lhs.reach, p[2]],
                    [p[0] + lhs.reach, p[1] + lhs.reach, p[2] + lhs.height],
                ))
            })
            .collect()
    };
    for (en, flags) in &ai {
        for (obj, team, name, mins, maxs) in &actors {
            let side = match &en.trigger_team {
                Some((t, exclude)) => (t == name) != *exclude,
                None => true,
            };
            if flags & team != 0 && side && touches(game, en, *mins, *maxs) {
                fire.push((en.obj.unwrap(), *obj));
            }
        }
    }
}

/// How a `trigger_damage` was hit: its spawnflags turn each kind away
/// (1 pistol, 2 rifle, 4 projectile, 8 explosion, 16 splash, 32 melee).
#[derive(Clone, Copy)]
pub(crate) enum DamageKind {
    Bullet,
    Explosion,
    Melee,
    /// A flying projectile's path this tick (the staffs' shots): only the
    /// damageable models it goes through, not zombies or triggers.
    Missile,
    /// A shot that lands without going off (the ballistic knife's blade):
    /// BO2 damages every `trigger_damage` its last stretch crosses with
    /// MOD_IMPACT, which no spawnflag turns away, and only triggers.
    Impact,
}

/// A shot, a knife or a blast through a `trigger_damage` brush fires it:
/// "damage" then "trigger" (Nuketown's bunker hatch plays Marlton when
/// knifed). `start..end` is the shot's line (to its first hit), or the
/// blast's centre to itself with `radius`.
pub(crate) fn damage_triggers(
    world: &mut World,
    from_pt: [f32; 3],
    end: [f32; 3],
    reach: f32,
    qty: i32,
    hitter: ClientId,
    gun: u32,
    how: DamageKind,
) {
    damage_triggers_but(world, from_pt, end, reach, qty, hitter, gun, how, &[]);
}

/// `damage_triggers` passing over `skip` (what this projectile already
/// went through); returns what it damaged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn damage_triggers_but(
    game: &mut World,
    from_pt: [f32; 3],
    end: [f32; 3],
    reach: f32,
    qty: i32,
    hitter: ClientId,
    gun: u32,
    how: DamageKind,
    skip: &[ObjRef],
) -> Vec<ObjRef> {
    if !game.contains_resource::<super::T6Runtime>() {
        return Vec::new();
    }
    let projectile = matches!(how, DamageKind::Missile);
    let impact = matches!(how, DamageKind::Impact);
    let cands: Vec<Ent> = {
        let zm = game.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(n, en)| {
                let trigger = en.class_tag == "trigger_damage" && !en.trigger_off;
                en.obj.is_some_and(|o| !skip.contains(&o))
                    && if projectile {
                        damageable_model(en) && !zm.actors.contains_key(n)
                    } else if impact {
                        trigger
                    } else {
                        trigger || damageable_model(en)
                    }
            })
            .map(|(_, en)| en.clone())
            .collect()
    };
    if cands.is_empty() {
        return Vec::new();
    }
    let w = super::weapon_text(game, gun);
    let block = match how {
        DamageKind::Melee => 32,
        DamageKind::Explosion => 8 | 16,
        DamageKind::Missile => 4,
        DamageKind::Impact => 0,
        DamageKind::Bullet => {
            let pistol = [
                "m1911",
                "python",
                "fiveseven",
                "judge",
                "kard",
                "beretta93r",
                "c96",
                "rnma",
            ]
            .iter()
            .any(|p| w.starts_with(p));
            if pistol { 1 } else { 2 }
        }
    };
    // A knife names the knife the scripts gave him, as a knifed zombie does
    // (`entity_hit`): Buried's tunnel signs light only for the Bowie Knife or
    // the Galvaknuckles (`zm_buried_sq_ts`).
    let w = if matches!(how, DamageKind::Melee) {
        game
            .resource::<Zm>()
            .players
            .get(&hitter.0)
            .and_then(|p| p.melee_weapon.clone())
            .unwrap_or_else(|| "knife_zm".to_owned())
    } else {
        w
    };
    let len = gsc_t6::math::length(gsc_t6::math::sub(end, from_pt));
    let steps = ((len / 4.0).ceil() as usize).clamp(1, 2048);
    let mut hit = Vec::new();
    let mut boxed = Vec::with_capacity(cands.len());
    for en in &cands {
        // A model the scripts made damageable (`setcandamage`): its model's
        // box (Mob's Afterlife shock boxes, zapped with the lightning hands).
        if damageable_model(en) && en.brush.is_none() {
            let Some((low, hi)) = super::brushes::model_box(game, en) else {
                continue;
            };
            let near =
                |at: [f32; 3], r: f32| (0..3).all(|k| at[k] >= low[k] - r && at[k] <= hi[k] + r);
            let first = match how {
                DamageKind::Explosion => near(from_pt, reach).then_some(0),
                _ => (0..=steps).position(|i| {
                    let t = i as f32 / steps as f32;
                    near(
                        std::array::from_fn(|k| from_pt[k] + (end[k] - from_pt[k]) * t),
                        2.0,
                    )
                }),
            };
            let touched = first.is_some();
            if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                diag::info!(
                    Sim,
                    "bo2zm t6 damageable {} box {low:.0?}-{hi:.0?} shot {from_pt:.0?}->{end:.0?} touched {touched}",
                    en.mesh
                );
            }
            if let Some(first) = first {
                hit.push(en.obj.unwrap());
                boxed.push((en.obj.unwrap(), low, hi, first));
            }
            continue;
        }
        let touched = match how {
            DamageKind::Explosion => {
                let c = center(game, en);
                gsc_t6::math::length(gsc_t6::math::sub(c, from_pt)) <= reach
                    || touches(
                        game,
                        en,
                        from_pt.map(|v| v - reach * 0.5),
                        from_pt.map(|v| v + reach * 0.5),
                    )
            }
            _ => (0..=steps).any(|i| {
                let t = i as f32 / steps as f32;
                let at: [f32; 3] = std::array::from_fn(|k| from_pt[k] + (end[k] - from_pt[k]) * t);
                touches(game, en, at.map(|v| v - 2.0), at.map(|v| v + 2.0))
            }),
        };
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() && en.class_tag != "trigger_damage" {
            diag::info!(
                Sim,
                "bo2zm t6 damageable brush {} {} at {:.0?} shot {from_pt:.0?}->{end:.0?} touched {touched}",
                en.class_tag,
                en.mesh,
                en.origin
            );
        }
        if touched {
            hit.push(en.obj.unwrap());
        }
    }
    // Models whose boxes nest (Origins' wind rings, one inside the other
    // on the chamber ceiling): a shot meets the ring whose band it goes
    // through, so the outer box does not count where the shot was already
    // inside an inner one when it reached it.
    if !matches!(how, DamageKind::Explosion) {
        let inside = |a: &([f32; 3], [f32; 3]), b: &([f32; 3], [f32; 3])| {
            a != b && (0..3).all(|k| b.0[k] >= a.0[k] && b.1[k] <= a.1[k])
        };
        let mut drop: Vec<ObjRef> = Vec::new();
        for (o, low, hi, first) in boxed.iter() {
            if boxed.iter().any(|(_, l, h, f)| f <= first && inside(&(*low, *hi), &(*l, *h))) {
                drop.push(*o);
            }
        }
        hit.retain(|o| !drop.contains(o));
    }
    if hit.is_empty() {
        return Vec::new();
    }
    let done = hit.clone();
    let pobj = game
        .resource::<Zm>()
        .players
        .get(&hitter.0)
        .map(|p| p.obj);
    let means = match how {
        DamageKind::Melee => "MOD_MELEE",
        DamageKind::Explosion => "MOD_GRENADE_SPLASH",
        DamageKind::Missile => "MOD_PROJECTILE",
        DamageKind::Impact => "MOD_IMPACT",
        DamageKind::Bullet => {
            if block == 1 {
                "MOD_PISTOL_BULLET"
            } else {
                "MOD_RIFLE_BULLET"
            }
        }
    };
    let models: Vec<ObjRef> = cands
        .iter()
        .filter(|en| en.class_tag != "trigger_damage")
        .filter_map(|en| en.obj)
        .collect();
    let vehicles: Vec<ObjRef> = cands
        .iter()
        .filter(|en| en.class_tag == "script_vehicle")
        .filter_map(|en| en.obj)
        .collect();
    with_vm(game, |vm, game| {
        let sf = vm.intern("spawnflags");
        let cb = vm.intern("callbackvehicledamage");
        let vehicle_cb = !matches!(vm.get_field(game, vm.level, cb), ScriptValue::Undefined);
        for t in hit {
            if !vm.alive(t) {
                continue;
            }
            let model = models.contains(&t);
            let flags = match vm.get_field(game, t, sf) {
                ScriptValue::Int(i) if !model => i,
                _ => 0,
            };
            if flags & block != 0 {
                continue;
            }
            if std::env::var("BO2MP_T6_TRIGLOG").is_ok() {
                diag::info!(
                    Sim,
                    "bo2zm t6 damage trigger {t:?} by {hitter:?} {means} {qty}"
                );
            }
            let att = pobj.map_or(ScriptValue::Undefined, ScriptValue::Object);
            let dir = ScriptValue::Vec3(gsc_t6::math::normalize(gsc_t6::math::sub(end, from_pt)));
            // BO2's landing hit sends five: no point (the origin), no tag.
            if impact {
                let args = [
                    ScriptValue::Int(qty),
                    att.clone(),
                    dir,
                    ScriptValue::Vec3([0.0; 3]),
                    vm.string(means),
                ];
                vm.notify_str(game, t, "damage", &args);
                if vm.alive(t) {
                    vm.notify_str(game, t, "trigger", &[att]);
                }
                continue;
            }
            let args = [
                ScriptValue::Int(qty),
                att.clone(),
                dir,
                ScriptValue::Vec3(end),
                vm.string(means),
                vm.string(""),
                vm.string(""),
                vm.string(""),
                vm.string(&w),
                ScriptValue::Int(0),
            ];
            // A vehicle goes through the code callback too (Origins' fire
            // staff biplane comes down in `level.callbackvehicledamage`).
            if vehicle_cb && vehicles.contains(&t) {
                let cargs = vec![
                    att.clone(),
                    att.clone(),
                    ScriptValue::Int(qty),
                    ScriptValue::Int(0),
                    vm.string(means),
                    vm.string(&w),
                    ScriptValue::Vec3(end),
                    args[2].clone(),
                    vm.string("none"),
                    ScriptValue::Int(0),
                    ScriptValue::Int(0),
                    ScriptValue::Int(0),
                    vm.string(""),
                ];
                vm.spawn_named(
                    game,
                    "maps/mp/gametypes_zm/_callbacksetup",
                    "codecallback_vehicledamage",
                    ScriptValue::Object(t),
                    cargs,
                );
            }
            vm.notify_str(game, t, "damage", &args);
            if !model && vm.alive(t) {
                vm.notify_str(game, t, "trigger", &[att]);
            }
        }
    });
    done
}

/// A shown (or ghosted) model entity the scripts made damageable
/// (`setcandamage`).
fn damageable_model(e: &Ent) -> bool {
    // A script brush ("*N", kept in `brush`, no model name) counts too:
    // Origins' ghosted wind smoke detectors (air_puzzle_run_smoke_direction).
    e.can_damage
        && (!e.unseen || e.ghosted)
        && (!e.mesh.is_empty() || e.brush.is_some())
        && !e.class_tag.starts_with("trigger")
}

#[allow(dead_code)]
fn _volume_type(_: Volume<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A long thin brush trigger along +x, turned a quarter to the left
    /// (yaw 90), lies along +y: a box there touches it, one on the old
    /// spot does not.
    #[test]
    fn a_turned_brush_trigger_turns_its_brushes() {
        let brush = crate::SolidBrush {
            sides: vec![
                [1.0, 0.0, 0.0, 100.0],
                [-1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 10.0],
                [0.0, -1.0, 0.0, 10.0],
                [0.0, 0.0, 1.0, 10.0],
                [0.0, 0.0, -1.0, 0.0],
            ],
            solid_bits: 0,
            plane_material_flags: Vec::new(),
            glass_hit_code: 0,
            vertex_points: Vec::new(),
            hit_surface_flags: 0,
        };
        let brush_list = [brush];
        let v = Volume::Brushes {
            origin: [0.0; 3],
            ids: &[0],
            brushes: &brush_list,
        };
        let at = |x: f32, y: f32| ([x - 1.0, y - 1.0, 4.0], [x + 1.0, y + 1.0, 6.0]);
        let turn = turn_between([0.0; 3], [0.0, 90.0, 0.0]);
        let (mins, maxs) = at(50.0, 0.0);
        assert!(v.touches(mins, maxs));
        assert!(!v.touches_turned(mins, maxs, turn));
        let (mins, maxs) = at(0.0, 50.0);
        assert!(!v.touches(mins, maxs));
        assert!(v.touches_turned(mins, maxs, turn));
        // No turn is the plain test.
        let none = turn_between([0.0, 30.0, 0.0], [0.0, 30.0, 0.0]);
        let (mins, maxs) = at(50.0, 0.0);
        assert!(v.touches_turned(mins, maxs, none));
    }

    /// A buyable door's hint (low priority) yields to a window's even when
    /// he faces the door better; between two of a kind the better faced wins.
    #[test]
    fn a_low_priority_hint_yields() {
        assert!(hint_beats(0.2, false, Some((0.9, true))));
        assert!(!hint_beats(0.9, true, Some((0.2, false))));
        assert!(hint_beats(0.9, true, Some((0.2, true))));
        assert!(!hint_beats(0.2, false, Some((0.9, false))));
        assert!(hint_beats(0.1, true, None));
    }
}
