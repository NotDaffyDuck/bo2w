//! World visibility, once a frame: which cells the eye sees through the
//! portals, which of their surfaces and static models are inside the view
//! frustum, and the draw items that result.

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use dpvs_t6::{
    AabbCullStats, AabbTreeCull, BspDrawSurfCensus, BspDrawSurfRun, CellClip, DpvsPlanes,
    DpvsVisData, GfxDrawSurf, VisBits, add_aabb_tree_surfaces_in_frustum,
    add_bsp_draw_surfs_camera, cell_caster_row_words, msb_get, or_caster_rows_for_visible,
    smodel_cull_dist_hides, words_for_bits,
};

use crate::assemble::drawsurf::list::{CONTENT_ID_SEED, mix_content_bytes, mix_content_id};
use crate::prepare::scene::view_parms::{PreparedSceneView, RLockPvs};
use crate::prepare::scene::world::{
    WorldCull, WorldDpvs, WorldDrawItem, WorldDrawItemKind, WorldScene,
};

/// What the last visibility pass saw and produced. Counters are for the log
/// and the diagnostics; the vectors are the pass's products, which later
/// systems read.
#[derive(Resource, Clone, Debug, Default)]
pub struct DpvsFrameStats {
    /// The cell the eye is in, if it is in one.
    pub eye_cell: Option<usize>,
    /// Frames in which the eye was in no cell (cumulative).
    pub eye_cell_unresolved: u32,
    pub visible_cells: u32,
    pub portals_frustum_skipped: u32,

    pub aabb_nodes_visited: u32,
    pub aabb_planes_dropped: u32,
    pub surfaces_rejected_by_bounds: u32,
    pub surfaces_admitted_unbounded: u32,
    pub smodels_rejected_by_bounds: u32,
    pub smodels_admitted_unbounded: u32,

    pub bsp_visible_surfaces: u32,
    pub bsp_admitted_surfaces: u32,
    pub bsp_run_n: u32,
    pub bsp_input_gap_n: u32,
    pub bsp_ranges_unavailable: u32,

    pub keys: u32,
    pub material_ordinal_refused: u32,
    pub rebinds: u32,
    pub surfaces: u32,
    pub triangles: u32,
    pub lightmapped_triangles: u32,
    pub fallback_triangles: u32,
    pub unculled_triangles: u32,
    pub single_cell: bool,
    pub aspect: f32,
    pub fov_deg: f32,

    pub cell_static_started: Option<std::time::Instant>,

    pub submitted_batches: u32,
    pub rewritten_index_bytes: u64,
    pub visibility_changes: u32,

    /// 1 once the frame had a prepared view to cull against.
    pub view_prepared: u8,
    /// 1 while the PVS lock holds the culling view still.
    pub lock_pvs: u8,

    /// Per static model: 1 if visible.
    pub smodel_vis: Vec<u8>,
    /// Content hash of `smodel_vis`.
    pub smodel_vis_id: u64,

    /// World surfaces that were visible but had no draw-surf word.
    pub g0_world_surfs: Vec<u16>,

    pub frustum_planes: Vec<[f32; 4]>,

    /// Bit set of visible cells (`cell_vis_count` cells).
    pub cell_vis: Vec<u32>,
    pub cell_vis_count: usize,
    /// The clip planes each visible cell was first reached through.
    pub cell_clips: Vec<CellClip>,
    /// Every cell is visible (the eye is outside the world).
    pub cell_vis_all: bool,
}

/// `BO2MP_SINGLE_CELL=1` shows only the eye's own cell (a debugging aid).
fn single_cell_from_env() -> bool {
    matches!(
        std::env::var("BO2MP_SINGLE_CELL").as_deref(),
        Ok("1" | "true" | "yes")
    )
}

#[derive(Component)]
pub struct WorldMeshEntity;

#[derive(Component)]
pub struct StaticModelEntity;

#[derive(Component)]
pub struct ScriptModelEntity;

pub use render_scene::DynEntModelEntity;

/// Tag of a `script_model` game object the map's scripts left in place.
#[derive(Component)]
pub struct ScriptModelGameObject(pub String);

/// True when the static model in `slot` is too far from `eye` to draw.
/// Without an eye nothing is skipped; a slot with no distance never hides.
pub(crate) fn smodel_cull_dist_skips_slot(
    cull_dists: &[u16],
    slot: usize,
    origin: [f32; 3],
    eye: Option<Vec3>,
    scale_last: Option<f32>,
) -> bool {
    let Some(eye) = eye else {
        return false;
    };
    let cull_dist = cull_dists.get(slot).copied().unwrap_or(0);
    let offset = eye - Vec3::from_array(origin);
    smodel_cull_dist_hides(cull_dist, offset.length_squared(), scale_last)
}

/// How the linked portals look from `eye` (`render_frame::portal_eyes`).
fn portal_looks(eye: Vec3) -> Vec<render_frame::portal_eyes::PortalLook> {
    use render_frame::portal_eyes::PortalEnd;
    let ends: Vec<PortalEnd> = sim::guest::portals()
        .iter()
        .map(|p| PortalEnd {
            centre: Vec3::from_array(p.centre),
            normal: Vec3::from_array(p.normal).normalize_or(Vec3::Z),
            up: Vec3::from_array(p.up).normalize_or(Vec3::Y),
            orange: p.orange,
            linked: p.linked,
            pair: p.pair,
        })
        .collect();
    render_frame::portal_eyes::choose_looks(&ends, eye)
}

