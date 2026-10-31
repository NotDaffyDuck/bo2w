//! What the console dump commands print about recent deaths, weapon gives and configuration
//! changes. Every snapshot's events is folded in here; the logs only ever remember plain values.

use bevy::prelude::*;
use sim::{ClientId, PlayerSessionStage, HarmOrigin, EventRecord, MatchOccurrence};

/// Death rows, one per (tick, victim).
#[derive(Resource, Debug, Clone, Default)]
pub struct DeathLogDumper {
    pub rows: Vec<DeathDumpLine>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathDumpLine {
    pub at_tick: u32,
    pub dead_player: u32,
    pub killer: Option<u32>,
    pub gun: u32,
    pub self_kill: i32,
    pub source: &'static str,
    pub life_no: u32,
    pub killer_life: Option<u32>,
    pub replay_start_time: i32,
}

/// Short word for what dealt the killing blow ("none" when the world did).
fn damage_source_label(source: Option<HarmOrigin>) -> &'static str {
    source.map_or("none", |kind| match kind {
        HarmOrigin::Melee => "melee",
        HarmOrigin::Radius(..) => "radius",
        HarmOrigin::Projectile(..) => "projectile",
        HarmOrigin::Shot(..) => "shot",
    })
}

/// Turns one events entry into a death row, or `None` for anything that is not a death.
fn death_row_of(entry: &EventRecord) -> Option<DeathDumpLine> {
    let MatchOccurrence::Died { victim, life_sequence, attacker, attacker_life, source, gun: weapon, cam_ent_since: killcam_entity_start_time } =
        entry.event
    else {
        return None;
    };
    // Nobody to blame, or the victim themself, both read as a suicide.
    let self_inflicted = match attacker {
        Some(killer) => killer == victim,
        None => true,
    };
    Some(DeathDumpLine {
        at_tick: entry.tick.0,
        dead_player: victim.0,
        killer: attacker.map(|killer| killer.0),
        gun: weapon,
        self_kill: i32::from(self_inflicted),
        source: damage_source_label(source),
        life_no: life_sequence.0,
        killer_life: attacker_life.map(|seq| seq.0),
        replay_start_time: killcam_entity_start_time,
    })
}

impl DeathLogDumper {
    fn holds(&self, candidate: &DeathDumpLine) -> bool {
        self.rows.iter().any(|kept| (kept.at_tick, kept.dead_player) == (candidate.at_tick, candidate.dead_player))
    }

    pub(super) fn fold_in(&mut self, events: &[EventRecord]) {
        for candidate in events.iter().filter_map(death_row_of) {
            if !self.holds(&candidate) {
                self.rows.push(candidate);
            }
        }
    }
}

/// Outcome of the most recent give-weapon request.
#[derive(Resource, Debug, Clone, Default)]
pub struct GrantLogDumper {
    pub ask_id: Option<u32>,
    pub weapon: Option<u32>,
    pub accepted: Option<u8>,
    pub refusal_cause: Option<&'static str>,
}

impl GrantLogDumper {
    pub(super) fn fold_in(&mut self, events: &[EventRecord]) {
        // Only the newest give verdict in the events matters; older ones would be overwritten anyway.
        let newest = events.iter().rev().find_map(|entry| match entry.event {
            MatchOccurrence::HandoutTaken { request_id, gun: weapon } => Some((request_id, weapon, None)),
            MatchOccurrence::HandoutRefused { request_id, gun: weapon, reason } => {
                Some((request_id, weapon, Some(reason.as_str())))
            }
            _ => None,
        });
        let Some((id, gun, refusal)) = newest else { return };
        self.ask_id = Some(id);
        self.weapon = Some(gun);
        self.accepted = Some(u8::from(refusal.is_none()));
        self.refusal_cause = refusal;
    }
}

/// Outcome of the most recent weapon-configuration change request.
#[derive(Resource, Debug, Clone, Default)]
pub struct DumpConfigurationChangeLog {
    pub ask_id: Option<u32>,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub accepted: Option<bool>,
    pub refusal_cause: Option<&'static str>,
}

