//! What building a world's mesh reports: its size, and why it can fail.

use std::fmt;

/// The size of a built world mesh.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldMeshStats {
    pub vertices: usize,
    pub triangles: usize,
    pub surfaces: usize,
    /// Lowest corner of the world's bounding box.
    pub min: [f32; 3],
    /// Highest corner of the world's bounding box.
    pub max: [f32; 3],
}

/// Which of the world's bounds tables a box came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsTable {
    /// A node of one cell's surface tree.
    AabbNode { cell: usize },
    Surface,
    /// An entry of the static model instance list.
    SmodelInst,
}

impl fmt::Display for BoundsTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AabbNode { cell } => write!(f, "cell {cell} surface tree node"),
            Self::Surface => f.write_str("surface bounds"),
            Self::SmodelInst => f.write_str("static model instance bounds"),
        }
    }
}

/// Why a world mesh could not be built.
#[derive(Debug)]
pub enum WorldMeshError {
    /// The zone has no world geometry to draw.
    NoGeometry,
    /// A box was stored as (mins, maxs) where (mid, half) was expected, which
    /// shows up as a negative half-size.
    NegativeBoundsHalf {
        table: BoundsTable,
        index: usize,
        axis: usize,
        half: f32,
    },
}

impl fmt::Display for WorldMeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGeometry => f.write_str("the zone has no world geometry"),
            Self::NegativeBoundsHalf {
                table,
                index,
                axis,
                half,
            } => write!(
                f,
                "{table} {index}: half-size {half} on axis {axis} is negative, \
                 so the box was probably read as mins/maxs instead of mid/half"
            ),
        }
    }
}

impl std::error::Error for WorldMeshError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_half_names_the_table_and_the_box() {
        let error = WorldMeshError::NegativeBoundsHalf {
            table: BoundsTable::AabbNode { cell: 3 },
            index: 7,
            axis: 1,
            half: -2.5,
        };
        let text = error.to_string();
        assert!(text.starts_with("cell 3 surface tree node 7:"), "{text}");
        assert!(text.contains("-2.5") && text.contains("axis 1"), "{text}");
    }

    #[test]
    fn a_world_without_geometry_says_so() {
        assert_eq!(
            WorldMeshError::NoGeometry.to_string(),
            "the zone has no world geometry"
        );
    }
}
