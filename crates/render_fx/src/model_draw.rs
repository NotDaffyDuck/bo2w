//! Drawing plan for effect-spawned models. One shared, reference-counted geometry block
//! holds every vertex and index; each frame adds a list of surface draws pointing into it.

use std::sync::Arc;

use bevy::prelude::Resource;
use render_frame::PropVertex;
use render_frame::InputChangeCounters;
use render_scene::{ResolvedModelIllum, PropPassMaterial, FigureFacetDraw};

/// Lowest object id reserved for effect models; other model kinds stay below it.
pub const FIGURE_OBJECT_ID_EFFECT_BASE: u16 = 0x0700;

/// Locates the surfaces of one (model index, lod) pair inside the shared geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct FxModelSurfaces {
    pub surf_list: Vec<(u32, u32)>,
    pub lod: u8,
    pub mesh_slot_number: usize,
}

/// Immutable-once-built vertex/index storage shared between staged and live plans.
#[derive(Debug, Default, Clone)]
pub(super) struct FxModelGeometry {
    pub(super) compact_vertices: asset_world::PackedVertexPayload,
    pub(super) vertices: Vec<PropVertex>,
    pub(super) indices: Vec<u32>,
    pub(super) facet_ranges: Vec<(u32, u32)>,
    pub(super) materials: Vec<PropPassMaterial>,
    pub(super) assets: Vec<FxModelSurfaces>,
}

/// Scratch plan that a frame is assembled in before it is published.
#[derive(Default, Resource)]
pub struct FxModelStagingPlan(pub FxModelDrawList);

/// Plan holding geometry that was built ahead of time and is ready to be adopted.
#[derive(Debug, Default, Clone, Resource)]
pub struct PreparedFxModelPlan(pub FxModelDrawList);

/// Why an effect model was left out of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDrop {
    NoCatalog,
    NoPose,
    NoLod,
    Culled,
    NoMaterial,
    NoLighting,
    RenderFlags,
}

/// How many models were left out for each reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropTally([u32; 7]);

impl DropTally {
    pub fn count_of(&self, why: ModelDrop) -> u32 {
        self.0[why as usize]
    }

    pub fn add_to(&mut self, why: ModelDrop, total: u32) {
        let slot = &mut self.0[why as usize];
        *slot = slot.saturating_add(total);
    }

    pub fn set_to(&mut self, why: ModelDrop, tally: u32) {
        self.0[why as usize] = tally;
    }
}

/// Everything the renderer needs to draw effect models for one frame.
#[derive(Debug, Default, Clone, Resource)]
pub struct FxModelDrawList {
    pub(super) geometry: Arc<FxModelGeometry>,
    pub(super) draw_list: Vec<FigureFacetDraw>,
    /// Models that were emitted this frame.
    pub emitted: u32,
    /// Models left out, one counter per reason.
    pub dropped: DropTally,
    pub revisions: InputChangeCounters,
    pub generation: u64,
    pub edit_no: u64,
}

impl FxModelDrawList {
    /// A plan for material generation `generation` that holds no geometry and has drawn nothing.
    pub fn without_geometry(generation: u64) -> Self {
        Self {
            geometry: Arc::new(FxModelGeometry::default()),
            draw_list: Vec::new(),
            emitted: 0,
            dropped: DropTally::default(),
            revisions: InputChangeCounters::default(),
            generation,
            edit_no: 0,
        }
    }

