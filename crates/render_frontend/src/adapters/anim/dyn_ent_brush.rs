//! Which cells hold the map's moving brushes and the scene's entities, and
//! which of them the camera sees.
//!
//! Each frame the brushes and entities are filed into the cells their boxes
//! reach. The per-cell commands then walk a cell's rows and keep what the
//! cell's planes do not cull: brushes join the draw items, entities are
//! marked visible (and skinned) or deferred until their bounds are built.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use dpvs_t6::msb_iter;
use dpvs_t6::{
    Bounds, DpvsPlanes, GFX_CFG_ENT_COUNT, dyn_brush_scene_list_admits, dyn_ent_in_cell,
    filter_bmodel_into_cells, filter_dyn_ent_into_cells, scene_ent_cell_bits_len,
    scene_ent_cell_row, scene_ent_cell_row_second_pass, scene_ent_cell_walk_bits,
    write_dyn_brush_vis,
};
use render_frontend::{
    CellFrustumWorkerCmd, DpvsEntWorkerCmd, GfxScene, GfxSceneDobj, SCENE_INDEX_EMPTY,
    WORKER_CMD_CELL_DYN_BRUSH, WORKER_CMD_CELL_SCENE_ENT, WORKER_CMD_DPVS_ENT, WorkerCmdBusyInput,
    scene_info_entnum,
};

use crate::adapters::anim::dyn_ent::{
    DynEntBrushPrimaryLightVis, DynEntCellBits, ensure_primary_light_words,
    relink_dyn_ent_primary_lights,
};
use crate::prepare::scene::cull::{
    DpvsFrameStats, append_bmodel_colour_span, draw_item_rebinds, refresh_draw_items_id,
};
use crate::prepare::scene::gfx_scene::{
    HostGfxScene, SceneEntSkinInputs, SceneEntSurfaceCache, expand_scene_ent_pending,
};
use crate::prepare::scene::smodel_geom_cache::FrontendWorkerCmds;
use crate::prepare::scene::world::{WorldCull, WorldDynEntBrush, WorldScene, posed_brush_bounds};

/// The cells each moving brush reaches.
#[derive(Resource, Clone, Debug, Default)]
pub struct DynEntBrushCellBits {
    pub membership: DynEntCellBits,
}

/// The cells each scene entity reaches, in two banks (entities, then brush
/// models), and the bounds of every brush model filed.
#[derive(Resource, Clone, Debug, Default)]
pub struct SceneEntCellBits {
    pub cell_count: usize,
    pub bits: Vec<u32>,
    pub ent_info: Vec<Option<Bounds>>,
}

impl SceneEntCellBits {
    /// Size for `cell_count` cells; a change of size starts over empty.
    fn ensure(&mut self, cell_count: usize) {
        if self.cell_count == cell_count && self.is_sized() {
            return;
        }
        self.cell_count = cell_count;
        self.bits = vec![0; scene_ent_cell_bits_len(cell_count)];
        self.ent_info = vec![None; GFX_CFG_ENT_COUNT as usize];
    }

    fn is_sized(&self) -> bool {
        self.bits.len() == scene_ent_cell_bits_len(self.cell_count)
            && self.ent_info.len() == GFX_CFG_ENT_COUNT as usize
    }

    fn is_ready(&self) -> bool {
        self.cell_count > 0 && self.is_sized()
    }
}

pub fn register_dyn_ent_brush_systems(app: &mut App) {
    app.init_resource::<DynEntBrushCellBits>()
        .init_resource::<DynEntBrushPrimaryLightVis>()
        .init_resource::<SceneEntCellBits>()
        .add_systems(
            Update,
            (
                link_dyn_ent_brush_cells,
                exec_cell_dyn_brush_cmds
                    .after(link_dyn_ent_brush_cells)
                    .after(frame::WorkerCmdSet::CellStatic),
            )
                .in_set(frame::RenderSet::Anim)
                .in_set(frame::WorkerCmdSet::CellDynBrush),
        )
        .add_systems(
            Update,
            (
                size_scene_ent_cell_bits,
                link_scene_ents
                    .after(size_scene_ent_cell_bits)
                    .after(crate::prepare::scene::gfx_scene::GfxSceneAdd),
                exec_cell_scene_ent_cmds
                    .after(link_scene_ents)
                    .after(frame::WorkerCmdSet::CellStatic),
            )
                .in_set(frame::RenderSet::Anim)
                .in_set(frame::WorkerCmdSet::CellSceneEnt),
        )
        .add_systems(
            Update,
            drain_dpvs_ent_cmds
                .after(frame::WorkerCmdSet::CellSceneEnt)
                .in_set(frame::WorkerCmdSet::DpvsEnt),
        );
}

/// The cull data, if the world has a BSP to file things into.
fn dpvs_cull(scene: Option<&WorldScene>) -> Option<&WorldCull> {
    scene?
        .cull
        .as_ref()
        .filter(|cull| !cull.dpvs.nodes.is_empty() && cull.dpvs.cell_count != 0)
}

fn dpvs_planes(cull: &WorldCull) -> DpvsPlanes<'_> {
    DpvsPlanes {
        planes: &cull.dpvs.planes,
        nodes: &cull.dpvs.nodes,
        cell_count: cull.dpvs.cell_count as u32,
    }
}

/// The camera frustum's planes this frame; none before the first frame.
fn frustum_planes(stats: Option<&DpvsFrameStats>) -> Vec<[f32; 4]> {
    stats.map_or_else(Vec::new, |stats| stats.frustum_planes.clone())
}

