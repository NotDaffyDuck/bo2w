//! A retained draw with its sort data, plus the viewmodel flag helper.

use render_material::MaterialAssetId;

use super::kind::RetainedDrawKind;
use crate::vertex::SurfaceSamplerInputs;

#[derive(Clone, Copy, Debug)]
pub struct RetainedDrawItem {
    pub material_id: Option<MaterialAssetId>,
    pub key: u64,
    pub material_rank: u32,
    pub kind: RetainedDrawKind,
    pub surface_samplers: SurfaceSamplerInputs,
    pub camera_region: Option<u8>,
}

impl RetainedDrawItem {
    /// The order the host draws in: the sort bucket from the top of the key, then
    /// the material rank, then the low half of the key.
    #[must_use]
    pub const fn host_sort_key(&self) -> (u32, u32, u32) {
        let bucket = (self.key >> 44) as u32;
        let tie = (self.key & 0xffff_ffff) as u32;
        (bucket, self.material_rank, tie)
    }
}

/// Object id the viewmodel is registered under.
pub const XMODEL_OBJECT_ID_VIEWMODEL: u16 = 1;

/// Render-fx flag that keeps a model from poking through the world.
pub const RENDER_FX_DEPTH_HACK: u32 = 2;

/// Render-fx flags a dynamic model gets from its object id alone.
#[must_use]
pub const fn host_viewmodel_render_fx_flags(object_id: u16) -> u32 {
    match object_id {
        XMODEL_OBJECT_ID_VIEWMODEL => RENDER_FX_DEPTH_HACK,
        _ => 0,
    }
}
