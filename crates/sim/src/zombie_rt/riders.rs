//! Black Ops II's moving platforms (`setmovingplatformenabled(1)`:
//! Tranzit's bus and the clip linked to it): who stands on one, and
//! carrying them as it moves and turns (BO2's engine moves a player with
//! the platform under his feet). `getmoverent` names the platform: the bus
//! scripts count riders by it (`busupdateplayers`).

use super::pairs::KeepByValue;
use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::World;

use super::{Closest, Ent, Zm, frame};
use crate::sim_parts::ClientId;

/// Riders and the platforms' poses last tick.
#[derive(Clone, Debug, Default)]
pub(crate) struct Riders {
    /// Platform -> (origin, angles) at the end of last tick.
    pub poses: BTreeMap<u32, ([f32; 3], [f32; 3])>,
    /// Player -> the platform he stands on (the root of its links).
    pub on: BTreeMap<u32, u32>,
    /// Players its last move carried. BO2 moves a rider with its mover and
    /// traces nothing (G_PushEntity), so the bus takes him through the
    /// map's own clip brushes; only the movement code's 1-unit fix runs in
    /// his move after. The step's wider unstick leaves these alone.
    pub carried: BTreeSet<u32>,
}

/// A platform's box in its own space (mins, maxs): a brush entity's
/// submodel, a model's bounds.
fn local_box(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    if let Some(n) = super::triggers::brush_index(e) {
        let f = frame(world);
        let m = f.collision_submodel_set().models.get(n as usize)?;
        return Some((m.lo, m.hi));
    }
    if e.mesh.is_empty() {
        return None;
    }
    let (mid, half) = frame(world).model_capability(&e.mesh).flatten()?.bounds?;
    Some((
        std::array::from_fn(|i| mid[i] - half[i]),
        std::array::from_fn(|i| mid[i] + half[i]),
    ))
}

/// A world point in an entity's own space (IW axes: forward, left, up).
pub(crate) fn to_local(pt: [f32; 3], spot: [f32; 3], rot: [f32; 3]) -> [f32; 3] {
    let (f, r, u) = gsc_t6::math::facing_axes(rot);
    let d = gsc_t6::math::sub(pt, spot);
    [
        gsc_t6::math::dot(d, f),
        -gsc_t6::math::dot(d, r),
        gsc_t6::math::dot(d, u),
    ]
}

/// A point in an entity's own space (forward, left, up) in the world.
pub(crate) fn to_world(l: [f32; 3], spot: [f32; 3], rot: [f32; 3]) -> [f32; 3] {
    let (f, r, u) = gsc_t6::math::facing_axes(rot);
    std::array::from_fn(|i| spot[i] + f[i] * l[0] - r[i] * l[1] + u[i] * l[2])
}

/// The scripted solid under a point (`getgroundent`): the smallest
/// script brush model or moving platform whose box holds it (its top a few
/// units under it at most). Brush model hits come back from the traces as
/// the world, so Die Rise's elevators are found this way
/// (`object_is_on_elevator`: a piece or equipment placed in one).
pub(crate) fn ground_ent(game: &mut World, p: [f32; 3]) -> Option<u32> {
    let cands: Vec<(u32, Ent)> = game
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| !en.unseen && (en.platform || en.class_tag == "script_brushmodel"))
        .map(|(n, en)| (*n, en.clone()))
        .collect();
    let mut pick = Closest::new();
    for (n, en) in cands {
        let Some((low, hi)) = local_box(game, &en) else {
            continue;
        };
        let l = to_local(p, en.origin, en.rot);
        let inside = (0..2).all(|i| l[i] >= low[i] && l[i] <= hi[i])
            && l[2] >= low[2] - 2.0
            && l[2] <= hi[2] + 4.0;
        let vol = (hi[0] - low[0]) * (hi[1] - low[1]) * (hi[2] - low[2]).max(1.0);
        if inside {
            pick.offer(vol, n);
        }
    }
    pick.into_item()
}

