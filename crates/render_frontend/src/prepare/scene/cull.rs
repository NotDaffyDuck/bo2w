use bevy::platform::collections::HashSet;

use bevy::prelude::*;
use dpvs_t6::{
    cell_caster_row_words, msb_get, or_caster_rows_for_visible, smodel_cull_dist_hides,
};
use dpvs_t6::{
    AabbCullStats, AabbTreeCull, BspDrawSurfCensus, BspDrawSurfRun, CellClip, DpvsPlanes,
    DpvsVisData, GfxDrawSurf, VisBits, add_aabb_tree_surfaces_in_frustum,
    add_bsp_draw_surfs_camera, words_for_bits,
};

use crate::prepare::scene::view_parms::PreparedSceneView;
use crate::prepare::scene::world::{WorldDrawItem, WorldDrawItemKind, WorldScene};

#[derive(Resource, Clone, Debug, Default)]
pub struct DpvsFrameStats {
    pub eye_cell: Option<usize>,

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

    pub view_prepared: u8,

    pub lock_pvs: u8,

    pub smodel_vis: Vec<u8>,

    pub smodel_vis_id: u64,

    pub g0_world_surfs: Vec<u16>,

    pub frustum_planes: Vec<[f32; 4]>,

    pub cell_vis: Vec<u32>,
    pub cell_vis_count: usize,

    pub cell_clips: Vec<CellClip>,

    pub cell_vis_all: bool,
}

