//! World visibility, once a frame: which cells the eye sees through the
//! portals, which of their surfaces and static models are inside the view
//! frustum, and the draw items that result.

use std::ops::Range;

use asset_world::CameraSpanKind;
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use dpvs_t6::{
    AabbHideStats, BoxTreeVisibilityWalk, WorldFaceRun, CellClip, SortedDraw,
    LevelTreeFacetDrawCensus, VisibilityBitView, VisibilityCellPlanes, VisibilityResultSet,
    queue_world_faces_for_view, admit_box_tree_surfaces_inside_frustum, cell_caster_bitset_width,
    merge_caster_rows_of_visible, top_bit_get, prop_beyond_cull_range, word_span,
};

use crate::assemble::draw_items::list::{CONTENT_KEY_SEED, blend_content_id, fold_content_bytes};
use crate::prepare::scene::eye_parms::{FrozenVisibilitySet, StagedStageEye};
use crate::{
    LevelDrawItem, LevelDrawItemKind, LevelHide, LevelSight, LevelStage,
};

/// What the last visibility pass saw and produced. The plain fields are read
/// by the draw-surf builders and the render diagnostics; `view`, `aabb`,
/// `triangle_split` and the two portal/eye counters only feed this file's
/// log line.
#[derive(Resource, Clone, Debug, Default)]
pub struct SightFrameStats {
    pub cells_seen: u32,
    pub bsp_shown: u32,
    pub bsp_accepted: u32,
    pub bsp_runs: u32,
    pub bsp_input_gaps: u32,
    pub bsp_unranged: u32,
    pub keys: u32,
    pub rebinds: u32,
    pub surfaces: u32,
    pub triangles: u32,
    pub unhidden_triangles: u32,
    pub batches_sent: u32,
    pub index_bytes_rewritten: u64,
    pub vis_flips: u32,

    /// Per static model: 1 if visible.
    pub prop_vis: Vec<u8>,
    /// Content hash of `prop_vis`.
    pub prop_vis_id: u64,
    /// World surfaces that were visible but had no draw-surf word.
    pub g0_level_facets: Vec<u16>,
    pub view_volume_planes: Vec<[f32; 4]>,
    /// Bit set of visible cells (`room_vis_tally` cells).
    pub room_vis: Vec<u32>,
    pub room_vis_tally: usize,
    /// The clip planes each visible cell was first reached through.
    pub room_clips: Vec<CellClip>,

    /// How the view of this frame stood; reset with the products.
    pub view: CullViewFacts,
    /// Bounds-tree counters of the camera pass.
    pub aabb: AabbHideStats,
    /// Drawn triangles: (lightmapped batches, fallback batches).
    pub triangle_split: (u32, u32),
    /// Portals the walk dropped because they lay outside the frustum.
    pub portal_clip_skips: u32,
    /// Frames, all told, in which the eye stood in no cell.
    pub eyeless_frames: u32,
}

/// The view a visibility pass culled against.
#[derive(Clone, Copy, Debug, Default)]
pub struct CullViewFacts {
    /// The frame had a prepared view.
    pub prepared: bool,
    /// The PVS lock held the culling view still.
    pub pvs_locked: bool,
    /// The cell holding the eye; None outside the world or in a script shot.
    pub eye_room: Option<usize>,
    /// Every cell counted as visible.
    pub every_cell: bool,
    /// Only the eye's own cell was shown (`BO2MP_SINGLE_CELL`).
    pub only_eye_cell: bool,
    pub width_per_height: f32,
    pub fov_degrees: f32,
}

#[derive(Component)]
pub struct PropEntity;

#[derive(Component)]
pub struct ScriptedPropEntity;

pub use render_scene::DynamicPropModelEntity;

/// Tag of a `script_model` game object the map's scripts left in place.
#[derive(Component)]
pub struct ScriptModelActorItem(pub String);

/// A yes-word (`1`, `true`, `yes`) in environment variable `name`.
fn env_says_yes(name: &str) -> bool {
    std::env::var(name).is_ok_and(|word| ["1", "true", "yes"].contains(&word.as_str()))
}

/// True when the static model in `slot` is too far from `eye` to draw.
/// Without an eye nothing is skipped; a slot with no distance never hides.
pub(crate) fn prop_hide_dist_skips_slot(
    hide_dists: &[u16],
    seat: usize,
    pos: [f32; 3],
    eye_pos: Option<Vec3>,
    last_scale: Option<f32>,
) -> bool {
    eye_pos.is_some_and(|eye| {
        let limit = hide_dists.get(seat).copied().unwrap_or_default();
        prop_beyond_cull_range(limit, eye.distance_squared(Vec3::from(pos)), last_scale)
    })
}

/// How the linked portals look from `eye` (`render_frame::portal_eyes`).
fn portal_looks(eye: Vec3) -> Vec<render_frame::portal_eyes::PortalLook> {
    use render_frame::portal_eyes::PortalEnd;
    let ends: Vec<PortalEnd> = sim::guest_arms::portals()
        .iter()
        .map(|p| PortalEnd {
            centre: Vec3::from_array(p.centre),
            normal: Vec3::from_array(p.face_dir).normalize_or(Vec3::Z),
            up: Vec3::from_array(p.up_dir).normalize_or(Vec3::Y),
            orange: p.orange,
            linked: p.paired,
            pair: p.pair,
        })
        .collect();
    render_frame::portal_eyes::choose_looks(&ends, eye)
}

/// The walk's view of the world's cells: one `dpvs_t6::Cell` per cell, its
/// portals laid out back to back. Portals with no hull axes (a world in the
/// older format) are left out.
fn walk_cells(sight: &LevelSight) -> (Vec<dpvs_t6::Cell>, Vec<dpvs_t6::Portal>) {
    let mut portals: Vec<dpvs_t6::Portal> = Vec::new();
    let mut cells: Vec<dpvs_t6::Cell> = (0..sight.cells_n)
        .map_while(|cell| sight.portals_per_room.get(cell))
        .map(|own| {
            let portal_start = portals.len() as u32;
            let usable = own
                .iter()
                .filter_map(|door| Some((door, door.hull_axes_dir?)));
            for (door, hull_axes_dir) in usable {
                portals.push(dpvs_t6::Portal {
                    hull_axes_dir,
                    cell: door.adjacent_cell,
                    plane: door.split,
                    vertex_first: door.vertex_first as u32,
                    vertex_tally: door.vertex_tally as u32,
                });
            }
            dpvs_t6::Cell {
                portal_start,
                portal_opening_total: portals.len() as u32 - portal_start,
                ..Default::default()
            }
        })
        .collect();
    cells.resize(sight.cells_n, dpvs_t6::Cell::default());
    (cells, portals)
}

/// bo2zm: BO2's portal walk (`dpvs_t6`) from `start_cell`. Sets the bit of
/// every cell the walk reached and returns each cell's clip planes (those of
/// its first visit; empty for cells not reached).
fn t6_walk(
    sight: &LevelSight,
    start_cell: usize,
    params: &dpvs_t6::WalkParams<'_>,
    seen: &mut VisibilityBitView<'_>,
) -> (Vec<CellClip>, dpvs_t6::TraversalTally) {
    let (cells, portals) = walk_cells(sight);
    let plane_rows: Vec<[f32; 4]> = sight
        .plane_set
        .iter()
        .map(|p| {
            let [x, y, z] = p.normal;
            [x, y, z, p.offset]
        })
        .collect();
    let mut work_buf = dpvs_t6::TraversalScratchpad::default();
    let walked = dpvs_t6::visit_portals(
        &dpvs_t6::DpvsWorld {
            cells: &cells,
            portals: &portals,
            vertex_points: &sight.portal_vertices,
            planes: &plane_rows,
            nodes: sight.node_list.as_slice(),
        },
        start_cell,
        params,
        &mut work_buf,
    );
    let clips = (0..sight.cells_n)
        .map(|room| match work_buf.cell_visible(room) {
            false => CellClip::BLANK,
            true => {
                seen.set(room);
                work_buf.first_visit_clip(room).unwrap_or(CellClip::BLANK)
            }
        })
        .collect();
    (clips, walked)
}

