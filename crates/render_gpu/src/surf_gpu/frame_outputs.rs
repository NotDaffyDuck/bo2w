use std::sync::Arc;

use render_frame::FrameOutputsSnapshot;

/// The frame's products as the extract step hands them to the render world.
#[derive(Clone, Debug)]
pub struct ExtractedRenderFrameOutputs(pub Arc<FrameOutputsSnapshot>);

impl Default for ExtractedRenderFrameOutputs {
    fn default() -> Self {
        let nothing = FrameOutputsSnapshot::empty();
        Self(Arc::new(nothing))
    }
}
