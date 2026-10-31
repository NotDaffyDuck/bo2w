//! Sampling one remote player's pose out of the buffer.

use playerstate_t6::game::{AvatarState, entity_flag_bits};
use sim::ClientId;

use super::blend::blend_avatar;
use super::types::{
    CONSTANT_LAG_POLICY_GENERATION, SmoothingSampleVerdict, SmoothingSampleOrigin,
    SmoothingInstant, SmootherReading, RemoteBufferShortfall,
};
use super::{OtherPlayerSmoother, lagged_clock, pose_of_peer};

impl OtherPlayerSmoother {
    /// `peer`'s pose at `draw_clock_millis`, with how it was arrived at.
    pub fn pose_for(&self, peer: ClientId, draw_clock_millis: i32) -> SmootherReading {
        let sample = self.raw_pose_for(peer, draw_clock_millis);
        self.hold_across_teleport(peer, sample)
    }

    /// The raw sample: exact, interpolated or starved.
    fn raw_pose_for(&self, peer: ClientId, draw_clock_millis: i32) -> SmootherReading {
        let resolved_clock = SmoothingInstant(lagged_clock(draw_clock_millis));
        let sample_origin_tag = |outcome| SmoothingSampleOrigin {
            policy_revision: CONSTANT_LAG_POLICY_GENERATION,
            resolved_clock,
            outcome,
        };
        let exact_pose = |state: &AvatarState, whole_frame| SmootherReading::Pose {
            ps: *state,
            sample_origin_tag: sample_origin_tag(SmoothingSampleVerdict::Exact { whole_frame }),
        };

        // What a starved sample falls back to: the oldest row when the time is before the buffer,
        // else the newest row there is.
        let before_buffer = self
            .history
            .front()
            .is_some_and(|oldest| resolved_clock.0 < oldest.clock_stamp_ms);
        let fallback = if before_buffer {
            self.history.front().and_then(|entry| {
                pose_of_peer(&entry.frame.players, peer).map(|state| (entry.frame.tick, *state))
            })
        } else {
            self.latest_reading(peer)
        };
        let starved = |reason| SmootherReading::RanDry {
            held: fallback.map(|(_, held_snap)| held_snap),
            sample_origin_tag: sample_origin_tag(SmoothingSampleVerdict::RanDry {
                reason,
                retained_world_state: fallback.map(|(tick, _)| tick),
            }),
        };

        let (Some(eldest), Some(newest)) = (self.history.front(), self.history.back()) else {
            return starved(RemoteBufferShortfall::NothingBuffered);
        };
        if resolved_clock.0 < eldest.clock_stamp_ms {
            return starved(RemoteBufferShortfall::PriorToEarliestFrame);
        }
        if resolved_clock.0 > newest.clock_stamp_ms {
            return starved(RemoteBufferShortfall::PastNewestFrame);
        }

        if let Some(on_time) = self
            .history
            .iter()
            .find(|entry| entry.clock_stamp_ms == resolved_clock.0)
        {
            return match pose_of_peer(&on_time.frame.players, peer) {
                Some(state) => exact_pose(state, on_time.frame.tick),
                None => starved(RemoteBufferShortfall::EntityEntryAbsent),
            };
        }

        let (Some(earlier), Some(later)) = self.bracket(resolved_clock.0) else {
            return starved(RemoteBufferShortfall::EntityEntryAbsent);
        };
        let (Some(from), Some(to)) = (
            pose_of_peer(&earlier.frame.players, peer),
            pose_of_peer(&later.frame.players, peer),
        ) else {
            return starved(RemoteBufferShortfall::EntityEntryAbsent);
        };
        // A respawn between the two is a cut: show the later pose.
        let respawned = earlier
            .frame
            .meta
            .for_client(peer)
            .zip(later.frame.meta.for_client(peer))
            .is_some_and(|(before, after)| before.life_sequence != after.life_sequence);
        if respawned {
            return exact_pose(to, later.frame.tick);
        }
        let span = (later.clock_stamp_ms - earlier.clock_stamp_ms) as f32;
        let alpha = (resolved_clock.0 - earlier.clock_stamp_ms) as f32 / span;
        SmootherReading::Pose {
            ps: blend_avatar(from, to, alpha),
            sample_origin_tag: sample_origin_tag(SmoothingSampleVerdict::SmoothedBetweenFrames {
                from_tick: earlier.frame.tick,
                to_tick: later.frame.tick,
                alpha,
            }),
        }
    }

