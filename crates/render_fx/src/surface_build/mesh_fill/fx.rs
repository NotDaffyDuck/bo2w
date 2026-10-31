//! Retained geometry for code-generated effect meshes: sprite quads, trail strips and
//! post-light volumes. The tessellator refills it every frame, the draw pass only reads it.

use std::sync::Arc;

use asset_t6::T6_PACKED_VERTEX;
use bevy::prelude::{Handle, Image, Resource, Transform, Vec3};
use fx_t6::runtime::{sprite_quad_indices, trail_pack_normal, trail_pack_texcoord};
use fx_t6::runtime::{pack_code_mesh_vertex, BillboardSheetCell, BILLBOARD_CORNER_OFFSETS};
use render_frame::{reserve_code_mesh, reserve_code_mesh_indices, reserve_code_mesh_verts};
use render_frame::{shrink_code_mesh_verts, MeshPoolBlock, PackedVertexRefusal};

/// Texture and sort rank of one material; shared by every effect plan.
#[derive(Debug, Clone)]
pub struct FxMaterialSlot {
    pub texture: Option<Handle<Image>>,
    /// Rank among the sorted materials, when the catalog has one.
    pub sorted_rank: Option<u32>,
    /// Bucket that decides when this material draws relative to others.
    pub bin: u8,
}

/// A consecutive stretch of indices that shares a single material.
#[derive(Debug, Clone, Copy)]
pub struct FxDrawRun {
    pub surface_material_index: u32,
    pub in_viewmodel: bool,
    /// The stretch of `FxCodeDrawPlan::indices` this run draws.
    pub indices: RunSpan,
    /// The rows of `FxCodeDrawPlan::shader_args` this run owns.
    pub args: RunSpan,
}

/// Where a run's rows start and how many there are.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunSpan {
    pub first: u32,
    pub count: u32,
}

impl RunSpan {
    /// The span from `first` up to a pool that now holds `total` rows.
    fn until(first: u32, total: u32) -> Self {
        Self { first, count: total.saturating_sub(first) }
    }

    fn as_pair(self) -> (u32, u32) {
        (self.first, self.count)
    }
}

/// A run that has indices to draw, paired with the material slot it points at.
pub struct LiveRun<'a, R> {
    pub index: usize,
    pub run: &'a R,
    pub slot: Option<&'a FxMaterialSlot>,
}

/// Pairs every run that draws anything with its material slot, skipping empty runs.
pub(super) fn live_runs<'a, R>(
    runs: &'a [R],
    slots: &'a [FxMaterialSlot],
    spans: impl Fn(&R) -> (u32, usize) + 'a,
) -> impl Iterator<Item = LiveRun<'a, R>> {
    runs.iter().enumerate().filter_map(move |(index, run)| {
        let (index_list, slot) = spans(run);
        (index_list != 0).then(|| LiveRun { index, run, slot: slots.get(slot) })
    })
}

#[derive(Debug, Clone, Default, Resource)]
pub struct FxCodeDrawPlan {
    pub vertices: Arc<Vec<[u8; T6_PACKED_VERTEX]>>,
    pub indices: Arc<Vec<u32>>,
    /// Per-draw shader parameters, four floats per row.
    pub shader_args: Vec<[f32; 4]>,
    pub slots: Vec<FxMaterialSlot>,
    pub runs: Vec<FxDrawRun>,
    pub revision: u64,
    /// Draws dropped because no material could be bound for them.
    pub unmatched_materials: u32,
    /// Fill levels of the vertex, index and argument pools.
    pub pools: MeshPoolBlock,
    /// Appends turned away this frame because a pool had no room left.
    pub refused_appends: u32,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
}

/// Normalized `dir`, or `fallback` if `dir` has no usable length.
fn unit_or(dir: Vec3, backup_dir: Vec3) -> Vec3 {
    dir.try_normalize().unwrap_or(backup_dir)
}

/// Packs the rotated `axis` of a sprite into its trail-style normal word.
fn packed_axis(rotation: bevy::prelude::Quat, axis: Vec3) -> u32 {
    trail_pack_normal(unit_or(rotation * axis, axis).to_array())
}

