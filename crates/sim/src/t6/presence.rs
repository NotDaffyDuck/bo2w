//! Script models on screen: every entity with a model gets a script mover
//! (a networked entity plus a model row the snapshot carries, as MW2's
//! script models do); its pose, visibility and attachments follow the
//! entity every frame, and it goes when the entity does.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;

use super::{Zm, frame};
use crate::ScriptModelId;
use crate::bullet_collision::{AuthorityDObjState, EntityCollisionCapabilities, MASK_PLAYER_SOLID};

/// Spawned script movers for BO2 entities: inside the spawned range the
/// client spawns models for, clear of the old engine's own.
const T6_PRESENCE_BASE: u32 = 0x6000_0000;

/// A living zombie's box as legacy packs an entity's `solid` (half-width 15,
/// 1 below its feet, 72 tall: BO2's actor size, as a player's): players
/// cannot walk through zombies.
const ZOMBIE_SOLID: i32 = ((72 + 32) << 16) | (1 << 8) | 15;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub id: ScriptModelId,
    pub number: i32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub model: String,
    pub attachments: Vec<(String, String)>,
    pub hidden: bool,
    /// It moved last tick: its client carries it on at that speed until it
    /// is told it stopped.
    pub moving: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Presences {
    pub by_ent: BTreeMap<u32, Shown>,
    pub next: u32,
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 0.01)
}

/// Rows for a zbarrier's pieces: the entity's number shifted, the piece
/// in the low bits, this bit set.
const PIECE_BIT: u32 = 0x8000_0000;

/// Loops the scripts play on entities (`playloopsound`: the box's tune, a
/// perk machine's hum, a power-up's hum), each where its entity stands,
/// owned by its model row (or its number, for an entity with no model).
pub(super) fn publish_loops(world: &mut World) {
    let engines = super::engine_sounds::rows(world);
    let mut rows: Vec<(ScriptModelId, String, [f32; 3], f32, f32)> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter_map(|(n, e)| {
                let alias = e.loop_sound.as_ref()?.to_ascii_lowercase();
                let owner = zm.presences.by_ent.get(n).map_or_else(
                    || ScriptModelId::from_wire(0x2000_0000 | (n & 0x0fff_ffff)),
                    |s| s.id,
                );
                Some((owner, alias, e.origin, 1.0, 1.0))
            })
            .collect()
    };
    // Helicopter and drone engines (BO2's client script's loops), owned by
    // the vehicle's model row as its other loops are.
    {
        let zm = world.resource::<Zm>();
        for (n, alias, origin, volume, pitch) in engines {
            let owner = zm.presences.by_ent.get(&n).map_or_else(
                || ScriptModelId::from_wire(0x2000_0000 | (n & 0x0fff_ffff)),
                |s| s.id,
            );
            rows.push((owner, alias.to_owned(), origin, volume, pitch));
        }
    }
    let mut f = frame(world);
    let rows = rows
        .into_iter()
        .map(
            |(owner, alias, origin, volume, pitch)| crate::world_objects::DestructibleLoopSound {
                owner,
                alias_index: f.sound_alias_index(&alias),
                origin,
                volume: crate::world_objects::DestructibleLoopSound::hundredths(volume),
                pitch: crate::world_objects::DestructibleLoopSound::hundredths(pitch),
            },
        )
        .collect();
    f.world_objects_mut().set_destructible_loop_sounds(rows);
}

/// A model that is only something to bump into (Treyarch names them
/// `collision_*`, `zm_collision_*`; their material never draws in BO2):
/// kept solid, never shown.
/// A collision wall's half-size from its name (`collision_wall_128x128x10_
/// standard`: width x height x thickness, centred on its origin; local x,
/// z, y). These models carry no collision surfaces and only a cube for
/// bounds; Nuketown's script builds a closed box from six of them, which
/// fixes the axes.
fn named_wall_size(model: &str) -> Option<[f32; 3]> {
    if !model.starts_with("collision_") || !(model.contains("_wall_") || model.contains("_player_"))
    {
        return None;
    }
    let dims = model.split('_').find_map(|part| {
        let v: Vec<f32> = part.split('x').filter_map(|d| d.parse().ok()).collect();
        (v.len() == 3).then_some(v)
    })?;
    Some([dims[0] * 0.5, dims[2] * 0.5, dims[1] * 0.5])
}