/// The cull input of bounds tree `tree` (one per cell, then some for
/// submodels), or None past the last tree.
fn tree_input(sight: &LevelSight, tree: usize) -> Option<BoxTreeVisibilityWalk<'_>> {
    let nodes = sight.box_trees.get(tree)?;
    let prop_indexes = sight
        .aabb_prop_indices
        .get(tree)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Some(BoxTreeVisibilityWalk {
        nodes,
        prop_indexes,
        sorted_facet_index: &sight.sorted_facet_index,
        face_spans: &sight.facet_bounds,
        prop_spans: sight.prop_bounds.as_slice(),
    })
}

/// Marks what the bounds trees `trees` hold inside `frustum`.
fn gather_trees(
    sight: &LevelSight,
    trees: impl IntoIterator<Item = usize>,
    cut_planes: &[[f32; 4]],
    marks: &mut VisibilityResultSet<'_>,
    counters: &mut AabbHideStats,
) {
    for input in trees.into_iter().filter_map(|tree| tree_input(sight, tree)) {
        admit_box_tree_surfaces_inside_frustum(&input, cut_planes, marks, counters);
    }
}

/// The trees past the last cell hold the surfaces no cell lists (submodels).
fn submodel_trees(sight: &LevelSight) -> Range<usize> {
    sight.cells_n..sight.box_trees.len()
}

/// True while the local player is in a script-driven shot (the match's
/// intermission).
fn in_script_camera(
    snapshot: Option<&net::ShownWorldFrame>,
    client: Option<&net::HostViewClient>,
) -> bool {
    let (Some(snapshot), Some(client)) = (snapshot, client) else {
        return false;
    };
    snapshot
        .player(client.0)
        .is_some_and(|ps| ps.movement_mode == playerstate_t6::game::MOVE_MODE_INTERLUDE)
}

/// `BO2MP_DPVS_LOG=1`: once a second, where the eye is and whether a script
/// camera looks.
fn log_eye_cell(eye: [f32; 3], cell: Option<usize>, script_camera: bool) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LOGGED_SECOND: AtomicU64 = AtomicU64::new(0);
    if std::env::var_os("BO2MP_DPVS_LOG").is_none() {
        return;
    }
    let now = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |since| since.as_secs());
    if LOGGED_SECOND.swap(now, Ordering::Relaxed) == now {
        return;
    }
    let [x, y, z] = eye;
    diag::info!(
        World,
        "bo2zm dpvs: eye ({x:.0} {y:.0} {z:.0}) cell {cell:?} script camera {script_camera}"
    );
}