impl DumpConfigurationChangeLog {
    pub(super) fn fold_in(&mut self, events: &[EventRecord]) {
        let newest = events.iter().rev().find_map(|entry| match entry.event {
            MatchOccurrence::ConfigurationChangeAccepted { request_id, from, to } => Some((request_id, from, to, None)),
            MatchOccurrence::ConfigurationChangeRejected { request_id, from, to, reason } => {
                Some((request_id, from, to, Some(reason.as_str())))
            }
            _ => None,
        });
        let Some((id, old, new, refusal)) = newest else { return };
        self.ask_id = Some(id);
        (self.from, self.to) = (Some(old), Some(new));
        self.accepted = Some(refusal.is_none());
        self.refusal_cause = refusal;
    }
}

/// A player's origin, entity flags, other flags and delta time from a snapshot's player list.
pub(super) fn player_origin_flags(
    roster: &[(ClientId, playerstate_t6::game::AvatarState)],
    who: ClientId,
) -> Option<([f32; 3], i32, i32, i32)> {
    roster
        .iter()
        .filter(|entry| entry.0 == who)
        .map(|(_, avatar)| (avatar.origin, avatar.e_flags as i32, avatar.secondary_state_bits as i32, avatar.time_slice))
        .next()
}

/// Every lifecycle stage next to the word the dump commands print for it (the variant name).
const LIFECYCLE_WORDS: [(PlayerSessionStage, &str); 8] = [
    (PlayerSessionStage::Connecting, "Connecting"),
    (PlayerSessionStage::PickingLoadout, "PickingLoadout"),
    (PlayerSessionStage::AwaitingFirstLife, "AwaitingFirstLife"),
    (PlayerSessionStage::Alive, "Alive"),
    (PlayerSessionStage::Dead, "Dead"),
    (PlayerSessionStage::AwaitingNextLife, "AwaitingNextLife"),
    (PlayerSessionStage::Spectating, "Spectating"),
    (PlayerSessionStage::Intermission, "Intermission"),
];

