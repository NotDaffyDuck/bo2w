//! What the models that carry per-surface materials have in common.

use asset_core::MaterialKey;

/// The name of the material a surface draws from, when it is bound to one.
pub(crate) fn bound_material_name<'a>(keys: &'a [Option<MaterialKey>], links: &[asset_core::AssetDependencyLink<asset_core::SurfaceShaderSpace>], at: usize) -> Option<&'a str> {
    let bound = links.get(at)?.is_attached();
    let key = keys.get(at)?.as_ref()?;
    bound.then_some(key.name.as_str())
}
