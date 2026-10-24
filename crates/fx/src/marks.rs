//! The pool of impact marks (bullet holes, scorches): fixed-size storage for
//! the marks themselves and for the triangle and point groups they own,
//! recycled least-recently-drawn first, plus the mesh that draws them.

use marks_t6::{
    FX_MARK_HANDLE_NONE, FX_MARKS_INIT_ALLOCED_COUNT, FX_MARKS_LIMIT, FX_MARKS_RANGE_DEFAULT,
    FX_POINT_GROUP_CHAIN_NONE, FX_POINT_GROUP_LIMIT, FX_POINT_GROUP_NEXT_NONE,
    FX_TRI_GROUP_CHAIN_NONE, FX_TRI_GROUP_LIMIT, FX_TRI_GROUP_NEXT_NONE, FxAllocMarkRequest,
    FxMarkConstructed, FxMarkStagingPoint, FxMarkStagingTri, FxMarksGrid, FxPointGroup,
    FxTriGroup, GFX_MARK_MESH_VERTEX_STRIDE, GfxMarkContext, GfxMarkMeshBudget, GfxMarkMeshRefuse,
    add_mark_mesh_draw_surf, alloc_and_construct_mark, copy_mark_points, copy_mark_tris,
    generate_mark_verts_begin, impact_mark_models_generate, impact_mark_outer_gate,
    impact_mark_within_range, init_mark_next_handle, init_point_next_slot, init_tri_next_slot,
    lru_mark_pick, mark_contexts_equal, mark_point_groups_for_count, mark_tri_groups_for_staging,
    marks_active_limit, pack_mark_model_vertex, pack_mark_world_vertex,
};

/// One drawable run of a mark's indices: the same material context throughout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GfxMarkMeshSurf {
    pub index_start: u32,
    pub index_count: u32,
    pub mark_slot: u16,
    pub context: GfxMarkContext,
}

/// The mesh built from every mark drawn this frame, with the budget it was
/// built under.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GfxMarkMeshCensus {
    pub budget: GfxMarkMeshBudget,
    pub first_xyz: Option<[f32; 3]>,
    pub verts: Vec<FxMarkStagingPoint>,
    pub packed: Vec<[u8; GFX_MARK_MESH_VERTEX_STRIDE]>,
    pub indices: Vec<u16>,
    pub surfs: Vec<GfxMarkMeshSurf>,
}

/// What was recorded about one impact-mark attempt, for the debug console.
#[derive(Clone, Debug)]
pub struct MarkTraceRecord {
    pub id: u32,
    pub bolt: u8,
    pub entered: bool,
    pub against_world: bool,
    pub against_models: bool,
    pub parent: Option<String>,
    pub elem: Option<u8>,
    pub msec: Option<i32>,
    pub origin: Option<[f32; 3]>,
    pub size0: Option<f32>,
    pub color: Option<u32>,
    pub color_kind: &'static str,
    pub mat0: Option<String>,
    pub mat1: Option<String>,
    pub bound: Option<String>,
    pub slot: Option<u16>,
    pub tri_n: Option<u8>,
    pub point_n: Option<i16>,
    pub native_color: Option<u32>,
    pub context: Option<GfxMarkContext>,
    pub skip_why: Option<&'static str>,
    pub vis: Option<u8>,
    pub mat0_edge: Option<String>,
    pub mat1_edge: Option<String>,
    pub nx: Option<f32>,
    pub ny: Option<f32>,
    pub nz: Option<f32>,
}

/// Which kinds of surface are allowed to take marks.
#[derive(Clone, Copy, Debug)]
pub struct MarkReceiverEnable {
    pub fx_marks: bool,
    pub fx_marks_ents: bool,
    pub fx_marks_smodels: bool,
}

