//! The saved form of the area entity sectors: plain rows that go over the wire and rebuild the sector tree.

/// A sector in use: its place in the sector table, what it holds and how it connects to its neighbours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaSectorSnapshot {
    pub index: u16,
    pub contents_entities: u32,
    /// Head of the chain of entities filed in this sector (0 for none; otherwise an entity number plus one).
    pub entities: u16,
    /// Where the sector splits its box, along `axis`.
    pub dist: f32,
    pub axis: u16,
    pub parent: u16,
    pub child: [u16; 2],
}

/// An entity filed in a sector, with the box and contents it was linked with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaEntityLinkSnapshot {
    pub entity_num: u16,
    pub world_sector: u16,
    /// The next entity in the same sector's chain (an entity number plus one, 0 at the end).
    pub next_entity: u16,
    pub linkcontents: u32,
    pub linkmin: [f32; 3],
    pub linkmax: [f32; 3],
    pub contents: u32,
}

/// The whole area sector table: the world's box, which sectors are free, the sectors in use and the entities filed.
///
/// The free list runs head to tail through the sectors' parent fields. Almost always it ends in an unbroken run up
/// to the last sector, so only the part before that run is listed, plus the first sector of the run.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaEntityWorldSnapshot {
    pub world_mins: [f32; 3],
    pub world_maxs: [f32; 3],
    /// The free list up to (not including) the closing run.
    pub free_prefix: Vec<u16>,
    /// First sector of the closing run `head, head + 1, .., last`; 0 when the free list has no such run.
    pub contiguous_free_head: u16,
    pub sectors: Vec<AreaSectorSnapshot>,
    pub entities: Vec<AreaEntityLinkSnapshot>,
}

/// Why saved rows do not make a sector table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaEntityWorldSnapshotError {
    InvalidWorldBounds,
    InvalidFreeChain,
    InvalidSectorRows,
    InvalidEntityRows,
    /// The entity chains threaded through the sectors do not match the entities listed.
    InvalidRetailRows,
}

impl AreaEntityWorldSnapshot {
    /// Whether the rows rebuild into a sector table.
    pub fn validate(&self) -> Result<(), AreaEntityWorldSnapshotError> {
        self.restore().map(|_| ())
    }
}
