//! What a trace hit: BO2's `TraceHitType`, the entity numbers and the
//! `Trace_*HitId` helpers.

use crate::Trace;

/// The game's entity slots (`entity_t6::MAX_GENTITIES`; BO2 has 1024).
pub const MAX_GENTITIES: u16 = entity_t6::MAX_GENTITIES as u16;

/// The world: what a trace that hit a dynamic entity, glass or a static model
/// reports as its entity (`Trace_GetEntityHitId`).
pub const ENTITYNUM_WORLD: u16 = MAX_GENTITIES - 2;

/// No entity.
pub const ENTITYNUM_NONE: u16 = MAX_GENTITIES - 1;

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

/// BO2's `DynEntityDrawType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DynEntDrawType {
    Model = 0,
    Brush = 1,
}

/// `Trace_SetHitId`.
pub fn trace_set_hit_id(trace: &mut Trace, hit_type: TraceHitType, hit_id: u16) {
    trace.hit_type = hit_type;
    trace.hit_id = hit_id;
}

/// `Trace_GetEntityHitId`: the entity a trace hit. Dynamic entities and glass
/// count as the world; a trace that hit nothing gives `ENTITYNUM_NONE`.
#[must_use]
pub const fn trace_get_entity_hit_id(trace: &Trace) -> u16 {
    match trace.hit_type {
        TraceHitType::DynEntModel | TraceHitType::DynEntBrush | TraceHitType::Glass => {
            ENTITYNUM_WORLD
        }
        TraceHitType::Entity => trace.hit_id,
        TraceHitType::None => ENTITYNUM_NONE,
    }
}

/// `Trace_GetDynEntHitId`: the dynamic entity a trace hit and how it draws,
/// or `None` (BO2 returns 0xffff and leaves the draw type alone).
#[must_use]
pub const fn trace_get_dyn_ent_hit_id(trace: &Trace) -> Option<(u16, DynEntDrawType)> {
    match trace.hit_type {
        TraceHitType::DynEntModel => Some((trace.hit_id, DynEntDrawType::Model)),
        TraceHitType::DynEntBrush => Some((trace.hit_id, DynEntDrawType::Brush)),
        _ => None,
    }
}

/// `Trace_GetGlassHitId`: the glass piece a trace hit, or `None` (BO2
/// returns -1).
#[must_use]
pub const fn trace_get_glass_hit_id(trace: &Trace) -> Option<u16> {
    match trace.hit_type {
        TraceHitType::Glass => Some(trace.hit_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(hit_type: TraceHitType, hit_id: u16) -> Trace {
        let mut t = Trace::new();
        trace_set_hit_id(&mut t, hit_type, hit_id);
        t
    }

    #[test]
    fn entity_numbers_are_bo2s() {
        assert_eq!(ENTITYNUM_WORLD, MAX_GENTITIES - 2);
        assert_eq!(ENTITYNUM_NONE, MAX_GENTITIES - 1);
    }

    #[test]
    fn entity_hit_id_per_hit_type() {
        assert_eq!(trace_get_entity_hit_id(&hit(TraceHitType::None, 7)), ENTITYNUM_NONE);
        assert_eq!(trace_get_entity_hit_id(&hit(TraceHitType::Entity, 7)), 7);
        for t in [TraceHitType::DynEntModel, TraceHitType::DynEntBrush, TraceHitType::Glass] {
            assert_eq!(trace_get_entity_hit_id(&hit(t, 7)), ENTITYNUM_WORLD);
        }
    }

    #[test]
    fn dyn_ent_and_glass_ids() {
        assert_eq!(
            trace_get_dyn_ent_hit_id(&hit(TraceHitType::DynEntBrush, 3)),
            Some((3, DynEntDrawType::Brush))
        );
        assert_eq!(trace_get_dyn_ent_hit_id(&hit(TraceHitType::Glass, 3)), None);
        assert_eq!(trace_get_glass_hit_id(&hit(TraceHitType::Glass, 9)), Some(9));
        assert_eq!(trace_get_glass_hit_id(&hit(TraceHitType::Entity, 9)), None);
    }
}
