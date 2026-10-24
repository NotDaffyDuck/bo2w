//! Tessellation plans: the per-frame draw rows each kind of geometry hands
//! to the renderer, built by the module of that kind.

pub mod glass;
pub mod sky;
pub mod smodel;
pub mod world;
pub mod xmodel;

pub use render_fx::drawsurf::tess::{fx, mark, particle_cloud};

pub use glass::{GlassTable, T6GlassCue, T6GlassCues, T6GlassMesh};
pub use render_fx::drawsurf::tess::mark::GfxMarkSubKey;
pub use render_fx::drawsurf::tess::mark::mark_mesh_surface_samplers;
pub use render_fx::{FxCodeMeshPlan, FxParticleCloudPlan, GfxMarkMeshPlan};
pub use smodel::{PackedVertexRefusal, SmodelGpuPlan, SmodelPassMaterial, SmodelVertex};
pub use world::{WorldDrawGpuPlan, WorldPassMaterial, WorldVertex, WorldVertexRefusal};
pub use xmodel::{
    DynEntDrawPlan, FpvDrawPlan, FxModelDrawPlan, ItemDrawPlan, MissileDrawPlan,
    RemoteBodyDrawPlan, ScriptModelDrawPlan, XModelDrawPlan,
};

use std::sync::Arc;

/// How many buffers a shared row set rotates through: one being written, one
/// the renderer holds, one in flight between.
pub(crate) const SHARE_BANKS: usize = 3;

/// A rotating set of shared row buffers. The writer takes a bank nobody else
/// holds, fills it and publishes it; a bank still held by a reader is skipped.
#[derive(Clone, Debug)]
pub(crate) struct ShareBanks<T> {
    banks: [Arc<Vec<T>>; SHARE_BANKS],
    write: usize,
}

impl<T> Default for ShareBanks<T> {
    fn default() -> Self {
        Self {
            banks: std::array::from_fn(|_| Arc::default()),
            write: 0,
        }
    }
}

impl<T> ShareBanks<T> {
    /// Moves the write position to a bank nobody else holds and takes its
    /// rows (contents kept). The flag is true when every bank was held and a
    /// fresh empty one took its place.
    pub(crate) fn take_write_keep(&mut self) -> (Vec<T>, bool) {
        let free = (0..SHARE_BANKS)
            .map(|step| (self.write + step) % SHARE_BANKS)
            .find(|&at| Arc::strong_count(&self.banks[at]) == 1);
        let fresh = free.is_none();
        self.write = free.unwrap_or((self.write + 1) % SHARE_BANKS);
        if fresh {
            self.banks[self.write] = Arc::default();
        }
        let bank = Arc::get_mut(&mut self.banks[self.write]).expect("write bank is unique");
        (std::mem::take(bank), fresh)
    }

    /// Like [`Self::take_write_keep`], with the rows cleared.
    pub(crate) fn take_write(&mut self) -> Vec<T> {
        let mut rows = self.take_write_keep().0;
        rows.clear();
        rows
    }

    pub(crate) fn write_index(&self) -> usize {
        self.write
    }

    /// Gives the write bank its rows back.
    pub(crate) fn put_write(&mut self, rows: Vec<T>) {
        *Arc::get_mut(&mut self.banks[self.write]).expect("write bank unique after take_write") =
            rows;
    }

    /// The write bank, shared.
    pub(crate) fn published(&self) -> Arc<Vec<T>> {
        Arc::clone(&self.banks[self.write])
    }
}

pub(crate) fn published_rows<T>(share: &Option<Arc<Vec<T>>>) -> &[T] {
    share.as_deref().map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_bank_is_skipped_and_a_full_set_gets_a_fresh_one() {
        let mut banks = ShareBanks::<u32>::default();
        let mut rows = banks.take_write();
        rows.push(7);
        banks.put_write(rows);
        let held = banks.published();
        let first = banks.write_index();

        let (_, fresh) = banks.take_write_keep();
        assert!(!fresh);
        assert_ne!(banks.write_index(), first);
        banks.put_write(Vec::new());

        // Hold every bank: the next write gets a fresh one.
        let mut held_all = vec![held];
        for _ in 1..SHARE_BANKS {
            let rows = banks.take_write();
            banks.put_write(rows);
            held_all.push(banks.published());
        }
        let (rows, fresh) = banks.take_write_keep();
        assert!(fresh && rows.is_empty());
        assert_eq!(held_all[0].as_slice(), &[7]);
    }
}
