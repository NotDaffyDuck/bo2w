//! Cell filing for moving brushes and scene entities, and the per-cell walks
//! that decide what the camera can see.
//!
//! Every frame each moving brush and each scene entity is dropped into the
//! portal cells its box overlaps. When the frame's cell commands run, the
//! occupants of a cell are tested against that cell's planes: brushes that
//! pass put their surfaces on the draw list, entities that pass are flagged
//! visible (a dobj is skinned first), and a dobj whose posed box is not built
//! yet is handed to a bounding job that finishes the test.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use dpvs_t6::{Bounds, VisibilityCellPlanes, msb_iter};
use frame::{DrawPassStage, JobQueueStage};
use render_frontend::{RoomViewVolumeTaskOrder, SightActorTaskOrder, VisualStage, VisualStagePuppet};
use render_scene::{StageActorSkinnedFacets, stage_info_actor_num};

use super::debris_piece;
use crate::prepare::scene::cull;
use crate::prepare::scene::cull::SightFrameStats;
use crate::prepare::scene::visual_stage::{
    HostVisualStage, RenderSceneInsert, EntitySkinningInputSet, EntitySurfaceMemo,
    grow_pending_scene_entities,
};
use crate::prepare::scene::prop_geom_memo::FrontendTaskOrders;
use crate::prepare::scene::world::{LevelHide, LevelDebrisSolid, LevelStage};

/// Entity slots a cell row can hold.
const ENT_SLOTS: usize = dpvs_t6::VISUAL_CFG_ACTOR_TALLY as usize;

/// Which cells every moving brush overlaps this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct MovingBrushCells {
    pub filing: debris_piece::DebrisRoomBits,
}

/// Cell rows for the scene's entities: one bank for models and dobjs, a
/// second for brush models, plus the world box of each brush model filed.
#[derive(Resource, Clone, Debug, Default)]
pub struct EntityCellRows {
    pub cells: usize,
    pub rows: Vec<u32>,
    pub brush_boxes: Vec<Option<Bounds>>,
}

impl EntityCellRows {
    /// Make room for `cells` cells. Same size keeps the contents; any other
    /// size starts from nothing.
    fn resize_for(&mut self, cells: usize) {
        if cells != self.cells || !self.has_room() {
            *self = Self {
                cells,
                rows: vec![0; dpvs_t6::entity_cell_bitset_length(cells)],
                brush_boxes: vec![None; ENT_SLOTS],
            };
        }
    }

    fn has_room(&self) -> bool {
        let rows_fit = self.rows.len() == dpvs_t6::entity_cell_bitset_length(self.cells);
        rows_fit && self.brush_boxes.len() == ENT_SLOTS
    }

    fn usable(&self) -> bool {
        self.cells != 0 && self.has_room()
    }
}

pub fn add_cell_filing_systems(client: &mut App) {
    let brushes = (
        file_moving_brushes,
        cull_moving_brushes
            .after(file_moving_brushes)
            .after(JobQueueStage::CellStatic),
    )
        .in_set(DrawPassStage::Anim)
        .in_set(JobQueueStage::CellDynBrush);
    let entities = (
        size_entity_rows,
        file_scene_entities
            .after(size_entity_rows)
            .after(RenderSceneInsert),
        cull_scene_entities
            .after(file_scene_entities)
            .after(JobQueueStage::CellStatic),
    )
        .in_set(DrawPassStage::Anim)
        .in_set(JobQueueStage::CellSceneEnt);
    let bounding = run_bounding_jobs
        .in_set(JobQueueStage::DpvsEnt)
        .after(JobQueueStage::CellSceneEnt);
    client.init_resource::<MovingBrushCells>()
        .init_resource::<debris_piece::DebrisSolidKeyLightVis>()
        .init_resource::<EntityCellRows>();
    client.add_systems(Update, brushes);
    client.add_systems(Update, entities);
    client.add_systems(Update, bounding);
}

/// The world's portal tree, when it has nodes and cells to file into.
fn portal_tree(world: Option<&LevelStage>) -> Option<&LevelHide> {
    let cull = world?.culling.as_ref()?;
    let has_cells = cull.dpvs.cells_n != 0 && !cull.dpvs.node_list.is_empty();
    has_cells.then_some(cull)
}

fn tree_view(cull: &LevelHide) -> VisibilityCellPlanes<'_> {
    let box_tree = &cull.dpvs;
    VisibilityCellPlanes {
        planes: &box_tree.plane_set,
        nodes: &box_tree.node_list,
        leaf_cells: box_tree.cells_n as u32,
    }
}

/// The tree a box is checked against when deciding which cell it reaches;
/// unlike filing, an empty tree still counts.
fn world_tree(world: Option<&LevelStage>) -> Option<VisibilityCellPlanes<'_>> {
    world?.culling.as_ref().map(tree_view)
}

/// This frame's camera frustum; empty until the first frame is culled.
fn camera_frustum(stats: Option<&SightFrameStats>) -> Vec<[f32; 4]> {
    stats.map(|s| s.view_volume_planes.clone()).unwrap_or_default()
}

/// Slots the moving brushes need: the highest brush index plus one.
fn brush_slots(list: &[LevelDebrisSolid]) -> u32 {
    list.iter()
        .fold(0, |most, mover| most.max(u32::from(mover.index) + 1))
}

