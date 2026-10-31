//! Tessellation plans shared by the effect draw passes.

use std::sync::Arc;

pub mod fx; // code-generated sprite, trail and post-light meshes
pub mod stain; // decal-style mark meshes
pub mod cloud_lattice; // spark and smoke lattice clouds

pub use self::fx::FxCodeDrawPlan;
pub use self::stain::VisualStainMeshPlan;
pub use self::cloud_lattice::FxCloudDrawPlan;

/// Empties a shared row buffer. If another holder still references it, a fresh buffer
/// replaces it rather than cloning data that is about to be thrown away.
pub(crate) fn recycle_rows<T>(rows: &mut Arc<Vec<T>>) {
    match Arc::get_mut(rows) {
        Some(sole_owner) => sole_owner.clear(),
        None => *rows = Arc::new(Vec::new()),
    }
}

/// Collects `(first_index, num_indices)` pairs into one shareable list.
pub(crate) fn collect_index_ranges<I: IntoIterator<Item = (u32, u32)>>(
    spans: I,
) -> Arc<Vec<(u32, u32)>> {
    Arc::new(spans.into_iter().collect())
}
