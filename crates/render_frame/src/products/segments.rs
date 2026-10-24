//! Where each owner's rows sit inside a shared packed buffer.

/// Number of owners that share one packed buffer.
pub const PACKED_SEGMENT_OWNERS: usize = 8;

/// One owner's span of rows, with the revision of what is in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PackedSegment {
    pub start: u32,
    pub rows: u32,
    pub revision: u64,
}

/// The spans of all owners, plus a `layout` counter that moves whenever a span
/// moved or resized (a span's `revision` changing alone does not count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PackedSegments {
    pub layout: u64,
    pub live: bool,
    pub owners: [PackedSegment; PACKED_SEGMENT_OWNERS],
}

impl PackedSegments {
    /// Drops every span. The layout counts as changed.
    pub fn forget(&mut self) {
        self.live = false;
        self.owners = [PackedSegment::default(); PACKED_SEGMENT_OWNERS];
        self.layout = self.layout.wrapping_add(1);
    }

    /// Installs new spans; the layout counter moves only when the buffer was not
    /// live before or some span's start or length differs.
    pub fn publish(&mut self, owners: [PackedSegment; PACKED_SEGMENT_OWNERS]) {
        let same_places = self.live
            && self
                .owners
                .iter()
                .zip(&owners)
                .all(|(old, new)| (old.start, old.rows) == (new.start, new.rows));
        self.owners = owners;
        self.live = true;
        if !same_places {
            self.layout = self.layout.wrapping_add(1);
        }
    }
}
