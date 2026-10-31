//! Effects a map script placed in the world (`createfx`-style loops and the
//! timed bursts a script starts), carried to the client in each snapshot's
//! objective block.
//!
//! Each listed effect is either held (a looping effect that runs from its
//! start time until the script restarts or removes it) or repeating (a burst
//! every `repeat_ms`). [`plan`] decides, from the effect's remembered state
//! and the server clock, what has to start this frame; [`sync_script_fx`]
//! carries those decisions out.

use std::collections::HashMap;

use bevy::prelude::*;
use fx::PlaybackResult;
use net::NewestTakenSnapshot;
use render_fx::EffectsEyePosition;
use sim::PlacedEffect;

use super::stage::{FxStage, Pose};

/// What the client remembers of one scripted effect between frames.
#[derive(Default)]
pub(super) struct ScriptFxRow {
    /// Which effect (an index into the snapshot's effect names) the row has.
    effect: u8,
    /// The start time last seen; a different one is a restart.
    start: Option<i32>,
    /// The running looping effect, for a held row.
    held: Option<u16>,
    /// Server time of the next burst, for a repeating row.
    next_ms: i32,
}

/// Whether a far-away effect is left out. A cull distance of zero or less
/// means never.
fn out_of_range(eye: Option<Vec3>, pos: [f32; 3], cull_distance: f32) -> bool {
    cull_distance > 0.0
        && eye.is_some_and(|eye| eye.distance(Vec3::from_array(pos)) > cull_distance)
}

#[derive(Debug, PartialEq)]
enum Plan {
    Nothing,
    /// Stop the held effect, if any, and spawn it again from this server time.
    Respawn { start_ms: i32 },
    /// Play a burst at each of these server times.
    Bursts(Vec<i32>),
}

fn plan(row: &mut ScriptFxRow, fx: &PlacedEffect, server_now: i32, eye: Option<Vec3>) -> Plan {
    let restarted = row.start != fx.start_ms;
    row.start = fx.start_ms;
    let Some(start) = fx.start_ms else {
        return Plan::Nothing;
    };
    if fx.repeat_ms <= 0 {
        return if restarted {
            Plan::Respawn { start_ms: start }
        } else {
            Plan::Nothing
        };
    }
    if restarted {
        row.next_ms = start;
    }
    // A client that fell far behind (a long load, a stall) skips the bursts it
    // missed instead of playing them all at once.
    let behind = server_now.saturating_sub(row.next_ms);
    if behind > fx.repeat_ms.saturating_mul(4) {
        row.next_ms = server_now - behind % fx.repeat_ms;
    }
    let silent = out_of_range(eye, fx.origin, fx.cull_distance);
    let mut due_at_ms = Vec::new();
    while row.next_ms <= server_now {
        if !silent {
            due_at_ms.push(row.next_ms);
        }
        row.next_ms = row.next_ms.saturating_add(fx.repeat_ms);
    }
    Plan::Bursts(due_at_ms)
}

fn held_handle(result: PlaybackResult) -> Option<u16> {
    match result {
        PlaybackResult::Kept { handle } => Some(handle),
        _ => None,
    }
}

