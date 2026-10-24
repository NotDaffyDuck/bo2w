//! The world's visibility cells, and which of them the camera can see.

use bevy::prelude::*;
use dpvs_t6::CPlane;

/// The cell tree the visibility walk descends: split planes and the node table.
#[derive(Resource, Clone, Debug, Default)]
pub struct WorldDpvsCells {
    pub planes: Vec<CPlane>,
    pub nodes: Vec<u16>,
    pub cell_count: usize,
}

impl WorldDpvsCells {
    /// The tree can be walked once it has nodes and at least one cell.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        !self.nodes.is_empty() && self.cell_count > 0
    }
}

/// The visible-cell bitset for the frame; `vis_all` means everything is.
#[derive(Resource, Clone, Debug, Default)]
pub struct PublishedCellVis {
    pub words: Vec<u32>,
    pub cell_count: usize,
    pub vis_all: bool,
}

/// Tags the entity that carries a dynamic entity's model.
#[derive(Component)]
pub struct DynEntModelEntity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_need_nodes_and_a_cell_count_to_be_ready() {
        let mut cells = WorldDpvsCells::default();
        assert!(!cells.is_ready());
        cells.nodes = vec![0, 1];
        assert!(!cells.is_ready());
        cells.cell_count = 2;
        assert!(cells.is_ready());
        cells.nodes.clear();
        assert!(!cells.is_ready());
    }
}
