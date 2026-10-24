//! Lag compensation: choosing which moment of the past a shot is judged against, and the players and entities as
//! they stood then.

use super::state::SimState;
use super::{ClientId, Tick};
use crate::bullet_collision::{
    CollisionHistory, CurrentAuthorityReason, EntityCollisionCapabilities, EntityCollisionEpoch, EntityCollisionHistory,
    EntityCollisionSample, HistoryClampReason, HistoryPhase, HistoryRefusalReason, HistorySample, HistorySampleVerdict,
    LAGCOMP_MAX_REWIND_TICKS, LagcompQuery, PlayerCollisionPose, ShotSampleProvenance,
};
use playerstate_t6::game::PlayerState;

/// What a shot is judged against.
#[derive(Clone, Copy)]
enum Verdict {
    /// The present: there is no claim to rewind to, or it points at this very tick, or the past is gone.
    Now(CurrentAuthorityReason),
    /// Nothing: the claim cannot be honoured.
    Refuse { requested: Tick, reason: HistoryRefusalReason },
    /// A past tick. `clamped` when the wanted one was older than the window allows and `used` is the oldest allowed.
    Past { requested: Tick, used: Tick, clamped: bool },
}

/// The verdict a rewound sample carries, from the frame it was taken from.
fn past_verdict(requested: Tick, used: Tick, clamped: bool, frame: Tick, phase: HistoryPhase) -> HistorySampleVerdict {
    if clamped {
        HistorySampleVerdict::Clamped { requested, used, frame, phase, reason: HistoryClampReason::MaxRewindWindow }
    } else {
        HistorySampleVerdict::Exact { requested, frame, phase }
    }
}

fn missing_frame(used: Tick) -> HistorySampleVerdict {
    HistorySampleVerdict::Refused { requested: used, reason: HistoryRefusalReason::MissingFrame }
}

impl SimState {
    #[must_use]
    pub fn collision_history(&self) -> &CollisionHistory {
        &self.replay.history
    }

    #[must_use]
    pub fn entity_collision_history(&self) -> &EntityCollisionHistory {
        &self.replay.entity_history
    }

    // ---- the claims each shot comes with ----------------------------------------------------------------------

    /// Replaces the table of claims by (client, command time), and forgets every claim picked so far.
    pub fn set_lagcomp_commands(&mut self, rows: impl IntoIterator<Item = ((ClientId, i32), ShotSampleProvenance)>) {
        self.replay.sample.clear();
        self.replay.commands.clear();
        self.replay.commands.extend(rows);
    }

    /// Picks the claim of the command a client's shot came from; no claim when the table has none.
    pub(crate) fn select_lagcomp_command(&mut self, client: ClientId, command_time: i32) {
        let claim = self.replay.commands.remove(&(client, command_time)).unwrap_or(ShotSampleProvenance::NO_CLAIM);
        self.set_lagcomp_sample(client, claim);
    }

    pub fn set_lagcomp_sample(&mut self, client: ClientId, sample: ShotSampleProvenance) {
        self.replay.sample.insert(client, sample);
    }

    #[must_use]
    pub fn lagcomp_sample(&self, client: ClientId) -> Option<ShotSampleProvenance> {
        self.replay.sample.get(&client).copied()
    }

    /// Client prediction only: where this client draws the living other players right now, as (id, origin, velocity).
    pub fn set_lunge_seen(&mut self, rows: impl IntoIterator<Item = (ClientId, [f32; 3], [f32; 3])>) {
        self.replay.lunge_seen.clear();
        self.replay.lunge_seen.extend(rows.into_iter().map(|(id, origin, velocity)| (id, (origin, velocity))));
    }

    #[must_use]
    pub(crate) fn lunge_seen(&self, id: ClientId) -> Option<([f32; 3], [f32; 3])> {
        self.replay.lunge_seen.get(&id).copied()
    }

    // ---- choosing the moment ----------------------------------------------------------------------------------

