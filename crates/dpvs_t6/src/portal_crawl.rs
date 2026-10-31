use alloc::vec;
use alloc::vec::Vec;

use crate::cache::{CachedCell, CellClip, PLANE_POOL_MAX};
use crate::cell::DpvsWorld;
use crate::portal_cut::slice_by_portal;
use crate::cut::{Bevels, CLIP_PLANE_CEILING, ViewOrigin, frustum_clip_planes_for};
use crate::hull::{HULL_POINTS_MAX, HullPoints, convex_hull, hull_point, hull_uv};
use crate::plane::{box_culled, plane_dist, winding_behind_plane};
use crate::cell_bits::{bit_get, bit_set};

/// The portal queue: a min-heap of 256 ("More than %i queued portals").
pub const QUEUED_PORTAL_MAX: usize = 256;

/// The band around a portal plane the eye counts as standing in.
pub const EYE_EPSILON: f32 = 0.125;

/// A popped portal's plane must still have the eye this far in front of it at most.
pub const POP_EYE_LIMIT: f32 = EYE_EPSILON * 0.999;

/// The start cell is visited with at most this many of the camera's planes.
pub const START_PLANE_LIMIT: usize = 14;

/// Occluders: up to 64, each a volume of 5 planes.
pub const OCCLUDER_MAX: usize = 64;
pub const OCCLUDER_PLANES: usize = 5;

/// The hull-point pool: 256 buffers, one per queued portal.
const HULL_POOL: usize = 256;

/// Not BO2: how deep stepping through portals the eye stands in may nest, and how many
/// portals one walk may pop when no r_portalWalkLimit is set. Guards against bad map data.
const STEP_DEPTH_MAX: u16 = 16;
const POP_GUARD: u32 = 0x4000;

const NONE: u32 = u32::MAX;
const NO_HULL: u16 = u16::MAX;

/// What one walk reads besides the map.
#[derive(Clone, Copy, Debug)]
pub struct WalkParams<'a> {
    pub eye: [f32; 3],
    /// The plane through the eye facing the view; a portal's queue key is how far along it
    /// its furthest point lies.
    pub view_plane: [f32; 4],
    /// The camera's planes (sides, near, far); the start cell takes the first 14.
    pub planes: &'a [[f32; 4]],
    /// The near plane each child plane set ends with, pulled back to the portal.
    pub near: Option<[f32; 4]>,
    pub far: Option<[f32; 4]>,
    /// r_portalWalkLimit: stop after this many portals; 0 walks them all.
    pub walk_limit: u32,
    pub bevels: Option<&'a Bevels>,
    /// cellForceInvisible bits when r_dpvs_useCellForceInvisibleBits is on.
    pub force_invisible: Option<&'a [u32]>,
    /// Occluder volumes; a portal fully inside one (in front of all 5 planes) is skipped.
    pub occluders: &'a [[[f32; 4]; OCCLUDER_PLANES]],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraversalTally {
    pub cells_added: u32,
    pub portals_visited: u32,
    pub skipped_by_parent_total: u32,
    pub skipped_by_orientation_total: u32,
    pub skipped_by_clip_total: u32,
    pub skip_occluded: u32,
    pub clipped_to_nothing: u32,
    pub queued_total: u32,
    pub requeued: u32,
    pub stepped_into: u32,
    pub popped: u32,
    pub hull_empty: u32,
    pub pop_eye_skip: u32,
    pub queue_overflow: u32,
    pub hull_pool_empty: u32,
    pub walk_limit_hit: u32,
    pub plane_pool_full: u32,
    pub forced_invisible: u32,
}

/// GfxPortal.writable.
#[derive(Clone, Copy, Debug)]
struct PortalState {
    queued: bool,
    ancestor: bool,
    depth: u16,
    hull: u16,
    owner: u32,
    key: f32,
}

impl PortalState {
    const CLEAR: Self = Self {
        queued: false,
        ancestor: false,
        depth: 0,
        hull: NO_HULL,
        owner: NONE,
        key: 0.0,
    };
}

