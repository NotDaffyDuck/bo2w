//! Lag compensation: choosing which moment of the past a shot is judged against, and the players and entities as
//! they stood then.

use super::state::MatchSimulation;
use super::{SeatNo, StepNo};
use crate::shot_trace::{
    LiveStateCause, EntityHitAbilities, EntityCollisionMoment, EntityHitboxReading, EntityHitboxSnapshot, EntityRayShape, RewindLimitCause,
    RewindStage, RewindDenialCause, RewindSample, RewindSampleRuling, MAX_COMPENSATION_STEPS, RewindRequest, SHOT_STEP_MS, ANTILAG_SKIP_MS,
    PlayerHitboxPose, ShotRecordOrigin,
};
use playerstate_t6::game::AvatarState;

/// What a shot is judged against.
#[derive(Clone, Copy)]
enum Verdict {
    /// The present: there is no claim to rewind to, or it points at this very tick, or the past is gone. With the
    /// moment the shooter drew (in ms) when there is one: BO2 still puts its movers there.
    Now(LiveStateCause, Option<u32>),
    /// Nothing: the claim cannot be honoured.
    Refuse { requested: StepNo, reason: RewindDenialCause },
    /// A past moment: `progress` of the way from the kept frame `used` to the next one, `toward` (the same frame when
    /// the moment falls on it). `held_back` when the wanted moment was older than the window allows and this is the
    /// oldest allowed. `shot_ms` is the moment the shooter drew, before that limit: where the movers stand.
    Past { requested: StepNo, used: StepNo, toward: StepNo, progress: f32, held_back: bool, shot_ms: u32 },
}

/// The verdict a rewound sample carries, from the frame it was taken from.
fn past_verdict(requested: StepNo, used: StepNo, bounded: bool, frame: StepNo, phase: RewindStage) -> RewindSampleRuling {
    if bounded {
        RewindSampleRuling::Clamped { requested, used, frame, phase, reason: RewindLimitCause::BeyondRewindLimit }
    } else {
        RewindSampleRuling::Exact { requested, frame, phase }
    }
}

/// Each thing in the earlier frame blended toward the one with the same `key` in the later frame. One kept in only
/// one of the two frames stands as it stood there.
fn blend_frames<T: Clone, K: PartialEq>(earlier: Vec<T>, later: &[T], key: impl Fn(&T) -> K, blend: impl Fn(&T, &T) -> T) -> Vec<T> {
    let only_later: Vec<_> = later.iter().filter(|next| earlier.iter().all(|then| key(then) != key(next))).cloned().collect();
    earlier
        .into_iter()
        .map(|then| match later.iter().find(|next| key(next) == key(&then)) {
            Some(next) => blend(&then, next),
            None => then,
        })
        .chain(only_later)
        .collect()
}

fn missing_frame(spent: StepNo) -> RewindSampleRuling {
    RewindSampleRuling::Declined { requested: spent, reason: RewindDenialCause::NoFrameForTick }
}

impl MatchSimulation {
    #[must_use]
    pub fn collision_history(&self) -> &crate::shot_trace::HitboxTimeline {
        &self.replay.history
    }

    #[must_use]
    pub fn entity_collision_history(&self) -> &crate::shot_trace::EntityHitboxTimeline {
        &self.replay.entity_history
    }

    // ---- the claims each shot comes with ----------------------------------------------------------------------

    /// Replaces the table of claims by (client, command time), and forgets every claim picked so far.
    pub fn store_rewind_inputs(&mut self, rows: impl IntoIterator<Item = ((SeatNo, i32), ShotRecordOrigin)>) {
        self.replay.sample.clear();
        self.replay.commands.clear();
        self.replay.commands.extend(rows);
    }

    /// Picks the claim of the command a client's shot came from; no claim when the table has none.
    pub(crate) fn pick_rewind_order(&mut self, shooter: SeatNo, cmd_time: i32) {
        let claim = self.replay.commands.remove(&(shooter, cmd_time)).unwrap_or(ShotRecordOrigin::UNCLAIMED);
        self.set_lagcomp_sample(shooter, claim);
    }

    pub fn set_lagcomp_sample(&mut self, client: SeatNo, sample: ShotRecordOrigin) {
        self.replay.sample.insert(client, sample);
    }

    #[must_use]
    pub fn lagcomp_sample(&self, client: SeatNo) -> Option<ShotRecordOrigin> {
        self.replay.sample.get(&client).copied()
    }

