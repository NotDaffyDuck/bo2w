//! Last-seen records of death, give and configuration-change events, kept for the console dump
//! commands. Each log folds a snapshot's event journal into plain rows.

use bevy::prelude::*;
use sim::{ClientId, ClientLifecycle, DamageSource, EventRecord, SimEvent};

/// Death rows, one per (tick, victim).
#[derive(Resource, Debug, Clone, Default)]
pub struct DumpDeathLog {
    pub rows: Vec<DumpDeathRow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DumpDeathRow {
    pub tick: u32,
    pub victim: u32,
    pub attacker: Option<u32>,
    pub weapon: u32,
    pub suicide: i32,
    pub source: &'static str,

    pub life_sequence: u32,

    pub attacker_life: Option<u32>,

    pub killcam_entity_start_time: i32,
}

fn damage_source_label(source: Option<DamageSource>) -> &'static str {
    match source {
        Some(DamageSource::Shot(_)) => "shot",
        Some(DamageSource::Projectile(_)) => "projectile",
        Some(DamageSource::Radius(_)) => "radius",
        Some(DamageSource::Melee) => "melee",
        None => "none",
    }
}

impl DumpDeathLog {
    pub(super) fn push_journal(&mut self, journal: &[EventRecord]) {
        for record in journal {
            let SimEvent::Died {
                victim,
                life_sequence,
                attacker,
                attacker_life,
                source,
                weapon,
                killcam_entity_start_time,
            } = &record.event
            else {
                continue;
            };
            let already_logged = self
                .rows
                .iter()
                .any(|row| row.tick == record.tick.0 && row.victim == victim.0);
            if already_logged {
                continue;
            }
            self.rows.push(DumpDeathRow {
                tick: record.tick.0,
                victim: victim.0,
                attacker: attacker.map(|a| a.0),
                weapon: *weapon,
                suicide: i32::from(attacker.is_none_or(|a| a == *victim)),
                source: damage_source_label(*source),
                life_sequence: life_sequence.0,
                attacker_life: attacker_life.map(|life| life.0),
                killcam_entity_start_time: *killcam_entity_start_time,
            });
        }
    }
}

/// Outcome of the most recent give-weapon request.
#[derive(Resource, Debug, Clone, Default)]
pub struct DumpGiveLog {
    pub request_id: Option<u32>,
    pub weapon: Option<u32>,

    pub accepted: Option<u8>,

    pub reject_reason: Option<&'static str>,
}

impl DumpGiveLog {
    pub(super) fn push_journal(&mut self, journal: &[EventRecord]) {
        for record in journal {
            let (request_id, weapon, verdict) = match record.event {
                SimEvent::GiveAccepted { request_id, weapon } => (request_id, weapon, None),
                SimEvent::GiveRejected {
                    request_id,
                    weapon,
                    reason,
                } => (request_id, weapon, Some(reason.as_str())),
                _ => continue,
            };
            self.request_id = Some(request_id);
            self.weapon = Some(weapon);
            self.accepted = Some(u8::from(verdict.is_none()));
            self.reject_reason = verdict;
        }
    }
}

/// Outcome of the most recent weapon-configuration change request.
#[derive(Resource, Debug, Clone, Default)]
pub struct DumpConfigurationChangeLog {
    pub request_id: Option<u32>,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub accepted: Option<bool>,
    pub reject_reason: Option<&'static str>,
}

impl DumpConfigurationChangeLog {
    pub(super) fn push_journal(&mut self, journal: &[EventRecord]) {
        for record in journal {
            let (request_id, from, to, verdict) = match record.event {
                SimEvent::ConfigurationChangeAccepted {
                    request_id,
                    from,
                    to,
                } => (request_id, from, to, None),
                SimEvent::ConfigurationChangeRejected {
                    request_id,
                    from,
                    to,
                    reason,
                } => (request_id, from, to, Some(reason.as_str())),
                _ => continue,
            };
            self.request_id = Some(request_id);
            self.from = Some(from);
            self.to = Some(to);
            self.accepted = Some(verdict.is_none());
            self.reject_reason = verdict;
        }
    }
}

/// A player's origin, entity flags, other flags and delta time from a snapshot's player list.
pub(super) fn player_origin_flags(
    players: &[(ClientId, playerstate_t6::game::PlayerState)],
    id: ClientId,
) -> Option<([f32; 3], i32, i32, i32)> {
    let (_, state) = players.iter().find(|(client, _)| *client == id)?;
    Some((
        state.origin,
        state.e_flags as i32,
        state.other_flags as i32,
        state.delta_time,
    ))
}