/// Everything one walk writes, kept between frames so it allocates once.
#[derive(Debug, Default)]
pub struct TraversalScratchpad {
    portals: Vec<PortalState>,
    queue: Vec<u32>,
    collision_volumes: Vec<HullPoints>,
    free_hulls: Vec<u16>,
    planes: Vec<[f32; 4]>,
    cached: Vec<CachedCell>,
    cell_to_cached: Vec<(u32, u32)>,
    visible: Vec<u32>,
    chop_a: Vec<[f32; 3]>,
    chop_b: Vec<[f32; 3]>,
}

impl TraversalScratchpad {
    fn reset(&mut self, world: &DpvsWorld<'_>) {
        self.portals.clear();
        self.portals.resize(world.portals.len(), PortalState::CLEAR);
        self.queue.clear();
        if self.collision_volumes.len() != HULL_POOL {
            self.collision_volumes = vec![HullPoints::BLANK; HULL_POOL];
        }
        self.free_hulls.clear();
        self.free_hulls.extend((0..HULL_POOL as u16).rev());
        self.planes.clear();
        self.cached.clear();
        self.cell_to_cached.clear();
        self.cell_to_cached.resize(world.cells.len(), (0, 0));
        self.visible.clear();
        self.visible.resize(world.cells.len().div_ceil(32), 0);
        let longest = world
            .portals
            .iter()
            .map(|p| p.vertex_tally as usize)
            .max()
            .unwrap_or(0);
        let chop_len = (longest + CLIP_PLANE_CEILING + START_PLANE_LIMIT).max(HULL_POINTS_MAX);
        self.chop_a.resize(chop_len, [0.0; 3]);
        self.chop_b.resize(chop_len, [0.0; 3]);
    }

    #[must_use]
    pub fn cell_visible(&self, cell: usize) -> bool {
        bit_get(&self.visible, cell)
    }

    /// The visible cells' bits, 32 cells to a word.
    #[must_use]
    pub fn visible_bits(&self) -> &[u32] {
        &self.visible
    }

    /// Every visit of the last walk, sorted by cell.
    #[must_use]
    pub fn cached_cells(&self) -> &[CachedCell] {
        &self.cached
    }

    /// The planes a cached visit was seen through.
    #[must_use]
    pub fn cached_planes(&self, entry: &CachedCell) -> &[[f32; 4]] {
        let start = entry.plane_index as usize;
        let end = (start + usize::from(entry.plane_tally)).min(self.planes.len());
        self.planes.get(start..end).unwrap_or(&[])
    }

    /// The visits of one cell (cellToCachedCellMap).
    #[must_use]
    pub fn cell_visits(&self, cell: usize) -> &[CachedCell] {
        let Some(&(first_ix, count)) = self.cell_to_cached.get(cell) else {
            return &[];
        };
        let (start, count) = (first_ix as usize, count as usize);
        self.cached.get(start..start + count).unwrap_or(&[])
    }

    /// The planes of a cell's first visit, or `None` when the walk did not reach it.
    #[must_use]
    pub fn first_visit_clip(&self, cell: usize) -> Option<CellClip> {
        let first = self.cell_visits(cell).first()?;
        Some(CellClip::new(self.cached_planes(first), first.view_wall_total))
    }

    /// R_DpvsBoxIsVisible: with nothing cached every box is visible; otherwise a box must
    /// overlap a visible cell's bounds and pass the planes of one of its visits.
    #[must_use]
    pub fn box_is_visible(&self, world: &DpvsWorld<'_>, lo: [f32; 3], hi: [f32; 3]) -> bool {
        if self.cached.is_empty() {
            return true;
        }
        self.cached.iter().any(|entry| {
            let Some(cell) = world.cells.get(usize::from(entry.cell)) else {
                return false;
            };
            let overlaps = (0..3).all(|i| lo[i] <= cell.hi[i] && hi[i] >= cell.lo[i]);
            overlaps && !box_culled(self.cached_planes(entry), lo, hi)
        })
    }
}

