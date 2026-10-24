//! The frame's products gathered into one value for hand-off.

use super::status::{FrameProduct, FrameProductKind};

/// What the debug focus picked (a model under inspection) and what became of it.
#[derive(Clone, Debug)]
pub struct RenderFocusFrame {
    pub owner_id: u32,
    pub model: Option<String>,
    pub outcome: &'static str,
    pub object_id: Option<u16>,
    pub camera_hidden: Option<bool>,
    pub lighting_handle: Option<u32>,
    pub planned_surfaces: u32,
}

impl RenderFocusFrame {
    /// A focus that never resolved to a model: only the owner and the outcome
    /// are known.
    pub fn unresolved(owner_id: u32, outcome: &'static str) -> Self {
        Self {
            owner_id,
            outcome,
            model: None,
            object_id: None,
            camera_hidden: None,
            lighting_handle: None,
            planned_surfaces: 0,
        }
    }
}

/// Every product of one frame. `products` always holds one entry per
/// [`FrameProductKind`].
#[derive(Debug)]
pub struct FrameProductsSnapshot {
    pub frame_id: u64,
    pub focus: Option<RenderFocusFrame>,
    pub products: Vec<FrameProduct>,
    /// Surface lists of multi-surface world runs, shared by all products.
    pub world_run_surfs: Vec<u16>,
    pub world_run_revision: u64,
}

impl FrameProductsSnapshot {
    /// A frame before anything has been produced: every product is missing.
    pub fn empty() -> Self {
        Self {
            frame_id: 0,
            focus: None,
            products: FrameProductKind::ALL.into_iter().map(FrameProduct::vacant).collect(),
            world_run_surfs: Vec::new(),
            world_run_revision: 0,
        }
    }

    pub fn product(&self, kind: FrameProductKind) -> &FrameProduct {
        let found = self.products.iter().find(|p| p.kind == kind);
        found.expect("RenderFrameProducts must contain every declared product")
    }

    pub fn focus(&self) -> Option<&RenderFocusFrame> {
        self.focus.as_ref()
    }
}
