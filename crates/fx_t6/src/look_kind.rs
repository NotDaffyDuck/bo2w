//! BO2's 13 elem types and the three classes elems are kept in.

pub const FX_ELEM_TYPE_COUNT: u8 = 13;

pub const FX_ELEM_TYPE_LAST_SPRITE: u8 = 5;

pub const FX_ELEM_TYPE_LAST_DRAWN: u8 = 9;

pub const FX_ELEM_CLASS_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ParticleShape {
    FacesCamera = 0,
    FaceAligned = 1,
    Rotated = 2,
    Tail = 3,
    Line = 4,
    Trail = 5,
    Cloud = 6,
    Model = 7,
    RoundGlow = 8,
    SpotLight = 9_u8,
    Sound = 10,
    Decal = 11,
    ScenarioDriver = 12,
}

impl ParticleShape {
    pub const ALL: [ParticleShape; FX_ELEM_TYPE_COUNT as usize] = [
        ParticleShape::FacesCamera,
        ParticleShape::FaceAligned,
        ParticleShape::Rotated,
        ParticleShape::Tail,
        ParticleShape::Line,
        ParticleShape::Trail,
        ParticleShape::Cloud,
        ParticleShape::Model,
        ParticleShape::RoundGlow,
        ParticleShape::SpotLight,
        ParticleShape::Sound,
        ParticleShape::Decal,
        ParticleShape::ScenarioDriver,
    ];

    #[inline]
    pub fn from_u8(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }

    #[inline]
    pub fn class(self) -> FxElemClass {
        match self as u8 {
            0..=FX_ELEM_TYPE_LAST_SPRITE => FxElemClass::Sprite,
            6 => FxElemClass::Cloud,
            _ => FxElemClass::NonSprite,
        }
    }

    #[inline]
    pub fn is_sprite(self) -> bool {
        self as u8 <= FX_ELEM_TYPE_LAST_SPRITE
    }

    /// Sound, decal and runner elems are handled at spawn and never drawn.
    #[inline]
    pub fn is_drawn(self) -> bool {
        self as u8 <= FX_ELEM_TYPE_LAST_DRAWN
    }

    /// Decals go to the marks system, sounds play, runners spawn an effect;
    /// none of them takes an elem from the pool.
    #[inline]
    pub fn allocates_elem(self) -> bool {
        !matches!(
            self,
            ParticleShape::Sound | ParticleShape::Decal | ParticleShape::ScenarioDriver
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FxElemClass {
    Sprite = 0,
    NonSprite = 1,
    Cloud = 2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_round_trip() {
        for raw in 0..FX_ELEM_TYPE_COUNT {
            assert_eq!(ParticleShape::from_u8(raw).map(|t| t as u8), Some(raw));
        }
        assert_eq!(ParticleShape::from_u8(13), None);
    }

    #[test]
    fn classes() {
        for t in ParticleShape::ALL {
            let want = match t as u8 {
                0..=5 => FxElemClass::Sprite,
                6 => FxElemClass::Cloud,
                _ => FxElemClass::NonSprite,
            };
            assert_eq!(t.class(), want, "{t:?}");
        }
        assert!(ParticleShape::Trail.is_sprite());
        assert!(!ParticleShape::Cloud.is_sprite());
    }

    #[test]
    fn drawn_and_pooled() {
        assert!(ParticleShape::SpotLight.is_drawn());
        assert!(!ParticleShape::Sound.is_drawn());
        assert!(ParticleShape::Model.allocates_elem());
        assert!(!ParticleShape::ScenarioDriver.allocates_elem());
        assert!(!ParticleShape::Decal.allocates_elem());
    }
}