/// How far `eye` is from the box `b` (0 inside it).
fn distance_to_box(eye: [f32; 3], b: dpvs_t6::Bounds) -> f32 {
    let (mid, half) = (b.mid(), b.half());
    let out: [f32; 3] = core::array::from_fn(|k| ((eye[k] - mid[k]).abs() - half[k]).max(0.0));
    (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt()
}

/// `BO2MP_WALL_LOG=1`: twice a second, the map surfaces within 256 units of
/// the eye that lie in the view but the portal walk left out (marked by a cull
/// over every cell, not by the walk's cells), and how far the eye is in front
/// of the near plane. A wall that vanishes up close with none dropped here was
/// lost later (near plane or draw); one counted here was lost to the walk.
fn log_near_walls_dropped(
    sight: &LevelSight,
    view: &CullView,
    reach: &CellReach,
    walked: &VisibilityResultSet<'_>,
    portal_looks: usize,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LOGGED_AT: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |since| since.as_millis() as u64 / 500);
    if LOGGED_AT.swap(now, Ordering::Relaxed) == now {
        return;
    }
    let mut facet_vis = vec![0u8; walked.facet_vis.len()];
    let mut prop_vis = vec![0u8; walked.prop_vis.len()];
    let mut all = VisibilityResultSet { facet_vis: &mut facet_vis, prop_vis: &mut prop_vis };
    let mut ignored = AabbHideStats::default();
    gather_trees(sight, (0..sight.cells_n).chain(submodel_trees(sight)), &view.clip_planes, &mut all, &mut ignored);
    let eye = view.eye.to_array();
    let (mut near, mut dropped, mut closest) = (0u32, Vec::new(), f32::MAX);
    // The closest flat surface (a box under 2 units thick), seen or not.
    let mut nearest_flat = f32::MAX;
    for (facet, bounds) in sight.facet_bounds.iter().enumerate() {
        let d = distance_to_box(eye, *bounds);
        if bounds.half().iter().any(|&h| h < 1.0) {
            nearest_flat = nearest_flat.min(d);
        }
        if d > 256.0 || all.facet_vis.get(facet).copied().unwrap_or(0) == 0 {
            continue;
        }
        near += 1;
        if walked.facet_vis.get(facet).copied().unwrap_or(0) == 0 {
            closest = closest.min(d);
            dropped.push((facet, d));
        }
    }
    dropped.sort_by(|a, b| a.1.total_cmp(&b.1));
    // Which cell's tree holds each of the closest dropped surfaces.
    let owner_of = |facet: usize| {
        (0..sight.cells_n).find(|&cell| {
            let mut one = vec![0u8; walked.facet_vis.len()];
            let mut none = vec![0u8; walked.prop_vis.len()];
            let mut out = VisibilityResultSet { facet_vis: &mut one, prop_vis: &mut none };
            gather_trees(sight, std::iter::once(cell), &view.clip_planes, &mut out, &mut AabbHideStats::default());
            out.facet_vis.get(facet).copied().unwrap_or(0) != 0
        })
    };
    // The eye cell's doorways: the cell beyond, the doorway plane's n.eye and d, its corners.
    let doors: Vec<String> = reach
        .eye_room
        .and_then(|room| sight.portals_per_room.get(room))
        .map(|own| {
            own.iter()
                .map(|door| {
                    let [a, b, c, d] = door.split;
                    let along = a * eye[0] + b * eye[1] + c * eye[2];
                    let corners = &sight.portal_vertices[door.vertex_first..door.vertex_first + door.vertex_tally];
                    let mid = corners.iter().fold([0.0f32; 3], |m, v| [m[0] + v[0], m[1] + v[1], m[2] + v[2]]);
                    let n = corners.len().max(1) as f32;
                    format!(
                        "to {} n.eye {along:.0} d {d:.0} mid ({:.0} {:.0} {:.0}) corners {}",
                        door.adjacent_cell,
                        mid[0] / n,
                        mid[1] / n,
                        mid[2] / n,
                        corners.len()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let near_plane = view.clip_planes.first().map_or(f32::NAN, |p| p[0] * eye[0] + p[1] * eye[1] + p[2] * eye[2] + p[3]);
    let cells_seen = reach.bits.iter().map(|w| w.count_ones()).sum::<u32>();
    let [x, y, z] = eye;
    let rays = rays_through_doors(sight, view, reach);
    let sample: Vec<String> = dropped
        .iter()
        .take(4)
        .map(|&(facet, d)| {
            let b = sight.facet_bounds[facet];
            let (m, h) = (b.mid(), b.half());
            format!(
                "#{facet} {d:.0} away in cell {:?} at ({:.0} {:.0} {:.0}) half ({:.0} {:.0} {:.0})",
                owner_of(facet),
                m[0],
                m[1],
                m[2],
                h[0],
                h[1],
                h[2]
            )
        })
        .collect();
    diag::info!(
        World,
        "bo2mp walls: eye ({x:.0} {y:.0} {z:.0}) cell {:?} cells seen {cells_seen}/{} portal looks {portal_looks}; near surfaces in view {near}, walk dropped {} (closest {:.0}); eye to near plane {near_plane:.2}, nearest flat surface {nearest_flat:.1}; {}; doors {}; {} rays reach {} cells, unwalked [{}]",
        reach.eye_room,
        sight.cells_n,
        dropped.len(),
        if dropped.is_empty() { 0.0 } else { closest },
        sample.join(", "),
        doors.join(" | "),
        rays.0,
        rays.1,
        rays.2.join(", ")
    );
}

/// Rays from the eye across the view, out to 512 units: a ray goes from cell to cell only
/// through a doorway between them and stops at a wall. Returns each cell a ray reaches
/// that the walk did not mark, with the doorway it came through and how far away.
fn rays_through_doors(sight: &LevelSight, view: &CullView, reach: &CellReach) -> (usize, usize, Vec<String>) {
    let Some(start) = reach.eye_room else {
        return (0, 0, Vec::new());
    };
    let finder = VisibilityCellPlanes {
        leaf_cells: sight.cells_n as u32,
        nodes: sight.node_list.as_slice(),
        planes: sight.plane_set.as_slice(),
    };
    let eye = view.eye.to_array();
    let walked = |cell: usize| reach.bits.get(cell / 32).is_some_and(|w| w & (1 << (cell % 32)) != 0);
    let mut missed: Vec<(usize, usize, f32, [f32; 3])> = Vec::new();
    let (mut rays, mut reached) = (0usize, vec![start]);
    const RAYS: usize = 6000;
    for i in 0..RAYS {
        // Fibonacci sphere; keep the directions inside the view.
        let z = 1.0 - 2.0 * (i as f32 + 0.5) / RAYS as f32;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let a = i as f32 * 2.399_963;
        let dir = [r * a.cos(), r * a.sin(), z];
        let probe = [eye[0] + dir[0] * 16.0, eye[1] + dir[1] * 16.0, eye[2] + dir[2] * 16.0];
        if view.clip_planes.iter().any(|p| p[0] * probe[0] + p[1] * probe[1] + p[2] * probe[2] + p[3] < 0.0) {
            continue;
        }
        rays += 1;
        let mut cell = start;
        let mut t = 0.0f32;
        while t < 512.0 {
            let t1 = t + 2.0;
            let at = [eye[0] + dir[0] * t1, eye[1] + dir[1] * t1, eye[2] + dir[2] * t1];
            let Some(next) = finder.cell_holding(at) else {
                break;
            };
            if next != cell {
                let door = sight.portals_per_room.get(cell).and_then(|own| {
                    own.iter().find(|door| {
                        usize::from(door.adjacent_cell) == next
                            && ray_meets_door(eye, dir, door.split, &sight.portal_vertices[door.vertex_first..door.vertex_first + door.vertex_tally], t - 2.0, t1 + 2.0)
                    })
                });
                let Some(door) = door else {
                    break;
                };
                if !walked(next) && !missed.iter().any(|m| m.0 == next) {
                    let corners = &sight.portal_vertices[door.vertex_first..door.vertex_first + door.vertex_tally];
                    let mid = corners.iter().fold([0.0f32; 3], |m, v| [m[0] + v[0], m[1] + v[1], m[2] + v[2]]);
                    let n = corners.len().max(1) as f32;
                    missed.push((next, cell, t1, [mid[0] / n, mid[1] / n, mid[2] / n]));
                }
                if !reached.contains(&next) {
                    reached.push(next);
                }
                cell = next;
            }
            t = t1;
        }
    }
    let missed = missed
        .iter()
        .map(|(to, from, t, m)| format!("{to} via door {from}->{to} at ({:.0} {:.0} {:.0}) {t:.0} away", m[0], m[1], m[2]))
        .collect();
    (rays, reached.len(), missed)
}

/// Whether the ray `eye + dir * t`, for t in `lo..hi`, passes through the doorway `corners`
/// on `split` (1 unit of slack at the edges).
fn ray_meets_door(eye: [f32; 3], dir: [f32; 3], split: [f32; 4], corners: &[[f32; 3]], lo: f32, hi: f32) -> bool {
    let [a, b, c, d] = split;
    let along = a * dir[0] + b * dir[1] + c * dir[2];
    if along.abs() < 1e-6 || corners.len() < 3 {
        return false;
    }
    let t = -(a * eye[0] + b * eye[1] + c * eye[2] + d) / along;
    if t < lo || t > hi {
        return false;
    }
    let hit = [eye[0] + dir[0] * t, eye[1] + dir[1] * t, eye[2] + dir[2] * t];
    let n = [a, b, c];
    let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let cross = |p: [f32; 3], q: [f32; 3]| [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]];
    let dot = |p: [f32; 3], q: [f32; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let mut sign = 0.0f32;
    for (i, &v) in corners.iter().enumerate() {
        let w = corners[(i + 1) % corners.len()];
        let out = cross(n, sub(w, v));
        let len = dot(out, out).sqrt().max(1e-6);
        let side = dot(out, sub(hit, v)) / len;
        if sign == 0.0 {
            sign = if dot(out, sub(corners[(i + 2) % corners.len()], v)) >= 0.0 { 1.0 } else { -1.0 };
        }
        if side * sign < -1.0 {
            return false;
        }
    }
    true
}

/// Forgets the last frame's products; the counters stay.
fn clear_products(stats: &mut SightFrameStats) {
    let SightFrameStats {
        prop_vis,
        prop_vis_id,
        view_volume_planes,
        room_vis,
        room_vis_tally,
        room_clips,
        view,
        ..
    } = stats;
    prop_vis.clear();
    view_volume_planes.clear();
    room_vis.clear();
    room_clips.clear();
    *prop_vis_id = 0;
    *room_vis_tally = 0;
    *view = CullViewFacts::default();
}

/// The brush model a script mover entity shows, if it shows one.
fn shown_brush_model(rt: &net::ClientEntityPoseState) -> Option<u32> {
    use entity_t6::script_mover::{ScriptMoverDraw, script_mover_draw};
    let state = &rt.upcoming_condition;
    let mover = rt.present_in_newer_frame() && state.e_type == entity_t6::et::SCRIPTMOVER;
    // eFlags2 arrives with BO2's EntityState; no demo-hide flag until then.
    let draws_brush = mover
        && script_mover_draw(state.lerp.e_flags, 0, state.solid, false, false)
            == ScriptMoverDraw::BrushModel;
    draws_brush
        .then(|| u32::try_from(state.index).ok())
        .flatten()
}

/// codmp: a Black Ops II submodel is drawn only while a live entity shows
/// it: a brush mover naming it that no script hid. Scripts hide some
/// (Nuketown's display glass) and delete others (Socotra's overwatch floors),
/// so the surfaces of every other submodel are marked invisible.
fn hide_unshown_brush_models(
    brushes: &[asset_world::T6BrushSurfaces],
    facet_vis: &mut [u8],
    entities: &Query<&net::ClientEntityPoseState>,
) {
    if brushes.is_empty() {
        return;
    }
    let shown: HashSet<u32> = entities.iter().filter_map(shown_brush_model).collect();
    let end = facet_vis.len();
    let clamp = |at: usize| at.min(end);
    for hidden in brushes.iter().filter(|b| !shown.contains(&b.solid_model)) {
        let from = clamp(hidden.lead_skin as usize);
        let to = clamp(from + hidden.facet_tally as usize);
        facet_vis[from..to].fill(0);
    }
}

/// What the finished draw items add up to.
#[derive(Default)]
struct DrawTally {
    surfaces: u32,
    lightmapped_tris: u32,
    fallback_tris: u32,
    /// Batches at least one drawn surface belongs to.
    batches_touched: u32,
}

/// The hash of the draw items: their count, then each item's identity.
fn draw_items_content_id(cull: &LevelHide) -> u64 {
    let mut id = CONTENT_KEY_SEED;
    blend_content_id(&mut id, cull.draw_batch_list.len() as u64);
    cull.draw_batch_list
        .iter()
        .for_each(|item| mix_draw_item_id(&mut id, item, cull.draw_item_surfs(item)));
    id
}

/// Counts the surfaces and triangles `cull.draw_batch_list` draw and the batches
/// they touch. Surfaces with no batch range or no indices are not counted.
fn tally(cull: &LevelHide) -> DrawTally {
    let mut touched = vec![false; cull.batches_n as usize];
    let mut sum = DrawTally::default();
    let drawn = cull
        .draw_batch_list
        .iter()
        .flat_map(|item| cull.draw_item_surfs(item))
        .filter_map(|&surf| cull.facet_batch_ranges.get(usize::from(surf)))
        .filter(|&&(_, _, index_count)| index_count != 0);
    for &(batch, _, index_count) in drawn {
        sum.surfaces = sum.surfaces.saturating_add(1);
        if let Some(flag) = touched.get_mut(batch) {
            *flag = true;
        }
        let lit = cull.batch_lightbaked.get(batch).copied().unwrap_or(false);
        let lane = if lit {
            &mut sum.lightmapped_tris
        } else {
            &mut sum.fallback_tris
        };
        *lane = lane.saturating_add(index_count / 3);
    }
    sum.batches_touched = touched.into_iter().filter(|&t| t).count() as u32;
    sum
}

/// The view the cull runs against, read once (the PVS lock may hold it).
struct CullView {
    eye: Vec3,
    forward: Vec3,
    clip_planes: Vec<[f32; 4]>,
    /// The camera's planes as the start cell tests portals: the near plane
    /// moved back to the eye (see `start_planes`).
    start_planes: Vec<[f32; 4]>,
    bevels: Option<dpvs_t6::Bevels>,
    fov: f32,
    width_per_height: f32,
}

impl CullView {
    fn read(lock: &FrozenVisibilitySet, live: &StagedStageEye) -> Self {
        let (fov, width_per_height) = lock.walk_lens(live);
        let (eye, forward) = (lock.walk_eye(live), lock.walk_forward(live));
        let clip_planes = lock.walk_planes(live).to_vec();
        Self {
            eye,
            forward,
            start_planes: start_planes(&clip_planes, eye, forward),
            clip_planes,
            bevels: lock.walk_bevels(live),
            fov,
            width_per_height,
        }
    }

    /// The portal walk's parameters for this view.
    fn walk_params(&self) -> dpvs_t6::WalkParams<'_> {
        let eye = self.eye.to_array();
        dpvs_t6::WalkParams {
            eye,
            view_plane: dpvs_t6::facing_plane(self.forward.to_array(), eye),
            planes: &self.start_planes,
            near: self.clip_planes.first().copied(),
            far: self.clip_planes.get(1).copied(),
            walk_limit: 0,
            bevels: self.bevels.as_ref(),
            force_invisible: None,
            occluders: &[],
        }
    }
}

/// bo2mp: the start cell's portal planes with the near plane (`r_znear`
/// ahead) swapped for the plane through the eye. Against the near plane a
/// doorway less than that far ahead lies wholly behind it and the room past
/// it went undrawn for that frame (frostbite's cheese shop, 10-29 clip at
/// 73 s). Children still end with the near plane pulled back to their portal.
fn start_planes(clip_planes: &[[f32; 4]], eye: Vec3, forward: Vec3) -> Vec<[f32; 4]> {
    let mut planes = clip_planes.to_vec();
    if let Some(first) = planes.first_mut() {
        *first = dpvs_t6::facing_plane(forward.to_array(), eye.to_array());
    }
    planes
}

/// Which cells the eye reaches, and how.
struct CellReach {
    eye_room: Option<usize>,
    bits: Vec<u32>,
    clips: Option<Vec<CellClip>>,
    clip_skips: u32,
}

impl CellReach {
    fn visible_cells(&mut self, cell_count: usize) -> Vec<usize> {
        let bits = VisibilityBitView::new(&mut self.bits);
        (0..cell_count).filter(|&cell| bits.get(cell)).collect()
    }

    fn mark_all(&mut self, cells_n: usize) {
        let mut bits = VisibilityBitView::new(&mut self.bits);
        (0..cells_n).for_each(|cell| bits.set(cell));
    }
}

/// Finds the eye's cell and walks the portals from it. A script camera or an
/// eye in no cell sees every cell; `only_eye_cell` keeps just the eye's.
fn reach_cells(
    sight: &LevelSight,
    view: &CullView,
    script_camera: bool,
    only_eye_cell: bool,
) -> CellReach {
    let eye_at = view.eye.to_array();
    let finder = VisibilityCellPlanes {
        leaf_cells: sight.cells_n as u32,
        nodes: sight.node_list.as_slice(),
        planes: sight.plane_set.as_slice(),
    };
    let located = finder.cell_holding(eye_at);
    log_eye_cell(eye_at, located, script_camera);

    let mut reach = CellReach {
        eye_room: located.filter(|_| !script_camera),
        bits: vec![0; word_span(sight.cells_n.max(1))],
        clips: None,
        clip_skips: 0,
    };
    let Some(start) = reach.eye_room else {
        reach.mark_all(sight.cells_n);
        return reach;
    };
    let mut bits = VisibilityBitView::new(&mut reach.bits);
    if only_eye_cell {
        bits.set(start);
    } else {
        let (clips, walked) = t6_walk(sight, start, &view.walk_params(), &mut bits);
        reach.clips = Some(clips);
        reach.clip_skips = walked.skipped_by_clip_total;
    }
    reach
}

/// The view frustum seen from behind each linked portal's far end, its near
/// plane swapped for the exit plane.
fn look_frustums(
    cut_planes: &[[f32; 4]],
    looks: &[render_frame::portal_eyes::PortalLook],
) -> Vec<Vec<[f32; 4]>> {
    looks
        .iter()
        .map(|look| {
            let mut moved = render_frame::portal_eyes::move_planes(cut_planes, &look.through);
            if let Some(first) = moved.first_mut() {
                *first = look.exit_plane.to_array();
            }
            moved
        })
        .collect()
}

/// Adds field by field, saturating; `entry_record_total` is not summed.
fn add_census(total: &mut LevelTreeFacetDrawCensus, part: &LevelTreeFacetDrawCensus) {
    for (sum, add) in [
        (&mut total.span_total, part.span_total),
        (&mut total.seen_total, part.seen_total),
        (&mut total.accepted_total, part.accepted_total),
        (&mut total.runs_seen, part.runs_seen),
        (&mut total.input_gap_tally, part.input_gap_tally),
    ] {
        *sum = sum.saturating_add(add);
    }
}

/// Empties `list` and refills it with `zeroes` zero bytes.
fn zero_fill(list: &mut Vec<u8>, zeroes: usize) {
    list.clear();
    list.resize(zeroes, 0);
}

/// Empties the lists a cull pass fills.
fn start_frame_lists(cull: &mut LevelHide) {
    let facet_tally = cull.facet_batch_ranges.len();
    let model_count = cull.prop_entities.len();
    zero_fill(&mut cull.facet_vis, facet_tally);
    zero_fill(&mut cull.prop_vis, model_count);
    for surfs in [&mut cull.item_surfs, &mut cull.g0_facets] {
        surfs.clear();
    }
    cull.draw_batch_list.clear();
}

/// What `apply_sight_hide` reads and writes.
#[derive(SystemParam)]
pub struct DpvsCullFrame<'w, 's> {
    stage: ResMut<'w, LevelStage>,
    stats: ResMut<'w, SightFrameStats>,
    published: ResMut<'w, render_scene::ExportedCellVisibility>,
    live_view: Res<'w, StagedStageEye>,
    pvs_lock: Res<'w, FrozenVisibilitySet>,
    snapshot: Option<Res<'w, net::ShownWorldFrame>>,
    viewer_seat: Option<Res<'w, net::HostViewClient>>,
    movers: Query<'w, 's, &'static net::ClientEntityPoseState>,
}

pub fn apply_sight_hide(frame: DpvsCullFrame) {
    let _span = perf::Span::DrawHideCpuMillis.enter();
    let DpvsCullFrame {
        stage: mut scene,
        mut stats,
        mut published,
        live_view,
        pvs_lock,
        snapshot,
        viewer_seat: own_client,
        movers,
    } = frame;
    // bo2zm M3: a script camera (the intermission shot of Nuketown's
    // game-over rocket flying down from the sky) sees every cell, as from
    // outside the world: from high over the map no portal leads to the town.
    let script_camera = in_script_camera(snapshot.as_deref(), own_client.as_deref());

    clear_products(&mut stats);
    *published = Default::default();
    render_frame::portal_eyes::set_looks(Vec::new());

    let has_draws = |c: &&mut LevelHide| c.batches_n != 0 || !c.prop_entities.is_empty();
    let Some(cull) = scene.culling.as_mut().filter(has_draws) else {
        return;
    };
    if !live_view.primed {
        return;
    }
    let stats = &mut *stats;
    stats.view.prepared = true;
    stats.view.pvs_locked = pvs_lock.enabled && pvs_lock.frozen.is_some();

    let view = CullView::read(&pvs_lock, &live_view);
    if cull.dpvs.cells_n == 0 {
        return;
    }
    stats.view_volume_planes.clone_from(&view.clip_planes);

    let only_eye_cell = env_says_yes("BO2MP_SINGLE_CELL");
    let mut reach = reach_cells(&cull.dpvs, &view, script_camera, only_eye_cell);
    if let Some(clips) = reach.clips.take() {
        stats.room_clips = clips;
    }
    start_frame_lists(cull);
    stats.g0_level_facets.clear();

    // What an eye behind the other end of each linked portal would see.
    let looks = match script_camera {
        true => Vec::new(),
        false => portal_looks(view.eye),
    };
    let through_portals = look_frustums(&view.clip_planes, &looks);

    let sight = &cull.dpvs;
    let mut aabb = AabbHideStats::default();
    let mut marks = VisibilityResultSet {
        facet_vis: &mut cull.facet_vis,
        prop_vis: &mut cull.prop_vis,
    };
    let in_view = reach.visible_cells(sight.cells_n);
    let camera_trees = in_view.into_iter().chain(submodel_trees(sight));
    gather_trees(
        sight,
        camera_trees,
        &view.clip_planes,
        &mut marks,
        &mut aabb,
    );
    if env_says_yes("BO2MP_WALL_LOG") {
        log_near_walls_dropped(sight, &view, &reach, &marks, through_portals.len());
    }
    // That eye stands behind a wall, outside the cells the walk starts
    // from, so the walk would drop the ground right past the exit. Every
    // cell is offered to the moved frustum instead; it culls by bounds.
    if !through_portals.is_empty() {
        reach.mark_all(sight.cells_n);
    }
    for moved in &through_portals {
        let every_tree = (0..sight.cells_n).chain(submodel_trees(sight));
        gather_trees(sight, every_tree, moved, &mut marks, &mut aabb);
    }
    render_frame::portal_eyes::set_looks(looks);

    hide_unshown_brush_models(&cull.t6_brush_surfaces, &mut cull.facet_vis, &movers);

    // The camera's draw items, one range kind at a time.
    let mut census = LevelTreeFacetDrawCensus::default();
    let mut out = CameraOut {
        vis: &mut cull.facet_vis,
        word_list: &cull.capture.compact_facet_draws,
        runs: &mut cull.level_tree_run_scratch,
        item_surfs: &mut cull.item_surfs,
        items: &mut cull.draw_batch_list,
        unworded: &mut cull.g0_facets,
    };
    for range in sight.camera_spans.iter() {
        add_census(&mut census, &out.append(range.kind, range.from..range.until));
    }

    let sum = tally(cull);
    cull.draw_batch_list_id = draw_items_content_id(cull);
    let every_cell = reach.eye_room.is_none();
    cull.room_vis.clone_from(&reach.bits);
    cull.room_vis_all = every_cell;

    perf::Counter::TallySentBatches.emit(f64::from(sum.batches_touched));
    let mut smodel_hash = CONTENT_KEY_SEED;
    fold_content_bytes(&mut smodel_hash, &cull.prop_vis);
    let earlier = std::mem::take(stats);
    *stats = SightFrameStats {
        view: CullViewFacts {
            eye_room: reach.eye_room,
            every_cell,
            only_eye_cell,
            width_per_height: view.width_per_height,
            fov_degrees: view.fov.to_degrees(),
            ..earlier.view
        },
        eyeless_frames: earlier.eyeless_frames.saturating_add(u32::from(every_cell)),
        cells_seen: VisibilityBitView::new(&mut reach.bits).set_total(),
        portal_clip_skips: reach.clip_skips,
        aabb,
        room_vis: reach.bits.clone(),
        room_vis_tally: sight.cells_n,
        bsp_shown: census.seen_total,
        bsp_accepted: census.accepted_total,
        bsp_runs: census.runs_seen,
        bsp_input_gaps: census.input_gap_tally,
        keys: cull.draw_batch_list.len() as u32,
        g0_level_facets: cull.g0_facets.clone(),
        rebinds: draw_item_rebinds(&cull.draw_batch_list),
        surfaces: sum.surfaces,
        triangles: sum.lightmapped_tris.saturating_add(sum.fallback_tris),
        triangle_split: (sum.lightmapped_tris, sum.fallback_tris),
        unhidden_triangles: (cull.compact_indices.len() / 3) as u32,
        batches_sent: sum.batches_touched,
        index_bytes_rewritten: 0,
        vis_flips: 0,
        prop_vis: cull.prop_vis.clone(),
        prop_vis_id: smodel_hash,
        ..earlier
    };
    *published = render_scene::ExportedCellVisibility {
        cells_in: sight.cells_n,
        sight_all: every_cell,
        words: reach.bits,
    };
}

/// True the first time it is asked about `flag`.
fn first_time(flag: &mut bool) -> bool {
    !std::mem::replace(flag, true)
}

/// Logs the tags of the `script_model` props the scripts kept, once.
pub(crate) fn log_script_model_gaps_one_time(
    props: Query<&ScriptModelActorItem>,
    mut logged: Local<bool>,
) {
    if props.is_empty() || !first_time(&mut logged) {
        return;
    }
    let kept = props.iter().count();
    let tags: Vec<&str> = props
        .iter()
        .map(|prop| prop.0.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    diag::info!(
        World,
        "script_model gameobjects after _gameobjects::main: {kept} props kept, tags={tags:?}"
    );
}

/// The once-only culling log line, as `name=value` pairs.
fn dpvs_log_fields(s: &SightFrameStats) -> Vec<(&'static str, String)> {
    let a = &s.aabb;
    let (lit, fallback) = s.triangle_split;
    vec![
        ("eye_cell", format!("{:?}", s.view.eye_room)),
        ("unresolved", s.eyeless_frames.to_string()),
        ("visible_cells", s.cells_seen.to_string()),
        ("portals_frustum_skipped", s.portal_clip_skips.to_string()),
        ("aabb_nodes", a.nodes_walked.to_string()),
        ("planes_dropped", a.planes_discarded.to_string()),
        (
            "surf_rejected_by_bounds",
            a.facets_rejected_by_bounds.to_string(),
        ),
        ("surf_unbounded", a.facets_admitted_unbounded.to_string()),
        (
            "smodel_rejected_by_bounds",
            a.props_rejected_by_bounds.to_string(),
        ),
        ("smodel_unbounded", a.props_admitted_unbounded.to_string()),
        ("bsp_visible", s.bsp_shown.to_string()),
        ("bsp_admitted", s.bsp_accepted.to_string()),
        ("bsp_runs", s.bsp_runs.to_string()),
        ("bsp_input_gap", s.bsp_input_gaps.to_string()),
        (
            "bsp_ranges_unavailable",
            s.bsp_unranged.to_string(),
        ),
        ("keys", s.keys.to_string()),
        // Every input gap is a refused material ordinal.
        ("ordinal_refused", s.bsp_input_gaps.to_string()),
        ("g0_world", s.g0_level_facets.len().to_string()),
        ("rebinds", s.rebinds.to_string()),
        ("surfaces", s.surfaces.to_string()),
        (
            "tris",
            format!(
                "{}/{} (lightmapped={lit} fallback={fallback})",
                s.triangles, s.unhidden_triangles
            ),
        ),
        ("single_cell", s.view.only_eye_cell.to_string()),
        ("batches", s.batches_sent.to_string()),
        ("rewrite_bytes", s.index_bytes_rewritten.to_string()),
        ("vis_chg", s.vis_flips.to_string()),
        ("aspect", format!("{:.3}", s.view.width_per_height)),
        ("fov_deg", format!("{:.1}", s.view.fov_degrees)),
    ]
}

/// Logs the first frame's culling numbers, once.
pub(crate) fn log_sight_stats_once(stats: Res<SightFrameStats>, mut logged: Local<bool>) {
    if stats.unhidden_triangles == 0 || !first_time(&mut logged) {
        return;
    }
    let line = dpvs_log_fields(&stats)
        .into_iter()
        .map(|pair| format!("{}={}", pair.0, pair.1))
        .collect::<Vec<_>>()
        .join(" ");
    diag::info!(World, "dpvs frame: {line}");
}

/// Marks the surfaces a shadow-casting view sees: the cells a camera sees
/// plus the cells that can cast into them, each culled to `frustum`. Returns
/// the cull counters and how many cells were offered. `draw_top_bit` receives the
/// cells offered, as a most-significant-bit-first bit set.
pub(crate) fn add_level_facets_view_volume_only(
    sight: &LevelSight,
    cut_planes: &[[f32; 4]],
    camera_room_vis_lsb: &[u32],
    camera_sight_all: bool,
    facet_vis: &mut [u8],
    prop_vis: &mut [u8],
    draw_top_bit: &mut Vec<u32>,
) -> (AabbHideStats, u32) {
    draw_top_bit.clear();
    draw_top_bit.resize(cell_caster_bitset_width(sight.cells_n), 0);
    // No caster table: every cell may cast.
    let every_cell = sight.room_caster_bits.is_empty();
    let offered = if every_cell {
        sight.cells_n as u32
    } else {
        merge_caster_rows_of_visible(
            &sight.room_caster_bits,
            sight.cells_n,
            camera_room_vis_lsb,
            camera_sight_all,
            draw_top_bit.as_mut_slice(),
        )
    };
    let rows: &[u32] = draw_top_bit;
    let casters = (0..sight.cells_n).filter(|&cell| every_cell || top_bit_get(rows, cell));
    let mut counters = AabbHideStats::default();
    let mut marks = VisibilityResultSet {
        facet_vis,
        prop_vis,
    };
    gather_trees(sight, casters, cut_planes, &mut marks, &mut counters);
    (counters, offered)
}

/// The tables the camera's draw items are built from, and where they go.
struct CameraOut<'a> {
    /// Per world surface: nonzero while visible.
    vis: &'a mut [u8],
    /// Per world surface: its draw-surf word (0 for none).
    word_list: &'a [SortedDraw],
    runs: &'a mut Vec<WorldFaceRun<CameraSpanKind>>,
    item_surfs: &'a mut Vec<u16>,
    items: &'a mut Vec<LevelDrawItem>,
    /// Visible surfaces that had no draw-surf word.
    unworded: &'a mut Vec<u16>,
}

/// The census of a camera range that is wholly an input gap.
fn gap_census(span_total: u32, seen_total: u32) -> LevelTreeFacetDrawCensus {
    LevelTreeFacetDrawCensus {
        span_total,
        seen_total,
        input_gap_tally: span_total.max(1),
        ..Default::default()
    }
}

impl CameraOut<'_> {
    /// The surface slots `span` covers, or, when it does not fit the tables,
    /// the census of the input gap it is.
    fn slots(&self, span: &Range<u32>) -> Result<Range<usize>, LevelTreeFacetDrawCensus> {
        let Some(span_total) = span.end.checked_sub(span.start) else {
            return Err(gap_census(0, 0));
        };
        let (Ok(start), Ok(end)) = (usize::try_from(span.start), usize::try_from(span.end)) else {
            return Err(gap_census(span_total, 0));
        };
        if end <= self.vis.len().min(self.word_list.len()) {
            return Ok(start..end);
        }
        let marked = self
            .vis
            .get(start..end.min(self.vis.len()))
            .map_or(0, |bytes| bytes.iter().filter(|&&b| b != 0).count() as u32);
        Err(gap_census(span_total, marked))
    }

    /// Unmarks the visible surfaces of `slots` that have no draw-surf word
    /// and lists them; returns how many there were.
    fn drop_unworded(&mut self, slots: Range<usize>) -> u32 {
        let first = slots.start;
        let pairs = self.vis[slots.clone()].iter_mut().zip(&self.word_list[slots]);
        let mut dropped = 0u32;
        for (step, (mark, word)) in pairs.enumerate() {
            if *mark == 0 || word.packed != 0 {
                continue;
            }
            *mark = 0;
            dropped = dropped.saturating_add(1);
            self.unworded.extend(u16::try_from(first + step).ok());
        }
        dropped
    }

    /// Turns the runs of the last range into draw items.
    fn push_run_items(&mut self) {
        let mut entry = 0u16;
        let surfs: &[u16] = self.item_surfs;
        self.items.extend(self.runs.iter().map(|run| {
            let head = surfs[run.list_start as usize];
            // A run that extends the entry before it keeps that entry's head.
            entry = if run.extends_entry { entry } else { head };
            LevelDrawItem {
                key: run.facet_draw.packed,
                facet_ix: head,
                batch_run: run.len,
                list_start: run.list_start,
                entry_first: entry,
                kind: LevelDrawItemKind::Bsp(run.kind),
            }
        }));
    }

    /// Turns one camera surface range into draw items. Surfaces marked
    /// visible but lacking a draw-surf word are unmarked and listed in
    /// `unworded`. A range that does not fit the tables counts as an input
    /// gap and adds no items.
    fn append(&mut self, kind: CameraSpanKind, span: Range<u32>) -> LevelTreeFacetDrawCensus {
        let slots = match self.slots(&span) {
            Ok(slots) => slots,
            Err(gap) => return gap,
        };
        let unworded = self.drop_unworded(slots);
        self.runs.clear();
        let (first, last) = (span.start, span.end);
        let mut census = queue_world_faces_for_view(
            kind,
            first,
            last,
            self.vis,
            self.word_list,
            self.item_surfs,
            self.runs,
        );
        // They were seen, but draw nothing: gaps in the input.
        census.seen_total = census.seen_total.saturating_add(unworded);
        census.input_gap_tally = census.input_gap_tally.saturating_add(unworded);
        self.push_run_items();
        census
    }
}

/// The sort key of world surface `facet` drawn with its colour pass: the
/// draw-surf word the world capture holds, else the material's baked key with
/// the surface's scene light index put in. None for a surface with no material
/// or no key.
pub(crate) fn compact_level_colour_key(
    cull: &LevelHide,
    facet: usize,
    baked_material_ids: &[Option<u64>],
    batch_key_lights: &[u8],
) -> Option<u64> {
    let material = cull.facet_materials.get(facet).copied().flatten()?;
    let batch_light = || {
        let &(batch, _, _) = cull.facet_batch_ranges.get(facet)?;
        batch_key_lights.get(batch).copied()
    };
    let baked_with_light = || {
        let baked = baked_material_ids
            .get(material.order())
            .copied()
            .flatten()?;
        let own_light = cull.facet_key_lights.get(facet).copied();
        let light = own_light.or_else(batch_light).unwrap_or(0);
        Some(crate::assemble::draw_items::scene_light_index_of(
            baked, light,
        ))
    };
    cull.capture
        .compact_facet_draws
        .get(facet)
        .map(|word| word.packed)
        .filter(|&packed| packed != 0)
        .or_else(baked_with_light)
}

/// Adds one draw item per surface of a brush model's span `start_facet..` that
/// is not in `drawn`, drawing it with its colour key. Returns how many items
/// were added and how many surfaces had no key.
pub(crate) fn append_solid_model_colour_span(
    cull: &mut LevelHide,
    start_facet: u16,
    facet_tally: u16,
    drawn: &mut HashSet<u16>,
    baked_material_ids: &[Option<u64>],
    batch_key_lights: &[u8],
) -> (u32, u32) {
    let from = usize::from(start_facet);
    let span = from..from.saturating_add(usize::from(facet_tally));
    let fresh: Vec<u16> = span
        .filter_map(|surf| u16::try_from(surf).ok())
        .filter(|&surf| drawn.insert(surf))
        .collect();
    let mut keyless = 0u32;
    let before = cull.draw_batch_list.len();
    for surf in fresh {
        let facet = surf as usize;
        let found = compact_level_colour_key(cull, facet, baked_material_ids, batch_key_lights);
        let Some(colour_key) = found else {
            keyless += 1;
            continue;
        };
        let list_start = cull.item_surfs.len() as u32;
        cull.item_surfs.push(surf);
        cull.draw_batch_list.push(LevelDrawItem {
            key: colour_key,
            facet_ix: surf,
            batch_run: 1,
            list_start,
            entry_first: surf,
            kind: LevelDrawItemKind::BModel,
        });
    }
    ((cull.draw_batch_list.len() - before) as u32, keyless)
}

/// Recomputes `cull.draw_batch_list_id` after draw items were added by hand.
pub(crate) fn refresh_draw_items_id(cull: &mut LevelHide) {
    cull.draw_batch_list_id = draw_items_content_id(cull);
}

/// The code a draw item's kind adds to its identity.
fn kind_code(kind: LevelDrawItemKind) -> u64 {
    match kind {
        LevelDrawItemKind::BModel => 3,
        LevelDrawItemKind::Bsp(range) => match range {
            CameraSpanKind::LitOpaque => 0,
            CameraSpanKind::LitTrans => 1,
            CameraSpanKind::Emissive => 2,
            CameraSpanKind::Decal => 4,
        },
    }
}

/// Folds one draw item into a content hash: key, entry, length, surfaces,
/// then the kind.
fn mix_draw_item_id(id: &mut u64, item: &LevelDrawItem, item_surfs: &[u16]) {
    let head = [item.key, u64::from(item.entry_first), u64::from(item.batch_run)];
    let surfs = item_surfs.iter().map(|&surf| u64::from(surf));
    for word in head.into_iter().chain(surfs).chain([kind_code(item.kind)]) {
        blend_content_id(id, word);
    }
}

/// How many times drawing `items` in order switches material.
#[must_use]
pub(crate) fn draw_item_rebinds(items: &[LevelDrawItem]) -> u32 {
    let mut materials = items
        .iter()
        .map(|item| SortedDraw::unpack_from_bits(item.key).material_draw_rank_slot());
    let Some(mut bound) = materials.next() else {
        return 0;
    };
    let switches = materials
        .filter(|&material| std::mem::replace(&mut bound, material) != material)
        .count();
    u32::try_from(switches).map_or(u32::MAX, |n| n.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draw-surf word that sorts on `material` alone.
    fn key(material: u64) -> u64 {
        material << 32
    }

    fn words(keys: &[u64]) -> Vec<SortedDraw> {
        keys.iter()
            .copied()
            .map(SortedDraw::unpack_from_bits)
            .collect()
    }

    /// The lists one or more camera ranges were appended to.
    #[derive(Default)]
    struct Lists {
        runs: Vec<WorldFaceRun<CameraSpanKind>>,
        item_surfs: Vec<u16>,
        items: Vec<LevelDrawItem>,
        g0: Vec<u16>,
    }

    impl Lists {
        fn append(
            &mut self,
            kind: CameraSpanKind,
            span: Range<u32>,
            marks: &mut [u8],
            table: &[SortedDraw],
        ) -> LevelTreeFacetDrawCensus {
            CameraOut {
                vis: marks,
                word_list: table,
                runs: &mut self.runs,
                item_surfs: &mut self.item_surfs,
                items: &mut self.items,
                unworded: &mut self.g0,
            }
            .append(kind, span)
        }
    }

    fn item(key: u64, kind: LevelDrawItemKind) -> LevelDrawItem {
        LevelDrawItem {
            key,
            facet_ix: 0,
            batch_run: 1,
            list_start: 0,
            entry_first: 0,
            kind,
        }
    }

    fn skips(
        dists: &[u16],
        seat: usize,
        at: [f32; 3],
        eye_pos: Option<Vec3>,
        scale: Option<f32>,
    ) -> bool {
        prop_hide_dist_skips_slot(dists, seat, at, eye_pos, scale)
    }

    #[test]
    fn a_model_with_no_eye_or_no_distance_is_never_skipped() {
        let far = [10_000.0, 0.0, 0.0];
        let table = [100u16, 0];
        assert!(!skips(&table, 0, far, None, None));
        assert!(!skips(&table, 1, far, Some(Vec3::ZERO), None));
        // A slot past the table has no distance either.
        assert!(!skips(&table, 9, far, Some(Vec3::ZERO), None));
    }

    #[test]
    fn a_model_beyond_its_distance_is_skipped() {
        let eye = Some(Vec3::ZERO);
        let cases = [
            ([99.0, 0.0, 0.0], false),
            ([100.0, 0.0, 0.0], true),
            // Distance counts in all three axes.
            ([60.0, 60.0, 60.0], true),
        ];
        for (at, hidden) in cases {
            assert_eq!(skips(&[100], 0, at, eye, None), hidden, "{at:?}");
        }
    }

    #[test]
    fn the_lod_scale_stretches_the_distance_a_model_is_judged_at() {
        let at = [60.0, 0.0, 0.0];
        assert!(!skips(&[100], 0, at, Some(Vec3::ZERO), None));
        assert!(skips(&[100], 0, at, Some(Vec3::ZERO), Some(2.0)));
    }

    #[test]
    fn clearing_the_products_keeps_the_counters() {
        let mut stats = SightFrameStats {
            view_volume_planes: vec![[0.0, 0.0, 1.0, 0.0]],
            prop_vis: vec![1, 0],
            prop_vis_id: 7,
            room_vis: vec![3],
            room_vis_tally: 2,
            room_clips: vec![CellClip::BLANK],
            view: CullViewFacts {
                every_cell: true,
                prepared: true,
                pvs_locked: true,
                ..CullViewFacts::default()
            },
            eyeless_frames: 4,
            keys: 9,
            ..SightFrameStats::default()
        };
        clear_products(&mut stats);
        assert!(stats.view_volume_planes.is_empty() && stats.prop_vis.is_empty());
        assert!(stats.room_vis.is_empty() && stats.room_clips.is_empty());
        assert_eq!((stats.prop_vis_id, stats.room_vis_tally), (0, 0));
        let view = stats.view;
        assert!(!view.every_cell && !view.prepared && !view.pvs_locked);
        // The running count of frames with no eye cell is not a product.
        assert_eq!((stats.eyeless_frames, stats.keys), (4, 9));
    }

    #[test]
    fn rebinds_count_the_material_changes_in_draw_order() {
        let bsp = LevelDrawItemKind::Bsp(CameraSpanKind::LitOpaque);
        assert_eq!(draw_item_rebinds(&[]), 0);
        let items: Vec<_> = [1, 1, 2, 1].map(|m| item(key(m), bsp)).into();
        assert_eq!(draw_item_rebinds(&items), 3);
        assert_eq!(draw_item_rebinds(&items[..2]), 1);
    }

    fn identity(it: &LevelDrawItem, surfs: &[u16]) -> u64 {
        let mut id = CONTENT_KEY_SEED;
        mix_draw_item_id(&mut id, it, surfs);
        id
    }

    #[test]
    fn a_draw_items_identity_follows_its_key_length_surfaces_and_kind() {
        let base = item(key(1), LevelDrawItemKind::Bsp(CameraSpanKind::LitOpaque));
        let reference = identity(&base, &[4, 5]);
        assert_eq!(reference, identity(&base, &[4, 5]));
        let changed = [
            identity(&base, &[4, 6]),
            identity(&item(key(2), base.kind), &[4, 5]),
            identity(&LevelDrawItem { batch_run: 2, ..base }, &[4, 5]),
            identity(
                &LevelDrawItem {
                    entry_first: 4,
                    ..base
                },
                &[4, 5],
            ),
        ];
        assert!(changed.iter().all(|&other| other != reference));
    }

    #[test]
    fn every_kind_of_draw_item_has_its_own_identity() {
        let kinds = [
            LevelDrawItemKind::Bsp(CameraSpanKind::LitOpaque),
            LevelDrawItemKind::Bsp(CameraSpanKind::LitTrans),
            LevelDrawItemKind::Bsp(CameraSpanKind::Emissive),
            LevelDrawItemKind::Bsp(CameraSpanKind::Decal),
            LevelDrawItemKind::BModel,
        ];
        let distinct: std::collections::HashSet<u64> = kinds
            .iter()
            .map(|&kind| identity(&item(key(1), kind), &[1]))
            .collect();
        assert_eq!(distinct.len(), kinds.len());
    }

    #[test]
    fn a_range_turns_into_one_item_per_run_of_equal_keys() {
        let table = words(&[key(1), key(1), key(2), key(1)]);
        let mut marks = [1u8; 4];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::LitOpaque, 0..4, &mut marks, &table);
        let counts = (
            census.span_total,
            census.seen_total,
            census.accepted_total,
            census.runs_seen,
        );
        assert_eq!(counts, (4, 4, 4, 3));
        assert_eq!(census.input_gap_tally, 0);
        assert_eq!(lists.item_surfs, [0, 1, 2, 3]);
        let shape: Vec<_> = lists
            .items
            .iter()
            .map(|it| (it.facet_ix, it.batch_run, it.list_start, it.entry_first, it.key))
            .collect();
        assert_eq!(
            shape,
            [
                (0, 2, 0, 0, key(1)),
                (2, 1, 2, 2, key(2)),
                (3, 1, 3, 3, key(1))
            ]
        );
        let opaque = LevelDrawItemKind::Bsp(CameraSpanKind::LitOpaque);
        assert!(lists.items.iter().all(|it| it.kind == opaque));
        assert!(lists.g0.is_empty());
    }

    #[test]
    fn a_surface_that_is_not_visible_is_left_out_and_does_not_split_a_run() {
        let table = words(&[key(1); 3]);
        let mut marks = [1u8, 0, 1];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::LitTrans, 0..3, &mut marks, &table);
        assert_eq!(census.seen_total, 2);
        assert_eq!(lists.item_surfs, [0, 2]);
        let runs: Vec<_> = lists.items.iter().map(|it| it.batch_run).collect();
        assert_eq!(runs, [2]);
    }

    #[test]
    fn a_visible_surface_with_no_draw_surf_word_is_unmarked_and_listed() {
        let table = words(&[key(1), 0, key(1)]);
        let mut marks = [1u8; 3];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::LitOpaque, 0..3, &mut marks, &table);
        assert_eq!(marks, [1, 0, 1]);
        assert_eq!(lists.g0, [1]);
        assert_eq!(lists.item_surfs, [0, 2]);
        // It was seen, but it is a gap in the input and draws nothing.
        let counts = (
            census.seen_total,
            census.accepted_total,
            census.input_gap_tally,
        );
        assert_eq!(counts, (3, 2, 1));
    }

    #[test]
    fn a_run_past_128_surfaces_continues_the_same_entry() {
        let n = 130usize;
        let table = words(&vec![key(5); n]);
        let mut marks = vec![1u8; n];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::Emissive, 0..n as u32, &mut marks, &table);
        assert_eq!((census.runs_seen, census.entry_record_total), (2, 1));
        let shape: Vec<_> = lists
            .items
            .iter()
            .map(|it| (it.batch_run, it.facet_ix, it.entry_first, it.list_start))
            .collect();
        // The second run shares its entry's first surface.
        assert_eq!(shape, [(128, 0, 0, 0), (2, 128, 0, 128)]);
    }

    #[test]
    fn a_new_range_starts_a_new_entry() {
        let table = words(&[key(1); 4]);
        let mut marks = [1u8; 4];
        let mut lists = Lists::default();
        lists.append(CameraSpanKind::LitOpaque, 0..2, &mut marks, &table);
        lists.append(CameraSpanKind::Decal, 2..4, &mut marks, &table);
        let [_, second] = lists.items[..] else {
            panic!("expected two items, got {}", lists.items.len());
        };
        assert_eq!((second.entry_first, second.list_start), (2, 2));
        assert_eq!(second.kind, LevelDrawItemKind::Bsp(CameraSpanKind::Decal));
    }

    #[test]
    fn a_range_that_ends_before_it_begins_is_one_input_gap() {
        let table = words(&[key(1)]);
        let mut marks = [1u8];
        let mut lists = Lists::default();
        #[allow(clippy::reversed_empty_ranges)]
        let census = lists.append(CameraSpanKind::LitOpaque, 3..1, &mut marks, &table);
        assert_eq!((census.input_gap_tally, census.span_total), (1, 0));
        assert!(lists.items.is_empty());
        assert_eq!(marks, [1]);
    }

    #[test]
    fn a_range_past_the_tables_adds_nothing_and_counts_every_surface() {
        let table = words(&[key(1); 3]);
        let mut marks = [1u8, 0, 1];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::LitOpaque, 0..5, &mut marks, &table);
        let counts = (
            census.span_total,
            census.seen_total,
            census.input_gap_tally,
            census.accepted_total,
        );
        assert_eq!(counts, (5, 2, 5, 0));
        assert!(lists.items.is_empty() && lists.item_surfs.is_empty());
    }

    #[test]
    fn an_empty_range_adds_nothing() {
        let table = words(&[key(1)]);
        let mut marks = [1u8];
        let mut lists = Lists::default();
        let census = lists.append(CameraSpanKind::LitOpaque, 1..1, &mut marks, &table);
        assert_eq!(census, LevelTreeFacetDrawCensus::default());
        assert!(lists.items.is_empty());
    }
}
