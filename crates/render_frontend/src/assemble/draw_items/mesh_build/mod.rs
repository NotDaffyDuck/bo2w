//! Tessellation plans: the per-frame draw rows each kind of geometry hands
//! to the renderer, built by the module of that kind.

pub mod glass_piece;
pub mod sky;
pub mod prop_mesh;
pub mod world;
pub mod figure_mesh;

pub use render_fx::surface_build::mesh_fill::{fx, stain, cloud_lattice};
// Old paths kept for the render crate until the rename sweep (RENAMES.txt).
/// Old path of `stain`.
pub mod mark {
    pub use render_fx::surface_build::mesh_fill::stain::VisualStainMeshPlan;
}

/// Old path of `cloud_lattice`.
pub mod particle_cloud {
    pub use render_fx::surface_build::mesh_fill::cloud_lattice::FxCloudDrawPlan;
}

pub use glass_piece::{GlassTable, T6GlassCue, T6GlassCues, T6GlassMesh};
pub use render_fx::surface_build::mesh_fill::stain::DecalSubmeshKey;
pub use render_fx::surface_build::mesh_fill::stain::decal_mesh_surface_samplers;
pub use render_fx::{FxCodeDrawPlan, FxCloudDrawPlan, VisualStainMeshPlan};
pub use self::prop_mesh::{PackedVertexRefusal, PropGpuPlan, PropPassMaterial, PropVertex};
pub use render_frame::{LevelVertex, WorldVertexRefusal};
pub use world::{LevelPassMaterial, WorldGpuSubmission};
pub use self::figure_mesh::DynEntPaintPlan;
pub use self::figure_mesh::FirstPersonPaintPlan;
pub use self::figure_mesh::FxModelDrawList;
pub use self::figure_mesh::PickupPaintPlan;
pub use self::figure_mesh::ThrownPaintPlan;
pub use self::figure_mesh::OtherBodyPaintPlan;
pub use self::figure_mesh::ScriptedModelPaintPlan;
pub use self::figure_mesh::FigureDrawPlan;

use std::sync::Arc;

/// How many buffers a shared row set rotates through: one being written, one
/// the renderer holds, one in flight between.
pub(crate) const PORTION_BANKS: usize = 3;

/// A rotating set of shared row buffers. The writer takes a bank nobody else
/// holds, fills it and publishes it; a bank still held by a reader is skipped.
#[derive(Clone, Debug)]
pub(crate) struct PortionBanks<T> {
    bank_list: [Arc<Vec<T>>; PORTION_BANKS],
    write: usize,
}

impl<T> Default for PortionBanks<T> {
    fn default() -> Self {
        Self {
            bank_list: std::array::from_fn(|_| Arc::default()),
            write: 0,
        }
    }
}

impl<T> PortionBanks<T> {
    /// Moves the write position to a bank nobody else holds and takes its
    /// rows (contents kept). The flag is true when every bank was held and a
    /// fresh empty one took its place.
    pub(crate) fn claim_write_keep(&mut self) -> (Vec<T>, bool) {
        let free = (0..PORTION_BANKS)
            .map(|step| (self.write + step) % PORTION_BANKS)
            .find(|&at| Arc::strong_count(&self.bank_list[at]) == 1);
        let fresh = free.is_none();
        self.write = free.unwrap_or((self.write + 1) % PORTION_BANKS);
        if fresh {
            self.bank_list[self.write] = Arc::default();
        }
        let bank = Arc::get_mut(&mut self.bank_list[self.write]).expect("write bank is unique");
        (std::mem::take(bank), fresh)
    }

    /// Like [`Self::take_write_keep`], with the rows cleared.
    pub(crate) fn claim_write(&mut self) -> Vec<T> {
        let mut row_list = self.claim_write_keep().0;
        row_list.clear();
        row_list
    }

    pub(crate) fn write_at(&self) -> usize {
        self.write
    }

    /// Gives the write bank its rows back.
    pub(crate) fn store_write(&mut self, row_list: Vec<T>) {
        *Arc::get_mut(&mut self.bank_list[self.write]).expect("write bank unique after take_write") =
            row_list;
    }

    /// The write bank, shared.
    pub(crate) fn posted(&self) -> Arc<Vec<T>> {
        Arc::clone(&self.bank_list[self.write])
    }
}

pub(crate) fn published_rows<T>(share: &Option<Arc<Vec<T>>>) -> &[T] {
    share.as_deref().map(|list| list.as_slice()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_bank_is_skipped_and_a_full_set_gets_a_fresh_one() {
        let mut banks = PortionBanks::<u32>::default();
        let mut rows = banks.claim_write();
        rows.push(7);
        banks.store_write(rows);
        let held = banks.posted();
        let first = banks.write_at();

        let (_, fresh) = banks.claim_write_keep();
        assert!(!fresh);
        assert_ne!(banks.write_at(), first);
        banks.store_write(Vec::new());

        // Hold every bank: the next write gets a fresh one.
        let mut held_all = vec![held];
        for _ in 1..PORTION_BANKS {
            let rows = banks.claim_write();
            banks.store_write(rows);
            held_all.push(banks.posted());
        }
        let (rows, fresh) = banks.claim_write_keep();
        assert!(fresh && rows.is_empty());
        assert_eq!(held_all[0].as_slice(), &[7]);
    }
}
