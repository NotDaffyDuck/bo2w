//! Telling the rest of the client when a player's life starts and ends.
//!
//! The snapshot says, per client, whether they are alive and which life they
//! are on. Comparing it with what the last snapshot said gives the starts and
//! ends; nothing else in the client has to diff snapshots to learn them.

use std::collections::BTreeMap;

use bevy::prelude::*;
use frame::{
    AbortKillcam, BeginKillcam, KillcamEnded, LifeEndCause, LifeEnded, LifeFrontPublished,
    LifeStartReason, LifeStarted, PresentedPublished, SpawnedPlayer,
};
use net::{ClientSet, LocalPresentClient, PresentedSnapshot};
use sim::ClientLifecycle;

/// How many of each script notification have gone by. For the probe and the
/// tests; nothing reads it to decide anything.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct LifeNotifyCensus {
    pub begin_killcam: u32,
    pub abort_killcam: u32,
    pub killcam_ended: u32,
    pub spawned_player: u32,
}

/// What the life front has announced, and what it knew about each client at
/// the last snapshot.
#[derive(Resource, Debug, Default, Clone)]
pub struct LifeFrontCensus {
    /// Lives started and ended since the match began.
    pub started: u32,
    pub ended: u32,
    /// The same, for the snapshot just looked at.
    pub tick_started: u32,
    pub tick_ended: u32,
    /// The life number the local client is on, once they have had one.
    pub local_seq: Option<u32>,
    known: BTreeMap<u32, Known>,
}

#[derive(Clone, Copy, Debug)]
struct Known {
    life: u32,
    alive: bool,
}

/// One client as a snapshot lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub client: u32,
    pub alive: bool,
    pub life: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Started(LifeStarted),
    Ended(LifeEnded),
}

impl LifeFrontCensus {
    fn start_tick(&mut self) {
        self.tick_started = 0;
        self.tick_ended = 0;
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
        let mut changes = Vec::new();
        for seen in clients {
            let before = self.known.insert(
                seen.client,
                Known {
                    life: seen.life,
                    alive: seen.alive,
                },
            );
            match before {
                Some(before) if before.alive && !seen.alive => {
                    changes.push(ended(seen.client, before.life, LifeEndCause::LeftAlive));
                }
                Some(before) if before.alive && seen.life != before.life => {
                    changes.push(ended(seen.client, before.life, LifeEndCause::Replaced));
                    changes.push(started(seen.client, seen.life));
                }
                Some(before) if !before.alive && seen.alive => {
                    changes.push(started(seen.client, seen.life));
                }
                None if seen.alive => changes.push(started(seen.client, seen.life)),
                _ => {}
            }
        }
        let gone: Vec<u32> = self
            .known
            .keys()
            .copied()
            .filter(|known| clients.iter().all(|seen| seen.client != *known))
            .collect();
        for client in gone {
            if let Some(last) = self.known.remove(&client)
                && last.alive
            {
                changes.push(ended(client, last.life, LifeEndCause::Dropped));
            }
        }
        for change in &changes {
            match change {
                Change::Started(life) => {
                    self.started = self.started.saturating_add(1);
                    self.tick_started = self.tick_started.saturating_add(1);
                    if life.client == local {
                        self.local_seq = Some(life.life);
                    }
                }
                Change::Ended(_) => {
                    self.ended = self.ended.saturating_add(1);
                    self.tick_ended = self.tick_ended.saturating_add(1);
                }
            }
        }
        changes
    }
}

fn started(client: u32, life: u32) -> Change {
    Change::Started(LifeStarted {
        client,
        life,
        reason: LifeStartReason::BecameAlive,
    })
}

fn ended(client: u32, life: u32, cause: LifeEndCause) -> Change {
    Change::Ended(LifeEnded {
        client,
        life,
        cause,
    })
}

fn publish_life_front(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut census: ResMut<LifeFrontCensus>,
    mut starts: MessageWriter<LifeStarted>,
    mut ends: MessageWriter<LifeEnded>,
) {
    census.start_tick();
    let Some(snapshot) = presented.snapshot() else {
        census.forget();
        return;
    };
    let clients: Vec<Seen> = snapshot
        .meta
        .clients
        .iter()
        .map(|(id, meta)| Seen {
            client: id.0,
            alive: meta.lifecycle == ClientLifecycle::Alive,
            life: meta.life_sequence.0,
        })
        .collect();
    for change in census.observe(local.0.0, &clients) {
        match change {
            Change::Started(message) => {
                starts.write(message);
            }
            Change::Ended(message) => {
                ends.write(message);
            }
        }
    }
}

fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

fn count_begin_killcam(_: On<BeginKillcam>, mut census: ResMut<LifeNotifyCensus>) {
    bump(&mut census.begin_killcam);
}

fn count_abort_killcam(_: On<AbortKillcam>, mut census: ResMut<LifeNotifyCensus>) {
    bump(&mut census.abort_killcam);
}

fn count_killcam_ended(_: On<KillcamEnded>, mut census: ResMut<LifeNotifyCensus>) {
    bump(&mut census.killcam_ended);
}

