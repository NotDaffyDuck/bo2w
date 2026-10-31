//! Telling the rest of the client when a player's life starts and ends.
//!
//! The snapshot says, per client, whether they are alive and which life they
//! are on. Comparing it with what the last snapshot said gives the starts and
//! ends; nothing else in the client has to diff snapshots to learn them.

use std::collections::BTreeMap;

use bevy::prelude::*;
use net::{ClientStep, HostViewClient, ShownWorldFrame};
use sim::PlayerSessionStage;
use frame::{SpawnLifeBegun, SpawnLifeOver, DeathCircumstance, SpawnCause, BeginKillcam, AbortKillcam, KillReplayDone, SpawnedPlayer, ShownStatePosted, LifeStatePosted};

/// How many of each script notification have gone by. For the probe and the
/// tests; nothing reads it to decide anything.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SpawnAlertTally {
    pub start_deathreplay: u32,
    pub cancel_deathreplay: u32,
    pub deathreplay_finished: u32,
    pub player_spawns: u32,
}

/// What the life front has announced, and what it knew about each client at
/// the last snapshot.
#[derive(Resource, Debug, Default, Clone)]
pub struct SpawnWaveTally {
    /// Lives started and ended since the match began.
    pub begun: u32,
    pub over: u32,
    /// The same, for the snapshot just looked at.
    pub tick_begun: u32,
    pub tick_over: u32,
    /// The life number the local client is on, once they have had one.
    pub self_sequence_number: Option<u32>,
    known: BTreeMap<u32, Remembered>,
}

#[derive(Clone, Copy, Debug)]
struct Remembered {
    number: u32,
    living: bool,
}

/// One client as a snapshot lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub client: u32,
    pub living: bool,
    pub number: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Started(SpawnLifeBegun),
    Ended(SpawnLifeOver),
}

impl SpawnWaveTally {
    fn start_tick(&mut self) {
        (self.tick_begun, self.tick_over) = (0, 0);
    }

    /// No snapshot to look at: forget everyone, so the next one reads as a
    /// fresh start.
    fn forget(&mut self) {
        self.known.clear();
    }

    /// Compares `clients` with what was known and returns the lives that
    /// started or ended, in the order `clients` lists them (clients that
    /// left come last).
    pub(crate) fn observe(&mut self, local: u32, clients: &[Seen]) -> Vec<Change> {
        let mut out = Vec::new();
        for now in clients {
            let remembered = Remembered {
                number: now.number,
                living: now.living,
            };
            let Some(prior) = self.known.insert(now.client, remembered) else {
                if now.living {
                    out.push(began(now.client, now.number));
                }
                continue;
            };
            if prior.living && !now.living {
                out.push(finished(now.client, prior.number, DeathCircumstance::LeftAlive));
            } else if prior.living && now.number != prior.number {
                out.push(finished(now.client, prior.number, DeathCircumstance::Replaced));
                out.push(began(now.client, now.number));
            } else if !prior.living && now.living {
                out.push(began(now.client, now.number));
            }
        }
        let departed: Vec<u32> = self
            .known
            .keys()
            .copied()
            .filter(|id| !clients.iter().any(|now| now.client == *id))
            .collect();
        for client in departed {
            let Some(last) = self.known.remove(&client) else {
                continue;
            };
            if last.living {
                out.push(finished(client, last.number, DeathCircumstance::Dropped));
            }
        }
        for change in &out {
            if let Change::Started(begun) = change {
                bump(&mut self.begun);
                bump(&mut self.tick_begun);
                if begun.client == local {
                    self.self_sequence_number = Some(begun.life);
                }
            } else {
                bump(&mut self.over);
                bump(&mut self.tick_over);
            }
        }
        out
    }
}

fn began(client: u32, life: u32) -> Change {
    let reason = SpawnCause::JustRespawned;
    Change::Started(SpawnLifeBegun {
        client,
        life,
        reason,
    })
}

fn finished(client: u32, life: u32, cause: DeathCircumstance) -> Change {
    Change::Ended(SpawnLifeOver {
        client,
        life,
        cause,
    })
}

fn announce_lives(
    presented: Res<ShownWorldFrame>,
    me: Res<HostViewClient>,
    mut front: ResMut<SpawnWaveTally>,
    mut starts: MessageWriter<SpawnLifeBegun>,
    mut ends: MessageWriter<SpawnLifeOver>,
) {
    front.start_tick();
    let Some(view) = presented.snapshot() else {
        front.forget();
        return;
    };
    let listed: Vec<Seen> = view
        .meta
        .clients
        .iter()
        .map(|(who, meta)| Seen {
            client: who.0,
            living: meta.lifecycle == PlayerSessionStage::Alive,
            number: meta.life_sequence.0,
        })
        .collect();
    for change in front.observe(me.0.0, &listed) {
        if let Change::Started(message) = change {
            starts.write(message);
        } else if let Change::Ended(message) = change {
            ends.write(message);
        }
    }
}

fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

fn count_begin_killcam(_: On<BeginKillcam>, mut tally: ResMut<SpawnAlertTally>) {
    bump(&mut tally.start_deathreplay);
}

fn count_abort_killcam(_: On<AbortKillcam>, mut tally: ResMut<SpawnAlertTally>) {
    bump(&mut tally.cancel_deathreplay);
}

fn count_killcam_ended(_: On<KillReplayDone>, mut tally: ResMut<SpawnAlertTally>) {
    bump(&mut tally.deathreplay_finished);
}