/// The brush models worth drawing: brushes a cell saw (`seen` holds one byte
/// per brush slot) that also have surfaces.
fn brush_models_to_draw(list: &[LevelDebrisSolid], seen: &[u8]) -> Vec<u16> {
    let mut picked = Vec::new();
    for mover in list {
        let flag = seen.get(usize::from(mover.index)).map_or(0, |&f| f);
        if dpvs_t6::debris_brush_listed(flag, mover.facet_tally) {
            picked.push(mover.solid_model);
        }
    }
    picked
}

/// Drop every moving brush into the cells its box overlaps, and record which
/// primary lights reach it.
fn file_moving_brushes(
    world: Option<Res<LevelStage>>,
    at_point: Res<render_scene::LooseAtSpotLookup>,
    mut filed: ResMut<MovingBrushCells>,
    mut lights: ResMut<debris_piece::DebrisSolidKeyLightVis>,
) {
    let world = world.as_deref();
    let Some((world, cull_set)) = world.zip(portal_tree(world)) else {
        *filed = MovingBrushCells::default();
        *lights = Default::default();
        return;
    };
    let tree = tree_view(cull_set);
    let slots = brush_slots(&world.debris_solids);
    let cells = &mut filed.filing;
    let _ = cells.ensure(slots, cull_set.dpvs.cells_n);
    cells.bits.fill(0);
    let sun = world.daylight_key_light_tally;
    let primaries = world.key_light_hide.len() as u32;
    lights.sunlight_primary_flag_index = sun;
    lights.main_light_total = primaries;
    debris_piece::reserve_main_light_bits(&mut lights.words, slots, sun, primaries);
    lights.words.fill(0);
    for mover in &world.debris_solids {
        if let Some(bx) = mover.bbox {
            let slot = u32::from(mover.index);
            let words = cells.dword_tally;
            dpvs_t6::filter_debris_into_rooms(&tree, bx, &mut cells.bits, words, slot);
            debris_piece::relink_debris_key_lights(
                &mut lights.words,
                None,
                &at_point,
                slot,
                bx,
                false,
            );
        }
    }
}

/// Run the brush cell commands: a brush filed in a walked cell that its
/// planes do not cull is seen, and seen brushes with surfaces get drawn.
fn cull_moving_brushes(
    mut world: Option<ResMut<LevelStage>>,
    mut stats: Option<ResMut<SightFrameStats>>,
    filed: Res<MovingBrushCells>,
    mut cmds: ResMut<FrontendTaskOrders>,
) {
    let frustum = camera_frustum(stats.as_deref());
    let list = world
        .as_deref()
        .map(|w| w.debris_solids.as_slice())
        .unwrap_or_default();
    let cell_list = &filed.filing;
    let mut seen = vec![0u8; brush_slots(list) as usize];
    let _ = cmds.queues.wait_for_type(
        render_frontend::TASK_ORDER_ROOM_DEBRIS_SOLID,
        render_frontend::TaskOrderBusyInput::default(),
        |bytes| {
            let Some(cmd) = RoomViewVolumeTaskOrder::from_le(bytes) else {
                return;
            };
            if !cell_list.sized() {
                return;
            }
            let walked = cmd.cell as usize;
            for mover in list {
                let Some(bx) = mover.bbox else { continue };
                let slot = usize::from(mover.index);
                let here =
                    dpvs_t6::debris_marked_in_cell(&cell_list.bits, cell_list.dword_tally, walked, slot as u32);
                if let (true, Some(flag)) = (here, seen.get_mut(slot)) {
                    dpvs_t6::write_debris_solid_vis(flag, bx, &frustum);
                }
            }
        },
    );
    let models = brush_models_to_draw(list, &seen);
    if let Some(world) = world.as_deref_mut() {
        add_brush_model_surfaces(world, stats.as_deref_mut(), &models);
    }
}

/// Put the surfaces of `models` on the draw list. Model 0 is the world
/// itself, which is drawn elsewhere.
fn add_brush_model_surfaces(
    world: &mut LevelStage,
    stats: Option<&mut SightFrameStats>,
    models: &[u16],
) {
    let needed: Vec<u16> = models.iter().copied().filter(|&m| m != 0).collect();
    if needed.is_empty() {
        return;
    }
    let catalog = &world.live_material_ledger.materials;
    let baked: Vec<Option<u64>> = catalog.iter().map(|m| m.baked_facet_draw).collect();
    let batch_lights: Vec<u8> = world
        .batches
        .iter()
        .map(|b| b.key_light_index)
        .collect();
    let Some(cull) = world.culling.as_mut() else {
        return;
    };
    let mut present: HashSet<u16> = cull.item_surfs.iter().copied().collect();
    let mut sum = 0u32;
    for model in needed {
        if let Some(span) = cull.solid_models.get(usize::from(model)).copied() {
            let (first, count) = (span.start_facet, span.facet_tally);
            let (n, _) = cull::append_solid_model_colour_span(
                cull,
                first,
                count,
                &mut present,
                &baked,
                &batch_lights,
            );
            sum = sum.saturating_add(n);
        }
    }
    if sum > 0 {
        cull::refresh_draw_items_id(cull);
        if let Some(stats) = stats {
            let items = &cull.draw_batch_list;
            stats.keys = items.len() as u32;
            stats.rebinds = cull::draw_item_rebinds(items);
        }
    }
}