fn count_spawned_player(_: On<SpawnedPlayer>, mut census: ResMut<LifeNotifyCensus>) {
    bump(&mut census.spawned_player);
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<LifeNotifyCensus>()
        .init_resource::<LifeFrontCensus>()
        .add_message::<LifeStarted>()
        .add_message::<LifeEnded>()
        .add_observer(count_begin_killcam)
        .add_observer(count_abort_killcam)
        .add_observer(count_killcam_ended)
        .add_observer(count_spawned_player)
        .add_systems(Update, publish_life_front.in_set(PresentedPublished))
        .configure_sets(
            Update,
            LifeFrontPublished
                .in_set(ClientSet::Present)
                .after(publish_life_front),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(client: u32, alive: bool, life: u32) -> Seen {
        Seen {
            client,
            alive,
            life,
        }
    }

    #[test]
    fn a_client_first_seen_alive_starts_a_life() {
        let mut census = LifeFrontCensus::default();
        assert_eq!(census.observe(9, &[seen(1, true, 4)]), [started(1, 4)]);
        assert_eq!((census.started, census.tick_started), (1, 1));
    }

    #[test]
    fn a_client_first_seen_dead_or_connecting_starts_nothing() {
        let mut census = LifeFrontCensus::default();
        assert!(census.observe(9, &[seen(1, false, 0)]).is_empty());
        assert_eq!(census.started, 0);
    }

    #[test]
    fn the_same_snapshot_twice_says_nothing_the_second_time() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        assert!(census.observe(9, &[seen(1, true, 4)]).is_empty());
    }

    #[test]
    fn dying_ends_the_life_that_was_alive() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        assert_eq!(
            census.observe(9, &[seen(1, false, 4)]),
            [ended(1, 4, LifeEndCause::LeftAlive)]
        );
        assert_eq!(census.ended, 1);
    }

    #[test]
    fn coming_back_alive_starts_the_next_life() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        census.observe(9, &[seen(1, false, 4)]);
        assert_eq!(census.observe(9, &[seen(1, true, 5)]), [started(1, 5)]);
    }

    #[test]
    fn a_new_life_number_while_alive_replaces_the_old_life_in_one_look() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        census.start_tick();
        assert_eq!(
            census.observe(9, &[seen(1, true, 5)]),
            [ended(1, 4, LifeEndCause::Replaced), started(1, 5),]
        );
        assert_eq!((census.tick_started, census.tick_ended), (1, 1));
    }

    #[test]
    fn a_dead_client_changing_life_number_says_nothing() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, false, 4)]);
        assert!(census.observe(9, &[seen(1, false, 5)]).is_empty());
    }

    #[test]
    fn a_client_leaving_while_alive_has_its_life_dropped() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4), seen(2, true, 1)]);
        assert_eq!(
            census.observe(9, &[seen(2, true, 1)]),
            [ended(1, 4, LifeEndCause::Dropped)]
        );
    }

    #[test]
    fn a_client_leaving_while_dead_ends_nothing() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, false, 4)]);
        assert!(census.observe(9, &[]).is_empty());
        assert_eq!(census.ended, 0);
    }

    #[test]
    fn only_the_local_clients_life_sets_local_seq() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        assert_eq!(census.local_seq, None);
        census.observe(9, &[seen(1, true, 4), seen(9, true, 2)]);
        assert_eq!(census.local_seq, Some(2));
    }

    #[test]
    fn the_per_tick_counts_start_over_and_the_totals_do_not() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        census.start_tick();
        census.observe(9, &[seen(1, true, 4), seen(2, true, 1)]);
        assert_eq!((census.started, census.tick_started), (2, 1));
    }

    #[test]
    fn forgetting_makes_the_next_snapshot_a_fresh_start() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(1, true, 4)]);
        census.forget();
        assert_eq!(census.observe(9, &[seen(1, true, 4)]), [started(1, 4)]);
    }

    #[test]
    fn changes_come_in_snapshot_order_with_the_dropped_last() {
        let mut census = LifeFrontCensus::default();
        census.observe(9, &[seen(7, true, 1), seen(3, true, 1)]);
        let changes = census.observe(9, &[seen(5, true, 1), seen(3, false, 1)]);
        assert_eq!(
            changes,
            [
                started(5, 1),
                ended(3, 1, LifeEndCause::LeftAlive),
                ended(7, 1, LifeEndCause::Dropped),
            ]
        );
    }

    #[test]
    fn script_notifications_are_counted() {
        let mut app = App::new();
        install(&mut app);
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(BeginKillcam { entity });
        app.world_mut().trigger(BeginKillcam { entity });
        app.world_mut().trigger(AbortKillcam { entity });
        app.world_mut().trigger(KillcamEnded { entity });
        app.world_mut().trigger(SpawnedPlayer { entity });
        assert_eq!(
            *app.world().resource::<LifeNotifyCensus>(),
            LifeNotifyCensus {
                begin_killcam: 2,
                abort_killcam: 1,
                killcam_ended: 1,
                spawned_player: 1,
            }
        );
    }
}
