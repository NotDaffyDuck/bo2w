//! Blasts (a grenade, a rocket, a script's radius damage) breaking the props within reach.

use super::publish::{Cause, publish};
use super::{DamageKind, Notice, State, dobj_of, dobj_of_mut, hit_log, queue_notice};
use crate::frame::FrameWorld;
use crate::world::ClientId;
use crate::{AuthorityModelOwner, Tick};
use glam::{Mat4, Vec3};
use xmodel_runtime::RetainedModelCapability;

/// A plain T5 prop is looked at when its origin is within the blast's radius plus this much.
const PLAIN_PROP_MARGIN: f32 = 128.0;
/// A plain prop's piece is out of the blast's sight when a wall stops the line more than this far short of it.
const WALL_TOLERANCE: f32 = 12.0;
/// How far above the blast the sight line to a plain prop's piece starts.
const SIGHT_LIFT: f32 = 4.0;
/// A map prop's push direction is raised by this much when the entity itself hears the damage.
const ENTITY_PUSH_LIFT: f32 = 24.0;
/// `dflags` on the entity damage a blast gives a map prop: it was a blast's share, and it had a position.
const BLAST_ENTITY_FLAGS: i32 = 5;
/// The contents mask a map prop's `CanDamage` traces by.
const SIGHT_MASK: u32 = 0x0080_2013;

/// One blast: where, how far, how hard at each distance, and who is to blame.
pub(super) struct Blast<'a> {
    pub origin: Vec3,
    pub radius: f32,
    pub amount_at: &'a dyn Fn(f32) -> f32,
    pub means: u8,
    pub attacker: Option<ClientId>,
    pub weapon: u32,
}

impl Blast<'_> {
    /// The push a piece at distance `d` gets pointing along `along`: Black Ops II's push falls off in a line to nothing at
    /// the blast's edge, and the length of the direction carries that.
    fn push(&self, along: Vec3, d: f32) -> [f32; 3] {
        let falloff = (1.0 - d / self.radius.max(1.0)).clamp(0.0, 1.0);
        (along * falloff).to_array()
    }

    fn cause(&self, push: [f32; 3]) -> Cause {
        Cause {
            kind: DamageKind::Explosive,
            dir: Some(push),
            attacker: self.attacker,
            weapon: self.weapon,
        }
    }
}

/// A piece of a plain T5 prop that a blast reached.
struct PieceHit {
    owner: AuthorityModelOwner,
    piece: usize,
    amount: i32,
    push: [f32; 3],
}

/// A Black Ops II map prop in the world: its owner, its placement, and its bounds as (middle, half size).
struct MapProp {
    owner: AuthorityModelOwner,
    world_from_model: Mat4,
    bounds: Option<([f32; 3], [f32; 3])>,
}

/// A blast by `attacker`'s `weapon` damages each standing piece of every breakable prop in reach, by the distance from
/// the piece's bone, times the piece's explosive scale. Returns each prop it reached (the strings are always empty now:
/// a map prop's notifies are queued as [`Notice`]s instead).
///
/// A Black Ops II map prop takes it the way BO2's `G_RadiusDamage` gives it: it is reached when the blast is nearer
/// than `radius` to its bounds and can see it ([`can_damage`]; `skip` are the blast's own models, its inflictor's, which
/// it spares). Its pieces take their share by [`radius_pieces`], then the entity itself takes a [`Notice::Damage`] of
/// the amount at that distance. `means` is the blast's kind of death.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_blast_by(
    world: &mut FrameWorld,
    tick: Tick,
    origin: [f32; 3],
    radius: f32,
    amount_at: impl Fn(f32) -> f32,
    means: u8,
    skip: &[AuthorityModelOwner],
    attacker: Option<ClientId>,
    weapon: u32,
) -> Vec<(AuthorityModelOwner, Vec<String>)> {
    let blast = Blast {
        origin: Vec3::from_array(origin),
        radius,
        amount_at: &amount_at,
        means,
        attacker,
        weapon,
    };
    let mut reached = Vec::new();

    for hit in plain_piece_hits(world, &blast) {
        let Some(state) = dobj_of_mut(world, hit.owner).and_then(|dobj| dobj.t5_destructible.as_mut()) else {
            continue;
        };
        let out = state.hit_piece(hit.piece, hit.amount, means, true, None);
        if hit_log() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {} -{} -> health {:?} breaks {}",
                state.definition.name,
                hit.piece,
                hit.amount,
                state.health,
                out.breaks.len()
            );
        }
        publish(world, tick, hit.owner, out, blast.cause(hit.push));
        reached.push((hit.owner, Vec::new()));
    }

    // Which map props the blast sees is settled before any of them is hurt.
    let seen: Vec<(AuthorityModelOwner, f32, Vec3)> = map_props(world)
        .into_iter()
        .filter(|prop| !skip.contains(&prop.owner))
        .filter_map(|prop| {
            let at = prop.world_from_model.w_axis.truncate();
            let d = distance_to_bounds(&prop, blast.origin);
            if d >= radius {
                return None;
            }
            let mut ignore = vec![prop.owner];
            ignore.extend_from_slice(skip);
            if !can_damage(world, blast.origin, prop.world_from_model, prop.bounds, &ignore) {
                if hit_log() {
                    diag::info!(
                        Sim,
                        "destructible blast at {} hidden from {:?} ({d} away)",
                        blast.origin,
                        prop.owner
                    );
                }
                return None;
            }
            Some((prop.owner, d, at))
        })
        .collect();
    for (owner, d, at) in seen {
        radius_pieces(world, tick, owner, &blast);
        let lifted = at - blast.origin + Vec3::Z * ENTITY_PUSH_LIFT;
        queue_notice(
            world,
            owner,
            Notice::Damage {
                amount: (amount_at(d) as i32).max(1),
                attacker,
                means,
                weapon,
                point: origin,
                dir: lifted.try_normalize().map(|v| v.to_array()),
                flags: BLAST_ENTITY_FLAGS,
            },
        );
        reached.push((owner, Vec::new()));
    }
    reached
}

