//! Black Ops II's brush-model entities (doors, debris piles: a
//! `script_brushmodel` whose model is a map submodel, `*49`): where their
//! brush stands in the world, what they block, and the AI paths they cut
//! while closed (`disconnectpaths` / `connectpaths`).

use super::pairs::KeepByValue;
use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Ent, Zm, entnum, frame};
use crate::ScriptModelId;
use crate::shot_trace::{EntityHitAbilities, AttachedSolidShape};

/// A brush entity's collision as last sent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BrushRow {
    pub spot: [f32; 3],
    pub rot: [f32; 3],
    pub solid: bool,
    /// A script hid it (`hide`): its mover goes out not drawn.
    pub unseen: bool,
    /// A bit per player it does not stop (hidden from him).
    pub passes_for: u64,
    /// The networked entity that carries it to the client's
    /// prediction (a brush mover naming the submodel, as MW2's are).
    pub number: Option<i32>,
    /// It moved last tick: its client carries it on until told it stopped.
    pub moving: bool,
}

/// Collision rows for brush entities: their own range, clear of the
/// script movers' (`presence`) and the loop sounds' owners.
const T6_BRUSH_BASE: u32 = 0x6800_0000;

fn owner(n: u32) -> ScriptModelId {
    ScriptModelId::decode_net(T6_BRUSH_BASE | (n & 0x07ff_ffff))
}

/// The brush entity a collision row stands for (what a bullet struck).
pub(super) fn ent_of(world: &World, id: ScriptModelId) -> Option<u32> {
    let w = id.encode_for_net();
    let n = w & 0x07ff_ffff;
    (w & !0x07ff_ffff == T6_BRUSH_BASE && world.resource::<Zm>().brush_rows.contains_key(&n))
        .then_some(n)
}

/// Every `script_brushmodel` blocks players, zombies and bullets as its
/// submodel's brushes, placed by the entity's origin and angles (a door
/// swings open and stands against the wall, solid again), while the
/// scripts keep it solid (`notsolid` while it moves; a bought debris pile
/// is deleted). A brush mover naming the submodel carries each to the
/// client, whose movement prediction clips against it and whose world
/// draws the submodel's own surfaces while it lives and is not hidden
/// (Nuketown's carpet patch; most doors have none: the visible door is a
/// script model).
pub(super) fn sync(world: &mut World, clock_millis: i64) {
    let wanted: BTreeMap<u32, (u32, [f32; 3], [f32; 3], bool, bool, u64)> = {
        let zm = world.resource::<Zm>();
        // A bit per player in the match (a player's entity number is his client).
        let everyone = bits(zm.players.keys());
        zm.ents
            .iter()
            .where_value(|en| en.class_tag == "script_brushmodel")
            .filter_map(|(nn, en)| {
                let m = super::triggers::brush_index(en)?;
                // A brsh hidden from a player (Mob's afterlife doors while he
                // is a ghost, `afterlife_doors_open`) does not stop him, as
                // BO2's query_should_pass_gentity; it stops the others.
                let hidden_from = bits(en.invisible_to.iter()) & everyone;
                let solid = en.solid && (hidden_from == 0 || hidden_from != everyone);
                let passes_for = if solid { hidden_from } else { 0 };
                Some((*nn, (m, en.origin, en.rot, solid, en.unseen, passes_for)))
            })
            .collect()
    };
    let log = std::env::var_os("BO2MP_T6_PATHLOG").is_some();
    let now = clock_millis as i32;
    let mut rows = std::mem::take(&mut world.resource_mut::<Zm>().brush_rows);
    {
        let mut f = frame(world);
        rows.retain(|nn, row_no| {
            if wanted.contains_key(nn) {
                return true;
            }
            f.remove_collision_owner(owner(*nn));
            if let Some(number) = row_no.number {
                f.remove_script_mover_by_number(number);
            }
            false
        });
        for (nn, &(collision_geometry, spot, rot, solid, hidden, passes_for)) in &wanted {
            let row_no = rows.entry(*nn).or_insert_with(|| {
                let number = f.create_scripted_mover(owner(*nn), spot, rot).ok();
                match number.and_then(|number| f.script_mover_mut_by_number(number)) {
                    Some(mover) => {
                        mover.state.index = collision_geometry as i16;
                        mover.state.solid = entity_t6::script_mover::SOLID_BMODEL;
                        if hidden {
                            mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
                        } else {
                            mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
                        }
                        mover.nonsolid = !solid;
                        mover.passes_for = passes_for;
                    }
                    None => diag::warn!(Sim, "bo2zm t6: no free entity for brush *{collision_geometry}"),
                }
                let mut cap = EntityHitAbilities::now_tick(
                    crate::ServerModelSource::ScriptModel(owner(*nn)),
                    None,
                    vec![AttachedSolidShape {
                        collision_model_ref: collision_geometry,
                        origin: spot,
                        turn: rot,
                    }],
                );
                cap.solid = solid;
                cap.passes_for = passes_for;
                cap.followed_pose = Some((spot, rot));
                f.insert_collision_owner(cap);
                BrushRow {
                    spot,
                    rot,
                    solid,
                    unseen: hidden,
                    passes_for,
                    number,
                    moving: false,
                }
            });
            let moved = row_no.spot != spot || row_no.rot != rot;
            // A move that ended goes out once more, standing still.
            if moved || row_no.moving {
                row_no.moving = moved;
                if let Some(number) = row_no.number {
                    f.set_script_mover_pose(number, now, spot, rot);
                }
                if let Some(cap) = f.collision_owner_mut(owner(*nn)) {
                    for brsh in &mut cap.attached_solids {
                        brsh.origin = spot;
                        brsh.turn = rot;
                    }
                    cap.followed_pose = Some((spot, rot));
                }
                row_no.spot = spot;
                row_no.rot = rot;
            }
            if row_no.unseen != hidden {
                row_no.unseen = hidden;
                if let Some(mover) = row_no
                    .number
                    .and_then(|number| f.script_mover_mut_by_number(number))
                {
                    if hidden {
                        mover.state.lerp.e_flags |= entity_t6::missile::EF_NODRAW;
                    } else {
                        mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
                    }
                }
            }
            if row_no.solid != solid || row_no.passes_for != passes_for {
                row_no.solid = solid;
                row_no.passes_for = passes_for;
                if let Some(mover) = row_no
                    .number
                    .and_then(|number| f.script_mover_mut_by_number(number))
                {
                    mover.nonsolid = !solid;
                    mover.passes_for = passes_for;
                }
                if let Some(cap) = f.collision_owner_mut(owner(*nn)) {
                    cap.solid = solid;
                    cap.passes_for = passes_for;
                }
                if log {
                    diag::info!(
                        Sim,
                        "bo2zm t6 brush ent{nn} *{collision_geometry}: solid {solid} passes {passes_for:#b}"
                    );
                }
            }
        }
    }
    world.resource_mut::<Zm>().brush_rows = rows;
}

