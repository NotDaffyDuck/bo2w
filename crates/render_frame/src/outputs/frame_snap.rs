//! The frame's products gathered into one value for hand-off.

use super::status::{FrameOutputItem, FrameOutputKind};

/// What the debug focus picked (a model under inspection) and what became of it.
#[derive(Clone, Debug)]
pub struct FocusedFrameSnapshot {
    pub owner_no: u32,
    pub model: Option<String>,
    pub outcome: &'static str,
    pub object_tag: Option<u16>,
    pub hidden_from_viewer: Option<bool>,
    pub illum_handle: Option<u32>,
    pub planned_facets: u32,
}

impl FocusedFrameSnapshot {
    /// A focus that never resolved to a model: only the owner and the outcome
    /// are known.
    pub fn never_found(who_id: u32, why: &'static str) -> Self {
        Self {
            owner_no: who_id,
            outcome: why,
            model: None,
            object_tag: None,
            hidden_from_viewer: None,
            illum_handle: None,
            planned_facets: 0,
        }
    }
}

/// Every product of one frame. `products` always holds one entry per
/// [`FrameOutputKind`].
#[derive(Debug)]
pub struct FrameOutputsSnapshot {
    pub frame_seq_no: u64,
    pub focus: Option<FocusedFrameSnapshot>,
    pub products: Vec<FrameOutputItem>,
    /// Surface lists of multi-surface world runs, shared by all products.
    pub level_run_facets: Vec<u16>,
    pub level_run_edition: u64,
}

impl FrameOutputsSnapshot {
    /// A frame before anything has been produced: every product is missing.
    pub fn empty() -> Self {
        Self {
            frame_seq_no: 0,
            focus: None,
            products: FrameOutputKind::ALL_KINDS.into_iter().map(FrameOutputItem::vacant).collect(),
            level_run_facets: Vec::new(),
            level_run_edition: 0,
        }
    }

    pub fn product(&self, kind: FrameOutputKind) -> &FrameOutputItem {
        let found = self.products.iter().find(|p| p.kind == kind);
        found.expect("RenderFrameProducts must contain every declared product")
    }

    pub fn focus(&self) -> Option<&FocusedFrameSnapshot> {
        self.focus.as_ref()
    }
}
