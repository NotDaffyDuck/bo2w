//! The visibility data a spawned world carries: portals, the cell tree, the
//! per-surface draw runs and the frame-to-frame cull scratch.

use bevy::prelude::*;
use dpvs_t6::{AabbTree, Bounds, PlaneEquation};

/// One opening from a cell into its neighbour, as the map loader hands it over.
pub use asset_world::SelfContainedVisibilityPortal;

/// The cell tree, portals and bounds the camera walk reads.
#[derive(Clone, Debug)]
pub struct LevelSight {
    pub plane_set: Vec<PlaneEquation>,
    pub node_list: Vec<u16>,
    pub cells_n: usize,
    pub box_trees: Vec<Vec<AabbTree>>,
    pub aabb_prop_indices: Vec<Vec<u16>>,
    pub sorted_facet_index: Vec<u16>,
    pub static_facet_tally: usize,
    pub static_facet_tally_no_stain: usize,
    pub facet_bounds: Vec<Bounds>,
    pub prop_bounds: Vec<Bounds>,
    pub portal_vertices: Vec<[f32; 3]>,
    pub portals_per_room: Vec<Vec<SelfContainedVisibilityPortal>>,
    pub room_env_probes: Vec<Vec<u8>>,
    /// Which cells can cast into which, for the sun's shadow partition.
    pub room_caster_bits: Vec<u32>,
    pub lit_solid_begin: u32,
    pub lit_solid_end: u32,
    pub camera_spans: asset_world::ViewSurfaceSpans,
    pub emissive_facets_begin: u32,
    pub emissive_facets_end: u32,
}

impl LevelSight {
    /// Fills `room_caster_bits` for a sun shining along `toward`; a zero
    /// direction leaves the matrix empty.
    pub(super) fn bake_cell_casters(&mut self, toward: [f32; 3]) {
        let magnitude_squared: f32 = toward.iter().map(|c| c * c).sum();
        if magnitude_squared < 1e-12 {
            self.room_caster_bits.clear();
            return;
        }

        // The cells and portals as the camera walk reads them.
        let mut cells = Vec::with_capacity(self.cells_n);
        let mut portals = Vec::new();
        for list in self.portals_per_room.iter().take(self.cells_n) {
            let first = portals.len();
            for portal in list {
                let Some(hull_axes_dir) = portal.hull_axes_dir else {
                    continue;
                };
                portals.push(dpvs_t6::Portal {
                    plane: portal.split,
                    cell: portal.adjacent_cell,
                    vertex_first: portal.vertex_first as u32,
                    vertex_tally: portal.vertex_tally as u32,
                    hull_axes_dir,
                });
            }
            cells.push(dpvs_t6::Cell {
                portal_start: first as u32,
                portal_opening_total: (portals.len() - first) as u32,
                ..Default::default()
            });
        }
        cells.resize(self.cells_n, dpvs_t6::Cell::default());

        let view = dpvs_t6::DpvsWorld {
            cells: &cells,
            portals: &portals,
            vertex_points: &self.portal_vertices,
            planes: &[],
            nodes: &[],
        };
        let mut bit_set = vec![0u32; dpvs_t6::cell_caster_grid_word_total(self.cells_n)];
        dpvs_t6::collect_shadow_caster_cells(
            &view,
            toward,
            &mut dpvs_t6::TraversalScratchpad::default(),
            &mut bit_set,
        );
        self.room_caster_bits = bit_set;
    }
}

/// A run of surfaces drawn under one sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelDrawItem {
    pub key: u64,
    /// The first surface of the run: its identity in sorts and ids.
    pub facet_ix: u16,
    /// How many surfaces the run holds. They are
    /// `LevelHide::item_surfs[list_start..][..run]` and need not be contiguous.
    pub batch_run: u16,
    pub list_start: u32,
    /// First surface of the draw-surf list entry; runs split by a 128 flush
    /// share their entry's.
    pub entry_first: u16,
    pub kind: LevelDrawItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelDrawItemKind {
    Bsp(asset_world::CameraSpanKind),
    BModel,
}

/// Everything the per-frame world cull reads and writes.
#[derive(Debug)]
pub struct LevelHide {
    pub compact_indices: Vec<u32>,
    pub facet_index_ranges: Vec<(u32, u32)>,
    pub facet_draw_fields: Vec<asset_world::FaceDrawAttributes>,
    pub facet_batch_ranges: Vec<(usize, u32, u32)>,
    pub facet_materials: Vec<Option<assets::SurfaceMaterialId>>,
    pub facet_lightbake_indices: Vec<u8>,
    pub facet_env_probes: Vec<u8>,
    pub facet_key_lights: Vec<u8>,
    pub warp_sort_key: Option<u32>,
    pub facet_sort_keys: Vec<u8>,
    pub capture: asset_world::LevelSnapshot,
    pub solid_models: Vec<asset_world::SolidModelFaces>,
    pub solid_model_bounds: Vec<asset_world::SolidModelExtents>,
    pub solid_model_level_from_local: Vec<Mat4>,
    pub t6_brush_surfaces: Vec<asset_world::T6BrushSurfaces>,
    pub dpvs: LevelSight,
    pub batches_n: u32,
    pub batch_lightbaked: Vec<bool>,
    pub facet_vis: Vec<u8>,
    pub draw_batch_list: Vec<LevelDrawItem>,
    /// Surface indices of every draw item, in item order.
    pub item_surfs: Vec<u16>,
    pub level_tree_run_scratch: Vec<dpvs_t6::WorldFaceRun<asset_world::CameraSpanKind>>,
    pub draw_batch_list_id: u64,
    pub g0_facets: Vec<u16>,
    pub prop_entities: Vec<Option<Entity>>,
    pub prop_hide_dists: Vec<u16>,
    pub prop_vis: Vec<u8>,
    pub room_vis: Vec<u32>,
    pub room_vis_all: bool,
}

impl LevelHide {
    /// The surfaces of one draw item.
    #[must_use]
    pub fn draw_item_surfs(&self, item: &LevelDrawItem) -> &[u16] {
        let start = item.list_start as usize;
        self.item_surfs
            .get(start..start.saturating_add(usize::from(item.batch_run)))
            .unwrap_or(&[])
    }
}