/// One more than the highest brush index: the number of dyn ent clients the
/// brushes need room for.
fn client_count(brushes: &[WorldDynEntBrush]) -> u32 {
    brushes
        .iter()
        .map(|brush| u32::from(brush.index) + 1)
        .max()
        .unwrap_or(0)
}

/// The brush models of the brushes the cells found visible and that have a
/// surface to draw. `vis` holds a byte per brush index.
fn admitted_brush_models(brushes: &[WorldDynEntBrush], vis: &[u8]) -> Vec<u16> {
    brushes
        .iter()
        .filter(|brush| {
            let seen = vis.get(usize::from(brush.index)).copied().unwrap_or(0);
            dyn_brush_scene_list_admits(seen, brush.surface_count)
        })
        .map(|brush| brush.brush_model)
        .collect()
}

fn clear_brush_cells(membership: &mut DynEntBrushCellBits, vis: &mut DynEntBrushPrimaryLightVis) {
    membership.membership = DynEntCellBits::default();
    *vis = DynEntBrushPrimaryLightVis::default();
}

/// File every moving brush into the cells its box reaches, and its primary
/// light visibility.
fn link_dyn_ent_brush_cells(
    scene: Option<Res<WorldScene>>,
    atpoint: Res<render_scene::DynAtPointLookup>,
    mut cells: ResMut<DynEntBrushCellBits>,
    mut vis: ResMut<DynEntBrushPrimaryLightVis>,
) {
    let Some(scene) = scene.as_deref() else {
        clear_brush_cells(&mut cells, &mut vis);
        return;
    };
    let Some(cull) = dpvs_cull(Some(scene)) else {
        clear_brush_cells(&mut cells, &mut vis);
        return;
    };
    let dpvs = dpvs_planes(cull);
    let clients = client_count(&scene.dyn_ent_brushes);
    let membership = &mut cells.membership;
    let _ = membership.ensure(clients, cull.dpvs.cell_count);
    membership.bits.fill(0);
    vis.sun_primary = scene.sun_primary_light_count;
    vis.primary_count = scene.primary_light_cull.len() as u32;
    let (sun, primary_count) = (vis.sun_primary, vis.primary_count);
    ensure_primary_light_words(&mut vis.words, clients, sun, primary_count);
    vis.words.fill(0);
    for brush in &scene.dyn_ent_brushes {
        let Some(bounds) = brush.bounds else {
            continue;
        };
        let id = u32::from(brush.index);
        filter_dyn_ent_into_cells(
            &dpvs,
            bounds,
            &mut membership.bits,
            membership.word_count,
            id,
        );
        relink_dyn_ent_primary_lights(&mut vis.words, None, &atpoint, id, bounds, false);
    }
}

/// Walk the cells the camera sees: every brush filed in one that survives
/// its planes is visible, and the visible brushes with a surface join the
/// draw items.
fn exec_cell_dyn_brush_cmds(
    mut scene: Option<ResMut<WorldScene>>,
    mut stats: Option<ResMut<DpvsFrameStats>>,
    cells: Res<DynEntBrushCellBits>,
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
) {
    let planes = frustum_planes(stats.as_deref());
    let brushes: &[WorldDynEntBrush] = scene
        .as_deref()
        .map_or(&[], |scene| scene.dyn_ent_brushes.as_slice());
    let filed = &cells.membership;
    let mut vis = vec![0u8; client_count(brushes) as usize];
    let _ = worker_cmds.queues.wait_of_type(
        WORKER_CMD_CELL_DYN_BRUSH,
        WorkerCmdBusyInput::default(),
        |data| {
            let Some(cmd) = CellFrustumWorkerCmd::from_bytes(data) else {
                return;
            };
            if !filed.is_ready() {
                return;
            }
            let cell = cmd.cell as usize;
            for brush in brushes {
                let Some(bounds) = brush.bounds else {
                    continue;
                };
                if !dyn_ent_in_cell(&filed.bits, filed.word_count, cell, u32::from(brush.index)) {
                    continue;
                }
                if let Some(slot) = vis.get_mut(usize::from(brush.index)) {
                    write_dyn_brush_vis(slot, bounds, &planes);
                }
            }
        },
    );
    let admitted = admitted_brush_models(brushes, &vis);
    let Some(scene) = scene.as_deref_mut() else {
        return;
    };
    append_brush_models(scene, stats.as_deref_mut(), &admitted);
}

/// Add the surfaces of the brush models to the draw items. Model 0 is the
/// world itself and is skipped.
fn append_brush_models(scene: &mut WorldScene, stats: Option<&mut DpvsFrameStats>, models: &[u16]) {
    if models.iter().all(|&model| model == 0) {
        return;
    }
    let baked_keys: Vec<Option<u64>> = scene
        .runtime_material_catalog
        .materials
        .iter()
        .map(|material| material.baked_draw_surf)
        .collect();
    let batch_lights: Vec<u8> = scene
        .batches
        .iter()
        .map(|batch| batch.primary_light_index)
        .collect();
    let Some(cull) = scene.cull.as_mut() else {
        return;
    };
    let mut already: HashSet<u16> = cull.item_surfs.iter().copied().collect();
    let mut added = 0u32;
    for &model in models.iter().filter(|&&model| model != 0) {
        let Some(span) = cull.brush_models.get(usize::from(model)).copied() else {
            continue;
        };
        let (span_added, _) = append_bmodel_colour_span(
            cull,
            span.start_surf,
            span.surface_count,
            &mut already,
            &baked_keys,
            &batch_lights,
        );
        added = added.saturating_add(span_added);
    }
    if added == 0 {
        return;
    }
    refresh_draw_items_id(cull);
    if let Some(stats) = stats {
        stats.keys = cull.draw_items.len() as u32;
        stats.rebinds = draw_item_rebinds(&cull.draw_items);
    }
}

