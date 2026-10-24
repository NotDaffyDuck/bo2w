//! The sim's own events: verdicts and facts that only the authority can know.

use super::MatchEndReason;
use super::loadout::{ClassRejectReason, ConfigurationChangeRejectReason, GiveRejectReason};
use crate::identities::{DamageSource, LifeSequence};
use crate::input::{ActionRequestId, ClassId};
use crate::world::ClientId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimEvent {
    ClassAccepted { request_id: ActionRequestId, class_id: ClassId, revision: u32 },
    ClassRejected { request_id: ActionRequestId, class_id: ClassId, revision: u32, reason: ClassRejectReason },
    GiveAccepted { request_id: ActionRequestId, weapon: u32 },
    GiveRejected { request_id: ActionRequestId, weapon: u32, reason: GiveRejectReason },
    ConfigurationChangeAccepted { request_id: ActionRequestId, from: u32, to: u32 },
    ConfigurationChangeRejected {
        request_id: ActionRequestId,
        from: u32,
        to: u32,
        reason: ConfigurationChangeRejectReason,
    },
    Spawned { class_id: ClassId, spawn_index: u32, life_sequence: LifeSequence },
    Died {
        victim: ClientId,
        life_sequence: LifeSequence,
        attacker: Option<ClientId>,
        attacker_life: Option<LifeSequence>,
        source: Option<DamageSource>,
        weapon: u32,
        killcam_entity_start_time: i32,
    },
    ScoreChanged { client: ClientId, score: i32, kills: i32, deaths: i32 },
    MatchEnded { reason: MatchEndReason },
    AttackReleased,
    WeaponSwitchRequested { weapon: u32 },
}

impl SimEvent {
    /// The name of the variant, as the roster lists it.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::ClassAccepted { .. } => "ClassAccepted",
            Self::ClassRejected { .. } => "ClassRejected",
            Self::GiveAccepted { .. } => "GiveAccepted",
            Self::GiveRejected { .. } => "GiveRejected",
            Self::ConfigurationChangeAccepted { .. } => "ConfigurationChangeAccepted",
            Self::ConfigurationChangeRejected { .. } => "ConfigurationChangeRejected",
            Self::Spawned { .. } => "Spawned",
            Self::Died { .. } => "Died",
            Self::ScoreChanged { .. } => "ScoreChanged",
            Self::MatchEnded { .. } => "MatchEnded",
            Self::AttackReleased => "AttackReleased",
            Self::WeaponSwitchRequested { .. } => "WeaponSwitchRequested",
        }
    }
}

/// One line of the roster: a variant, whether it must arrive, and why a client cannot work it out alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimEventRow {
    pub variant: &'static str,
    pub reliable: bool,
    pub control_fact: &'static str,
}

const fn row(variant: &'static str, reliable: bool, control_fact: &'static str) -> SimEventRow {
    SimEventRow { variant, reliable, control_fact }
}

/// Every sim event, with the reason it is sent. Only `AttackReleased` may be dropped.
pub const SIM_EVENT_ROSTER: &[SimEventRow] = &[
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
pub fn sim_event_is_reliable(event: &SimEvent) -> bool {
    let name = event.variant_name();
    SIM_EVENT_ROSTER.iter().any(|entry| entry.variant == name && entry.reliable)
}