struct Walk<'a> {
    world: &'a DpvsWorld<'a>,
    params: &'a WalkParams<'a>,
    s: &'a mut TraversalScratchpad,
    stats: TraversalTally,
    /// The eye, or for the sun-shadow caster walk the direction back to the sun.
    origin: ViewOrigin,
}

/// R_VisitPortals: the cells seen from `start_cell` through its portals, each visit kept
/// with its planes in `scratch`. Read the result with the scratch's methods.
pub fn visit_portals(
    world: &DpvsWorld<'_>,
    start_cell: usize,
    params: &WalkParams<'_>,
    scratch: &mut TraversalScratchpad,
) -> TraversalTally {
    scratch.reset(world);
    if start_cell >= world.cells.len() {
        return TraversalTally::default();
    }
    let mut walk = Walk {
        world,
        params,
        s: scratch,
        stats: TraversalTally::default(),
        origin: ViewOrigin::Point(params.eye),
    };
    let start = &params.planes[..params.planes.len().min(START_PLANE_LIMIT)];
    walk.s.planes.extend_from_slice(start);
    let count = start.len() as u8;
    walk.visit_cell(start_cell, 0, count, count, 0, NONE);
    walk.drain_queue();
    walk.finish_cache();
    walk.stats
}

/// R_VisitPortalsNoFrustum: the cells between `seed_cell` and the sun, through its portals,
/// one row of cellCasterBits (cells that may hold something shadowing the seed cell).
/// `sun_dir` points toward the sun (BO2's sun direction). The view origin is that direction
/// negated with w 0 (viewOrgIsDir), an eye infinitely far on the side away from the sun;
/// the near plane is the sun direction with d 0, there is no far plane, and the seed cell
/// has no planes. A portal is followed
/// when, besides the usual ancestor, plane and chop tests, its plane has that direction at
/// most 0.125 in front of it (MW2 also drops portals facing away from the view direction;
/// BO2 does not). Every followed portal is queued; none is stepped through. Read the cells
/// with the scratch's `cell_visible` or `visible_bits`.
pub fn crawl_portals_unbounded(
    world: &DpvsWorld<'_>,
    seed_cell: usize,
    toward_sun: [f32; 3],
    scratch: &mut TraversalScratchpad,
) -> TraversalTally {
    scratch.reset(world);
    if seed_cell >= world.cells.len() {
        return TraversalTally::default();
    }
    let near = [toward_sun[0], toward_sun[1], toward_sun[2], 0.0];
    let params = WalkParams {
        eye: [0.0; 3],
        view_plane: near,
        planes: &[],
        near: Some(near),
        far: None,
        walk_limit: 0,
        bevels: None,
        force_invisible: None,
        occluders: &[],
    };
    let mut walk = Walk {
        world,
        params: &params,
        s: scratch,
        stats: TraversalTally::default(),
        origin: ViewOrigin::Dir([-toward_sun[0], -toward_sun[1], -toward_sun[2]]),
    };
    walk.visit_cell(seed_cell, 0, 0, 0, 0, NONE);
    walk.drain_queue();
    walk.finish_cache();
    walk.stats
}