    fn pick_moment(&self, attacker: ClientId, current: Tick) -> Verdict {
        let no_claim = Verdict::Now(CurrentAuthorityReason::NoSampleClaim);
        let Some(claim) = self.lagcomp_sample(attacker) else {
            return no_claim;
        };
        if !claim.quality.claims_history() {
            return no_claim;
        }
        if claim.right.0 > current.0 {
            return Verdict::Refuse { requested: claim.right, reason: HistoryRefusalReason::SampleAfterShot };
        }
        if !claim.is_valid_for(current) {
            return Verdict::Refuse { requested: claim.requested_tick(), reason: HistoryRefusalReason::SampleMalformed };
        }
        let requested = claim.requested_tick();
        if requested == current {
            return Verdict::Now(CurrentAuthorityReason::SampleAtShot);
        }
        let oldest = Tick(current.0.saturating_sub(LAGCOMP_MAX_REWIND_TICKS));
        let clamped = requested.0 < oldest.0;
        let used = if clamped { oldest } else { requested };
        if self.replay.history.frame_at(used).is_none() {
            return Verdict::Now(CurrentAuthorityReason::HistoryUnavailable { requested: used });
        }
        Verdict::Past { requested, used, clamped }
    }

    /// The players and entities `attacker`'s shot at `current` is judged against. `player` gives a client's player
    /// state as it is now.
    pub fn lagcomp_query_for(
        &self,
        attacker: ClientId,
        current: Tick,
        player: impl Fn(ClientId) -> Option<PlayerState>,
    ) -> LagcompQuery {
        let verdict = self.pick_moment(attacker, current);
        LagcompQuery {
            players: self.players_at(current, verdict, player),
            entities: self.entities_at(current, verdict),
        }
    }

    fn players_at(&self, current: Tick, verdict: Verdict, player: impl Fn(ClientId) -> Option<PlayerState>) -> HistorySample {
        match verdict {
            Verdict::Now(reason) => HistorySample {
                poses: self.alive_collision_poses(player),
                verdict: HistorySampleVerdict::CurrentAuthority { tick: current, reason },
            },
            Verdict::Refuse { requested, reason } => {
                HistorySample { poses: Vec::new(), verdict: HistorySampleVerdict::Refused { requested, reason } }
            }
            Verdict::Past { requested, used, clamped } => match self.replay.history.frame_at(used) {
                None => HistorySample { poses: Vec::new(), verdict: missing_frame(used) },
                Some(frame) => HistorySample {
                    poses: self.poses_of_current_life(&frame.poses),
                    verdict: past_verdict(requested, used, clamped, frame.tick, frame.phase),
                },
            },
        }
    }

    /// Poses from the past, minus the players who have died and come back since.
    fn poses_of_current_life(&self, poses: &[PlayerCollisionPose]) -> Vec<PlayerCollisionPose> {
        let same_life = |pose: &&PlayerCollisionPose| {
            self.client_meta(pose.client).is_none_or(|meta| meta.life_sequence == pose.life_sequence)
        };
        poses.iter().filter(same_life).cloned().collect()
    }

    fn entities_at(&self, current: Tick, verdict: Verdict) -> EntityCollisionSample {
        match verdict {
            Verdict::Now(reason) => EntityCollisionSample {
                rows: self.space.caps.iter().map(EntityCollisionCapabilities::trace_geom).collect(),
                verdict: HistorySampleVerdict::CurrentAuthority { tick: current, reason },
            },
            Verdict::Refuse { requested, reason } => {
                EntityCollisionSample { rows: Vec::new(), verdict: HistorySampleVerdict::Refused { requested, reason } }
            }
            Verdict::Past { requested, used, clamped } => {
                let Some(frame) = self.replay.entity_history.frame_at(used) else {
                    return EntityCollisionSample { rows: Vec::new(), verdict: missing_frame(used) };
                };
                // Only players are rewound; any other entity is taken as it stands now. A destructible keeps its
                // parts as they are now too, so a part that broke off stops no bullet however far back the shot
                // looks.
                let now_destructible = |owner| {
                    self.space
                        .caps
                        .iter()
                        .find(|caps| caps.owner == owner && caps.dobj.as_ref().is_some_and(|d| d.t5_destructible.is_some()))
                };
                let rows = frame
                    .rows
                    .iter()
                    .map(|then| {
                        let mut row = now_destructible(then.owner).map_or_else(|| then.clone(), |caps| caps.trace_geom());
                        row.epoch = EntityCollisionEpoch::Historical { frame: frame.tick };
                        row
                    })
                    .collect();
                EntityCollisionSample { rows, verdict: past_verdict(requested, used, clamped, frame.tick, frame.phase) }
            }
        }
    }
}
