//! Particle clouds. A fixed lattice of cells is expanded once into a vertex/index template
//! (spark cells first, then smoke cells); every frame's draws just select a slice of it.

use std::cmp::Ordering;
use std::sync::Arc;

use bevy::prelude::{Handle, Image, Resource};
use fx_t6::runtime::{point_uv_corner_bytes, classic_dice_unit, CLASSIC_DICE_START};
use fx_t6::runtime::{VisualParticleCloud, PointUvCorner, POINT_UV_CORNER_BYTES};
use fx_t6::runtime::{SWARM_IS_EMBERS, SWARM_STAMP_SPECKS};
use fx_t6::runtime::{SWARM_LATTICE_ACROSS, SWARM_LATTICE_DEEP};
use fx_t6::runtime::{
    particle_cloud_particle_id, particle_spark_cell_indices, particle_spark_cell_verts,
    SWARM_LATTICE_TALL,
};
use fx_t6::runtime::{SWARM_TRIANGLE_SLOTS_EACH_SPECK, SWARM_CORNERS_EACH_SPECK};
use fx_t6::runtime::{EMBER_TRIANGLE_SLOTS_EACH_SPECK, EMBER_CORNERS_EACH_SPECK};
use fx_t6::runtime::{particle_cloud_cell_indices, particle_cloud_cell_radius_sq, particle_cloud_cell_verts};
use fx_t6::runtime::{particle_cloud_cell_xyz, particle_cloud_compare_cell_radius, particle_cloud_draw_counts};

use super::fx::FxMaterialSlot;

/// One cloud draw: the material it uses, its index span and the three cloud descriptors.
#[derive(Debug, Clone, Copy)]
pub struct FxCloudDrawRun {
    pub surface_material_index: u32,
    pub first_index: u32,
    pub num_indices: u32,
    pub descriptors: [VisualParticleCloud; 3],
}

#[derive(Debug, Clone, Resource)]
pub struct FxCloudDrawPlan {
    pub vertices: Arc<Vec<[u8; POINT_UV_CORNER_BYTES]>>,
    pub indices: Arc<Vec<u32>>,
    pub cell_list: Vec<FxMaterialSlot>,
    pub runs: Vec<FxCloudDrawRun>,
    pub revision: u64,
    /// Clouds dropped because no material could be bound for them.
    pub unmatched_materials: u32,
    /// Length of the spark part of the index template; smoke indices follow it directly.
    pub spark_index_total: u32,
    /// Vertex and index counts of the template at the head of each pool. Whatever lies
    /// past those lengths was appended for the current frame.
    template_verts: u32,
    template_indices: u32,
    /// Position, squared radius and rand seed of the first template cell (diagnostics only).
    pub probe_first_xyz: [f32; 3],
    pub probe_first_r2: f32,
    pub probe_rand_seed: u32,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
}

impl core::default::Default for FxCloudDrawPlan {
    fn default() -> FxCloudDrawPlan {
        let built = CloudTemplate::generate();
        let template_verts = built.vertex_points.len() as u32;
        let template_indices = built.index_list.len() as u32;

        FxCloudDrawPlan {
            vertices: Arc::new(built.vertex_points),
            indices: Arc::new(built.index_list),
            cell_list: vec![],
            runs: vec![],
            revision: u64::default(),
            unmatched_materials: 0,
            spark_index_total: built.spark_index_total,
            template_verts,
            template_indices,
            probe_first_xyz: built.first_cell_xyz,
            probe_first_r2: built.first_cell_r2,
            probe_rand_seed: built.dice_origin,
            range_portion: Default::default(),
        }
    }
}

impl FxCloudDrawPlan {
    /// The runs that draw something, each with its material slot.
    pub fn live_runs(&self) -> impl Iterator<Item = super::fx::LiveRun<'_, FxCloudDrawRun>> {
        super::fx::live_runs(&self.runs, &self.cell_list, |run| {
            (run.num_indices, run.surface_material_index as usize)
        })
    }

    /// Drops the frame's runs and trims both pools back to the bare template.
    pub fn drop_frame_draws(&mut self) {
        self.cell_list.clear();
        self.runs.clear();
        drop(self.range_portion.take());
        self.unmatched_materials = 0;

        let (verts_kept, indices_kept) = self.kept_template_rows();
        if verts_kept < self.vertices.len() {
            Arc::make_mut(&mut self.vertices).truncate(verts_kept);
        }
        if indices_kept < self.indices.len() {
            Arc::make_mut(&mut self.indices).truncate(indices_kept);
        }
    }

    pub fn advance_revision(&mut self) {
        self.revision = 1u64.wrapping_add(self.revision);
    }

    /// `(vertices, indices)` occupied by the template at the head of the pools.
    pub fn kept_template_rows(&self) -> (usize, usize) {
        (self.template_verts as usize, self.template_indices as usize)
    }

    /// Snapshots each run's index span for passes that only need the ranges.
    pub fn publish_draw_ranges(&mut self) {
        let spans = self.runs.iter().map(|run| (run.first_index, run.num_indices));
        self.range_portion = Some(super::collect_index_ranges(spans));
    }

    /// Queues a run over the template and returns its number. Spark clouds start at the
    /// front of the index pool; smoke clouds start after the spark section.
    /// Counts `draws` that found no material to bind.
    pub fn note_unmatched(&mut self, draws: u32) {
        self.unmatched_materials = self.unmatched_materials.saturating_add(draws);
    }

    pub fn open_cloud_run(self: &mut Self, clouds: [VisualParticleCloud; 3], texture: Option<Handle<Image>>, bucket: u8, sorted_rank: Option<u32>) -> u32 {
        let run_number = self.runs.len() as u32;
        let surface_material_index = self.cell_list.len() as u32;
        self.cell_list.push(FxMaterialSlot { texture, sorted_rank, bin: bucket });

        let is_spark = clouds[0].flags & SWARM_IS_EMBERS != 0;
        let (_, triangles) = particle_cloud_draw_counts(is_spark, clouds[0].flags);
        let first_index = if is_spark { 0 } else { self.spark_index_total };
        self.runs.push(FxCloudDrawRun {
            surface_material_index,
            first_index,
            num_indices: triangles.wrapping_mul(3),
            descriptors: clouds,
        });
        run_number
    }
}

