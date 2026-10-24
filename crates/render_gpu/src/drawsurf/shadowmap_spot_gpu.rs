use bevy::render::render_resource::TextureFormat;

// TODO(fact): BO2's spot target is one D32 depth surface sampled with the compare sampler
// (0x65); the R32F colour copy and this depth format are kept until the spot pass is moved.
pub const SHADOWMAP_SPOT_COLOR_FORMAT: TextureFormat = TextureFormat::R32Float;

pub const SHADOWMAP_SPOT_DEPTH_FORMAT: TextureFormat = TextureFormat::Depth24PlusStencil8;

/// BO2's one spot-shadow target (slot 0x10): a 2x2 grid of tiles.
pub const SHADOWMAP_SPOT_LABEL: &str = "t6_shadowmap_spot";
pub const SHADOWMAP_SUN_LABEL: &str = "bo2_shadowmap_sun";
