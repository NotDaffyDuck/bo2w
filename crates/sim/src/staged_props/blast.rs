//! Blasts (a grenade, a rocket, a script's radius damage) breaking the props within reach.

use super::publish::{Cause, publish};
use super::{DamageKind, Notice, BreakState, dobj_of, dobj_of_mut, hit_log, queue_notice};
use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use crate::{ServerModelSource, StepNo};
use glam::{Mat4, Vec3};
use xmodel_runtime::KeptModelAbility;

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
    pub reach: f32,
    pub amount_at: &'a dyn Fn(f32) -> f32,
    pub means: u8,
    pub attacker: Option<SeatNo>,
    pub gun: u32,
}

impl Blast<'_> {
    /// The push a piece at distance `d` gets pointing along `along`: Black Ops II's push falls off in a line to nothing at
    /// the blast's edge, and the length of the direction carries that.
    fn push(&self, along: Vec3, d: f32) -> [f32; 3] {
        let falloff = (1.0 - d / self.reach.max(1.0)).clamp(0.0, 1.0);
        (along * falloff).to_array()
    }

    fn cause(&self, push: [f32; 3]) -> Cause {
        Cause {
            kind: DamageKind::Explosive,
            dir: Some(push),
            attacker: self.attacker,
            gun: self.gun,
        }
    }
}

/// A piece of a plain T5 prop that a blast reached.
struct PieceHit {
    owner: ServerModelSource,
    piece_ix: usize,
    much: i32,
    push: [f32; 3],
}

