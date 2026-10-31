//! Decides whether a fired weapon's impact needs a client-side trace.

use fx_t6::runtime::impact_table_row;

/// Impact types that arrive from the server as bullet-hit events, so the client must not
/// trace them a second time. They cover the bullet types and the shield type.
///
/// TODO(fact): the numbers (1, 2, 9 for bullets, 3 for shields) mirror the rows used by
/// `impact_table_row`; the proper names for these impact types are not recorded yet.
const EVENT_DRIVEN_IMPACT_TYPES: [i32; 4] = [1, 2, 3, 9];

/// True when `impact_kind_id` has a table row and is not already delivered as an event.
pub fn impact_needs_client_trace(impact_kind_id: i32) -> bool {
    impact_table_row(impact_kind_id, false).is_some_and(|_| !EVENT_DRIVEN_IMPACT_TYPES.contains(&impact_kind_id))
}