/// What a collision model stops players with: boxes in its own axes, none
/// when players walk through it. BO2's own data (common_mp and patch_mp,
/// read with t6props): every `collision_clip_*` collision map is contents
/// 0x20030200 (player clip), every `collision_physics_*` one 0x20000400
/// (item clip: only items and physics objects stop), `collision_nosight_*`
/// is AI sight and `collision_missile_*` missiles only; Slums' curved wall,
/// Takeoff's solar panels and Nightclub's desk are missile or bullet
/// surfaces. Their bounds are cubes of the widest side, so a piece's box
/// comes from its name (`_AxBxC`: A x B x C along local x, y, z); Magma's
/// barrier slabs (64x64x10) were 64-unit cubes and Slums' physics pieces
/// walled players off. `None`: not one of these, the caller's own rule.
fn named_player_boxes(
    f: &crate::frame::FrameWorld<'_>,
    model: &str,
) -> Option<Vec<([f32; 3], [f32; 3])>> {
    let centred = |half: [f32; 3]| vec![(half.map(|v| -v), half)];
    let dims = |part: &str| -> Vec<f32> { part.split('x').filter_map(|d| d.parse().ok()).collect() };
    if ["collision_physics_", "collision_nosight_", "collision_missile_"]
        .iter()
        .any(|p| model.starts_with(p))
    {
        return Some(Vec::new());
    }
    if let Some(rest) = model.strip_prefix("collision_clip_") {
        if let Some(half) = named_wall_size(model) {
            return Some(centred(half));
        }
        // The ramp's one brush (its slope side is not kept).
        if rest == "ramp" {
            return Some(vec![([-128.0, -53.0, -12.0], [127.0, 42.0, 14.0])]);
        }
        // Their brushes' boxes (a cylinder's 12 and a sphere's extra sides
        // are not kept).
        if let Some(d) = rest.strip_prefix("cylinder_").map(dims)
            && d.len() == 2
        {
            return Some(centred([d[0] * 0.5, d[0] * 0.5, d[1] * 0.5]));
        }
        if let Some(d) = rest.strip_prefix("sphere_").map(dims)
            && d.len() == 1
        {
            return Some(centred([d[0] * 0.5; 3]));
        }
        let d = dims(rest);
        return (d.len() == 3).then(|| centred([d[0] * 0.5, d[1] * 0.5, d[2] * 0.5]));
    }
    // Frost's kitchen piece: three player-clip brushes (patch_mp), not its
    // bullet surface's bounds.
    if model == "collision_mp_frost_kitchen_weap" {
        return Some(vec![
            ([106.0, -3.0, 0.0], [134.0, 28.0, 20.0]),
            ([14.0, 0.0, 0.0], [106.0, 28.0, 20.0]),
            ([-28.0, -38.0, 0.0], [0.0, -16.0, 20.0]),
        ]);
    }
    // A model solid by its collision surfaces alone stops players only when
    // their contents do.
    let cap = f.model_capability(model).flatten()?;
    if cap.coll_surfs.is_empty() || !model.starts_with("collision_") {
        return None;
    }
    let contents = cap.coll_surfs.iter().fold(cap.contents.unwrap_or(0), |c, s| c | s.contents);
    if contents & crate::bullet_collision::MASK_PLAYER_SOLID == 0 {
        return Some(Vec::new());
    }
    None
}

fn collision_only(model: &str) -> bool {
    model.starts_with("collision_")
        || model.starts_with("zm_collision_")
        || model.contains("_collision_")
}

/// BO2MP_T6_NEAR=<seconds>: at that time, every drawn entity within 250
/// units of player 0 (debugging what stands in view).
fn report_near(world: &mut World, now_ms: i64) {
    static AT: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_NEAR")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
    }) else {
        return;
    };
    if !(at * 1000..at * 1000 + i64::from(crate::MATCH_TICK_MS)).contains(&now_ms) {
        return;
    }
    let Some(me) = frame(world)
        .player(crate::world::ClientId(0))
        .map(|p| p.origin)
    else {
        return;
    };
    let zm = world.resource::<Zm>();
    for (n, s) in &zm.presences.by_ent {
        let d = ((s.origin[0] - me[0]).powi(2)
            + (s.origin[1] - me[1]).powi(2)
            + (s.origin[2] - me[2]).powi(2))
        .sqrt();
        if d < 250.0 {
            let class = zm.ents.get(n).map_or("", |e| e.classname.as_str());
            diag::info!(
                Sim,
                "bo2zm t6 near: ent{n} {class} model {} hidden {} at {:.0} ({:.0} {:.0} {:.0}) attached {:?}",
                s.model,
                s.hidden,
                d,
                s.origin[0],
                s.origin[1],
                s.origin[2],
                s.attachments
            );
        }
    }
}