fn size_entity_rows(world: Option<Res<LevelStage>>, mut rows: ResMut<EntityCellRows>) {
    let cells = world
        .as_deref()
        .and_then(|w| w.culling.as_ref())
        .map(|c| c.dpvs.cells_n);
    match cells {
        Some(n) => rows.resize_for(n),
        None => *rows = EntityCellRows::default(),
    }
}

/// A box by the bit patterns of its middle and half size.

fn box_key(bx: Bounds) -> [u32; 6] {
    let (m, h) = (bx.mid(), bx.half());
    [m[0], m[1], m[2], h[0], h[1], h[2]].map(f32::to_bits)
}

/// Each entity's last filing, so one that has not moved does not walk the
/// tree again.
#[derive(Default)]
struct EntityFilingMemo {
    /// Identity of the tree the remembered cells belong to: its node array's
    /// address and length.
    tree_id: (usize, usize),
    remembered: HashMap<u32, (Vec<u32>, [u32; 6])>,
    /// Entities filed this frame; filing one again first takes it back out
    /// of the cells it went into.
    this_frame: HashSet<u32>,
}

impl EntityFilingMemo {
    /// Begin a frame; a different tree forgets everything remembered.
    fn start_frame(&mut self, tree_id: (usize, usize)) {
        if tree_id != self.tree_id {
            self.remembered.clear();
            self.tree_id = tree_id;
        }
        self.this_frame.clear();
    }

    fn file(&mut self, tree: &VisibilityCellPlanes<'_>, rows: &mut [u32], bx: Bounds, ent: u32) {
        if !self.this_frame.insert(ent) {
            dpvs_t6::clear_actor_cells_main_view(rows, tree.leaf_cells as usize, ent);
        }
        let key = box_key(bx);
        let (cells, known_key) = self.remembered.entry(ent).or_default();
        if *known_key != key || cells.is_empty() {
            *known_key = key;
            cells.clear();
            dpvs_t6::scene_ent_cells(tree, bx, &mut |c| cells.push(c));
        }
        for &c in cells.iter() {
            dpvs_t6::mark_actor_in_cell(rows, c as usize, ent);
        }
    }
}

/// The box an entity is filed by: its posed box, else the cube around its
/// sphere, else none.
fn filing_box(origin: [f32; 3], rad: Option<f32>, posed: Option<Bounds>) -> Option<Bounds> {
    match (posed, rad) {
        (Some(bx), _) => Some(bx),
        (None, Some(r)) => Some(Bounds::from_center_extent(origin, [r, r, r])),
        (None, None) => None,
    }
}

/// Drop the scene's models, dobjs and brush models into the cell rows.
fn file_scene_entities(
    world: Option<Res<LevelStage>>,
    host: Res<HostVisualStage>,
    mut rows: ResMut<EntityCellRows>,
    mut memo: Local<EntityFilingMemo>,
) {
    if !rows.usable() {
        return;
    }
    let EntityCellRows {
        rows: bits,
        brush_boxes,
        ..
    } = &mut *rows;
    bits.fill(0);
    brush_boxes.fill(None);
    let Some(cull) = portal_tree(world.as_deref()) else {
        return;
    };
    let tree = tree_view(cull);
    let nodes = &cull.dpvs.node_list;
    memo.start_frame((nodes.as_ptr() as usize, nodes.len()));
    let gfx = &host.scene;
    let models = gfx
        .stage_models
        .iter()
        .map(|m| (m.about, filing_box(m.origin, m.reach, m.pose_bounds)));
    let dobjs = gfx
        .stage_puppets
        .iter()
        .map(|d| (d.about, filing_box(d.origin, d.reach, d.pose_bounds)));
    for (info, bx) in models.chain(dobjs) {
        if let Some(bx) = bx {
            memo.file(&tree, bits, bx, stage_info_actor_num(info));
        }
    }
    for placed in &gfx.scene_brushes {
        let local = cull.solid_model_bounds.get(placed.mesh_slot_number as usize);
        let Some(ent) = placed.extra_slot_param else {
            continue;
        };
        let Some(local) = local else {
            continue;
        };
        let ent = u32::from(ent);
        let turn_matrix = placed.spin.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let world_box =
            crate::prepare::scene::world::posed_solid_bounds(placed.origin, turn_matrix, *local);
        dpvs_t6::sort_brush_model_into_cells(&tree, world_box, bits, ent);
        if let Some(kept) = brush_boxes.get_mut(ent as usize) {
            *kept = Some(world_box);
        }
    }
}

/// Whether `bx` files into `cell`; with no tree, every cell counts.
fn box_in_cell(tree: Option<&VisibilityCellPlanes<'_>>, bx: Bounds, which: usize) -> bool {
    tree.is_none_or(|planes| dpvs_t6::actor_box_touches_cell(planes, bx, which))
}

/// Where a dobj's skin comes from: the per-entity inputs and the surface
/// cache that pending inputs expand through.
struct Skinning<'a> {
    inputs: &'a mut EntitySkinningInputSet,
    surfaces: &'a mut EntitySurfaceMemo,
}

