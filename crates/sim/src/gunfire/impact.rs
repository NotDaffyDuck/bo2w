//! Which hits show an impact, and which event announces one.

use crate::shot_trace::{HitTargetId, HITLOC_SHIELD, is_sky};

/// A hit shows an impact unless it landed on the sky (or touched nothing).
pub(crate) fn shows_impact(collider: Option<HitTargetId>) -> bool {
    collider.is_some_and(|hit| !is_sky(hit))
}

/// Whether the hit struck the riot shield a player bears. Only Black Ops II content has these, with their own
/// events (`t6::riotshield::broadcast_hit`).
pub(crate) fn on_shield(collider: Option<HitTargetId>, bo2: bool) -> bool {
    bo2 && matches!(collider, Some(HitTargetId::Player { hitloc: HITLOC_SHIELD, .. }))
}

/// The bullet hit event for a weapon's impact type; `local` picks the one the struck player gets. Black Ops II's
/// list puts armour-piercing at 3, where the legacy list has the shield, so there it hits like a large round (2).
pub(crate) fn hit_event(impact_kind_id: i32, local: bool, bo2: bool) -> Option<entity_t6::EventCode> {
    let kind = match impact_kind_id {
        3 if bo2 => 2,
        rest_one => rest_one,
    };
    entity_t6::bullet_hit_event(kind, local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_hit_shows_no_impact() {
        assert!(!shows_impact(None));
    }

    #[test]
    fn only_bo2_reads_armour_piercing_as_a_large_round() {
        assert_eq!(hit_event(3, false, true), hit_event(2, false, true));
        assert_eq!(hit_event(3, true, true), hit_event(2, true, true));
        assert_eq!(hit_event(3, false, false), entity_t6::bullet_hit_event(3, false));
    }

    #[test]
    fn a_shield_hit_needs_the_bo2_flag() {
        assert!(!on_shield(None, true));
    }
}
