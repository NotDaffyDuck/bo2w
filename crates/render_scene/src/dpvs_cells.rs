//! The world's visibility cells, and which of them the camera can see.

use bevy::prelude::*;
use dpvs_t6::PlaneEquation;

/// The cell tree the visibility walk descends: split planes and the node table.
#[derive(Resource, Clone, Debug, Default)]
pub struct LevelSightRooms {
    pub planes: Vec<PlaneEquation>,
    pub knots: Vec<u16>,
    pub cells_in: usize,
}

impl LevelSightRooms {
    /// The tree can be walked once it has nodes and at least one cell.
    #[must_use]
    pub fn can_walk(&self) -> bool {
        let has_cells = self.cells_in != 0;
        has_cells && self.knots.first().is_some()
    }
}

/// The visible-cell bitset for the frame; `sight_all` means everything is.
#[derive(Resource, Clone, Debug, Default)]
pub struct ExportedCellVisibility {
    pub words: Vec<u32>,
    pub cells_in: usize,
    pub sight_all: bool,
}

/// Tags the entity that carries a dynamic entity's model.
#[derive(Component)]
pub struct DynamicPropModelEntity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_need_nodes_and_a_cell_count_to_be_ready() {
        let mut cells = LevelSightRooms::default();
        assert!(!cells.can_walk());
        cells.knots = vec![0, 1];
        assert!(!cells.can_walk());
        cells.cells_in = 2;
        assert!(cells.can_walk());
        cells.knots.clear();
        assert!(!cells.can_walk());
    }
}
