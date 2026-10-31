//! Whether a surface takes a mark. BO2 asks the same of world brushes, entity brushes,
//! models and glass.

/// Decals sort with this key, and a mark never lands on a decal.
const DECAL_SORT_KEY: u8 = 0x18;
/// The material game flag that says "no marks here".
const NO_MARKS_FLAG: u8 = 0x4;

/// What the mark test reads from the material of the surface that was hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecalSurfaceGate {
    pub draw_rank: u8,
    /// The low byte of the material's game flags.
    pub game_flags: u8,
    /// The surface kinds the material counts as (metal, wood, ...), `None` when unknown.
    pub facet_type_bits: Option<u32>,
}

/// The answer of [`mark_allow`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalAdmit {
    Keep,
    Reject,
    /// Something the test needs was not known.
    Unknown,
}

/// A decal or a no-marks material refuses the mark. Otherwise the surface takes it when its
/// material counts as every surface kind the mark's material names.
pub fn mark_allow(
    receiver: Option<DecalSurfaceGate>,
    wanted_kinds: Option<u32>,
) -> DecalAdmit {
    let Some(receiver) = receiver else {
        return DecalAdmit::Unknown;
    };
    if receiver.draw_rank == DECAL_SORT_KEY || receiver.game_flags & NO_MARKS_FLAG != 0 {
        return DecalAdmit::Reject;
    }
    match (receiver.facet_type_bits, wanted_kinds) {
        (Some(has), Some(wants)) if has & wants == wants => DecalAdmit::Keep,
        (Some(_), Some(_)) => DecalAdmit::Reject,
        _ => DecalAdmit::Unknown,
    }
}

/// Only a surface that surely takes the mark is cut for it.
pub fn mark_include_in_world_clip(allow: DecalAdmit) -> bool {
    allow == DecalAdmit::Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(bits: Option<u32>) -> DecalSurfaceGate {
        DecalSurfaceGate {
            draw_rank: 2,
            game_flags: 0,
            facet_type_bits: bits,
        }
    }

    #[test]
    fn surface_kinds_must_cover_the_mark() {
        assert_eq!(
            mark_allow(Some(wall(Some(0b1110))), Some(0b0110)),
            DecalAdmit::Keep
        );
        assert_eq!(
            mark_allow(Some(wall(Some(0b1110))), Some(0b0001)),
            DecalAdmit::Reject
        );
        assert_eq!(mark_allow(Some(wall(Some(0))), Some(0)), DecalAdmit::Keep);
    }

    #[test]
    fn decals_and_no_marks_refuse_even_when_kinds_are_unknown() {
        let decal = DecalSurfaceGate {
            draw_rank: DECAL_SORT_KEY,
            ..wall(None)
        };
        let no_marks = DecalSurfaceGate {
            game_flags: NO_MARKS_FLAG | 1,
            ..wall(Some(!0))
        };
        assert_eq!(mark_allow(Some(decal), None), DecalAdmit::Reject);
        assert_eq!(mark_allow(Some(no_marks), Some(1)), DecalAdmit::Reject);
    }

    #[test]
    fn missing_facts_are_unknown_and_not_cut() {
        assert_eq!(mark_allow(None, Some(1)), DecalAdmit::Unknown);
        assert_eq!(mark_allow(Some(wall(None)), Some(1)), DecalAdmit::Unknown);
        assert_eq!(mark_allow(Some(wall(Some(1))), None), DecalAdmit::Unknown);
        assert!(mark_include_in_world_clip(DecalAdmit::Keep));
        assert!(!mark_include_in_world_clip(DecalAdmit::Reject));
        assert!(!mark_include_in_world_clip(DecalAdmit::Unknown));
    }
}