impl Walk<'_> {
    /// How far the view origin stands in front of a plane; a direction (w 0) leaves out the
    /// plane's d.
    fn origin_dist(&self, cut_plane: [f32; 4]) -> f32 {
        match self.origin {
            ViewOrigin::Point(eye) => plane_dist(cut_plane, eye),
            ViewOrigin::Dir(d) => cut_plane[0] * d[0] + cut_plane[1] * d[1] + cut_plane[2] * d[2],
        }
    }

    /// R_VisitPortalsForCell.
    fn visit_cell(
        &mut self,
        cell: usize,
        plane_index: u32,
        plane_tally: u8,
        view_wall_total: u8,
        depth: u16,
        owner: u32,
    ) {
        self.add_cell(cell, plane_index, plane_tally, view_wall_total);
        let start = plane_index as usize;
        let planes_end = start + usize::from(plane_tally);
        let world = self.world;
        let eye = self.params.eye;
        for portal_index in world.cell_portals(cell) {
            self.stats.portals_visited += 1;
            if self.s.portals[portal_index].ancestor {
                self.stats.skipped_by_parent_total += 1;
                continue;
            }
            let portal = world.portals[portal_index];
            let eye_dist = self.origin_dist(portal.plane);
            if eye_dist > EYE_EPSILON {
                self.stats.skipped_by_orientation_total += 1;
                continue;
            }
            let vertex_points = world.portal_vertices(portal_index);
            let planes = &self.s.planes[start..planes_end];
            if planes.iter().any(|&p| winding_behind_plane(p, vertex_points)) {
                self.stats.skipped_by_clip_total += 1;
                continue;
            }
            if self.occluded(vertex_points) {
                self.stats.skip_occluded += 1;
                continue;
            }
            if eye_dist <= -EYE_EPSILON || matches!(self.origin, ViewOrigin::Dir(_)) {
                self.queue_portal(portal_index, start, planes_end, depth, owner);
            } else if depth < STEP_DEPTH_MAX && winding_contains_eye(portal.plane, vertex_points, eye) {
                // The eye stands in the portal: step through at once with the current planes
                // plus one. Read as plus the portal's own plane, which keeps the walk from
                // stepping back out through the neighbour's portal home (QUESTIONS.md).
                let neighbour = usize::from(portal.cell);
                if neighbour >= world.cells.len() {
                    continue;
                }
                self.stats.stepped_into += 1;
                let (index, count, frustum) =
                    if self.s.planes.len() + usize::from(plane_tally) < PLANE_POOL_MAX {
                        let index = self.s.planes.len();
                        self.s.planes.extend_from_within(start..planes_end);
                        self.s.planes.push(portal.plane);
                        (index as u32, plane_tally + 1, 0)
                    } else {
                        self.stats.plane_pool_full += 1;
                        (plane_index, plane_tally, view_wall_total)
                    };
                self.s.portals[portal_index].owner = owner;
                self.s.portals[portal_index].ancestor = true;
                self.visit_cell(
                    neighbour,
                    index,
                    count,
                    frustum,
                    depth + 1,
                    portal_index as u32,
                );
                self.s.portals[portal_index].ancestor = false;
            }
        }
    }

    /// R_AddCellSurfaces and the visible bit, unless the cell is forced invisible; every
    /// visit is cached with its planes.
    fn add_cell(&mut self, cell: usize, plane_index: u32, plane_tally: u8, fr_bits: u8) {
        if self
            .params
            .force_invisible
            .is_some_and(|bits| bit_get(bits, cell))
        {
            self.stats.forced_invisible += 1;
            return;
        }
        if !bit_get(&self.s.visible, cell) {
            self.stats.cells_added += 1;
            bit_set(&mut self.s.visible, cell);
        }
        self.s.cached.push(CachedCell {
            cell: cell as u16,
            plane_index,
            plane_tally,
            view_wall_total: fr_bits,
        });
    }

    fn occluded(&self, vertex_points: &[[f32; 3]]) -> bool {
        self.params.occluders.iter().take(OCCLUDER_MAX).any(|occluder| {
            occluder
                .iter()
                .all(|&p| vertex_points.iter().all(|&v| plane_dist(p, v) > 0.0))
        })
    }

    /// Chop the portal by the planes it is seen through, add what is left to its hull
    /// points and queue it (or merge into its queued entry).
    fn queue_portal(
        &mut self,
        portal_index: usize,
        planes_start: usize,
        planes_end: usize,
        depth: u16,
        owner: u32,
    ) {
        let world = self.world;
        let portal = world.portals[portal_index];
        let vertex_points = world.portal_vertices(portal_index);
        let n = {
            let s = &mut *self.s;
            slice_by_portal(
                vertex_points,
                &s.planes[planes_start..planes_end],
                &mut s.chop_a,
                &mut s.chop_b,
            )
        };
        if n == 0 {
            self.stats.clipped_to_nothing += 1;
            return;
        }
        let state = self.s.portals[portal_index];
        let hull = if state.queued {
            self.stats.requeued += 1;
            state.hull
        } else {
            if self.s.queue.len() == QUEUED_PORTAL_MAX {
                self.stats.queue_overflow += 1;
                return;
            }
            let Some(hull) = self.s.free_hulls.pop() else {
                self.stats.hull_pool_empty += 1;
                return;
            };
            self.s.collision_volumes[usize::from(hull)].count = 0;
            hull
        };
        let mut key = if state.queued {
            state.key
        } else {
            f32::NEG_INFINITY
        };
        for i in 0..n {
            let v = self.s.chop_a[i];
            key = key.max(plane_dist(self.params.view_plane, v));
            let _ = self.s.collision_volumes[usize::from(hull)].push_point(hull_uv(v, portal.hull_axes_dir));
        }
        let entry = &mut self.s.portals[portal_index];
        entry.key = key;
        if state.queued {
            entry.depth = entry.depth.min(depth);
            if entry.owner != owner {
                entry.owner = NONE;
            }
        } else {
            entry.queued = true;
            entry.hull = hull;
            entry.depth = depth;
            entry.owner = owner;
            self.s.queue.push(portal_index as u32);
            self.stats.queued_total += 1;
        }
    }

    /// R_NextQueuedPortal: the queued portal whose furthest point is nearest.
    fn pop_portal(&mut self) -> Option<usize> {
        let portals = &self.s.portals;
        let (slot, _) = self.s.queue.iter().enumerate().min_by(|a, b| {
            let ka = portals[*a.1 as usize].key;
            let pressed_key_latch = portals[*b.1 as usize].key;
            ka.total_cmp(&pressed_key_latch)
        })?;
        Some(self.s.queue.swap_remove(slot) as usize)
    }

    fn drain_queue(&mut self) {
        let world = self.world;
        let limit = if self.params.walk_limit == 0 {
            POP_GUARD
        } else {
            self.params.walk_limit
        };
        while let Some(portal_index) = self.pop_portal() {
            let state = self.s.portals[portal_index];
            self.s.portals[portal_index].queued = false;
            self.s.portals[portal_index].hull = NO_HULL;
            let mut pt2 = [[0.0f32; 2]; HULL_POINTS_MAX];
            let mut points = 0;
            if state.hull != NO_HULL {
                let h = &self.s.collision_volumes[usize::from(state.hull)];
                points = convex_hull(&h.point_count[..h.count], &mut pt2);
                self.s.free_hulls.push(state.hull);
            }
            if points == 0 {
                self.stats.hull_empty += 1;
                continue;
            }
            if self.stats.popped >= limit {
                self.stats.walk_limit_hit += 1;
                break;
            }
            self.stats.popped += 1;
            let portal = world.portals[portal_index];
            if self.origin_dist(portal.plane) > POP_EYE_LIMIT {
                self.stats.pop_eye_skip += 1;
                continue;
            }
            let neighbour = usize::from(portal.cell);
            if neighbour >= world.cells.len() {
                continue;
            }

            let mut ring = [[0.0f32; 3]; HULL_POINTS_MAX];
            for i in 0..points {
                ring[i] = hull_point(portal.plane, portal.hull_axes_dir, pt2[i]);
            }
            face_the_eye(&mut ring[..points], portal.plane);
            let mut sub = [[0.0f32; 4]; CLIP_PLANE_CEILING];
            let count = frustum_clip_planes_for(
                &ring[..points],
                self.origin,
                self.params.near,
                self.params.far,
                self.params.bevels,
                &mut sub,
            );
            let (plane_index, plane_tally) = if self.s.planes.len() + count <= PLANE_POOL_MAX {
                let index = self.s.planes.len() as u32;
                self.s.planes.extend_from_slice(&sub[..count]);
                (index, count as u8)
            } else {
                // Out of plane room: fall back to the camera's planes (seeing more, never less).
                self.stats.plane_pool_full += 1;
                (0, START_PLANE_LIMIT.min(self.params.planes.len()) as u8)
            };

            self.set_ancestors(portal_index, true);
            self.visit_cell(
                neighbour,
                plane_index,
                plane_tally,
                0,
                state.depth + 1,
                portal_index as u32,
            );
            self.set_ancestors(portal_index, false);
        }
    }

    /// R_SetAncestorListStatus: the popped portal and its chain of queued parents.
    fn set_ancestors(&mut self, portal_index: usize, on: bool) {
        let mut at = portal_index as u32;
        for _ in 0..self.s.portals.len() {
            let Some(state) = self.s.portals.get_mut(at as usize) else {
                break;
            };
            state.ancestor = on;
            at = state.owner;
            if at == NONE {
                break;
            }
        }
    }

    /// Sort the visits by cell (DpvsCachedCell::Compare) and fill cellToCachedCellMap.
    fn finish_cache(&mut self) {
        let s = &mut *self.s;
        s.cached
            .sort_by_key(|c| (c.cell, c.view_wall_total == 0, c.plane_index));
        let mut i = 0;
        while i < s.cached.len() {
            let cell = usize::from(s.cached[i].cell);
            let mut j = i;
            while j < s.cached.len() && usize::from(s.cached[j].cell) == cell {
                j += 1;
            }
            s.cell_to_cached[cell] = (i as u32, (j - i) as u32);
            i = j;
        }
    }
}

