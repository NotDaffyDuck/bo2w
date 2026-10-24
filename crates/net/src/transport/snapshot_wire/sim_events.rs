//! What happened in the match, as the clients are told: class and weapon answers, spawns, deaths,
//! scores and the end of the match, each addressed to an audience and stamped with a sequence.

use sim::{
    ClassRejectReason, ConfigurationChangeRejectReason, DamageSource, EventAudience, EventRecord,
    GiveRejectReason, MatchEndReason, ProjectileId, ShotId, SimEvent,
};

use super::codec::{Decoded, Wire, put_short_list, refuse, take_short_list};
use crate::transport::wire::{WireReader, WireWriter};

wire_tags! {
    ClassRejectReason, "unknown ClassRejectReason tag", {
        ClassRejectReason::UnknownOrStaleClass => 1,
        ClassRejectReason::NoSpawnAvailable => 2,
        ClassRejectReason::LockedContent => 3,
        ClassRejectReason::UnknownWeaponId => 4,
    }
}

wire_tags! {
    GiveRejectReason, "unknown GiveRejectReason tag", {
        GiveRejectReason::NotAlive => 1,
        GiveRejectReason::InvalidWeapon => 2,
        GiveRejectReason::UnknownWeaponId => 3,
        GiveRejectReason::EmptyCombatProfile => 4,
        GiveRejectReason::UnsupportedWeapon => 5,
    }
}

wire_tags! {
    ConfigurationChangeRejectReason, "unknown ConfigurationChangeRejectReason tag", {
        ConfigurationChangeRejectReason::NotAlive => 1,
        ConfigurationChangeRejectReason::StaleSource => 2,
        ConfigurationChangeRejectReason::InvalidTarget => 3,
        ConfigurationChangeRejectReason::DifferentFamily => 4,
        ConfigurationChangeRejectReason::NoInventorySlot => 5,
        ConfigurationChangeRejectReason::AmmoTableFull => 6,
        ConfigurationChangeRejectReason::Busy => 7,
        ConfigurationChangeRejectReason::SharedAmmoConflict => 8,
    }
}

wire_tags! {
    MatchEndReason, "unknown MatchEndReason tag", {
        MatchEndReason::ScoreLimit => 0,
        MatchEndReason::TimeLimit => 1,
    }
}

wire_newtype! {
    ShotId => u32,
    ProjectileId => u32,
}

impl Wire for DamageSource {
    fn put(&self, out: &mut WireWriter) {
        match self {
            Self::Shot(shot) => {
                out.put_u8(1);
                shot.put(out);
            }
            Self::Projectile(projectile) => {
                out.put_u8(2);
                projectile.put(out);
            }
            Self::Radius(model) => {
                out.put_u8(3);
                model.put(out);
            }
            Self::Melee => out.put_u8(4),
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            1 => Ok(Self::Shot(Wire::take(input)?)),
            2 => Ok(Self::Projectile(Wire::take(input)?)),
            3 => Ok(Self::Radius(Wire::take(input)?)),
            4 => Ok(Self::Melee),
            _ => refuse("unknown DamageSource tag"),
        }
    }
}

wire_variants! {
    SimEvent, "unknown SimEvent tag", {
        1 => ClassAccepted { request_id, class_id, revision },
        2 => ClassRejected { request_id, class_id, revision, reason },
        3 => Spawned { class_id, spawn_index, life_sequence },
        5 => AttackReleased,
        6 => Died {
            victim,
            life_sequence,
            attacker: flag "bad Died attacker tag",
            attacker_life: flag "bad Died attacker_life tag",
            source: flag "bad Died source tag",
            weapon,
            killcam_entity_start_time,
        },
        11 => ScoreChanged { client, score, kills, deaths },
        12 => MatchEnded { reason },
        14 => GiveAccepted { request_id, weapon },
        15 => GiveRejected { request_id, weapon, reason },
        16 => ConfigurationChangeAccepted { request_id, from, to },
        17 => ConfigurationChangeRejected { request_id, from, to, reason },
        18 => WeaponSwitchRequested { weapon },
    }
}

impl Wire for EventAudience {
    fn put(&self, out: &mut WireWriter) {
        match self {
            Self::All => out.put_u8(0),
            Self::Client(client) => {
                out.put_u8(1);
                client.put(out);
            }
            Self::Clients(clients) => {
                out.put_u8(2);
                put_short_list(out, clients);
            }
            Self::AllExcept(client) => {
                out.put_u8(3);
                client.put(out);
            }
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            0 => Ok(Self::All),
            1 => Ok(Self::Client(Wire::take(input)?)),
            2 => Ok(Self::Clients(take_short_list(input, 16)?)),
            3 => Ok(Self::AllExcept(Wire::take(input)?)),
            _ => refuse("unknown EventAudience tag"),
        }
    }
}

wire_fields!(EventRecord { sequence, tick, audience, event });

pub(crate) fn encode_event(out: &mut WireWriter, event: &SimEvent) {
    event.put(out);
}

pub(crate) fn decode_event(input: &mut WireReader<'_>) -> Decoded<SimEvent> {
    SimEvent::take(input)
}