/// The level restarts (`map_restart`): every model it showed goes (a car
/// blown up last round comes back whole); its serials keep counting.
pub(super) fn retire_all(world: &mut World) {
    let gone = std::mem::take(&mut world.resource_mut::<Zm>().presences.by_ent);
    let mut f = frame(world);
    for shown in gone.into_values() {
        f.remove_script_mover_by_number(shown.number);
        f.remove_collision_owner(shown.id);
    }
}

pub(super) fn sync(world: &mut World, now_ms: i64) {
    report_near(world, now_ms);
    let now = now_ms as i32;
    let zm = world.resource::<Zm>();
    let pieces: Vec<(
        u32,
        ([f32; 3], [f32; 3], String, Vec<(String, String)>, bool),
    )> = zm
        .ents
        .iter()
        .filter_map(|(n, e)| e.zbarrier.as_ref().map(|z| (*n, e, z)))
        .flat_map(|(n, e, z)| {
            z.pieces.iter().enumerate().filter_map(move |(i, p)| {
                // A weapon-rise piece (a bare tag) shows the box's
                // spinning gun while it opens, where its clip's root
                // motion has lifted it, turned as the box turns the gun
                // it picks (the chest's angles + 180 yaw).
                let anim = zm.anims.get(&p.open_anim);
                if let Some((gun, off)) = super::zbarrier::rise_gun(p, anim, now_ms) {
                    let (f, r, u) = gsc_t6::math::angle_vectors(e.angles);
                    let origin = std::array::from_fn(|c| {
                        e.origin[c] + f[c] * off[0] - r[c] * off[1] + u[c] * off[2]
                    });
                    let angles = [e.angles[0], e.angles[1] + 180.0, e.angles[2]];
                    return Some((
                        PIECE_BIT | (n << 4) | i as u32,
                        (origin, angles, gun, Vec::new(), e.hidden),
                    ));
                }
                if p.model.is_empty() || p.model == "tag_origin" {
                    return None;
                }
                Some((
                    PIECE_BIT | (n << 4) | i as u32,
                    (
                        e.origin,
                        e.angles,
                        p.model.clone(),
                        Vec::new(),
                        !p.shown || e.hidden,
                    ),
                ))
            })
        })
        .collect();
    // Collision models (a perk machine's zm_collision_perks1, Nuketown's
    // patch walls, some lying on their sides) block players as their own
    // bounds turned with them while the scripts keep them solid (fix list
    // 3: they were square boxes of their widest side, ignoring the turn: a
    // 128-unit block for each 10-unit patch wall, one by the bunker, one in
    // the middle of the map, and the walls on their sides stayed open).
    let mut blocker_ents: Vec<(u32, [f32; 3], [f32; 3], String)> = zm
        .ents
        .iter()
        .filter(|(_, e)| e.solid && collision_only(&e.model))
        .map(|(n, e)| (*n, e.origin, e.angles, e.model.clone()))
        .collect();
    // The Mystery Box is solid where it stands (any box piece shown there:
    // the box, or the closed one): before, he walked through it.
    blocker_ents.extend(zm.ents.iter().filter_map(|(n, e)| {
        let z = e.zbarrier.as_ref()?;
        let piece = z
            .pieces
            .iter()
            .find(|p| p.shown && p.model.contains("magic_box"))?;
        (!e.hidden).then(|| (*n, e.origin, e.angles, piece.model.clone()))
    }));
    let mut wanted: BTreeMap<u32, ([f32; 3], [f32; 3], String, Vec<(String, String)>, bool)> = zm
        .ents
        .iter()
        .filter(|(n, e)| {
            e.obj.is_some()
                && !e.model.is_empty()
                && !e.model.starts_with('*')
                // Spawners are never drawn (the actors they spawn are).
                && (!e.classname.starts_with("actor_") || zm.actors.contains_key(n))
        })
        .map(|(n, e)| {
            (
                *n,
                (
                    e.origin,
                    e.angles,
                    e.model.clone(),
                    e.attached.clone(),
                    e.hidden || e.invisible_to_all || collision_only(&e.model),
                ),
            )
        })
        .collect();
    wanted.extend(pieces);
    // Retire the gone.
    let gone: Vec<(u32, Shown)> = world
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .filter(|(n, _)| !wanted.contains_key(n))
        .map(|(n, s)| (*n, s.clone()))
        .collect();
    for (n, shown) in gone {
        let mut f = frame(world);
        f.remove_script_mover_by_number(shown.number);
        f.remove_collision_owner(shown.id);
        world.resource_mut::<Zm>().presences.by_ent.remove(&n);
    }
    for (n, (origin, angles, model, attachments, hidden)) in wanted {
        // codmp: a breakable's map entity names the editor's stand-in
        // ("veh_t6_civ_van_sprinter"); its definition names the model the
        // game shows and is solid with (".._whole").
        let def_model = {
            let zm = world.resource::<Zm>();
            zm.ents
                .get(&n)
                .and_then(|e| e.destructible.as_deref())
                .and_then(|name| zm.destructibles.get(name))
                .map(|d| d.model.clone())
        };
        let existing = world.resource::<Zm>().presences.by_ent.get(&n).cloned();
        let mut shown = match existing {
            Some(s) => s,
            None => {
                let serial = {
                    let mut zm = world.resource_mut::<Zm>();
                    let s = zm.presences.next;
                    zm.presences.next += 1;
                    s
                };
                let id = ScriptModelId::from_wire(T6_PRESENCE_BASE + serial);
                let mut f = frame(world);
                let Ok(number) = f.spawn_script_mover(id, origin, angles) else {
                    diag::warn!(Sim, "bo2zm t6: no free entity for model {model}");
                    continue;
                };
                let capability = f.model_capability(&model).flatten();
                let shape = model_shape(&f, &model, def_model.as_deref(), origin, angles);
                if let Some(b) = shape.first() {
                    diag::info!(
                        Sim,
                        "codmp t6 solid model ent{n} {model} ({}) brush model {} at {origin:?}",
                        def_model.as_deref().unwrap_or("-"),
                        b.cmodel_handle
                    );
                }
                f.insert_collision_owner(EntityCollisionCapabilities::current_tick(
                    crate::AuthorityModelOwner::ScriptModel(id),
                    Some(AuthorityDObjState::at_pose(
                        &model, capability, origin, angles,
                    )),
                    shape,
                ));
                Shown {
                    id,
                    number,
                    origin,
                    angles,
                    model: String::new(),
                    attachments: Vec::new(),
                    hidden: !hidden,
                    moving: false,
                }
            }
        };
        let (yaw_goal, holds_still) = world
            .resource::<Zm>()
            .actors
            .get(&n)
            .filter(|a| a.alive)
            .map_or((None, false), |a| {
                let still = matches!(a.script.as_str(), "combat" | "stop")
                    && a.scripted.is_none()
                    && a.traverse.is_none();
                (a.yaw_goal, still)
            });
        let prev_yaw = shown.angles[1];
        let mut f = frame(world);
        let moved = !near(shown.origin, origin) || !near(shown.angles, angles);
        // A move that ended goes out once more, standing still (else the
        // client keeps it going: perk machines flew off into the sky).
        if moved || shown.moving || yaw_goal.is_some() {
            shown.moving = moved;
            f.set_script_mover_pose(shown.number, now, origin, angles);
            // bo2zm M3 fix list 2: a turning zombie's client turns it toward
            // the facing it is turning to at the server's rate and stops
            // there, as the next tick will. Carried on at the last tick's
            // turn instead, the body swung past the end of every turn (up to
            // 12 degrees) and snapped back.
            // A zombie that has stopped (attacking, standing) moves no
            // further next tick: it is drawn where it stands, not carried on
            // a few units past its stop and pulled back.
            if holds_still && let Some(mover) = f.script_mover_mut_by_number(shown.number) {
                let state = &mut mover.state;
                state.lerp.pos.tr_type = entity_t6::trajectory::tr_type::STATIONARY;
                state.lerp.pos.tr_time = now;
                state.lerp.pos.tr_base = origin;
                state.lerp.pos.tr_delta = [0.0; 3];
                state.lerp.pos.tr_duration = 0;
            }
            // BO2MP_T6_NOTURNGOAL=1: the old carry-on, to compare (test aid).
            let use_goal = std::env::var_os("BO2MP_T6_NOTURNGOAL").is_none();
            if use_goal
                && let Some(goal) = yaw_goal
                && let Some(mover) = f.script_mover_mut_by_number(shown.number)
            {
                // From the facing it had last tick (where his client shows it
                // now): the turn trails the server by a tick, so it starts
                // smoothly instead of jumping a whole tick's turn (18 degrees).
                let from = [angles[0], prev_yaw, angles[2]];
                let left = game_math::angle_subtract(goal, from[1]);
                let state = &mut mover.state;
                if left.abs() > 0.05 {
                    let rate = super::actors::TURN_DEG_PER_S;
                    state.lerp.apos.tr_type = entity_t6::trajectory::tr_type::LINEAR_STOP;
                    state.lerp.apos.tr_time = now;
                    state.lerp.apos.tr_base = from;
                    state.lerp.apos.tr_delta = [0.0, rate.copysign(left), 0.0];
                    state.lerp.apos.tr_duration = ((left.abs() / rate) * 1000.0).round().max(1.0) as i32;
                } else {
                    state.lerp.apos.tr_type = entity_t6::trajectory::tr_type::STATIONARY;
                    state.lerp.apos.tr_time = now;
                    state.lerp.apos.tr_base = angles;
                    state.lerp.apos.tr_delta = [0.0; 3];
                    state.lerp.apos.tr_duration = 0;
                }
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                place_row(row, origin, angles);
            }
            shown.origin = origin;
            shown.angles = angles;
        }
        if shown.hidden != hidden {
            if let Some(mover) = f.script_mover_mut_by_number(shown.number) {
                if hidden {
                    mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
                } else {
                    mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
                }
                mover.nonsolid = true;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                row.hidden = hidden;
                row.solid = false;
            }
            // codmp: a car stands solid while it is drawn (its model's
            // collision-map brushes); hidden, players pass.
            let solid = !hidden
                && f.collision_owner_mut(shown.id)
                    .is_some_and(|row| !row.linked_brushes.is_empty());
            if solid {
                if let Some(mover) = f.script_mover_mut_by_number(shown.number) {
                    mover.nonsolid = false;
                }
                if let Some(row) = f.collision_owner_mut(shown.id) {
                    row.solid = true;
                }
            }
            shown.hidden = hidden;
        }
        if shown.model != model || shown.attachments != attachments {
            let capability = f.model_capability(&model).flatten();
            // codmp: the new model's solid shape (a burnt-out car's husk).
            let shape = model_shape(&f, &model, def_model.as_deref(), origin, angles);
            let solid = !hidden && !shape.is_empty();
            if let Some(mover) = f.script_mover_mut_by_number(shown.number) {
                mover.nonsolid = !solid;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                row.linked_brushes = shape;
                row.solid = solid;
                match row.dobj.as_mut() {
                    Some(dobj) => {
                        if dobj.current_model != model {
                            dobj.replace_model(&model, capability);
                        }
                    }
                    None => {
                        row.dobj = Some(AuthorityDObjState::at_pose(
                            &model, capability, origin, angles,
                        ))
                    }
                }
                if let Some(dobj) = row.dobj.as_mut() {
                    let a: Vec<(&str, &str)> = attachments
                        .iter()
                        .map(|(m, t)| (m.as_str(), t.as_str()))
                        .collect();
                    dobj.set_attachments(&a);
                }
            }
            shown.model = model;
            shown.attachments = attachments;
        }
        // bo2mp: a map destructible (a mannequin, a car) breaks by its
        // definition; its shown pieces stop bullets.
        let destructible = (n & PIECE_BIT == 0)
            .then(|| {
                let zm = world.resource::<Zm>();
                let name = zm.ents.get(&n)?.destructible.as_ref()?;
                zm.destructibles.get(name).cloned()
            })
            .flatten();
        if let Some(def) = &destructible {
            let mut f = frame(world);
            if let Some(row) = f.collision_owner_mut(shown.id)
                && let Some(dobj) = row.dobj.as_mut()
                && !crate::t5_destructible::installed(dobj, &def.name)
            {
                crate::t5_destructible::install_hosted(dobj, def.clone());
                diag::info!(
                    Sim,
                    "bo2mp destructible {} ent{n} at {:?}",
                    def.name,
                    shown.origin
                );
            }
        }
        // Actors: bullets hit the living (their posed hit boxes), and the
        // playing animation goes out as the model row's one leaf.
        let alive_actor = world
            .resource::<Zm>()
            .actors
            .get(&n)
            .is_some_and(|a| a.alive);
        let boxes = if alive_actor && !hidden {
            let f = frame(world);
            let placement = f
                .entity_collision_capabilities()
                .iter()
                .find(|row| row.owner.script_model() == Some(shown.id))
                .and_then(|row| row.dobj.as_ref())
                .map(|d| d.world_from_model);
            placement.and_then(|m| super::actors::hitboxes(world, n, now_ms, m))
        } else {
            None
        };
        // bo2mp: a scorestreak's plane, drone, vehicle or turret is hit by
        // bullets on its model's own hit boxes, as the engine traces any
        // entity's DObj: before, every shot went through the RC-XD.
        let streak = !alive_actor && n & PIECE_BIT == 0 && super::killstreaks::damage_target(world, n);
        {
            let mut f = frame(world);
            let packed = if boxes.is_some() { ZOMBIE_SOLID } else { 0 };
            if let Some(mover) = f.script_mover_mut_by_number(shown.number)
                && mover.state.solid != packed
            {
                mover.state.solid = packed;
            }
            let shape = f
                .collision_owner_mut(shown.id)
                .is_some_and(|row| !row.linked_brushes.is_empty())
                && !hidden;
            if shape && let Some(mover) = f.script_mover_mut_by_number(shown.number) {
                mover.nonsolid = false;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                if destructible.is_some() && !alive_actor {
                    // Its bones as posed (bullet_collision materialize).
                    row.solid = !hidden;
                } else if streak {
                    row.solid = !hidden;
                    // It may have become one after the branch below marked
                    // its (empty) collision as built: build it again.
                    if let Some(dobj) = row.dobj.as_mut()
                        && dobj.current_collision.is_none()
                    {
                        dobj.materialized_pose_revision = None;
                    }
                } else {
                    row.solid = boxes.is_some() || shape;
                    if let Some(dobj) = row.dobj.as_mut() {
                        dobj.current_collision = boxes.map(|bones| {
                            crate::bullet_collision::AuthorityDObjCollision { bones, coll: None }
                        });
                        dobj.materialized_model_revision = Some(dobj.model_revision);
                        dobj.materialized_pose_revision = Some(dobj.pose_revision);
                    }
                }
            }
        }
        let anim = if n & PIECE_BIT != 0 {
            let (e, i) = ((n & !PIECE_BIT) >> 4, (n & 0xf) as usize);
            world
                .resource::<Zm>()
                .ents
                .get(&e)
                .and_then(|e| e.zbarrier.as_ref())
                .and_then(|z| z.pieces.get(i))
                .and_then(|p| super::zbarrier::piece_pose(p, now_ms).1)
                .map(|(c, t, r)| vec![(c, t, r, 1.0)])
                .unwrap_or_default()
        } else {
            match super::actors::present_anim(world, n, now_ms) {
                Some((c, t, r)) => vec![(c, t, r, 1.0)],
                None => super::mp_fxanims::present(world, n, now_ms),
            }
        };
        if !anim.is_empty() {
            let mut f = frame(world);
            if let Some(row) = f.collision_owner_mut(shown.id)
                && let Some(dobj) = row.dobj.as_mut()
            {
                use xmodel_runtime::{
                    XAnimNodeId, XAnimNodeState, XAnimSemanticNode, XAnimSemanticNodeKind, XAnimTreeSnapshot,
                };
                let leaf = |parent, (clip, time, rate, weight): (String, f32, f32, f32)| XAnimSemanticNode {
                    parent,
                    kind: XAnimSemanticNodeKind::Leaf,
                    clip: Some(clip),
                    parts: None,
                    state: XAnimNodeState {
                        time,
                        old_time: time,
                        weight,
                        goal_weight: weight,
                        rate,
                        ..XAnimNodeState::default()
                    },
                };
                // Two animations while a map script blends one into the next
                // (`clearanim( old, 0.5 )` then `animscripted( new, 1.0, 0.5,
                // 1.0 )`: Studio's brontosaurus, mp_studio_fx.csc), under one
                // blend node.
                let nodes = if anim.len() == 1 {
                    anim.into_iter().map(|a| leaf(None, a)).collect()
                } else {
                    let root = XAnimSemanticNode {
                        parent: None,
                        kind: XAnimSemanticNodeKind::Blend,
                        clip: None,
                        parts: None,
                        state: XAnimNodeState { weight: 1.0, goal_weight: 1.0, ..XAnimNodeState::default() },
                    };
                    std::iter::once(root).chain(anim.into_iter().map(|a| leaf(Some(XAnimNodeId(0)), a))).collect()
                };
                let rev = dobj.semantic_state.pose_revision.wrapping_add(1);
                dobj.semantic_state.pose_revision = rev;
                dobj.semantic_state.tree = Some(XAnimTreeSnapshot { definition_revision: 1, state_revision: rev, nodes });
            }
        }
        world.resource_mut::<Zm>().presences.by_ent.insert(n, shown);
    }
    // The collision models as turned boxes, for his movement (the server's
    // and his client's prediction: they go out in the snapshot).
    let mut f = frame(world);
    let blockers: Vec<crate::OrientedBlocker> = blocker_ents
        .iter()
        .flat_map(|(n, origin, angles, model)| {
            let (fw, rt, up) = gsc_t6::math::angle_vectors(*angles);
            let axis = [fw, [-rt[0], -rt[1], -rt[2]], up];
            // A collision model stops him the way BO2 traces it
            // (SV_TraceCapsuleToEntity): as its physics collmap's pieces,
            // and only when their contents meet his trace mask, so the
            // physics-only ones (collision_physics_*, item clip) stop thrown
            // things, not him. Before, each was a box of the model's bounds:
            // a 128-unit cube for collision_physics_32x32x128.
            let collmap = collision_only(model)
                .then(|| {
                    f.model_capability(model)
                        .flatten()
                        .and_then(|c| c.collmap.clone())
                })
                .flatten();
            if let Some(collmap) = collmap {
                if collmap.contents & MASK_PLAYER_SOLID == 0 {
                    return Vec::new();
                }
                let turn = |v: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|k| {
                        axis[0][k] * v[0] + axis[1][k] * v[1] + axis[2][k] * v[2]
                    })
                };
                return collmap
                    .boxes
                    .iter()
                    .map(|b| {
                        let center = turn(b.center);
                        crate::OrientedBlocker {
                            entnum: *n as u16,
                            origin: std::array::from_fn(|k| origin[k] + center[k]),
                            axis: b.axis.map(turn),
                            mins: b.half.map(|v| -v),
                            maxs: b.half,
                        }
                    })
                    .collect();
            }
            // Without a collmap: its pieces by its name, a wall's size or
            // the model's bounds.
            let boxes = named_player_boxes(&f, model).unwrap_or_else(|| match named_wall_size(model) {
                Some(half) => vec![(half.map(|v| -v), half)],
                None => {
                    // The model's bounds come as middle and half-size.
                    f.model_capability(model)
                        .flatten()
                        .and_then(|c| c.bounds)
                        .map(|(mid, half)| {
                            (
                                std::array::from_fn(|i| mid[i] - half[i]),
                                std::array::from_fn(|i| mid[i] + half[i]),
                            )
                        })
                        .into_iter()
                        .collect()
                }
            });
            boxes
                .into_iter()
                .map(|(lo, hi)| crate::OrientedBlocker {
                    entnum: *n as u16,
                    origin: *origin,
                    axis,
                    mins: lo,
                    maxs: hi,
                })
                .collect()
        })
        .collect();
    // BO2MP_T6_BLOCKERLOG=1: each blocker when the list changes (test aid).
    if std::env::var_os("BO2MP_T6_BLOCKERLOG").is_some()
        && f.oriented_blockers() != blockers.as_slice()
    {
        for b in &blockers {
            diag::info!(
                Sim,
                "bo2zm t6 blocker ent{} at ({:.0} {:.0} {:.0}) fwd ({:.2} {:.2} {:.2}) up ({:.2} {:.2} {:.2}) bounds {:?}..{:?}",
                b.entnum,
                b.origin[0],
                b.origin[1],
                b.origin[2],
                b.axis[0][0],
                b.axis[0][1],
                b.axis[0][2],
                b.axis[2][0],
                b.axis[2][1],
                b.axis[2][2],
                b.mins,
                b.maxs
            );
        }
    }
    f.set_oriented_blockers(blockers);
    drop(f);
    enemy_models(world);
}

