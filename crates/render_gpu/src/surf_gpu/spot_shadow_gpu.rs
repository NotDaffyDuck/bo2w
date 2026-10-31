use bevy::render::render_resource::TextureFormat;

/// Debug label of BO2's single spot-shadow target (slot 0x10), a 2x2 grid of tiles.
pub const SHADOWMAP_SPOT_LABEL: &str = "t6_shadowmap_spot";

// BO2 keeps this target as one D32 surface read through the compare sampler (0x65). The R32F
// colour copy and the depth format below stay until the spot pass is moved over to that.
pub const SHADOW_MAP_CONE_COLOR_FORMAT: TextureFormat = TextureFormat::R32Float;
pub const SPOT_SHADOW_DEPTH_FMT: TextureFormat = TextureFormat::Depth24PlusStencil8;
