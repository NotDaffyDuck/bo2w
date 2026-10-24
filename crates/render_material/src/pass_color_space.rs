use asset_core::AssetNamespace;
use lighting_t6::is_lit_remap_slot;

/// How a pass encodes the colors it writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PassColorSpace {
    #[default]
    Linear,
    GammaEncoded,
    Unknown,
}

pub fn pass_color_space(namespace: AssetNamespace, tech_slot: u8) -> PassColorSpace {
    match namespace {
        AssetNamespace::T5 if is_lit_remap_slot(tech_slot) => PassColorSpace::GammaEncoded,
        AssetNamespace::T5 => PassColorSpace::Unknown,
        // bo2zm: Black Ops II passes are not measured yet.
        AssetNamespace::T6 => PassColorSpace::Unknown,
    }
}