/// The lifecycle name the dump commands print.
pub(super) fn lifecycle_label(life: ClientLifecycle) -> &'static str {
    match life {
        ClientLifecycle::Connecting => "Connecting",
        ClientLifecycle::ChoosingClass => "ChoosingClass",
        ClientLifecycle::SpawnPending => "SpawnPending",
        ClientLifecycle::Alive => "Alive",
        ClientLifecycle::Dead => "Dead",
        ClientLifecycle::RespawnPending => "RespawnPending",
        ClientLifecycle::Spectating => "Spectating",
        ClientLifecycle::Intermission => "Intermission",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{
        ConfigurationChangeRejectReason, EventAudience, EventSequence, GiveRejectReason,
        LifeSequence, ShotId, Tick,
    };

    fn record(tick: u32, event: SimEvent) -> EventRecord {
        EventRecord {
            sequence: EventSequence(0),
            tick: Tick(tick),
            audience: EventAudience::All,
            event,
        }
    }

    fn death(
        tick: u32,
        victim: u32,
        attacker: Option<u32>,
        source: Option<DamageSource>,
    ) -> EventRecord {
        record(
            tick,
            SimEvent::Died {
                victim: ClientId(victim),
                life_sequence: LifeSequence(2),
                attacker: attacker.map(ClientId),
                attacker_life: attacker.map(|_| LifeSequence(5)),
                source,
                weapon: 17,
                killcam_entity_start_time: 900,
            },
        )
    }

    #[test]
    fn a_kill_by_another_player_is_not_a_suicide() {
        let mut log = DumpDeathLog::default();
        log.push_journal(&[death(10, 1, Some(2), Some(DamageSource::Shot(ShotId(4))))]);
        let row = log.rows[0];
        assert_eq!(row.suicide, 0);
        assert_eq!(row.source, "shot");
        assert_eq!(row.attacker, Some(2));
        assert_eq!(row.attacker_life, Some(5));
        assert_eq!((row.tick, row.victim, row.weapon), (10, 1, 17));
        assert_eq!(row.life_sequence, 2);
        assert_eq!(row.killcam_entity_start_time, 900);
    }

    #[test]
    fn world_deaths_and_self_kills_are_suicides() {
        let mut log = DumpDeathLog::default();
        log.push_journal(&[
            death(10, 1, None, None),
            death(11, 2, Some(2), Some(DamageSource::Melee)),
        ]);
        assert_eq!(log.rows[0].suicide, 1);
        assert_eq!(log.rows[0].source, "none");
        assert_eq!(log.rows[1].suicide, 1);
        assert_eq!(log.rows[1].source, "melee");
    }

    #[test]
    fn the_same_death_is_logged_once() {
        let mut log = DumpDeathLog::default();
        let journal = [death(10, 1, Some(2), None)];
        log.push_journal(&journal);
        log.push_journal(&journal);
        assert_eq!(log.rows.len(), 1);
        log.push_journal(&[death(11, 1, Some(2), None), death(10, 3, Some(2), None)]);
        assert_eq!(log.rows.len(), 3);
    }

    #[test]
    fn source_labels_cover_every_kind() {
        use sim::{ProjectileId, ScriptModelId};
        assert_eq!(
            damage_source_label(Some(DamageSource::Projectile(ProjectileId(1)))),
            "projectile"
        );
        assert_eq!(
            damage_source_label(Some(DamageSource::Radius(ScriptModelId::from_wire(1)))),
            "radius"
        );
    }

    #[test]
    fn non_death_events_leave_the_death_log_alone() {
        let mut log = DumpDeathLog::default();
        log.push_journal(&[record(1, SimEvent::AttackReleased)]);
        assert!(log.rows.is_empty());
    }

    #[test]
    fn give_log_follows_the_latest_verdict() {
        let mut log = DumpGiveLog::default();
        log.push_journal(&[record(
            1,
            SimEvent::GiveRejected {
                request_id: 7,
                weapon: 3,
                reason: GiveRejectReason::NotAlive,
            },
        )]);
        assert_eq!(
            (log.request_id, log.weapon, log.accepted, log.reject_reason),
            (Some(7), Some(3), Some(0), Some("not_alive"))
        );
        log.push_journal(&[record(
            2,
            SimEvent::GiveAccepted {
                request_id: 8,
                weapon: 4,
            },
        )]);
        assert_eq!(
            (log.request_id, log.weapon, log.accepted, log.reject_reason),
            (Some(8), Some(4), Some(1), None)
        );
    }

    #[test]
    fn give_log_ignores_unrelated_journals() {
        let mut log = DumpGiveLog::default();
        log.push_journal(&[record(1, SimEvent::AttackReleased)]);
        assert_eq!(log.request_id, None);
        assert_eq!(log.accepted, None);
    }

    #[test]
    fn configuration_log_records_both_outcomes() {
        let mut log = DumpConfigurationChangeLog::default();
        log.push_journal(&[record(
            1,
            SimEvent::ConfigurationChangeRejected {
                request_id: 2,
                from: 10,
                to: 11,
                reason: ConfigurationChangeRejectReason::Busy,
            },
        )]);
        assert_eq!(log.accepted, Some(false));
        assert_eq!(log.reject_reason, Some("busy"));
        assert_eq!((log.from, log.to), (Some(10), Some(11)));
        log.push_journal(&[record(
            2,
            SimEvent::ConfigurationChangeAccepted {
                request_id: 3,
                from: 11,
                to: 12,
            },
        )]);
        assert_eq!(log.accepted, Some(true));
        assert_eq!(log.reject_reason, None);
        assert_eq!(log.request_id, Some(3));
    }

    #[test]
    fn within_one_journal_the_last_give_event_wins() {
        let mut log = DumpGiveLog::default();
        log.push_journal(&[
            record(
                1,
                SimEvent::GiveAccepted {
                    request_id: 1,
                    weapon: 1,
                },
            ),
            record(
                1,
                SimEvent::GiveRejected {
                    request_id: 2,
                    weapon: 2,
                    reason: GiveRejectReason::InvalidWeapon,
                },
            ),
        ]);
        assert_eq!(log.request_id, Some(2));
        assert_eq!(log.reject_reason, Some("invalid_weapon"));
    }

    #[test]
    fn lifecycle_labels_are_the_variant_names() {
        assert_eq!(lifecycle_label(ClientLifecycle::Alive), "Alive");
        assert_eq!(lifecycle_label(ClientLifecycle::RespawnPending), "RespawnPending");
        assert_eq!(lifecycle_label(ClientLifecycle::Intermission), "Intermission");
    }

    #[test]
    fn player_lookup_misses_unknown_clients() {
        assert_eq!(player_origin_flags(&[], ClientId(1)), None);
    }
}