/// bo2mp: the snapshot's `setenemymodel` rows: each drawn entity with a
/// model for the other side, with its side and owner as BO2 reads them
/// (`CG_WhatModelShouldLocalPlayerSee`): an actor's `aiteam` and
/// `setentityowner`; a vehicle someone drives, its driver's team; else its
/// own (`setteam`, `setowner`, its spawn's owner).
fn enemy_models(world: &mut World) {
    let found: Vec<(u32, ScriptModelId, String, i32, Option<u32>, Option<u32>)> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| !e.enemy_model.is_empty())
            .filter_map(|(n, e)| {
                let shown = zm.presences.by_ent.get(n)?;
                let (team, owner) = match zm.actors.get(n) {
                    Some(a) => (
                        super::objectives::team_number(&a.team)
                            .map_or(entity_t6::team::FREE, super::objectives::client_team),
                        a.owner,
                    ),
                    None => (e.faction_team, e.faction_owner),
                };
                Some((*n, shown.id, e.enemy_model.clone(), team, owner, None))
            })
            .collect()
    };
    let found: Vec<_> = found
        .into_iter()
        .map(|(n, id, model, team, owner, _)| {
            (n, id, model, team, owner, super::killstreaks::driver_of(world, n))
        })
        .collect();
    let mut f = frame(world);
    let rows: Vec<crate::EnemyModel> = found
        .into_iter()
        .map(|(n, id, model, team, owner, driver)| {
            let team = driver
                .and_then(|d| f.client_meta(crate::ClientId(d)))
                .map_or(team, |m| m.client_state_team);
            if !f.enemy_models().iter().any(|r| r.id == id && r.model == model) {
                let has = f.model_capability(&model).flatten().is_some();
                diag::info!(
                    Sim,
                    "bo2mp t6 enemy model ent{n} {model} team {team} owner {owner:?} (in his assets: {has})"
                );
            }
            crate::EnemyModel { id, model, team, owner }
        })
        .collect();
    f.set_enemy_models(rows);
}

