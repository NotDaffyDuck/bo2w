//! Impact marks (bullet holes, scorches) live here: fixed-size storage for
//! the marks and for the triangle and point groups they own, recycled
//! least-recently-drawn first, and the mesh that draws them.

use std::iter::{from_fn, successors};

use marks_t6::{
    DECAL_HANDLE_ABSENT, DECAL_INITIAL_CLAIMED, DECAL_POOL_CAPACITY, FX_MARKS_RANGE_DEFAULT,
};
use marks_t6::{
    POINT_CHUNK_CHAIN_TERMINATOR, DECAL_POINT_BLOCK_CAPACITY, POINT_CHUNK_SUCCESSOR_ABSENT,
    TRI_CHUNK_CHAIN_TERMINATOR, DECAL_TRIANGLE_BLOCK_CAPACITY, TRI_CHUNK_SUCCESSOR_ABSENT,
};
use marks_t6::{
    DecalAllocationAsk, BuiltDecalRecord, EffectStainStagingPoint, EffectStainStagingTri, FxMarksGrid,
    DecalPointChunk, DecalTriangleChunk,
};
use marks_t6::{
    DECAL_MESH_VERTEX_BYTES, GfxMarkContext, DecalMeshAllowance, DecalMeshDenial,
    add_mark_mesh_draw_surf, generate_mark_verts_begin, mark_contexts_equal,
    pack_mark_model_vertex, pack_mark_world_vertex,
};
use marks_t6::{
    alloc_and_construct_mark, copy_mark_points, copy_mark_tris, impact_mark_models_generate,
    impact_mark_outer_gate, impact_mark_within_range, init_mark_next_handle, init_point_next_slot,
    init_tri_next_slot, lru_mark_pick, mark_point_groups_for_count, mark_tri_groups_for_staging,
    marks_active_limit,
};

/// One drawable run of a mark's indices, all under a single material context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecalMeshPatch {
    pub context: GfxMarkContext,
    pub stain_cell: u16,
    pub index_count: u32,
    pub first_triangle_slot: u32,
}

/// Every mark drawn this frame as one mesh, plus the budget it was built under.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecalMeshTally {
    pub surfs: Vec<DecalMeshPatch>,
    pub index_list: Vec<u16>,
    pub vert_blob: Vec<[u8; DECAL_MESH_VERTEX_BYTES]>,
    pub vertex_points: Vec<EffectStainStagingPoint>,
    pub lead_point: Option<[f32; 3]>,
    pub budget: DecalMeshAllowance,
}

/// What the debug console is told about one impact-mark attempt.
#[derive(Clone, Debug)]
pub struct DecalRayLog {
    pub id: u32,
    pub bolt_tag: u8,
    pub went_in: bool,
    pub against_level: bool,
    pub versus_models: bool,
    pub source: Option<String>,
    pub part: Option<u8>,
    pub ms_now: Option<i32>,
    pub spot: Option<[f32; 3]>,
    pub start_extent: Option<f32>,
    pub tint: Option<u32>,
    pub tint_category: &'static str,
    pub tex_a: Option<String>,
    pub tex_b: Option<String>,
    pub bound_to: Option<String>,
    pub cell: Option<u16>,
    pub triangle_tally: Option<u8>,
    pub point_tally: Option<i16>,
    pub authored_tint: Option<u32>,
    pub context: Option<GfxMarkContext>,
    pub bypass_cause: Option<&'static str>,
    pub seen: Option<u8>,
    pub tex_a_edge: Option<String>,
    pub tex_b_edge: Option<String>,
    pub facing_x: Option<f32>,
    pub facing_y: Option<f32>,
    pub facing_z: Option<f32>,
}

/// Which kinds of surface are allowed to take marks.
#[derive(Clone, Copy, Debug)]
pub struct DecalAcceptToggle {
    pub stains_on_props: bool,
    pub stains_on_ents: bool,
    pub stains: bool,
}

