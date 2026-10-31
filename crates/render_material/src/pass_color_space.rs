use asset_core::AssetCategoryScope;
use lighting_t6::slot_is_lit_substitute;

/// How a pass encodes the colors it writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PassColorEncoding {
    #[default]
    Linear,
    NonlinearEncoded,
    Unknown,
}

pub fn pass_color_space(scope: AssetCategoryScope, tech_slot: u8) -> PassColorEncoding {
    match scope {
        AssetCategoryScope::T5 if slot_is_lit_substitute(tech_slot) => PassColorEncoding::NonlinearEncoded,
        AssetCategoryScope::T5 => PassColorEncoding::Unknown,
        // bo2zm: Black Ops II passes are not measured yet.
        AssetCategoryScope::T6 => PassColorEncoding::Unknown,
    }
}
