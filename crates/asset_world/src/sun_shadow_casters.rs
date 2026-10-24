//! Which of a world's surfaces cast the sun's shadow.

/// One bit per surface, set when that surface casts the sun's shadow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCastsSunShadow {
    bits: Vec<u32>,
    surfaces: usize,
}

impl SurfaceCastsSunShadow {
    /// A set for `surfaces` surfaces, none of them casting.
    pub fn with_len(surfaces: usize) -> Self {
        Self {
            bits: vec![0; surfaces.div_ceil(32)],
            surfaces,
        }
    }

    /// How many surfaces the set covers.
    pub fn len(&self) -> usize {
        self.surfaces
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces == 0
    }

    /// Whether `surface` casts. A surface past the end never does.
    pub fn get(&self, surface: usize) -> bool {
        surface < self.surfaces && (self.bits[surface / 32] >> (surface % 32)) & 1 != 0
    }

    /// Marks `surface` as casting. A surface past the end is ignored.
    pub fn set(&mut self, surface: usize) {
        if surface < self.surfaces {
            self.bits[surface / 32] |= 1 << (surface % 32);
        }
    }

    /// The bits as 32-bit words: surface `n` is bit `n % 32` of word `n / 32`.
    pub fn words(&self) -> &[u32] {
        &self.bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_land_in_the_word_that_holds_them() {
        let mut casters = SurfaceCastsSunShadow::with_len(70);
        casters.set(0);
        casters.set(33);
        casters.set(69);
        assert_eq!(casters.words(), [1, 2, 1 << 5]);
        assert!(casters.get(33) && !casters.get(34));
    }

    #[test]
    fn surfaces_past_the_end_are_ignored() {
        let mut casters = SurfaceCastsSunShadow::with_len(3);
        casters.set(3);
        casters.set(1000);
        assert!(!casters.get(3) && !casters.get(1000));
        assert_eq!(casters.words(), [0]);
        assert_eq!(casters.len(), 3);
        assert!(SurfaceCastsSunShadow::default().is_empty());
    }
}
