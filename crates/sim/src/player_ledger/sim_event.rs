//! The sim's own events: verdicts and facts that only the authority can know.

use super::MatchOverCause;
use super::loadout::{LoadoutRefusal, ConfigurationChangeRejectReason, HandoutRefusal};
use crate::match_ids::LifeCount;
use crate::tick_feed::LoadoutClass;
use crate::sim_parts::SeatNo;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatchOccurrence {
    LoadoutTaken { request_id: u32, class_id: LoadoutClass, revision: u32 },
    LoadoutRefused { request_id: u32, class_id: LoadoutClass, revision: u32, reason: LoadoutRefusal },
    HandoutTaken { request_id: u32, gun: u32 },
    HandoutRefused { request_id: u32, gun: u32, reason: HandoutRefusal },
    ConfigurationChangeAccepted { request_id: u32, from: u32, to: u32 },
    ConfigurationChangeRejected {
        request_id: u32,
        from: u32,
        to: u32,
        reason: ConfigurationChangeRejectReason,
    },
    Spawned { class_id: LoadoutClass, spawn_slot: u32, life_sequence: LifeCount },
    Died {
        victim: SeatNo,
        life_sequence: crate::match_ids::LifeCount,
        attacker: Option<crate::sim_parts::SeatNo>,
        attacker_life: Option<crate::match_ids::LifeCount>,
        source: Option<crate::match_ids::HarmOrigin>,
        gun: u32,
        cam_ent_since: i32,
    },
    TallyMoved { client: SeatNo, score: i32, kills: i32, times_died: i32 },
    MatchEnded { reason: MatchOverCause },
    TriggerLet,
    WeaponSwitchRequested { weapon: u32 },
}

impl MatchOccurrence {
    /// The name of the variant, as the roster lists it.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::LoadoutTaken { .. } => "ClassAccepted",
            Self::LoadoutRefused { .. } => "ClassRejected",
            Self::HandoutTaken { .. } => "GiveAccepted",
            Self::HandoutRefused { .. } => "GiveRejected",
            Self::ConfigurationChangeAccepted { .. } => "ConfigurationChangeAccepted",
            Self::ConfigurationChangeRejected { .. } => "ConfigurationChangeRejected",
            Self::Spawned { .. } => "Spawned",
            Self::Died { .. } => "Died",
            Self::TallyMoved { .. } => "ScoreChanged",
            Self::MatchEnded { .. } => "MatchEnded",
            Self::TriggerLet => "AttackReleased",
            Self::WeaponSwitchRequested { .. } => "WeaponSwitchRequested",
        }
    }
}

/// One line of the roster: a variant, whether it must arrive, and why a client cannot work it out alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccurrenceTableRow {
    pub shape: &'static str,
    pub sure: bool,
    pub wire_control_note: &'static str,
}

const fn row(variant: &'static str, reliable: bool, wire_control_note: &'static str) -> OccurrenceTableRow {
    OccurrenceTableRow { shape: variant, sure: reliable, wire_control_note }
}

/// Every sim event, with the reason it is sent. Only `AttackReleased` may be dropped.
pub const OCCURRENCE_TABLE: &[OccurrenceTableRow] = &[
    row(
        "ClassAccepted",
        true,
        "the authority's yes on a class request, with the revision the client must echo back; the client asked, \
         so it cannot know the answer",
    ),
    row(
        "ClassRejected",
        true,
        "the no and its reason; without it a client waits on a request the authority already closed",
    ),
    row("GiveAccepted", true, "a console or dev give went through; names the weapon index the authority settled on"),
    row("GiveRejected", true, "a give was refused, with the reason, so the console can say why"),
    row("ConfigurationChangeAccepted", true, "the authority took the held weapon's attachment-set change"),
    row("ConfigurationChangeRejected", true, "the authority refused a stale or invalid attachment-set change"),
    row(
        "Spawned",
        true,
        "which authored spawn point the authority chose and the life it started; choosing a spawn is authority-only",
    ),
    row(
        "Died",
        true,
        "the life that ended and why: attacker, source and the killcam entity's birth time, none of which a client \
         can work out from its own simulation (the on-screen obituary is a different event)",
    ),
    row(
        "ScoreChanged",
        true,
        "the scoreboard row once the authority applied it; score has no predicted twin on the client",
    ),
    row(
        "MatchEnded",
        true,
        "the phase change and which limit caused it; a client has neither the real clock nor everyone's score",
    ),
    row(
        "AttackReleased",
        false,
        "the authority's reading of the attack button for other players' avatars, whose commands this client \
         never sees; for the local player it only repeats the client's own input, hence the one droppable event",
    ),
    row(
        "WeaponSwitchRequested",
        true,
        "the authority asks a client to pick a weapon; the client owns selection, so a server-side switch has to \
         travel as that client's command to get the drop and raise",
    ),
];

/// Whether `event` must be delivered, by the roster.
pub fn occurrence_needs_delivery(ev: &MatchOccurrence) -> bool {
    let name = ev.variant_name();
    OCCURRENCE_TABLE.iter().any(|entry| entry.shape == name && entry.sure)
}