/// Flag an entity visible, skinning it first if it is a dobj seen for the
/// first time this frame.
fn skin_and_show(gfx: &mut VisualStage, skinning: &mut Skinning<'_>, ent: u32) {
    let first_time = match gfx.stage_puppet(ent) {
        Some(d) => {
            let mut gate = d.visibility_gate_bits;
            dpvs_t6::scene_dobj_skin_begin(&mut gate)
        }
        None => false,
    };
    if first_time {
        skin_dobj(gfx, skinning, ent);
    }
    gfx.stain_stage_actor_visible(ent);
}

/// Lay out a dobj's skinned surfaces in this frame's skinning space.
fn skin_dobj(gfx: &mut VisualStage, skinning: &mut Skinning<'_>, ent: u32) {
    let source = match skinning.inputs.per_entity.get(&ent) {
        Some(ready) => Some(ready.clone()),
        None => skinning
            .inputs
            .pending
            .get(&ent)
            .map(|p| grow_pending_scene_entities(p, skinning.surfaces)),
    };
    let offset = skinning.inputs.frame_bytes_consumed;
    let mut placed = Vec::new();
    let summary = match &source {
        None => dpvs_t6::PreSkinningDigest::default(),
        Some(src) => {
            let parts: Vec<_> = src
                .models
                .iter()
                .map(|m| dpvs_t6::PreposedModelInput {
                    lod: m.lod,
                    joint_total: m.joint_total,
                    surfaces: &m.surfaces,
                })
                .collect();
            dpvs_t6::bake_scene_entity_bones_early(&parts, &src.hidden_parts, offset, |e| placed.push(e))
        }
    };
    skinning.inputs.frame_bytes_consumed = offset.saturating_add(summary.bytes);
    let skinned = StageActorSkinnedFacets {
        entries: placed,
        summary,
    };
    gfx.store_stage_actor_skin(ent, skinned);
}

/// What a cell's walk does with a dobj in its entity row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DobjStep {
    /// Culled, or already dealt with.
    Leave,
    /// No posed box yet: the bounding job decides.
    NeedsBounds,
    /// In sight and still to be skinned.
    Skin,
}

/// Until a dobj's box is built only its sphere is tried, against the cell's
/// child planes. Once built, the box is tried against those, then against
/// every plane of the cell, and last against the cells it files into.
fn dobj_step(
    d: &VisualStagePuppet,
    child: &[[f32; 4]],
    all: &[[f32; 4]],
    reaches: impl FnOnce(Bounds) -> bool,
) -> DobjStep {
    let (Some(r), Some(bx)) = (d.reach, d.pose_bounds) else {
        return DobjStep::NeedsBounds;
    };
    if dpvs_t6::scene_dobj_bounds_pending(d.visibility_gate_bits) {
        return if dpvs_t6::scene_ent_sphere_culled(d.origin, r, child) {
            DobjStep::Leave
        } else {
            DobjStep::NeedsBounds
        };
    }
    let hidden = dpvs_t6::box_outside_planes(bx, child)
        || dpvs_t6::box_outside_planes(bx, all)
        || d.visibility_gate_bits == dpvs_t6::SCENE_DOBJ_CULL_DONE;
    if hidden || !reaches(bx) {
        DobjStep::Leave
    } else {
        DobjStep::Skin
    }
}

/// A model with no sphere is never culled.
fn model_hidden(plane_set: &[[f32; 4]], centre: [f32; 3], rad: Option<f32>) -> bool {
    match rad {
        Some(r) => dpvs_t6::scene_ent_sphere_culled(centre, r, plane_set),
        None => false,
    }
}

/// The job that builds the box of dobj `slot` for the cell `cmd` walked.
fn bounds_job_for(slot: u16, cmd: &RoomViewVolumeTaskOrder) -> SightActorTaskOrder {
    let cell = u16::try_from(cmd.cell).unwrap_or(u16::MAX);
    SightActorTaskOrder {
        stage_puppet: slot.into(),
        plane_tally: cmd.plane_tally.into(),
        cell,
    }
}

/// A cell's own clip planes, or the whole frustum when it has none.
fn planes_for_cell(
    stats: Option<&SightFrameStats>,
    cell: usize,
    view_planes: &[[f32; 4]],
) -> Vec<[f32; 4]> {
    let own = stats
        .and_then(|s| s.room_clips.get(cell))
        .filter(|clip| clip.plane_tally > 0);
    match own {
        Some(clip) => clip.as_slice().to_vec(),
        None => view_planes.to_vec(),
    }
}

/// Everything a frame's cell walks read besides the scene.
struct FrameView<'a> {
    rows: &'a EntityCellRows,
    stats: Option<&'a SightFrameStats>,
    frustum: &'a [[f32; 4]],
    tree: Option<VisibilityCellPlanes<'a>>,
}