fn count_spawned_player(_: On<SpawnedPlayer>, mut tally: ResMut<SpawnAlertTally>) {
    bump(&mut tally.player_spawns);
}

pub(crate) fn install(host: &mut App) {
    const PRESENT: ClientStep = ClientStep::Present;
    host.init_resource::<SpawnAlertTally>();
    host.init_resource::<SpawnWaveTally>();
    host.add_message::<SpawnLifeBegun>();
    host.add_message::<SpawnLifeOver>();
    host.add_observer(count_begin_killcam);
    host.add_observer(count_abort_killcam);
    host.add_observer(count_killcam_ended);
    host.add_observer(count_spawned_player);
    host.add_systems(bevy::app::Update, announce_lives.in_set(ShownStatePosted));
    host.configure_sets(
        bevy::app::Update,
        LifeStatePosted
            .in_set(PRESENT)
            .after(announce_lives),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::{DeathCircumstance, BeginKillcam, AbortKillcam, KillReplayDone, SpawnedPlayer};

    fn row(client: u32, living: bool, number: u32) -> Seen {
        Seen {
            client,
            living,
            number,
        }
    }

    #[test]
    fn a_client_first_seen_alive_starts_a_life() {
        let mut tracker = SpawnWaveTally::default();
        assert_eq!(tracker.observe(9, &[row(1, true, 4)]), [began(1, 4)]);
        assert_eq!((tracker.begun, tracker.tick_begun), (1, 1));
    }

    #[test]
    fn a_client_first_seen_dead_or_connecting_starts_nothing() {
        let mut tracker = SpawnWaveTally::default();
        assert!(tracker.observe(9, &[row(1, false, 0)]).is_empty());
        assert_eq!(tracker.begun, 0);
    }

    #[test]
    fn the_same_snapshot_twice_says_nothing_the_second_time() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        assert!(tracker.observe(9, &[row(1, true, 4)]).is_empty());
    }

    #[test]
    fn dying_ends_the_life_that_was_alive() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        assert_eq!(
            tracker.observe(9, &[row(1, false, 4)]),
            [finished(1, 4, DeathCircumstance::LeftAlive)]
        );
        assert_eq!(tracker.over, 1);
    }

    #[test]
    fn coming_back_alive_starts_the_next_life() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        tracker.observe(9, &[row(1, false, 4)]);
        assert_eq!(tracker.observe(9, &[row(1, true, 5)]), [began(1, 5)]);
    }

    #[test]
    fn a_new_life_number_while_alive_replaces_the_old_life_in_one_look() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        tracker.start_tick();
        assert_eq!(
            tracker.observe(9, &[row(1, true, 5)]),
            [finished(1, 4, DeathCircumstance::Replaced), began(1, 5),]
        );
        assert_eq!((tracker.tick_begun, tracker.tick_over), (1, 1));
    }

    #[test]
    fn a_dead_client_changing_life_number_says_nothing() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, false, 4)]);
        assert!(tracker.observe(9, &[row(1, false, 5)]).is_empty());
    }

    #[test]
    fn a_client_leaving_while_alive_has_its_life_dropped() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4), row(2, true, 1)]);
        assert_eq!(
            tracker.observe(9, &[row(2, true, 1)]),
            [finished(1, 4, DeathCircumstance::Dropped)]
        );
    }

    #[test]
    fn a_client_leaving_while_dead_ends_nothing() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, false, 4)]);
        assert!(tracker.observe(9, &[]).is_empty());
        assert_eq!(tracker.over, 0);
    }

    #[test]
    fn only_the_local_clients_life_sets_local_seq() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        assert_eq!(tracker.self_sequence_number, None);
        tracker.observe(9, &[row(1, true, 4), row(9, true, 2)]);
        assert_eq!(tracker.self_sequence_number, Some(2));
    }

    #[test]
    fn the_per_tick_counts_start_over_and_the_totals_do_not() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        tracker.start_tick();
        tracker.observe(9, &[row(1, true, 4), row(2, true, 1)]);
        assert_eq!((tracker.begun, tracker.tick_begun), (2, 1));
    }

    #[test]
    fn forgetting_makes_the_next_snapshot_a_fresh_start() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(1, true, 4)]);
        tracker.forget();
        assert_eq!(tracker.observe(9, &[row(1, true, 4)]), [began(1, 4)]);
    }

    #[test]
    fn changes_come_in_snapshot_order_with_the_dropped_last() {
        let mut tracker = SpawnWaveTally::default();
        tracker.observe(9, &[row(7, true, 1), row(3, true, 1)]);
        let changes = tracker.observe(9, &[row(5, true, 1), row(3, false, 1)]);
        assert_eq!(
            changes,
            [
                began(5, 1),
                finished(3, 1, DeathCircumstance::LeftAlive),
                finished(7, 1, DeathCircumstance::Dropped),
            ]
        );
    }

    #[test]
    fn script_notifications_are_counted() {
        let mut app = App::default();
        install(&mut app);
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(BeginKillcam { entity });
        app.world_mut().trigger(BeginKillcam { entity });
        app.world_mut().trigger(AbortKillcam { entity });
        app.world_mut().trigger(KillReplayDone { entity });
        app.world_mut().trigger(SpawnedPlayer { entity });
        assert_eq!(
            *app.world().resource::<SpawnAlertTally>(),
            SpawnAlertTally {
                start_deathreplay: 2,
                cancel_deathreplay: 1,
                deathreplay_finished: 1,
                player_spawns: 1,
            }
        );
    }
}