    /// Client prediction only: where this client draws the living other players right now, as (id, origin, velocity).
    pub fn set_lunge_seen(&mut self, rows: impl IntoIterator<Item = (SeatNo, [f32; 3], [f32; 3])>) {
        self.replay.lunge_seen.clear();
        self.replay.lunge_seen.extend(rows.into_iter().map(|(id, origin, velocity)| (id, (origin, velocity))));
    }

    #[must_use]
    pub(crate) fn lunge_seen(&self, id: SeatNo) -> Option<([f32; 3], [f32; 3])> {
        self.replay.lunge_seen.get(&id).copied()
    }

    // ---- choosing the moment ----------------------------------------------------------------------------------

    fn pick_moment(&self, attacker: SeatNo, current: StepNo) -> Verdict {
        let no_claim = Verdict::Now(LiveStateCause::ShooterNamedNoTick, None);
        let Some(claim) = self.lagcomp_sample(attacker) else {
            return no_claim;
        };
        if !claim.quality.asserts_past_record() {
            return no_claim;
        }
        if claim.right.0 > current.0 {
            return Verdict::Refuse { requested: claim.right, reason: RewindDenialCause::TickAfterShot };
        }
        if !claim.still_fresh_at(current) {
            return Verdict::Refuse { requested: claim.asked_for_tick(), reason: RewindDenialCause::ClaimUnreadable };
        }
        let requested = claim.asked_for_tick();
        if current == requested {
            return Verdict::Now(LiveStateCause::NamedTickIsShotTick, None);
        }
        // BO2 `G_AntiLagRewindClientPos`: back to the moment the shooter saw, but no further back than 400 ms, and no
        // rewind at all for 50 ms or less.
        let now_ms = current.0 * SHOT_STEP_MS;
        let window_ms = MAX_COMPENSATION_STEPS * SHOT_STEP_MS;
        let wanted_ms = claim.drawn_at_ms(SHOT_STEP_MS).min(now_ms);
        let held_back = now_ms - wanted_ms > window_ms;
        let at_ms = if held_back { now_ms - window_ms } else { wanted_ms };
        if now_ms - at_ms <= ANTILAG_SKIP_MS {
            return Verdict::Now(LiveStateCause::TooRecentToRewind { requested }, Some(wanted_ms));
        }
        // BO2 `SV_GetClientPositionsAtTimeInternal`: the kept frames on either side of that moment, and how far
        // between them it lies.
        let used = StepNo(at_ms / SHOT_STEP_MS);
        let between_ms = at_ms % SHOT_STEP_MS;
        let toward = StepNo(used.0 + u32::from(between_ms > 0));
        let history = &self.replay.history;
        if history.sample_for(used).is_none() && history.sample_for(toward).is_none() {
            return Verdict::Now(LiveStateCause::NothingStored { requested: used }, Some(wanted_ms));
        }
        let progress = between_ms as f32 / SHOT_STEP_MS as f32;
        Verdict::Past { requested, used, toward, progress, held_back, shot_ms: wanted_ms }
    }

    /// The players and entities `attacker`'s shot at `current` is judged against. `player` gives a client's player
    /// state as it is now.
    pub fn rewind_request_for(
        &self,
        attacker: SeatNo,
        current: StepNo,
        live_state: impl Fn(SeatNo) -> Option<AvatarState>,
    ) -> RewindRequest {
        let moment = self.pick_moment(attacker, current);
        RewindRequest {
            players: self.players_at(current, moment, live_state),
            entities: self.entities_at(current, moment),
        }
    }

    fn players_at(&self, present: StepNo, moment: Verdict, live_state: impl Fn(SeatNo) -> Option<AvatarState>) -> RewindSample {
        match moment {
            Verdict::Now(why, _) => RewindSample {
                bodies_then: self.alive_collision_poses(live_state),
                verdict: RewindSampleRuling::LiveNow { tick: present, reason: why },
            },
            Verdict::Refuse { requested: asked, reason: why } => {
                RewindSample { bodies_then: Vec::new(), verdict: RewindSampleRuling::Declined { requested: asked, reason: why } }
            }
            Verdict::Past { requested: asked, used, toward, progress, held_back, .. } => {
                let history = &self.replay.history;
                let (earlier, later) = (history.sample_for(used), history.sample_for(toward));
                let Some(frame) = earlier.or(later) else {
                    return RewindSample { bodies_then: Vec::new(), verdict: missing_frame(used) };
                };
                let mut bodies = self.poses_of_current_life(&frame.poses);
                if let (Some(_), Some(later)) = (earlier, later) {
                    if later.tick != frame.tick {
                        let later = self.poses_of_current_life(&later.poses);
                        bodies = blend_frames(bodies, &later, |pose| pose.client, |then, next| then.blended_toward(next, progress));
                    }
                }
                RewindSample { bodies_then: bodies, verdict: past_verdict(asked, used, held_back, frame.tick, frame.phase) }
            }
        }
    }

