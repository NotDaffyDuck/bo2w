//! Change counters. Consumers compare these instead of the data they describe.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// One counter per kind of source data. A counter only ever moves forward
/// (wrapping), so "equal to last frame" means "nothing of that kind changed".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InputChangeCounters {
    pub topology: u64,
    pub vertices: u64,
    pub materials: u64,
    pub draws: u64,
    pub admission: u64,
}

fn step(counter: &mut u64) {
    *counter = counter.wrapping_add(1);
}

impl InputChangeCounters {
    /// Topology is a content hash rather than a counter: re-decoding identical
    /// geometry must leave it where it was.
    pub fn stamp_topology(&mut self, index_list: &[u32], spans: &[(u32, u32)], decoded_tally: usize) {
        let mut state = DefaultHasher::new();
        decoded_tally.hash(&mut state);
        index_list.hash(&mut state);
        spans.hash(&mut state);
        self.topology = state.finish();
    }

    pub fn advance_vertexdata_stamp(&mut self) {
        step(&mut self.vertices);
    }

    pub fn bump_materials(&mut self) {
        step(&mut self.materials);
    }

    /// A surface edit touches both its vertices and its material binding.
    pub fn bump_surfaces(&mut self) {
        self.advance_vertexdata_stamp();
        self.bump_materials();
    }

    pub fn advance_drawlist_stamp(&mut self) {
        step(&mut self.draws);
    }

    pub fn advance_acceptance_stamp(&mut self) {
        step(&mut self.admission);
    }

    /// A write into the packed buffers: surfaces and draws move, and so does the
    /// buffer's own revision, which the caller owns.
    pub fn note_packed_data_rewritten(&mut self, buf_stamp: &mut u64) {
        self.bump_surfaces();
        self.advance_drawlist_stamp();
        step(buf_stamp);
    }
}

/// Publishes a rebuilt row list in place of the previous one and reports whether
/// the rows differ.
///
/// The comparison is made here, once, by the list's owner, so downstream code
/// can trust [`InputChangeCounters`] and never has to diff rows itself. Either way
/// `rebuilt` is returned empty with its allocation kept, ready for the next
/// rebuild.
pub fn publish_rows<T: PartialEq>(published: &mut Vec<T>, rebuilt: &mut Vec<T>) -> bool {
    let changed = *published != *rebuilt;
    if changed {
        std::mem::swap(published, rebuilt);
    }
    rebuilt.clear();
    changed
}