fn single_cell_from_env() -> bool {
    matches!(
        std::env::var("BO2MP_SINGLE_CELL").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

#[derive(Component)]
pub struct WorldMeshEntity;

#[derive(Component)]
pub struct StaticModelEntity;

#[derive(Component)]
pub struct ScriptModelEntity;

pub use render_scene::DynEntModelEntity;

#[derive(Component)]
pub struct ScriptModelGameObject(pub String);

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
    let dx = eye.x - origin[0];
    let dy = eye.y - origin[1];
    let dz = eye.z - origin[2];
    smodel_cull_dist_hides(cull_dist, dx * dx + dy * dy + dz * dz, scale_last)
}

/// The linked portals' looks from this eye (`render_frame::portal_eyes`).
fn portal_looks(eye: Vec3) -> Vec<render_frame::portal_eyes::PortalLook> {
    let ends: Vec<_> = sim::guest::portals()
        .iter()
        .map(|p| render_frame::portal_eyes::PortalEnd {
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

/// bo2zm: BO2's walk (`dpvs_t6`) over this map's cells and portals; each visible cell's clip
/// planes are those of its first visit. A portal without hull axes (an MW2-format world) is
/// left out of the walk.
fn t6_walk(
    dpvs: &crate::prepare::scene::world::WorldDpvs,
    start_cell: usize,
    params: &dpvs_t6::WalkParams<'_>,
    vis: &mut VisBits<'_>,
) -> (Vec<CellClip>, dpvs_t6::WalkStats) {
    let mut cells = Vec::with_capacity(dpvs.cell_count);
    let mut portals = Vec::new();
    for cell_portals in dpvs.portals_per_cell.iter().take(dpvs.cell_count) {
        let portal_start = portals.len();
        portals.extend(cell_portals.iter().filter_map(|p| {
            Some(dpvs_t6::Portal {
                plane: p.plane,
                cell: p.neighbor,
                vert_start: p.vert_start as u32,
                vert_count: p.vert_count as u32,
                hull_axis: p.hull_axis?,
            })
        }));
        cells.push(dpvs_t6::Cell {
            portal_start: portal_start as u32,
            portal_count: (portals.len() - portal_start) as u32,
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
    let walk_stats = dpvs_t6::visit_portals(&world, start_cell, params, &mut scratch);
    let mut clips = vec![CellClip::EMPTY; dpvs.cell_count];
    for (cell, clip) in clips.iter_mut().enumerate() {
        if !scratch.cell_visible(cell) {
            continue;
        }
        vis.set(cell);
        if let Some(first) = scratch.first_visit_clip(cell) {
            *clip = first;
        }
    }
    (clips, walk_stats)
}

/// One cell's surfaces and models inside `frustum`.
fn gather_cell(
    dpvs: &crate::prepare::scene::world::WorldDpvs,
    cell: usize,
    frustum: &[[f32; 4]],
    vis_data: &mut DpvsVisData<'_>,
    aabb_stats: &mut AabbCullStats,
) {
    let Some(tree) = dpvs.aabb_trees.get(cell) else {
        return;
    };
    let cull_input = AabbTreeCull {
        nodes: tree,
        smodel_indexes: dpvs
            .aabb_smodel_indices
            .get(cell)
            .map_or(&[][..], Vec::as_slice),
        sorted_surf_index: &dpvs.sorted_surf_index,
        surfaces_bounds: &dpvs.surface_bounds,
        smodel_bounds: &dpvs.smodel_bounds,
    };
    add_aabb_tree_surfaces_in_frustum(&cull_input, frustum, vis_data, aabb_stats);
}

pub fn apply_dpvs_cull(
    mut scene: ResMut<WorldScene>,
    mut stats: ResMut<DpvsFrameStats>,
    mut published_vis: ResMut<render_scene::PublishedCellVis>,
    prepared: Res<PreparedSceneView>,
    lock_pvs: Res<crate::prepare::scene::view_parms::RLockPvs>,
    view_of: (
        Option<Res<net::PresentedSnapshot>>,
        Option<Res<net::LocalPresentClient>>,
    ),
    entities: Query<&net::CEntityRuntime>,
) {
    let _cull = perf::Span::RenderCullCpuMs.enter();
    // bo2zm M3: a script camera's shot (his intermission: Nuketown's
    // game-over rocket flying down from the sky) sees every cell, as from
    // outside the world: from high over the map no portal leads to the town.
    let script_camera = match (view_of.0.as_deref(), view_of.1.as_deref()) {
        (Some(p), Some(l)) => p
            .player(l.0)
            .is_some_and(|ps| ps.pm_type == playerstate_t6::game::PM_TYPE_INTERMISSION),
        _ => false,
    };
    stats.frustum_planes.clear();
    stats.smodel_vis.clear();
    stats.smodel_vis_id = 0;
    stats.cell_vis.clear();
    stats.cell_vis_count = 0;
    stats.cell_clips.clear();
    stats.cell_vis_all = false;
    stats.view_prepared = 0;
    stats.lock_pvs = 0;
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
    let eye = [eye_pos.x, eye_pos.y, eye_pos.z];
    let (fov, aspect) = lock_pvs.dpvs_fov_aspect(&prepared);
    let dpvs = &cull.dpvs;
    if dpvs.cell_count == 0 {
        return;
    }

    let frustum = lock_pvs.dpvs_frustum(&prepared).to_vec();
    stats.frustum_planes.clone_from(&frustum);

    let planes = DpvsPlanes {
        planes: &dpvs.planes,
        nodes: &dpvs.nodes,
        cell_count: dpvs.cell_count as u32,
    };

    let eye_cell = planes.cell_for_point(eye).filter(|_| !script_camera);
    // BO2MP_DPVS_LOG=1: the eye, its cell and whether a script camera looks.
    if std::env::var_os("BO2MP_DPVS_LOG").is_some() {
        static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Ok(mut last) = LAST.lock()
            && *last != now
        {
            *last = now;
            diag::info!(
                World,
                "bo2zm dpvs: eye ({:.0} {:.0} {:.0}) cell {:?} script camera {script_camera}",
                eye[0],
                eye[1],
                eye[2],
                planes.cell_for_point(eye)
            );
        }
    }
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
            let (cell_clips, walk_stats) = t6_walk(dpvs, cell, &params, &mut vis);
            stats.cell_clips = cell_clips;
            portals_frustum_skipped = walk_stats.skip_clip;
        }

        None => {
            for cell in 0..dpvs.cell_count {
                vis.set(cell);
            }
        }
    }

    let surf_count = cull.surface_batch_ranges.len();
    cull.surface_vis.clear();
    cull.surface_vis.resize(surf_count, 0);
    cull.smodel_vis.clear();
    cull.smodel_vis.resize(cull.static_model_entities.len(), 0);
    cull.draw_items.clear();
    cull.item_surfs.clear();
    cull.g0_surfs.clear();
    stats.g0_world_surfs.clear();

    let looks = if script_camera { Vec::new() } else { portal_looks(eye_pos) };
    let look_frustums: Vec<Vec<[f32; 4]>> = looks
        .iter()
        .map(|look| {
            let mut f = render_frame::portal_eyes::move_planes(&frustum, &look.through);
            if let Some(near) = f.first_mut() {
                *near = look.exit_plane.to_array();
            }
            f
        })
        .collect();
    let mut aabb_stats = AabbCullStats::default();
    {
        let mut vis_data = DpvsVisData {
            surface_vis: &mut cull.surface_vis,
            smodel_vis: &mut cull.smodel_vis,
        };
        for cell in 0..dpvs.cell_count {
            if !vis.get(cell) {
                continue;
            }
            gather_cell(dpvs, cell, &frustum, &mut vis_data, &mut aabb_stats);
        }
        // bo2zm: trees past the last cell hold surfaces no cell lists (submodels).
        for tree in dpvs.cell_count..dpvs.aabb_trees.len() {
            gather_cell(dpvs, tree, &frustum, &mut vis_data, &mut aabb_stats);
        }
        // Seeing through a linked portal: what an eye behind the other end
        // sees (`render_frame::portal_eyes`). That eye stands behind a wall,
        // outside the cells the walk starts from, so the walk drops the
        // ground right past the exit; every cell is offered to the moved
        // frustum instead, which culls by bounds.
        for through_frustum in &look_frustums {
            for cell in 0..dpvs.cell_count {
                vis.set(cell);
                gather_cell(dpvs, cell, through_frustum, &mut vis_data, &mut aabb_stats);
            }
            for tree in dpvs.cell_count..dpvs.aabb_trees.len() {
                gather_cell(dpvs, tree, through_frustum, &mut vis_data, &mut aabb_stats);
            }
        }
    }

    render_frame::portal_eyes::set_looks(looks);
    stats.material_ordinal_refused = 0;

    // codmp: a Black Ops II submodel is drawn while a live entity shows
    // it (a brush mover naming it that no script hid): scripts hide some
    // (Nuketown's display glass) and delete others (Socotra's overwatch
    // floors).
    if !cull.t6_brush_surfaces.is_empty() {
        let shown: HashSet<u32> = entities
            .iter()
            .filter(|rt| {
                rt.in_next_snap()
                    && rt.next_state.e_type == entity_t6::et::SCRIPTMOVER
                    // eFlags2 comes with BO2's EntityState; 0 (no demo hide) until then.
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
        for brush in &cull.t6_brush_surfaces {
            if shown.contains(&brush.brush_model) {
                continue;
            }
            let start = (brush.first_surf as usize).min(cull.surface_vis.len());
            let end = (start + brush.surface_count as usize).min(cull.surface_vis.len());
            cull.surface_vis[start..end].fill(0);
        }
    }

    let mut bsp_census = BspDrawSurfCensus::default();
    for range in dpvs.camera_ranges.iter() {
        let range = append_camera_bsp_range(
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
        bsp_census.range_n = bsp_census.range_n.saturating_add(range.range_n);
        bsp_census.visible_n = bsp_census.visible_n.saturating_add(range.visible_n);
        bsp_census.admitted_n = bsp_census.admitted_n.saturating_add(range.admitted_n);
        bsp_census.run_n = bsp_census.run_n.saturating_add(range.run_n);
        bsp_census.input_gap_n = bsp_census.input_gap_n.saturating_add(range.input_gap_n);
    }
    stats.bsp_visible_surfaces = bsp_census.visible_n;
    stats.bsp_admitted_surfaces = bsp_census.admitted_n;
    stats.bsp_run_n = bsp_census.run_n;
    stats.bsp_input_gap_n = bsp_census.input_gap_n;
    stats.material_ordinal_refused = stats
        .material_ordinal_refused
        .saturating_add(bsp_census.input_gap_n);

    let mut draw_items_id = crate::assemble::drawsurf::list::CONTENT_ID_SEED;
    crate::assemble::drawsurf::list::mix_content_id(
        &mut draw_items_id,
        cull.draw_items.len() as u64,
    );

    let batch_count = cull.batch_count as usize;
    let mut batch_visible = vec![false; batch_count];
    let mut surfaces = 0u32;
    let mut prev_mat: Option<u16> = None;
    let mut rebinds = 0u32;
    let mut lightmapped_tris = 0u32;
    let mut fallback_tris = 0u32;
    for item in &cull.draw_items {
        let item_surfs = cull.draw_item_surfs(item);
        mix_draw_item_id(&mut draw_items_id, item, item_surfs);
        let mat = dpvs_t6::GfxDrawSurf::from_packed(item.key).material_sorted_index();
        if prev_mat != Some(mat) {
            rebinds += 1;
            prev_mat = Some(mat);
        }
        for &surf in item_surfs {
            let Some(&(batch, _s0, sc)) = cull.surface_batch_ranges.get(usize::from(surf)) else {
                continue;
            };
            if sc == 0 {
                continue;
            }
            surfaces = surfaces.saturating_add(1);
            if let Some(slot) = batch_visible.get_mut(batch) {
                *slot = true;
            }
            let tris = sc / 3;
            if cull.batch_lightmapped.get(batch).copied().unwrap_or(false) {
                lightmapped_tris = lightmapped_tris.saturating_add(tris);
            } else {
                fallback_tris = fallback_tris.saturating_add(tris);
            }
        }
    }
    let tris = lightmapped_tris.saturating_add(fallback_tris);
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
        words: words.clone(),
        cell_count: dpvs.cell_count,
        vis_all: eye_cell.is_none(),
    };
    stats.portals_frustum_skipped = portals_frustum_skipped;
    stats.aabb_nodes_visited = aabb_stats.nodes_visited;
    stats.aabb_planes_dropped = aabb_stats.planes_dropped;
    stats.surfaces_rejected_by_bounds = aabb_stats.surfaces_rejected_by_bounds;
    stats.surfaces_admitted_unbounded = aabb_stats.surfaces_admitted_unbounded;
    stats.smodels_rejected_by_bounds = aabb_stats.smodels_rejected_by_bounds;
    stats.smodels_admitted_unbounded = aabb_stats.smodels_admitted_unbounded;
    stats.keys = cull.draw_items.len() as u32;
    cull.draw_items_id = draw_items_id;
    stats.g0_world_surfs.clone_from(&cull.g0_surfs);
    stats.rebinds = rebinds;
    stats.surfaces = surfaces;
    stats.triangles = tris;
    stats.lightmapped_triangles = lightmapped_tris;
    stats.fallback_triangles = fallback_tris;
    stats.unculled_triangles = (cull.packed_indices.len() / 3) as u32;
    stats.single_cell = single_cell;
    stats.aspect = aspect;
    stats.fov_deg = fov.to_degrees();

    let submitted_batches = batch_visible.iter().filter(|&&v| v).count() as u32;
    perf::Counter::CounterSubmittedBatches.emit(f64::from(submitted_batches));

    stats.submitted_batches = submitted_batches;
    stats.rewritten_index_bytes = 0;
    stats.visibility_changes = 0;
    stats.smodel_vis.clear();
    stats.smodel_vis.extend_from_slice(&cull.smodel_vis);
    let mut smodel_vis_id = crate::assemble::drawsurf::list::CONTENT_ID_SEED;
    crate::assemble::drawsurf::list::mix_content_bytes(&mut smodel_vis_id, &cull.smodel_vis);
    stats.smodel_vis_id = smodel_vis_id;
}

pub(crate) fn log_script_model_gaps_once(
    props: Query<&ScriptModelGameObject>,
    mut done: Local<bool>,
) {
    if *done || props.is_empty() {
        return;
    }
    *done = true;
    let mut names: Vec<&str> = props.iter().map(|prop| prop.0.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    diag::info!(
        World,
        "script_model gameobjects after _gameobjects::main: {} props kept, tags={:?}",
        props.iter().count(),
        names
    );
}

pub(crate) fn log_dpvs_stats_once(stats: Res<DpvsFrameStats>, mut done: Local<bool>) {
    if *done || stats.unculled_triangles == 0 {
        return;
    }
    *done = true;
    diag::info!(
        World,
        "dpvs frame: eye_cell={:?} unresolved={} visible_cells={} portals_frustum_skipped={} \
         aabb_nodes={} planes_dropped={} \
         surf_rejected_by_bounds={} surf_unbounded={} smodel_rejected_by_bounds={} \
         smodel_unbounded={} bsp_visible={} bsp_admitted={} bsp_runs={} \
         bsp_input_gap={} bsp_ranges_unavailable={} keys={} ordinal_refused={} g0_world={} rebinds={} surfaces={} \
         tris={}/{} (lightmapped={} fallback={}) single_cell={} batches={} rewrite_bytes={} \
         vis_chg={} \
         aspect={:.3} fov_deg={:.1}",
        stats.eye_cell,
        stats.eye_cell_unresolved,
        stats.visible_cells,
        stats.portals_frustum_skipped,
        stats.aabb_nodes_visited,
        stats.aabb_planes_dropped,
        stats.surfaces_rejected_by_bounds,
        stats.surfaces_admitted_unbounded,
        stats.smodels_rejected_by_bounds,
        stats.smodels_admitted_unbounded,
        stats.bsp_visible_surfaces,
        stats.bsp_admitted_surfaces,
        stats.bsp_run_n,
        stats.bsp_input_gap_n,
        stats.bsp_ranges_unavailable,
        stats.keys,
        stats.material_ordinal_refused,
        stats.g0_world_surfs.len(),
        stats.rebinds,
        stats.surfaces,
        stats.triangles,
        stats.unculled_triangles,
        stats.lightmapped_triangles,
        stats.fallback_triangles,
        stats.single_cell,
        stats.submitted_batches,
        stats.rewritten_index_bytes,
        stats.visibility_changes,
        stats.aspect,
        stats.fov_deg,
    );
}

pub(crate) fn add_world_surfaces_frustum_only(
    dpvs: &crate::prepare::scene::world::WorldDpvs,
    frustum: &[[f32; 4]],
    camera_cell_vis_lsb: &[u32],
    camera_vis_all: bool,
    surface_vis: &mut [u8],
    smodel_vis: &mut [u8],
    draw_msb: &mut Vec<u32>,
) -> (AabbCullStats, u32) {
    let mut stats = AabbCullStats::default();
    let mut vis_data = DpvsVisData {
        surface_vis,
        smodel_vis,
    };
    let words = cell_caster_row_words(dpvs.cell_count);
    if draw_msb.len() != words {
        draw_msb.clear();
        draw_msb.resize(words, 0);
    } else {
        draw_msb.fill(0);
    }
    let draw_n = if dpvs.cell_caster_bits.is_empty() {
        for cell in 0..dpvs.cell_count {
            add_one_cell_frustum_only(dpvs, cell, frustum, &mut vis_data, &mut stats);
        }
        dpvs.cell_count as u32
    } else {
        let n = or_caster_rows_for_visible(
            &dpvs.cell_caster_bits,
            dpvs.cell_count,
            camera_cell_vis_lsb,
            camera_vis_all,
            &mut draw_msb[..],
        );
        for cell in 0..dpvs.cell_count {
            if !msb_get(&draw_msb, cell) {
                continue;
            }
            add_one_cell_frustum_only(dpvs, cell, frustum, &mut vis_data, &mut stats);
        }
        n
    };
    (stats, draw_n)
}

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
    let range_n = end.saturating_sub(begin);
    if end < begin {
        return BspDrawSurfCensus {
            input_gap_n: 1,
            ..BspDrawSurfCensus::default()
        };
    }
    let (Ok(begin_i), Ok(end_i)) = (usize::try_from(begin), usize::try_from(end)) else {
        return BspDrawSurfCensus {
            range_n,
            input_gap_n: range_n.max(1),
            ..BspDrawSurfCensus::default()
        };
    };
    if end_i > surface_vis.len() || end_i > draw_surfs.len() {
        let visible_n = surface_vis
            .get(begin_i..surface_vis.len().min(end_i))
            .map_or(0, |bytes| {
                bytes.iter().filter(|byte| **byte != 0).count() as u32
            });
        return BspDrawSurfCensus {
            range_n,
            visible_n,
            input_gap_n: range_n.max(1),
            ..BspDrawSurfCensus::default()
        };
    }

    let mut host_input_gap_n = 0u32;
    for surf in begin_i..end_i {
        if surface_vis[surf] == 0 || draw_surfs[surf].packed != 0 {
            continue;
        }
        surface_vis[surf] = 0;
        host_input_gap_n = host_input_gap_n.saturating_add(1);
        if let Ok(surf) = u16::try_from(surf) {
            g0_surfs.push(surf);
        }
    }

    runs.clear();
    let mut census =
        add_bsp_draw_surfs_camera(kind, begin, end, surface_vis, draw_surfs, item_surfs, runs);
    census.visible_n = census.visible_n.saturating_add(host_input_gap_n);
    census.input_gap_n = census.input_gap_n.saturating_add(host_input_gap_n);
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

fn add_one_cell_frustum_only(
    dpvs: &crate::prepare::scene::world::WorldDpvs,
    cell: usize,
    frustum: &[[f32; 4]],
    vis_data: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    let Some(tree) = dpvs.aabb_trees.get(cell) else {
        return;
    };
    let cull_input = AabbTreeCull {
        nodes: tree,
        smodel_indexes: dpvs
            .aabb_smodel_indices
            .get(cell)
            .map_or(&[][..], Vec::as_slice),
        sorted_surf_index: &dpvs.sorted_surf_index,
        surfaces_bounds: &dpvs.surface_bounds,
        smodel_bounds: &dpvs.smodel_bounds,
    };
    add_aabb_tree_surfaces_in_frustum(&cull_input, frustum, vis_data, stats);
}

pub(crate) fn packed_world_colour_key(
    cull: &crate::prepare::scene::world::WorldCull,
    surf: usize,
    baked_material_keys: &[Option<u64>],
    batch_primary_lights: &[u8],
) -> Option<u64> {
    let material_asset_id = cull.surface_materials.get(surf).copied().flatten()?;
    if let Some(word) = cull
        .capture
        .packed_draw_surfs
        .get(surf)
        .copied()
        .filter(|word| word.packed != 0)
    {
        return Some(word.packed);
    }
    let packed = baked_material_keys
        .get(material_asset_id.order())
        .copied()
        .flatten()?;
    let scene_light_index = if surf < cull.surface_primary_lights.len() {
        cull.surface_primary_lights[surf]
    } else {
        cull.surface_batch_ranges
            .get(surf)
            .and_then(|&(batch, _, _)| batch_primary_lights.get(batch).copied())
            .unwrap_or(0)
    };
    Some(crate::assemble::drawsurf::with_scene_light_index(
        packed,
        scene_light_index,
    ))
}

pub(crate) fn append_bmodel_colour_span(
    cull: &mut crate::prepare::scene::world::WorldCull,
    start_surf: u16,
    surface_count: u16,
    already: &mut HashSet<u16>,
    baked_material_keys: &[Option<u64>],
    batch_primary_lights: &[u8],
) -> (u32, u32) {
    let mut added = 0u32;
    let mut refused = 0u32;
    let start = usize::from(start_surf);
    let end = start.saturating_add(usize::from(surface_count));
    for surf in start..end {
        let Ok(surf_u16) = u16::try_from(surf) else {
            continue;
        };
        if !already.insert(surf_u16) {
            continue;
        }
        match packed_world_colour_key(cull, surf, baked_material_keys, batch_primary_lights) {
            Some(packed) => {
                cull.draw_items.push(WorldDrawItem {
                    key: packed,
                    surf: surf_u16,
                    run: 1,
                    list_start: cull.item_surfs.len() as u32,
                    entry_first: surf_u16,
                    kind: WorldDrawItemKind::BModel,
                });
                cull.item_surfs.push(surf_u16);
                added = added.saturating_add(1);
            }
            None => refused = refused.saturating_add(1),
        }
    }
    (added, refused)
}

pub(crate) fn refresh_draw_items_id(cull: &mut crate::prepare::scene::world::WorldCull) {
    let mut draw_items_id = crate::assemble::drawsurf::list::CONTENT_ID_SEED;
    crate::assemble::drawsurf::list::mix_content_id(
        &mut draw_items_id,
        cull.draw_items.len() as u64,
    );
    for item in &cull.draw_items {
        mix_draw_item_id(&mut draw_items_id, item, cull.draw_item_surfs(item));
    }
    cull.draw_items_id = draw_items_id;
}

fn mix_draw_item_id(id: &mut u64, item: &WorldDrawItem, item_surfs: &[u16]) {
    use crate::assemble::drawsurf::list::mix_content_id;
    mix_content_id(id, item.key);
    mix_content_id(id, u64::from(item.entry_first));
    mix_content_id(id, u64::from(item.run));
    for &surf in item_surfs {
        mix_content_id(id, u64::from(surf));
    }
    mix_content_id(
        id,
        match item.kind {
            WorldDrawItemKind::Bsp(asset_world::CameraRangeKind::LitOpaque) => 0,
            WorldDrawItemKind::Bsp(asset_world::CameraRangeKind::LitTrans) => 1,
            WorldDrawItemKind::Bsp(asset_world::CameraRangeKind::Emissive) => 2,
            WorldDrawItemKind::BModel => 3,
            WorldDrawItemKind::Bsp(asset_world::CameraRangeKind::Decal) => 4,
        },
    );
}

#[must_use]
pub(crate) fn draw_item_rebinds(items: &[WorldDrawItem]) -> u32 {
    let mut prev_mat: Option<u16> = None;
    let mut rebinds = 0u32;
    for item in items {
        let mat = dpvs_t6::GfxDrawSurf::from_packed(item.key).material_sorted_index();
        if prev_mat != Some(mat) {
            rebinds = rebinds.saturating_add(1);
            prev_mat = Some(mat);
        }
    }
    rebinds
}
