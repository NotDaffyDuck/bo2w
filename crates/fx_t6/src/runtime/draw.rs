//! The element types the runtime tells apart, and which of them have a draw
//! routine.

/// An element's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxElemType {
    Billboard = 0,
    Oriented = 1,
    Tail = 2,
    Trail = 3,
    Cloud = 4,
    Model = 7,
    OmniLight = 8,
    SpotLight = 9,
    Sound = 10,
    Decal = 11,
    Runner = 12,
}

impl FxElemType {
    /// The type for a stored byte, or `None` for a byte no type uses.
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Billboard,
            1 => Self::Oriented,
            2 => Self::Tail,
            3 => Self::Trail,
            4 => Self::Cloud,
            7 => Self::Model,
            8 => Self::OmniLight,
            9 => Self::SpotLight,
            10 => Self::Sound,
            11 => Self::Decal,
            12 => Self::Runner,
            _ => return None,
        })
    }

    /// Whether elements of this type have nothing to draw.
    pub const fn draw_elem_handler_is_null(self) -> bool {
        matches!(self, Self::Trail | Self::Sound | Self::Decal | Self::Runner)
    }

    /// Whether spawning an element of this type does its work at once and
    /// leaves no element behind.
    pub const fn spawn_elem_early_out(self) -> bool {
        matches!(self, Self::Sound | Self::Decal | Self::Runner)
    }
}

/// Whether a stored element type byte has a draw routine. Bytes past the end
/// of the table (11 and up) have none.
pub const fn draw_elem_handler_present(elem_type: u8) -> bool {
    matches!(elem_type, 0..=2 | 4..=9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_byte_round_trips() {
        for byte in 0..=u8::MAX {
            if let Some(ty) = FxElemType::from_u8(byte) {
                assert_eq!(ty as u8, byte);
            }
        }
        assert_eq!(FxElemType::from_u8(5), None);
        assert_eq!(FxElemType::from_u8(6), None);
        assert_eq!(FxElemType::from_u8(13), None);
    }

    #[test]
    fn the_null_draw_types_match_the_draw_table() {
        for byte in 0..=u8::MAX {
            if let Some(ty) = FxElemType::from_u8(byte) {
                if byte < 11 {
                    assert_eq!(
                        ty.draw_elem_handler_is_null(),
                        !draw_elem_handler_present(byte),
                        "{ty:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn spawn_gives_up_early_for_sound_decal_and_runner() {
        assert!(FxElemType::Sound.spawn_elem_early_out());
        assert!(FxElemType::Decal.spawn_elem_early_out());
        assert!(FxElemType::Runner.spawn_elem_early_out());
        assert!(!FxElemType::Trail.spawn_elem_early_out());
        assert!(!FxElemType::Billboard.spawn_elem_early_out());
    }

    #[test]
    fn nothing_past_the_table_has_a_draw_routine() {
        assert!(!draw_elem_handler_present(11));
        assert!(!draw_elem_handler_present(12));
        assert!(!draw_elem_handler_present(255));
        assert!(draw_elem_handler_present(9));
        assert!(!draw_elem_handler_present(10));
    }
}