/// The platform an entity is linked under (its link chain's root).
fn root(zm: &Zm, mut n: u32) -> u32 {
    for _ in 0..8 {
        match zm.ents.get(&n).and_then(|e| e.link) {
            Some((p, _, _)) if zm.ents.get(&p).is_some_and(|e| e.platform) => n = p,
            _ => break,
        }
    }
    n
}

thread_local! {
    /// Dense ride log: where the last carry put a rider (player, place).
    static LAST_TO: std::cell::Cell<Option<(u32, [f32; 3])>> = const { std::cell::Cell::new(None) };
    /// Dense ride log: the brush models each platform carries, listed once.
    static LISTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Each tick, after the vehicles and linked entities moved: each player
/// on the ground inside a platform's box (as it stood last tick, where his
/// move was made against it) rides it: he is moved and turned as it moved
/// and turned since. Riders are kept for `getmoverent`.
pub(crate) fn advance(world: &mut World) {
    ride_test(world);
    let plats: Vec<(u32, Ent)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.platform && !en.unseen)
        .map(|(nn, en)| (*nn, en.clone()))
        .collect();
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let prev = std::mem::take(&mut world.resource_mut::<Zm>().riders.poses);
    let mut on = BTreeMap::new();
    let log = std::env::var_os("BO2MP_T6_RIDELOG").is_some();
    // Boxes once per tick.
    let boxes: Vec<(u32, ([f32; 3], [f32; 3]), Ent)> = plats
        .into_iter()
        .filter_map(|(nn, en)| local_box(world, &en).map(|rhs| (nn, rhs, en)))
        .collect();
    if log {
        for (nn, (lo, hi), en) in boxes.iter().filter(|(nn, _, _)| !prev.contains_key(nn)) {
            diag::info!(
                Sim,
                "bo2zm t6 platform ent{nn} {} {} at ({:.0} {:.0} {:.0}) box ({:.0} {:.0} {:.0})..({:.0} {:.0} {:.0})",
                en.class_tag,
                if en.mesh.is_empty() {
                    en.brush.as_deref().unwrap_or("")
                } else {
                    &en.mesh
                },
                en.origin[0],
                en.origin[1],
                en.origin[2],
                lo[0],
                lo[1],
                lo[2],
                hi[0],
                hi[1],
                hi[2]
            );
        }
    }
    if log && dense(world) && !LISTED.get() {
        LISTED.set(true);
        let zm = world.resource::<Zm>();
        for (nn, _, _) in &boxes {
            for (m, en) in zm.ents.iter().filter(|(m, en)| **m != *nn && en.brush.is_some() && root(&zm, **m) == *nn) {
                diag::info!(
                    Sim,
                    "bo2zm t6 platform ent{nn} carries ent{m} {} {} solid {} hidden {} at ({:.1} {:.1} {:.1}) angles ({:.1} {:.1} {:.1})",
                    en.class_tag,
                    en.brush.as_deref().unwrap_or(""),
                    en.solid,
                    en.unseen,
                    en.origin[0],
                    en.origin[1],
                    en.origin[2],
                    en.rot[0],
                    en.rot[1],
                    en.rot[2]
                );
            }
        }
    }
    let was = world.resource::<Zm>().riders.on.clone();
    let mut carried = BTreeSet::new();
    for p in players {
        let peer = ClientId(p);
        let Some(pstate) = frame(world).avatar(peer).copied() else {
            continue;
        };
        let grounded = pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT
            || pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::LADDER != 0;
        // Where his own move left him (the dense ride log).
        let start = pstate.origin;
        // Dense log: the zombies whose square box overlapped him where the
        // last carry put him, as his move this tick met them (the step's
        // unstick check: his prone-high box against theirs, as of the end of
        // the last tick), in the bus's space as it stood then.
        let solids = match (log && dense(world), LAST_TO.get()) {
            (true, Some((c, last))) if c == p => frame(world)
                .packed_solid_movers()
                .into_iter()
                .filter_map(|(nn, o, s)| {
                    let x = (s & 0xff) as f32 + 15.0;
                    let below = ((s >> 8) & 0xff) as f32;
                    let above = ((s >> 16) & 0xff) as f32 - 32.0;
                    let hit = (o[0] - last[0]).abs() < x
                        && (o[1] - last[1]).abs() < x
                        && o[2] - below < last[2] + movement_t6::bridge::BELLY_HULL_TOP
                        && o[2] + above > last[2];
                    let (bo, ba) = was.get(&p).and_then(|c| prev.get(c)).copied()?;
                    hit.then(|| {
                        let l = to_local(o, bo, ba);
                        format!(" ent{nn} local ({:.0} {:.0} {:.0})", l[0], l[1], l[2])
                    })
                })
                .collect::<String>(),
            _ => String::new(),
        };
        // The smallest box he stands in or on wins (the clip on the bus
        // over the bus's own mdl). In the air, only a vehicle's box holds
        // him (`floor_z` keeps him on its floor). One already riding a
        // platform stays on it while in its box, in the air too: a Die Rise
        // elevator going down drops away under his feet each tick, and he
        // falls back onto its floor.
        let mut best: Option<(u32, f32, ([f32; 3], [f32; 3]))> = None;
        for (nn, (lo, hi), en) in &boxes {
            if !grounded
                && en.class_tag != "script_vehicle"
                && was.get(&p) != Some(&root(&world.resource::<Zm>(), *nn))
            {
                continue;
            }
            let (o, a) = prev.get(nn).copied().unwrap_or((en.origin, en.rot));
            let l = to_local(pstate.origin, o, a);
            let inside = (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i])
                && l[2] >= lo[2] - 2.0
                && l[2] <= hi[2] + 4.0;
            if !inside {
                continue;
            }
            // A Die Rise car holds only who stands on its floor or roof
            // (one in the doorway as it comes up from below is not lifted);
            // the trigger linked goal it adds nothing.
            let zm = world.resource::<Zm>();
            let car = |m: u32| {
                zm.ents
                    .get(&m)
                    .map(|r| super::highrise::platform_slabs(&r.mesh)).unwrap_or(&[])
            };
            if !car(root(&zm, *nn)).is_empty() {
                let slabs = car(*nn);
                if !slabs
                    .iter()
                    .any(|&(_, top)| l[2] >= top - 6.0 && l[2] <= top + 60.0)
                {
                    continue;
                }
            }
            let vol = (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2]).max(1.0);
            if best.is_none_or(|(_, v, _)| vol < v) {
                best = Some((*nn, vol, (*lo, *hi)));
            }
        }
        let Some((plat, _, plat_box)) = best else {
            if log && let Some(c) = was.get(&p) {
                let at = world.resource::<Zm>().ents.get(c).map(|en| en.origin);
                diag::info!(
                    Sim,
                    "bo2zm t6 rider {p} off ent{c} (at {at:?}): at ({:.0} {:.0} {:.0}) grounded {grounded}",
                    pstate.origin[0],
                    pstate.origin[1],
                    pstate.origin[2]
                );
            }
            continue;
        };
        let carrier = root(&world.resource::<Zm>(), plat);
        on.insert(p, carrier);
        hold_on_floor(world, peer, carrier, prev.get(&carrier).copied());
        let Some(pstate) = frame(world).avatar(peer).copied() else {
            continue;
        };
        // Carried by the root's move (all its linked pieces move with it).
        let Some(&(o0, a0)) = prev.get(&carrier) else {
            continue;
        };
        let Some((o1, a1)) = world
            .resource::<Zm>()
            .ents
            .get(&carrier)
            .map(|en| (en.origin, en.rot))
        else {
            continue;
        };
        let dyaw = game_math::degree_gap(a1[1], a0[1]);
        if o0 == o1 && a0 == a1 {
            if log && log_now(world) {
                let l = to_local(pstate.origin, o1, a1);
                diag::info!(
                    Sim,
                    "bo2zm t6 rider {p} on ent{carrier} (bus still) local ({:.0} {:.0} {:.0})",
                    l[0],
                    l[1],
                    l[2]
                );
            }
            continue;
        }
        // A Die Rise car's doors are shut while it moves: its rider stays
        // inside its walls (the mdl is not solid goal him here).
        let mut from = pstate.origin;
        let is_car = world
            .resource::<Zm>()
            .ents
            .get(&plat)
            .is_some_and(|en| !super::highrise::platform_slabs(&en.mesh).is_empty());
        if is_car && o0 != o1 {
            from = super::highrise::keep_inside(from, o0, a0, plat_box);
        }
        // His place in the platform's own space, put back where it is now
        // (turned and tilted: the bus climbs and dips along its road).
        let l = to_local(from, o0, a0);
        let mut goal = to_world(l, o1, a1);
        // One standing on the bus stays standing: put back just over what
        // he stands on, as it is tilted now. Lifted clear of it instead, he
        // was in the air every tick it moved: no friction, and each landing
        // on its sloped floor pushed him downhill (runs/bl2: 100-250 units
        // along the aisle on its climbs and dips).
        let mdl = world
            .resource::<Zm>()
            .ents
            .get(&carrier)
            .map(|en| en.mesh.clone())
            .unwrap_or_default();
        let top = stand_top(&mdl, l, tilt_rest(a0), pstate.velocity[2]);
        match top {
            Some(top) => goal = to_world([l[0], l[1], top + tilt_rest(a1) + 0.125], o1, a1),
            None => goal[2] += 0.25,
        }
        let mut fl = frame(world);
        if let Some(pstate) = fl.occupant_mut(peer) {
            pstate.origin = goal;
            pstate.view_offset_angles[1] += dyaw;
            pstate.look_angles[1] += dyaw;
        }
        fl.translate_player_area(peer, gsc_t6::math::sub(goal, pstate.origin));
        carried.insert(p);
        LAST_TO.set(Some((p, goal)));
        if log && log_now(world) {
            let from = l;
            let l = to_local(goal, o1, a1);
            diag::info!(
                Sim,
                "bo2zm t6 rider {p} on ent{carrier} (via ent{plat}) at ({:.0} {:.0} {:.0}) local ({:.0} {:.0} {:.1}) vel ({:.0} {:.0} {:.0}) grounded {grounded}{}",
                goal[0],
                goal[1],
                goal[2],
                l[0],
                l[1],
                l[2],
                pstate.velocity[0],
                pstate.velocity[1],
                pstate.velocity[2],
                if dense(world) {
                    let moved = to_local(start, o0, a0);
                    format!(
                        " | moved to local ({:.2} {:.2} {:.2}), floored ({:.2} {:.2} {:.2}), top {top:?}, angles ({:.2} {:.2} {:.2}), ground ent {}, zombie boxes on him:{}",
                        moved[0], moved[1], moved[2], from[0], from[1], from[2], a1[0], a1[1], a1[2], pstate.standing_on,
                        if solids.is_empty() { " none" } else { &solids }
                    )
                } else {
                    String::new()
                }
            );
        }
    }
    push_clear(world, &prev, &on, log);
    carry_actors(world, &prev);
    let poses = world
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.platform)
        .map(|(nn, en)| (*nn, (en.origin, en.rot)))
        .collect();
    let mut zm = world.resource_mut::<Zm>();
    if log && zm.riders.on != on {
        diag::info!(Sim, "bo2zm t6 riders now {:?} (player -> platform)", on);
    }
    zm.riders.poses = poses;
    zm.riders.on = on;
    zm.riders.carried = carried;
}

