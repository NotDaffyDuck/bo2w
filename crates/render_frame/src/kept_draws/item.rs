//! A retained draw with its sort data, plus the viewmodel flag helper.

use render_material::MaterialAssetKey;

use super::kind::HeldDrawKind;
use crate::vertex::FacetSamplerInputs;

#[derive(Clone, Copy, Debug)]
pub struct HeldDrawItem {
    pub material_key: Option<MaterialAssetKey>,
    pub key: u64,
    pub material_order_no: u32,
    pub kind: HeldDrawKind,
    pub facet_samplers: FacetSamplerInputs,
    pub camera_zone: Option<u8>,
}

impl HeldDrawItem {
    /// The order the host draws in: the sort bucket from the top of the key, then
    /// the material rank, then the low half of the key.
    #[must_use]
    pub const fn submission_order_key(&self) -> (u32, u32, u32) {
        let bucket = (self.key >> 44) as u32;
        let tie = (self.key & 0xffff_ffff) as u32;
        (bucket, self.material_order_no, tie)
    }
}

/// Object id the viewmodel is registered under.
pub const MODEL_OBJECT_ID_FIRST_PERSON_WEAPON: u16 = 1;

/// Render-fx flag that keeps a model from poking through the world.
pub const RENDER_EFFECT_DEPTH_HACK: u32 = 2;

/// Render-fx flags a dynamic model gets from its object id alone.
#[must_use]
pub const fn viewmodel_fx_flags(object_tag: u16) -> u32 {
    match object_tag {
        MODEL_OBJECT_ID_FIRST_PERSON_WEAPON => RENDER_EFFECT_DEPTH_HACK,
        _ => 0,
    }
}
