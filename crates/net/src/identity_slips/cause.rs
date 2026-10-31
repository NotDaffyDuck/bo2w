//! Why a gap was raised.

use core::fmt;

use diag::gap::ShortfallOrigin;
use sim::ClientId;

use super::kinds::{LinkDeficiency, ScriptEventKind};
use crate::client::presentation::entity_slots::ENTITY_NUMBER_COUNT;

/// The specific thing that went wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkDeficiencyOrigin {
    ClientOutOfNumberSpace { client: ClientId },
    NumberClaimedTwice { number: u16 },
    EventNumberHasNoEntity { number: i32 },
    EventNumberOutOfRange { number: i32 },
    NotifyTargetHasNoEntity { peer: ClientId, notify: ScriptEventKind },
}

impl ShortfallOrigin for LinkDeficiencyOrigin {
    type Gap = LinkDeficiency;

    fn gap(&self) -> LinkDeficiency {
        match *self {
            Self::NotifyTargetHasNoEntity { .. } => LinkDeficiency::ScenarioAlertRecipient,
            Self::EventNumberOutOfRange { .. } | Self::EventNumberHasNoEntity { .. } => {
                LinkDeficiency::ObjectEventRecipient
            }
            Self::NumberClaimedTwice { .. } | Self::ClientOutOfNumberSpace { .. } => {
                LinkDeficiency::ClientEntityIndex
            }
        }
    }
}

impl fmt::Display for LinkDeficiencyOrigin {
    fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Three of the causes are about an entity number and share the opening.
        let (number, what_happened) = match *self {
            Self::NumberClaimedTwice { number } => {
                (i64::from(number), "was claimed twice in one snapshot")
            }
            Self::EventNumberHasNoEntity { number } => (i64::from(number), "has no live entity"),
            Self::EventNumberOutOfRange { number } => {
                (i64::from(number), "is not a u16 entity number")
            }
            Self::ClientOutOfNumberSpace { client: ClientId(id) } => {
                let slots = ENTITY_NUMBER_COUNT;
                return write!(sink, "client {id} is outside the {slots}-slot entity-number space");
            }
            Self::NotifyTargetHasNoEntity { peer: ClientId(id), notify } => {
                return write!(sink, "{notify} for client {id} has no player entity");
            }
        };
        write!(sink, "entity number {number} {what_happened}")
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: ClientId = ClientId(40);

    #[test]
    fn each_cause_belongs_to_one_gap() {
        let abort = ScriptEventKind::AbortKillcam;
        let by_gap = [
            (LinkDeficiency::ClientEntityIndex, LinkDeficiencyOrigin::ClientOutOfNumberSpace { client: CLIENT }),
            (LinkDeficiency::ClientEntityIndex, LinkDeficiencyOrigin::NumberClaimedTwice { number: 9 }),
            (LinkDeficiency::ObjectEventRecipient, LinkDeficiencyOrigin::EventNumberHasNoEntity { number: 9 }),
            (LinkDeficiency::ObjectEventRecipient, LinkDeficiencyOrigin::EventNumberOutOfRange { number: -1 }),
            (LinkDeficiency::ScenarioAlertRecipient, LinkDeficiencyOrigin::NotifyTargetHasNoEntity { peer: CLIENT, notify: abort }),
        ];
        for (expected, cause) in by_gap {
            assert_eq!(cause.gap(), expected, "{cause:?}");
        }
    }

    #[test]
    fn the_messages_name_what_was_wrong() {
        let said = |cause: LinkDeficiencyOrigin| cause.to_string();
        assert_eq!(
            said(LinkDeficiencyOrigin::ClientOutOfNumberSpace { client: CLIENT }),
            format!("client 40 is outside the {ENTITY_NUMBER_COUNT}-slot entity-number space")
        );
        assert_eq!(
            said(LinkDeficiencyOrigin::NumberClaimedTwice { number: 12 }),
            "entity number 12 was claimed twice in one snapshot"
        );
        assert_eq!(
            said(LinkDeficiencyOrigin::EventNumberHasNoEntity { number: 77 }),
            "entity number 77 has no live entity"
        );
        assert_eq!(
            said(LinkDeficiencyOrigin::EventNumberOutOfRange { number: 70000 }),
            "entity number 70000 is not a u16 entity number"
        );
        assert_eq!(
            said(LinkDeficiencyOrigin::NotifyTargetHasNoEntity {
                peer: CLIENT,
                notify: ScriptEventKind::SpawnedPlayer,
            }),
            "SpawnedPlayer for client 40 has no player entity"
        );
    }
}
