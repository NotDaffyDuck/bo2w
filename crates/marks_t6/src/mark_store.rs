//! The mark pool: mark handles, and the triangle and point groups a mark's pieces are
//! stored in, two to a group, chained from the mark.

use crate::{EffectStainStagingPoint, EffectStainStagingTri, GfxMarkContext};

/// Marks per local client.
pub const DECAL_POOL_CAPACITY: u32 = 256;
/// Triangle groups per local client.
pub const DECAL_TRIANGLE_BLOCK_CAPACITY: u32 = 1024;
/// Point groups per local client.
pub const DECAL_POINT_BLOCK_CAPACITY: u32 = 1536;

/// No mark: the end of the free mark list.
pub const DECAL_HANDLE_ABSENT: u16 = 0xffff;
/// The end of the free triangle-group list. Slot 0 is never handed out, so 0 ends it.
pub const TRI_CHUNK_SUCCESSOR_ABSENT: u32 = 0;
/// The end of the free point-group list, as for triangle groups.
pub const POINT_CHUNK_SUCCESSOR_ABSENT: u32 = 0;
/// The end of a mark's chain of triangle groups.
pub const TRI_CHUNK_CHAIN_TERMINATOR: u16 = 0xffff;
/// The end of a mark's chain of point groups.
pub const POINT_CHUNK_CHAIN_TERMINATOR: u16 = 0xffff;

/// With more than one local client (split screen) only half the marks stay alive.
pub const fn marks_active_limit(local_client_count: u32) -> u32 {
    if local_client_count > 1 {
        DECAL_POOL_CAPACITY / 2
    } else {
        DECAL_POOL_CAPACITY
    }
}

/// At start every mark slot points at the next one; the last ends the list.
pub fn init_mark_next_handle(bay: u32) -> u16 {
    if bay + 1 >= DECAL_POOL_CAPACITY {
        DECAL_HANDLE_ABSENT
    } else {
        (bay + 1) as u16
    }
}

/// At start every triangle group points at the next one; the last ends the list.
pub fn init_tri_next_slot(bay: u32) -> u32 {
    if bay + 1 >= DECAL_TRIANGLE_BLOCK_CAPACITY {
        TRI_CHUNK_SUCCESSOR_ABSENT
    } else {
        bay + 1
    }
}

/// At start every point group points at the next one; the last ends the list.
pub fn init_point_next_slot(bay: u32) -> u32 {
    if bay + 1 >= DECAL_POINT_BLOCK_CAPACITY {
        POINT_CHUNK_SUCCESSOR_ABSENT
    } else {
        bay + 1
    }
}

/// Up to two of a mark's triangles that come from the same surface. 24 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecalTriangleChunk {
    pub indices: [[u16; 3]; 2],
    pub context: GfxMarkContext,
    pub tri_tally: u8,
    /// The next group of the same mark.
    pub next: u16,
    pub tail_fill: u16,
}

const _: () = assert!(size_of::<DecalTriangleChunk>() == 24);

impl DecalTriangleChunk {
    pub const ZERO: Self = Self {
        indices: [[0; 3]; 2],
        context: GfxMarkContext::ZERO,
        tri_tally: 0,
        next: TRI_CHUNK_CHAIN_TERMINATOR,
        tail_fill: 0,
    };
}

/// Up to two of a mark's points. 68 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecalPointChunk {
    pub points: [EffectStainStagingPoint; 2],
    /// The next group of the same mark.
    pub next: u16,
    pub tail_fill: [u8; 2],
}

const _: () = assert!(size_of::<DecalPointChunk>() == 68);

impl DecalPointChunk {
    pub const ZERO: Self = Self {
        points: [EffectStainStagingPoint::ZERO; 2],
        next: POINT_CHUNK_CHAIN_TERMINATOR,
        tail_fill: [0; 2],
    };
}

/// Two triangles share a group only when they come from the same surface.
pub fn mark_contexts_equal(a: &GfxMarkContext, b: &GfxMarkContext) -> bool {
    a == b
}

/// How many triangles go into the group that starts with `triangle_list[0]`: two when the next one
/// has the same context, else one.
fn tris_for_group(triangle_list: &[EffectStainStagingTri]) -> usize {
    match triangle_list {
        [] => 0,
        [first, second, ..] if mark_contexts_equal(&first.context, &second.context) => 2,
        _ => 1,
    }
}

