//! The collision world's entity grid: which grid cell holds which entity, so a second PC traces
//! against the same cells as the host.

use sim::{AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaSectorSnapshot};

use super::codec::{Decoded, Wire, put_list, refuse, take_list_within};
use crate::transport::wire::{WireReader, WireWriter};

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
    fn put(&self, out: &mut WireWriter) {
        self.world_mins.put(out);
        self.world_maxs.put(out);
        put_list(out, &self.free_prefix);
        self.contiguous_free_head.put(out);
        put_list(out, &self.sectors);
        put_list(out, &self.entities);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let world_mins = Wire::take(input)?;
        let world_maxs = Wire::take(input)?;
        let free_prefix =
            take_list_within(input, FREE_PREFIX_LIMIT, "CM area free prefix exceeds sector pool")?;
        let contiguous_free_head = Wire::take(input)?;
        let sectors = take_list_within(input, SECTOR_LIMIT, "CM area live rows exceed sector pool")?;
        let entities = take_list_within(input, LINK_LIMIT, "CM area links exceed entity pool")?;
        let grid = Self {
            world_mins,
            world_maxs,
            free_prefix,
            contiguous_free_head,
            sectors,
            entities,
        };
        match grid.validate() {
            Ok(()) => Ok(grid),
            Err(_) => refuse("invalid CM area-sector snapshot"),
        }
    }
}
