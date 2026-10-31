//! Where each owner's rows sit inside a shared packed buffer.

/// Number of owners that share one packed buffer.
pub const COMPACT_SEGMENT_CLAIMANTS: usize = 8;

/// One owner's span of rows, with the revision of what is in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CompactSegment {
    pub start: u32,
    pub rows: u32,
    pub revision: u64,
}

/// The spans of all owners, plus a `layout` counter that moves whenever a span
/// moved or resized (a span's `revision` changing alone does not count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CompactSegments {
    pub bind_layout: u64,
    pub live: bool,
    pub spans: [CompactSegment; COMPACT_SEGMENT_CLAIMANTS],
}

impl CompactSegments {
    /// Drops every span. The layout counts as changed.
    pub fn forget(&mut self) {
        self.live = false;
        self.spans = [CompactSegment::default(); COMPACT_SEGMENT_CLAIMANTS];
        self.bind_layout = self.bind_layout.wrapping_add(1);
    }

    /// Installs new spans; the layout counter moves only when the buffer was not
    /// live before or some span's start or length differs.
    pub fn publish(&mut self, fresh_spans: [CompactSegment; COMPACT_SEGMENT_CLAIMANTS]) {
        let same_places = self.live
            && self
                .spans
                .iter()
                .zip(&fresh_spans)
                .all(|(old, new)| (old.start, old.rows) == (new.start, new.rows));
        self.spans = fresh_spans;
        self.live = true;
        if !same_places {
            self.bind_layout = self.bind_layout.wrapping_add(1);
        }
    }
}