fn size_scene_ent_cell_bits(scene: Option<Res<WorldScene>>, mut bits: ResMut<SceneEntCellBits>) {
    match scene.as_deref().and_then(|scene| scene.cull.as_ref()) {
        Some(cull) => bits.ensure(cull.dpvs.cell_count),
        None => *bits = SceneEntCellBits::default(),
    }
}

/// What the entities' last filing was, so an entity that has not moved is
/// not walked down the BSP again.
#[derive(Default)]
struct SceneEntCellCache {
    /// Which BSP the cells belong to: its nodes' address and length.
    tree: (usize, usize),
    by_ent: HashMap<u32, ([u32; 6], Vec<u32>)>,
    /// The entities filed this frame; one that is filed twice leaves its
    /// first cells.
    linked: HashSet<u32>,
}

/// A box as the bit patterns of its middle and half size.
fn bounds_key(bounds: Bounds) -> [u32; 6] {
    let (mid, half) = (bounds.mid(), bounds.half());
    [mid[0], mid[1], mid[2], half[0], half[1], half[2]].map(f32::to_bits)
}

impl SceneEntCellCache {
    /// Forget everything if the BSP is not the one the cache was built for.
    fn follow_tree(&mut self, tree: (usize, usize)) {
        if self.tree != tree {
            *self = Self {
                tree,
                ..Self::default()
            };
        }
        self.linked.clear();
    }

    fn link(&mut self, dpvs: &DpvsPlanes<'_>, bits: &mut [u32], bounds: Bounds, ent_id: u32) {
        let key = bounds_key(bounds);
        if !self.linked.insert(ent_id) {
            dpvs_t6::unfilter_scene_ent_from_cells_view0(bits, dpvs.cell_count as usize, ent_id);
        }
        let entry = self
            .by_ent
            .entry(ent_id)
            .or_insert_with(|| (key, Vec::new()));
        if entry.0 != key || entry.1.is_empty() {
            entry.0 = key;
            entry.1.clear();
            let found = &mut entry.1;
            dpvs_t6::scene_ent_cells(dpvs, bounds, &mut |cell| found.push(cell));
        }
        for &cell in &entry.1 {
            dpvs_t6::add_scene_ent_to_cell(bits, cell as usize, ent_id);
        }
    }
}

/// The box an entity is filed by: its posed bounds, or failing that the cube
/// around its sphere.
fn scene_ent_pose_bounds(
    origin: [f32; 3],
    radius: Option<f32>,
    posed_bounds: Option<Bounds>,
) -> Option<Bounds> {
    posed_bounds.or_else(|| radius.map(|r| Bounds::from_mid_half(origin, [r; 3])))
}

/// File the scene's models, dobjs and brush models into the cells.
fn link_scene_ents(
    scene: Option<Res<WorldScene>>,
    gfx: Res<HostGfxScene>,
    mut bits: ResMut<SceneEntCellBits>,
    mut cache: Local<SceneEntCellCache>,
) {
    if !bits.is_ready() {
        return;
    }
    bits.bits.fill(0);
    bits.ent_info.fill(None);
    let Some(cull) = dpvs_cull(scene.as_deref()) else {
        return;
    };
    let dpvs = dpvs_planes(cull);
    cache.follow_tree((cull.dpvs.nodes.as_ptr() as usize, cull.dpvs.nodes.len()));
    let scene = &gfx.scene;
    for model in &scene.scene_models {
        if let Some(bounds) = scene_ent_pose_bounds(model.origin, model.radius, model.posed_bounds)
        {
            cache.link(&dpvs, &mut bits.bits, bounds, scene_info_entnum(model.info));
        }
    }
    for dobj in &scene.scene_dobjs {
        if let Some(bounds) = scene_ent_pose_bounds(dobj.origin, dobj.radius, dobj.posed_bounds) {
            cache.link(&dpvs, &mut bits.bits, bounds, scene_info_entnum(dobj.info));
        }
    }
    for brush in &scene.scene_brushes {
        let Some(ent_id) = brush.param_4.map(u32::from) else {
            continue;
        };
        let Some(local) = cull.brush_model_bounds.get(brush.model_index as usize) else {
            continue;
        };
        let quat = brush.quat.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let bounds = posed_brush_bounds(brush.origin, quat, *local);
        filter_bmodel_into_cells(&dpvs, bounds, &mut bits.bits, ent_id);
        if let Some(slot) = bits.ent_info.get_mut(ent_id as usize) {
            *slot = Some(bounds);
        }
    }
}

/// Whether the filing of `bounds` puts it in `cell`; without a BSP, always.
fn reaches_cell(dpvs: Option<&DpvsPlanes<'_>>, bounds: Bounds, cell: usize) -> bool {
    dpvs.is_none_or(|dpvs| dpvs_t6::scene_ent_box_reaches_cell(dpvs, bounds, cell))
}