impl Default for DecalAcceptToggle {
    fn default() -> Self {
        let on = true;
        Self {
            stains: on,
            stains_on_ents: on,
            stains_on_props: on,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DecalContactAsk {
    pub receivers: DecalAcceptToggle,
    pub no_world: bool,
    /// None skips the range check.
    pub anchor: Option<[f32; 3]>,
    /// None while there is no camera yet (every mark is in range then).
    pub camera: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkOutcome {
    pub versus_models: bool,
    pub against_level: bool,
    pub went_inside: bool,
}

/// Fixed slots whose spare ones are threaded together: `links[s]` names the
/// spare slot after `s`, `first` is the first spare and `sentinel` ends the thread.
#[derive(Clone, Debug)]
struct SlotRing {
    links: Vec<u32>,
    first: u32,
    sentinel: u32,
}

impl SlotRing {
    fn exhausted(&self) -> bool {
        self.first == self.sentinel
    }

    fn spare(&self) -> u32 {
        let valid = |slot: &u32| *slot != self.sentinel && (*slot as usize) < self.links.len();
        successors(Some(self.first), |&slot| {
            self.links.get(slot as usize).copied()
        })
        .take_while(valid)
        .take(self.links.len())
        .fold(0, |hops: u32, _| hops.saturating_add(1))
    }

    /// Hands out `wanted` spare slots in thread order; nothing changes when the
    /// thread breaks before that many are found.
    fn claim(&mut self, wanted: u32) -> Option<Vec<u32>> {
        let mut cursor = self.first;
        let mut out = Vec::with_capacity(wanted as usize);
        for _ in 0..wanted {
            let following = *self.links.get(cursor as usize)?;
            out.push(cursor);
            cursor = following;
        }
        self.first = cursor;
        Some(out)
    }

    fn give_back(&mut self, slot: u32) {
        self.links[slot as usize] = std::mem::replace(&mut self.first, slot);
    }
}

/// What a pool of chained groups needs from the group type.
trait Chained: Clone {
    fn empty() -> Self;
    fn chain_next(&self) -> u16;
    fn set_chain_next(&mut self, next: u16);
}

macro_rules! chained_via_next {
    ($group:ty) => {
        impl Chained for $group {
            fn empty() -> Self {
                <$group>::ZERO
            }
            fn chain_next(&self) -> u16 {
                self.next
            }
            fn set_chain_next(&mut self, next: u16) {
                self.next = next;
            }
        }
    };
}

chained_via_next!(DecalTriangleChunk);
chained_via_next!(DecalPointChunk);

/// Groups that marks borrow, in chains, from one fixed array.
#[derive(Clone, Debug)]
struct GroupPool<G: Chained> {
    ledger: SlotRing,
    /// The `next` value that ends a chain a mark holds.
    terminator: u16,
    slots: Vec<G>,
}

impl<G: Chained> GroupPool<G> {
    fn new(ledger: SlotRing, terminator: u16) -> Self {
        let slots = vec![G::empty(); ledger.links.len()];
        Self {
            ledger,
            terminator,
            slots,
        }
    }

    /// The groups of the chain starting at `head`, front to back; a broken
    /// link or a loop ends the walk.
    fn walk_ids(&self, head: u16) -> impl Iterator<Item = u16> + '_ {
        let mut cursor = Some(head).filter(|&first| first != self.terminator);
        from_fn(move || {
            let id = cursor?;
            let group = self.slots.get(usize::from(id))?;
            cursor = Some(group.chain_next()).filter(|&after| after != self.terminator);
            Some(id)
        })
        .take(self.slots.len())
    }

    fn walk(&self, head: u16) -> impl Iterator<Item = &G> + '_ {
        self.walk_ids(head).map(|id| &self.slots[usize::from(id)])
    }

    /// Lends `count` groups as one chain and returns its first group.
    fn lend(&mut self, wanted_n: u32) -> Option<u16> {
        let picked = self.ledger.claim(wanted_n).filter(|slots| !slots.is_empty())?;
        let end = self.terminator;
        let groups = &mut self.slots;
        Some(picked.into_iter().fold(end, |chain_so_far, slot| {
            let mut fresh = G::empty();
            fresh.set_chain_next(chain_so_far);
            groups[slot as usize] = fresh;
            slot as u16
        }))
    }

    /// Returns a whole chain to the spare thread.
    fn reclaim(&mut self, head: u16) {
        let chain: Vec<u16> = self.walk_ids(head).collect();
        for id in chain {
            self.slots[usize::from(id)] = G::empty();
            self.ledger.give_back(u32::from(id));
        }
    }
}

/// How many groups of each kind a new mark needs.
#[derive(Clone, Copy, Debug)]
struct Needs {
    tri_groups: u32,
    point_groups: u32,
}

#[derive(Clone, Debug)]
pub struct DecalOwner {
    /// fx_marks_range (BO2 default 1500): no mark further than this from the camera.
    pub range: f32,
    /// fx_marks_grid_enable (BO2 default on): refuse a mark when its cell already holds 8.
    pub grid_enable: bool,
    pub skip_decals: bool,

    handles: SlotRing,
    stored: Vec<Option<BuiltDecalRecord>>,
    mat_names: Vec<Option<String>>,
    tri_pool: GroupPool<DecalTriangleChunk>,
    point_pool: GroupPool<DecalPointChunk>,
    in_use: u32,
    cap: u32,
    ever_allocated: u32,
    density: FxMarksGrid,
    last_msec: Option<i32>,
}

fn ring_from(links: Vec<u32>, first: u32, sentinel: u32) -> SlotRing {
    SlotRing {
        links,
        first,
        sentinel,
    }
}

fn keep_point(point: EffectStainStagingPoint) -> EffectStainStagingPoint {
    point
}

impl DecalOwner {
    pub fn init() -> Self {
        let handle_links: Vec<u32> = (0..DECAL_POOL_CAPACITY)
            .map(|slot| init_mark_next_handle(slot).into())
            .collect();
        let tri_links: Vec<u32> = (0..DECAL_TRIANGLE_BLOCK_CAPACITY).map(init_tri_next_slot).collect();
        let point_links: Vec<u32> = (0..DECAL_POINT_BLOCK_CAPACITY).map(init_point_next_slot).collect();
        let capacity = handle_links.len();
        // Group 0 is never lent out, so both group pools start their thread at 1.
        let tri_ring = ring_from(tri_links, 1, TRI_CHUNK_SUCCESSOR_ABSENT);
        let point_ring = ring_from(point_links, 1, POINT_CHUNK_SUCCESSOR_ABSENT);
        Self {
            range: FX_MARKS_RANGE_DEFAULT,
            grid_enable: true,
            skip_decals: bool::default(),
            handles: ring_from(handle_links, 0, DECAL_HANDLE_ABSENT.into()),
            stored: vec![None; capacity],
            mat_names: vec![None; capacity],
            tri_pool: GroupPool::new(tri_ring, TRI_CHUNK_CHAIN_TERMINATOR),
            point_pool: GroupPool::new(point_ring, POINT_CHUNK_CHAIN_TERMINATOR),
            in_use: 0,
            cap: marks_active_limit(1),
            ever_allocated: DECAL_INITIAL_CLAIMED,
            density: FxMarksGrid::new(),
            last_msec: None,
        }
    }

    pub fn active_total(&self) -> u32 {
        self.in_use
    }

    pub fn taken_count(&self) -> u32 {
        self.ever_allocated
    }

    /// The first spare triangle group.
    pub fn tri_free_head(&self) -> u32 {
        self.tri_pool.ledger.first
    }

    /// The first spare point group.
    pub fn point_free_head(&self) -> u32 {
        self.point_pool.ledger.first
    }

    /// The staged pieces a request asks for, or `None` when it asks for more
    /// than was staged.
    fn staged_for<'a>(
        request: &DecalAllocationAsk,
        triangle_list: &'a [EffectStainStagingTri],
        points: &'a [EffectStainStagingPoint],
    ) -> Option<(&'a [EffectStainStagingTri], &'a [EffectStainStagingPoint])> {
        Some((
            triangle_list.get(..request.tri_tally as usize)?,
            points.get(..request.vertex_total_in_mark as usize)?,
        ))
    }

    fn has_room_for(&self, needs: Needs) -> bool {
        self.in_use < self.cap
            && !self.handles.exhausted()
            && self.tri_pool.ledger.spare() >= needs.tri_groups
            && self.point_pool.ledger.spare() >= needs.point_groups
    }

    /// Stores a new mark built from the staged triangles and points, recycling
    /// the least recently drawn marks when the pools are full. `None` when the
    /// mark is refused (bad request, crowded spot, or nothing left to recycle).
    pub fn claim_decal_for_scene_hook(
        &mut self,
        request: DecalAllocationAsk,
        tri_stage: &[EffectStainStagingTri],
        point_stage: &[EffectStainStagingPoint],
    ) -> Option<u16> {
        // Build a throwaway mark first: a request that can never become one
        // is turned away before anything is spent on it.
        alloc_and_construct_mark(&request, 0, 0).ok()?;
        let crowded = self
            .density
            .too_many_marks_in_vicinity(self.grid_enable, request.origin);
        let (triangle_list, points) = Self::staged_for(&request, tri_stage, point_stage)?;
        let needs = Needs {
            tri_groups: mark_tri_groups_for_staging(triangle_list),
            point_groups: mark_point_groups_for_count(request.vertex_total_in_mark),
        };
        if crowded || needs.tri_groups == 0 || needs.point_groups == 0 {
            return None;
        }

        // Nothing is grown: free room by retiring whichever mark was drawn longest ago.
        while !self.has_room_for(needs) {
            let victim = lru_mark_pick(self.live_marks())?;
            self.release_mark(victim);
        }

        let tri_head = self.tri_pool.lend(needs.tri_groups)?;
        let Some(first_point) = self.point_pool.lend(needs.point_groups) else {
            self.tri_pool.reclaim(tri_head);
            return None;
        };
        let stored_tris = copy_mark_tris(&mut self.tri_pool.slots, tri_head, triangle_list);
        let stored_points = copy_mark_points(&mut self.point_pool.slots, first_point, points);
        let finished = (stored_tris == request.tri_tally && stored_points == request.vertex_total_in_mark)
            .then(|| alloc_and_construct_mark(&request, tri_head, first_point).ok())
            .flatten();
        let Some(mark) = finished else {
            self.tri_pool.reclaim(tri_head);
            self.point_pool.reclaim(first_point);
            return None;
        };
        let handle = self.handles.claim(1)?[0] as u16;
        self.stored[usize::from(handle)] = Some(mark);
        self.in_use = self.in_use.saturating_add(1);
        self.ever_allocated = self.ever_allocated.saturating_add(1);
        Some(handle)
    }

    /// Ages every mark by the frame time. Every 500 ms of frame time the
    /// density grid is rebuilt from the world marks that have not expired.
    pub fn update_grid(&mut self, now: i32) {
        let step = match self.last_msec.replace(now) {
            Some(before) => now.wrapping_sub(before),
            None => 0,
        };
        // How BO2 adds the time to a mark's age was not read out; one frame's
        // worth per call is what is assumed here.
        let aged_by = step.max(0);
        self.stored
            .iter_mut()
            .flatten()
            .for_each(|mark| mark.lived_ms = mark.lived_ms.saturating_add(aged_by));
        if self.density.advance(step) {
            self.density.clear();
            self.stored
                .iter()
                .flatten()
                .filter(|mark| mark.context.is_world_list())
                .filter(|mark| !mark.alpha_fade.expired(mark.lived_ms))
                .for_each(|mark| self.density.add(mark.origin));
        }
    }

    fn live_marks(&self) -> impl Iterator<Item = (u16, &BuiltDecalRecord)> {
        (0..)
            .zip(&self.stored)
            .filter_map(|(at, entry)| Some((at, entry.as_ref()?)))
    }

    pub fn assign_material_name(&mut self, slot: u16, label: Option<&str>) {
        let kept = label.filter(|text| !text.is_empty()).map(String::from);
        if let Some(cell) = self.mat_names.get_mut(usize::from(slot)) {
            *cell = kept;
        }
    }

    pub fn surface_title(&self, slot: u16) -> Option<&str> {
        self.mat_names.get(usize::from(slot))?.as_deref()
    }

    pub fn built_record(&self, slot: u16) -> Option<&BuiltDecalRecord> {
        self.stored.get(usize::from(slot))?.as_ref()
    }

    /// A pane breaking (BO2 `FX_MarkGlassDetachAll`) frees every world mark whose modelIndex
    /// is the glass id. Like BO2 it checks nothing else, and world-brush marks have
    /// modelIndex 0, so breaking pane 0 also frees every world-brush mark. Static-model and
    /// entity marks are on other lists in BO2, so they stay.
    pub fn hide_glass_marks(&mut self, pane: u16) -> u32 {
        let on_pane = |mark: &BuiltDecalRecord| {
            mark.context.mesh_slot_number == pane
                && mark.context.model_type() == marks_t6::MARK_MODEL_TYPE_WORLD_BRUSH
        };
        let hit: Vec<u16> = self
            .live_marks()
            .filter(|&(_, mark)| on_pane(mark))
            .map(|(at, _)| at)
            .collect();
        let freed = hit.len() as u32;
        hit.into_iter().for_each(|at| self.release_mark(at));
        freed
    }

    fn release_mark(&mut self, slot: u16) {
        let Some(mark) = self
            .stored
            .get_mut(usize::from(slot))
            .and_then(Option::take)
        else {
            return;
        };
        self.tri_pool.reclaim(mark.triangle_list);
        self.point_pool.reclaim(mark.points);
        self.mat_names[usize::from(slot)] = None;
        self.handles.give_back(slot.into());
        self.in_use -= 1;
    }

    /// The mesh of every mark on the world lists.
    pub fn generate_level_stain_vertices(&self) -> DecalMeshTally {
        let mut mesh = DecalMeshTally::default();
        // The first mark the budget refuses ends the mesh.
        let _refused = self
            .live_marks()
            .filter(|(_, BuiltDecalRecord { context, .. })| context.is_world_list())
            .try_for_each(|(slot, mark)| self.emit_mark(&mut mesh, slot, *mark, &keep_point));
        mesh
    }

    /// Adds one mark to the mesh, placed on a model instead of where it
    /// was made.
    pub fn add_mesh_decal(
        &self,
        mesh: &mut DecalMeshTally,
        slot: u16,
        at: [f32; 3],
        axis: [f32; 3],
        place: &dyn Fn(EffectStainStagingPoint) -> EffectStainStagingPoint,
    ) -> Result<(), DecalMeshDenial> {
        let Some(base) = self.built_record(slot) else {
            return Ok(());
        };
        let mut moved = *base;
        moved.origin = at;
        moved.uv_projection_axis = axis;
        self.emit_mark(mesh, slot, moved, place)
    }

    fn emit_mark(
        &self,
        mesh: &mut DecalMeshTally,
        slot: u16,
        record: BuiltDecalRecord,
        place: &dyn Fn(EffectStainStagingPoint) -> EffectStainStagingPoint,
    ) -> Result<(), DecalMeshDenial> {
        let (base, _) =
            generate_mark_verts_begin(&mut mesh.budget, record.vertex_total_in_mark, record.tri_tally)?;
        let total = mesh.budget.vertex_amount as usize;
        mesh.vertex_points.resize(total, EffectStainStagingPoint::ZERO);
        mesh.vert_blob.resize(total, [0; DECAL_MESH_VERTEX_BYTES]);
        self.emit_points(mesh, &record, usize::from(base), place);
        self.emit_triangles(mesh, slot, &record, base);
        Ok(())
    }

    /// Writes the mark's points into the mesh's vertex arrays, starting at `first`.
    fn emit_points(
        &self,
        mesh: &mut DecalMeshTally,
        decal_hit: &BuiltDecalRecord,
        first: usize,
        place: &dyn Fn(EffectStainStagingPoint) -> EffectStainStagingPoint,
    ) {
        let pack = match decal_hit.context.uses_world_vertex() {
            true => pack_mark_world_vertex,
            false => pack_mark_model_vertex,
        };
        let wanted = usize::try_from(decal_hit.vertex_total_in_mark).unwrap_or(0);
        let raw_points = self
            .point_pool
            .walk(decal_hit.points)
            .flat_map(|group| group.points)
            .take(wanted);
        for (at, raw) in (first..).zip(raw_points) {
            let placed = place(raw);
            if let Some(vert) = mesh.vertex_points.get_mut(at) {
                *vert = placed;
            }
            if let Some(row) = mesh.vert_blob.get_mut(at) {
                let (pos, rad, axis) = (decal_hit.origin, decal_hit.radius, decal_hit.uv_projection_axis);
                *row = pack(&placed, pos, rad, axis, decal_hit.authored_tint);
            }
            mesh.lead_point.get_or_insert(placed.spot);
        }
    }

    /// Appends the mark's triangle indices, cutting a new draw surface wherever
    /// the material context changes between groups.
    fn emit_triangles(
        &self,
        mesh: &mut DecalMeshTally,
        slot: u16,
        mark: &BuiltDecalRecord,
        base: u16,
    ) {
        // (context, first index, index count) per run.
        let mut runs: Vec<(GfxMarkContext, u32, u32)> = Vec::new();
        let mut remaining = mark.tri_tally;
        for group in self.tri_pool.walk(mark.triangle_list) {
            if remaining == 0 {
                break;
            }
            let used = remaining.min(group.tri_tally);
            remaining -= used;
            let start = mesh.index_list.len() as u32;
            let corners = group.indices.iter().take(usize::from(used)).flatten();
            mesh.index_list
                .extend(corners.map(|&corner| base.wrapping_add(corner)));
            let added = mesh.index_list.len() as u32 - start;
            match runs.iter_mut().next_back() {
                Some(run) if run.2 == 0 || mark_contexts_equal(&run.0, &group.context) => {
                    *run = (group.context, run.1, run.2.saturating_add(added));
                }
                _ => runs.push((group.context, start, added)),
            }
        }
        if runs.is_empty() {
            runs.push((GfxMarkContext::ZERO, mesh.index_list.len() as u32, 0));
        }
        for (context, first_triangle_slot, index_count) in runs {
            let _ = add_mark_mesh_draw_surf(&mut mesh.budget);
            mesh.surfs.push(DecalMeshPatch {
                context,
                stain_cell: slot,
                index_count,
                first_triangle_slot,
            });
        }
    }

    /// Whether an impact is allowed to leave a mark, and on what.
    pub fn hit_decal(&self, req: DecalContactAsk) -> MarkOutcome {
        let within = match req.anchor {
            Some(spot) => impact_mark_within_range(req.camera, spot, self.range),
            None => true,
        };
        let open = within && impact_mark_outer_gate(req.receivers.stains, self.skip_decals);
        let DecalAcceptToggle {
            stains_on_ents: ents,
            stains_on_props: fixed_props,
            ..
        } = req.receivers;
        MarkOutcome {
            versus_models: open && impact_mark_models_generate(ents, fixed_props),
            against_level: open && !req.no_world,
            went_inside: open,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request for a mark of `triangle_list` triangles whose first one carries `first`.
    fn ask_for(triangle_list: u32, first: GfxMarkContext) -> DecalAllocationAsk {
        DecalAllocationAsk {
            any_stains: triangle_list > 0,
            tri_tally: triangle_list,
            vertex_total_in_mark: 3,
            origin: [4.0, 5.0, 6.0],
            radius: 8.0,
            uv_projection_axis: [1.0, 0.0, 0.0],
            authored_tint: 0xffff_ffff,
            material: u32::MIN,
            anim_steps: 1,
            leading_triangle_mark_info: first,
        }
    }

    fn flat_tri(context: GfxMarkContext) -> EffectStainStagingTri {
        EffectStainStagingTri {
            indices: [0, 1, 2],
            context,
            tail_fill: 0,
        }
    }

    /// Stores a one-triangle mark carrying `context`.
    fn put_mark(host: &mut DecalOwner, context: GfxMarkContext) -> u16 {
        host.claim_decal_for_scene_hook(
            ask_for(1, context),
            &[flat_tri(context)],
            &[EffectStainStagingPoint::ZERO; 3],
        )
        .expect("the mark should be accepted")
    }

    #[test]
    fn broken_pane_frees_only_its_world_marks() {
        let mut host = DecalOwner::init();
        let broken = put_mark(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 3));
        let neighbour = put_mark(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 4));
        let wall = put_mark(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        let fixed_prop = put_mark(
            &mut host,
            GfxMarkContext {
                model_type_and_surf: marks_t6::MARK_MODEL_TYPE_STATIC_MODEL,
                ..GfxMarkContext::glass_pane(1, 2, 3, 3)
            },
        );
        let entity = put_mark(&mut host, GfxMarkContext::entity_model(3, 0));
        let held_model = put_mark(
            &mut host,
            GfxMarkContext {
                model_type_and_surf: marks_t6::MARK_MODEL_TYPE_ENTITY_MODEL
                    | marks_t6::MARK_MODEL_VIEWMODEL_BIT,
                ..GfxMarkContext::entity_model(3, 0)
            },
        );

        assert_eq!(host.hide_glass_marks(3), 1);
        assert!(host.built_record(broken).is_none());
        for survivor in [neighbour, wall, fixed_prop, entity, held_model] {
            assert!(host.built_record(survivor).is_some());
        }
        assert_eq!(host.active_total(), 5);
    }

    #[test]
    fn breaking_pane_0_also_frees_world_brush_marks_as_in_bo2() {
        let mut host = DecalOwner::init();
        let wall = put_mark(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        let pane_0 = put_mark(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 0));
        let pane_1 = put_mark(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 1));

        assert_eq!(host.hide_glass_marks(0), 2);
        assert!(host.built_record(wall).is_none());
        assert!(host.built_record(pane_0).is_none());
        assert!(host.built_record(pane_1).is_some());
        assert_eq!(host.active_total(), 1);
    }

    fn ring_of_four() -> SlotRing {
        ring_from(vec![1, 2, 3, 9], 0, 9)
    }

    #[test]
    fn slot_ring_counts_and_hands_out_in_thread_order() {
        let mut ring = ring_of_four();
        assert_eq!(ring.spare(), 4);
        assert!(!ring.exhausted());
        assert_eq!(ring.claim(2), Some(vec![0, 1]));
        assert_eq!((ring.first, ring.spare()), (2, 2));
        assert_eq!(ring.claim(2), Some(vec![2, 3]));
        assert!(ring.exhausted());
        assert_eq!(ring.spare(), 0);
    }

    #[test]
    fn slot_ring_take_changes_nothing_when_the_thread_runs_dry() {
        let mut ring = ring_of_four();
        assert_eq!(ring.claim(5), None);
        assert_eq!(ring.first, 0);
        assert_eq!(ring.spare(), 4);
    }

    #[test]
    fn slot_ring_gives_a_slot_back_to_the_front() {
        let mut ring = ring_of_four();
        let taken = ring.claim(3).unwrap();
        ring.give_back(taken[1]);
        assert_eq!(ring.first, 1);
        assert_eq!(ring.claim(1), Some(vec![1]));
    }

    #[test]
    fn group_pool_lends_a_chain_and_takes_it_back() {
        let ring = ring_from(vec![1, 2, 3, 4, 7], 1, 7);
        let mut pool: GroupPool<DecalPointChunk> = GroupPool::new(ring, POINT_CHUNK_CHAIN_TERMINATOR);
        let head = pool.lend(3).expect("three spare groups");
        let chain: Vec<u16> = pool.walk_ids(head).collect();
        assert_eq!(chain, vec![3, 2, 1]);
        assert_eq!(pool.ledger.spare(), 1);
        assert_eq!(pool.lend(0), None);
        assert_eq!(pool.lend(2), None);
        pool.reclaim(head);
        assert_eq!(pool.ledger.spare(), 4);
        assert_eq!(pool.walk(head).count(), 1);
    }

    #[test]
    fn material_names_follow_the_mark() {
        let mut host = DecalOwner::init();
        let wall = put_mark(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        assert_eq!(host.surface_title(wall), None);
        host.assign_material_name(wall, Some("decal_hole"));
        assert_eq!(host.surface_title(wall), Some("decal_hole"));
        host.assign_material_name(wall, Some(""));
        assert_eq!(host.surface_title(wall), None);
        host.assign_material_name(wall, Some("decal_hole"));
        host.hide_glass_marks(0);
        assert_eq!(host.surface_title(wall), None);
    }

    #[test]
    fn marks_age_by_the_frame_time_between_updates() {
        let mut host = DecalOwner::init();
        let wall = put_mark(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        host.update_grid(1_000);
        assert_eq!(host.built_record(wall).unwrap().lived_ms, 0);
        host.update_grid(1_040);
        host.update_grid(1_050);
        assert_eq!(host.built_record(wall).unwrap().lived_ms, 50);
    }

    #[test]
    fn a_refused_request_leaves_the_pools_alone() {
        let mut host = DecalOwner::init();
        let (tri_free, point_free) = (host.tri_free_head(), host.point_free_head());
        let context = GfxMarkContext::world_surface(1, 2, 3);
        let nothing_cut = DecalAllocationAsk {
            any_stains: false,
            ..ask_for(1, context)
        };
        let points = [EffectStainStagingPoint::ZERO; 3];
        assert!(
            host.claim_decal_for_scene_hook(nothing_cut, &[flat_tri(context)], &points)
                .is_none()
        );
        // Two triangles asked for, one staged.
        assert!(
            host.claim_decal_for_scene_hook(ask_for(2, context), &[flat_tri(context)], &points)
                .is_none()
        );
        assert_eq!(host.active_total(), 0);
        assert_eq!((host.tri_free_head(), host.point_free_head()), (tri_free, point_free));
    }

    #[test]
    fn a_stored_mark_uses_groups_and_deleting_it_returns_them() {
        let mut host = DecalOwner::init();
        let (tri_free, point_free) = (host.tri_free_head(), host.point_free_head());
        let alloced = host.taken_count();
        put_mark(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 0));
        assert_ne!(host.tri_free_head(), tri_free);
        assert_ne!(host.point_free_head(), point_free);
        assert_eq!(host.taken_count(), alloced + 1);
        host.hide_glass_marks(0);
        assert_eq!(host.active_total(), 0);
        assert_eq!(host.tri_pool.ledger.spare(), host.tri_pool.ledger.links.len() as u32 - 1);
    }

    #[test]
    fn impact_marks_obey_the_gate_the_world_flag_and_the_receivers() {
        let mut host = DecalOwner::init();
        let ask = DecalContactAsk {
            receivers: DecalAcceptToggle::default(),
            no_world: false,
            anchor: None,
            camera: None,
        };
        let verdict = |entered, world, models| MarkOutcome {
            went_inside: entered,
            against_level: world,
            versus_models: models,
        };
        assert_eq!(host.hit_decal(ask), verdict(true, true, true));
        let skipping = DecalContactAsk {
            no_world: true,
            ..ask
        };
        assert_eq!(host.hit_decal(skipping), verdict(true, false, true));
        let no_models = DecalContactAsk {
            receivers: DecalAcceptToggle {
                stains_on_ents: false,
                stains_on_props: false,
                ..ask.receivers
            },
            ..ask
        };
        assert!(!host.hit_decal(no_models).versus_models);
        host.skip_decals = true;
        assert_eq!(host.hit_decal(ask), verdict(false, false, false));
    }

    #[test]
    fn impacts_beyond_the_range_are_refused() {
        let host = DecalOwner::init();
        let at_distance = |metres: f32| DecalContactAsk {
            receivers: DecalAcceptToggle::default(),
            no_world: false,
            anchor: Some([metres, 0.0, 0.0]),
            camera: Some([0.0; 3]),
        };
        assert!(host.hit_decal(at_distance(10.0)).went_inside);
        assert!(!host.hit_decal(at_distance(host.range * 2.0)).went_inside);
    }

    #[test]
    fn world_mesh_holds_world_marks_only() {
        let mut host = DecalOwner::init();
        put_mark(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        put_mark(&mut host, GfxMarkContext::entity_model(3, 0));
        let mesh = host.generate_level_stain_vertices();
        assert_eq!(mesh.vertex_points.len(), 3);
        assert_eq!(mesh.vert_blob.len(), 3);
        assert_eq!(mesh.index_list, vec![0, 1, 2]);
        assert_eq!(mesh.surfs.len(), 1);
        assert_eq!(
            (mesh.surfs[0].first_triangle_slot, mesh.surfs[0].index_count),
            (0, 3)
        );
        assert_eq!(mesh.lead_point, Some(EffectStainStagingPoint::ZERO.spot));
    }

    #[test]
    fn a_mark_that_changes_context_gets_one_surface_per_context() {
        let mut host = DecalOwner::init();
        let (left, right) = (
            GfxMarkContext::world_surface(1, 2, 3),
            GfxMarkContext::world_surface(4, 5, 6),
        );
        let slot = host
            .claim_decal_for_scene_hook(
                ask_for(2, left),
                &[flat_tri(left), flat_tri(right)],
                &[EffectStainStagingPoint::ZERO; 3],
            )
            .expect("accepted");
        let mesh = host.generate_level_stain_vertices();
        assert_eq!(mesh.index_list, vec![0, 1, 2, 0, 1, 2]);
        let spans: Vec<(u32, u32, u16)> = mesh
            .surfs
            .iter()
            .map(|face| (face.first_triangle_slot, face.index_count, face.stain_cell))
            .collect();
        assert_eq!(spans, vec![(0, 3, slot), (3, 3, slot)]);
        assert_eq!(mesh.surfs[0].context, left);
        assert_eq!(mesh.surfs[1].context, right);
    }

    #[test]
    fn a_model_mark_is_placed_by_the_given_origin_axis_and_transform() {
        let mut host = DecalOwner::init();
        let slot = put_mark(&mut host, GfxMarkContext::entity_model(3, 0));
        let mut mesh = DecalMeshTally::default();
        let lift = |mut point: EffectStainStagingPoint| {
            point.spot[2] += 10.0;
            point
        };
        host.add_mesh_decal(&mut mesh, slot, [1.0, 2.0, 3.0], [0.0, 1.0, 0.0], &lift)
            .unwrap();
        assert_eq!(mesh.lead_point, Some([0.0, 0.0, 10.0]));
        assert_eq!(mesh.vertex_points.len(), 3);
        // A slot with no mark adds nothing.
        host.add_mesh_decal(&mut mesh, slot + 1, [0.0; 3], [1.0, 0.0, 0.0], &lift)
            .unwrap();
        assert_eq!(mesh.vertex_points.len(), 3);
        assert_eq!(mesh.surfs.len(), 1);
    }
}