    /// When the sampled pose and the newest pose of `peer` disagree on the teleport flag, show
    /// the first pose after the flag last flipped instead: the player has teleported since, and
    /// an older pose would show them where they no longer are.
    fn hold_across_teleport(&self, peer: ClientId, sample: SmootherReading) -> SmootherReading {
        let (sampled, sample_origin_tag) = match &sample {
            SmootherReading::Pose { ps, sample_origin_tag } => (Some(ps), *sample_origin_tag),
            SmootherReading::RanDry { held, sample_origin_tag } => (held.as_ref(), *sample_origin_tag),
        };
        let (Some(sampled), Some((_, newest))) = (sampled, self.latest_reading(peer)) else {
            return sample;
        };
        let teleport_bit = |state: &AvatarState| state.e_flags & entity_flag_bits::TELEPORT;
        if teleport_bit(sampled) == teleport_bit(&newest) {
            return sample;
        }
        let first_after_flip = self
            .history
            .iter()
            .rev()
            .filter_map(|entry| {
                pose_of_peer(&entry.frame.players, peer).map(|state| (entry.frame.tick, state))
            })
            .take_while(|(_, state)| teleport_bit(state) == teleport_bit(&newest))
            .last();
        let Some((tick, state)) = first_after_flip else {
            return sample;
        };
        SmootherReading::Pose {
            ps: *state,
            sample_origin_tag: SmoothingSampleOrigin {
                outcome: SmoothingSampleVerdict::Exact { whole_frame: tick },
                ..sample_origin_tag
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_tick::peer_mail::HOST_STEP_MILLIS;
    use crate::client::remote_buffer::STANDIN_LAG_MILLIS;

    const ME: ClientId = ClientId(1);

    fn at(x: f32, teleport: bool) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin[0] = x;
        if teleport {
            state.e_flags |= entity_flag_bits::TELEPORT;
        }
        state
    }

    fn snapshot(tick: u32, state: Option<AvatarState>) -> sim::Snapshot {
        let mut snapshot = sim::Snapshot::unpublished(sim::Tick(0));
        snapshot.tick = sim::Tick(tick);
        snapshot.players = state.map(|state| (ME, state)).into_iter().collect();
        snapshot
    }

    fn proxy_of(snapshots: Vec<sim::Snapshot>) -> OtherPlayerSmoother {
        let mut proxy = OtherPlayerSmoother::new();
        for snapshot in &snapshots {
            proxy.take_copy(snapshot);
        }
        proxy
    }

    fn render_time(tick: i32, extra_ms: i32) -> i32 {
        tick * HOST_STEP_MILLIS + extra_ms + STANDIN_LAG_MILLIS
    }

    fn outcome(sample: SmootherReading) -> SmoothingSampleVerdict {
        match sample {
            SmootherReading::Pose { sample_origin_tag, .. } | SmootherReading::RanDry { sample_origin_tag, .. } => {
                sample_origin_tag.outcome
            }
        }
    }

    #[test]
    fn an_empty_buffer_is_starved_with_nothing_held() {
        let sample = OtherPlayerSmoother::new().pose_for(ME, 1000);
        assert!(matches!(
            sample,
            SmootherReading::RanDry {
                held: None,
                ..
            }
        ));
        assert!(matches!(
            outcome(sample),
            SmoothingSampleVerdict::RanDry {
                reason: RemoteBufferShortfall::NothingBuffered,
                retained_world_state: None
            }
        ));
    }

    #[test]
    fn on_a_snapshot_the_pose_is_exact() {
        let proxy = proxy_of(vec![snapshot(10, Some(at(1.0, false))), snapshot(20, Some(at(3.0, false)))]);
        let sample = proxy.pose_for(ME, render_time(10, 0));
        let SmootherReading::Pose { ps, sample_origin_tag } = sample else {
            panic!("expected a pose, got {sample:?}");
        };
        assert_eq!(ps.origin[0], 1.0);
        assert_eq!(sample_origin_tag.policy_revision, CONSTANT_LAG_POLICY_GENERATION);
        assert_eq!(
            sample_origin_tag.outcome,
            SmoothingSampleVerdict::Exact { whole_frame: sim::Tick(10) }
        );
    }

    #[test]
    fn between_snapshots_the_pose_is_blended() {
        let proxy = proxy_of(vec![snapshot(10, Some(at(0.0, false))), snapshot(20, Some(at(10.0, false)))]);
        let SmootherReading::Pose { ps, sample_origin_tag } =
            proxy.pose_for(ME, render_time(15, 0))
        else {
            panic!("expected a pose");
        };
        assert!((ps.origin[0] - 5.0).abs() < 1e-4);
        assert!(matches!(
            sample_origin_tag.outcome,
            SmoothingSampleVerdict::SmoothedBetweenFrames { .. }
        ));
    }

    #[test]
    fn after_the_newest_the_last_pose_is_held_and_the_sample_starved() {
        let proxy = proxy_of(vec![snapshot(10, Some(at(1.0, false))), snapshot(20, Some(at(3.0, false)))]);
        let sample = proxy.pose_for(ME, render_time(30, 0));
        let SmootherReading::RanDry { held, .. } = sample else {
            panic!("expected starved, got {sample:?}");
        };
        assert_eq!(held.map(|state| state.origin[0]), Some(3.0));
        assert!(matches!(
            outcome(sample),
            SmoothingSampleVerdict::RanDry {
                reason: RemoteBufferShortfall::PastNewestFrame,
                retained_world_state: Some(_)
            }
        ));
    }

    #[test]
    fn a_player_missing_from_the_snapshot_is_starved_as_missing() {
        let proxy = proxy_of(vec![snapshot(10, None), snapshot(20, None)]);
        let sample = proxy.pose_for(ME, render_time(10, 0));
        assert!(matches!(
            outcome(sample),
            SmoothingSampleVerdict::RanDry {
                reason: RemoteBufferShortfall::EntityEntryAbsent,
                ..
            }
        ));
    }

    #[test]
    fn a_teleport_since_the_sampled_time_shows_the_first_pose_after_it() {
        // The flag flips between tick 20 and 30; sampling tick 20 shows tick 30's pose instead.
        let proxy = proxy_of(vec![
            snapshot(10, Some(at(1.0, false))),
            snapshot(20, Some(at(2.0, false))),
            snapshot(30, Some(at(900.0, true))),
            snapshot(40, Some(at(901.0, true))),
        ]);
        let SmootherReading::Pose { ps, sample_origin_tag } = proxy.pose_for(ME, render_time(20, 0))
        else {
            panic!("expected a pose");
        };
        assert_eq!(ps.origin[0], 900.0);
        assert_eq!(
            sample_origin_tag.outcome,
            SmoothingSampleVerdict::Exact { whole_frame: sim::Tick(30) }
        );
    }
}