/// The lifecycle name the dump commands print.
pub(super) fn lifecycle_label(life: PlayerSessionStage) -> &'static str {
    LIFECYCLE_WORDS
        .iter()
        .find_map(|&(stage, word)| (stage == life).then_some(word))
        .unwrap_or("Connecting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{
        ConfigurationChangeRejectReason, EventRecipients, EventSequence, GiveRejectReason,
        LifeSequence, ShotId, Tick,
    };

    fn at(tick: u32, event: MatchOccurrence) -> EventRecord {
        EventRecord { sequence: EventSequence(0), tick: Tick(tick), audience: EventRecipients::All, event }
    }

    /// A death of `victim` at `tick`; the killer (if any) is on life 5, the victim on life 2, weapon 17.
    fn killed(tick: u32, dead: u32, by: Option<u32>, how: Option<HarmOrigin>) -> EventRecord {
        let event = MatchOccurrence::Died {
            victim: ClientId(dead),
            life_sequence: LifeSequence(2),
            attacker: by.map(ClientId),
            attacker_life: by.and(Some(LifeSequence(5))),
            source: how,
            gun: 17,
            cam_ent_since: 900,
        };
        at(tick, event)
    }

    fn give_ok(id: u32, gun: u32) -> MatchOccurrence {
        MatchOccurrence::HandoutTaken { request_id: id, gun }
    }

    fn give_no(id: u32, gun: u32, why: GiveRejectReason) -> MatchOccurrence {
        MatchOccurrence::HandoutRefused { request_id: id, gun, reason: why }
    }

    fn grant_fields(log: &GrantLogDumper) -> (Option<u32>, Option<u32>, Option<u8>, Option<&'static str>) {
        (log.ask_id, log.weapon, log.accepted, log.refusal_cause)
    }

    #[test]
    fn a_kill_by_another_player_is_not_a_suicide() {
        let mut log = DeathLogDumper::default();
        log.fold_in(&[killed(10, 1, Some(2), Some(HarmOrigin::Shot(ShotId(4))))]);
        let expected = DeathDumpLine {
            at_tick: 10,
            dead_player: 1,
            killer: Some(2),
            gun: 17,
            self_kill: 0,
            source: "shot",
            life_no: 2,
            killer_life: Some(5),
            replay_start_time: 900,
        };
        assert_eq!(log.rows, vec![expected]);
    }

    #[test]
    fn world_deaths_and_self_kills_are_suicides() {
        let mut log = DeathLogDumper::default();
        log.fold_in(&[killed(10, 1, None, None), killed(11, 2, Some(2), Some(HarmOrigin::Melee))]);
        let seen: Vec<(i32, &str)> = log.rows.iter().map(|r| (r.self_kill, r.source)).collect();
        assert_eq!(seen, [(1, "none"), (1, "melee")]);
    }

    #[test]
    fn the_same_death_is_logged_once() {
        let mut log = DeathLogDumper::default();
        let once = [killed(10, 1, Some(2), None)];
        for _ in 0..2 {
            log.fold_in(&once);
        }
        assert_eq!(log.rows.len(), 1);
        log.fold_in(&[killed(11, 1, Some(2), None), killed(10, 3, Some(2), None)]);
        assert_eq!(log.rows.len(), 3);
    }

    #[test]
    fn source_labels_cover_every_kind() {
        use sim::{ProjectileId, ScriptModelId};
        let rocket = HarmOrigin::Projectile(ProjectileId(1));
        let blast = HarmOrigin::Radius(ScriptModelId::decode_net(1));
        assert_eq!(damage_source_label(Some(rocket)), "projectile");
        assert_eq!(damage_source_label(Some(blast)), "radius");
    }

    #[test]
    fn non_death_events_leave_the_death_log_alone() {
        let mut log = DeathLogDumper::default();
        log.fold_in(&[at(1, MatchOccurrence::TriggerLet)]);
        assert_eq!(log.rows.len(), 0);
    }

    #[test]
    fn give_log_follows_the_latest_verdict() {
        let mut log = GrantLogDumper::default();
        log.fold_in(&[at(1, give_no(7, 3, GiveRejectReason::NotAlive))]);
        assert_eq!(grant_fields(&log), (Some(7), Some(3), Some(0), Some("not_alive")));
        log.fold_in(&[at(2, give_ok(8, 4))]);
        assert_eq!(grant_fields(&log), (Some(8), Some(4), Some(1), None));
    }

    #[test]
    fn give_log_ignores_unrelated_journals() {
        let mut log = GrantLogDumper::default();
        log.fold_in(&[at(1, MatchOccurrence::TriggerLet)]);
        assert_eq!(grant_fields(&log), (None, None, None, None));
    }

    #[test]
    fn configuration_log_records_both_outcomes() {
        let mut log = DumpConfigurationChangeLog::default();
        let refused = MatchOccurrence::ConfigurationChangeRejected {
            request_id: 2,
            from: 10,
            to: 11,
            reason: ConfigurationChangeRejectReason::Busy,
        };
        log.fold_in(&[at(1, refused)]);
        assert_eq!((log.accepted, log.refusal_cause), (Some(false), Some("busy")));
        assert_eq!((log.from, log.to), (Some(10), Some(11)));
        let granted = MatchOccurrence::ConfigurationChangeAccepted { request_id: 3, from: 11, to: 12 };
        log.fold_in(&[at(2, granted)]);
        assert_eq!((log.accepted, log.refusal_cause, log.ask_id), (Some(true), None, Some(3)));
    }

    #[test]
    fn within_one_journal_the_last_give_event_wins() {
        let mut log = GrantLogDumper::default();
        log.fold_in(&[at(1, give_ok(1, 1)), at(1, give_no(2, 2, GiveRejectReason::InvalidWeapon))]);
        assert_eq!((log.ask_id, log.refusal_cause), (Some(2), Some("invalid_weapon")));
    }

    #[test]
    fn lifecycle_labels_are_the_variant_names() {
        for (stage, word) in LIFECYCLE_WORDS {
            assert_eq!(lifecycle_label(stage), word);
            assert_eq!(format!("{stage:?}"), word);
        }
    }

    #[test]
    fn player_lookup_misses_unknown_clients() {
        assert!(player_origin_flags(&[], ClientId(1)).is_none());
    }
}