pub(super) fn sync_script_fx(
    adopted: Option<Res<NewestTakenSnapshot>>,
    camera: Option<Res<EffectsEyePosition>>,
    mut stage: FxStage,
    mut rows: Local<HashMap<u32, ScriptFxRow>>,
) {
    let Some(adopted) = adopted else {
        return;
    };
    let Some(snap) = adopted.next() else {
        return;
    };
    let Some(mut play) = stage.playback() else {
        return;
    };
    let effects = &snap.meta.objectives.effects;
    // A row whose effect left the list, or was swapped for another, ends.
    rows.retain(|id, row| {
        let listed = effects
            .iter()
            .any(|fx| fx.id == *id && fx.effect == row.effect);
        if !listed && let Some(handle) = row.held {
            play.host().stop_owned(handle);
        }
        listed
    });
    if effects.is_empty() {
        return;
    }

    let server_now = sim::level_time_ms(snap.tick);
    // Bursts and spawns are dated on the server's clock; the host plays on its
    // own, a fixed offset away.
    let offset = play.now().wrapping_sub(server_now);
    let eye = camera.map(|camera| Vec3::from_array(camera.0));
    for fx in effects {
        let row = rows.entry(fx.id).or_insert_with(|| ScriptFxRow {
            effect: fx.effect,
            ..Default::default()
        });
        let Some(authored) = adopted.effect_label(fx.effect) else {
            continue;
        };
        let name = play.defs().map_fx_name(authored);
        let at = Pose {
            anchor: fx.origin,
            // A script gives an effect its forward and up.
            turn: Pose::axes_from(Vec3::from_array(fx.forward), Vec3::from_array(fx.up_dir)),
        };
        match plan(row, fx, server_now, eye) {
            Plan::Nothing => {}
            Plan::Respawn { start_ms } => {
                if let Some(handle) = row.held.take() {
                    play.host().stop_owned(handle);
                }
                row.held = play
                    .spawned(name, at, start_ms.wrapping_add(offset))
                    .and_then(held_handle);
            }
            Plan::Bursts(times) => {
                for when in times {
                    play.oriented_at(name, at, when.wrapping_add(offset));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeating(start: Option<i32>, repeat_ms: i32) -> PlacedEffect {
        PlacedEffect {
            id: 1,
            effect: 2,
            start_ms: start,
            repeat_ms,
            ..Default::default()
        }
    }

    #[test]
    fn forward_and_up_make_a_right_handed_frame() {
        assert_eq!(
            Pose::axes_from(Vec3::X, Vec3::Z),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        // Turned a quarter: forward along y puts left along -x.
        assert_eq!(
            Pose::axes_from(Vec3::Y, Vec3::Z),
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        );
    }

    #[test]
    fn a_zero_cull_distance_never_culls() {
        let far = Some(Vec3::new(1.0e6, 0.0, 0.0));
        assert!(!out_of_range(far, [0.0; 3], 0.0));
        assert!(!out_of_range(far, [0.0; 3], -5.0));
    }

    #[test]
    fn an_effect_is_culled_only_past_its_distance() {
        let eye = Some(Vec3::new(100.0, 0.0, 0.0));
        assert!(out_of_range(eye, [0.0; 3], 99.0));
        assert!(!out_of_range(eye, [0.0; 3], 101.0));
        // No camera yet: nothing is culled.
        assert!(!out_of_range(None, [0.0; 3], 1.0));
    }

    #[test]
    fn an_effect_without_a_start_does_nothing() {
        let mut row = ScriptFxRow::default();
        let fx = repeating(None, 500);
        assert_eq!(plan(&mut row, &fx, 9_000, None), Plan::Nothing);
    }

    #[test]
    fn a_held_effect_spawns_once_and_again_only_on_restart() {
        let mut row = ScriptFxRow::default();
        let started = repeating(Some(4_000), 0);
        assert_eq!(
            plan(&mut row, &started, 4_100, None),
            Plan::Respawn { start_ms: 4_000 }
        );
        assert_eq!(plan(&mut row, &started, 4_150, None), Plan::Nothing);
        let restarted = repeating(Some(8_000), 0);
        assert_eq!(
            plan(&mut row, &restarted, 8_050, None),
            Plan::Respawn { start_ms: 8_000 }
        );
    }

    #[test]
    fn a_repeating_effect_plays_every_burst_that_fell_due() {
        let mut row = ScriptFxRow::default();
        let fx = repeating(Some(1_000), 500);
        assert_eq!(
            plan(&mut row, &fx, 2_200, None),
            Plan::Bursts(vec![1_000, 1_500, 2_000])
        );
        assert_eq!(row.next_ms, 2_500);
        // The same frame again, or a little later, owes nothing new.
        assert_eq!(plan(&mut row, &fx, 2_300, None), Plan::Bursts(vec![]));
        assert_eq!(plan(&mut row, &fx, 2_500, None), Plan::Bursts(vec![2_500]));
    }

    #[test]
    fn a_restart_counts_the_bursts_again_from_the_new_start() {
        let mut row = ScriptFxRow::default();
        assert_eq!(
            plan(&mut row, &repeating(Some(0), 100), 150, None),
            Plan::Bursts(vec![0, 100])
        );
        assert_eq!(
            plan(&mut row, &repeating(Some(1_000), 100), 1_050, None),
            Plan::Bursts(vec![1_000])
        );
    }

    #[test]
    fn a_client_far_behind_skips_to_the_latest_burst() {
        let mut row = ScriptFxRow::default();
        let fx = repeating(Some(0), 100);
        assert_eq!(
            plan(&mut row, &fx, 10_050, None),
            Plan::Bursts(vec![10_000])
        );
        assert_eq!(row.next_ms, 10_100);
    }

    #[test]
    fn four_beats_behind_is_still_played_in_full() {
        let mut row = ScriptFxRow::default();
        let fx = repeating(Some(0), 100);
        assert_eq!(
            plan(&mut row, &fx, 400, None),
            Plan::Bursts(vec![0, 100, 200, 300, 400])
        );
    }

    #[test]
    fn a_culled_effect_keeps_its_schedule_but_plays_nothing() {
        let mut row = ScriptFxRow::default();
        let fx = PlacedEffect {
            cull_distance: 500.0,
            ..repeating(Some(0), 100)
        };
        let far = Some(Vec3::new(5_000.0, 0.0, 0.0));
        assert_eq!(plan(&mut row, &fx, 250, far), Plan::Bursts(vec![]));
        assert_eq!(row.next_ms, 300);
        // Back in range, it plays from the schedule, not from the backlog.
        let near = Some(Vec3::ZERO);
        assert_eq!(plan(&mut row, &fx, 320, near), Plan::Bursts(vec![300]));
    }

    #[test]
    fn only_a_held_result_gives_a_handle() {
        assert_eq!(held_handle(PlaybackResult::Kept { handle: 9 }), Some(9));
    }
}