// Reading a plan back.
impl FxCodeDrawPlan {
    /// The runs that draw something, each with its material slot.
    pub fn live_runs(&self) -> impl Iterator<Item = LiveRun<'_, FxDrawRun>> {
        live_runs(&self.runs, &self.slots, |run| {
            (run.indices.count, run.surface_material_index as usize)
        })
    }

    /// The shader argument rows of a run: at most two, zero where the plan has none.
    pub fn shader_rows(&self, run: &FxDrawRun) -> (u8, [[f32; 4]; 2]) {
        let wanted = (run.args.count as usize).min(2);
        let first = run.args.first as usize;
        let mut rows = [[0.0; 4]; 2];
        for (row, found) in rows.iter_mut().zip(self.shader_args.iter().skip(first)).take(wanted) {
            *row = *found;
        }
        (wanted as u8, rows)
    }
}

// Pool access and bookkeeping.
impl FxCodeDrawPlan {
    const ACCEPTED: bool = true;

    fn vertex_pool(&mut self) -> &mut Vec<[u8; T6_PACKED_VERTEX]> {
        let rows = &mut self.vertices;
        Arc::make_mut(rows)
    }

    fn index_pool(&mut self) -> &mut Vec<u32> {
        let rows = &mut self.indices;
        Arc::make_mut(rows)
    }

    /// Counts a rejected append and reports it as not accepted.
    fn refuse(&mut self) -> bool {
        let refused = &mut self.refused_appends;
        *refused = refused.saturating_add(1);
        !Self::ACCEPTED
    }

    /// Returns the plan to its empty per-frame state.
    pub fn reset_frame(&mut self) {
        let Self { vertices, indices, shader_args, slots: cells, runs, .. } = self;
        super::recycle_rows(vertices);
        super::recycle_rows(indices);
        shader_args.clear();
        cells.clear();
        runs.clear();
        (self.range_portion, self.unmatched_materials, self.refused_appends) = (None, 0, 0);
        self.pools = MeshPoolBlock::engine_generated_mesh();
    }

    pub fn advance_revision(&mut self) {
        self.revision = 1u64.wrapping_add(self.revision);
    }

    /// Snapshots each run's index span so other passes can see it without the plan.
    pub fn publish_draw_ranges(&mut self) {
        let spans = self.runs.iter().map(|run| run.indices.as_pair());
        self.range_portion = Some(super::collect_index_ranges(spans));
    }

    pub fn packed_vertex_rows(&self) -> &[[u8; T6_PACKED_VERTEX]] {
        &self.vertices[..]
    }

    pub fn index_slice(&self) -> &[u32] {
        &self.indices[..]
    }

    /// Hands out the packed rows only when the declaration table agrees on their stride.
    pub fn precise_compact_vertices(&self) -> Result<&[[u8; T6_PACKED_VERTEX]], PackedVertexRefusal> {
        use render_material::decl_table::{stream_extent, COMPRESSED_VERTEX_KIND};
        let table_stride = stream_extent(COMPRESSED_VERTEX_KIND, 0);
        if table_stride == Some(T6_PACKED_VERTEX as u16) {
            return Ok(self.packed_vertex_rows());
        }
        Err(PackedVertexRefusal::StrideMismatch { table_stride })
    }
}

// Appending geometry.
impl FxCodeDrawPlan {
    /// Adds a flat sprite quad (4 vertices, 6 indices) placed by `placement`. If either pool
    /// is full nothing is added and `false` comes back.
    pub fn append_sprite_quad(
        self: &mut Self,
        placement: Transform,
        rgba: [u8; 4],
        atlas_rect: BillboardSheetCell,
    ) -> bool {
        // The four corners and their six indices are taken together or not at all.
        let Some((first_vert, _, _)) = reserve_code_mesh(&mut self.pools, 4, 6, 0) else { return self.refuse() };
        let first_vert = u32::from(first_vert);
        debug_assert_eq!(first_vert as usize, self.vertices.len());

        let tangent_word = packed_axis(placement.rotation, Vec3::X);
        let normal_word = packed_axis(placement.rotation, Vec3::Z);

        let corner_uvs = atlas_rect.corners();
        let rows = BILLBOARD_CORNER_OFFSETS
            .iter()
            .zip(corner_uvs.iter())
            .map(|(local, uv)| {
                let at = placement.transform_point(Vec3::new(local[0], local[1], 0.0));
                pack_code_mesh_vertex(
                    at.to_array(),
                    rgba,
                    trail_pack_texcoord(uv[0], uv[1]),
                    normal_word,
                    tangent_word,
                )
            })
            .collect::<Vec<_>>();
        self.vertex_pool().extend(rows);
        self.index_pool().extend_from_slice(&sprite_quad_indices(first_vert));
        Self::ACCEPTED
    }

