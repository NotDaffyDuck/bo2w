//! Depth layers. The first-person gun and arms draw into the nearest sliver of the
//! reversed depth buffer, so a wall the player stands against never cuts into them.

use render_frame::{RENDER_EFFECT_DEPTH_HACK, HeldDrawKind, viewmodel_fx_flags};

/// Share of the depth range kept for the gun layer.
pub const GUN_DEPTH_SHARE: f32 = 1.0 / 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthLayer {
    World,
    Gun,
}

/// Does the model entity behind this draw key carry the view-model depth flag?
fn entity_wants_gun_depth(key: u64) -> bool {
    let entity = dpvs_t6::SortedDraw { packed: key }.object_tag();
    viewmodel_fx_flags(entity) & RENDER_EFFECT_DEPTH_HACK != 0
}

impl DepthLayer {
    /// The viewport's (min, max) depth for this layer. Depth is reversed (near is
    /// 1.0), so the gun owns the top slice and the world everything below it.
    pub fn viewport_depth(self) -> (f32, f32) {
        let boundary = 1.0 - GUN_DEPTH_SHARE;
        match self {
            Self::World => (0.0, boundary),
            Self::Gun => (boundary, 1.0),
        }
    }

    /// The layer of one retained draw: a model whose entity carries the
    /// view-model depth flag, or a code mesh built for the view model.
    pub fn of_draw(kind: &HeldDrawKind, key: u64) -> Self {
        let for_gun = match kind {
            HeldDrawKind::XModel { .. } => entity_wants_gun_depth(key),
            HeldDrawKind::CodeMesh { gun_view: viewmodel, .. } => *viewmodel,
            _ => false,
        };
        if for_gun { Self::Gun } else { Self::World }
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
