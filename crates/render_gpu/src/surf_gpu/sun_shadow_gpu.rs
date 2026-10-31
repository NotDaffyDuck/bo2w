use bevy::render::render_resource::TextureFormat;

/// Debug label of the sun-shadow target.
pub const SHADOWMAP_SUN_LABEL: &str = "bo2_shadowmap_sun";

pub const SHADOW_MAP_DAYLIGHT_COLOR_FORMAT: TextureFormat = TextureFormat::R32Float;
pub const SUN_SHADOW_DEPTH_FMT: TextureFormat = TextureFormat::Depth24PlusStencil8;