/// Walk one cell: first its entity row, then its brush model row. Dobjs that
/// still need a box go on `jobs`.
fn walk_cell(
    gfx: &mut VisualStage,
    view: &FrameView<'_>,
    skinning: &mut Skinning<'_>,
    command: &RoomViewVolumeTaskOrder,
    jobs: &mut Vec<SightActorTaskOrder>,
) {
    let cell = command.cell as usize;
    let cells = view.rows.cells;
    if cell >= cells {
        return;
    }
    gfx.stage_actor_walked = true;
    let planes = planes_for_cell(view.stats, cell, view.frustum);
    let child = dpvs_t6::scene_ent_child_planes(&planes, command.plane_first, command.plane_tally);
    let width = dpvs_t6::actor_cell_mask(dpvs_t6::VISUAL_CFG_ACTOR_TALLY);
    let slot_in = |table: &[u16], ent: usize| {
        table
            .get(ent)
            .copied()
            .filter(|&slot| slot != render_frontend::SLOT_INDEX_NONE)
    };

    let ent_row = dpvs_t6::entity_cell_bitset_row(&view.rows.rows, cell);
    for ent in msb_iter(ent_row, width) {
        let id = ent as u32;
        if gfx.stage_actor_visible(id) {
            continue;
        }
        if let Some(slot) = slot_in(&gfx.animated_object_slot_table, ent) {
            let Some(d) = gfx.stage_puppets.get(usize::from(slot)) else {
                continue;
            };
            let in_cell = |bx| box_in_cell(view.tree.as_ref(), bx, cell);
            match dobj_step(d, child, &planes, in_cell) {
                DobjStep::Leave => {}
                DobjStep::NeedsBounds => jobs.push(bounds_job_for(slot, command)),
                DobjStep::Skin => skin_and_show(gfx, skinning, id),
            }
        } else if let Some(slot) = slot_in(&gfx.static_model_slot_table, ent) {
            let shown = gfx
                .stage_models
                .get(usize::from(slot))
                .is_some_and(|m| !model_hidden(child, m.origin, m.reach));
            if shown {
                gfx.stain_stage_actor_visible(id);
            }
        }
    }

    let brush_row = dpvs_t6::entity_cell_row_later_pass(&view.rows.rows, cells, cell);
    for ent in msb_iter(brush_row, width) {
        let id = ent as u32;
        if gfx.stage_actor_visible(id) {
            continue;
        }
        let kept = view.rows.brush_boxes.get(ent).copied().flatten();
        if kept.is_some_and(|bx| !dpvs_t6::box_outside_planes(bx, child)) {
            gfx.stain_stage_actor_visible(id);
        }
    }
}

/// Run the entity cell commands, then queue a bounding job for every dobj
/// that was waiting on its box.
fn cull_scene_entities(
    rows: Res<EntityCellRows>,
    mut host: ResMut<HostVisualStage>,
    mut inputs: ResMut<EntitySkinningInputSet>,
    mut surfaces: ResMut<EntitySurfaceMemo>,
    world: Option<Res<LevelStage>>,
    stats: Option<Res<SightFrameStats>>,
    mut cmds: ResMut<FrontendTaskOrders>,
) {
    let _ = cmds.queues.reset_kind(render_frontend::TASK_ORDER_SIGHT_ACTOR);
    host.scene.stage_actor_walked = false;
    let usable = rows.usable();
    let frustum = camera_frustum(stats.as_deref());
    let view = FrameView {
        rows: &rows,
        stats: stats.as_deref(),
        frustum: &frustum,
        tree: world_tree(world.as_deref()),
    };
    let mut skinning = Skinning {
        inputs: &mut inputs,
        surfaces: &mut surfaces,
    };
    let gfx = &mut host.scene;
    let mut job_list = Vec::new();
    let _ = cmds.queues.wait_for_type(
        render_frontend::TASK_ORDER_ROOM_STAGE_ACTOR,
        render_frontend::TaskOrderBusyInput::default(),
        |bytes| {
            let cmd = RoomViewVolumeTaskOrder::from_le(bytes).filter(|_| usable);
            if let Some(cmd) = cmd {
                walk_cell(gfx, &view, &mut skinning, &cmd, &mut job_list);
            }
        },
    );
    for job in job_list {
        let _ = cmds.queues.queue_record(render_frontend::TASK_ORDER_SIGHT_ACTOR, &job.record_bytes());
    }
}

/// Run the bounding jobs: build each dobj's box and, when it is in sight of
/// the cell that asked, skin it and flag it visible.
fn run_bounding_jobs(
    mut cmds: ResMut<FrontendTaskOrders>,
    mut host: ResMut<HostVisualStage>,
    mut inputs: ResMut<EntitySkinningInputSet>,
    mut surfaces: ResMut<EntitySurfaceMemo>,
    world: Option<Res<LevelStage>>,
    stats: Option<Res<SightFrameStats>>,
) {
    let frustum = camera_frustum(stats.as_deref());
    let tree = world_tree(world.as_deref());
    let mut skinning = Skinning {
        inputs: &mut inputs,
        surfaces: &mut surfaces,
    };
    let gfx = &mut host.scene;
    let _ = cmds.queues.wait_for_type(
        render_frontend::TASK_ORDER_SIGHT_ACTOR,
        render_frontend::TaskOrderBusyInput::default(),
        |bytes| {
            let shown = SightActorTaskOrder::read_record(bytes)
                .and_then(|job| claim_and_bound(gfx, &job, &frustum, tree.as_ref()));
            if let Some(ent) = shown {
                skin_and_show(gfx, &mut skinning, ent);
            }
        },
    );
}