/// A model that moved: its drawn pose, every collision-map brush it is
/// solid with, and the pose the snapshot follower compares against, all at
/// the new spot (bo2mp: only the last moved before, so `follow_movers` saw
/// no change and the brushes stayed where the model loaded: Studio's
/// sliding targets and Vertigo's glass doors blocked players at their first
/// spot and let them through where they were).
fn place_row(row: &mut EntityCollisionCapabilities, origin: [f32; 3], angles: [f32; 3]) {
    if let Some(dobj) = row.dobj.as_mut() {
        dobj.set_world_pose(origin, angles);
    }
    for brush in &mut row.linked_brushes {
        brush.origin = origin;
        brush.angles = angles;
    }
    row.followed_pose = Some((origin, angles));
}

/// codmp: the brush model a Black Ops II model is solid with (a car's
/// collision maps), placed where the model stands; none for most models.
fn model_shape(
    f: &crate::frame::FrameWorld<'_>,
    model: &str,
    def_model: Option<&str>,
    origin: [f32; 3],
    angles: [f32; 3],
) -> Vec<crate::bullet_collision::LinkedBrushCollisionBrush> {
    let cmodels = &f.clip_cmodels().model_cmodels;
    cmodels
        .get(model)
        .or_else(|| def_model.and_then(|d| cmodels.get(d)))
        .map(|&cmodel_handle| crate::bullet_collision::LinkedBrushCollisionBrush {
            cmodel_handle,
            origin,
            angles,
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bullet_collision::LinkedBrushCollisionBrush;
    use crate::gentity::ScriptMoverGentity;

    fn brush_pose(row: &EntityCollisionCapabilities) -> ([f32; 3], [f32; 3]) {
        (row.linked_brushes[0].origin, row.linked_brushes[0].angles)
    }

    #[test]
    fn a_moved_model_takes_its_collision_along() {
        let id = ScriptModelId::from_wire(T6_PRESENCE_BASE + 7);
        let start = ([10.0, 20.0, 0.0], [0.0, 0.0, 0.0]);
        let mut row = EntityCollisionCapabilities::current_tick(
            crate::AuthorityModelOwner::ScriptModel(id),
            None,
            vec![LinkedBrushCollisionBrush {
                cmodel_handle: 3,
                origin: start.0,
                angles: start.1,
            }],
        );
        row.followed_pose = Some(start);
        // Studio's target slides 240 units and turns.
        let to = ([250.0, 20.0, 0.0], [0.0, 90.0, 0.0]);
        place_row(&mut row, to.0, to.1);
        assert_eq!(brush_pose(&row), to);
        assert_eq!(row.followed_pose, Some(to));
        // The snapshot follower, with the mover standing there, keeps it.
        let mut mover = ScriptMoverGentity {
            id,
            ..Default::default()
        };
        mover.state.lerp.pos.tr_base = to.0;
        mover.state.lerp.apos.tr_base = to.1;
        let mut rows = [row];
        crate::presence::follow_movers(&mut rows, &[mover], 1000);
        assert_eq!(brush_pose(&rows[0]), to);
    }
}
