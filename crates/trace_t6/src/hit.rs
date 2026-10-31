//! What a trace hit: BO2's `TraceHitType`, the entity numbers and the
//! `Trace_*HitId` helpers.

use crate::result::TraceResult;

/// The game's entity slots (`entity_t6::MAX_GENTITIES`; BO2 has 1024).
pub const MAX_GENTITIES: u16 = entity_t6::MAX_GENTITIES as u16;

/// The world: what a trace that hit a dynamic entity, glass or a static model
/// reports as its entity (`Trace_GetEntityHitId`).
pub const WORLD_ENTITY_SLOT: u16 = MAX_GENTITIES - 2;
/// Older spelling ENTITYNUM_WORLD still calls; the rename sweep moves it over.
pub use self::WORLD_ENTITY_SLOT as ENTITYNUM_WORLD;
/// Older spelling trace_get_entity_hit_id still calls; the rename sweep moves it over.
pub use self::entity_hit_slot as trace_get_entity_hit_id;

/// No entity.
pub const NO_ENTITY_SLOT: u16 = MAX_GENTITIES - 1;

/// BO2's `TraceHitType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TraceHitType {
    #[default]
    None = 0,
    Entity = 1,
    DynEntModel = 2,
    DynEntBrush = 3,
    Glass = 4,
}

/// `Trace_SetHitId`.
pub fn trace_set_hit_id(trace: &mut TraceResult, impact_category: TraceHitType, struck_entity_num: u16) {
    *trace = TraceResult { impact_category, struck_entity_num, ..*trace };
}

/// `Trace_GetEntityHitId`: the entity a trace hit. Dynamic entities and glass
/// count as the world; a trace that hit nothing gives `NO_ENTITY_SLOT`.
#[must_use]
pub const fn entity_hit_slot(trace: &TraceResult) -> u16 {
    match trace.impact_category {
        TraceHitType::DynEntModel | TraceHitType::DynEntBrush | TraceHitType::Glass => {
            WORLD_ENTITY_SLOT
        }
        TraceHitType::Entity => trace.struck_entity_num,
        TraceHitType::None => NO_ENTITY_SLOT,
    }
}

/// `Trace_GetGlassHitId`: the glass piece a trace hit, or `None` (BO2
/// returns -1).
#[must_use]
pub const fn glass_hit_slot(trace: &TraceResult) -> Option<u16> {
    match trace.impact_category {
        TraceHitType::Glass => Some(trace.struck_entity_num),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(impact_category: TraceHitType, struck_entity_num: u16) -> TraceResult {
        let mut t = TraceResult::new();
        trace_set_hit_id(&mut t, impact_category, struck_entity_num);
        t
    }

    #[test]
    fn entity_numbers_are_bo2s() {
        assert_eq!(WORLD_ENTITY_SLOT, MAX_GENTITIES - 2);
        assert_eq!(NO_ENTITY_SLOT, MAX_GENTITIES - 1);
    }

    #[test]
    fn entity_hit_id_per_hit_type() {
        assert_eq!(entity_hit_slot(&hit(TraceHitType::None, 7)), NO_ENTITY_SLOT);
        assert_eq!(entity_hit_slot(&hit(TraceHitType::Entity, 7)), 7);
        for t in [TraceHitType::DynEntModel, TraceHitType::DynEntBrush, TraceHitType::Glass] {
            assert_eq!(entity_hit_slot(&hit(t, 7)), WORLD_ENTITY_SLOT);
        }
    }

    #[test]
    fn glass_ids() {
        assert_eq!(glass_hit_slot(&hit(TraceHitType::Glass, 9)), Some(9));
        assert_eq!(glass_hit_slot(&hit(TraceHitType::Entity, 9)), None);
    }
}
