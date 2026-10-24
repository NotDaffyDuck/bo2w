//! The mark pool: mark handles, and the triangle and point groups a mark's pieces are
//! stored in, two to a group, chained from the mark.

use crate::{FxMarkStagingPoint, FxMarkStagingTri, GfxMarkContext};

/// Marks per local client.
pub const FX_MARKS_LIMIT: u32 = 256;
/// Triangle groups per local client.
pub const FX_TRI_GROUP_LIMIT: u32 = 1024;
/// Point groups per local client.
pub const FX_POINT_GROUP_LIMIT: u32 = 1536;

/// No mark: the end of the free mark list.
pub const FX_MARK_HANDLE_NONE: u16 = 0xffff;
/// The end of the free triangle-group list. Slot 0 is never handed out, so 0 ends it.
pub const FX_TRI_GROUP_NEXT_NONE: u32 = 0;
/// The end of the free point-group list, as for triangle groups.
pub const FX_POINT_GROUP_NEXT_NONE: u32 = 0;
/// The end of a mark's chain of triangle groups.
pub const FX_TRI_GROUP_CHAIN_NONE: u16 = 0xffff;
/// The end of a mark's chain of point groups.
pub const FX_POINT_GROUP_CHAIN_NONE: u16 = 0xffff;

/// With more than one local client (split screen) only half the marks stay alive.
pub const fn marks_active_limit(local_client_count: u32) -> u32 {
    if local_client_count > 1 {
        FX_MARKS_LIMIT / 2
    } else {
        FX_MARKS_LIMIT
    }
}

/// At start every mark slot points at the next one; the last ends the list.
pub fn init_mark_next_handle(slot: u32) -> u16 {
    if slot + 1 >= FX_MARKS_LIMIT {
        FX_MARK_HANDLE_NONE
    } else {
        (slot + 1) as u16
    }
}

/// At start every triangle group points at the next one; the last ends the list.
pub fn init_tri_next_slot(slot: u32) -> u32 {
    if slot + 1 >= FX_TRI_GROUP_LIMIT {
        FX_TRI_GROUP_NEXT_NONE
    } else {
        slot + 1
    }
}

/// At start every point group points at the next one; the last ends the list.
pub fn init_point_next_slot(slot: u32) -> u32 {
    if slot + 1 >= FX_POINT_GROUP_LIMIT {
        FX_POINT_GROUP_NEXT_NONE
    } else {
        slot + 1
    }
}

/// Up to two of a mark's triangles that come from the same surface. 24 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxTriGroup {
    pub indices: [[u16; 3]; 2],
    pub context: GfxMarkContext,
    pub tri_count: u8,
    /// The next group of the same mark.
    pub next: u16,
    pub _pad: u16,
}

const _: () = assert!(size_of::<FxTriGroup>() == 24);

impl FxTriGroup {
    pub const ZERO: Self = Self {
        indices: [[0; 3]; 2],
        context: GfxMarkContext::ZERO,
        tri_count: 0,
        next: FX_TRI_GROUP_CHAIN_NONE,
        _pad: 0,
    };
}

/// Up to two of a mark's points. 68 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxPointGroup {
    pub points: [FxMarkStagingPoint; 2],
    /// The next group of the same mark.
    pub next: u16,
    pub _pad: [u8; 2],
}

const _: () = assert!(size_of::<FxPointGroup>() == 68);

impl FxPointGroup {
    pub const ZERO: Self = Self {
        points: [FxMarkStagingPoint::ZERO; 2],
        next: FX_POINT_GROUP_CHAIN_NONE,
        _pad: [0; 2],
    };
}

/// Two triangles share a group only when they come from the same surface.
pub fn mark_contexts_equal(a: &GfxMarkContext, b: &GfxMarkContext) -> bool {
    a == b
}

/// How many triangles go into the group that starts with `tris[0]`: two when the next one
/// has the same context, else one.
fn tris_for_group(tris: &[FxMarkStagingTri]) -> usize {
    match tris {
        [] => 0,
        [first, second, ..] if mark_contexts_equal(&first.context, &second.context) => 2,
        _ => 1,
    }
}

/// How many triangle groups the staged triangles need.
pub fn mark_tri_groups_for_staging(tris: &[FxMarkStagingTri]) -> u32 {
    let mut groups = 0;
    let mut rest = tris;
    while !rest.is_empty() {
        rest = &rest[tris_for_group(rest)..];
        groups += 1;
    }
    groups
}

/// How many point groups `point_count` points need.
pub fn mark_point_groups_for_count(point_count: u32) -> u32 {
    point_count.div_ceil(2)
}