/// A Black Ops II map prop in the world: its owner, its placement, and its bounds as (middle, half size).
struct MapProp {
    owner: ServerModelSource,
    placement_matrix: Mat4,
    extent: Option<([f32; 3], [f32; 3])>,
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
    level: &mut StepWorldView,
    tick: StepNo,
    spot: [f32; 3],
    blast_reach: f32,
    amount_at: impl Fn(f32) -> f32,
    means: u8,
    skip: &[ServerModelSource],
    culprit: Option<SeatNo>,
    gun_no: u32,
) -> Vec<(ServerModelSource, Vec<String>)> {
    let blast = Blast {
        origin: Vec3::from_array(spot),
        reach: blast_reach,
        amount_at: &amount_at,
        means,
        attacker: culprit,
        gun: gun_no,
    };
    let mut reached = Vec::new();

    for hit in plain_piece_hits(level, &blast) {
        let Some(state) = dobj_of_mut(level, hit.owner).and_then(|dobj| dobj.breakable.as_mut()) else {
            continue;
        };
        let out = state.hit_piece(hit.piece_ix, hit.much, means, true, None);
        if hit_log() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {} -{} -> health {:?} breaks {}",
                state.blueprint.name,
                hit.piece_ix,
                hit.much,
                state.piece_health,
                out.breaks.len()
            );
        }
        publish(level, tick, hit.owner, out, blast.cause(hit.push));
        reached.push((hit.owner, Vec::new()));
    }

    // Which map props the blast sees is settled before any of them is hurt.
    let seen: Vec<(ServerModelSource, f32, Vec3)> = map_props(level)
        .into_iter()
        .filter(|prop| !skip.contains(&prop.owner))
        .filter_map(|prop| {
            let at = prop.placement_matrix.w_axis.truncate();
            let d = distance_to_bounds(&prop, blast.origin);
            if blast_reach <= d {
                return None;
            }
            let mut ignore = vec![prop.owner];
            ignore.extend_from_slice(skip);
            if !can_damage(level, blast.origin, prop.placement_matrix, prop.extent, &ignore) {
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
        radius_pieces(level, tick, owner, &blast);
        let lifted = at - blast.origin + Vec3::Z * ENTITY_PUSH_LIFT;
        queue_notice(
            level,
            owner,
            Notice::Damage {
                amount: (amount_at(d) as i32).max(1),
                attacker: culprit,
                means,
                gun: gun_no,
                at: spot,
                way: lifted.try_normalize().map(|v| v.to_array()),
                flags: BLAST_ENTITY_FLAGS,
            },
        );
        reached.push((owner, Vec::new()));
    }
    reached
}

/// The pieces of the plain T5 props in reach that the blast can see, with what each takes.
fn plain_piece_hits(world: &StepWorldView, blast: &Blast) -> Vec<PieceHit> {
    let mut found = Vec::new();
    for row in world.entity_hit_support() {
        let Some(dobj) = row.dobj.as_ref() else {
            continue;
        };
        let (Some(state), Some(cap)) = (&dobj.breakable, &dobj.can_do) else {
            continue;
        };
        if state.hosted {
            continue;
        }
        let home = dobj.placement_matrix.w_axis;
        if home.truncate().distance(blast.origin) > blast.reach + PLAIN_PROP_MARGIN {
            continue;
        }
        let Ok(pose) = cap.pose(&dobj.pose_query, dobj.placement_matrix) else {
            continue;
        };
        for (i, piece) in state.blueprint.pieces.iter().enumerate() {
            let Some(stage) = piece.stage(i, state.piece_health[i]) else {
                continue;
            };
            let at = piece.stages[stage]
                .reveal_joint
                .as_ref()
                .and_then(|name| cap.pose.joint_labels.iter().position(|n| n == name))
                .and_then(|b| pose.get(b))
                .map_or(home, |m| m.w_axis)
                .truncate();
            let d = at.distance(blast.origin);
            if d > blast.reach {
                continue;
            }
            // Not through walls (a wall prop's own wall stops the line just short of it).
            let start = blast.origin + Vec3::Z * SIGHT_LIFT;
            let (from, to) = (start.to_array(), at.to_array());
            let line = world.trace_static_world(
                from,
                to,
                [0.0; 3],
                [0.0; 3],
                crate::shot_trace::MASK_SHOT,
            );
            if (1.0 - line.fraction) * d > WALL_TOLERANCE {
                continue;
            }
            let amount = ((blast.amount_at)(d).round() * piece.blast_damage_multiplier) as i32;
            if amount > 0 {
                let along = (at - blast.origin).normalize_or(Vec3::Z);
                found.push(PieceHit {
                    owner: row.owner,
                    piece_ix: i,
                    much: amount,
                    push: blast.push(along, d),
                });
            }
        }
    }
    found
}

/// Every Black Ops II map prop that has a model, in the world's order.
fn map_props(world: &StepWorldView) -> Vec<MapProp> {
    let entries = world.entity_hit_support();
    entries.iter()
        .filter_map(|entry| {
            let dobj = entry.dobj.as_ref()?;
            let (state, cap) = (dobj.breakable.as_ref()?, dobj.can_do.as_ref()?);
            state.hosted.then(|| MapProp {
                owner: entry.owner,
                placement_matrix: dobj.placement_matrix,
                extent: cap.bounds,
            })
        })
        .collect()
}

/// BO2 measures a blast's distance to the nearest point of the model's bounds (or to its origin without any).
fn distance_to_bounds(prop: &MapProp, from: Vec3) -> f32 {
    match prop.extent {
        Some((mid, half)) => {
            let (mid, half) = (Vec3::from_array(mid), Vec3::from_array(half));
            let local = prop.placement_matrix.inverse().transform_point3(from);
            local.distance(local.clamp(mid - half, mid + half))
        }
        None => prop.placement_matrix.w_axis.truncate().distance(from),
    }
}

/// BO2's `CanDamage` on a prop: whether a blast at `blast_at` sees the middle of its bounds or one of four points
/// around it, square to the line from the blast (a trace that leaves out `skip`).
fn can_damage(
    world: &StepWorldView,
    blast_at: Vec3,
    placement_matrix: Mat4,
    box_mid_half: Option<([f32; 3], [f32; 3])>,
    skip: &[ServerModelSource],
) -> bool {
    let (mid, half) = box_mid_half.map_or((Vec3::ZERO, Vec3::ZERO), |(m, h)| (Vec3::from_array(m), Vec3::from_array(h)));
    let centre = placement_matrix.w_axis.truncate() + mid;
    let to_blast = blast_at - centre;
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
            .trace_world_skipping(blast_at.to_array(), p.to_array(), SIGHT_MASK, skip)
            .fraction
            >= 1.0
    })
}

