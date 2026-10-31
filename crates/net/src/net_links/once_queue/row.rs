use sim::{IntentSerial, MatchOccurrence};

/// How the server answered a client's request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionRulingNotice {
    TakenIntoEffect,
    Declined,
    DuplicateIdDifferentBody,
    TimedOut,
}

impl ActionRulingNotice {
    pub(super) const fn tag(self) -> u8 {
        match self {
            Self::TakenIntoEffect => 0,
            Self::Declined => 1,
            Self::DuplicateIdDifferentBody => 2,
            Self::TimedOut => 3,
        }
    }

    pub(super) const fn from_tag(tag: u8) -> Option<Self> {
        Some(match tag {
            0 => Self::TakenIntoEffect,
            1 => Self::Declined,
            2 => Self::DuplicateIdDifferentBody,
            3 => Self::TimedOut,
            _ => return None,
        })
    }
}

/// One thing a client is owed.
#[derive(Clone, Debug, PartialEq)]
pub enum GuaranteedEntry {
    RefusalText(String),
    Sound(crate::HostSoundCue),
    Card(crate::HostCardSlotMsg),
    Menu(crate::HostMenuOpenMsg),
    Splash(crate::HostScreenFlash),
    Notify(crate::HostGameAnnouncement),
    Standings(String),
    Event(MatchOccurrence),
    CommandResolution {
        request_id: IntentSerial,
        verdict: ActionRulingNotice,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ActionRulingNotice; 4] = [
        ActionRulingNotice::TakenIntoEffect,
        ActionRulingNotice::Declined,
        ActionRulingNotice::DuplicateIdDifferentBody,
        ActionRulingNotice::TimedOut,
    ];

    #[test]
    fn verdict_tags_are_zero_to_three_in_order() {
        let tags: Vec<u8> = ALL.iter().map(|verdict| verdict.tag()).collect();
        assert_eq!(tags, [0, 1, 2, 3]);
    }

    #[test]
    fn every_verdict_survives_its_tag() {
        for verdict in ALL {
            assert_eq!(ActionRulingNotice::from_tag(verdict.tag()), Some(verdict));
        }
    }

    #[test]
    fn a_tag_past_the_last_verdict_is_unknown() {
        assert_eq!(ActionRulingNotice::from_tag(4), None);
        assert_eq!(ActionRulingNotice::from_tag(u8::MAX), None);
    }
}
