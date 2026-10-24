use core::ops::Range;

/// GfxCell: its bounds and its run of portals (each leads out of this cell).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub portal_start: u32,
    pub portal_count: u32,
}

/// GfxPortal: the DpvsPlane it lies in, the cell it leads into, its winding and the two axes
/// its hull points are kept in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Portal {
    pub plane: [f32; 4],
    pub cell: u16,
    pub vert_start: u32,
    pub vert_count: u32,
    pub hull_axis: [[f32; 3]; 2],
}

/// A BO2 map's cells, portals and the GfxWorldDpvsPlanes BSP that finds a point's cell.
/// `planes` are cplane_s as `[normal, dist]`; `nodes` is the u16 node array.
#[derive(Clone, Copy, Debug)]
pub struct DpvsWorld<'a> {
    pub cells: &'a [Cell],
    pub portals: &'a [Portal],
    pub verts: &'a [[f32; 3]],
    pub planes: &'a [[f32; 4]],
    pub nodes: &'a [u16],
}

impl DpvsWorld<'_> {
    #[must_use]
    pub fn cell_portals(&self, cell: usize) -> Range<usize> {
        let Some(c) = self.cells.get(cell) else {
            return 0..0;
        };
        let start = (c.portal_start as usize).min(self.portals.len());
        let end = (start + c.portal_count as usize).min(self.portals.len());
        start..end
    }

    #[must_use]
    pub fn portal_verts(&self, portal: usize) -> &[[f32; 3]] {
        let Some(p) = self.portals.get(portal) else {
            return &[];
        };
        let start = (p.vert_start as usize).min(self.verts.len());
        let end = (start + p.vert_count as usize).min(self.verts.len());
        &self.verts[start..end]
    }

    /// R_CellForPoint over this world's planes and nodes.
    #[must_use]
    pub fn cell_for_point(&self, p: [f32; 3]) -> Option<usize> {
        cell_for_point(self.planes, self.nodes, self.cells.len(), p)
    }
}

/// cplane_s: normal, dist and type (0..2 for a plane along that axis, 3 otherwise).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CPlane {
    pub normal: [f32; 3],
    pub dist: f32,
    pub r#type: u8,
}

/// GfxWorldDpvsPlanes: the planes and u16 nodes of the BSP that finds a point's cell and
/// files scene ents into cells.
#[derive(Clone, Copy, Debug)]
pub struct DpvsPlanes<'a> {
    pub planes: &'a [CPlane],
    pub nodes: &'a [u16],
    pub cell_count: u32,
}

impl DpvsPlanes<'_> {
    /// R_CellForPoint, as [`cell_for_point`].
    #[must_use]
    pub fn cell_for_point(&self, p: [f32; 3]) -> Option<usize> {
        let cell_count = self.cell_count as usize;
        if self.nodes.is_empty() || cell_count == 0 {
            return None;
        }
        let mut i = 0usize;
        for _ in 0..self.nodes.len() {
            let value = usize::from(*self.nodes.get(i)?);
            if value <= cell_count {
                return value.checked_sub(1).filter(|&c| c < cell_count);
            }
            let plane = self.planes.get(value - (cell_count + 1))?;
            let n = plane.normal;
            let d = n[0] * p[0] + n[1] * p[1] + n[2] * p[2] - plane.dist;
            i += if d > 0.0 {
                2
            } else {
                usize::from(*self.nodes.get(i + 1)?)
            };
        }
        None
    }
}

/// R_CellForPoint over a GfxWorldDpvsPlanes BSP alone (`planes` as `[normal, dist]`): a node
/// value at or below `cell_count` is leaf `value - 1`, a larger one is plane
/// `value - (cell_count + 1)` followed by the offset of its back child. In front
/// (distance > 0) goes to the next node.
#[must_use]
pub fn cell_for_point(
    planes: &[[f32; 4]],
    nodes: &[u16],
    cell_count: usize,
    p: [f32; 3],
) -> Option<usize> {
    if nodes.is_empty() || cell_count == 0 {
        return None;
    }
    let mut i = 0usize;
    for _ in 0..nodes.len() {
        let value = usize::from(*nodes.get(i)?);
        if value <= cell_count {
            return value.checked_sub(1).filter(|&c| c < cell_count);
        }
        let plane = planes.get(value - (cell_count + 1))?;
        let d = plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] - plane[3];
        i += if d > 0.0 {
            2
        } else {
            usize::from(*nodes.get(i + 1)?)
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
            verts: &[],
            planes: &planes,
            nodes: &nodes,
        };
        assert_eq!(world.cell_for_point([5.0, 0.0, 0.0]), Some(0));
        assert_eq!(world.cell_for_point([-5.0, 0.0, 0.0]), Some(1));
    }
}