/// bo2zm: BO2's portal walk (`dpvs_t6`) from `start_cell`. Sets the bit of
/// every cell the walk reached and returns each reached cell's clip planes,
/// those of its first visit. A portal without hull axes (a world in the
/// older format) is left out of the walk.
fn t6_walk(
    dpvs: &WorldDpvs,
    start_cell: usize,
    params: &dpvs_t6::WalkParams<'_>,
    vis: &mut VisBits<'_>,
) -> (Vec<CellClip>, dpvs_t6::WalkStats) {
    let mut cells = Vec::with_capacity(dpvs.cell_count);
    let mut portals = Vec::new();
    for source in dpvs.portals_per_cell.iter().take(dpvs.cell_count) {
        let first = portals.len();
        portals.extend(source.iter().filter_map(|p| {
            Some(dpvs_t6::Portal {
                plane: p.plane,
                cell: p.neighbor,
                vert_start: p.vert_start as u32,
                vert_count: p.vert_count as u32,
                hull_axis: p.hull_axis?,
            })
        }));
        cells.push(dpvs_t6::Cell {
            portal_start: first as u32,
            portal_count: (portals.len() - first) as u32,
            ..Default::default()
        });
    }
    cells.resize(dpvs.cell_count, dpvs_t6::Cell::default());
    let planes: Vec<[f32; 4]> = dpvs
        .planes
        .iter()
        .map(|p| [p.normal[0], p.normal[1], p.normal[2], p.dist])
        .collect();
    let world = dpvs_t6::DpvsWorld {
        cells: &cells,
        portals: &portals,
        verts: &dpvs.portal_verts,
        planes: &planes,
        nodes: &dpvs.nodes,
    };
    let mut scratch = dpvs_t6::WalkScratch::default();
    let stats = dpvs_t6::visit_portals(&world, start_cell, params, &mut scratch);

    let mut clips = vec![CellClip::EMPTY; dpvs.cell_count];
    for (cell, clip) in clips.iter_mut().enumerate() {
        if !scratch.cell_visible(cell) {
            continue;
        }
        vis.set(cell);
        if let Some(first_visit) = scratch.first_visit_clip(cell) {
            *clip = first_visit;
        }
    }
    (clips, stats)
}

/// Marks the surfaces and models of the bounds tree `tree` (one per cell,
/// then some for submodels) that lie inside `frustum`.
fn gather_tree(
    dpvs: &WorldDpvs,
    tree: usize,
    frustum: &[[f32; 4]],
    vis_data: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    let Some(nodes) = dpvs.aabb_trees.get(tree) else {
        return;
    };
    let input = AabbTreeCull {
        nodes,
        smodel_indexes: dpvs
            .aabb_smodel_indices
            .get(tree)
            .map_or(&[][..], Vec::as_slice),
        sorted_surf_index: &dpvs.sorted_surf_index,
        surfaces_bounds: &dpvs.surface_bounds,
        smodel_bounds: &dpvs.smodel_bounds,
    };
    add_aabb_tree_surfaces_in_frustum(&input, frustum, vis_data, stats);
}

/// The trees past the last cell hold surfaces no cell lists (submodels).
fn gather_submodel_trees(
    dpvs: &WorldDpvs,
    frustum: &[[f32; 4]],
    vis_data: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    for tree in dpvs.cell_count..dpvs.aabb_trees.len() {
        gather_tree(dpvs, tree, frustum, vis_data, stats);
    }
}

/// True while the local player is in a script-driven shot (the match's
/// intermission).
fn in_script_camera(
    snapshot: Option<&net::PresentedSnapshot>,
    client: Option<&net::LocalPresentClient>,
) -> bool {
    let (Some(snapshot), Some(client)) = (snapshot, client) else {
        return false;
    };
    snapshot
        .player(client.0)
        .is_some_and(|ps| ps.pm_type == playerstate_t6::game::PM_TYPE_INTERMISSION)
}

/// `BO2MP_DPVS_LOG=1`: once a second, where the eye is and whether a script
/// camera looks.
fn log_eye_cell(eye: [f32; 3], cell: Option<usize>, script_camera: bool) {
    if std::env::var_os("BO2MP_DPVS_LOG").is_none() {
        return;
    }
    static LAST_SECOND: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let second = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let Ok(mut last) = LAST_SECOND.lock() else {
        return;
    };
    if *last == second {
        return;
    }
    *last = second;
    diag::info!(
        World,
        "bo2zm dpvs: eye ({:.0} {:.0} {:.0}) cell {:?} script camera {script_camera}",
        eye[0],
        eye[1],
        eye[2],
        cell
    );
}

/// Forgets the last frame's products.
fn clear_products(stats: &mut DpvsFrameStats) {
    stats.frustum_planes.clear();
    stats.smodel_vis.clear();
    stats.smodel_vis_id = 0;
    stats.cell_vis.clear();
    stats.cell_vis_count = 0;
    stats.cell_clips.clear();
    stats.cell_vis_all = false;
    stats.view_prepared = 0;
    stats.lock_pvs = 0;
}

/// codmp: a Black Ops II submodel is drawn only while a live entity shows
/// it: a brush mover naming it that no script hid. Scripts hide some
/// (Nuketown's display glass) and delete others (Socotra's overwatch floors),
/// so the surfaces of every other submodel are marked invisible.
fn hide_unshown_brush_models(
    brushes: &[asset_world::T6BrushSurfaces],
    surface_vis: &mut [u8],
    entities: &Query<&net::CEntityRuntime>,
) {
    if brushes.is_empty() {
        return;
    }
    let shown: HashSet<u32> = entities
        .iter()
        .filter(|rt| {
            rt.in_next_snap()
                && rt.next_state.e_type == entity_t6::et::SCRIPTMOVER
                // eFlags2 arrives with BO2's EntityState; no demo-hide flag until then.
                && entity_t6::script_mover::script_mover_draw(
                    rt.next_state.lerp.e_flags,
                    0,
                    rt.next_state.solid,
                    false,
                    false,
                ) == entity_t6::script_mover::ScriptMoverDraw::BrushModel
        })
        .filter_map(|rt| u32::try_from(rt.next_state.index).ok())
        .collect();
    for brush in brushes {
        if shown.contains(&brush.brush_model) {
            continue;
        }
        let len = surface_vis.len();
        let first = (brush.first_surf as usize).min(len);
        let last = (first + brush.surface_count as usize).min(len);
        surface_vis[first..last].fill(0);
    }
}

