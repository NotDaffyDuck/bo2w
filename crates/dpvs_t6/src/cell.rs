//! The map's cells and portals, and the small BSP that says which cell a
//! point is in.

use core::ops::Range;

/// One cell: its bounds and the run of portals leading out of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub portal_start: u32,
    pub portal_opening_total: u32,
}

/// One portal: the plane it lies in, the cell it opens into, its run of
/// winding points and the two axes its hull points are expressed in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Portal {
    pub plane: [f32; 4],
    pub cell: u16,
    pub vertex_first: u32,
    pub vertex_tally: u32,
    pub hull_axes_dir: [[f32; 3]; 2],
}

/// A map's cells and portals plus the point-to-cell BSP (`planes` as
/// `[normal, dist]`, `nodes` as the packed node codes).
#[derive(Clone, Copy, Debug)]
pub struct DpvsWorld<'a> {
    pub cells: &'a [Cell],
    pub portals: &'a [Portal],
    pub vertex_points: &'a [[f32; 3]],
    pub planes: &'a [[f32; 4]],
    pub nodes: &'a [u16],
}

/// `start .. start + len`, both ends clamped to `total`.
fn clamped_run(start: u32, len: u32, total: usize) -> Range<usize> {
    let first = total.min(start as usize);
    first..total.min(first + len as usize)
}

impl DpvsWorld<'_> {
    /// Indices of the portals leading out of `cell` (empty for an unknown cell).
    #[must_use]
    pub fn cell_portals(&self, cell: usize) -> Range<usize> {
        match self.cells.get(cell) {
            Some(c) => clamped_run(c.portal_start, c.portal_opening_total, self.portals.len()),
            None => 0..0,
        }
    }

    /// The winding points of `portal` (empty for an unknown portal).
    #[must_use]
    pub fn portal_vertices(&self, portal: usize) -> &[[f32; 3]] {
        match self.portals.get(portal) {
            Some(p) => &self.vertex_points[clamped_run(p.vertex_first, p.vertex_tally, self.vertex_points.len())],
            None => &[],
        }
    }

    /// The cell `spot` is in, by this world's BSP.
    #[must_use]
    pub fn cell_holding(&self, spot: [f32; 3]) -> Option<usize> {
        cell_holding(self.planes, self.nodes, self.cells.len(), spot)
    }
}

/// A splitting plane: normal, distance and axis kind (0..2 along that axis, 3 otherwise).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaneEquation {
    pub normal: [f32; 3],
    pub offset: f32,
    pub r#type: u8,
}

/// The point-to-cell BSP on its own, which also files scene entities into cells.
#[derive(Clone, Copy, Debug)]
pub struct VisibilityCellPlanes<'a> {
    pub planes: &'a [PlaneEquation],
    pub nodes: &'a [u16],
    pub leaf_cells: u32,
}

impl VisibilityCellPlanes<'_> {
    /// The cell `probe` is in; see [`cell_for_point`].
    #[must_use]
    pub fn cell_holding(&self, probe: [f32; 3]) -> Option<usize> {
        descend(self.nodes, self.leaf_cells as usize, |slot| {
            let split = self.planes.get(slot)?;
            let [nx, ny, nz] = split.normal;
            Some(nx * probe[0] + ny * probe[1] + nz * probe[2] - split.offset)
        })
    }
}

/// The cell `at_p` is in, by the BSP `planes` (`[normal, dist]`) and `nodes`.
///
/// Node codes: `1..=cell_count` is the leaf for cell `code - 1` (0 is
/// outside every cell); a larger code is plane `code - cell_count - 1`, and
/// the node after it holds how far ahead its back child is. Strictly in front
/// of the plane continues two nodes on.
#[must_use]
pub fn cell_holding(planes: &[[f32; 4]], nodes: &[u16], cells_total: usize, at_p: [f32; 3]) -> Option<usize> {
    descend(nodes, cells_total, |node_ix| {
        let [nx, ny, nz, dist] = *planes.get(node_ix)?;
        Some(nx * at_p[0] + ny * at_p[1] + nz * at_p[2] - dist)
    })
}

/// Walks the node codes from the root; `distance(plane)` is the point's
/// signed distance from a splitting plane. None for a broken tree.
fn descend(nodes: &[u16], n_cells: usize, dist_fn: impl Fn(usize) -> Option<f32>) -> Option<usize> {
    if n_cells == 0 || nodes.is_empty() {
        return None;
    }
    let mut cur = 0usize;
    // A sound tree reaches a leaf in fewer steps than it has nodes.
    for _ in nodes {
        let code = usize::from(*nodes.get(cur)?);
        let Some(plane) = code.checked_sub(n_cells + 1) else {
            return code.checked_sub(1);
        };
        cur += match dist_fn(plane)? > 0.0 {
            true => 2,
            false => usize::from(*nodes.get(cur + 1)?),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_for_point_splits_on_one_plane() {
        // Two cells split at x = 0: plane node (2 + 1 + 0), back child 3 nodes on.
        let cells = [Cell::default(); 2];
        let planes = [[1.0, 0.0, 0.0, 0.0]];
        let nodes = [3, 3, 1, 2];
        let world = DpvsWorld {
            cells: &cells,
            portals: &[],
            vertex_points: &[],
            planes: &planes,
            nodes: &nodes,
        };
        assert_eq!(world.cell_holding([5.0, 0.0, 0.0]), Some(0));
        assert_eq!(world.cell_holding([-5.0, 0.0, 0.0]), Some(1));
    }
}