/// Spark cell: eight vertices.
/// Smoke cell: four vertices.

/// Output of the one-time lattice build.
struct CloudTemplate {
    vertex_points: Vec<[u8; POINT_UV_CORNER_BYTES]>,
    index_list: Vec<u32>,
    spark_index_total: u32,
    first_cell_xyz: [f32; 3],
    first_cell_r2: f32,
    dice_origin: u32,
}

/// Sort predicate: nearer-to-centre cells first, judged by each cell's first vertex.
fn nearer_cell_first<const N: usize>(
    a: &[PointUvCorner; N],
    b: &[PointUvCorner; N],
) -> Ordering {
    particle_cloud_compare_cell_radius(a[0].spot, b[0].spot).cmp(&0)
}

/// Raw lattice contents before sorting and packing.
struct Scatter {
    sparks: Vec<[PointUvCorner; 8]>,
    smokes: Vec<[PointUvCorner; 4]>,
    spark_idx: Vec<u32>,
    seed: u32,
}

/// Walks the lattice once, drawing three rand values per cell (x, y, z order) so the
/// scatter is identical on every run.
fn scatter_cells() -> Scatter {
    let capacity = SWARM_STAMP_SPECKS;
    let mut out = Scatter {
        sparks: Vec::with_capacity(capacity),
        smokes: Vec::with_capacity(capacity),
        spark_idx: Vec::with_capacity(capacity * EMBER_TRIANGLE_SLOTS_EACH_SPECK as usize),
        seed: CLASSIC_DICE_START,
    };

    for (gx, gy, gz) in (0..SWARM_LATTICE_ACROSS).flat_map(|x| (0..SWARM_LATTICE_DEEP).flat_map(move |y| (0..SWARM_LATTICE_TALL).map(move |z| (x, y, z)))) {
        let seed = &mut out.seed;
        let jitter = [classic_dice_unit(seed), classic_dice_unit(seed), classic_dice_unit(seed)];
        let middle = particle_cloud_cell_xyz(gx, gy, gz, jitter);
        out.sparks.push(particle_spark_cell_verts(middle));
        out.smokes.push(particle_cloud_cell_verts(middle));

        let id = particle_cloud_particle_id(gx, gy, gz);
        let spark_cell_idx = particle_spark_cell_indices(id);
        out.spark_idx.extend(spark_cell_idx.into_iter().map(u32::from));
    }
    out
}

impl CloudTemplate {
    /// Builds the whole template: spark and smoke cells sorted by radius, spark indices
    /// first, smoke indices rebased past the spark vertices.
    fn generate() -> Self {
        let cell_total = SWARM_STAMP_SPECKS;
        let scattered = scatter_cells();
        let (mut sparks, mut smokes) = (scattered.sparks, scattered.smokes);

        sparks.sort_unstable_by(nearer_cell_first);
        smokes.sort_unstable_by(nearer_cell_first);

        let first_cell_xyz = sparks[0][0].spot;
        let spark_verts = cell_total * EMBER_CORNERS_EACH_SPECK as usize;
        let vertex_points: Vec<[u8; POINT_UV_CORNER_BYTES]> = sparks
            .iter()
            .flatten()
            .chain(smokes.iter().flatten())
            .copied()
            .map(point_uv_corner_bytes)
            .collect();

        let spark_index_total = scattered.spark_idx.len() as u32;
        let smoke_base = spark_verts as u32;
        let mut index_list = scattered.spark_idx;
        for cell in 0..cell_total as u32 {
            for idx in particle_cloud_cell_indices(cell) {
                index_list.push(smoke_base.wrapping_add(u32::from(idx)));
            }
        }

        debug_assert_eq!(
            vertex_points.len(),
            spark_verts + cell_total * SWARM_CORNERS_EACH_SPECK as usize
        );
        let indices_per_cell = EMBER_TRIANGLE_SLOTS_EACH_SPECK as usize + SWARM_TRIANGLE_SLOTS_EACH_SPECK as usize;
        debug_assert_eq!(index_list.len(), cell_total * indices_per_cell);

        CloudTemplate {
            vertex_points,
            index_list,
            spark_index_total,
            first_cell_xyz,
            first_cell_r2: particle_cloud_cell_radius_sq(first_cell_xyz),
            dice_origin: scattered.seed,
        }
    }
}