/// What the finished draw items add up to.
#[derive(Default)]
struct DrawTally {
    surfaces: u32,
    rebinds: u32,
    lightmapped_tris: u32,
    fallback_tris: u32,
    batch_visible: Vec<bool>,
}

/// The hash of the draw items: their count, then each item's identity.
fn draw_items_content_id(cull: &WorldCull) -> u64 {
    let mut id = CONTENT_ID_SEED;
    mix_content_id(&mut id, cull.draw_items.len() as u64);
    for item in &cull.draw_items {
        mix_draw_item_id(&mut id, item, cull.draw_item_surfs(item));
    }
    id
}

/// Counts the surfaces, material changes and triangles of `cull.draw_items`
/// and notes which batches they touch.
fn tally(cull: &WorldCull) -> DrawTally {
    let mut tally = DrawTally {
        batch_visible: vec![false; cull.batch_count as usize],
        ..DrawTally::default()
    };
    let mut previous_material: Option<u16> = None;
    for item in &cull.draw_items {
        let material = GfxDrawSurf::from_packed(item.key).material_sorted_index();
        if previous_material != Some(material) {
            tally.rebinds += 1;
            previous_material = Some(material);
        }
        for &surf in cull.draw_item_surfs(item) {
            let Some(&(batch, _, index_count)) = cull.surface_batch_ranges.get(usize::from(surf))
            else {
                continue;
            };
            if index_count == 0 {
                continue;
            }
            tally.surfaces = tally.surfaces.saturating_add(1);
            if let Some(seen) = tally.batch_visible.get_mut(batch) {
                *seen = true;
            }
            let triangles = index_count / 3;
            if cull.batch_lightmapped.get(batch).copied().unwrap_or(false) {
                tally.lightmapped_tris = tally.lightmapped_tris.saturating_add(triangles);
            } else {
                tally.fallback_tris = tally.fallback_tris.saturating_add(triangles);
            }
        }
    }
    tally
}

