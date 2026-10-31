//! The collision world's entity grid: which grid cell holds which entity, so a second PC traces
//! against the same cells as the host.

use sim::{AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaSectorSnapshot};

use super::codec::{Wire, put_list, refuse, take_list_within};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Room in the grid's pools; a snapshot claiming more is not one of ours.
const FREE_PREFIX_LIMIT: usize = 1022;
const SECTOR_LIMIT: usize = 1023;
const LINK_LIMIT: usize = 1024;

wire_fields!(AreaSectorSnapshot {
    index,
    contents_entities,
    entities,
    dist,
    axis,
    parent,
    child,
});

wire_fields!(AreaEntityLinkSnapshot {
    entity_num,
    world_sector,
    next_entity,
    linkcontents,
    linkmin,
    linkmax,
    contents,
});

impl Wire for AreaEntityWorldSnapshot {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.world_mins.put(wr);
        self.world_maxs.put(wr);
        put_list(wr, &self.free_prefix);
        self.contiguous_free_head.put(wr);
        put_list(wr, &self.sectors);
        put_list(wr, &self.entities);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let world_mins = Wire::take(rd)?;
        let world_maxs = Wire::take(rd)?;
        let free_prefix_val =
            take_list_within(rd, FREE_PREFIX_LIMIT, "CM area free prefix exceeds sector pool")?;
        let contiguous_free_head_val = Wire::take(rd)?;
        let sectors = take_list_within(rd, SECTOR_LIMIT, "CM area live rows exceed sector pool")?;
        let entities = take_list_within(rd, LINK_LIMIT, "CM area links exceed entity pool")?;
        let grid = Self {
            world_mins,
            world_maxs,
            free_prefix: free_prefix_val,
            contiguous_free_head: contiguous_free_head_val,
            sectors,
            entities,
        };
        match grid.validate() {
            Ok(()) => Ok(grid),
            Err(_) => refuse("invalid CM area-sector snapshot"),
        }
    }
}
