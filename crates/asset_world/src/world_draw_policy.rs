//! How a world's surfaces are drawn.

/// The drawing rules that depend on which game the world came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldDrawPolicy {
    /// A surface with a lightmap is drawn only once the lightmap image has
    /// loaded.
    pub lightmap_requires_image: bool,
}

impl WorldDrawPolicy {
    /// The rules for a Black Ops II world.
    pub const fn t6() -> Self {
        Self {
            lightmap_requires_image: true,
        }
    }
}