pub fn apply_dpvs_cull(
    mut scene: ResMut<WorldScene>,
    mut stats: ResMut<DpvsFrameStats>,
    mut published_vis: ResMut<render_scene::PublishedCellVis>,
    prepared: Res<PreparedSceneView>,
    lock_pvs: Res<RLockPvs>,
    presented: (
        Option<Res<net::PresentedSnapshot>>,
        Option<Res<net::LocalPresentClient>>,
    ),
    entities: Query<&net::CEntityRuntime>,
) {
    let _span = perf::Span::RenderCullCpuMs.enter();
    // bo2zm M3: a script camera (the intermission shot of Nuketown's
    // game-over rocket flying down from the sky) sees every cell, as from
    // outside the world: from high over the map no portal leads to the town.
    let script_camera = in_script_camera(presented.0.as_deref(), presented.1.as_deref());

    clear_products(&mut stats);
    *published_vis = render_scene::PublishedCellVis::default();
    render_frame::portal_eyes::set_looks(Vec::new());

    let Some(cull) = scene.cull.as_mut() else {
        return;
    };
    if cull.batch_count == 0 && cull.static_model_entities.is_empty() {
        return;
    }
    if !prepared.ready {
        return;
    }
    stats.view_prepared = 1;
    stats.lock_pvs = u8::from(lock_pvs.enabled && lock_pvs.frozen.is_some());

    let eye_pos = lock_pvs.dpvs_eye(&prepared);
    let eye = eye_pos.to_array();
    let (fov, aspect) = lock_pvs.dpvs_fov_aspect(&prepared);
    let dpvs = &cull.dpvs;
    if dpvs.cell_count == 0 {
        return;
    }
    let frustum = lock_pvs.dpvs_frustum(&prepared).to_vec();
    stats.frustum_planes.clone_from(&frustum);

    // Which cells are visible.
    let tree = DpvsPlanes {
        planes: &dpvs.planes,
        nodes: &dpvs.nodes,
        cell_count: dpvs.cell_count as u32,
    };
    let eye_cell = tree.cell_for_point(eye).filter(|_| !script_camera);
    log_eye_cell(eye, tree.cell_for_point(eye), script_camera);
    let single_cell = single_cell_from_env();

    let mut words = vec![0u32; words_for_bits(dpvs.cell_count.max(1))];
    let mut vis = VisBits::new(&mut words);
    let mut portals_frustum_skipped = 0;
    match eye_cell {
        Some(cell) if single_cell => vis.set(cell),
        Some(cell) => {
            let bevels = lock_pvs.dpvs_portal_bevels(&prepared);
            let params = dpvs_t6::WalkParams {
                eye,
                view_plane: dpvs_t6::view_plane(lock_pvs.dpvs_forward(&prepared).to_array(), eye),
                planes: &frustum,
                near: frustum.first().copied(),
                far: frustum.get(1).copied(),
                walk_limit: 0,
                bevels: bevels.as_ref(),
                force_invisible: None,
                occluders: &[],
            };
            let (clips, walk) = t6_walk(dpvs, cell, &params, &mut vis);
            stats.cell_clips = clips;
            portals_frustum_skipped = walk.skip_clip;
        }
        None => (0..dpvs.cell_count).for_each(|cell| vis.set(cell)),
    }

    // Reset the per-frame lists the passes below fill.
    cull.surface_vis.clear();
    cull.surface_vis.resize(cull.surface_batch_ranges.len(), 0);
    cull.smodel_vis.clear();
    cull.smodel_vis.resize(cull.static_model_entities.len(), 0);
    cull.draw_items.clear();
    cull.item_surfs.clear();
    cull.g0_surfs.clear();
    stats.g0_world_surfs.clear();

    // What an eye behind the other end of each linked portal would see.
    let looks = if script_camera {
        Vec::new()
    } else {
        portal_looks(eye_pos)
    };
    let look_frustums: Vec<Vec<[f32; 4]>> = looks
        .iter()
        .map(|look| {
            let mut planes = render_frame::portal_eyes::move_planes(&frustum, &look.through);
            if let Some(near) = planes.first_mut() {
                *near = look.exit_plane.to_array();
            }
            planes
        })
        .collect();

    let mut aabb = AabbCullStats::default();
    {
        let mut vis_data = DpvsVisData {
            surface_vis: &mut cull.surface_vis,
            smodel_vis: &mut cull.smodel_vis,
        };
        for cell in 0..dpvs.cell_count {
            if vis.get(cell) {
                gather_tree(dpvs, cell, &frustum, &mut vis_data, &mut aabb);
            }
        }
        gather_submodel_trees(dpvs, &frustum, &mut vis_data, &mut aabb);
        // That eye stands behind a wall, outside the cells the walk starts
        // from, so the walk would drop the ground right past the exit.
        // Every cell is offered to the moved frustum instead; it culls by
        // bounds.
        for through in &look_frustums {
            for cell in 0..dpvs.cell_count {
                vis.set(cell);
                gather_tree(dpvs, cell, through, &mut vis_data, &mut aabb);
            }
            gather_submodel_trees(dpvs, through, &mut vis_data, &mut aabb);
        }
    }
    render_frame::portal_eyes::set_looks(looks);

    hide_unshown_brush_models(&cull.t6_brush_surfaces, &mut cull.surface_vis, &entities);

    // The camera's draw items, one range kind at a time.
    let mut census = BspDrawSurfCensus::default();
    for range in dpvs.camera_ranges.iter() {
        let part = append_camera_bsp_range(
            range.kind,
            range.begin,
            range.end,
            &mut cull.surface_vis,
            &cull.capture.packed_draw_surfs,
            &mut cull.bsp_run_scratch,
            &mut cull.item_surfs,
            &mut cull.draw_items,
            &mut cull.g0_surfs,
        );
        census.range_n = census.range_n.saturating_add(part.range_n);
        census.visible_n = census.visible_n.saturating_add(part.visible_n);
        census.admitted_n = census.admitted_n.saturating_add(part.admitted_n);
        census.run_n = census.run_n.saturating_add(part.run_n);
        census.input_gap_n = census.input_gap_n.saturating_add(part.input_gap_n);
    }
    stats.bsp_visible_surfaces = census.visible_n;
    stats.bsp_admitted_surfaces = census.admitted_n;
    stats.bsp_run_n = census.run_n;
    stats.bsp_input_gap_n = census.input_gap_n;
    stats.material_ordinal_refused = census.input_gap_n;

    let totals = tally(cull);
    cull.draw_items_id = draw_items_content_id(cull);

    // Publish.
    stats.eye_cell = eye_cell;
    if eye_cell.is_none() {
        stats.eye_cell_unresolved = stats.eye_cell_unresolved.saturating_add(1);
    }
    stats.visible_cells = vis.count_ones();
    drop(vis);
    stats.cell_vis.clone_from(&words);
    stats.cell_vis_count = dpvs.cell_count;
    stats.cell_vis_all = eye_cell.is_none();
    cull.cell_vis.clone_from(&words);
    cull.cell_vis_all = eye_cell.is_none();
    *published_vis = render_scene::PublishedCellVis {
        words,
        cell_count: dpvs.cell_count,
        vis_all: eye_cell.is_none(),
    };
    stats.portals_frustum_skipped = portals_frustum_skipped;
    stats.aabb_nodes_visited = aabb.nodes_visited;
    stats.aabb_planes_dropped = aabb.planes_dropped;
    stats.surfaces_rejected_by_bounds = aabb.surfaces_rejected_by_bounds;
    stats.surfaces_admitted_unbounded = aabb.surfaces_admitted_unbounded;
    stats.smodels_rejected_by_bounds = aabb.smodels_rejected_by_bounds;
    stats.smodels_admitted_unbounded = aabb.smodels_admitted_unbounded;
    stats.keys = cull.draw_items.len() as u32;
    stats.g0_world_surfs.clone_from(&cull.g0_surfs);
    stats.rebinds = totals.rebinds;
    stats.surfaces = totals.surfaces;
    stats.triangles = totals.lightmapped_tris.saturating_add(totals.fallback_tris);
    stats.lightmapped_triangles = totals.lightmapped_tris;
    stats.fallback_triangles = totals.fallback_tris;
    stats.unculled_triangles = (cull.packed_indices.len() / 3) as u32;
    stats.single_cell = single_cell;
    stats.aspect = aspect;
    stats.fov_deg = fov.to_degrees();

    let submitted = totals.batch_visible.iter().filter(|seen| **seen).count() as u32;
    perf::Counter::CounterSubmittedBatches.emit(f64::from(submitted));
    stats.submitted_batches = submitted;
    stats.rewritten_index_bytes = 0;
    stats.visibility_changes = 0;
    stats.smodel_vis.extend_from_slice(&cull.smodel_vis);
    let mut smodel_id = CONTENT_ID_SEED;
    mix_content_bytes(&mut smodel_id, &cull.smodel_vis);
    stats.smodel_vis_id = smodel_id;
}