/// Wind a rebuilt hull so its side planes face inward: its normal points back at the eye,
/// which stands behind the portal plane (BO2's Com_ConvexHull winding order is unknown).
fn face_the_eye(ring_pts: &mut [[f32; 3]], half_space: [f32; 4]) {
    let n = ring_pts.len();
    let mut normal = [0.0f32; 3];
    for i in 0..n {
        let a = ring_pts[i];
        let b = ring_pts[(i + 1) % n];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    if normal[0] * half_space[0] + normal[1] * half_space[1] + normal[2] * half_space[2] > 0.0 {
        ring_pts.reverse();
    }
}

/// The winding, projected on its two major axes, holds the eye (either winding order).
fn winding_contains_eye(face_plane: [f32; 4], vertex_points: &[[f32; 3]], viewpoint: [f32; 3]) -> bool {
    if vertex_points.len() < 3 {
        return false;
    }
    let (ax_u, ax_v) = projection_axes([face_plane[0], face_plane[1], face_plane[2]]);
    let mut turn_seen = 0.0f32;
    let mut prev = vertex_points[vertex_points.len() - 1];
    for &cur in vertex_points {
        let side =
            (cur[ax_u] - prev[ax_u]) * (viewpoint[ax_v] - prev[ax_v]) - (cur[ax_v] - prev[ax_v]) * (viewpoint[ax_u] - prev[ax_u]);
        if side != 0.0 {
            if turn_seen != 0.0 && side.signum() != turn_seen {
                return false;
            }
            turn_seen = side.signum();
        }
        prev = cur;
    }
    true
}

/// The two axes a plane is projected on: drop its largest normal component.
fn projection_axes(nrm: [f32; 3]) -> (usize, usize) {
    let a = [nrm[0].abs(), nrm[1].abs(), nrm[2].abs()];
    if a[0] >= a[1] && a[0] >= a[2] {
        (1, 2)
    } else if a[1] >= a[2] {
        (2, 0)
    } else {
        (0, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{Cell, Portal};

    // Rooms along +x, each 10 wide (cell i spans x 10i..10i+10, y and z -5..5), joined by
    // doorways 2 wide and 2 tall. Each portal's plane faces into the room it leads to, so an
    // eye in the room it leaves stands behind it.
    fn rooms(count: usize) -> (Vec<Cell>, Vec<Portal>, Vec<[f32; 3]>) {
        let mut reached = Vec::new();
        let mut portals = Vec::new();
        let mut vertex_points = Vec::new();
        for i in 0..count {
            let x0 = 10.0 * i as f32;
            let start = portals.len() as u32;
            let mut add = |x: f32, dir: f32, to: usize| {
                let vertex_first = vertex_points.len() as u32;
                for (y, z) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
                    vertex_points.push([x, y, z]);
                }
                portals.push(Portal {
                    plane: [dir, 0.0, 0.0, -dir * x],
                    cell: to as u16,
                    vertex_first,
                    vertex_tally: 4,
                    hull_axes_dir: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                });
            };
            if i + 1 < count {
                add(x0 + 10.0, 1.0, i + 1);
            }
            if i > 0 {
                add(x0, -1.0, i - 1);
            }
            reached.push(Cell {
                lo: [x0, -5.0, -5.0],
                hi: [x0 + 10.0, 5.0, 5.0],
                portal_start: start,
                portal_opening_total: portals.len() as u32 - start,
            });
        }
        (reached, portals, vertex_points)
    }

    fn world<'a>(
        cells: &'a [Cell],
        portals: &'a [Portal],
        vertex_points: &'a [[f32; 3]],
    ) -> DpvsWorld<'a> {
        DpvsWorld {
            cells,
            portals,
            vertex_points,
            planes: &[],
            nodes: &[],
        }
    }

    fn params(eye: [f32; 3], cut_planes: &[[f32; 4]]) -> WalkParams<'_> {
        WalkParams {
            eye,
            view_plane: [1.0, 0.0, 0.0, -eye[0]],
            planes: cut_planes,
            near: Some([1.0, 0.0, 0.0, -(eye[0] + 0.1)]),
            far: None,
            walk_limit: 0,
            bevels: None,
            force_invisible: None,
            occluders: &[],
        }
    }

    // Looking down +x with a wide open view: only the near plane.
    const LOOK_X: [[f32; 4]; 1] = [[1.0, 0.0, 0.0, -5.1]];

    #[test]
    fn walk_sees_down_the_hall() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let stats = visit_portals(&w, 0, &params([5.0, 0.0, 0.0], &LOOK_X), &mut s);
        assert!(s.cell_visible(0) && s.cell_visible(1) && s.cell_visible(2));
        assert_eq!(stats.cells_added, 3);
        assert_eq!(stats.popped, 2);
        // Room 2 is seen through the second doorway: 4 sides and the near plane.
        let visits = s.cell_visits(2);
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].view_wall_total, 0);
        assert_eq!(s.cached_planes(&visits[0]).len(), 5);
    }

    #[test]
    fn looking_away_sees_only_the_start_room() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let back = [[-1.0, 0.0, 0.0, 4.9]];
        let stats = visit_portals(&w, 0, &params([5.0, 0.0, 0.0], &back), &mut s);
        assert!(s.cell_visible(0) && !s.cell_visible(1));
        assert_eq!(stats.skipped_by_clip_total, 1);
    }

    #[test]
    fn doorway_narrows_what_the_next_room_shows() {
        // Seen from the middle of room 0, room 1 shows only what lines up with the doorway.
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        visit_portals(&w, 0, &params([5.0, 0.0, 0.0], &LOOK_X), &mut s);
        assert!(s.box_is_visible(&w, [14.0, -0.5, -0.5], [15.0, 0.5, 0.5]));
        assert!(!s.box_is_visible(&w, [14.0, 3.5, -0.5], [15.0, 4.5, 0.5]));
    }

    #[test]
    fn eye_in_the_doorway_steps_through() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        // 0.11 short of the doorway: inside its 0.125 band, the near plane just past it.
        let eye = [9.89, 0.0, 0.0];
        let planes = [[1.0, 0.0, 0.0, -9.99]];
        let stats = visit_portals(&w, 0, &params(eye, &planes), &mut s);
        assert_eq!(stats.stepped_into, 1);
        assert!(s.cell_visible(1) && s.cell_visible(2));
        // Room 0 is visited once: the doorway's own plane stops the step back.
        assert_eq!(s.cell_visits(0).len(), 1);
        assert_eq!(s.cached_planes(&s.cell_visits(1)[0]).len(), 2);
    }

    #[test]
    fn forced_invisible_cell_is_not_added() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let bits = [0b10u32];
        let mut p = params([5.0, 0.0, 0.0], &LOOK_X);
        p.force_invisible = Some(&bits);
        let stats = visit_portals(&w, 0, &p, &mut s);
        assert!(!s.cell_visible(1) && s.cell_visible(2));
        assert_eq!(stats.forced_invisible, 1);
    }

    #[test]
    fn walk_limit_stops_the_queue() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let mut p = params([5.0, 0.0, 0.0], &LOOK_X);
        p.walk_limit = 1;
        let stats = visit_portals(&w, 0, &p, &mut s);
        assert!(s.cell_visible(1) && !s.cell_visible(2));
        assert_eq!(stats.walk_limit_hit, 1);
    }

    #[test]
    fn nothing_cached_means_every_box_is_visible() {
        let (cells, portals, vertex_points) = rooms(1);
        let w = world(&cells, &portals, &vertex_points);
        let s = TraversalScratchpad::default();
        assert!(s.box_is_visible(&w, [100.0; 3], [101.0; 3]));
    }

    #[test]
    fn occluder_hides_a_doorway() {
        let (cells, portals, vertex_points) = rooms(2);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        // A box from x 9 to 11 around the doorway, planes facing in.
        let occluder = [[
            [1.0, 0.0, 0.0, -9.0],
            [-1.0, 0.0, 0.0, 11.0],
            [0.0, 1.0, 0.0, 3.0],
            [0.0, -1.0, 0.0, 3.0],
            [0.0, 0.0, 1.0, 3.0],
        ]];
        let mut p = params([5.0, 0.0, 0.0], &LOOK_X);
        p.occluders = &occluder;
        let stats = visit_portals(&w, 0, &p, &mut s);
        assert!(!s.cell_visible(1));
        assert_eq!(stats.skip_occluded, 1);
    }

    #[test]
    fn caster_walk_runs_down_the_hall_toward_the_sun() {
        let (cells, portals, vertex_points) = rooms(3);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let stats = crawl_portals_unbounded(&w, 0, [1.0, 0.0, 0.0], &mut s);
        assert!(s.cell_visible(0) && s.cell_visible(1) && s.cell_visible(2));
        assert_eq!(stats.stepped_into, 0);
        assert_eq!(stats.popped, 2);
        crawl_portals_unbounded(&w, 2, [1.0, 0.0, 0.0], &mut s);
        assert!(s.cell_visible(2) && !s.cell_visible(1));
    }

    #[test]
    fn sun_follows_a_portal_it_grazes_from_in_front() {
        // The sun lies down -z and a little toward -x: the view origin stands 0.0995 in front
        // of the doorway's plane, inside BO2's 0.125 (MW2's facing test drops this portal).
        let (cells, portals, vertex_points) = rooms(2);
        let w = world(&cells, &portals, &vertex_points);
        let mut s = TraversalScratchpad::default();
        let k = 1.0 / libm::sqrtf(1.01);
        crawl_portals_unbounded(&w, 0, [-0.1 * k, 0.0, -k], &mut s);
        assert!(s.cell_visible(1));
        // Past 0.125 it is not followed.
        let k = 1.0 / libm::sqrtf(1.04);
        crawl_portals_unbounded(&w, 0, [-0.2 * k, 0.0, -k], &mut s);
        assert!(!s.cell_visible(1));
    }
}
