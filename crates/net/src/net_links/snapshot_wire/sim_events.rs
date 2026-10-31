//! What happened in the match, as the clients are told: class and weapon answers, spawns, deaths,
//! scores and the end of the match, each addressed to an audience and stamped with a sequence.

use sim::{
    ClassRejectReason, ConfigurationChangeRejectReason, HarmOrigin, EventRecipients, EventRecord,
    GiveRejectReason, MatchOverCause, ProjectileId, ShotId, MatchOccurrence,
};

use super::codec::{Wire, put_short_list, refuse, take_short_list};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_tags! {
    ClassRejectReason, "unknown ClassRejectReason tag", {
        ClassRejectReason::UnknownOrStaleClass => 1,
        ClassRejectReason::NoSpawnAvailable => 2,
        ClassRejectReason::LockedContent => 3,
        ClassRejectReason::NoSuchGun => 4,
    }
}

wire_tags! {
    GiveRejectReason, "unknown GiveRejectReason tag", {
        GiveRejectReason::NotAlive => 1,
        GiveRejectReason::InvalidWeapon => 2,
        GiveRejectReason::NoSuchGun => 3,
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
    MatchOverCause, "unknown MatchEndReason tag", {
        MatchOverCause::PointsReached => 0,
        MatchOverCause::TimeLimit => 1,
    }
}

wire_newtype! {
    ShotId => u32,
    ProjectileId => u32,
}

impl Wire for HarmOrigin {
    fn put(&self, wr: &mut ByteSinkWriter) {
        match self {
            Self::Shot(shot) => {
                wr.put_u8(1);
                shot.put(wr);
            }
            Self::Projectile(projectile) => {
                wr.put_u8(2);
                projectile.put(wr);
            }
            Self::Radius(model) => {
                wr.put_u8(3);
                model.put(wr);
            }
            Self::Melee => wr.put_u8(4),
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            1 => Ok(Self::Shot(Wire::take(rd)?)),
            2 => Ok(Self::Projectile(Wire::take(rd)?)),
            3 => Ok(Self::Radius(Wire::take(rd)?)),
            4 => Ok(Self::Melee),
            _ => refuse("unknown DamageSource tag"),
        }
    }
}

wire_variants! {
    MatchOccurrence, "unknown SimEvent tag", {
        1 => LoadoutTaken { request_id, class_id, revision },
        2 => LoadoutRefused { request_id, class_id, revision, reason },
        3 => Spawned { class_id, spawn_slot, life_sequence },
        5 => TriggerLet,
        6 => Died {
            victim,
            life_sequence,
            attacker: flag "bad Died attacker tag",
            attacker_life: flag "bad Died attacker_life tag",
            source: flag "bad Died source tag",
            gun,
            cam_ent_since,
        },
        11 => TallyMoved { client, score, kills, times_died },
        12 => MatchEnded { reason },
        14 => HandoutTaken { request_id, gun },
        15 => HandoutRefused { request_id, gun, reason },
        16 => ConfigurationChangeAccepted { request_id, from, to },
        17 => ConfigurationChangeRejected { request_id, from, to, reason },
        18 => WeaponSwitchRequested { weapon },
    }
}

impl Wire for EventRecipients {
    fn put(&self, wr: &mut ByteSinkWriter) {
        match self {
            Self::All => wr.put_u8(0),
            Self::Client(client) => {
                wr.put_u8(1);
                client.put(wr);
            }
            Self::Clients(clients) => {
                wr.put_u8(2);
                put_short_list(wr, clients);
            }
            Self::AllExcept(client) => {
                wr.put_u8(3);
                client.put(wr);
            }
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            0 => Ok(Self::All),
            1 => Ok(Self::Client(Wire::take(rd)?)),
            2 => Ok(Self::Clients(take_short_list(rd, 16)?)),
            3 => Ok(Self::AllExcept(Wire::take(rd)?)),
            _ => refuse("unknown EventAudience tag"),
        }
    }
}

wire_fields!(EventRecord { sequence, tick, audience, event });

pub(crate) fn pack_occurrence(wr: &mut ByteSinkWriter, event: &MatchOccurrence) {
    event.put(wr);
}

pub(crate) fn read_occurrence(rd: &mut ByteCursorReader<'_>) -> Result<MatchOccurrence, PacketDecodeFault> {
    MatchOccurrence::take(rd)
}
