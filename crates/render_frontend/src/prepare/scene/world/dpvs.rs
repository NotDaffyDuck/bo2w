//! The visibility data a spawned world carries: portals, the cell tree, the
//! per-surface draw runs and the frame-to-frame cull scratch.

use bevy::prelude::*;
use dpvs_t6::{AabbTree, Bounds, CPlane};

/// One opening from a cell into its neighbour.
#[derive(Clone, Debug)]
pub struct WorldPortal {
    pub plane: [f32; 4],
    pub neighbor: u16,
    pub vert_start: usize,
    pub vert_count: usize,
    pub hull_axis: Option<[[f32; 3]; 2]>,
}

/// The cell tree, portals and bounds the camera walk reads.
#[derive(Clone, Debug)]
pub struct WorldDpvs {
    pub planes: Vec<CPlane>,
    pub nodes: Vec<u16>,
    pub cell_count: usize,
    pub aabb_trees: Vec<Vec<AabbTree>>,
    pub aabb_smodel_indices: Vec<Vec<u16>>,
    pub sorted_surf_index: Vec<u16>,
    pub static_surface_count: usize,
    pub static_surface_count_no_decal: usize,
    pub surface_bounds: Vec<Bounds>,
    pub smodel_bounds: Vec<Bounds>,
    pub portal_verts: Vec<[f32; 3]>,
    pub portals_per_cell: Vec<Vec<WorldPortal>>,
    pub cell_reflection_probes: Vec<Vec<u8>>,
    /// Which cells can cast into which, for the sun's shadow partition.
    pub cell_caster_bits: Vec<u32>,
    pub lit_opaque_begin: u32,
    pub lit_opaque_end: u32,
    pub camera_ranges: asset_world::CameraSurfRanges,
    pub emissive_surfs_begin: u32,
    pub emissive_surfs_end: u32,
}

impl WorldDpvs {
    /// Fills `cell_caster_bits` for a sun shining along `sun_dir`; a zero
    /// direction leaves the matrix empty.
    pub(super) fn bake_cell_casters(&mut self, sun_dir: [f32; 3]) {
        let len_sq: f32 = sun_dir.iter().map(|c| c * c).sum();
        if len_sq < 1e-12 {
            self.cell_caster_bits.clear();
            return;
        }

        // The cells and portals as the camera walk reads them.
        let mut cells = Vec::with_capacity(self.cell_count);
        let mut portals = Vec::new();
        for list in self.portals_per_cell.iter().take(self.cell_count) {
            let first = portals.len();
            for portal in list {
                let Some(hull_axis) = portal.hull_axis else {
                    continue;
                };
                portals.push(dpvs_t6::Portal {
                    plane: portal.plane,
                    cell: portal.neighbor,
                    vert_start: portal.vert_start as u32,
                    vert_count: portal.vert_count as u32,
                    hull_axis,
                });
            }
            cells.push(dpvs_t6::Cell {
                portal_start: first as u32,
                portal_count: (portals.len() - first) as u32,
                ..Default::default()
            });
        }
        cells.resize(self.cell_count, dpvs_t6::Cell::default());

        let view = dpvs_t6::DpvsWorld {
            cells: &cells,
            portals: &portals,
            verts: &self.portal_verts,
            planes: &[],
            nodes: &[],
        };
        let mut bits = vec![0u32; dpvs_t6::cell_caster_matrix_words(self.cell_count)];
        dpvs_t6::generate_shadow_map_caster_cells(
            &view,
            sun_dir,
            &mut dpvs_t6::WalkScratch::default(),
            &mut bits,
        );
        self.cell_caster_bits = bits;
    }
}

/// A run of surfaces drawn under one sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldDrawItem {
    pub key: u64,
    /// The first surface of the run: its identity in sorts and ids.
    pub surf: u16,
    /// How many surfaces the run holds. They are
    /// `WorldCull::item_surfs[list_start..][..run]` and need not be contiguous.
    pub run: u16,
    pub list_start: u32,
    /// First surface of the draw-surf list entry; runs split by a 128 flush
    /// share their entry's.
    pub entry_first: u16,
    pub kind: WorldDrawItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldDrawItemKind {
    Bsp(asset_world::CameraRangeKind),
    BModel,
}

/// Everything the per-frame world cull reads and writes.
#[derive(Debug)]
pub struct WorldCull {
    pub packed_indices: Vec<u32>,
    pub surface_index_ranges: Vec<(u32, u32)>,
    pub surface_draw_fields: Vec<asset_world::SurfaceDrawFields>,
    pub surface_batch_ranges: Vec<(usize, u32, u32)>,
    pub surface_materials: Vec<Option<assets::MaterialIndex>>,
    pub surface_lightmap_indices: Vec<u8>,
    pub surface_reflection_probes: Vec<u8>,
    pub surface_primary_lights: Vec<u8>,
    pub sort_key_distortion: Option<u32>,
    pub surface_sort_keys: Vec<u8>,
    pub capture: asset_world::WorldCapture,
    pub brush_models: Vec<asset_world::GfxBrushModelSurfs>,
    pub brush_model_bounds: Vec<asset_world::GfxBrushModelBounds>,
    pub bmodel_world_from_local: Vec<Mat4>,
    pub t6_brush_surfaces: Vec<asset_world::T6BrushSurfaces>,
    pub dpvs: WorldDpvs,
    pub batch_count: u32,
    pub batch_lightmapped: Vec<bool>,
    pub surface_vis: Vec<u8>,
    pub draw_items: Vec<WorldDrawItem>,
    /// Surface indices of every draw item, in item order.
    pub item_surfs: Vec<u16>,
    pub bsp_run_scratch: Vec<dpvs_t6::BspDrawSurfRun<asset_world::CameraRangeKind>>,
    pub draw_items_id: u64,
    pub g0_surfs: Vec<u16>,
    pub static_model_entities: Vec<Option<Entity>>,
    pub static_model_cull_dists: Vec<u16>,
    pub smodel_vis: Vec<u8>,
    pub cell_vis: Vec<u32>,
    pub cell_vis_all: bool,
}

impl WorldCull {
    /// The surfaces of one draw item.
    #[must_use]
    pub fn draw_item_surfs(&self, item: &WorldDrawItem) -> &[u16] {
        let start = item.list_start as usize;
        self.item_surfs
            .get(start..start.saturating_add(usize::from(item.run)))
            .unwrap_or(&[])
    }
}