/// The skin inputs and surface cache entities are skinned from.
struct SkinSources<'a> {
    inputs: &'a mut SceneEntSkinInputs,
    surfaces: &'a mut SceneEntSurfaceCache,
}

/// Skin a dobj entity the first time it is seen this frame, then mark the
/// entity visible.
fn skin_scene_ent_and_mark(scene: &mut GfxScene, skin: &mut SkinSources<'_>, entnum: u32) {
    let first_sight = scene.scene_dobj(entnum).is_some_and(|dobj| {
        let mut gate = dobj.cull_gate;
        dpvs_t6::scene_dobj_skin_begin(&mut gate)
    });
    if first_sight {
        let ready = skin.inputs.by_ent.get(&entnum).cloned();
        let input = ready.or_else(|| {
            let pending = skin.inputs.pending.get(&entnum)?;
            Some(expand_scene_ent_pending(pending, skin.surfaces))
        });
        let mut entries = Vec::new();
        let used = skin.inputs.frame_bytes_used;
        let summary = input
            .as_ref()
            .map_or_else(dpvs_t6::PreSkinSummary::default, |input| {
                let models: Vec<_> = input
                    .models
                    .iter()
                    .map(|model| dpvs_t6::PreSkinModel {
                        lod: model.lod,
                        bone_count: model.bone_count,
                        surfaces: &model.surfaces,
                    })
                    .collect();
                dpvs_t6::pre_skin_scene_ent(&models, &input.hide_part_bits, used, |entry| {
                    entries.push(entry);
                })
            });
        skin.inputs.frame_bytes_used = used.saturating_add(summary.bytes);
        scene.store_scene_ent_skin(
            entnum,
            render_frontend::SceneEntSkinnedSurfs { entries, summary },
        );
    }
    scene.mark_scene_ent_visible(entnum);
}

/// What a cell's walk does with a dobj in the cell's entity row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DobjWalk {
    /// Culled, or nothing left to do for it.
    Skip,
    /// Its bounds are not built yet: the bounding job decides.
    Defer,
    /// In sight and not yet skinned.
    Skin,
}

/// Decide a dobj's walk. Until its bounds are built only its sphere is
/// tested, against the cell's child planes; after, its box is tested against
/// those and then every plane of the cell.
fn walk_dobj(
    dobj: &GfxSceneDobj,
    child_planes: &[[f32; 4]],
    cell_planes: &[[f32; 4]],
    reaches: impl FnOnce(Bounds) -> bool,
) -> DobjWalk {
    let (Some(radius), Some(bounds)) = (dobj.radius, dobj.posed_bounds) else {
        return DobjWalk::Defer;
    };
    let unbounded = dpvs_t6::scene_dobj_bounds_pending(dobj.cull_gate);
    let culled = if unbounded {
        dpvs_t6::scene_ent_sphere_culled(dobj.origin, radius, child_planes)
    } else {
        dpvs_t6::bounds_culled(bounds, child_planes)
    };
    if culled {
        return DobjWalk::Skip;
    }
    if unbounded {
        return DobjWalk::Defer;
    }
    if dpvs_t6::bounds_culled(bounds, cell_planes)
        || dobj.cull_gate == dpvs_t6::SCENE_DOBJ_CULL_DONE
        || !reaches(bounds)
    {
        return DobjWalk::Skip;
    }
    DobjWalk::Skin
}

/// A model is out of sight only when it has a sphere and the planes cull it.
fn model_culled(origin: [f32; 3], radius: Option<f32>, planes: &[[f32; 4]]) -> bool {
    radius.is_some_and(|radius| dpvs_t6::scene_ent_sphere_culled(origin, radius, planes))
}

/// The job that bounds `dobj_slot` in `cell`, for the next stage.
fn bounding_job(dobj_slot: u16, cmd: &CellFrustumWorkerCmd) -> DpvsEntWorkerCmd {
    DpvsEntWorkerCmd {
        scene_dobj: u32::from(dobj_slot),
        plane_count: u16::from(cmd.plane_count),
        cell: u16::try_from(cmd.cell).unwrap_or(u16::MAX),
    }
}

/// The planes a cell is culled by: its own clip, else the whole frustum.
fn cell_planes(stats: Option<&DpvsFrameStats>, cell: usize, frustum: &[[f32; 4]]) -> Vec<[f32; 4]> {
    stats
        .and_then(|stats| stats.cell_clips.get(cell))
        .filter(|clip| clip.plane_count > 0)
        .map_or_else(|| frustum.to_vec(), |clip| clip.as_slice().to_vec())
}

/// What one frame's walks of the cells need besides the scene.
struct CellWalkInputs<'a> {
    bits: &'a SceneEntCellBits,
    stats: Option<&'a DpvsFrameStats>,
    frustum: &'a [[f32; 4]],
    dpvs: Option<DpvsPlanes<'a>>,
}