/// Claim the job's dobj and settle its box. Returns the entity to skin when
/// the dobj has a box and is in sight of the job's cell.
fn claim_and_bound(
    gfx: &mut VisualStage,
    job: &SightActorTaskOrder,
    view_planes: &[[f32; 4]],
    tree: Option<&VisibilityCellPlanes<'_>>,
) -> Option<u32> {
    let d = gfx.stage_puppets.get_mut(job.stage_puppet as usize)?;
    if !dpvs_t6::scene_dobj_cull_begin(&mut d.visibility_gate_bits) {
        return None;
    }
    match d.pose_bounds {
        None => {
            d.visibility_gate_bits = dpvs_t6::SCENE_DOBJ_CULL_DONE;
            None
        }
        Some(bx) => {
            d.visibility_gate_bits = dpvs_t6::SCENE_DOBJ_CULL_BOUNDED;
            let visible =
                !dpvs_t6::box_outside_planes(bx, view_planes) && box_in_cell(tree, bx, usize::from(job.cell));
            visible.then(|| stage_info_actor_num(d.about))
        }
    }
}

#[cfg(test)]
mod tests {
    use dpvs_t6::{
        PlaneEquation, VISUAL_CFG_ACTOR_TALLY, entity_cell_bitset_length, entity_cell_bitset_row, write_debris_solid_vis,
    };

    use super::*;

    fn brush(index: u16, solid_model: u16, facet_tally: u16) -> LevelDebrisSolid {
        LevelDebrisSolid {
            index,
            at_point: [0.0; 3],
            bbox: None,
            solid_model,
            facet_tally,
        }
    }

    fn dobj(radius: Option<f32>, bounds: Option<Bounds>, gate: u32) -> VisualStagePuppet {
        let origin = [0.0; 3];
        VisualStagePuppet {
            origin,
            illum_origin: [0.0; 3],
            reach: radius,
            about: 0,
            spin: None,
            visibility_gate_bits: gate,
            pose_bounds: bounds,
            detail_tiers: Vec::new(),
            skinned_facets: None,
        }
    }

    /// A bounding job for the first dobj slot, cell 0.
    fn first_job() -> SightActorTaskOrder {
        SightActorTaskOrder {
            stage_puppet: 0,
            plane_tally: 0,
            cell: 0,
        }
    }

    /// Looking down +x from the origin: only x > 0 is in front.
    const FRONT: [[f32; 4]; 1] = [[1.0, 0.0, 0.0, 0.0]];

    fn cube(mid: [f32; 3], half: f32) -> Bounds {
        Bounds::from_center_extent(mid, [half; 3])
    }

    fn axial(dist: f32) -> PlaneEquation {
        PlaneEquation {
            normal: [1.0, 0.0, 0.0],
            offset: dist,
            r#type: 0,
        }
    }

    // x = 0 splits cells 0 (front) and 1 (back); behind it x = -10 splits 1
    // and 2.
    const PLANES: [PlaneEquation; 2] = [
        PlaneEquation {
            normal: [1.0, 0.0, 0.0],
            offset: 0.0,
            r#type: 0,
        },
        PlaneEquation {
            normal: [1.0, 0.0, 0.0],
            offset: -10.0,
            r#type: 0,
        },
    ];
    const NODES: [u16; 7] = [4, 3, 1, 5, 3, 2, 3];