/// BO2's `DestructibleRadiusDamage` on one prop: each piece in turn (a later piece sees what the earlier ones did to it)
/// takes the blast's amount at its distance from the blast times its explosive scale, measured from where
/// [`piece_point`] puts it. Whether the blast sees the prop at all is the caller's business.
pub(super) fn radius_pieces(level: &mut StepWorldView, tick: StepNo, owner: ServerModelSource, blast: &Blast) {
    let found = dobj_of(level, owner).and_then(|d| {
        let cap = d.can_do.clone()?;
        let pose = cap.pose(&d.pose_query, d.placement_matrix).ok()?.to_vec();
        let pieces = d.breakable.as_ref()?.piece_health.len();
        Some((cap, pose, d.placement_matrix, pieces))
    });
    let Some((cap, pose, placement_matrix, pieces)) = found else {
        return;
    };
    for index in 0..pieces {
        let Some((at, scale)) = dobj_of(level, owner)
            .and_then(|d| d.breakable.as_ref())
            .and_then(|state| {
                let at = piece_point(state, &cap, &pose, placement_matrix, index)?;
                Some((at, state.blueprint.pieces[index].blast_damage_multiplier))
            })
        else {
            continue;
        };
        let d = at.distance(blast.origin);
        if d >= blast.reach {
            continue;
        }
        let amount = ((blast.amount_at)(d) * scale) as i32;
        if amount == 0 {
            continue;
        }
        let Some(state) = dobj_of_mut(level, owner).and_then(|dobj| dobj.breakable.as_mut()) else {
            return;
        };
        let out = state.hit_piece(index, amount, blast.means, true, None);
        if hit_log() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {index} -{amount} (mod {}) -> health {:?} breaks {}",
                state.blueprint.name,
                blast.means,
                state.piece_health,
                out.breaks.len()
            );
        }
        let along = if d > 0.001 { (at - blast.origin) / d } else { Vec3::Z };
        publish(level, tick, owner, out, blast.cause(blast.push(along, d)));
    }
}

/// Where BO2's `DestructibleRadiusDamage` measures a piece from: the middle of its shown bone's bounds. A destroyed
/// piece shows no bone on the server, so it counts from the prop's origin, as does a bone the model lacks. `None` skips
/// the piece: a child piece's front, right or left effect bone, or a bone with no place.
fn piece_point(state: &BreakState, cap: &KeptModelAbility, pose: &[Mat4], placed: Mat4, index: usize) -> Option<Vec3> {
    let piece = &state.blueprint.pieces[index];
    let shown = piece
        .stage(index, state.piece_health[index])
        .and_then(|s| piece.stages[s].reveal_joint.as_deref());
    let is_effect_bone = |b: &str| matches!(b, "tag_fx_front" | "tag_fx_right" | "tag_fx_left");
    if usize::from(piece.owner_fragment) < state.piece_health.len() && shown.is_some_and(is_effect_bone) {
        return None;
    }
    let Some(cell) = shown.and_then(|name| cap.pose.joint_labels.iter().position(|n| n == name)) else {
        return Some(placed.w_axis.truncate());
    };
    let collision_volumes = &cap.joint_hit_volume;
    let middle = collision_volumes
        .get(cell)
        .and_then(|c| c.as_ref())
        .map_or([0.0; 3], |c| c.midpoint);
    Some(pose.get(cell)?.transform_point3(Vec3::from_array(middle)))
}