/// Walk one cell's rows: the entities row, then the brush models row. The
/// dobjs that need their bounds built are added to `jobs`.
fn walk_cell(
    scene: &mut GfxScene,
    walk: &CellWalkInputs<'_>,
    skin: &mut SkinSources<'_>,
    cmd: &CellFrustumWorkerCmd,
    jobs: &mut Vec<DpvsEntWorkerCmd>,
) {
    let cell = cmd.cell as usize;
    let cell_count = walk.bits.cell_count;
    if cell >= cell_count {
        return;
    }
    scene.scene_ent_walked = true;
    let rows = [
        (false, scene_ent_cell_row(&walk.bits.bits, cell)),
        (
            true,
            scene_ent_cell_row_second_pass(&walk.bits.bits, cell_count, cell),
        ),
    ];
    let bit_count = scene_ent_cell_walk_bits(GFX_CFG_ENT_COUNT);
    let cell_planes = cell_planes(walk.stats, cell, walk.frustum);
    let child_planes =
        dpvs_t6::scene_ent_child_planes(&cell_planes, cmd.plane_begin, cmd.plane_count);
    for (second_pass, row) in rows {
        for entnum in msb_iter(row, bit_count) {
            let id = entnum as u32;
            if scene.scene_ent_visible(id) {
                continue;
            }
            if second_pass {
                let bounds = walk.bits.ent_info.get(entnum).copied().flatten();
                if bounds.is_some_and(|bounds| !dpvs_t6::bounds_culled(bounds, child_planes)) {
                    scene.mark_scene_ent_visible(id);
                }
                continue;
            }
            let slot_of = |index: &[u16]| {
                index
                    .get(entnum)
                    .copied()
                    .filter(|&slot| slot != SCENE_INDEX_EMPTY)
            };
            if let Some(slot) = slot_of(&scene.scene_dobj_index) {
                let Some(dobj) = scene.scene_dobjs.get(usize::from(slot)) else {
                    continue;
                };
                let step = walk_dobj(dobj, child_planes, &cell_planes, |bounds| {
                    reaches_cell(walk.dpvs.as_ref(), bounds, cell)
                });
                match step {
                    DobjWalk::Skip => {}
                    DobjWalk::Defer => jobs.push(bounding_job(slot, cmd)),
                    DobjWalk::Skin => skin_scene_ent_and_mark(scene, skin, id),
                }
                continue;
            }
            let Some(slot) = slot_of(&scene.scene_model_index) else {
                continue;
            };
            let Some(model) = scene.scene_models.get(usize::from(slot)) else {
                continue;
            };
            if !model_culled(model.origin, model.radius, child_planes) {
                scene.mark_scene_ent_visible(id);
            }
        }
    }
}

/// Walk the cells the camera sees for entities, and queue the bounding jobs
/// for the dobjs that are not bounded yet.
fn exec_cell_scene_ent_cmds(
    bits: Res<SceneEntCellBits>,
    mut gfx: ResMut<HostGfxScene>,
    mut skin_inputs: ResMut<SceneEntSkinInputs>,
    mut surface_cache: ResMut<SceneEntSurfaceCache>,
    world: Option<Res<WorldScene>>,
    stats: Option<Res<DpvsFrameStats>>,
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
) {
    let _ = worker_cmds.queues.reset_type(WORKER_CMD_DPVS_ENT);
    let ready = bits.is_ready();
    gfx.scene.scene_ent_walked = false;
    let frustum = frustum_planes(stats.as_deref());
    let walk = CellWalkInputs {
        bits: &bits,
        stats: stats.as_deref(),
        frustum: &frustum,
        dpvs: world
            .as_deref()
            .and_then(|world| world.cull.as_ref())
            .map(dpvs_planes),
    };
    let mut skin = SkinSources {
        inputs: &mut skin_inputs,
        surfaces: &mut surface_cache,
    };
    let mut jobs = Vec::new();
    let scene = &mut gfx.scene;
    let _ = worker_cmds.queues.wait_of_type(
        WORKER_CMD_CELL_SCENE_ENT,
        WorkerCmdBusyInput::default(),
        |data| {
            if !ready {
                return;
            }
            if let Some(cmd) = CellFrustumWorkerCmd::from_bytes(data) {
                walk_cell(scene, &walk, &mut skin, &cmd, &mut jobs);
            }
        },
    );
    for job in &jobs {
        let _ = worker_cmds.queues.add(WORKER_CMD_DPVS_ENT, &job.to_bytes());
    }
}

/// Run the bounding jobs: build the dobj's bounds, and if it is in sight of
/// the cell that asked, skin it and mark it visible.
fn drain_dpvs_ent_cmds(
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
    mut gfx: ResMut<HostGfxScene>,
    mut skin_inputs: ResMut<SceneEntSkinInputs>,
    mut surface_cache: ResMut<SceneEntSurfaceCache>,
    world: Option<Res<WorldScene>>,
    stats: Option<Res<DpvsFrameStats>>,
) {
    let frustum = frustum_planes(stats.as_deref());
    let dpvs = world
        .as_deref()
        .and_then(|world| world.cull.as_ref())
        .map(dpvs_planes);
    let mut skin = SkinSources {
        inputs: &mut skin_inputs,
        surfaces: &mut surface_cache,
    };
    let scene = &mut gfx.scene;
    let _ = worker_cmds.queues.wait_of_type(
        WORKER_CMD_DPVS_ENT,
        WorkerCmdBusyInput::default(),
        |data| {
            let Some(job) = DpvsEntWorkerCmd::from_bytes(data) else {
                return;
            };
            if let Some(entnum) = bound_dobj(scene, &job, &frustum, dpvs.as_ref()) {
                skin_scene_ent_and_mark(scene, &mut skin, entnum);
            }
        },
    );
}

