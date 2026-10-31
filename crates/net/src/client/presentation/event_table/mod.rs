//! The table of entity events a client knows about, and what it does for each.

mod rows;

use entity_t6::{EntityEventResponse, EventCode};

/// What the client does on seeing an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityEventHandlerKind {
    /// Handled by an observer, as this action.
    Observer(EntityEventResponse),
    /// Handled by nothing, for this reason.
    NoHandler(&'static str),
}

/// A table row: the event's name as the game's tables spell it, the event, and its handling.
#[derive(Clone, Copy, Debug)]
pub struct EntityEventTableEntry {
    pub name: &'static str,
    pub kind: EventCode,
    pub dispatch: EntityEventHandlerKind,
}

const ROW_COUNT: usize = rows::OBSERVED.len() + rows::UNSUPPORTED.len();

/// Both groups of rows laid end to end in one array.
const fn joined(
    first: &[EntityEventTableEntry],
    second: &[EntityEventTableEntry],
) -> [EntityEventTableEntry; ROW_COUNT] {
    let mut all = [first[0]; ROW_COUNT];
    let mut slot = 0;
    while slot < ROW_COUNT {
        all[slot] = if slot < first.len() {
            first[slot]
        } else {
            second[slot - first.len()]
        };
        slot += 1;
    }
    all
}

const ALL_ROWS: [EntityEventTableEntry; ROW_COUNT] = joined(rows::OBSERVED, rows::UNSUPPORTED);

pub const EVENT_ROUTE_TABLE: &[EntityEventTableEntry] = &ALL_ROWS;

/// The table's row for `event`, when it has one.
pub fn event_route_of(event: EventCode) -> Option<&'static EntityEventTableEntry> {
    EVENT_ROUTE_TABLE
        .iter()
        .find(|candidate| candidate.kind.0 == event.0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn observer_action(event: EventCode) -> Option<EntityEventResponse> {
        match event_route_of(event)?.dispatch {
            EntityEventHandlerKind::Observer(action) => Some(action),
            EntityEventHandlerKind::NoHandler(_) => None,
        }
    }

    #[test]
    fn the_registry_is_the_observed_rows_followed_by_the_unsupported_ones() {
        let row_total = rows::OBSERVED.len() + rows::UNSUPPORTED.len();
        assert_eq!(row_total, 137);
        assert_eq!(EVENT_ROUTE_TABLE.len(), row_total);
        let (observed, rest) = EVENT_ROUTE_TABLE.split_at(rows::OBSERVED.len());
        assert_eq!(
            observed.last().map(|row| row.kind),
            rows::OBSERVED.last().map(|row| row.kind)
        );
        assert_eq!(rest.first().map(|row| row.name), Some("EV_RELOAD_ADDAMMO"));
    }

    #[test]
    fn events_and_names_are_unique() {
        let distinct_events: HashSet<i32> =
            EVENT_ROUTE_TABLE.iter().map(|r| r.kind.0).collect();
        let distinct_names: HashSet<&str> = EVENT_ROUTE_TABLE.iter().map(|r| r.name).collect();
        assert_eq!(
            distinct_events.len(),
            EVENT_ROUTE_TABLE.len(),
            "an event repeats"
        );
        assert_eq!(
            distinct_names.len(),
            EVENT_ROUTE_TABLE.len(),
            "a name repeats"
        );
    }

    #[test]
    fn names_carry_the_ev_prefix() {
        let misnamed: Vec<_> = EVENT_ROUTE_TABLE
            .iter()
            .filter(|row| !row.name.starts_with("EV_"))
            .collect();
        assert!(misnamed.is_empty(), "{misnamed:?}");
    }

    #[test]
    fn the_reload_top_up_is_the_only_unsupported_row_and_gives_its_reason() {
        let mut reasons = EVENT_ROUTE_TABLE
            .iter()
            .filter_map(|row| match row.dispatch {
                EntityEventHandlerKind::NoHandler(reason) => Some((row.name, reason)),
                EntityEventHandlerKind::Observer(_) => None,
            });
        let (name, reason) = reasons.next().expect("one unsupported row");
        assert!(reasons.next().is_none());
        assert_eq!(name, "EV_RELOAD_ADDAMMO");
        assert!(reason.contains("double-count"));
    }

    #[test]
    fn lookup_by_event_returns_the_matching_row() {
        assert!(event_route_of(EventCode(i32::MAX)).is_none());

        let top_up =
            event_route_of(EventCode::RELOAD_ADDAMMO).expect("reload top-up is listed");
        assert!(matches!(
            top_up.dispatch,
            EntityEventHandlerKind::NoHandler(_)
        ));

        let fire = event_route_of(EventCode::FIRE_WEAPON).expect("fire is in the table");
        assert_eq!(
            observer_action(fire.kind),
            Some(EntityEventResponse::GunShot)
        );
        assert_eq!(fire.name, "EV_FIRE_WEAPON");
    }

    #[test]
    fn jumps_landings_and_footsteps_are_movement_sounds() {
        let movement = [
            EventCode::LEAP,
            EventCode::LANDING_WOOD,
            EventCode::FOOTSTEP_RUN,
        ];
        for event in movement {
            assert_eq!(
                observer_action(event),
                Some(EntityEventResponse::FootstepNoise)
            );
        }
    }
}