/// bo2zm: one beside the moving bus (not riding it) whom its body swept
/// into moves with it, kept where he stood against it, as a mover pushes
/// what it meets. Before, its thin walls (3 units) ended up around him,
/// the step's unstick put him out on either side, and the ride carried him
/// on inside them (a TranZit playtest 10-10: "the bus in general's
/// hitbox is a little bit messed up. I can glitch into the back"). BO2's
/// mover push (G_MoverPush) moves only what stands on the mover and is not
/// a player; its body is solid in his own move there. One the bus already
/// holds inside its body goes out of its nearest side. Where the map is
/// solid at that place he stays (the ride log says so).
fn push_clear(
    world: &mut World,
    prev: &BTreeMap<u32, ([f32; 3], [f32; 3])>,
    on: &BTreeMap<u32, u32>,
    log: bool,
) {
    let buses: Vec<(u32, &'static [([f32; 3], [f32; 3])], ([f32; 3], [f32; 3]), ([f32; 3], [f32; 3]))> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter_map(|(n, e)| {
            if !e.platform || e.unseen || e.class_tag != "script_vehicle" {
                return None;
            }
            let boxes = super::transit::platform_boxes(&e.mesh);
            let was = *prev.get(n)?;
            let now = (e.origin, e.rot);
            (!boxes.is_empty() && was != now).then_some((*n, boxes, was, now))
        })
        .collect();
    if buses.is_empty() {
        return;
    }
    let players: Vec<u32> = world
        .resource::<Zm>()
        .players
        .keys()
        .copied()
        .filter(|p| !on.contains_key(p))
        .collect();
    for p in players {
        let peer = ClientId(p);
        let Some(pstate) = frame(world).avatar(peer).copied() else {
            continue;
        };
        let tall = if pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::LYING_FLAT != 0 {
            movement_t6::bridge::BELLY_HULL_TOP
        } else if pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::DUCK_BUTTON_BIT != 0 {
            movement_t6::bridge::SQUAT_HULL_TOP
        } else {
            movement_t6::bridge::UPRIGHT_HULL_TOP
        };
        for &(n, boxes, (o0, a0), (o1, a1)) in &buses {
            let now = to_local(pstate.origin, o1, a1);
            if !boxes.iter().any(|b| in_box(now, tall, b)) {
                continue;
            }
            // Where he stood against it, as it stands now; or out of its
            // nearest side if it held him already.
            let before = to_local(pstate.origin, o0, a0);
            let swept = !boxes.iter().any(|b| in_box(before, tall, b));
            let l = if swept {
                before
            } else {
                out_of(now, tall, boxes)
            };
            // Sideways only: he keeps his own height (the bus's climb or
            // dip would put him into the road or in the air).
            let mut goal = to_world(l, o1, a1);
            goal[2] = pstate.origin[2];
            let mut fl = frame(world);
            let t = fl.trace_static_world(
                goal,
                goal,
                crate::shot_trace::BODY_BOX_LOWER,
                [15.0, 15.0, tall],
                crate::shot_trace::PLAYER_BODY_MASK,
            );
            let clear = !t.starts_inside && !t.fully_embedded;
            if log {
                diag::info!(
                    Sim,
                    "bo2zm t6 rider {p} {} by ent{n}: local ({:.1} {:.1} {:.1}) -> ({:.1} {:.1} {:.1}){}",
                    if swept { "pushed" } else { "put out" },
                    now[0],
                    now[1],
                    now[2],
                    l[0],
                    l[1],
                    l[2],
                    if clear {
                        ""
                    } else {
                        ", map solid there: stays"
                    }
                );
            }
            if clear {
                if let Some(pstate) = fl.occupant_mut(peer) {
                    pstate.origin = goal;
                }
                fl.translate_player_area(peer, gsc_t6::math::sub(goal, pstate.origin));
            }
            break;
        }
    }
}