/// How many triangle groups the staged triangles need.
pub fn mark_tri_groups_for_staging(triangle_list: &[EffectStainStagingTri]) -> u32 {
    let mut groups = 0;
    let mut left_tris = triangle_list;
    while !left_tris.is_empty() {
        left_tris = &left_tris[tris_for_group(left_tris)..];
        groups += 1;
    }
    groups
}

/// How many point groups `vertex_total_in_mark` points need.
pub fn mark_point_groups_for_count(vertex_total_in_mark: u32) -> u32 {
    vertex_total_in_mark.div_ceil(2)
}

/// Fills the chain of triangle groups starting at `head` with the staged triangles and
/// returns how many were stored. Each group keeps its place in the chain.
pub fn copy_mark_tris(groups: &mut [DecalTriangleChunk], head: u16, staging: &[EffectStainStagingTri]) -> u32 {
    let mut copied = 0;
    let mut rest = staging;
    let mut handle = head;
    while !rest.is_empty() && handle != TRI_CHUNK_CHAIN_TERMINATOR {
        let Some(group) = groups.get_mut(handle as usize) else {
            break;
        };
        let take = tris_for_group(rest);
        group.context = rest[0].context;
        for (slot, tri) in group.indices.iter_mut().zip(&rest[..take]) {
            *slot = tri.indices;
        }
        group.tri_tally = take as u8;
        copied += take as u32;
        rest = &rest[take..];
        handle = group.next;
    }
    copied
}

/// Fills the chain of point groups starting at `head` with the staged points, two to a
/// group, and returns how many were stored.
pub fn copy_mark_points(
    groups: &mut [DecalPointChunk],
    head: u16,
    staging: &[EffectStainStagingPoint],
) -> u32 {
    let mut copied = 0;
    let mut rest = staging;
    let mut handle = head;
    while !rest.is_empty() && handle != POINT_CHUNK_CHAIN_TERMINATOR {
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
        assert_eq!(init_mark_next_handle(255), DECAL_HANDLE_ABSENT);
        assert_eq!(init_tri_next_slot(1022), 1023);
        assert_eq!(init_tri_next_slot(1023), TRI_CHUNK_SUCCESSOR_ABSENT);
        assert_eq!(init_point_next_slot(1534), 1535);
        assert_eq!(init_point_next_slot(1535), POINT_CHUNK_SUCCESSOR_ABSENT);
    }

    fn tri(indices: [u16; 3], baked_light_slot: u8) -> EffectStainStagingTri {
        EffectStainStagingTri {
            indices,
            context: GfxMarkContext::world_surface(baked_light_slot, 0, 0),
            tail_fill: 0,
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
        let mut groups = [DecalTriangleChunk::ZERO; 4];
        groups[3].next = 1;
        let staging = [tri([0, 1, 2], 1), tri([3, 4, 5], 2), tri([3, 5, 6], 2)];
        assert_eq!(copy_mark_tris(&mut groups, 3, &staging), 3);
        assert_eq!((groups[3].tri_tally, groups[3].indices[0]), (1, [0, 1, 2]));
        assert_eq!(groups[3].context, staging[0].context);
        assert_eq!(
            (groups[1].tri_tally, groups[1].indices),
            (2, [[3, 4, 5], [3, 5, 6]])
        );
        assert_eq!(
            (groups[3].next, groups[1].next),
            (1, TRI_CHUNK_CHAIN_TERMINATOR)
        );
        // A chain one group short stores what fits.
        let mut short = [DecalTriangleChunk::ZERO; 2];
        assert_eq!(copy_mark_tris(&mut short, 0, &staging), 1);
    }

    #[test]
    fn copying_points_goes_two_to_a_group() {
        let mut groups = [DecalPointChunk::ZERO; 3];
        groups[0].next = 2;
        let point = |x: f32| EffectStainStagingPoint {
            spot: [x, 0.0, 0.0],
            ..EffectStainStagingPoint::ZERO
        };
        let staging = [point(1.0), point(2.0), point(3.0)];
        assert_eq!(copy_mark_points(&mut groups, 0, &staging), 3);
        assert_eq!(groups[0].points, [point(1.0), point(2.0)]);
        assert_eq!(groups[2].points[0], point(3.0));
        assert_eq!(
            copy_mark_points(&mut groups, POINT_CHUNK_CHAIN_TERMINATOR, &staging),
            0
        );
    }
}
