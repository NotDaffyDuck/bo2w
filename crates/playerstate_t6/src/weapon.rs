//! BO2's `Weapon`: a weapon and its attachments packed in one 32-bit word.
//!
//! Bits 0..8 are the weapon's index, then three 6-bit attachment slots
//! (8..14, 14..20, 20..26); the top 6 bits are unused.

/// BO2's `Weapon` (`weaponData`). `Weapon::NONE` is no weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Weapon(pub u32);

impl Weapon {
    pub const NONE: Self = Self(0);

    /// A weapon with up to three attachments (each 0..64).
    #[must_use]
    pub const fn new(weapon_idx: u8, attachment1: u8, attachment2: u8, attachment3: u8) -> Self {
        Self(
            weapon_idx as u32
                | (attachment1 as u32 & 0x3f) << 8
                | (attachment2 as u32 & 0x3f) << 14
                | (attachment3 as u32 & 0x3f) << 20,
        )
    }

    #[must_use]
    pub const fn weapon_idx(self) -> u8 {
        (self.0 & 0xff) as u8
    }

    #[must_use]
    pub const fn attachment1(self) -> u8 {
        (self.0 >> 8 & 0x3f) as u8
    }

    #[must_use]
    pub const fn attachment2(self) -> u8 {
        (self.0 >> 14 & 0x3f) as u8
    }

    #[must_use]
    pub const fn attachment3(self) -> u8 {
        (self.0 >> 20 & 0x3f) as u8
    }

    #[must_use]
    pub const fn is_none(self) -> bool {
        self.weapon_idx() == 0
    }
}

/// BO2's `renderOptions_s`: how a weapon draws (camo, reticle, lens and so
/// on), one packed word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RenderOptions(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_like_bo2() {
        let w = Weapon::new(0x2a, 3, 17, 63);
        assert_eq!(w.0, 0x2a | 3 << 8 | 17 << 14 | 63 << 20);
        assert_eq!(
            (w.weapon_idx(), w.attachment1(), w.attachment2(), w.attachment3()),
            (0x2a, 3, 17, 63)
        );
        assert!(Weapon::NONE.is_none());
        assert!(!w.is_none());
    }
}