    fn bsp() -> VisibilityCellPlanes<'static> {
        VisibilityCellPlanes {
            planes: &PLANES,
            nodes: &NODES,
            leaf_cells: 3,
        }
    }

    /// Spans x from -5 to 5: cells 0 and 1, not the one behind x = -10.
    fn straddling_zero() -> Bounds {
        Bounds::from_corner_extremes([-5.0, 0.0, 0.0], [5.0, 1.0, 1.0])
    }

    fn rows_holding(bits: &[u32], ent: usize) -> Vec<usize> {
        (0..3)
            .filter(|&cell| msb_iter(entity_cell_bitset_row(bits, cell), 64).any(|id| id == ent))
            .collect()
    }

    #[test]
    fn the_test_tree_is_the_one_the_planes_describe() {
        assert_eq!(axial(0.0), PLANES[0]);
        let mut found = Vec::new();
        dpvs_t6::scene_ent_cells(&bsp(), straddling_zero(), &mut |c| found.push(c));
        assert_eq!(found, vec![0, 1]);
    }

    #[test]
    fn scene_ent_bits_are_sized_by_cell_count_and_ready_only_with_cells() {
        let mut bits = EntityCellRows::default();
        assert!(!bits.usable());
        bits.resize_for(0);
        assert!(!bits.usable(), "no cells is not ready");
        bits.resize_for(4);
        assert!(bits.usable());
        assert_eq!(bits.rows.len(), entity_cell_bitset_length(4));
        assert_eq!(bits.brush_boxes.len(), VISUAL_CFG_ACTOR_TALLY as usize);
    }

    #[test]
    fn ensuring_the_same_size_keeps_the_bits_and_a_new_size_clears_them() {
        let mut bits = EntityCellRows::default();
        bits.resize_for(2);
        bits.rows[1] = 7;
        bits.brush_boxes[3] = Some(cube([0.0; 3], 1.0));
        bits.resize_for(2);
        assert_eq!(bits.rows[1], 7);
        assert!(bits.brush_boxes[3].is_some());
        bits.resize_for(5);
        assert!(bits.rows.iter().all(|&w| w == 0));
        assert!(bits.brush_boxes.iter().all(Option::is_none));
    }

    #[test]
    fn a_resized_vector_is_not_ready() {
        let mut bits = EntityCellRows::default();
        bits.resize_for(3);
        bits.rows.pop();
        assert!(!bits.usable());
    }

    #[test]
    fn the_brush_slots_is_one_past_the_highest_brush_index() {
        assert_eq!(brush_slots(&[]), 0);
        assert_eq!(
            brush_slots(&[brush(4, 1, 1), brush(9, 2, 1), brush(0, 3, 1)]),
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
        assert_eq!(brush_models_to_draw(&brushes, &vis), vec![5]);
        assert!(brush_models_to_draw(&brushes, &[]).is_empty());
    }

    #[test]
    fn brush_visibility_is_written_once_and_kept() {
        let mut seen = 0u8;
        write_debris_solid_vis(&mut seen, cube([-50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 0, "behind the plane");
        write_debris_solid_vis(&mut seen, cube([50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 1);
        write_debris_solid_vis(&mut seen, cube([-50.0, 0.0, 0.0], 1.0), &FRONT);
        assert_eq!(seen, 1, "a later cell cannot hide it");
    }

    #[test]
    fn the_frustum_is_empty_before_the_first_frame() {
        assert!(camera_frustum(None).is_empty());
        let stats = SightFrameStats {
            view_volume_planes: FRONT.to_vec(),
            ..Default::default()
        };
        assert_eq!(camera_frustum(Some(&stats)), FRONT.to_vec());
    }

    #[test]
    fn a_cell_without_its_own_clip_uses_the_frustum() {
        assert_eq!(planes_for_cell(None, 0, &FRONT), FRONT.to_vec());
        let stats = SightFrameStats::default();
        assert_eq!(planes_for_cell(Some(&stats), 7, &FRONT), FRONT.to_vec());
    }

    #[test]
    fn posed_bounds_win_over_the_sphere_and_no_size_means_no_box() {
        let posed = cube([1.0, 2.0, 3.0], 4.0);
        let origin = [9.0, 9.0, 9.0];
        assert_eq!(filing_box(origin, Some(2.0), Some(posed)), Some(posed));
        assert_eq!(filing_box(origin, Some(2.0), None), Some(cube(origin, 2.0)));
        assert_eq!(filing_box(origin, None, None), None);
    }

    #[test]
    fn box_keys_differ_by_any_component() {
        let a = box_key(cube([1.0, 2.0, 3.0], 1.0));
        assert_eq!(a, box_key(cube([1.0, 2.0, 3.0], 1.0)));
        assert_ne!(a, box_key(cube([1.0, 2.0, 3.5], 1.0)));
        assert_ne!(a, box_key(cube([1.0, 2.0, 3.0], 1.5)));
    }

    #[test]
    fn without_a_bsp_every_cell_is_reached() {
        assert!(box_in_cell(None, cube([0.0; 3], 1.0), 99));
    }

    #[test]
    fn with_a_bsp_only_the_cells_the_box_reaches_count() {
        let bsp = bsp();
        let near = straddling_zero();
        assert!(box_in_cell(Some(&bsp), near, 0));
        assert!(box_in_cell(Some(&bsp), near, 1));
        assert!(!box_in_cell(Some(&bsp), near, 2));
    }

    #[test]
    fn an_entity_is_filed_in_the_cells_it_reaches() {
        let bsp = bsp();
        let mut bits = vec![0; entity_cell_bitset_length(3)];
        let mut cache = EntityFilingMemo::default();
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, straddling_zero(), 37);
        assert_eq!(rows_holding(&bits, 37), vec![0, 1]);
    }

    #[test]
    fn a_moved_entity_is_filed_again_and_a_still_one_reuses_its_cells() {
        let bsp = bsp();
        let mut bits = vec![0; entity_cell_bitset_length(3)];
        let mut cache = EntityFilingMemo::default();
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![0]);
        bits.fill(0);
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![0], "same box, same cells");
        bits.fill(0);
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, cube([-15.0, 0.0, 0.0], 1.0), 3);
        assert_eq!(rows_holding(&bits, 3), vec![2]);
    }

    #[test]
    fn an_entity_filed_twice_in_a_frame_keeps_only_the_last_filing() {
        let bsp = bsp();
        let mut bits = vec![0; entity_cell_bitset_length(3)];
        let mut cache = EntityFilingMemo::default();
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 8);
        cache.file(&bsp, &mut bits, cube([-15.0, 0.0, 0.0], 1.0), 8);
        assert_eq!(rows_holding(&bits, 8), vec![2]);
    }

    #[test]
    fn a_different_tree_empties_the_cache() {
        let bsp = bsp();
        let mut bits = vec![0; entity_cell_bitset_length(3)];
        let mut cache = EntityFilingMemo::default();
        cache.start_frame((1, 7));
        cache.file(&bsp, &mut bits, cube([5.0, 0.0, 0.0], 1.0), 3);
        assert!(!cache.remembered.is_empty());
        cache.start_frame((2, 7));
        assert!(cache.remembered.is_empty() && cache.this_frame.is_empty());
        assert_eq!(cache.tree_id, (2, 7));
    }

    #[test]
    fn a_dobj_without_a_sphere_or_box_waits_for_its_bounds() {
        let planes = FRONT;
        let walk = |d: &VisualStagePuppet| dobj_step(d, &planes, &planes, |_| true);
        assert_eq!(
            walk(&dobj(None, Some(cube([5.0; 3], 1.0)), 0)),
            DobjStep::NeedsBounds
        );
        assert_eq!(walk(&dobj(Some(1.0), None, 0)), DobjStep::NeedsBounds);
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
            dobj_step(&behind, &planes, &planes, |_| true),
            DobjStep::Leave
        );
        let mut ahead = behind.clone();
        ahead.origin = [10.0, 0.0, 0.0];
        assert_eq!(
            dobj_step(&ahead, &planes, &planes, |_| true),
            DobjStep::NeedsBounds
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
        assert_eq!(dobj_step(&seen, &planes, &planes, |_| true), DobjStep::Skin);
        assert_eq!(
            dobj_step(&seen, &planes, &planes, |_| false),
            DobjStep::Leave
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
        assert_eq!(dobj_step(&seen, &behind, &FRONT, |_| true), DobjStep::Leave);
        assert_eq!(dobj_step(&seen, &FRONT, &behind, |_| true), DobjStep::Leave);
        assert_eq!(dobj_step(&seen, &[], &[], |_| true), DobjStep::Skin);
    }

    #[test]
    fn a_dobj_already_skinned_has_nothing_left_to_do() {
        let done = dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_DONE,
        );
        assert_eq!(dobj_step(&done, &FRONT, &FRONT, |_| true), DobjStep::Leave);
    }

    #[test]
    fn the_reach_test_runs_only_once_the_dobj_survives_the_planes() {
        let culled = dobj(
            Some(1.0),
            Some(cube([-5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED,
        );
        let step = dobj_step(&culled, &FRONT, &FRONT, |_| panic!("not asked"));
        assert_eq!(step, DobjStep::Leave);
    }

    #[test]
    fn a_model_is_culled_only_with_a_sphere_behind_a_plane() {
        assert!(model_hidden(&FRONT, [-10.0, 0.0, 0.0], Some(1.0)));
        assert!(!model_hidden(&FRONT, [10.0, 0.0, 0.0], Some(1.0)));
        assert!(!model_hidden(&FRONT, [-10.0, 0.0, 0.0], None));
    }

    #[test]
    fn a_bounding_job_carries_the_slot_planes_and_cell() {
        let far = RoomViewVolumeTaskOrder {
            cell: 70_000,
            plane_tally: 5,
            plane_first: 2,
            view_pos: 0,
        };
        let job = bounds_job_for(12, &far);
        assert_eq!(
            job,
            SightActorTaskOrder {
                stage_puppet: 12,
                plane_tally: 5,
                cell: u16::MAX,
            }
        );
        let near = RoomViewVolumeTaskOrder { cell: 9, ..far };
        assert_eq!(bounds_job_for(1, &near).cell, 9);
    }

    #[test]
    fn a_dobj_that_cannot_be_claimed_is_left_alone() {
        let mut scene = VisualStage::default();
        scene.stage_puppets.push(dobj(
            Some(1.0),
            Some(cube([5.0, 0.0, 0.0], 1.0)),
            dpvs_t6::SCENE_DOBJ_CULL_DONE,
        ));
        let job = first_job();
        assert_eq!(claim_and_bound(&mut scene, &job, &[], None), None);
        assert_eq!(scene.stage_puppets[0].visibility_gate_bits, dpvs_t6::SCENE_DOBJ_CULL_DONE);
        let missing = SightActorTaskOrder {
            stage_puppet: 4,
            ..job
        };
        assert_eq!(claim_and_bound(&mut scene, &missing, &[], None), None);
    }

    #[test]
    fn a_claimed_dobj_without_bounds_is_finished() {
        let mut scene = VisualStage::default();
        let boxless = dobj(Some(1.0), None, dpvs_t6::SCENE_DOBJ_CULL_IDLE);
        scene.stage_puppets.push(boxless);
        let job = first_job();
        assert_eq!(claim_and_bound(&mut scene, &job, &[], None), None);
        assert_eq!(scene.stage_puppets[0].visibility_gate_bits, dpvs_t6::SCENE_DOBJ_CULL_DONE);
    }

    #[test]
    fn a_claimed_dobj_is_bounded_and_skinned_only_in_sight() {
        let make = |x: f32| {
            let mut scene = VisualStage::default();
            let mut d = dobj(
                Some(1.0),
                Some(cube([x, 0.0, 0.0], 1.0)),
                dpvs_t6::SCENE_DOBJ_CULL_IDLE,
            );
            d.about = 5;
            scene.stage_puppets.push(d);
            scene
        };
        let job = first_job();
        let mut seen = make(5.0);
        assert_eq!(
            claim_and_bound(&mut seen, &job, &FRONT, None),
            Some(stage_info_actor_num(5))
        );
        assert_eq!(seen.stage_puppets[0].visibility_gate_bits, dpvs_t6::SCENE_DOBJ_CULL_BOUNDED);
        let mut hidden = make(-5.0);
        assert_eq!(claim_and_bound(&mut hidden, &job, &FRONT, None), None);
        assert_eq!(
            hidden.stage_puppets[0].visibility_gate_bits,
            dpvs_t6::SCENE_DOBJ_CULL_BOUNDED
        );
    }
}