/// His box (`tall` high, at `l` in a platform's own space) inside a solid
/// box of it by more than a touch.
fn in_box(l: [f32; 3], tall: f32, (lo, hi): &([f32; 3], [f32; 3])) -> bool {
    const R: f32 = 15.0 - 0.5;
    (0..2).all(|i| l[i] + R > lo[i] && l[i] - R < hi[i])
        && l[2] + tall - 0.5 > lo[2]
        && l[2] + 0.5 < hi[2]
}

/// The nearest place beside a platform's body at his height (front, back
/// or a side) clear of it: one it holds inside goes out there.
fn out_of(l: [f32; 3], tall: f32, boxes: &[([f32; 3], [f32; 3])]) -> [f32; 3] {
    let near: Vec<&([f32; 3], [f32; 3])> = boxes
        .iter()
        .filter(|(lo, hi)| l[2] + tall > lo[2] && l[2] < hi[2])
        .collect();
    let lo = |i: usize| near.iter().map(|rhs| rhs.0[i]).fold(f32::MAX, f32::min);
    let hi = |i: usize| near.iter().map(|rhs| rhs.1[i]).fold(f32::MIN, f32::max);
    let r = 15.125;
    let sides = [
        (0, lo(0) - r),
        (0, hi(0) + r),
        (1, lo(1) - r),
        (1, hi(1) + r),
    ];
    let (i, v) = sides
        .into_iter()
        .min_by(|lhs, rhs| (lhs.1 - l[lhs.0]).abs().total_cmp(&(rhs.1 - l[rhs.0]).abs()))
        .unwrap_or((0, l[0]));
    let mut out = l;
    out[i] = v;
    out
}

