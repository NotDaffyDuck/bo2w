//! Putting a player on a spawn point: finding the floor under it, refusing spots that are taken, and the order points
//! are tried in.

use movement_t6::bridge::{CollisionBackend, GroundTraceInput};
use trace_t6::Trace;

use super::choose::spawn_candidate_indices_for;
use super::{AuthoredSpawnPoint, SpawnDecision};
use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
use crate::frame::FrameWorld;
use crate::identities::MatchRng;
use crate::SpawnPick;

/// `SpawnDecision::source_index` of a spot that was asked for by position, not drawn from the map's points.
pub const FORCED_SPAWN_SOURCE: usize = usize::MAX;

/// Why a spawn point was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnReject {
    NoAuthoredCandidates,
    NoGroundHit,
    StartSolid,
    Occupied,
    UnsupportedCoverage,
    AllRejected,
}

impl SpawnReject {
    /// The name the logs print.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoAuthoredCandidates => "no_authored_candidates",
            Self::NoGroundHit => "no_ground_hit",
            Self::StartSolid => "start_solid",
            Self::Occupied => "occupied",
            Self::UnsupportedCoverage => "unsupported_coverage",
            Self::AllRejected => "all_rejected",
        }
    }
}

/// What trying to place a player came to: how many spots were tried, which were refused and why, and the one taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnAttemptReport {
    pub tried: u32,
    pub rejected: Vec<(usize, SpawnReject)>,
    pub accepted: Option<SpawnDecision>,
}

/// How far above a point the search for the floor starts, and how far below it can reach.
const RISE: f32 = 128.0;
const REACH_DOWN: f32 = 262_144.0;

fn point_along(from: [f32; 3], to: [f32; 3], fraction: f32) -> [f32; 3] {
    core::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * fraction)
}

/// The world's clipping, as the movement code asks for it when it has to push a player out of solid.
struct WorldClip<'a>(&'a FrameWorld<'a>);

impl CollisionBackend for WorldClip<'_> {
    fn trace(&self, input: GroundTraceInput) -> Trace {
        self.0.trace_clip(input.start, input.end, input.mins, input.maxs, input.tracemask)
    }
}

/// Where a player standing at `feet` ends up on the floor: lifted as far as the ceiling lets him (up to 128), dropped
/// to the ground under that, and nudged out of any solid he lands in.
pub(crate) fn ground_spawn(world: &FrameWorld, feet: [f32; 3]) -> Result<[f32; 3], SpawnReject> {
    if !world.has_world_clip() {
        return Err(SpawnReject::UnsupportedCoverage);
    }
    let sweep = |from: [f32; 3], to: [f32; 3]| world.trace_clip(from, to, PLAYER_MINS, PLAYER_MAXS, MASK_PLAYER_SOLID);
    let lift_to = [feet[0], feet[1], feet[2] + RISE];
    let raised = point_along(feet, lift_to, sweep(feet, lift_to).fraction);
    let drop_to = [raised[0], raised[1], raised[2] - REACH_DOWN];
    let fall = sweep(raised, drop_to);
    if fall.fraction >= 1.0 {
        return Err(if fall.startsolid || fall.allsolid { SpawnReject::StartSolid } else { SpawnReject::NoGroundHit });
    }
    let landed = point_along(raised, drop_to, fall.fraction);
    let rest = sweep(landed, landed);
    if !(rest.startsolid || rest.allsolid) {
        return Ok(landed);
    }
    match WorldClip(world).correct_solid(landed, PLAYER_MINS, PLAYER_MAXS, MASK_PLAYER_SOLID) {
        Some(fixed) => Ok(fixed.origin),
        None => Err(SpawnReject::StartSolid),
    }
}

/// Whether a player's box at `origin` would overlap the box of any player at `avoid`.
pub(super) fn crowded(avoid: &[[f32; 3]], origin: [f32; 3]) -> bool {
    let size: [f32; 3] = core::array::from_fn(|axis| PLAYER_MAXS[axis] - PLAYER_MINS[axis]);
    avoid.iter().any(|other| (0..3).all(|axis| (other[axis] - origin[axis]).abs() < size[axis]))
}

/// Tries the points of `spawns` in `order` and takes the first that has a floor and is not crowded.
fn first_free(
    world: &FrameWorld,
    spawns: &[AuthoredSpawnPoint],
    order: &[usize],
    avoid: &[[f32; 3]],
) -> SpawnAttemptReport {
    let mut report = SpawnAttemptReport::default();
    for &source_index in order {
        report.tried += 1;
        let point = &spawns[source_index];
        match ground_spawn(world, point.origin) {
            Err(reason) => report.rejected.push((source_index, reason)),
            Ok(traced_origin) if crowded(avoid, traced_origin) => {
                report.rejected.push((source_index, SpawnReject::Occupied));
            }
            Ok(traced_origin) => {
                report.accepted = Some(SpawnDecision {
                    classname: point.classname.clone(),
                    source_index,
                    raw_origin: point.origin,
                    raw_angles: point.angles,
                    traced_origin,
                });
                return report;
            }
        }
    }
    if !report.rejected.is_empty() {
        report.rejected.push((usize::MAX, SpawnReject::AllRejected));
    }
    report
}

/// Places a player by the pick a script forces: at a given spot when it is free, else (or when only a seed was given) on
/// one of the team's points in an order shuffled from the seed.
pub(crate) fn decide_forced_spawn(
    world: &FrameWorld,
    pick: SpawnPick,
    avoid: &[[f32; 3]],
    client_state_team: i32,
) -> SpawnAttemptReport {
    let mut spot_refusal = None;
    let seed = match pick {
        SpawnPick::Seeded(seed) => seed,
        SpawnPick::At { origin, yaw } => {
            match ground_spawn(world, origin) {
                Ok(traced_origin) if !crowded(avoid, traced_origin) => {
                    return SpawnAttemptReport {
                        tried: 1,
                        rejected: Vec::new(),
                        accepted: Some(SpawnDecision {
                            classname: String::from("forced"),
                            source_index: FORCED_SPAWN_SOURCE,
                            raw_origin: origin,
                            raw_angles: [0.0, yaw, 0.0],
                            traced_origin,
                        }),
                    };
                }
                Ok(_) => spot_refusal = Some(SpawnReject::Occupied),
                Err(reason) => spot_refusal = Some(reason),
            }
            0
        }
    };
    let setup = world.bootstrap_ref();
    let spawns = setup.spawns.clone();
    let mut order = spawn_candidate_indices_for(&spawns, setup.kind, client_state_team, false);
    // A shuffle from the seed: the last slot swaps with a random earlier-or-same one, and so on down.
    let mut rng = MatchRng::new(seed);
    for last in (1..order.len()).rev() {
        let other = rng.next_index(last + 1);
        order.swap(last, other);
    }
    let mut report = if order.is_empty() {
        SpawnAttemptReport {
            tried: 0,
            rejected: vec![(0, SpawnReject::NoAuthoredCandidates)],
            accepted: None,
        }
    } else {
        first_free(world, &spawns, &order, avoid)
    };
    if let Some(reason) = spot_refusal {
        report.tried += 1;
        report.rejected.insert(0, (FORCED_SPAWN_SOURCE, reason));
    }
    report
}
