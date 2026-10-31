//! The saved form of the area entity sectors: plain rows that go over the wire and rebuild the sector tree.

/// A sector in use: its place in the sector table, what it holds and how it connects to its neighbours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneCellRow {
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
/// Older spelling AreaSectorSnapshot still calls; the rename sweep moves it over.
pub use self::ZoneCellRow as AreaSectorSnapshot;

/// An entity filed in a sector, with the box and contents it was linked with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneLinkRow {
    pub entity_num: u16,
    pub world_sector: u16,
    /// The next entity in the same sector's chain (an entity number plus one, 0 at the end).
    pub next_entity: u16,
    pub linkcontents: u32,
    pub linkmin: [f32; 3],
    pub linkmax: [f32; 3],
    pub contents: u32,
}
/// Older spelling AreaEntityLinkSnapshot still calls; the rename sweep moves it over.
pub use self::ZoneLinkRow as AreaEntityLinkSnapshot;

/// The whole area sector table: the world's box, which sectors are free, the sectors in use and the entities filed.
///
/// The free list runs head to tail through the sectors' parent fields. Almost always it ends in an unbroken run up
/// to the last sector, so only the part before that run is listed, plus the first sector of the run.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneThingTable {
    pub world_mins: [f32; 3],
    pub world_maxs: [f32; 3],
    /// The free list up to (not including) the closing run.
    pub free_prefix: Vec<u16>,
    /// First sector of the closing run `head, head + 1, .., last`; 0 when the free list has no such run.
    pub contiguous_free_head: u16,
    pub sectors: Vec<ZoneCellRow>,
    pub entities: Vec<ZoneLinkRow>,
}
/// Older spelling AreaEntityWorldSnapshot still calls; the rename sweep moves it over.
pub use self::ZoneThingTable as AreaEntityWorldSnapshot;

/// Why saved rows do not make a sector table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionSnapshotFault {
    BadBounds,
    FreeListBroken,
    BadSectorTree,
    BadEntityLinks,
    /// The entity chains threaded through the sectors do not match the entities listed.
    SectorChainsBroken,
}

impl ZoneThingTable {
    /// Whether the rows rebuild into a sector table.
    pub fn validate(&self) -> Result<(), RegionSnapshotFault> {
        self.restore().map(drop)
    }
}
