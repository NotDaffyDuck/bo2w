use bevy::prelude::Resource;
use entity_t6::EventCode;

use crate::client::presentation::event_table::{EntityEventHandlerKind, event_route_of};

/// Server events this client has no handler for. All of them are counted; only the first one is
/// logged.
#[derive(Resource, Default, Debug)]
pub struct UnhandledEntityEvents {
    pub count_all: u32,
    pub earliest: Option<EventCode>,
    printed: bool,
}

impl UnhandledEntityEvents {
    pub(super) fn note(&mut self, kind: EventCode) {
        self.count_all = self.count_all.saturating_add(1);
        self.earliest.get_or_insert(kind);

        // `replace` hands back the old flag, so every call after the first stays quiet.
        if std::mem::replace(&mut self.printed, true) {
            return;
        }
        let (name, why) = kind_words(kind);
        diag::warn!(
            WireLink,
            "entity events: {} (#{:#x}) has no client handler — {} \
             (later ones are only counted)",
            name,
            kind.0,
            why
        );
    }
}

/// The registry's name for `kind` and the reason it has no handler.
fn kind_words(kind: EventCode) -> (&'static str, &'static str) {
    match event_route_of(kind) {
        None => (
            "not modelled",
            "missing from entity_t6::EventCode::TAXONOMY, so this build does not model \
             the slot at all",
        ),
        Some(row) => match row.dispatch {
            EntityEventHandlerKind::NoHandler(reason) => (row.name, reason),
            EntityEventHandlerKind::Observer(_) => (
                row.name,
                "the registry lists an observer for it but the classifier refused it \
                 (G-BUS-1 was meant to prevent that)",
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_adds_to_the_total_while_the_first_stays_remembered() {
        let mut seen = UnhandledEntityEvents::default();
        assert!(!seen.printed);
        for raw in [4001, 4002, 4001, 4003] {
            seen.note(EventCode(raw));
        }
        assert_eq!(seen.count_all, 4);
        assert_eq!(seen.earliest, Some(EventCode(4001)));
        assert!(seen.printed);
    }

    #[test]
    fn an_event_missing_from_the_table_is_described_as_unmodelled() {
        let (name, why) = kind_words(EventCode(i32::MAX));
        assert_eq!((name, why.contains("does not model")), ("not modelled", true));
    }

    #[test]
    fn the_deliberately_unsupported_event_names_itself() {
        let (name, why) = kind_words(EventCode::RELOAD_ADDAMMO);
        assert_eq!(name, "EV_RELOAD_ADDAMMO");
        assert!(why.contains("double-count"));
    }
}