/// Logs the tags of the `script_model` props the scripts kept, once.
pub(crate) fn log_script_model_gaps_once(
    props: Query<&ScriptModelGameObject>,
    mut done: Local<bool>,
) {
    if *done || props.is_empty() {
        return;
    }
    *done = true;
    let mut tags: Vec<&str> = props.iter().map(|prop| prop.0.as_str()).collect();
    tags.sort_unstable();
    tags.dedup();
    diag::info!(
        World,
        "script_model gameobjects after _gameobjects::main: {} props kept, tags={:?}",
        props.iter().count(),
        tags
    );
}

/// Logs the first frame's culling numbers, once.
pub(crate) fn log_dpvs_stats_once(stats: Res<DpvsFrameStats>, mut done: Local<bool>) {
    if *done || stats.unculled_triangles == 0 {
        return;
    }
    *done = true;
    let s = &*stats;
    diag::info!(
        World,
        "dpvs frame: eye_cell={:?} unresolved={} visible_cells={} portals_frustum_skipped={} \
         aabb_nodes={} planes_dropped={} surf_rejected_by_bounds={} surf_unbounded={} \
         smodel_rejected_by_bounds={} smodel_unbounded={} bsp_visible={} bsp_admitted={} \
         bsp_runs={} bsp_input_gap={} bsp_ranges_unavailable={} keys={} ordinal_refused={} \
         g0_world={} rebinds={} surfaces={} tris={}/{} (lightmapped={} fallback={}) \
         single_cell={} batches={} rewrite_bytes={} vis_chg={} aspect={:.3} fov_deg={:.1}",
        s.eye_cell,
        s.eye_cell_unresolved,
        s.visible_cells,
        s.portals_frustum_skipped,
        s.aabb_nodes_visited,
        s.aabb_planes_dropped,
        s.surfaces_rejected_by_bounds,
        s.surfaces_admitted_unbounded,
        s.smodels_rejected_by_bounds,
        s.smodels_admitted_unbounded,
        s.bsp_visible_surfaces,
        s.bsp_admitted_surfaces,
        s.bsp_run_n,
        s.bsp_input_gap_n,
        s.bsp_ranges_unavailable,
        s.keys,
        s.material_ordinal_refused,
        s.g0_world_surfs.len(),
        s.rebinds,
        s.surfaces,
        s.triangles,
        s.unculled_triangles,
        s.lightmapped_triangles,
        s.fallback_triangles,
        s.single_cell,
        s.submitted_batches,
        s.rewritten_index_bytes,
        s.visibility_changes,
        s.aspect,
        s.fov_deg,
    );
}

/// Marks the surfaces a shadow-casting view sees: the cells a camera sees
/// plus the cells that can cast into them, each culled to `frustum`. Returns
/// the cull counters and how many cells were offered. `draw_msb` receives the
/// cells offered, as a most-significant-bit-first bit set.
pub(crate) fn add_world_surfaces_frustum_only(
    dpvs: &WorldDpvs,
    frustum: &[[f32; 4]],
    camera_cell_vis_lsb: &[u32],
    camera_vis_all: bool,
    surface_vis: &mut [u8],
    smodel_vis: &mut [u8],
    draw_msb: &mut Vec<u32>,
) -> (AabbCullStats, u32) {
    let mut aabb = AabbCullStats::default();
    let mut vis_data = DpvsVisData {
        surface_vis,
        smodel_vis,
    };
    draw_msb.clear();
    draw_msb.resize(cell_caster_row_words(dpvs.cell_count), 0);

    if dpvs.cell_caster_bits.is_empty() {
        // No caster table: every cell may cast.
        for cell in 0..dpvs.cell_count {
            gather_tree(dpvs, cell, frustum, &mut vis_data, &mut aabb);
        }
        return (aabb, dpvs.cell_count as u32);
    }
    let offered = or_caster_rows_for_visible(
        &dpvs.cell_caster_bits,
        dpvs.cell_count,
        camera_cell_vis_lsb,
        camera_vis_all,
        &mut draw_msb[..],
    );
    for cell in 0..dpvs.cell_count {
        if msb_get(draw_msb, cell) {
            gather_tree(dpvs, cell, frustum, &mut vis_data, &mut aabb);
        }
    }
    (aabb, offered)
}

