//! Depth layers. The first-person gun and arms draw into the nearest sliver of the
//! reversed depth buffer, so a wall the player stands against never cuts into them.

use render_frame::{RENDER_FX_DEPTH_HACK, RetainedDrawKind, host_viewmodel_render_fx_flags};

/// Share of the depth range kept for the gun layer.
pub const GUN_DEPTH_SHARE: f32 = 1.0 / 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthLayer {
    World,
    Gun,
}

impl DepthLayer {
    /// The viewport's (min, max) depth for this layer. Depth is reversed (near is
    /// 1.0), so the gun owns the top slice and the world everything below it.
    pub fn viewport_depth(self) -> (f32, f32) {
        let split = 1.0 - GUN_DEPTH_SHARE;
        match self {
            DepthLayer::World => (0.0, split),
            DepthLayer::Gun => (split, 1.0),
        }
    }

    /// The layer of one retained draw: a model whose entity carries the
    /// view-model depth flag, or a code mesh built for the view model.
    pub fn of_draw(kind: &RetainedDrawKind, key: u64) -> Self {
        let gun = match kind {
            RetainedDrawKind::XModel { .. } => {
                let object = dpvs_t6::GfxDrawSurf { packed: key }.object_id();
                host_viewmodel_render_fx_flags(object) & RENDER_FX_DEPTH_HACK != 0
            }
            RetainedDrawKind::CodeMesh { viewmodel, .. } => *viewmodel,
            _ => false,
        };
        if gun { DepthLayer::Gun } else { DepthLayer::World }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_split_the_range_at_one_sixty_fourth() {
        assert_eq!(DepthLayer::World.viewport_depth(), (0.0, 0.984375));
        assert_eq!(DepthLayer::Gun.viewport_depth(), (0.984375, 1.0));
    }
}