/// Fills the chain of triangle groups starting at `head` with the staged triangles and
/// returns how many were stored. Each group keeps its place in the chain.
pub fn copy_mark_tris(groups: &mut [FxTriGroup], head: u16, staging: &[FxMarkStagingTri]) -> u32 {
    let mut copied = 0;
    let mut rest = staging;
    let mut handle = head;
    while !rest.is_empty() && handle != FX_TRI_GROUP_CHAIN_NONE {
        let Some(group) = groups.get_mut(handle as usize) else {
            break;
        };
        let take = tris_for_group(rest);
        group.context = rest[0].context;
        for (slot, tri) in group.indices.iter_mut().zip(&rest[..take]) {
            *slot = tri.indices;
        }
        group.tri_count = take as u8;
        copied += take as u32;
        rest = &rest[take..];
        handle = group.next;
    }
    copied
}

/// Fills the chain of point groups starting at `head` with the staged points, two to a
/// group, and returns how many were stored.
pub fn copy_mark_points(
    groups: &mut [FxPointGroup],
    head: u16,
    staging: &[FxMarkStagingPoint],
) -> u32 {
    let mut copied = 0;
    let mut rest = staging;
    let mut handle = head;
    while !rest.is_empty() && handle != FX_POINT_GROUP_CHAIN_NONE {
        let Some(group) = groups.get_mut(handle as usize) else {
            break;
        };
        let take = rest.len().min(2);
        group.points[..take].copy_from_slice(&rest[..take]);
        copied += take as u32;
        rest = &rest[take..];
        handle = group.next;
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_screen_halves_the_live_marks() {
        assert_eq!(marks_active_limit(0), 256);
        assert_eq!(marks_active_limit(1), 256);
        assert_eq!(marks_active_limit(2), 128);
        assert_eq!(marks_active_limit(4), 128);
    }

    #[test]
    fn free_lists_run_to_the_end() {
        assert_eq!(init_mark_next_handle(0), 1);
        assert_eq!(init_mark_next_handle(254), 255);
        assert_eq!(init_mark_next_handle(255), FX_MARK_HANDLE_NONE);
        assert_eq!(init_tri_next_slot(1022), 1023);
        assert_eq!(init_tri_next_slot(1023), FX_TRI_GROUP_NEXT_NONE);
        assert_eq!(init_point_next_slot(1534), 1535);
        assert_eq!(init_point_next_slot(1535), FX_POINT_GROUP_NEXT_NONE);
    }

    fn tri(indices: [u16; 3], lmap: u8) -> FxMarkStagingTri {
        FxMarkStagingTri {
            indices,
            context: GfxMarkContext::world_surface(lmap, 0, 0),
            _pad: 0,
        }
    }

    #[test]
    fn triangles_pair_up_only_on_the_same_surface() {
        assert_eq!(mark_tri_groups_for_staging(&[]), 0);
        let same = [tri([0, 1, 2], 1), tri([0, 2, 3], 1), tri([0, 3, 4], 1)];
        assert_eq!(mark_tri_groups_for_staging(&same), 2);
        let mixed = [tri([0, 1, 2], 1), tri([3, 4, 5], 2), tri([3, 5, 6], 2)];
        assert_eq!(mark_tri_groups_for_staging(&mixed), 2);
        let all_apart = [tri([0, 1, 2], 1), tri([3, 4, 5], 2), tri([6, 7, 8], 1)];
        assert_eq!(mark_tri_groups_for_staging(&all_apart), 3);
        assert_eq!(mark_point_groups_for_count(0), 0);
        assert_eq!(mark_point_groups_for_count(5), 3);
        assert_eq!(mark_point_groups_for_count(6), 3);
    }

    #[test]
    fn copying_triangles_follows_the_chain() {
        let mut groups = [FxTriGroup::ZERO; 4];
        groups[3].next = 1;
        let staging = [tri([0, 1, 2], 1), tri([3, 4, 5], 2), tri([3, 5, 6], 2)];
        assert_eq!(copy_mark_tris(&mut groups, 3, &staging), 3);
        assert_eq!((groups[3].tri_count, groups[3].indices[0]), (1, [0, 1, 2]));
        assert_eq!(groups[3].context, staging[0].context);
        assert_eq!(
            (groups[1].tri_count, groups[1].indices),
            (2, [[3, 4, 5], [3, 5, 6]])
        );
        assert_eq!(
            (groups[3].next, groups[1].next),
            (1, FX_TRI_GROUP_CHAIN_NONE)
        );
        // A chain one group short stores what fits.
        let mut short = [FxTriGroup::ZERO; 2];
        assert_eq!(copy_mark_tris(&mut short, 0, &staging), 1);
    }

    #[test]
    fn copying_points_goes_two_to_a_group() {
        let mut groups = [FxPointGroup::ZERO; 3];
        groups[0].next = 2;
        let point = |x: f32| FxMarkStagingPoint {
            xyz: [x, 0.0, 0.0],
            ..FxMarkStagingPoint::ZERO
        };
        let staging = [point(1.0), point(2.0), point(3.0)];
        assert_eq!(copy_mark_points(&mut groups, 0, &staging), 3);
        assert_eq!(groups[0].points, [point(1.0), point(2.0)]);
        assert_eq!(groups[2].points[0], point(3.0));
        assert_eq!(
            copy_mark_points(&mut groups, FX_POINT_GROUP_CHAIN_NONE, &staging),
            0
        );
    }
}
