//! Whether a surface takes a mark. BO2 asks the same of world brushes, entity brushes,
//! models and glass.

/// Decals sort with this key, and a mark never lands on a decal.
const DECAL_SORT_KEY: u8 = 0x18;
/// The material game flag that says "no marks here".
const NO_MARKS_FLAG: u8 = 0x4;

/// What the mark test reads from the material of the surface that was hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkReceiver {
    pub sort_key: u8,
    /// The low byte of the material's game flags.
    pub game_flags: u8,
    /// The surface kinds the material counts as (metal, wood, ...), `None` when unknown.
    pub surface_type_bits: Option<u32>,
}

/// The answer of [`mark_allow`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxMarkAllow {
    Keep,
    Reject,
    /// Something the test needs was not known.
    Unknown,
}

/// A decal or a no-marks material refuses the mark. Otherwise the surface takes it when its
/// material counts as every surface kind the mark's material names.
pub fn mark_allow(
    receiver: Option<MarkReceiver>,
    mark_surface_type_bits: Option<u32>,
) -> FxMarkAllow {
    let Some(receiver) = receiver else {
        return FxMarkAllow::Unknown;
    };
    if receiver.sort_key == DECAL_SORT_KEY || receiver.game_flags & NO_MARKS_FLAG != 0 {
        return FxMarkAllow::Reject;
    }
    match (receiver.surface_type_bits, mark_surface_type_bits) {
        (Some(has), Some(wants)) if has & wants == wants => FxMarkAllow::Keep,
        (Some(_), Some(_)) => FxMarkAllow::Reject,
        _ => FxMarkAllow::Unknown,
    }
}

/// Only a surface that surely takes the mark is cut for it.
pub fn mark_include_in_world_clip(allow: FxMarkAllow) -> bool {
    allow == FxMarkAllow::Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(bits: Option<u32>) -> MarkReceiver {
        MarkReceiver {
            sort_key: 2,
            game_flags: 0,
            surface_type_bits: bits,
        }
    }

    #[test]
    fn surface_kinds_must_cover_the_mark() {
        assert_eq!(
            mark_allow(Some(wall(Some(0b1110))), Some(0b0110)),
            FxMarkAllow::Keep
        );
        assert_eq!(
            mark_allow(Some(wall(Some(0b1110))), Some(0b0001)),
            FxMarkAllow::Reject
        );
        assert_eq!(mark_allow(Some(wall(Some(0))), Some(0)), FxMarkAllow::Keep);
    }

    #[test]
    fn decals_and_no_marks_refuse_even_when_kinds_are_unknown() {
        let decal = MarkReceiver {
            sort_key: DECAL_SORT_KEY,
            ..wall(None)
        };
        let no_marks = MarkReceiver {
            game_flags: NO_MARKS_FLAG | 1,
            ..wall(Some(!0))
        };
        assert_eq!(mark_allow(Some(decal), None), FxMarkAllow::Reject);
        assert_eq!(mark_allow(Some(no_marks), Some(1)), FxMarkAllow::Reject);
    }

    #[test]
    fn missing_facts_are_unknown_and_not_cut() {
        assert_eq!(mark_allow(None, Some(1)), FxMarkAllow::Unknown);
        assert_eq!(mark_allow(Some(wall(None)), Some(1)), FxMarkAllow::Unknown);
        assert_eq!(mark_allow(Some(wall(Some(1))), None), FxMarkAllow::Unknown);
        assert!(mark_include_in_world_clip(FxMarkAllow::Keep));
        assert!(!mark_include_in_world_clip(FxMarkAllow::Reject));
        assert!(!mark_include_in_world_clip(FxMarkAllow::Unknown));
    }
}
