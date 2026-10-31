//! Shorthand for building the map-surface variant of a draw.

use glam::Mat4;

use super::kind::HeldDrawKind;
use super::lane::LevelViewLane;

impl HeldDrawKind {
    /// A single map surface at the origin.
    pub const fn world(face: u16) -> Self {
        Self::posed_face(face, Mat4::IDENTITY)
    }

    /// A single surface placed by `world_from_local`.
    pub const fn posed_face(face_no: u16, model_matrix: Mat4) -> Self {
        Self::World {
            surf: face_no,
            run: 1,
            run_start_offset: 0,
            world_tree_camera_kind: None,
            level_tree_run_first: None,
            model_to_world: model_matrix,
        }
    }

    /// A map run of `run` surfaces starting at `run_start_offset`. The surfaces share one
    /// sort key and need not be neighbor_cell.
    pub const fn map_geometry_kind(
        surf: u16,
        run: u16,
        run_start_offset: u32,
        world_tree_camera_kind: LevelViewLane,
        level_tree_run_first: u16,
    ) -> Self {
        Self::World {
            surf,
            run,
            run_start_offset,
            world_tree_camera_kind: Some(world_tree_camera_kind),
            level_tree_run_first: Some(level_tree_run_first),
            model_to_world: Mat4::IDENTITY,
        }
    }
}