/// Claim and bound the job's dobj. The entity number to skin, if the dobj is
/// bounded and in sight of the job's cell.
fn bound_dobj(
    scene: &mut GfxScene,
    job: &DpvsEntWorkerCmd,
    frustum: &[[f32; 4]],
    dpvs: Option<&DpvsPlanes<'_>>,
) -> Option<u32> {
    let dobj = scene.scene_dobjs.get_mut(job.scene_dobj as usize)?;
    if !dpvs_t6::scene_dobj_cull_begin(&mut dobj.cull_gate) {
        return None;
    }
    let Some(bounds) = dobj.posed_bounds else {
        dobj.cull_gate = dpvs_t6::SCENE_DOBJ_CULL_DONE;
        return None;
    };
    dobj.cull_gate = dpvs_t6::SCENE_DOBJ_CULL_BOUNDED;
    let in_sight = !dpvs_t6::bounds_culled(bounds, frustum)
        && reaches_cell(dpvs, bounds, usize::from(job.cell));
    in_sight.then(|| scene_info_entnum(dobj.info))
}

#[cfg(test)]
mod tests {
    use dpvs_t6::CPlane;

    use super::*;

    fn brush(index: u16, brush_model: u16, surface_count: u16) -> WorldDynEntBrush {
        WorldDynEntBrush {
            index,
            origin: [0.0; 3],
            bounds: None,
            brush_model,
            surface_count,
        }
    }

    fn dobj(radius: Option<f32>, bounds: Option<Bounds>, gate: u32) -> GfxSceneDobj {
        GfxSceneDobj {
            origin: [0.0; 3],
            lighting_origin: [0.0; 3],
            radius,
            info: 0,
            quat: None,
            cull_gate: gate,
            posed_bounds: bounds,
            lods: Vec::new(),
            skinned_surfs: None,
        }
    }

    /// Looking down +x from the origin: only x > 0 is in front.
    const FRONT: [[f32; 4]; 1] = [[1.0, 0.0, 0.0, 0.0]];

    fn cube(mid: [f32; 3], half: f32) -> Bounds {
        Bounds::from_mid_half(mid, [half; 3])
    }

    fn axial(dist: f32) -> CPlane {
        CPlane {
            normal: [1.0, 0.0, 0.0],
            dist,
            r#type: 0,
        }
    }

    // x = 0 splits cells 0 (front) and 1 (back); behind it x = -10 splits 1
    // and 2.
    const PLANES: [CPlane; 2] = [
        CPlane {
            normal: [1.0, 0.0, 0.0],
            dist: 0.0,
            r#type: 0,
        },
        CPlane {
            normal: [1.0, 0.0, 0.0],
            dist: -10.0,
            r#type: 0,
        },
    ];
    const NODES: [u16; 7] = [4, 3, 1, 5, 3, 2, 3];