/// The pieces of the plain T5 props in reach that the blast can see, with what each takes.
fn plain_piece_hits(world: &FrameWorld, blast: &Blast) -> Vec<PieceHit> {
    let mut hits = Vec::new();
    for row in world.entity_collision_capabilities() {
        let Some(dobj) = row.dobj.as_ref() else {
            continue;
        };
        let (Some(state), Some(cap)) = (&dobj.t5_destructible, &dobj.capability) else {
            continue;
        };
        if state.hosted {
            continue;
        }
        let home = dobj.world_from_model.w_axis;
        if home.truncate().distance(blast.origin) > blast.radius + PLAIN_PROP_MARGIN {
            continue;
        }
        let Ok(pose) = cap.pose(&dobj.pose_request, dobj.world_from_model) else {
            continue;
        };
        for (i, piece) in state.definition.pieces.iter().enumerate() {
            let Some(stage) = piece.stage(i, state.health[i]) else {
                continue;
            };
            let at = piece.stages[stage]
                .show_bone
                .as_ref()
                .and_then(|name| cap.pose.bone_names.iter().position(|n| n == name))
                .and_then(|b| pose.get(b))
                .map_or(home, |m| m.w_axis)
                .truncate();
            let d = at.distance(blast.origin);
            if d > blast.radius {
                continue;
            }
            // Not through walls (a wall prop's own wall stops the line just short of it).
            let start = blast.origin + Vec3::Z * SIGHT_LIFT;
            let line = world.trace_static_world(
                start.to_array(),
                at.to_array(),
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            if (1.0 - line.fraction) * d > WALL_TOLERANCE {
                continue;
            }
            let amount = ((blast.amount_at)(d).round() * piece.explosive_damage_scale) as i32;
            if amount > 0 {
                let along = (at - blast.origin).normalize_or(Vec3::Z);
                hits.push(PieceHit {
                    owner: row.owner,
                    piece: i,
                    amount,
                    push: blast.push(along, d),
                });
            }
        }
    }
    hits
}

/// Every Black Ops II map prop that has a model, in the world's order.
fn map_props(world: &FrameWorld) -> Vec<MapProp> {
    world
        .entity_collision_capabilities()
        .iter()
        .filter_map(|row| {
            let dobj = row.dobj.as_ref()?;
            let (state, cap) = (dobj.t5_destructible.as_ref()?, dobj.capability.as_ref()?);
            state.hosted.then(|| MapProp {
                owner: row.owner,
                world_from_model: dobj.world_from_model,
                bounds: cap.bounds,
            })
        })
        .collect()
}

/// BO2 measures a blast's distance to the nearest point of the model's bounds (or to its origin without any).
fn distance_to_bounds(prop: &MapProp, from: Vec3) -> f32 {
    match prop.bounds {
        Some((mid, half)) => {
            let (mid, half) = (Vec3::from_array(mid), Vec3::from_array(half));
            let local = prop.world_from_model.inverse().transform_point3(from);
            local.distance(local.clamp(mid - half, mid + half))
        }
        None => prop.world_from_model.w_axis.truncate().distance(from),
    }
}

/// BO2's `CanDamage` on a prop: whether a blast at `origin` sees the middle of its bounds or one of four points
/// around it, square to the line from the blast (a trace that leaves out `skip`).
fn can_damage(
    world: &FrameWorld,
    origin: Vec3,
    world_from_model: Mat4,
    bounds: Option<([f32; 3], [f32; 3])>,
    skip: &[AuthorityModelOwner],
) -> bool {
    let (mid, half) = bounds.map_or((Vec3::ZERO, Vec3::ZERO), |(m, h)| (Vec3::from_array(m), Vec3::from_array(h)));
    let centre = world_from_model.w_axis.truncate() + mid;
    let to_blast = origin - centre;
    let len = to_blast.length();
    let mut dir = if len > 0.0 { to_blast / len } else { Vec3::ZERO };
    if dir.x * dir.x + dir.y * dir.y < 0.001 {
        dir = Vec3::X;
    }
    let side = Vec3::new(-dir.y, dir.x, 0.0).normalize_or_zero();
    let up = dir.cross(side);
    let across = side * ((half.x * side.x).abs() + (half.y * side.y).abs());
    let high = up * ((up.x * half.x).abs() + (up.y * half.y).abs() + (up.z * half.z).abs());
    [
        centre,
        centre + across + high,
        centre - across + high,
        centre + across - high,
        centre - across - high,
    ]
    .iter()
    .any(|p| {
        world
            .trace_world_skipping(origin.to_array(), p.to_array(), SIGHT_MASK, skip)
            .fraction
            >= 1.0
    })
}

/// BO2's `DestructibleRadiusDamage` on one prop: each piece in turn (a later piece sees what the earlier ones did to it)
/// takes the blast's amount at its distance from the blast times its explosive scale, measured from where
/// [`piece_point`] puts it. Whether the blast sees the prop at all is the caller's business.
pub(super) fn radius_pieces(world: &mut FrameWorld, tick: Tick, owner: AuthorityModelOwner, blast: &Blast) {
    let found = dobj_of(world, owner).and_then(|d| {
        let cap = d.capability.clone()?;
        let pose = cap.pose(&d.pose_request, d.world_from_model).ok()?.to_vec();
        let pieces = d.t5_destructible.as_ref()?.health.len();
        Some((cap, pose, d.world_from_model, pieces))
    });
    let Some((cap, pose, world_from_model, pieces)) = found else {
        return;
    };
    for index in 0..pieces {
        let Some((at, scale)) = dobj_of(world, owner)
            .and_then(|d| d.t5_destructible.as_ref())
            .and_then(|state| {
                let at = piece_point(state, &cap, &pose, world_from_model, index)?;
                Some((at, state.definition.pieces[index].explosive_damage_scale))
            })
        else {
            continue;
        };
        let d = at.distance(blast.origin);
        if d >= blast.radius {
            continue;
        }
        let amount = ((blast.amount_at)(d) * scale) as i32;
        if amount == 0 {
            continue;
        }
        let Some(state) = dobj_of_mut(world, owner).and_then(|dobj| dobj.t5_destructible.as_mut()) else {
            return;
        };
        let out = state.hit_piece(index, amount, blast.means, true, None);
        if hit_log() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {index} -{amount} (mod {}) -> health {:?} breaks {}",
                state.definition.name,
                blast.means,
                state.health,
                out.breaks.len()
            );
        }
        let along = if d > 0.001 { (at - blast.origin) / d } else { Vec3::Z };
        publish(world, tick, owner, out, blast.cause(blast.push(along, d)));
    }
}