/// Zombies standing in or on a vehicle (Tranzit's bus: its steps, aisle
/// and roof) ride it as players do: BO2's actor physics moves an actor
/// with the mover under its feet. Before, the bus drove out from under the
/// zombies that had climbed in. Their route (points on the bus) rides with
/// them; the scripts route again to the bus's own nodes every 0.1 s
/// (`zombiemoveonbus`). One linked (on a window, climbing on) goes with
/// its link instead; one in a scripted clip or a traversal stays put.
fn carry_actors(world: &mut World, prev: &BTreeMap<u32, ([f32; 3], [f32; 3])>) {
    let zm = world.resource::<Zm>();
    let cars: Vec<(([f32; 3], [f32; 3]), ([f32; 3], [f32; 3]), ([f32; 3], [f32; 3]))> = prev
        .iter()
        .filter_map(|(nn, &was)| {
            let e = zm.ents.get(nn)?;
            if e.unseen || e.class_tag != "script_vehicle" || (e.origin, e.rot) == was {
                return None;
            }
            let ride = super::transit::platform_ride_box(&e.mesh)?;
            Some((was, (e.origin, e.rot), ride))
        })
        .collect();
    if cars.is_empty() {
        return;
    }
    let actors: Vec<u32> = zm
        .actors
        .iter()
        .filter(|(nn, lhs)| {
            lhs.living
                && lhs.scripted.is_none()
                && lhs.custom.is_none()
                && lhs.traverse.is_none()
                && zm.ents.get(*nn).is_some_and(|e| e.link.is_none())
        })
        .map(|(nn, _)| *nn)
        .collect();
    drop(zm);
    let mut zm = world.resource_mut::<Zm>();
    let zm = &mut *zm;
    for nn in actors {
        let Some(e) = zm.ents.get_mut(&nn) else {
            continue;
        };
        // On the bus: in its box over its lowest step (one on the road
        // beside or under its front stays there).
        let held = cars.iter().find(|((o0, a0), _, (lo, hi))| {
            let l = to_local(e.origin, *o0, *a0);
            (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) && l[2] >= 15.0 && l[2] <= hi[2]
        });
        let Some(&((o0, a0), (o1, a1), (lo, hi))) = held else {
            continue;
        };
        let carry = |p: [f32; 3]| to_world(to_local(p, o0, a0), o1, a1);
        let on_bus = |p: [f32; 3]| {
            let l = to_local(p, o0, a0);
            (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) && l[2] >= lo[2] - 8.0 && l[2] <= hi[2]
        };
        e.origin = carry(e.origin);
        e.rot[1] += game_math::degree_gap(a1[1], a0[1]);
        if let Some(lhs) = zm.actors.get_mut(&nn) {
            for p in lhs.path.iter_mut().filter(|p| on_bus(**p)) {
                *p = carry(*p);
            }
            if let Some(g) = lhs.goal.as_mut().filter(|g| on_bus(**g)) {
                *g = carry(*g);
            }
            if let Some(g) = lhs.goal_node.as_mut().map(|node| &mut node.1).filter(|g| on_bus(**g)) {
                *g = carry(*g);
            }
        }
    }
}