    /// Poses from the past, minus the players who have died and come back since.
    fn poses_of_current_life(&self, poses: &[PlayerHitboxPose]) -> Vec<PlayerHitboxPose> {
        let same_life = |pose: &&PlayerHitboxPose| {
            self.client_meta(pose.client).is_none_or(|meta| meta.life_sequence == pose.life_sequence)
        };
        poses.iter().filter(same_life).cloned().collect()
    }

    fn entities_at(&self, present: StepNo, moment: Verdict) -> EntityHitboxReading {
        match moment {
            Verdict::Now(why, shot_ms) => EntityHitboxReading {
                rows: shot_ms
                    .and_then(|at_ms| self.movers_at(at_ms))
                    .unwrap_or_else(|| self.space.caps.iter().map(EntityHitAbilities::trace_geom).collect()),
                verdict: RewindSampleRuling::LiveNow { tick: present, reason: why },
            },
            Verdict::Refuse { requested: asked, reason: why } => {
                EntityHitboxReading { rows: Vec::new(), verdict: RewindSampleRuling::Declined { requested: asked, reason: why } }
            }
            Verdict::Past { requested: asked, used, toward, held_back, shot_ms, .. } => {
                let history = &self.replay.entity_history;
                let Some(frame) = history.sample_for(used).or_else(|| history.sample_for(toward)) else {
                    return EntityHitboxReading { rows: Vec::new(), verdict: missing_frame(used) };
                };
                let rows = self.movers_at(shot_ms).unwrap_or_default();
                EntityHitboxReading { rows, verdict: past_verdict(asked, used, held_back, frame.tick, frame.phase) }
            }
        }
    }

    /// The kept entity rows placed at `shot_ms`, as BO2's `AdjustMovers` places its movers for a shot
    /// (`mover_info_t::get_position`): blended between the kept frames on either side, with no 400 ms limit; before
    /// the oldest kept frame, that frame; from the newest on, the newest. `None` when no frame is kept.
    fn movers_at(&self, shot_ms: u32) -> Option<Vec<EntityRayShape>> {
        let history = &self.replay.entity_history;
        let at_ms = |frame: &EntityHitboxSnapshot| frame.tick.0 * SHOT_STEP_MS;
        let earlier = history.frames().filter(|frame| at_ms(frame) <= shot_ms).last();
        let later = history.frames().find(|frame| at_ms(frame) > shot_ms);
        let (tick, rows) = match (earlier, later) {
            (Some(then), Some(next)) => {
                let progress = (shot_ms - at_ms(then)) as f32 / (at_ms(next) - at_ms(then)) as f32;
                let blend = |row: &EntityRayShape, toward: &EntityRayShape| row.blended_toward(toward, progress);
                (then.tick, blend_frames(then.rows.clone(), &next.rows, |row| row.owner, blend))
            }
            (Some(only), None) | (None, Some(only)) => (only.tick, only.rows.clone()),
            (None, None) => return None,
        };
        // Script models are rewound as BO2 rewinds its movers; actors and vehicles have no rows here, as BO2 MP
        // leaves `g_actorAndVehicleAntilag` (singleplayer/co-op) off. A destructible keeps its parts as they are
        // now, so a part that broke off stops no bullet however far back the shot looks.
        let now_destructible = |owner| {
            self.space
                .caps
                .iter()
                .find(|caps| caps.owner == owner && caps.dobj.as_ref().is_some_and(|d| d.breakable.is_some()))
        };
        let stamp = EntityCollisionMoment::PastFrame { frame: tick };
        let placed = rows
            .into_iter()
            .map(|then| {
                let mut row = now_destructible(then.owner).map_or(then, EntityHitAbilities::trace_geom);
                row.epoch = stamp;
                row
            })
            .collect();
        Some(placed)
    }
}
