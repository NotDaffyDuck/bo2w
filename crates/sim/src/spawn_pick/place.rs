//! Putting a player on a spawn point: finding the floor under it, refusing spots that are taken, and the order points
//! are tried in.

use movement_t6::bridge::{SolidProbe, SweepAsk};
use trace_t6::Trace;

use super::choose::eligible_spawn_rows;
use super::{MapSpawnSpot, SpawnVerdict};
use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::step_view::StepWorldView;
use crate::match_ids::MatchRandom;
use crate::SpawnPick;

/// `SpawnVerdict::origin_row` of a spot that was asked for by position, not drawn from the map's points.
pub const FORCED_SPAWN_SOURCE: usize = usize::MAX;

/// Why a spawn point was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnRefusalCause {
    NoSpawnPoints,
    NoFloorBelow,
    StartSolid,
    Occupied,
    NoWorldClip,
    EveryPointRefused,
}

impl SpawnRefusalCause {
    /// The name the logs print.
    pub fn as_str(self) -> &'static str {
        // One label per variant, in declaration order.
        const LABELS: [&str; 6] = [
            "no_authored_candidates",
            "no_ground_hit",
            "start_solid",
            "occupied",
            "unsupported_coverage",
            "all_rejected",
        ];
        LABELS[self as usize]
    }
}

/// What trying to place a player came to: how many spots were tried, which were refused and why, and the one taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnTryOutcome {
    pub attempts: u32,
    pub refused: Vec<(usize, SpawnRefusalCause)>,
    pub accepted: Option<SpawnVerdict>,
}

/// How far above a point the search for the floor starts, and how far below it can reach.
const RISE: f32 = 128.0;
const REACH_DOWN: f32 = 262_144.0;

fn point_along(from: [f32; 3], till: [f32; 3], fraction: f32) -> [f32; 3] {
    core::array::from_fn(|axis| from[axis] + (till[axis] - from[axis]) * fraction)
}

/// The world's clipping, as the movement code asks for it when it has to push a player out of solid.
struct WorldClip<'a>(&'a StepWorldView<'a>);

impl SolidProbe for WorldClip<'_> {
    fn trace(&self, query: SweepAsk) -> Trace {
        self.0.trace_clip(query.start, query.end, query.lo_corner, query.hi_corner, query.collide_filter)
    }
}

/// Where a player standing at `feet` ends up on the floor: lifted as far as the ceiling lets him (up to 128), dropped
/// to the ground under that, and nudged out of any solid he lands in.
pub(crate) fn settle_on_floor(world: &StepWorldView, feet: [f32; 3]) -> Result<[f32; 3], SpawnRefusalCause> {
    world.has_static_collision().then_some(()).ok_or(SpawnRefusalCause::NoWorldClip)?;
    let sweep = |from: [f32; 3], till: [f32; 3]| world.trace_clip(from, till, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
    let lift_to = [feet[0], feet[1], feet[2] + RISE];
    let raised = point_along(feet, lift_to, sweep(feet, lift_to).fraction);
    let drop_to = [raised[0], raised[1], raised[2] - REACH_DOWN];
    let fall = sweep(raised, drop_to);
    if fall.fraction >= 1.0 {
        return Err(if fall.starts_inside || fall.fully_embedded { SpawnRefusalCause::StartSolid } else { SpawnRefusalCause::NoFloorBelow });
    }
    let landed = point_along(raised, drop_to, fall.fraction);
    let rest = sweep(landed, landed);
    if !(rest.starts_inside || rest.fully_embedded) {
        return Ok(landed);
    }
    match WorldClip(world).correct_solid(landed, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK) {
        Some(fixed) => Ok(fixed.origin),
        None => Err(SpawnRefusalCause::StartSolid),
    }
}

/// Whether a player's box at `origin` would overlap the box of any player at `avoid`.
pub(super) fn crowded(avoid: &[[f32; 3]], spot: [f32; 3]) -> bool {
    let size: [f32; 3] = core::array::from_fn(|axis| BODY_BOX_UPPER[axis] - BODY_BOX_LOWER[axis]);
    avoid.iter().any(|other| (0..3).all(|axis| (other[axis] - spot[axis]).abs() < size[axis]))
}

/// Tries the points of `candidates` in `order` and takes the first that has a floor and is not crowded.
fn first_free(
    world: &StepWorldView,
    candidates: &[MapSpawnSpot],
    order: &[usize],
    taken_spots: &[[f32; 3]],
) -> SpawnTryOutcome {
    let mut verdict = SpawnTryOutcome::default();
    for &slot in order {
        verdict.attempts += 1;
        let candidate = &candidates[slot];
        let landing = match settle_on_floor(world, candidate.origin) {
            Ok(landing) => landing,
            Err(why) => {
                verdict.refused.push((slot, why));
                continue;
            }
        };
        if crowded(taken_spots, landing) {
            verdict.refused.push((slot, SpawnRefusalCause::Occupied));
            continue;
        }
        verdict.accepted = Some(SpawnVerdict {
            kind_name: candidate.kind_name.clone(),
            origin_row: slot,
            authored_origin: candidate.origin,
            authored_angles: candidate.turn,
            dropped_origin: landing,
        });
        return verdict;
    }
    if !verdict.refused.is_empty() {
        verdict.refused.push((usize::MAX, SpawnRefusalCause::EveryPointRefused));
    }
    verdict
}

/// Places a player by the pick a script forces: at a given spot when it is free, else (or when only a seed was given) on
/// one of the team's points in an order shuffled from the seed.
pub(crate) fn decide_forced_spawn(
    world: &StepWorldView,
    pick: SpawnPick,
    taken_spots: &[[f32; 3]],
    team_number: i32,
) -> SpawnTryOutcome {
    let mut spot_refusal = None;
    let seed = match pick {
        SpawnPick::Seeded(seed) => seed,
        SpawnPick::At { origin, yaw } => {
            match settle_on_floor(world, origin) {
                Ok(landing) if !crowded(taken_spots, landing) => {
                    let forced = SpawnVerdict {
                        kind_name: String::from("forced"),
                        origin_row: FORCED_SPAWN_SOURCE,
                        authored_origin: origin,
                        authored_angles: [0.0, yaw, 0.0],
                        dropped_origin: landing,
                    };
                    return SpawnTryOutcome { attempts: 1, accepted: Some(forced), ..SpawnTryOutcome::default() };
                }
                Ok(_) => spot_refusal = Some(SpawnRefusalCause::Occupied),
                Err(reason) => spot_refusal = Some(reason),
            }
            0
        }
    };
    let setup = world.bootstrap_ref();
    let points = setup.spawns.clone();
    let mut order = eligible_spawn_rows(&points, setup.kind, team_number, false);
    // A shuffle from the seed: the last slot swaps with a random earlier-or-same one, and so on down.
    let mut rng = MatchRandom::new(seed);
    for last in (1..order.len()).rev() {
        let other = rng.below(last + 1);
        order.swap(last, other);
    }
    let mut outcome = if order.is_empty() {
        SpawnTryOutcome {
            refused: Vec::from([(0, SpawnRefusalCause::NoSpawnPoints)]),
            ..SpawnTryOutcome::default()
        }
    } else {
        first_free(world, &points, &order, taken_spots)
    };
    if let Some(reason) = spot_refusal {
        outcome.attempts += 1;
        outcome.refused.insert(0, (FORCED_SPAWN_SOURCE, reason));
    }
    outcome
}
