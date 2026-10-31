//! The clip volume a cascade gathers casters from.

/// Room for the two arc sides, the four sides of the cascade's own rectangle
/// and, on the far cascade, the seam against the near one.
pub const SUN_SHADOW_MAX_CLIP_PLANES: usize = 10;

/// World-space planes `[a, b, c, d]`; a point is inside the volume when it is on
/// the kept side of every plane.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowClipPlanes {
    planes: [[f32; 4]; SUN_SHADOW_MAX_CLIP_PLANES],
    count: u8,
}

impl SunShadowClipPlanes {
    pub const BLANK: Self = Self { planes: [[0.0; 4]; SUN_SHADOW_MAX_CLIP_PLANES], count: 0 };

    /// Adds a plane and says whether it fit. A refused plane is simply lost:
    /// the volume gets looser, which costs draw calls but never drops a caster.
    /// Taking an eleventh would need a bigger array here and in the visibility
    /// code that consumes it.
    pub fn push(&mut self, plane: [f32; 4]) -> bool {
        let Some(slot) = self.planes.get_mut(usize::from(self.count)) else {
            return false;
        };
        *slot = plane;
        self.count += 1;
        true
    }

    #[must_use]
    pub fn as_slice(&self) -> &[[f32; 4]] {
        self.planes.split_at(usize::from(self.count)).0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.count)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}
