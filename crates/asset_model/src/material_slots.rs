//! What the models that carry per-surface materials have in common.

use crate::{AssetEdge, MaterialSpace};
use asset_core::MaterialKey;

/// The name of the material a surface draws from, when it is bound to one.
pub(crate) fn bound_material_name<'a>(
    keys: &'a [Option<MaterialKey>],
    edges: &[AssetEdge<MaterialSpace>],
    surface: usize,
) -> Option<&'a str> {
    if !edges.get(surface)?.is_bound() {
        return None;
    }
    Some(keys.get(surface)?.as_ref()?.name.as_str())
}