    /// Surface draws queued for this frame.
    pub fn draws(&self) -> &'_ [FigureFacetDraw] {
        self.draw_list.as_slice()
    }

    pub fn queue_draw(&mut self, draw: FigureFacetDraw) {
        Vec::push(&mut self.draw_list, draw);
    }

    pub fn materials(&self) -> &'_ [PropPassMaterial] {
        self.geometry.materials.as_slice()
    }

    pub fn vertices(&self) -> &'_ [PropVertex] {
        self.geometry.vertices.as_slice()
    }

    pub fn indices(&self) -> &'_ [u32] {
        self.geometry.indices.as_slice()
    }

    pub fn facet_ranges(&self) -> &'_ [(u32, u32)] {
        self.geometry.facet_ranges.as_slice()
    }

    pub fn compact_vertices(&self) -> &'_ asset_world::PackedVertexPayload {
        &(*self.geometry).compact_vertices
    }

    /// Looks up the surfaces recorded for a model at a given lod.
    pub fn find_asset(&self, prop: usize, lod: u8) -> Option<&FxModelSurfaces> {
        let block = &self.geometry;
        block.assets.iter().find(|entry| entry.mesh_slot_number == prop && entry.lod == lod)
    }

    /// Resets for a new frame: drops the queued draws and zeroes every counter while
    /// keeping the shared geometry.
    pub fn reset_frame(&mut self) {
        Vec::clear(&mut self.draw_list);
        self.emitted = 0;
        self.dropped = DropTally::default();
    }

    /// Takes over geometry that was prepared elsewhere.
    pub fn adopt_prepared(&mut self, prepared: &Self) {
        self.geometry = Arc::clone(&prepared.geometry);
        let Self { generation: epoch, .. } = prepared;
        self.generation = *epoch;
    }

    /// Switches to the staged plan's geometry and refreshes the dependent revisions.
    fn adopt_geometry(&mut self, staged: &Self) {
        self.geometry = Arc::clone(&staged.geometry);
        let block = &*self.geometry;
        self.revisions.stamp_topology(
            &block.indices,
            &block.facet_ranges,
            block.vertices.len(),
        );
        let revision_slot = &mut self.edit_no;
        self.revisions.note_packed_data_rewritten(revision_slot);
    }

    /// Moves a staged rebuild into this plan. A revision is bumped only when the data it
    /// covers really changed.
    pub fn publish_staged(&mut self, staged: &mut Self) {
        let Self { generation: staged_generation, .. } = &*staged;
        self.generation = *staged_generation;
        let geometry_swapped = !Arc::ptr_eq(&self.geometry, &staged.geometry);
        if geometry_swapped {
            self.adopt_geometry(staged);
        }
        let rows_changed = render_frame::publish_rows(&mut self.draw_list, &mut staged.draw_list);
        if geometry_swapped || rows_changed {
            let revs = &mut self.revisions;
            revs.advance_drawlist_stamp();
        }
        self.emitted = staged.emitted;
        self.dropped = staged.dropped;
    }

    /// Resolves the lighting each draw requested. Draws that could not be lit are removed
    /// (every draw of the same object) and tallied once per object.
    pub fn finalize_illum(&mut self, table: &render_scene::ResolvedModelIllumTable) {
        let mut unlit_objects: Vec<_> = Vec::new();
        let mut touched = false;
        for draw in &mut self.draw_list {
            let Some(request) = Option::take(&mut draw.queued_illum) else { continue };
            match table.answer_for(request.owner) {
                Some(ResolvedModelIllum::Seated {
                    handle: new_handle,
                    stage_light_index: new_light,
                    env_probe_index: new_probe,
                    compact_illum: new_packed,
                }) => {
                    let unchanged = draw.illum_handle == new_handle
                        && draw.stage_light_index == new_light
                        && draw.env_probe_index == new_probe
                        && draw.compact_illum == new_packed;
                    if unchanged {
                        continue;
                    }
                    draw.illum_handle = new_handle;
                    draw.stage_light_index = new_light;
                    draw.env_probe_index = new_probe;
                    draw.compact_illum = new_packed;
                    touched = true;
                }
                Some(ResolvedModelIllum::Failed) | None => unlit_objects.push(draw.object_tag),
            }
        }
        if !unlit_objects.is_empty() {
            unlit_objects.sort_unstable();
            unlit_objects.dedup();
            let removed = u32::try_from(unlit_objects.len()).unwrap_or(u32::MAX);
            self.dropped.add_to(ModelDrop::NoLighting, removed);
            self.draw_list
                .retain(|draw| unlit_objects.binary_search(&draw.object_tag).is_err());
            let revs = &mut self.revisions;
            revs.advance_acceptance_stamp();
            touched = true;
        }
        if touched {
            let revs = &mut self.revisions;
            revs.advance_drawlist_stamp();
        }
    }
}