/// A vehicle's floor height in its own space (BO2MP_T6_BUSFLOOR, default
/// 37.84: the Tranzit bus's deck over its origin at the road).
fn floor_z() -> f32 {
    static Z: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *Z.get_or_init(|| {
        std::env::var("BO2MP_T6_BUSFLOOR")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(super::transit::BUS_FLOOR)
    })
}

/// The vehicle a point stands in or on (`getgroundent` of a zombie that
/// climbed into Tranzit's bus: its steps, aisle or roof): in its ride box
/// over its lowest step, as `carry_actors` holds it.
pub(crate) fn vehicle_under(world: &World, p: [f32; 3]) -> Option<u32> {
    world.resource::<Zm>().ents.iter().find_map(|(n, e)| {
        if e.unseen || e.class_tag != "script_vehicle" {
            return None;
        }
        let (lo, hi) = super::transit::platform_ride_box(&e.mesh)?;
        let l = to_local(p, e.origin, e.rot);
        ((0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) && l[2] >= 15.0 && l[2] <= hi[2]).then_some(*n)
    })
}

/// A point inside the bus, between its walls and under its roof: the spot
/// on its floor under it, in the world (the build test stands him there for
/// the hatch, which is built from inside).
pub(crate) fn bus_floor_under(game: &World, pt: [f32; 3]) -> Option<[f32; 3]> {
    let (lo, hi) = super::transit::BUS_INSIDE;
    game.resource::<Zm>().ents.values().find_map(|e| {
        if !e.platform || e.unseen || super::transit::platform_boxes(&e.mesh).is_empty() {
            return None;
        }
        let l = to_local(pt, e.origin, e.rot);
        let inside = (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) && l[2] > 0.0 && l[2] < 160.0;
        inside.then(|| to_world([l[0], l[1], floor_z() + 1.0], e.origin, e.rot))
    })
}