impl Default for MarkReceiverEnable {
    fn default() -> Self {
        Self {
            fx_marks: true,
            fx_marks_ents: true,
            fx_marks_smodels: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MarkImpactRequest {
    pub skip_world: bool,
    pub receivers: MarkReceiverEnable,
    /// None while there is no camera yet (every mark is in range then).
    pub camera: Option<[f32; 3]>,
    /// None skips the range check.
    pub origin: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkImpactResult {
    pub entered: bool,
    pub against_world: bool,
    pub against_models: bool,
}

/// A fixed set of slots whose unused ones are strung together through
/// `next`; `head` is the first unused one and `end` marks the last link.
#[derive(Clone, Debug)]
struct FreeChain {
    next: Vec<u32>,
    head: u32,
    end: u32,
}

impl FreeChain {
    fn is_empty(&self) -> bool {
        self.head == self.end
    }

    fn free_count(&self) -> u32 {
        let mut count = 0;
        let mut at = self.head;
        while at != self.end {
            count += 1;
            at = self.next[at as usize];
        }
        count
    }

    /// Takes `count` slots off the front, or none at all when the chain runs
    /// out of valid links first.
    fn pop_many(&mut self, count: u32) -> Option<Vec<u32>> {
        let before = self.head;
        let mut taken = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let Some(&after) = self.next.get(self.head as usize) else {
                self.head = before;
                return None;
            };
            taken.push(self.head);
            self.head = after;
        }
        Some(taken)
    }

    fn push(&mut self, slot: u32) {
        self.next[slot as usize] = self.head;
        self.head = slot;
    }
}

/// What a pool of linked groups needs from its element type.
trait Group: Clone {
    fn blank() -> Self;
    fn link(&self) -> u16;
    fn set_link(&mut self, next: u16);
}

impl Group for FxTriGroup {
    fn blank() -> Self {
        FxTriGroup::ZERO
    }
    fn link(&self) -> u16 {
        self.next
    }
    fn set_link(&mut self, next: u16) {
        self.next = next;
    }
}

impl Group for FxPointGroup {
    fn blank() -> Self {
        FxPointGroup::ZERO
    }
    fn link(&self) -> u16 {
        self.next
    }
    fn set_link(&mut self, next: u16) {
        self.next = next;
    }
}

/// Fixed storage for groups that marks borrow in chains.
#[derive(Clone, Debug)]
struct GroupPool<G: Group> {
    groups: Vec<G>,
    free: FreeChain,
    /// The `next` value that ends a chain a mark holds.
    chain_end: u16,
}

impl<G: Group> GroupPool<G> {
    fn new(links: Vec<u32>, head: u32, end: u32, chain_end: u16) -> Self {
        Self {
            groups: vec![G::blank(); links.len()],
            free: FreeChain {
                next: links,
                head,
                end,
            },
            chain_end,
        }
    }

    /// Lends out `count` groups as one chain and returns its first group.
    fn lend(&mut self, count: u32) -> Option<u16> {
        if count == 0 {
            return None;
        }
        let slots = self.free.pop_many(count)?;
        let mut head = self.chain_end;
        for slot in slots {
            let group = &mut self.groups[slot as usize];
            *group = G::blank();
            group.set_link(head);
            head = slot as u16;
        }
        Some(head)
    }

    /// Takes a chain back, group by group.
    fn reclaim(&mut self, mut head: u16) {
        while head != self.chain_end {
            let Some(group) = self.groups.get_mut(usize::from(head)) else {
                break;
            };
            let after = group.link();
            *group = G::blank();
            self.free.push(u32::from(head));
            head = after;
        }
    }
}

#[derive(Clone, Debug)]
pub struct FxMarksSystemHost {
    slots: FreeChain,
    live: u32,
    active_limit: u32,
    grid: FxMarksGrid,
    grid_msec: Option<i32>,
    alloced: u32,
    constructed: Vec<Option<FxMarkConstructed>>,
    material_names: Vec<Option<String>>,
    tris: GroupPool<FxTriGroup>,
    points: GroupPool<FxPointGroup>,

    pub no_marks: bool,
    /// fx_marks_grid_enable (BO2 default on): refuse a mark when its cell already holds 8.
    pub grid_enable: bool,
    /// fx_marks_range (BO2 default 1500): no mark further than this from the camera.
    pub range: f32,
}

impl FxMarksSystemHost {
    pub fn init() -> Self {
        let mark_links = (0..FX_MARKS_LIMIT)
            .map(|slot| u32::from(init_mark_next_handle(slot)))
            .collect::<Vec<u32>>();
        let mark_count = mark_links.len();
        let tri_links = (0..FX_TRI_GROUP_LIMIT)
            .map(init_tri_next_slot)
            .collect::<Vec<u32>>();
        let point_links = (0..FX_POINT_GROUP_LIMIT)
            .map(init_point_next_slot)
            .collect::<Vec<u32>>();
        Self {
            slots: FreeChain {
                next: mark_links,
                head: 0,
                end: u32::from(FX_MARK_HANDLE_NONE),
            },
            live: 0,
            active_limit: marks_active_limit(1),
            grid: FxMarksGrid::new(),
            grid_msec: None,
            alloced: FX_MARKS_INIT_ALLOCED_COUNT,
            constructed: vec![None; mark_count],
            material_names: vec![None; mark_count],
            // Group 0 is never lent out, so both group pools start at 1.
            tris: GroupPool::new(
                tri_links,
                1,
                FX_TRI_GROUP_NEXT_NONE,
                FX_TRI_GROUP_CHAIN_NONE,
            ),
            points: GroupPool::new(
                point_links,
                1,
                FX_POINT_GROUP_NEXT_NONE,
                FX_POINT_GROUP_CHAIN_NONE,
            ),
            no_marks: false,
            grid_enable: true,
            range: FX_MARKS_RANGE_DEFAULT,
        }
    }

    pub fn live_count(&self) -> u32 {
        self.live
    }

    pub fn alloced_count(&self) -> u32 {
        self.alloced
    }

    /// The first unused triangle group.
    pub fn tri_first_free(&self) -> u32 {
        self.tris.free.head
    }

    /// The first unused point group.
    pub fn point_first_free(&self) -> u32 {
        self.points.free.head
    }

    /// Stores a new mark built from the staged triangles and points, recycling
    /// the least recently drawn marks when the pools are full. `None` when the
    /// mark is refused (bad request, crowded spot, or nothing left to recycle).
    pub fn alloc_mark_from_go_callback(
        &mut self,
        req: FxAllocMarkRequest,
        staging_tris: &[FxMarkStagingTri],
        staging_points: &[FxMarkStagingPoint],
    ) -> Option<u16> {
        // A dry run of the construction rejects a request that could never
        // become a mark, before anything is spent on it.
        alloc_and_construct_mark(&req, 0, 0).ok()?;
        if self
            .grid
            .too_many_marks_in_vicinity(self.grid_enable, req.origin)
        {
            return None;
        }
        let tri_count = req.tri_count as usize;
        let point_count = req.point_count as usize;
        if staging_tris.len() < tri_count || staging_points.len() < point_count {
            return None;
        }
        let tris = &staging_tris[..tri_count];
        let points = &staging_points[..point_count];
        let tri_groups_needed = mark_tri_groups_for_staging(tris);
        let point_groups_needed = mark_point_groups_for_count(req.point_count);
        if tri_groups_needed == 0 || point_groups_needed == 0 {
            return None;
        }

        // Nothing is ever grown: make room by retiring the mark drawn longest ago.
        while self.live >= self.active_limit
            || self.slots.is_empty()
            || self.tris.free.free_count() < tri_groups_needed
            || self.points.free.free_count() < point_groups_needed
        {
            let oldest = lru_mark_pick(self.live_marks())?;
            self.release_mark(oldest);
        }

        let tri_head = self.tris.lend(tri_groups_needed)?;
        let point_head = self.points.lend(point_groups_needed)?;
        let tris_copied = copy_mark_tris(&mut self.tris.groups, tri_head, tris);
        let points_copied = copy_mark_points(&mut self.points.groups, point_head, points);
        if tris_copied != req.tri_count || points_copied != req.point_count {
            return None;
        }

        let mark = alloc_and_construct_mark(&req, tri_head, point_head).ok()?;
        let handle = self.slots.pop_many(1)?[0] as u16;
        self.constructed[usize::from(handle)] = Some(mark);
        self.live = self.live.saturating_add(1);
        self.alloced = self.alloced.saturating_add(1);
        Some(handle)
    }

    /// Ages every mark by the frame time. Every 500 ms of frame time the
    /// density grid is rebuilt from the world marks that have not expired.
    pub fn update_grid(&mut self, msec_now: i32) {
        let frame_msec = self
            .grid_msec
            .map_or(0, |previous| msec_now.wrapping_sub(previous));
        self.grid_msec = Some(msec_now);
        // How BO2 adds the time to a mark's age was not read out; one frame's
        // worth per call is what is assumed here.
        for mark in self.constructed.iter_mut().flatten() {
            mark.age_msec = mark.age_msec.saturating_add(frame_msec.max(0));
        }
        if !self.grid.advance(frame_msec) {
            return;
        }
        self.grid.clear();
        for mark in self.constructed.iter().flatten() {
            if mark.context.is_world_list() && !mark.alpha_fade.expired(mark.age_msec) {
                self.grid.add(mark.origin);
            }
        }
    }

    fn live_marks(&self) -> impl Iterator<Item = (u16, &FxMarkConstructed)> {
        self.constructed
            .iter()
            .enumerate()
            .filter_map(|(slot, mark)| mark.as_ref().map(|m| (slot as u16, m)))
    }

    pub fn set_material_name(&mut self, handle: u16, name: Option<&str>) {
        if let Some(entry) = self.material_names.get_mut(usize::from(handle)) {
            *entry = name.filter(|n| !n.is_empty()).map(str::to_owned);
        }
    }

    pub fn material_name(&self, handle: u16) -> Option<&str> {
        self.material_names
            .get(usize::from(handle))
            .and_then(|name| name.as_deref())
    }

    pub fn constructed(&self, handle: u16) -> Option<&FxMarkConstructed> {
        self.constructed
            .get(usize::from(handle))
            .and_then(|mark| mark.as_ref())
    }

    /// A pane breaking (BO2 `FX_MarkGlassDetachAll`) frees every world mark whose modelIndex
    /// is the glass id. Like BO2 it checks nothing else, and world-brush marks have
    /// modelIndex 0, so breaking pane 0 also frees every world-brush mark. Static-model and
    /// entity marks are on other lists in BO2, so they stay.
    pub fn hide_glass_marks(&mut self, glass_id: u16) -> u32 {
        let doomed: Vec<u16> = self
            .live_marks()
            .filter(|(_, mark)| {
                mark.context.model_type() == marks_t6::MARK_MODEL_TYPE_WORLD_BRUSH
                    && mark.context.model_index == glass_id
            })
            .map(|(slot, _)| slot)
            .collect();
        for &slot in &doomed {
            self.release_mark(slot);
        }
        doomed.len() as u32
    }

    fn release_mark(&mut self, handle: u16) {
        let index = usize::from(handle);
        let Some(mark) = self.constructed[index].take() else {
            return;
        };
        self.tris.reclaim(mark.tris);
        self.points.reclaim(mark.points);
        self.material_names[index] = None;
        self.slots.push(u32::from(handle));
        self.live -= 1;
    }

    /// The mesh of every mark on the world lists.
    pub fn generate_world_mark_verts(&self) -> GfxMarkMeshCensus {
        let mut census = GfxMarkMeshCensus::default();
        for (slot, mark) in self.live_marks() {
            if !mark.context.is_world_list() {
                continue;
            }
            if self
                .emit_mark(&mut census, slot, *mark, &|point| point)
                .is_err()
            {
                break;
            }
        }
        census
    }

    /// Adds one mark to the mesh, placed on a model instead of where it
    /// was made.
    pub fn append_model_mark(
        &self,
        census: &mut GfxMarkMeshCensus,
        slot: u16,
        origin: [f32; 3],
        tex_coord_axis: [f32; 3],
        transform_point: &dyn Fn(FxMarkStagingPoint) -> FxMarkStagingPoint,
    ) -> Result<(), GfxMarkMeshRefuse> {
        let Some(mut mark) = self.constructed(slot).copied() else {
            return Ok(());
        };
        mark.origin = origin;
        mark.tex_coord_axis = tex_coord_axis;
        self.emit_mark(census, slot, mark, transform_point)
    }

    fn emit_mark(
        &self,
        census: &mut GfxMarkMeshCensus,
        slot: u16,
        mark: FxMarkConstructed,
        transform_point: &dyn Fn(FxMarkStagingPoint) -> FxMarkStagingPoint,
    ) -> Result<(), GfxMarkMeshRefuse> {
        let (first_vert, _) =
            generate_mark_verts_begin(&mut census.budget, mark.point_count, mark.tri_count)?;
        let vert_total = census.budget.vert_n as usize;
        census.verts.resize(vert_total, FxMarkStagingPoint::ZERO);
        census
            .packed
            .resize(vert_total, [0u8; GFX_MARK_MESH_VERTEX_STRIDE]);

        self.emit_points(census, &mark, first_vert as usize, transform_point);
        self.emit_triangles(census, slot, &mark, first_vert);
        Ok(())
    }

    /// Writes the mark's points into the mesh's vertex arrays, two to a group.
    fn emit_points(
        &self,
        census: &mut GfxMarkMeshCensus,
        mark: &FxMarkConstructed,
        first_vert: usize,
        transform_point: &dyn Fn(FxMarkStagingPoint) -> FxMarkStagingPoint,
    ) {
        let pack = if mark.context.uses_world_vertex() {
            pack_mark_world_vertex
        } else {
            pack_mark_model_vertex
        };
        let mut left = mark.point_count.max(0) as u16;
        let mut group_at = mark.points;
        let mut write_at = first_vert;
        while left > 0 {
            let Some(group) = self.points.groups.get(usize::from(group_at)) else {
                break;
            };
            let here = left.min(2);
            for &raw in &group.points[..usize::from(here)] {
                let point = transform_point(raw);
                if let Some(vert) = census.verts.get_mut(write_at) {
                    *vert = point;
                }
                if let Some(row) = census.packed.get_mut(write_at) {
                    *row = pack(
                        &point,
                        mark.origin,
                        mark.radius,
                        mark.tex_coord_axis,
                        mark.native_color,
                    );
                }
                census.first_xyz.get_or_insert(point.xyz);
                write_at += 1;
            }
            left -= here;
            if group.next == FX_POINT_GROUP_CHAIN_NONE {
                break;
            }
            group_at = group.next;
        }
    }

    /// Appends the mark's triangle indices, starting a new draw surface each
    /// time the material context changes from one group to the next.
    fn emit_triangles(
        &self,
        census: &mut GfxMarkMeshCensus,
        slot: u16,
        mark: &FxMarkConstructed,
        first_vert: u16,
    ) {
        let mut left = mark.tri_count;
        let mut group_at = mark.tris;
        let mut context: Option<GfxMarkContext> = None;
        let mut run_start = census.indices.len() as u32;
        let mut run_len: u32 = 0;
        while left > 0 {
            let Some(group) = self.tris.groups.get(usize::from(group_at)) else {
                break;
            };
            let here = left.min(group.tri_count);
            if let Some(previous) = context {
                if run_len != 0 && !mark_contexts_equal(&previous, &group.context) {
                    let _ = add_mark_mesh_draw_surf(&mut census.budget);
                    census.surfs.push(GfxMarkMeshSurf {
                        index_start: run_start,
                        index_count: run_len,
                        mark_slot: slot,
                        context: previous,
                    });
                    run_start = census.indices.len() as u32;
                    run_len = 0;
                }
            }
            context = Some(group.context);
            for tri in group.indices.iter().take(here as usize) {
                for &corner in tri {
                    census.indices.push(first_vert.wrapping_add(corner));
                }
                run_len = run_len.saturating_add(3);
            }
            left -= here;
            if group.next == FX_TRI_GROUP_CHAIN_NONE {
                break;
            }
            group_at = group.next;
        }
        let _ = add_mark_mesh_draw_surf(&mut census.budget);
        census.surfs.push(GfxMarkMeshSurf {
            index_start: run_start,
            index_count: run_len,
            mark_slot: slot,
            context: context.unwrap_or(GfxMarkContext::ZERO),
        });
    }

    /// Whether an impact is allowed to leave a mark, and on what.
    pub fn impact_mark(&self, req: MarkImpactRequest) -> MarkImpactResult {
        let in_range = req
            .origin
            .is_none_or(|origin| impact_mark_within_range(req.camera, origin, self.range));
        if !impact_mark_outer_gate(req.receivers.fx_marks, self.no_marks) || !in_range {
            return MarkImpactResult {
                entered: false,
                against_world: false,
                against_models: false,
            };
        }
        MarkImpactResult {
            entered: true,
            against_world: !req.skip_world,
            against_models: impact_mark_models_generate(
                req.receivers.fx_marks_ents,
                req.receivers.fx_marks_smodels,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stores a one-triangle mark carrying `context`.
    fn stash(host: &mut FxMarksSystemHost, context: GfxMarkContext) -> u16 {
        let request = FxAllocMarkRequest {
            any_marks: true,
            tri_count: 1,
            point_count: 3,
            origin: [0.0; 3],
            radius: 8.0,
            tex_coord_axis: [1.0, 0.0, 0.0],
            native_color: 0xffff_ffff,
            material: 0,
            frame_count: 1,
            first_tri_context: context,
        };
        let tri = FxMarkStagingTri {
            indices: [0, 1, 2],
            context,
            _pad: 0,
        };
        host.alloc_mark_from_go_callback(request, &[tri], &[FxMarkStagingPoint::ZERO; 3])
            .expect("the mark should be accepted")
    }

    #[test]
    fn broken_pane_frees_only_its_world_marks() {
        let mut host = FxMarksSystemHost::init();
        let broken = stash(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 3));
        let neighbour = stash(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 4));
        let wall = stash(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        let static_model = stash(
            &mut host,
            GfxMarkContext {
                model_type_and_surf: marks_t6::MARK_MODEL_TYPE_STATIC_MODEL,
                ..GfxMarkContext::glass_pane(1, 2, 3, 3)
            },
        );
        let entity = stash(&mut host, GfxMarkContext::entity_model(3, 0));
        let held_model = stash(
            &mut host,
            GfxMarkContext {
                model_type_and_surf: marks_t6::MARK_MODEL_TYPE_ENTITY_MODEL
                    | marks_t6::MARK_MODEL_VIEWMODEL_BIT,
                ..GfxMarkContext::entity_model(3, 0)
            },
        );

        assert_eq!(host.hide_glass_marks(3), 1);
        assert!(host.constructed(broken).is_none());
        for survivor in [neighbour, wall, static_model, entity, held_model] {
            assert!(host.constructed(survivor).is_some());
        }
        assert_eq!(host.live_count(), 5);
    }

    #[test]
    fn breaking_pane_0_also_frees_world_brush_marks_as_in_bo2() {
        let mut host = FxMarksSystemHost::init();
        let wall = stash(&mut host, GfxMarkContext::world_surface(1, 2, 3));
        let pane_0 = stash(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 0));
        let pane_1 = stash(&mut host, GfxMarkContext::glass_pane(1, 2, 3, 1));

        assert_eq!(host.hide_glass_marks(0), 2);
        assert!(host.constructed(wall).is_none());
        assert!(host.constructed(pane_0).is_none());
        assert!(host.constructed(pane_1).is_some());
        assert_eq!(host.live_count(), 1);
    }
}