    fn bsp() -> DpvsPlanes<'static> {
        DpvsPlanes {
            planes: &PLANES,
            nodes: &NODES,
            cell_count: 3,
        }
    }

    /// Spans x from -5 to 5: cells 0 and 1, not the one behind x = -10.
    fn straddling_zero() -> Bounds {
        Bounds::from_mins_maxs([-5.0, 0.0, 0.0], [5.0, 1.0, 1.0])
    }

    fn rows_holding(bits: &[u32], ent: usize) -> Vec<usize> {
        (0..3)
            .filter(|&cell| msb_iter(scene_ent_cell_row(bits, cell), 64).any(|id| id == ent))
            .collect()
    }

    #[test]
    fn the_test_tree_is_the_one_the_planes_describe() {
        assert_eq!(axial(0.0), PLANES[0]);
        let mut cells = Vec::new();
        dpvs_t6::scene_ent_cells(&bsp(), straddling_zero(), &mut |c| cells.push(c));
        assert_eq!(cells, vec![0, 1]);
    }

    #[test]
    fn scene_ent_bits_are_sized_by_cell_count_and_ready_only_with_cells() {
        let mut bits = SceneEntCellBits::default();
        assert!(!bits.is_ready());
        bits.ensure(0);
        assert!(!bits.is_ready(), "no cells is not ready");
        bits.ensure(4);
        assert!(bits.is_ready());
        assert_eq!(bits.bits.len(), scene_ent_cell_bits_len(4));
        assert_eq!(bits.ent_info.len(), GFX_CFG_ENT_COUNT as usize);
    }

    #[test]
    fn ensuring_the_same_size_keeps_the_bits_and_a_new_size_clears_them() {
        let mut bits = SceneEntCellBits::default();
        bits.ensure(2);
        bits.bits[1] = 7;
        bits.ent_info[3] = Some(cube([0.0; 3], 1.0));
        bits.ensure(2);
        assert_eq!(bits.bits[1], 7);
        assert!(bits.ent_info[3].is_some());
        bits.ensure(5);
        assert!(bits.bits.iter().all(|&w| w == 0));
        assert!(bits.ent_info.iter().all(Option::is_none));
    }

    #[test]
    fn a_resized_vector_is_not_ready() {
        let mut bits = SceneEntCellBits::default();
        bits.ensure(3);
        bits.bits.pop();
        assert!(!bits.is_ready());
    }

    #[test]
    fn the_client_count_is_one_past_the_highest_brush_index() {
        assert_eq!(client_count(&[]), 0);
        assert_eq!(
            client_count(&[brush(4, 1, 1), brush(9, 2, 1), brush(0, 3, 1)]),
            10
        );
    }

    #[test]
    fn only_visible_brushes_with_surfaces_are_admitted() {
        let brushes = [
            brush(0, 5, 2),
            brush(1, 6, 0),
            brush(2, 7, 3),
            brush(9, 8, 1),
        ];
        let vis = [1, 1, 0];
        assert_eq!(admitted_brush_models(&brushes, &vis), vec![5]);
        assert!(admitted_brush_models(&brushes, &[]).is_empty());
    }

    #[test]
    fn brush_visibility_is_written_once_and_kept() {
        let mut seen = 0u8;
        write_dyn_brush_vis(&mut seen, cube([-50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 0, "behind the plane");
        write_dyn_brush_vis(&mut seen, cube([50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 1);
        write_dyn_brush_vis(&mut seen, cube([-50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 1, "a later cell cannot hide it");
    }

    #[test]
    fn the_frustum_is_empty_before_the_first_frame() {
        assert!(frustum_planes(None).is_empty());
        let stats = DpvsFrameStats {
            frustum_planes: FRONT.to_vec(),
            ..Default::default()
        };
        assert_eq!(frustum_planes(Some(&stats)), FRONT.to_vec());
    }

    #[test]
    fn a_cell_without_its_own_clip_uses_the_frustum() {
        assert_eq!(cell_planes(None, 0, &FRONT), FRONT.to_vec());
        let stats = DpvsFrameStats::default();
        assert_eq!(cell_planes(Some(&stats), 7, &FRONT), FRONT.to_vec());
    }

    #[test]
    fn posed_bounds_win_over_the_sphere_and_no_size_means_no_box() {
        let posed = cube([1.0, 2.0, 3.0], 4.0);
        let origin = [9.0, 9.0, 9.0];
        assert_eq!(
            scene_ent_pose_bounds(origin, Some(2.0), Some(posed)),
            Some(posed)
        );
        assert_eq!(
            scene_ent_pose_bounds(origin, Some(2.0), None),
            Some(cube(origin, 2.0))
        );
        assert_eq!(scene_ent_pose_bounds(origin, None, None), None);
    }

    #[test]
    fn bounds_keys_differ_by_any_component() {
        let a = bounds_key(cube([1.0, 2.0, 3.0], 1.0));
        assert_eq!(a, bounds_key(cube([1.0, 2.0, 3.0], 1.0)));
        assert_ne!(a, bounds_key(cube([1.0, 2.0, 3.5], 1.0)));
        assert_ne!(a, bounds_key(cube([1.0, 2.0, 3.0], 1.5)));
    }

    #[test]
    fn without_a_bsp_every_cell_is_reached() {
        assert!(reaches_cell(None, cube([0.0; 3], 1.0), 99));
    }

    #[test]
    fn with_a_bsp_only_the_cells_the_box_reaches_count() {
        let bsp = bsp();
        let near = straddling_zero();
        assert!(reaches_cell(Some(&bsp), near, 0));
        assert!(reaches_cell(Some(&bsp), near, 1));
        assert!(!reaches_cell(Some(&bsp), near, 2));
    }

    #[test]
    fn an_entity_is_filed_in_the_cells_it_reaches() {
        let bsp = bsp();
        let mut bits = vec![0; scene_ent_cell_bits_len(3)];
        let mut cache = SceneEntCellCache::default();
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, straddling_zero(), 37);
        assert_eq!(rows_holding(&bits, 37), vec![0, 1]);
    }

    #[test]
    fn a_moved_entity_is_filed_again_and_a_still_one_reuses_its_cells() {
        let bsp = bsp();
        let mut bits = vec![0; scene_ent_cell_bits_len(3)];
        let mut cache = SceneEntCellCache::default();
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![0]);
        bits.fill(0);
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![0], "same box, same cells");
        bits.fill(0);
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, cube([-15.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![2]);
    }

    #[test]
    fn an_entity_filed_twice_in_a_frame_keeps_only_the_last_filing() {
        let bsp = bsp();
        let mut bits = vec![0; scene_ent_cell_bits_len(3)];
        let mut cache = SceneEntCellCache::default();
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 8);
        cache.link(&bsp, &mut bits, cube([-15.0, 0.0, 0.0], 1.0), 8);
        assert_eq!(rows_holding(&bits, 8), vec![2]);
    }

    #[test]
    fn a_different_tree_empties_the_cache() {
        let bsp = bsp();
        let mut bits = vec![0; scene_ent_cell_bits_len(3)];
        let mut cache = SceneEntCellCache::default();
        cache.follow_tree((1, 7));
        cache.link(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert!(!cache.by_ent.is_empty());
        cache.follow_tree((2, 7));
        assert!(cache.by_ent.is_empty() && cache.linked.is_empty());
        assert_eq!(cache.tree, (2, 7));
    }

    #[test]
    fn a_dobj_without_a_sphere_or_box_waits_for_its_bounds() {
        let planes = FRONT;
        let walk = |d: &GfxSceneDobj| walk_dobj(d, &planes, &planes, |_| true);
        assert_eq!(
            walk(&dobj(None, Some(cube([5.0; 3], 1.0)), 0)),
            DobjWalk::Defer
        );
        assert_eq!(walk(&dobj(Some(1.0), None, 0)), DobjWalk::Defer);
    }

    #[test]
    fn an_unbounded_dobj_is_culled_by_its_sphere_and_otherwise_deferred() {
        let planes = FRONT;
        let mut behind = dobj(
            Some(1.0),
            Some(cube([5.0; 3], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_IDLE,
        );
        behind.origin = [-10.0, 0.0, 0.0];
        assert_eq!(
            walk_dobj(&behind, &planes, &planes, |_| true),
            DobjWalk::Skip
        );
        let mut ahead = behind.clone();
        ahead.origin = [10.0, 0.0, 0.0];
        assert_eq!(
            walk_dobj(&ahead, &planes, &planes, |_| true),
            DobjWalk::Defer
        );
    }

    #[test]
    fn a_bounded_dobj_in_sight_of_its_cell_is_skinned() {
        let planes = FRONT;
        let seen = dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED,
        );
        assert_eq!(walk_dobj(&seen, &planes, &planes, |_| true), DobjWalk::Skin);
        assert_eq!(
            walk_dobj(&seen, &planes, &planes, |_| false),
            DobjWalk::Skip
        );
    }

    #[test]
    fn a_bounded_dobj_is_tested_against_child_planes_then_every_cell_plane() {
        let behind = [[-1.0, 0.0, 0.0, 0.0]];
        let seen = dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED,
        );
        assert_eq!(walk_dobj(&seen, &behind, &FRONT, |_| true), DobjWalk::Skip);
        assert_eq!(walk_dobj(&seen, &FRONT, &behind, |_| true), DobjWalk::Skip);
        assert_eq!(walk_dobj(&seen, &[], &[], |_| true), DobjWalk::Skin);
    }

    #[test]
    fn a_dobj_already_skinned_has_nothing_left_to_do() {
        let done = dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_DONE,
        );
        assert_eq!(walk_dobj(&done, &FRONT, &FRONT, |_| true), DobjWalk::Skip);
    }

    #[test]
    fn the_reach_test_runs_only_once_the_dobj_survives_the_planes() {
        let culled = dobj(
            Some(1.0),
            Some(cube([-5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED,
        );
        let step = walk_dobj(&culled, &FRONT, &FRONT, |_| panic!("not asked"));
        assert_eq!(step, DobjWalk::Skip);
    }

    #[test]
    fn a_model_is_culled_only_with_a_sphere_behind_a_plane() {
        assert!(model_culled([-10.0, 0.0, 0.0], Some(1.0), &FRONT));
        assert!(!model_culled([10.0, 0.0, 0.0], Some(1.0), &FRONT));
        assert!(!model_culled([-10.0, 0.0, 0.0], None, &FRONT));
    }

    #[test]
    fn a_bounding_job_carries_the_slot_planes_and_cell() {
        let cmd = CellFrustumWorkerCmd {
            cell: 70_000,
            plane_count: 5,
            plane_begin: 2,
            view: 0,
        };
        let job = bounding_job(12, &cmd);
        assert_eq!(
            job,
            DpvsEntWorkerCmd {
                scene_dobj: 12,
                plane_count: 5,
                cell: u16::MAX,
            }
        );
        let near = CellFrustumWorkerCmd { cell: 9, ..cmd };
        assert_eq!(bounding_job(1, &near).cell, 9);
    }

    #[test]
    fn a_dobj_that_cannot_be_claimed_is_left_alone() {
        let mut scene = GfxScene::default();
        scene.scene_dobjs.push(dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_DONE,
        ));
        let job = DpvsEntWorkerCmd {
            scene_dobj: 0,
            plane_count: 0,
            cell: 0,
        };
        assert_eq!(bound_dobj(&mut scene, &job, &[], None), None);
        assert_eq!(
            scene.scene_dobjs[0].cull_gate,
            dpvs_t6::SCENE_DOBJ_CULL_DONE
        );
        let missing = DpvsEntWorkerCmd {
            scene_dobj: 4,
            ..job
        };
        assert_eq!(bound_dobj(&mut scene, &missing, &[], None), None);
    }

    #[test]
    fn a_claimed_dobj_without_bounds_is_finished() {
        let mut scene = GfxScene::default();
        scene
            .scene_dobjs
            .push(dobj(Some(1.0), None, dpvs_t6::SCENE_DOBJ_CULL_IDLE));
        let job = DpvsEntWorkerCmd {
            scene_dobj: 0,
            plane_count: 0,
            cell: 0,
        };
        assert_eq!(bound_dobj(&mut scene, &job, &[], None), None);
        assert_eq!(
            scene.scene_dobjs[0].cull_gate,
            dpvs_t6::SCENE_DOBJ_CULL_DONE
        );
    }

    #[test]
    fn a_claimed_dobj_is_bounded_and_skinned_only_in_sight() {
        let make = |x: f32| {
            let mut scene = GfxScene::default();
            let mut d = dobj(
                Some(1.0),
                Some(cube([x, 0.0, 0.0], 1.0)),
                dpvs_t6::SCENE_DOBJ_CULL_IDLE,
            );
            d.info = 5;
            scene.scene_dobjs.push(d);
            scene
        };
        let job = DpvsEntWorkerCmd {
            scene_dobj: 0,
            plane_count: 0,
            cell: 0,
        };
        let mut seen = make(5.0);
        assert_eq!(
            bound_dobj(&mut seen, &job, &FRONT, None),
            Some(scene_info_entnum(5))
        );
        assert_eq!(
            seen.scene_dobjs[0].cull_gate,
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED
        );
        let mut hidden = make(-5.0);
        assert_eq!(bound_dobj(&mut hidden, &job, &FRONT, None), None);
        assert_eq!(
            hidden.scene_dobjs[0].cull_gate,
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED
        );
    }
}
