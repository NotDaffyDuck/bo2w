//! Lookup data a material samples that is produced outside the material itself.

use render_material::LiveImageId;

/// The sky-visibility lookup: an image plus the sixteen constants that map a
/// world position into it.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenAirProbe {
    /// The image to sample, once it has been uploaded.
    pub image: Option<LiveImageId>,
    /// Constants mapping world space onto the image, as raw float bits.
    pub lookup: [u32; 16],
}

/// What a spot light's receivers need to sample its shadow map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeShadowSink {
    /// Row-major matrix taking a world position into the shadow map.
    pub lookup: [f32; 16],
    /// Offsets that line texel centres up with pixel centres.
    pub texel_offset_correction: [f32; 4],
}
