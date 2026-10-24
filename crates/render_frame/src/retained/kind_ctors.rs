//! Shorthand for building the map-surface variant of a draw.

use glam::Mat4;

use super::kind::RetainedDrawKind;
use super::lane::BspCameraLane;

impl RetainedDrawKind {
    /// A single map surface at the origin.
    pub const fn world(surf: u16) -> Self {
        Self::world_with_pose(surf, Mat4::IDENTITY)
    }

    /// A single surface placed by `world_from_local`.
    pub const fn world_with_pose(surf: u16, world_from_local: Mat4) -> Self {
        Self::World {
            surf,
            run: 1,
            run_off: 0,
            bsp_kind: None,
            bsp_run_first: None,
            world_from_local,
        }
    }

    /// A map run of `run` surfaces starting at `run_off`. The surfaces share one
    /// sort key and need not be adjacent.
    pub const fn bsp_world(
        surf: u16,
        run: u16,
        run_off: u32,
        bsp_kind: BspCameraLane,
        bsp_run_first: u16,
    ) -> Self {
        Self::World {
            surf,
            run,
            run_off,
            bsp_kind: Some(bsp_kind),
            bsp_run_first: Some(bsp_run_first),
            world_from_local: Mat4::IDENTITY,
        }
    }
}