/// How far over a surface of a tilted platform a player stands, along its
/// up: the player's trace is a capsule (radius 15, trace_t6), whose round
/// bottom touches a tilted floor 15 * (1 - cos tilt) over it. A box's
/// corner rest (15 * (|ux| + |uy|), ~3 units at 8 degrees) left him that
/// high over the floor on every hill: the 0.25 ground trace missed, he was
/// in the air with no friction and slid along the aisle (runs/bl3).
/// BO2MP_T6_RIDELOG=1: a rider line every half second; `=<a>-<b>`
/// (seconds) also logs every tick between them, with his height in the bus
/// before the carry, the top he stands on, its angles and his ground (test
/// aid: rf3 fell through the roof between two half-second lines).
fn dense(world: &World) -> bool {
    static AT: std::sync::OnceLock<Option<(i64, i64)>> = std::sync::OnceLock::new();
    let at = AT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_RIDELOG").ok()?;
        let (a, b) = v.split_once('-')?;
        Some(((a.trim().parse::<f32>().ok()? * 1000.0) as i64, (b.trim().parse::<f32>().ok()? * 1000.0) as i64))
    });
    let now = world.resource::<Zm>().clock_millis;
    at.is_some_and(|(a, b)| now >= a && now < b)
}

fn log_now(world: &World) -> bool {
    world.resource::<Zm>().clock_millis % 500 < crate::SIM_STEP_MILLIS as i64 || dense(world)
}

fn tilt_rest(angles: [f32; 3]) -> f32 {
    let (_, _, u) = gsc_t6::math::facing_axes(angles);
    15.0 * (1.0 - u[2].clamp(0.0, 1.0))
}

/// The top of the bus box he stands on (his own space), when he stands
/// on one: a box under his feet (his box overlapping its top) that he is
/// within a unit of standing on, not on his way up.
fn stand_top(model: &str, l: [f32; 3], rest: f32, vz: f32) -> Option<f32> {
    if vz > 0.0 {
        return None;
    }
    let top = super::transit::platform_boxes(model)
        .iter()
        .filter(|(lo, hi)| {
            (0..2).all(|i| l[i] >= lo[i] - 15.0 && l[i] <= hi[i] + 15.0) && hi[2] <= l[2] + 1.0
        })
        .map(|(_, hi)| hi[2])
        .reduce(f32::max)?;
    ((l[2] - top - rest).abs() <= 1.0).then_some(top)
}

