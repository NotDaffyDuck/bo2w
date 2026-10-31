//! What building a world's mesh reports: its size, and why it can fail.

use std::fmt;

/// The size of a built world mesh.
#[derive(Clone, Copy, Debug, Default)]
pub struct LevelMeshMetrics {
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
pub enum BoxSource {
    /// A node of one cell's surface tree.
    TreeNode { cell: usize },
    Surface,
    /// An entry of the static model instance list.
    PropInstance,
}

impl fmt::Display for BoxSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TreeNode { cell } => write!(f, "cell {cell} surface tree node"),
            Self::Surface => f.write_str("surface bounds"),
            Self::PropInstance => f.write_str("static model instance bounds"),
        }
    }
}

/// Why a world mesh could not be built.
#[derive(Debug)]
pub enum LevelMeshFault {
    /// The zone has no world geometry to draw.
    EmptyWorld,
    /// A box was stored as (mins, maxs) where (mid, half) was expected, which
    /// shows up as a negative half-size.
    InsideOutBox {
        table: BoxSource,
        index: usize,
        axis: usize,
        half: f32,
    },
}

impl fmt::Display for LevelMeshFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWorld => f.write_str("the zone has no world geometry"),
            Self::InsideOutBox {
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

impl std::error::Error for LevelMeshFault {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_half_names_the_table_and_the_box() {
        let error = LevelMeshFault::InsideOutBox {
            table: BoxSource::TreeNode { cell: 3 },
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
            LevelMeshFault::EmptyWorld.to_string(),
            "the zone has no world geometry"
        );
    }
}