/// A bit per client number (players are 0..64).
fn bits<'a>(clients: impl Iterator<Item = &'a u32>) -> u64 {
    clients.filter(|c| **c < 64).fold(0, |m, c| m | (1 << c))
}

/// The level restarts (`map_restart`): every brush row goes; the new
/// level's brush entities make their own.
pub(super) fn retire_all(world: &mut World) {
    let table_rows = std::mem::take(&mut world.resource_mut::<Zm>().brush_rows);
    let mut f = frame(world);
    for (nn, row_no) in table_rows {
        f.remove_collision_owner(owner(nn));
        if let Some(number) = row_no.number {
            f.remove_script_mover_by_number(number);
        }
    }
}

/// The entity's submodel bounds (its own space: a submodel's brushes are
/// authored about the entity's origin, measured: door `*49` spans
/// -1.8..23.8 x -0.6..59 x -52..52 at origin -503 361 -6), placed by its
/// origin and angles: the world box around them.
pub(super) fn world_box(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    let n = super::triggers::brush_index(e)?;
    let (lo, hi) = {
        let f = frame(world);
        let m = f.collision_submodel_set().models.get(n as usize)?;
        (m.lo, m.hi)
    };
    let (fw, rt, up) = gsc_t6::math::facing_axes(e.rot);
    // IW axes: forward, left (= -right), up.
    let left = [-rt[0], -rt[1], -rt[2]];
    let mut wlo = [f32::MAX; 3];
    let mut whi = [f32::MIN; 3];
    for corner in 0..8 {
        let p = [
            if corner & 1 == 0 { lo[0] } else { hi[0] },
            if corner & 2 == 0 { lo[1] } else { hi[1] },
            if corner & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for i in 0..3 {
            let w = e.origin[i] + fw[i] * p[0] + left[i] * p[1] + up[i] * p[2];
            wlo[i] = wlo[i].min(w);
            whi[i] = whi[i].max(w);
        }
    }
    Some((wlo, whi))
}

/// A model entity's world box: its model's bounds turned by its angles,
/// placed.
pub(super) fn model_box(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    let (mid, half) = frame(world).model_capability(&e.mesh).flatten()?.bounds?;
    let (fw, rt, up) = gsc_t6::math::facing_axes(e.rot);
    let left = [-rt[0], -rt[1], -rt[2]];
    let mut wlo = [f32::MAX; 3];
    let mut whi = [f32::MIN; 3];
    for corner in 0..8 {
        let pt: [f32; 3] = std::array::from_fn(|ix| {
            if corner & (1 << ix) == 0 {
                mid[ix] - half[ix]
            } else {
                mid[ix] + half[ix]
            }
        });
        for ix in 0..3 {
            let w = e.origin[ix] + fw[ix] * pt[0] + left[ix] * pt[1] + up[ix] * pt[2];
            wlo[ix] = wlo[ix].min(w);
            whi[ix] = whi[ix].max(w);
        }
    }
    Some((wlo, whi))
}

/// Once, at the start: each debris pile's pieces that the map flags as
/// cutting paths (spawnflags 1, BO2's dynamic path: the truck's chairs,
/// armoires and clip) cut the zombies' paths through them, as BO2's engine
/// does when they spawn; buying the debris joins them again (the scripts'
/// `connectpaths` on each piece). Before, zombies walked through the truck
/// while its debris still stood.
pub(crate) fn cut_debris_paths(world: &mut World) {
    world.resource_mut::<Zm>().debris_paths_cut = true;
    let pieces: Vec<(u32, Ent)> = super::with_vm(world, |vm, world| {
        let (tn, target, flags, model_f) = (
            vm.intern("targetname"),
            vm.intern("target"),
            vm.intern("spawnflags"),
            vm.intern("model"),
        );
        let zm = world.resource::<Zm>();
        let body_text = |vm: &Vm<World>, en: &Ent, f| {
            en.obj
                .map(|o| vm.to_text(&vm.raw_field(o, f)))
                .unwrap_or_default()
        };
        let targets: Vec<String> = zm
            .ents
            .values()
            .filter(|en| body_text(vm, en, tn) == "zombie_debris")
            .map(|en| body_text(vm, en, target))
            .filter(|t| !t.is_empty())
            .collect();
        zm.ents
            .iter()
            .where_value(|en| targets.contains(&body_text(vm, en, tn)))
            .where_value(|en| {
                en.obj
                    .and_then(|o| vm.raw_field(o, flags).as_int())
                    .is_some_and(|v| v & 1 != 0)
            })
            .map(|(n, en)| {
                // A clip brush's model may live only in its script fields.
                let mut en = en.clone();
                if en.mesh.is_empty()
                    && let Some(o) = en.obj
                {
                    en.mesh = vm.to_text(&vm.raw_field(o, model_f));
                }
                (*n, en)
            })
            .collect()
    })
    .unwrap_or_default();
    for (n, en) in pieces {
        let bounds = if en.mesh.starts_with('*') {
            world_box(world, &en)
        } else {
            model_box(world, &en)
        };
        if bounds.is_none() && std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 debris piece ent{n} {}: no bounds", en.mesh);
        }
        if let Some((lo, hi)) = bounds {
            let mut zm = world.resource_mut::<Zm>();
            zm.nav.disconnect(n, lo, hi);
            if std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
                let cut = zm.nav.cut_count();
                diag::info!(
                    Sim,
                    "bo2zm t6 debris piece ent{n} {} cuts paths: {cut} links cut in all",
                    en.mesh
                );
            }
        }
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // A closed door cuts the paths through it; an opened one joins them.
    vm.bind("disconnectpaths", true, |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(e) = world.resource::<Zm>().ents.get(&n).cloned() else {
            return Ok(ScriptValue::Undefined);
        };
        // A model entity cuts by its model's box (BO2's
        // Path_DisconnectPathsForEntity takes any entity's bounds): the hidden
        // riot shields Frost, Rush and Detour stand on a node keep bots off
        // unbroken glass and a bad link.
        if let Some((lo, hi)) = world_box(world, &e).or_else(|| model_box(world, &e)) {
            let mut zm = world.resource_mut::<Zm>();
            zm.nav.disconnect(n, lo, hi);
            if std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
                let cut = zm.nav.cut_count();
                diag::info!(
                    Sim,
                    "bo2zm t6 disconnectpaths ent{n} box ({:.0} {:.0} {:.0})..({:.0} {:.0} {:.0}): {cut} links cut in all",
                    lo[0],
                    lo[1],
                    lo[2],
                    hi[0],
                    hi[1],
                    hi[2]
                );
            }
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("connectpaths", true, |vm, world, s, _| {
        if let Some(n) = entnum(vm, s) {
            let mut zm = world.resource_mut::<Zm>();
            zm.nav.connect(n);
            if std::env::var_os("BO2MP_T6_PATHLOG").is_some() {
                diag::info!(
                    Sim,
                    "bo2zm t6 connectpaths ent{n}: {} links cut in all",
                    zm.nav.cut_count()
                );
            }
        }
        Ok(ScriptValue::Undefined)
    });
}