/// Turns one camera surface range into draw items. Surfaces marked visible
/// but lacking a draw-surf word are unmarked and listed in `g0_surfs`. A
/// range that does not fit the tables counts as an input gap and adds no
/// items.
fn append_camera_bsp_range(
    kind: asset_world::CameraRangeKind,
    begin: u32,
    end: u32,
    surface_vis: &mut [u8],
    draw_surfs: &[GfxDrawSurf],
    runs: &mut Vec<BspDrawSurfRun<asset_world::CameraRangeKind>>,
    item_surfs: &mut Vec<u16>,
    draw_items: &mut Vec<WorldDrawItem>,
    g0_surfs: &mut Vec<u16>,
) -> BspDrawSurfCensus {
    if end < begin {
        return BspDrawSurfCensus {
            input_gap_n: 1,
            ..BspDrawSurfCensus::default()
        };
    }
    let range_n = end - begin;
    let (Ok(first), Ok(last)) = (usize::try_from(begin), usize::try_from(end)) else {
        return BspDrawSurfCensus {
            range_n,
            input_gap_n: range_n.max(1),
            ..BspDrawSurfCensus::default()
        };
    };
    if last > surface_vis.len() || last > draw_surfs.len() {
        let visible_n = surface_vis
            .get(first..surface_vis.len().min(last))
            .map_or(0, |seen| seen.iter().filter(|v| **v != 0).count() as u32);
        return BspDrawSurfCensus {
            range_n,
            visible_n,
            input_gap_n: range_n.max(1),
            ..BspDrawSurfCensus::default()
        };
    }

    let mut missing_word = 0u32;
    for surf in first..last {
        if surface_vis[surf] == 0 || draw_surfs[surf].packed != 0 {
            continue;
        }
        surface_vis[surf] = 0;
        missing_word = missing_word.saturating_add(1);
        if let Ok(surf) = u16::try_from(surf) {
            g0_surfs.push(surf);
        }
    }

    runs.clear();
    let mut census =
        add_bsp_draw_surfs_camera(kind, begin, end, surface_vis, draw_surfs, item_surfs, runs);
    census.visible_n = census.visible_n.saturating_add(missing_word);
    census.input_gap_n = census.input_gap_n.saturating_add(missing_word);

    let mut entry_first = 0u16;
    for run in runs.iter() {
        let head = item_surfs[run.list_start as usize];
        if !run.extends_entry {
            entry_first = head;
        }
        draw_items.push(WorldDrawItem {
            key: run.draw_surf.packed,
            surf: head,
            run: run.len,
            list_start: run.list_start,
            entry_first,
            kind: WorldDrawItemKind::Bsp(run.kind),
        });
    }
    census
}

/// The sort key of world surface `surf` drawn with its colour pass: the
/// draw-surf word the world capture holds, else the material's baked key with
/// the surface's scene light index put in. None for a surface with no material
/// or no key.
pub(crate) fn packed_world_colour_key(
    cull: &WorldCull,
    surf: usize,
    baked_material_keys: &[Option<u64>],
    batch_primary_lights: &[u8],
) -> Option<u64> {
    let material = cull.surface_materials.get(surf).copied().flatten()?;
    if let Some(word) = cull.capture.packed_draw_surfs.get(surf) {
        if word.packed != 0 {
            return Some(word.packed);
        }
    }
    let baked = baked_material_keys.get(material.order()).copied().flatten()?;
    let light = match cull.surface_primary_lights.get(surf) {
        Some(&light) => light,
        None => cull
            .surface_batch_ranges
            .get(surf)
            .and_then(|&(batch, _, _)| batch_primary_lights.get(batch).copied())
            .unwrap_or(0),
    };
    Some(crate::assemble::drawsurf::with_scene_light_index(baked, light))
}

/// Adds one draw item per surface of a brush model's span `start_surf..` that
/// is not in `already`, drawing it with its colour key. Returns how many items
/// were added and how many surfaces had no key.
pub(crate) fn append_bmodel_colour_span(
    cull: &mut WorldCull,
    start_surf: u16,
    surface_count: u16,
    already: &mut HashSet<u16>,
    baked_material_keys: &[Option<u64>],
    batch_primary_lights: &[u8],
) -> (u32, u32) {
    let (mut added, mut refused) = (0u32, 0u32);
    let first = usize::from(start_surf);
    for surf in first..first.saturating_add(usize::from(surface_count)) {
        let Ok(surf16) = u16::try_from(surf) else {
            continue;
        };
        if !already.insert(surf16) {
            continue;
        }
        let Some(key) =
            packed_world_colour_key(cull, surf, baked_material_keys, batch_primary_lights)
        else {
            refused += 1;
            continue;
        };
        cull.draw_items.push(WorldDrawItem {
            key,
            surf: surf16,
            run: 1,
            list_start: cull.item_surfs.len() as u32,
            entry_first: surf16,
            kind: WorldDrawItemKind::BModel,
        });
        cull.item_surfs.push(surf16);
        added += 1;
    }
    (added, refused)
}

/// Recomputes `cull.draw_items_id` after draw items were added by hand.
pub(crate) fn refresh_draw_items_id(cull: &mut WorldCull) {
    cull.draw_items_id = draw_items_content_id(cull);
}

/// Folds one draw item into a content hash: key, entry, length, surfaces,
/// then the kind.
fn mix_draw_item_id(id: &mut u64, item: &WorldDrawItem, item_surfs: &[u16]) {
    use asset_world::CameraRangeKind::{Decal, Emissive, LitOpaque, LitTrans};
    mix_content_id(id, item.key);
    mix_content_id(id, u64::from(item.entry_first));
    mix_content_id(id, u64::from(item.run));
    for &surf in item_surfs {
        mix_content_id(id, u64::from(surf));
    }
    let kind_code = match item.kind {
        WorldDrawItemKind::Bsp(LitOpaque) => 0,
        WorldDrawItemKind::Bsp(LitTrans) => 1,
        WorldDrawItemKind::Bsp(Emissive) => 2,
        WorldDrawItemKind::BModel => 3,
        WorldDrawItemKind::Bsp(Decal) => 4,
    };
    mix_content_id(id, kind_code);
}

/// How many times drawing `items` in order switches material.
#[must_use]
pub(crate) fn draw_item_rebinds(items: &[WorldDrawItem]) -> u32 {
    let mut rebinds = 0u32;
    let mut previous: Option<u16> = None;
    for item in items {
        let material = GfxDrawSurf::from_packed(item.key).material_sorted_index();
        if previous != Some(material) {
            rebinds = rebinds.saturating_add(1);
            previous = Some(material);
        }
    }
    rebinds
}

#[cfg(test)]
mod tests {
    use asset_world::CameraRangeKind;

    use super::*;

    /// A draw-surf word that sorts on `material` alone.
    fn key(material: u64) -> u64 {
        material << 32
    }

    fn surfs(keys: &[u64]) -> Vec<GfxDrawSurf> {
        keys.iter().map(|&k| GfxDrawSurf::from_packed(k)).collect()
    }