/// The bus's floor is a blocker (`transit`); one inside it is also held up
/// at its floor (falling stops there), in case a carry step left him in it.
/// The bus is measured as it stood when his move was made (`before`, last
/// tick): measured where it is now, a climb lifted him here and again in
/// the carry after.
fn hold_on_floor(
    world: &mut World,
    peer: ClientId,
    carrier: u32,
    before: Option<([f32; 3], [f32; 3])>,
) {
    let Some((o, a)) = world
        .resource::<Zm>()
        .ents
        .get(&carrier)
        .filter(|e| e.class_tag == "script_vehicle")
        .map(|e| (e.origin, e.rot))
    else {
        return;
    };
    let deck = super::tomb_tank::deck_floor(world, carrier);
    let (o, a) = match deck {
        None => before.unwrap_or((o, a)),
        Some(_) => (o, a),
    };
    let mut f = frame(world);
    let Some(ps) = f.occupant_mut(peer) else {
        return;
    };
    let l = to_local(ps.origin, o, a);
    // Only between its walls: one beside it (in its box) stays on the road.
    // Nor one on a door well's steps: he climbs them (the first is 20 up,
    // a hop, as BO2's step is 18). Held, he was put up on the aisle from
    // the bottom step, and from the road in a jump at the door (a
    // playtest: "I was able to jump into the bus, even though it was too
    // high").
    let (lo, hi) = super::transit::BUS_INSIDE;
    if deck.is_none()
        && (!(0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) || super::transit::in_door_well(l))
    {
        return;
    }
    // On the bus's tilted floor his box clears it only so high.
    let floor = deck.unwrap_or_else(|| floor_z() + tilt_rest(a));
    let (_, _, u) = gsc_t6::math::facing_axes(a);
    let lift = (floor - l[2]) / u[2].max(0.5);
    if lift <= 0.0 {
        return;
    }
    ps.origin[2] += lift;
    ps.velocity[2] = ps.velocity[2].max(0.0);
    f.translate_player_area(peer, [0.0, 0.0, lift]);
}

/// BO2MP_T6_RIDE="x y z" (the bus's own space; default "0 0 80"): at 15 s
/// player 0 is put there in the first script vehicle that is a platform
/// (Tranzit's bus), once: a test of riding it.
fn ride_test(world: &mut World) {
    static AT: std::sync::OnceLock<Option<[f32; 3]>> = std::sync::OnceLock::new();
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(off) = *AT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_RIDE").ok()?;
        let n: Vec<f32> = v
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        Some(if n.len() == 3 {
            [n[0], n[1], n[2]]
        } else {
            [0.0, 0.0, 80.0]
        })
    }) else {
        return;
    };
    let zm = world.resource::<Zm>();
    // BO2MP_T6_RIDE_AT=<seconds>: when (default 15).
    let at = std::env::var("BO2MP_T6_RIDE_AT")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map_or(15_000, |s| (s * 1000.0) as i64);
    if zm.clock_millis < at || DONE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some((o, a)) = zm
        .ents
        .values()
        .find(|e| e.platform && e.class_tag == "script_vehicle")
        .map(|e| (e.origin, e.rot))
    else {
        return;
    };
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (f, r, u) = gsc_t6::math::facing_axes(a);
    let to: [f32; 3] =
        std::array::from_fn(|i| o[i] + f[i] * off[0] - r[i] * off[1] + u[i] * off[2]);
    diag::info!(
        Sim,
        "bo2zm t6 ride test: player 0 put in the bus at ({:.0} {:.0} {:.0})",
        to[0],
        to[1],
        to[2]
    );
    super::teleport_player(world, ClientId(0), to);
    // BO2MP_T6_RIDE_YAW=<degrees>: he faces that way from the bus's front.
    let yaw: f32 = std::env::var("BO2MP_T6_RIDE_YAW")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0);
    super::set_player_view(world, ClientId(0), [0.0, a[1] + yaw, 0.0]);
}
