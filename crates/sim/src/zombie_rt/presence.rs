//! Script models on screen: every entity with a model gets a script mover
//! (a networked entity plus a model row the snapshot carries, as MW2's
//! script models do); its pose, visibility and attachments follow the
//! entity every frame, and it goes when the entity does.

use super::pairs::KeepByValue;
use std::collections::BTreeMap;

use bevy_ecs::prelude::World;

use super::engine_sounds::EngineLoop;
use super::{Zm, frame};
use crate::ScriptModelId;
use crate::shot_trace::{ServerModelState, EntityHitAbilities, PLAYER_BODY_MASK};

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
    pub num: i32,
    pub spot: [f32; 3],
    pub rot: [f32; 3],
    pub mesh: String,
    pub attachments: Vec<(String, String, Option<xmodel_runtime::TagOffset>)>,
    pub unseen: bool,
    /// Hidden from some players only (`setinvisibletoplayer`): a bit per
    /// player it is still drawn for. 0 for everything else.
    pub shown_to: u64,
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
                let alias = e.repeating_audio.as_ref()?.to_ascii_lowercase();
                let owner = zm.presences.by_ent.get(n).map_or_else(
                    || ScriptModelId::decode_net(0x2000_0000 | (n & 0x0fff_ffff)),
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
        for EngineLoop { ent, alias, at, volume, pitch } in engines {
            let owner = zm.presences.by_ent.get(&ent).map_or_else(
                || ScriptModelId::decode_net(0x2000_0000 | (ent & 0x0fff_ffff)),
                |s| s.id,
            );
            rows.push((owner, alias.to_owned(), at, volume, pitch));
        }
    }
    let mut f = frame(world);
    let rows = rows
        .into_iter()
        .map(
            |(owner, alias, origin, volume, pitch)| crate::glass_and_props::DestructibleLoopSound {
                owner,
                alias_index: f.sound_alias_index(&alias),
                origin,
                volume: crate::glass_and_props::DestructibleLoopSound::hundredths(volume),
                pitch: crate::glass_and_props::DestructibleLoopSound::hundredths(pitch),
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
fn named_player_boxes(f: &crate::step_view::StepWorldView<'_>, model: &str) -> Option<Vec<([f32; 3], [f32; 3])>> {
    let centred = |half: [f32; 3]| vec![(half.map(|v| -v), half)];
    let dims = |part: &str| -> Vec<f32> { part.split('x').filter_map(|d| d.parse().ok()).collect() };
    // Stops no player at all.
    let passable = || Some(Vec::new());
    if ["collision_physics_", "collision_nosight_", "collision_missile_"]
        .iter()
        .any(|p| model.starts_with(p))
    {
        return passable();
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
        return Some(FROST_KITCHEN_CLIP.to_vec());
    }
    // A model solid by its collision surfaces alone stops players only when
    // their contents do.
    let cap = f.model_capability(model).flatten()?;
    if cap.collision_faces.is_empty() || !model.starts_with("collision_") {
        return None;
    }
    let contents = cap.collision_faces.iter().fold(cap.contents.unwrap_or(0), |c, s| c | s.contents);
    let stops_players = contents & crate::shot_trace::PLAYER_BODY_MASK != 0;
    if stops_players { None } else { passable() }
}

/// Frost's kitchen piece's three player-clip brushes (mins, maxs).
const FROST_KITCHEN_CLIP: [([f32; 3], [f32; 3]); 3] = [
    ([106.0, -3.0, 0.0], [134.0, 28.0, 20.0]),
    ([14.0, 0.0, 0.0], [106.0, 28.0, 20.0]),
    ([-28.0, -38.0, 0.0], [0.0, -16.0, 20.0]),
];

fn collision_only(model: &str) -> bool {
    model.starts_with("collision_")
        || model.starts_with("zm_collision_")
        || model.contains("_collision_")
}

/// BO2MP_T6_NEAR=<seconds>: at that time, every drawn entity within 250
/// units of player 0 (debugging what stands in view).
fn report_near(world: &mut World, clock_millis: i64) {
    static AT: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("BO2MP_T6_NEAR")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
    }) else {
        return;
    };
    if !(at * 1000..at * 1000 + i64::from(crate::SIM_STEP_MILLIS)).contains(&clock_millis) {
        return;
    }
    let Some(me) = frame(world)
        .avatar(crate::sim_parts::ClientId(0))
        .map(|p| p.origin)
    else {
        return;
    };
    let zm = world.resource::<Zm>();
    for (n, s) in &zm.presences.by_ent {
        let d = ((s.spot[0] - me[0]).powi(2)
            + (s.spot[1] - me[1]).powi(2)
            + (s.spot[2] - me[2]).powi(2))
        .sqrt();
        if d < 250.0 {
            let class = zm.ents.get(n).map_or("", |e| e.class_tag.as_str());
            diag::info!(
                Sim,
                "bo2zm t6 near: ent{n} {class} model {} hidden {} at {:.0} ({:.0} {:.0} {:.0}) attached {:?}",
                s.mesh,
                s.unseen,
                d,
                s.spot[0],
                s.spot[1],
                s.spot[2],
                s.attachments.iter().map(|(m, t, _)| (m, t)).collect::<Vec<_>>()
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
        f.remove_script_mover_by_number(shown.num);
        f.remove_collision_owner(shown.id);
    }
}

pub(super) fn sync(world: &mut World, clock_millis: i64) {
    report_near(world, clock_millis);
    let now = clock_millis as i32;
    let zm = world.resource::<Zm>();
    let pieces: Vec<(u32, ( [f32; 3], [f32; 3], String, Vec<(String, String, Option<xmodel_runtime::TagOffset>)>, bool, Option<Vec<String>>, ))> = zm
        .ents
        .iter()
        .filter_map(|(nn, en)| en.zbarrier.as_ref().map(|z| (*nn, en, z)))
        .flat_map(|(nn, en, z)| {
            z.pieces.iter().enumerate().filter_map(move |(i, pt)| {
                // A weapon-rise piece (a bare tag) shows the box's
                // spinning gun while it opens, where its clip_no's root
                // motion has lifted it, turned as the box turns the gun
                // it picks (the chest's rot + 180 yaw), wearing its
                // parts (magazine, scope, rocket) and hiding its hide tags.
                let lookup_key = PIECE_BIT | (nn << 4) | i as u32;
                let anim = zm.anims.get(&pt.open_anim);
                if let Some((gun, off)) = super::zbarrier::rise_gun(pt, anim, clock_millis) {
                    let (f, r, u) = gsc_t6::math::facing_axes(en.rot);
                    let spot = std::array::from_fn(|c| {
                        en.origin[c] + f[c] * off[0] - r[c] * off[1] + u[c] * off[2]
                    });
                    let rot = [en.rot[0], en.rot[1] + 180.0, en.rot[2]];
                    let parts = gun.parts.into_iter().map(|(m, t, at)| (m, t, Some(at)));
                    return Some((lookup_key, (spot, rot, gun.mesh, parts.collect(), en.unseen, Some(gun.hide))));
                }
                if pt.mesh.is_empty() || pt.mesh == "tag_origin" {
                    return None;
                }
                Some((
                    lookup_key,
                    (
                        super::zbarrier::piece_origin(zm, en, z, pt, clock_millis),
                        super::zbarrier::piece_angles(zm, en, z, pt, clock_millis),
                        pt.mesh.clone(),
                        Vec::new(),
                        !pt.shown
                            || en.unseen
                            || (pt.state == "open"
                                && z.def.as_ref().is_some_and(|d| d.auto_hide_open_pieces)),
                        None,
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
        .where_value(|en| en.solid && collision_only(&en.mesh))
        .map(|(nn, en)| (*nn, en.origin, en.rot, en.mesh.clone()))
        .collect();
    // The Mystery Box is solid where it stands (any box piece shown there:
    // the box, or the closed one): before, he walked through it.
    blocker_ents.extend(zm.ents.iter().filter_map(|(nn, en)| {
        let z = en.zbarrier.as_ref()?;
        let piece = z
            .pieces
            .iter()
            .find(|pt| pt.shown && pt.mesh.contains("magic_box"))?;
        (!en.unseen).then(|| (*nn, en.origin, en.rot, piece.mesh.clone()))
    }));
    // Tranzit's bus: its body as boxes (`transit`), and its ladder once
    // built (with the faces' surface flags).
    let platform_ents: Vec<(u32, [f32; 3], [f32; 3], Vec<(([f32; 3], [f32; 3]), u32)>)> = zm
        .ents
        .iter()
        .where_value(|en| en.platform && !en.unseen)
        .filter_map(|(nn, en)| {
            let boxes = super::transit::platform_boxes(&en.mesh);
            (!boxes.is_empty()).then(|| {
                let mut all: Vec<(([f32; 3], [f32; 3]), u32)> = boxes.iter().map(|&b| (b, 0)).collect();
                all.extend(super::transit::platform_ladder(zm, *nn, &en.mesh));
                (*nn, en.origin, en.rot, all)
            })
        })
        .collect();
    // Die Rise's elevator cars: their floor and roof (`highrise`), under
    // the car's own entity number as its clients know the car, so a
    // player's ground entity there names it (`platform_carry` draws his
    // ride smooth by it).
    let slab_ents: Vec<(u32, [f32; 3], [f32; 3], String, u16)> = zm
        .ents
        .iter()
        .where_value(|en| {
            en.platform && !en.unseen && !super::highrise::platform_slabs(&en.mesh).is_empty()
        })
        .map(|(nn, en)| {
            let number = zm
                .presences
                .by_ent
                .get(nn)
                .and_then(|s| u16::try_from(s.num).ok())
                .unwrap_or(*nn as u16);
            (*nn, en.origin, en.rot, en.mesh.clone(), number)
        })
        .collect();
    let mut wanted: BTreeMap<u32, ( [f32; 3], [f32; 3], String, Vec<(String, String, Option<xmodel_runtime::TagOffset>)>, bool, Option<Vec<String>>, )> = zm
        .ents
        .iter()
        .filter_map(|(nn, en)| {
            // A brush door (a map script_brushmodel's `*N`) draws its own
            // surfaces; brushes.rs gives it collision.
            let mdl = match &en.brush {
                Some(b) if en.mesh.is_empty() && en.class_tag == "script_brushmodel" => b,
                _ => &en.mesh,
            };
            (en.obj.is_some()
                && !mdl.is_empty()
                && (!mdl.starts_with('*') || en.class_tag == "script_brushmodel")
                // Spawners are never drawn (the actors they spawn are).
                && (!en.class_tag.starts_with("actor_") || zm.actors.contains_key(nn)))
            .then(|| (nn, en, mdl.clone()))
        })
        .map(|(nn, en, mdl)| {
            (
                *nn,
                (
                    en.origin,
                    en.rot,
                    mdl,
                    en.attached
                        .iter()
                        .map(|(m, t)| (m.clone(), t.clone(), None))
                        .chain(
                            en.weapon_parts
                                .iter()
                                .map(|(m, t, at)| (m.clone(), t.clone(), Some(*at))),
                        )
                        .collect(),
                    // Hidden from everyone. Hidden from some players only
                    // (Mob's afterlife doors while one is a ghost) is the
                    // mask below.
                    en.unseen || en.invisible_to_all || collision_only(&en.mesh),
                    // A breakable prop hides its own launched parts.
                    en.destructible.is_none().then(|| en.hidden_parts.clone()),
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
        .filter(|(nn, _)| !wanted.contains_key(nn))
        .map(|(nn, s)| (*nn, s.clone()))
        .collect();
    for (nn, shown) in gone {
        let mut f = frame(world);
        f.remove_script_mover_by_number(shown.num);
        f.remove_collision_owner(shown.id);
        world.resource_mut::<Zm>().presences.by_ent.remove(&nn);
    }
    // A bit per player in the match (a player's entity number is his client).
    let everyone: u64 = world
        .resource::<Zm>()
        .players
        .keys()
        .filter(|c| **c < 64)
        .fold(0, |m, c| m | (1 << c));
    for (nn, (spot, rot, mdl, attaches, hidden, hide)) in wanted {
        // codmp: a breakable's map entity names the editor's stand-in
        // ("veh_t6_civ_van_sprinter"); its definition names the mdl the
        // game shows and is solid with (".._whole").
        let def_model = {
            let zm = world.resource::<Zm>();
            zm.ents
                .get(&nn)
                .and_then(|en| en.destructible.as_deref())
                .and_then(|name| zm.destructibles.get(name))
                .map(|d| d.model.clone())
        };
        // BO2's `setinvisibletoplayer` hides it from those players only;
        // the rest still see it (before, only player 1's view counted).
        let mask: u64 = world.resource::<Zm>().ents.get(&nn).map_or(0, |en| {
            en.invisible_to
                .iter()
                .filter(|c| **c < 64)
                .fold(0, |m, c| m | (1 << c))
        }) & everyone;
        // Hidden from every player: no hit boxes either (as solo was).
        let hidden_all = hidden || (mask != 0 && mask == everyone);
        // Not drawn by default; shown back to the players in `shown_to`.
        let draw_hidden = hidden_all || mask != 0;
        let shown_to = if hidden_all || mask == 0 { 0 } else { everyone & !mask };
        let existing = world.resource::<Zm>().presences.by_ent.get(&nn).cloned();
        let mut shown = match existing {
            Some(kept) => kept,
            None => {
                let serial = {
                    let mut zm = world.resource_mut::<Zm>();
                    let s = zm.presences.next;
                    zm.presences.next += 1;
                    s
                };
                let id = ScriptModelId::decode_net(T6_PRESENCE_BASE + serial);
                let mut f = frame(world);
                let Ok(number) = f.create_scripted_mover(id, spot, rot) else {
                    diag::warn!(Sim, "bo2zm t6: no free entity for model {mdl}");
                    continue;
                };
                let caps = f.model_capability(&mdl).flatten();
                let shape = model_shape(&f, &mdl, def_model.as_deref(), spot, rot);
                if let Some(b) = shape.first() {
                    diag::info!(
                        Sim,
                        "codmp t6 solid model ent{nn} {mdl} ({}) brush model {} at {spot:?}",
                        def_model.as_deref().unwrap_or("-"),
                        b.collision_model_ref
                    );
                }
                // BO2MP_T6_PRESLOG=<part of a mdl name>: a line as it shows.
                if std::env::var("BO2MP_T6_PRESLOG").is_ok_and(|w| mdl.contains(w.as_str())) {
                    diag::info!(
                        Sim,
                        "bo2zm t6 presence ent{nn} {mdl} hidden={draw_hidden} capability={}",
                        caps.is_some()
                    );
                }
                let posed_obj = ServerModelState::at_pose(&mdl, caps, spot, rot);
                f.insert_collision_owner(EntityHitAbilities::now_tick(
                    crate::ServerModelSource::ScriptModel(id),
                    Some(posed_obj),
                    shape,
                ));
                Shown {
                    id,
                    num: number,
                    spot,
                    rot,
                    mesh: String::new(),
                    attachments: Vec::new(),
                    unseen: !draw_hidden,
                    shown_to: 0,
                    moving: false,
                }
            }
        };
        let (yaw_goal, holds_still) = world
            .resource::<Zm>()
            .actors
            .get(&nn)
            .filter(|a| a.living)
            .map_or((None, false), |a| {
                let still = matches!(a.script.as_str(), "combat" | "stop")
                    && a.scripted.is_none()
                    && a.traverse.is_none();
                (a.yaw_goal, still)
            });
        let prev_yaw = shown.rot[1];
        let is_vehicle = {
            let zm = world.resource::<Zm>();
            vehicle_borne(&zm, nn)
        };
        let mut f = frame(world);
        let moved = !near(shown.spot, spot) || !near(shown.rot, rot);
        // A move that ended goes out once more, standing still (else the
        // client keeps it going: perk machines flew off into the sky).
        if moved || shown.moving || yaw_goal.is_some() {
            shown.moving = moved;
            f.set_script_mover_pose(shown.num, now, spot, rot);
            // The last send of a move that ended stands still, facing where
            // it faces. Its turn was measured from the facing the turn below
            // started from (a tick behind), so a zombie killed centre-turn was
            // sent turning on with no end: its corpse spun until it went.
            if !moved && let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                let status = &mut mover.state;
                status.lerp.pos.tr_type = entity_t6::flight::path_kind::AT_REST;
                status.lerp.pos.began_at = now;
                status.lerp.pos.base_point = spot;
                status.lerp.pos.rate = [0.0; 3];
                status.lerp.pos.span_ms = 0;
                status.lerp.apos.tr_type = entity_t6::flight::path_kind::AT_REST;
                status.lerp.apos.began_at = now;
                status.lerp.apos.base_point = rot;
                status.lerp.apos.rate = [0.0; 3];
                status.lerp.apos.span_ms = 0;
            }
            // bo2zm M3 fix list 2: a turning zombie's client turns it toward
            // the facing it is turning to at the server's rate and stops
            // there, as the next tick will. Carried on at the last tick's
            // turn instead, the body swung past the end of every turn (up to
            // 12 degrees) and snapped back.
            // A zombie that has stopped (attacking, standing) moves no
            // further next tick: it is drawn where it stands, not carried on
            // a few units past its stop and pulled back.
            if holds_still && let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                let status = &mut mover.state;
                status.lerp.pos.tr_type = entity_t6::flight::path_kind::AT_REST;
                status.lerp.pos.began_at = now;
                status.lerp.pos.base_point = spot;
                status.lerp.pos.rate = [0.0; 3];
                status.lerp.pos.span_ms = 0;
            }
            // BO2MP_T6_NOTURNGOAL=1: the old carry-on, to compare (test aid).
            let use_goal = std::env::var_os("BO2MP_T6_NOTURNGOAL").is_none();
            if use_goal
                && let Some(goal) = yaw_goal
                && let Some(mover) = f.script_mover_mut_by_number(shown.num)
            {
                // From the facing it had last tick (where his client shows it
                // now): the turn trails the server by a tick, so it starts
                // smoothly instead of jumping a whole tick's turn (18 degrees).
                let from = [rot[0], prev_yaw, rot[2]];
                let left = game_math::degree_gap(goal, from[1]);
                let status = &mut mover.state;
                if left.abs() > 0.05 {
                    let rate = super::actors::TURN_DEG_PER_S;
                    status.lerp.apos.tr_type = entity_t6::flight::path_kind::STRAIGHT_THEN_STOP;
                    status.lerp.apos.began_at = now;
                    status.lerp.apos.base_point = from;
                    status.lerp.apos.rate = [0.0, rate.copysign(left), 0.0];
                    status.lerp.apos.span_ms = ((left.abs() / rate) * 1000.0).round().max(1.0) as i32;
                } else {
                    status.lerp.apos.tr_type = entity_t6::flight::path_kind::AT_REST;
                    status.lerp.apos.began_at = now;
                    status.lerp.apos.base_point = rot;
                    status.lerp.apos.rate = [0.0; 3];
                    status.lerp.apos.span_ms = 0;
                }
            }
            // bo2zm: a vehicle (Tranzit's bus) turns in steps at each path
            // node. Carried on at its last tick's speed and turn, it swung up
            // to 14 degrees past each turn and snapped back. It is drawn
            // easing from last tick's place to this tick's and stopping
            // there (a tick behind the server; his ride view uses the end).
            // What is linked to it (the driver, its doors and ladder, the
            // zombies on it) is drawn the same way: carried on a tick ahead
            // while the bus was a tick behind, the driver slid around in his
            // seat (a playtest, "the bus driver is sliding around").
            if moved
                && is_vehicle
                && let Some(mover) = f.script_mover_mut_by_number(shown.num)
            {
                let tick = crate::SIM_STEP_MILLIS as i32;
                let per_sec = 1000.0 / tick as f32;
                let status = &mut mover.state;
                status.lerp.pos.tr_type = entity_t6::flight::path_kind::STRAIGHT_THEN_STOP;
                status.lerp.pos.began_at = now;
                status.lerp.pos.span_ms = tick;
                status.lerp.pos.base_point = shown.spot;
                status.lerp.pos.rate = std::array::from_fn(|i| (spot[i] - shown.spot[i]) * per_sec);
                status.lerp.apos.tr_type = entity_t6::flight::path_kind::STRAIGHT_THEN_STOP;
                status.lerp.apos.began_at = now;
                status.lerp.apos.span_ms = tick;
                status.lerp.apos.base_point = shown.rot;
                status.lerp.apos.rate = std::array::from_fn(|i| {
                    game_math::degree_gap(rot[i], shown.rot[i]) * per_sec
                });
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                place_row(row, spot, rot);
            }
            shown.spot = spot;
            shown.rot = rot;
        }
        if shown.unseen != draw_hidden || shown.shown_to != shown_to {
            if let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                if draw_hidden {
                    mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
                } else {
                    mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
                }
                mover.nonsolid = true;
                mover.shown_to = shown_to;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                row.unseen = draw_hidden;
                row.solid = false;
            }
            // codmp: a car stands solid while it is drawn (its mdl's
            // collision-map brushes); hidden, players pass.
            let solid = !hidden_all
                && f.collision_owner_mut(shown.id)
                    .is_some_and(|row| !row.attached_solids.is_empty());
            if solid {
                if let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                    mover.nonsolid = false;
                }
                if let Some(row) = f.collision_owner_mut(shown.id) {
                    row.solid = true;
                }
            }
            // BO2MP_T6_SHOWNLOG=1: a line each time one is drawn for some
            // players only (a test aid for co-op).
            if (shown_to != 0 || shown.shown_to != 0)
                && std::env::var_os("BO2MP_T6_SHOWNLOG").is_some()
            {
                diag::info!(
                    Sim,
                    "bo2zm t6 shown ent{nn} {mdl} to {shown_to:#b} (was {:#b})",
                    shown.shown_to
                );
            }
            shown.unseen = draw_hidden;
            shown.shown_to = shown_to;
        }
        if shown.mesh != mdl || shown.attachments != attaches {
            let caps = f.model_capability(&mdl).flatten();
            // codmp: the new mdl's solid shape (a burnt-out car's husk).
            let shape = model_shape(&f, &mdl, def_model.as_deref(), spot, rot);
            let solid = !hidden_all && !shape.is_empty();
            if let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                mover.nonsolid = !solid;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                row.attached_solids = shape;
                row.solid = solid;
                match row.dobj.as_mut() {
                    Some(posed_obj) => {
                        if posed_obj.active_mesh != mdl {
                            posed_obj.replace_model(&mdl, caps);
                        }
                    }
                    None => {
                        row.dobj = Some(ServerModelState::at_pose(
                            &mdl, caps, spot, rot,
                        ))
                    }
                }
                if let Some(posed_obj) = row.dobj.as_mut() {
                    let a: Vec<(&str, &str, Option<xmodel_runtime::TagOffset>)> = attaches
                        .iter()
                        .map(|(m, t, at)| (m.as_str(), t.as_str(), *at))
                        .collect();
                    posed_obj.set_attachments_at(&a);
                }
            }
            shown.mesh = mdl;
            shown.attachments = attaches;
        }
        // bo2mp: a map destructible (a mannequin, a car) breaks as its
        // def entry says; the pieces it shows block bullets.
        let destructible = (nn & PIECE_BIT == 0)
            .then(|| {
                let zm = world.resource::<Zm>();
                let name = zm.ents.get(&nn)?.destructible.as_ref()?;
                zm.destructibles.get(name).cloned()
            })
            .flatten();
        if let Some(def) = &destructible {
            let mut f = frame(world);
            if let Some(row) = f.collision_owner_mut(shown.id)
                && let Some(posed_obj) = row.dobj.as_mut()
                && !crate::staged_props::installed(posed_obj, &def.name)
            {
                crate::staged_props::install_hosted(posed_obj, def.clone());
                diag::info!(
                    Sim,
                    "bo2mp destructible {} ent{nn} at {:?}",
                    def.name,
                    shown.spot
                );
            }
        }
        if let Some(hide) = &hide {
            set_hidden_parts(world, hide, shown.id);
        }
        // Actors: bullets hit the living (their posed hit boxes), and the
        // playing animation goes out as the mdl row's one leaf.
        let alive_actor = world
            .resource::<Zm>()
            .actors
            .get(&nn)
            .is_some_and(|a| a.living);
        let boxes = if alive_actor && !hidden_all {
            let f = frame(world);
            let placement = f
                .entity_hit_support()
                .iter()
                .find(|row| row.owner.scripted_model() == Some(shown.id))
                .and_then(|row| row.dobj.as_ref())
                .map(|d| d.placement_matrix);
            placement.and_then(|m| super::actors::hitboxes(world, nn, clock_millis, m))
        } else {
            None
        };
        // bo2mp: a scorestreak's plane, drone, vehicle or turret is hit by
        // bullets on its mdl's own hit boxes, as the engine traces any
        // entity's DObj: before, every shot went through the RC-XD.
        let streak = !alive_actor && nn & PIECE_BIT == 0 && super::killstreaks::damage_target(world, nn);
        {
            let mut f = frame(world);
            let packed = if boxes.is_some() { ZOMBIE_SOLID } else { 0 };
            if let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                if mover.state.solid != packed {
                    mover.state.solid = packed;
                }
                // The movers pass (`presence::follow_movers`, every tick)
                // copies this onto the hit row: left non-solid from the
                // first show, a living zombie lost its hit boxes after
                // the tick it was posed, and the Sliquifier bolt flew
                // through it (it only met zombies on the tick it was fired).
                mover.nonsolid = boxes.is_none();
            }
            let shape = f
                .collision_owner_mut(shown.id)
                .is_some_and(|row| !row.attached_solids.is_empty())
                && !hidden_all;
            if shape && let Some(mover) = f.script_mover_mut_by_number(shown.num) {
                mover.nonsolid = false;
            }
            if let Some(row) = f.collision_owner_mut(shown.id) {
                if destructible.is_some() && !alive_actor {
                    // Its bones as posed (bullet_collision bake_collision_shape).
                    row.solid = !hidden_all;
                } else if streak {
                    row.solid = !hidden_all;
                    // It may have become one after the branch below marked
                    // its (empty) collision as built: build it again.
                    if let Some(posed_obj) = row.dobj.as_mut()
                        && posed_obj.live_hitbox.is_none()
                    {
                        posed_obj.built_pose_stamp = None;
                    }
                } else {
                    row.solid = boxes.is_some() || shape;
                    if let Some(posed_obj) = row.dobj.as_mut() {
                        posed_obj.live_hitbox = boxes.map(|bones| {
                            crate::shot_trace::ServerModelCollision {
                                bones,
                                coll: None,
                                face_bits: 0,
                            }
                        });
                        posed_obj.built_model_stamp = Some(posed_obj.model_version_stamp);
                        posed_obj.built_pose_stamp = Some(posed_obj.pose_rev);
                    }
                }
            }
        }
        let anim = if nn & PIECE_BIT != 0 {
            let (en, i) = ((nn & !PIECE_BIT) >> 4, (nn & 0xf) as usize);
            world
                .resource::<Zm>()
                .ents
                .get(&en)
                .and_then(|en| en.zbarrier.as_ref())
                .and_then(|z| {
                    let boards = super::zbarrier::has_boards(z);
                    z.pieces.get(i).map(|pt| (pt, boards))
                })
                .and_then(|(pt, boards)| super::zbarrier::piece_pose(pt, clock_millis, boards).1)
                .map(|(c, t, r)| vec![(c, t, r, 1.0)])
                .unwrap_or_default()
        } else {
            match super::actors::present_anim(world, nn, clock_millis)
                .or_else(|| super::fxanims::present(world, nn, clock_millis))
                .or_else(|| script_anim(world, nn, clock_millis))
            {
                Some((c, t, r)) => vec![(c, t, r, 1.0)],
                None => super::mp_fxanims::present(world, nn, clock_millis),
            }
        };
        if !anim.is_empty() {
            let mut f = frame(world);
            if let Some(row) = f.collision_owner_mut(shown.id)
                && let Some(posed_obj) = row.dobj.as_mut()
            {
                use xmodel_runtime::{
                    GraphSlot, NodeClock, GraphNodeRecord, AnimGraphRole, GraphCapture,
                };
                let leaf = |parent, (clip_no, time, rate, share): (String, f32, f32, f32)| {
                    let status = NodeClock { time, prev_t: time, rate, weight: share, target_w: share, ..Default::default() };
                    GraphNodeRecord { parent, kind: AnimGraphRole::Leaf, clip: Some(clip_no), parts: None, state: status }
                };
                // Two animations while a map script blends one into the next
                // (`clearanim( old, 0.5 )` then `animscripted( new, 1.0, 0.5,
                // 1.0 )`: Studio's brontosaurus, mp_studio_fx.csc), under one
                // blend node.
                let graph_nodes = if anim.len() == 1 {
                    anim.into_iter().map(|a| leaf(None, a)).collect()
                } else {
                    let full = NodeClock { weight: 1.0, target_w: 1.0, ..Default::default() };
                    let root = GraphNodeRecord { parent: None, kind: AnimGraphRole::Blend, clip: None, parts: None, state: full };
                    std::iter::once(root).chain(anim.into_iter().map(|a| leaf(Some(GraphSlot(0)), a))).collect()
                };
                let rev = posed_obj.bone_role_state.pose_epoch.wrapping_add(1);
                posed_obj.bone_role_state.pose_epoch = rev;
                posed_obj.bone_role_state.tree = Some(GraphCapture { shape_epoch: 1, clock_epoch: rev, nodes: graph_nodes });
            }
        }
        // Gone limp (`startragdoll`): the clients drop the body.
        let ragdoll = world.resource::<Zm>().ents.get(&nn).and_then(|en| en.ragdoll);
        if let Some(row) = frame(world).collision_owner_mut(shown.id)
            && let Some(posed_obj) = row.dobj.as_mut()
            && posed_obj.bone_role_state.ragdoll != ragdoll
        {
            posed_obj.bone_role_state.ragdoll = ragdoll;
        }
        // bo2zm round 1: its eye brightness rides along for the eye shader.
        let eyes = world.resource::<Zm>().ents.get(&nn).and_then(|en| en.eye_luminance);
        if let Some(row) = frame(world).collision_owner_mut(shown.id)
            && let Some(posed_obj) = row.dobj.as_mut()
            && posed_obj.bone_role_state.eye_luminance != eyes
        {
            posed_obj.bone_role_state.eye_luminance = eyes;
        }
        world.resource_mut::<Zm>().presences.by_ent.insert(nn, shown);
    }
    // The collision models as turned boxes, for his movement (the server's
    // and his client's prediction: they go out in the snapshot).
    let mut f = frame(world);
    let blockers: Vec<crate::OrientedBlocker> = blocker_ents
        .iter()
        .flat_map(|(nn, spot, rot, mdl)| {
            let (fw, rt, up) = gsc_t6::math::facing_axes(*rot);
            let axis = [fw, [-rt[0], -rt[1], -rt[2]], up];
            // A collision mdl stops him the way BO2 traces it
            // (SV_TraceCapsuleToEntity): as its physics collmap's pieces,
            // and only when their contents meet his trace mask, so the
            // physics-only ones (collision_physics_*, item clip_no) stop thrown
            // things, not him. Before, each was a box of the mdl's bounds:
            // a 128-unit cube for collision_physics_32x32x128.
            let collmap = collision_only(mdl)
                .then(|| {
                    f.model_capability(mdl)
                        .flatten()
                        .and_then(|c| c.collmap.clone())
                })
                .flatten();
            if let Some(collmap) = collmap {
                if collmap.contents & PLAYER_BODY_MASK == 0 {
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
                            ent_no: *nn as u16,
                            origin: std::array::from_fn(|k| spot[k] + center[k]),
                            frame: b.axis.map(turn),
                            lo: b.half.map(|v| -v),
                            hi: b.half,
                            face_bits: 0,
                        }
                    })
                    .collect();
            }
            // Without a collmap: its pieces by its name, a wall's size or
            // the mdl's bounds.
            let boxes = named_player_boxes(&f, mdl).unwrap_or_else(|| match named_wall_size(mdl) {
                Some(half_ext) => vec![(half_ext.map(|v| -v), half_ext)],
                None => {
                    // The mdl's bounds come as middle and half_ext-size.
                    f.model_capability(mdl)
                        .flatten()
                        .and_then(|c| c.bounds)
                        .map(|(centre, half_ext)| {
                            (
                                std::array::from_fn(|i| centre[i] - half_ext[i]),
                                std::array::from_fn(|i| centre[i] + half_ext[i]),
                            )
                        })
                        .into_iter()
                        .collect()
                }
            });
            boxes
                .into_iter()
                .map(|(lo, hi)| crate::OrientedBlocker {
                    ent_no: *nn as u16,
                    origin: *spot,
                    frame: axis,
                    lo,
                    hi,
                    face_bits: 0,
                })
                .collect()
        })
        .collect();
    let mut blockers = blockers;
    for (_, spot, rot, mdl, number) in &slab_ents {
        let Some((centre, half_ext)) = f.model_capability(mdl).flatten().and_then(|c| c.bounds) else {
            continue;
        };
        let (fw, rt, up) = gsc_t6::math::facing_axes(*rot);
        for &(z0, z1) in super::highrise::platform_slabs(mdl) {
            blockers.push(crate::OrientedBlocker {
                ent_no: *number,
                origin: *spot,
                frame: [fw, [-rt[0], -rt[1], -rt[2]], up],
                lo: [centre[0] - half_ext[0], centre[1] - half_ext[1], z0],
                hi: [centre[0] + half_ext[0], centre[1] + half_ext[1], z1],
                face_bits: 0,
            });
        }
    }
    for (nn, spot, rot, boxes) in &platform_ents {
        let (fw, rt, up) = gsc_t6::math::facing_axes(*rot);
        blockers.extend(boxes.iter().map(|&((low, high), surface_flags)| crate::OrientedBlocker {
            ent_no: *nn as u16,
            origin: *spot,
            frame: [fw, [-rt[0], -rt[1], -rt[2]], up],
            lo: low,
            hi: high,
            face_bits: surface_flags,
        }));
    }
    // BO2MP_T6_BLOCKERLOG=1: each blocker when the list changes (test aid).
    if std::env::var_os("BO2MP_T6_BLOCKERLOG").is_some()
        && f.oriented_blockers() != blockers.as_slice()
    {
        for b in &blockers {
            diag::info!(
                Sim,
                "bo2zm t6 blocker ent{} at ({:.0} {:.0} {:.0}) fwd ({:.2} {:.2} {:.2}) up ({:.2} {:.2} {:.2}) bounds {:?}..{:?} flags {:#x}",
                b.ent_no,
                b.origin[0],
                b.origin[1],
                b.origin[2],
                b.frame[0][0],
                b.frame[0][1],
                b.frame[0][2],
                b.frame[2][0],
                b.frame[2][1],
                b.frame[2][2],
                b.lo,
                b.hi,
                b.face_bits
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
    /// A drawn entity with a model for the other side.
    struct Seen {
        ent: u32,
        id: ScriptModelId,
        mesh: String,
        team: i32,
        owner: Option<u32>,
        /// Who drives it, for a vehicle.
        driver: Option<u32>,
    }
    let mut found: Vec<Seen> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .where_value(|en| !en.enemy_model.is_empty())
            .filter_map(|(nn, en)| {
                let shown = zm.presences.by_ent.get(nn)?;
                let (team, owner) = match zm.actors.get(nn) {
                    Some(a) => (
                        super::goals::team_number(&a.team)
                            .map_or(entity_t6::team::FREE, super::goals::client_team),
                        a.owner,
                    ),
                    None => (en.faction_team, en.faction_owner),
                };
                Some(Seen { ent: *nn, id: shown.id, mesh: en.enemy_model.clone(), team, owner, driver: None })
            })
            .collect()
    };
    for s in &mut found {
        s.driver = super::killstreaks::driver_of(world, s.ent);
    }
    let mut f = frame(world);
    let rows: Vec<crate::EnemyModel> = found
        .into_iter()
        .map(|Seen { ent: nn, id, mesh: model, team, owner, driver }| {
            let team = driver
                .and_then(|d| f.client_meta(crate::ClientId(d)))
                .map_or(team, |m| m.seat_side);
            if !f.enemy_models().iter().any(|r| r.id == id && r.mesh == model) {
                let has = f.model_capability(&model).flatten().is_some();
                diag::info!(
                    Sim,
                    "bo2mp t6 enemy model ent{nn} {model} team {team} owner {owner:?} (in his assets: {has})"
                );
            }
            crate::EnemyModel { id, mesh: model, team, owner }
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
fn place_row(row_no: &mut EntityHitAbilities, spot: [f32; 3], rot: [f32; 3]) {
    if let Some(dobj) = row_no.dobj.as_mut() {
        dobj.set_world_pose(spot, rot);
    }
    for brush in &mut row_no.attached_solids {
        (brush.origin, brush.turn) = (spot, rot);
    }
    row_no.followed_pose = Some((spot, rot));
}

/// codmp: the brush model a Black Ops II model is solid with (a car's
/// collision maps), placed where the model stands; none for most models.
fn model_shape(
    f: &crate::step_view::StepWorldView<'_>,
    name: &str,
    def_model: Option<&str>,
    spot: [f32; 3],
    rot: [f32; 3],
) -> Vec<crate::shot_trace::AttachedSolidShape> {
    let cmodels = &f.collision_submodel_set().model_cmodels;
    cmodels
        .get(name)
        .or_else(|| def_model.and_then(|d| cmodels.get(d)))
        .map(|&collision_model_ref| crate::shot_trace::AttachedSolidShape {
            collision_model_ref,
            origin: spot,
            turn: rot,
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shot_trace::AttachedSolidShape;
    use crate::ent_slots::MoverSlot;

    fn brush_pose(row: &EntityHitAbilities) -> ([f32; 3], [f32; 3]) {
        (row.attached_solids[0].origin, row.attached_solids[0].turn)
    }

    #[test]
    fn a_moved_model_takes_its_collision_along() {
        let id = ScriptModelId::decode_net(T6_PRESENCE_BASE + 7);
        let start = ([10.0, 20.0, 0.0], [0.0, 0.0, 0.0]);
        let mut row = EntityHitAbilities::now_tick(
            crate::ServerModelSource::ScriptModel(id),
            None,
            vec![AttachedSolidShape {
                collision_model_ref: 3,
                origin: start.0,
                turn: start.1,
            }],
        );
        row.followed_pose = Some(start);
        // Studio's target slides 240 units and turns.
        let to = ([250.0, 20.0, 0.0], [0.0, 90.0, 0.0]);
        place_row(&mut row, to.0, to.1);
        assert_eq!(brush_pose(&row), to);
        assert_eq!(row.followed_pose, Some(to));
        // The snapshot follower, with the mover standing there, keeps it.
        let mut mover = MoverSlot {
            id,
            ..Default::default()
        };
        mover.state.lerp.pos.base_point = to.0;
        mover.state.lerp.apos.base_point = to.1;
        let mut rows = [row];
        crate::mover_clip::follow_movers(&mut rows, &[mover], 1000);
        assert_eq!(brush_pose(&rows[0]), to);
    }
}

/// A vehicle, or an entity linked (through its link chain) to one.
fn vehicle_borne(zm: &Zm, mut n: u32) -> bool {
    for _ in 0..8 {
        if zm.vehicles.speed.contains_key(&n) {
            return true;
        }
        match zm.ents.get(&n).and_then(|e| e.link) {
            Some((p, _, _)) => n = p,
            None => break,
        }
    }
    false
}

/// The tags its scripts hid (`hidepart`) as the model's hide bits. Props
/// that break in stages keep their own.
fn set_hidden_parts(game: &mut World, pieces: &[String], cid: ScriptModelId) {
    let mut f = frame(game);
    let Some(dobj) = f.collision_owner_mut(cid).and_then(|r| r.dobj.as_mut()) else {
        return;
    };
    let mut chunks = [0u32; xmodel_runtime::HiddenParts::BIT_CHUNK_COUNT];
    if let Some(cap) = &dobj.can_do {
        for (i, bone) in cap.pose.joint_labels.iter().enumerate().take(xmodel_runtime::HiddenParts::BIT_CHUNK_COUNT * 32) {
            if pieces.iter().any(|p| p.eq_ignore_ascii_case(bone)) {
                chunks[i / 32] |= 0x8000_0000 >> (i % 32);
            }
        }
    }
    let mask = xmodel_runtime::HiddenParts::from_bit_chunks(chunks);
    if mask != dobj.bone_role_state.hidden_mask {
        // New hidden-part mask: both copies take it and the pose is marked stale.
        let next_rev = dobj.pose_rev.wrapping_add(1);
        dobj.pose_query.hidden_mask = mask;
        dobj.bone_role_state.hidden_mask = mask;
        dobj.pose_rev = next_rev;
        dobj.bone_role_state.pose_epoch = next_rev;
        dobj.live_hitbox = None;
        dobj.built_pose_stamp = None;
    }
}

/// A script model's `setanim` clip: (clip, fraction, rate per second),
/// looping when the clip loops, else held on its last frame.
pub(super) fn script_anim(world: &World, n: u32, now: i64) -> Option<(String, f32, f32)> {
    let zm = world.resource::<Zm>();
    let (clip, start) = zm.ents.get(&n)?.anim.clone()?;
    let a = zm.anims.get(&clip)?;
    let len = ((f32::from(a.frame_total) / a.fps.max(1.0)) * 1000.0).max(1.0) as i64;
    let elapsed = (now - start).max(0);
    Some(if a.looping {
        (clip, (elapsed % len) as f32 / len as f32, 1000.0 / len as f32)
    } else if elapsed >= len {
        (clip, 1.0, 0.0)
    } else {
        (clip, elapsed as f32 / len as f32, 1000.0 / len as f32)
    })
}