    struct Out {
        runs: Vec<BspDrawSurfRun<CameraRangeKind>>,
        item_surfs: Vec<u16>,
        items: Vec<WorldDrawItem>,
        g0: Vec<u16>,
    }

    impl Out {
        fn new() -> Self {
            Self {
                runs: Vec::new(),
                item_surfs: Vec::new(),
                items: Vec::new(),
                g0: Vec::new(),
            }
        }

        fn range(
            &mut self,
            kind: CameraRangeKind,
            begin: u32,
            end: u32,
            vis: &mut [u8],
            draw_surfs: &[GfxDrawSurf],
        ) -> BspDrawSurfCensus {
            append_camera_bsp_range(
                kind,
                begin,
                end,
                vis,
                draw_surfs,
                &mut self.runs,
                &mut self.item_surfs,
                &mut self.items,
                &mut self.g0,
            )
        }
    }

    fn item(key: u64, kind: WorldDrawItemKind) -> WorldDrawItem {
        WorldDrawItem {
            key,
            surf: 0,
            run: 1,
            list_start: 0,
            entry_first: 0,
            kind,
        }
    }

    #[test]
    fn a_model_with_no_eye_or_no_distance_is_never_skipped() {
        let dists = [100u16, 0];
        let far = [10_000.0, 0.0, 0.0];
        assert!(!smodel_cull_dist_skips_slot(&dists, 0, far, None, None));
        assert!(!smodel_cull_dist_skips_slot(
            &dists,
            1,
            far,
            Some(Vec3::ZERO),
            None
        ));
        // A slot past the table has no distance either.
        assert!(!smodel_cull_dist_skips_slot(
            &dists,
            9,
            far,
            Some(Vec3::ZERO),
            None
        ));
    }

    #[test]
    fn a_model_beyond_its_distance_is_skipped() {
        let dists = [100u16];
        let eye = Some(Vec3::new(0.0, 0.0, 0.0));
        assert!(!smodel_cull_dist_skips_slot(
            &dists,
            0,
            [99.0, 0.0, 0.0],
            eye,
            None
        ));
        assert!(smodel_cull_dist_skips_slot(
            &dists,
            0,
            [100.0, 0.0, 0.0],
            eye,
            None
        ));
        // Distance counts in all three axes.
        assert!(smodel_cull_dist_skips_slot(
            &dists,
            0,
            [60.0, 60.0, 60.0],
            eye,
            None
        ));
    }

    #[test]
    fn the_lod_scale_stretches_the_distance_a_model_is_judged_at() {
        let dists = [100u16];
        let eye = Some(Vec3::ZERO);
        let at = [60.0, 0.0, 0.0];
        assert!(!smodel_cull_dist_skips_slot(&dists, 0, at, eye, None));
        assert!(smodel_cull_dist_skips_slot(&dists, 0, at, eye, Some(2.0)));
    }

    #[test]
    fn clearing_the_products_keeps_the_counters() {
        let mut stats = DpvsFrameStats {
            frustum_planes: vec![[0.0, 0.0, 1.0, 0.0]],
            smodel_vis: vec![1, 0],
            smodel_vis_id: 7,
            cell_vis: vec![3],
            cell_vis_count: 2,
            cell_clips: vec![CellClip::EMPTY],
            cell_vis_all: true,
            view_prepared: 1,
            lock_pvs: 1,
            eye_cell_unresolved: 4,
            keys: 9,
            ..DpvsFrameStats::default()
        };
        clear_products(&mut stats);
        assert!(stats.frustum_planes.is_empty());
        assert!(stats.smodel_vis.is_empty());
        assert_eq!(stats.smodel_vis_id, 0);
        assert!(stats.cell_vis.is_empty());
        assert_eq!(stats.cell_vis_count, 0);
        assert!(stats.cell_clips.is_empty());
        assert!(!stats.cell_vis_all);
        assert_eq!((stats.view_prepared, stats.lock_pvs), (0, 0));
        // The running count of frames with no eye cell is not a product.
        assert_eq!(stats.eye_cell_unresolved, 4);
        assert_eq!(stats.keys, 9);
    }

    #[test]
    fn rebinds_count_the_material_changes_in_draw_order() {
        let bsp = WorldDrawItemKind::Bsp(CameraRangeKind::LitOpaque);
        assert_eq!(draw_item_rebinds(&[]), 0);
        let items = [
            item(key(1), bsp),
            item(key(1), bsp),
            item(key(2), bsp),
            item(key(1), bsp),
        ];
        assert_eq!(draw_item_rebinds(&items), 3);
        assert_eq!(draw_item_rebinds(&items[..2]), 1);
    }

    #[test]
    fn a_draw_items_identity_follows_its_key_length_surfaces_and_kind() {
        let base = item(key(1), WorldDrawItemKind::Bsp(CameraRangeKind::LitOpaque));
        let id = |it: &WorldDrawItem, surfs: &[u16]| {
            let mut id = CONTENT_ID_SEED;
            mix_draw_item_id(&mut id, it, surfs);
            id
        };
        let same = id(&base, &[4, 5]);
        assert_eq!(same, id(&base, &[4, 5]));
        assert_ne!(same, id(&base, &[4, 6]));
        assert_ne!(same, id(&item(key(2), base.kind), &[4, 5]));
        assert_ne!(
            same,
            id(
                &WorldDrawItem {
                    run: 2,
                    ..base
                },
                &[4, 5]
            )
        );
        assert_ne!(
            same,
            id(
                &WorldDrawItem {
                    entry_first: 4,
                    ..base
                },
                &[4, 5]
            )
        );
    }