/// Where BO2's `DestructibleRadiusDamage` measures a piece from: the middle of its shown bone's bounds. A destroyed
/// piece shows no bone on the server, so it counts from the prop's origin, as does a bone the model lacks. `None` skips
/// the piece: a child piece's front, right or left effect bone, or a bone with no place.
fn piece_point(
    state: &State,
    cap: &RetainedModelCapability,
    pose: &[Mat4],
    world_from_model: Mat4,
    index: usize,
) -> Option<Vec3> {
    let piece = &state.definition.pieces[index];
    let shown = piece
        .stage(index, state.health[index])
        .and_then(|s| piece.stages[s].show_bone.as_deref());
    let is_effect_bone = |b: &str| matches!(b, "tag_fx_front" | "tag_fx_right" | "tag_fx_left");
    if usize::from(piece.parent_piece) < state.health.len() && shown.is_some_and(is_effect_bone) {
        return None;
    }
    let Some(slot) = shown.and_then(|name| cap.pose.bone_names.iter().position(|n| n == name)) else {
        return Some(world_from_model.w_axis.truncate());
    };
    let middle = cap
        .bone_collision
        .get(slot)
        .and_then(|c| c.as_ref())
        .map_or([0.0; 3], |c| c.midpoint);
    Some(pose.get(slot)?.transform_point3(Vec3::from_array(middle)))
}