    /// Adds one trail vertex whose attributes are already packed.
    pub fn append_trail_vertex(
        self: &mut Self,
        xyz: [f32; 3],
        rgba: [u8; 4],
        word_trio: [u32; 3],
    ) -> bool {
        let Some(_slot) = reserve_code_mesh_verts(&mut self.pools, 1) else { return self.refuse() };
        let [uv_word, normal_word, tangent_word] = word_trio;
        let packed = pack_code_mesh_vertex(xyz, rgba, uv_word, normal_word, tangent_word);
        self.vertex_pool().push(packed);
        Self::ACCEPTED
    }

    /// Adds a post-light volume: 16 vertices, 84 indices and 2 argument rows.
    pub fn append_post_light(&mut self, volume: &fx_t6::AfterglowMeshBatch) -> bool {
        let Some((first_vert, _, first_arg)) = reserve_code_mesh(&mut self.pools, 16, 84, 2) else { return self.refuse() };
        let first_vert = u32::from(first_vert);
        debug_assert_eq!(first_vert as usize, self.vertices.len());
        debug_assert_eq!(first_arg, Some(self.shader_args.len() as u32));

        let color = volume.tint_word;
        let packed_corners: Vec<_> = volume
            .vertex_points
            .iter()
            .map(|&corner| fx_t6::post_light_pack_vert(corner, color))
            .collect();
        self.vertex_pool().extend(packed_corners);
        let shifted: Vec<u32> = volume
            .indices
            .iter()
            .map(|&local| first_vert.wrapping_add(u32::from(local)))
            .collect();
        self.index_pool().extend(shifted);
        self.shader_args.extend_from_slice(&volume.knobs);
        Self::ACCEPTED
    }

    /// Appends indices that already point into the vertex pool.
    pub fn append_indices(&mut self, more: &[u32]) -> bool {
        if reserve_code_mesh_indices(&mut self.pools, more.len() as u32).is_none() {
            return self.refuse();
        }
        self.index_pool().extend_from_slice(more);
        Self::ACCEPTED
    }

    /// Cuts the vertex pool back so only `keep` vertices remain.
    pub fn cut_vertices_to(&mut self, keep: u32) {
        let pools = &mut self.pools;
        shrink_code_mesh_verts(pools, pools.vertices_consumed.wrapping_sub(keep));
        let remaining = self.pools.vertices_consumed as usize;
        self.vertex_pool().truncate(remaining);
    }

    /// Cuts the index pool back so only `keep` indices remain.
    pub fn cut_indices_to(&mut self, keep: u32) {
        self.pools.index_fill = keep;
        self.index_pool().truncate(keep as usize);
    }
}

// Draw runs.
impl FxCodeDrawPlan {
    /// Starts a draw run for one material and returns the run number to pass to `close_run`.
    pub fn open_run(&mut self, texture: Option<Handle<Image>>, bucket: u8, sorted_rank: Option<u32>) -> u32 {
        let run_number = self.runs.len() as u32;
        let surface_material_index = self.slots.len() as u32;

        self.slots.push(FxMaterialSlot { texture, sorted_rank, bin: bucket });
        let starts = (self.indices.len() as u32, self.shader_args.len() as u32);
        self.runs.push(FxDrawRun {
            surface_material_index,
            in_viewmodel: false,
            indices: RunSpan { first: starts.0, count: 0 },
            args: RunSpan { first: starts.1, count: 0 },
        });
        run_number
    }

    /// Runs `fill` between opening and closing one draw run and gives back what it returned.
    pub fn in_run<T>(
        &mut self,
        texture: Option<Handle<Image>>,
        bucket: u8,
        sorted_rank: Option<u32>,
        fill: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let run = self.open_run(texture, bucket, sorted_rank);
        let filled = fill(self);
        self.close_run(run);
        filled
    }

    /// Counts `draws` that found no material to bind.
    pub fn note_unmatched(&mut self, draws: u32) {
        self.unmatched_materials = self.unmatched_materials.saturating_add(draws);
    }

    /// Finishes run `run_number` by measuring how much was appended since it started.
    pub fn close_run(&mut self, run_number: u32) {
        let totals = (self.indices.len() as u32, self.shader_args.len() as u32);
        let Some(run) = self.runs.get_mut(run_number as usize) else { return };
        run.indices = RunSpan::until(run.indices.first, totals.0);
        run.args = RunSpan::until(run.args.first, totals.1);
        run.args.count = run.args.count.min(u32::from(u16::MAX));
    }
}