    #[test]
    fn every_kind_of_draw_item_has_its_own_identity() {
        let mut ids = Vec::new();
        for kind in [
            WorldDrawItemKind::Bsp(CameraRangeKind::LitOpaque),
            WorldDrawItemKind::Bsp(CameraRangeKind::LitTrans),
            WorldDrawItemKind::Bsp(CameraRangeKind::Emissive),
            WorldDrawItemKind::Bsp(CameraRangeKind::Decal),
            WorldDrawItemKind::BModel,
        ] {
            let mut id = CONTENT_ID_SEED;
            mix_draw_item_id(&mut id, &item(key(1), kind), &[1]);
            ids.push(id);
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn a_range_turns_into_one_item_per_run_of_equal_keys() {
        let draw_surfs = surfs(&[key(1), key(1), key(2), key(1)]);
        let mut vis = [1u8; 4];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitOpaque, 0, 4, &mut vis, &draw_surfs);
        assert_eq!(census.range_n, 4);
        assert_eq!(census.visible_n, 4);
        assert_eq!(census.admitted_n, 4);
        assert_eq!(census.run_n, 3);
        assert_eq!(census.input_gap_n, 0);
        assert_eq!(out.item_surfs, [0, 1, 2, 3]);
        let shape: Vec<_> = out
            .items
            .iter()
            .map(|it| (it.surf, it.run, it.list_start, it.entry_first, it.key))
            .collect();
        assert_eq!(
            shape,
            [(0, 2, 0, 0, key(1)), (2, 1, 2, 2, key(2)), (3, 1, 3, 3, key(1))]
        );
        assert!(out.items.iter().all(|it| it.kind == WorldDrawItemKind::Bsp(CameraRangeKind::LitOpaque)));
        assert!(out.g0.is_empty());
    }

    #[test]
    fn a_surface_that_is_not_visible_is_left_out_and_does_not_split_a_run() {
        let draw_surfs = surfs(&[key(1), key(1), key(1)]);
        let mut vis = [1u8, 0, 1];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitTrans, 0, 3, &mut vis, &draw_surfs);
        assert_eq!(census.visible_n, 2);
        assert_eq!(out.item_surfs, [0, 2]);
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].run, 2);
    }

    #[test]
    fn a_visible_surface_with_no_draw_surf_word_is_unmarked_and_listed() {
        let draw_surfs = surfs(&[key(1), 0, key(1)]);
        let mut vis = [1u8; 3];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitOpaque, 0, 3, &mut vis, &draw_surfs);
        assert_eq!(vis, [1, 0, 1]);
        assert_eq!(out.g0, [1]);
        assert_eq!(out.item_surfs, [0, 2]);
        // It was seen, but it is a gap in the input and draws nothing.
        assert_eq!(census.visible_n, 3);
        assert_eq!(census.admitted_n, 2);
        assert_eq!(census.input_gap_n, 1);
    }

    #[test]
    fn a_run_past_128_surfaces_continues_the_same_entry() {
        let n = 130usize;
        let draw_surfs = surfs(&vec![key(5); n]);
        let mut vis = vec![1u8; n];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::Emissive, 0, n as u32, &mut vis, &draw_surfs);
        assert_eq!(census.run_n, 2);
        assert_eq!(census.entry_n, 1);
        assert_eq!(out.items.len(), 2);
        assert_eq!((out.items[0].run, out.items[1].run), (128, 2));
        assert_eq!((out.items[0].surf, out.items[1].surf), (0, 128));
        // The second run shares its entry's first surface.
        assert_eq!(out.items[0].entry_first, 0);
        assert_eq!(out.items[1].entry_first, 0);
        assert_eq!(out.items[1].list_start, 128);
    }

    #[test]
    fn a_new_range_starts_a_new_entry() {
        let draw_surfs = surfs(&[key(1), key(1), key(1), key(1)]);
        let mut vis = [1u8; 4];
        let mut out = Out::new();
        out.range(CameraRangeKind::LitOpaque, 0, 2, &mut vis, &draw_surfs);
        out.range(CameraRangeKind::Decal, 2, 4, &mut vis, &draw_surfs);
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[1].entry_first, 2);
        assert_eq!(out.items[1].list_start, 2);
        assert_eq!(out.items[1].kind, WorldDrawItemKind::Bsp(CameraRangeKind::Decal));
    }

    #[test]
    fn a_range_that_ends_before_it_begins_is_one_input_gap() {
        let draw_surfs = surfs(&[key(1)]);
        let mut vis = [1u8];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitOpaque, 3, 1, &mut vis, &draw_surfs);
        assert_eq!(census.input_gap_n, 1);
        assert_eq!(census.range_n, 0);
        assert!(out.items.is_empty());
        assert_eq!(vis, [1]);
    }

    #[test]
    fn a_range_past_the_tables_adds_nothing_and_counts_every_surface() {
        let draw_surfs = surfs(&[key(1), key(1), key(1)]);
        let mut vis = [1u8, 0, 1];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitOpaque, 0, 5, &mut vis, &draw_surfs);
        assert_eq!(census.range_n, 5);
        assert_eq!(census.visible_n, 2);
        assert_eq!(census.input_gap_n, 5);
        assert_eq!(census.admitted_n, 0);
        assert!(out.items.is_empty());
        assert!(out.item_surfs.is_empty());
    }

    #[test]
    fn an_empty_range_adds_nothing() {
        let draw_surfs = surfs(&[key(1)]);
        let mut vis = [1u8];
        let mut out = Out::new();
        let census = out.range(CameraRangeKind::LitOpaque, 1, 1, &mut vis, &draw_surfs);
        assert_eq!(census, BspDrawSurfCensus::default());
        assert!(out.items.is_empty());
    }
}
