//! The element types the runtime tells apart, and which of them have a draw
//! routine.

/// An element's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ParticleShape {
    FacesCamera = 0,
    FaceAligned = 1,
    Tail = 2,
    Trail = 3,
    Cloud = 4,
    Model = 7,
    RoundGlow = 8,
    SpotLight = 9_u8,
    Sound = 10,
    Decal = 11,
    ScenarioDriver = 12,
}

impl ParticleShape {
    const ALL: [Self; 11] = [
        Self::FacesCamera,
        Self::FaceAligned,
        Self::Tail,
        Self::Trail,
        Self::Cloud,
        ParticleShape::Model,
        Self::RoundGlow,
        Self::SpotLight,
        Self::Sound,
        Self::Decal,
        Self::ScenarioDriver,
    ];

    /// One bit per type byte, set for the types in `kinds`.
    const fn bits(kinds: &[Self]) -> u16 {
        let mut mask = 0u16;
        let mut at = 0;
        while at < kinds.len() {
            mask |= 1 << kinds[at] as u8;
            at += 1;
        }
        mask
    }

    /// The type for a stored byte, or `None` for a byte no type uses.
    pub const fn from_u8(val: u8) -> Option<Self> {
        let mut at = 0;
        while at < Self::ALL.len() {
            if Self::ALL[at] as u8 == val {
                return Some(Self::ALL[at]);
            }
            at += 1;
        }
        None
    }

    /// Whether elements of this type have nothing to draw.
    pub const fn particle_painter_absent(self) -> bool {
        const NULL: u16 = ParticleShape::bits(&[
            ParticleShape::Trail,
            ParticleShape::Sound,
            ParticleShape::Decal,
            ParticleShape::ScenarioDriver,
        ]);
        NULL >> self as u8 & 1 == 1
    }

    /// Whether spawning an element of this type does its work at once and
    /// leaves no element behind.
    pub const fn birth_particle_bail(self) -> bool {
        const EARLY: u16 =
            ParticleShape::bits(&[ParticleShape::Sound, ParticleShape::Decal, ParticleShape::ScenarioDriver]);
        EARLY >> self as u8 & 1 == 1
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
            if let Some(ty) = ParticleShape::from_u8(byte) {
                assert_eq!(ty as u8, byte);
            }
        }
        assert_eq!(ParticleShape::from_u8(5), None);
        assert_eq!(ParticleShape::from_u8(6), None);
        assert_eq!(ParticleShape::from_u8(13), None);
    }

    #[test]
    fn the_null_draw_types_match_the_draw_table() {
        for byte in 0..=u8::MAX {
            if let Some(ty) = ParticleShape::from_u8(byte) {
                if byte < 11 {
                    assert_eq!(
                        ty.particle_painter_absent(),
                        !draw_elem_handler_present(byte),
                        "{ty:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn spawn_gives_up_early_for_sound_decal_and_runner() {
        assert!(ParticleShape::Sound.birth_particle_bail());
        assert!(ParticleShape::Decal.birth_particle_bail());
        assert!(ParticleShape::ScenarioDriver.birth_particle_bail());
        assert!(!ParticleShape::Trail.birth_particle_bail());
        assert!(!ParticleShape::FacesCamera.birth_particle_bail());
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
