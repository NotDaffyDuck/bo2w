//! The authored parameters. `*_dot` values are cosines between the view
//! direction and the direction to the sun; times are in milliseconds.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunEffectsDef {
    pub sprite_material: Option<u32>,
    pub flare_material: Option<u32>,
    pub sprite_image: Option<u32>,
    pub flare_image: Option<u32>,
    pub sprite_size: f32,
    pub flare_min_size: f32,
    pub flare_min_dot: f32,
    pub flare_max_size: f32,
    pub flare_max_dot: f32,
    pub flare_max_alpha: f32,
    pub flare_fade_in_ms: i32,
    pub flare_fade_out_ms: i32,
    pub blind_min_dot: f32,
    pub blind_max_dot: f32,
    pub blind_max_darken: f32,
    pub blind_fade_in_ms: i32,
    pub blind_fade_out_ms: i32,
    pub glare_min_dot: f32,
    pub glare_max_dot: f32,
    pub glare_max_lighten: f32,
    pub glare_fade_in_ms: i32,
    pub glare_fade_out_ms: i32,
    /// Unit vector toward the sun.
    pub direction: [f32; 3],
}
