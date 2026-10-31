//! What a retained draw is, with the data each sort of draw needs.

use fx_t6::runtime::VisualParticleCloud;
use glam::Mat4;
use lighting_t6::PropFacetPath;

use super::lane::LevelViewLane;
use crate::src_lists::PropPrebuiltRange;

/// One of the kinds of thing the frame can draw. Each variant carries only what
/// the packing stage needs to turn it into a source-list entry.
#[derive(Clone, Copy, Debug)]
pub enum HeldDrawKind {
    /// Map geometry. A multi-surface run lists `run` surfaces from `run_start_offset` in
    /// the producer's surface table.
    World {
        /// The lead surface of the run, as an index into the surface table.
        surf: u16,
        /// How many surfaces the run holds; 1 for a lone surface.
        run: u16,
        /// Where the run starts in the producer's run table.
        run_start_offset: u32,
        /// The camera list this surface was sorted into, if it came from the map.
        world_tree_camera_kind: Option<LevelViewLane>,
        /// The first surface of the run in its sorted list.
        level_tree_run_first: Option<u16>,
        /// Placement of the geometry; the identity for map surfaces.
        model_to_world: Mat4,
    },
    /// Static model.
    Smodel {
        /// Which surface of the model to draw.
        surface: u32,
        /// The material asset's index.
        material: u32,
        /// Where the instance sits in the world.
        model_to_world: Mat4,
        /// Handle to the lighting data this instance reads.
        illum_handle: u32,
        /// Baked per-instance lighting, when the model has some.
        compact_illum: Option<[u8; 4]>,
        /// The instance's index in the level's placement list.
        placement: u32,
        /// How the surface reaches the GPU, which decides the list it joins.
        stream: Option<PropFacetPath>,
        /// Slot in the cached-geometry pool, for the cached path.
        cache_slot_no: Option<u16>,
        /// Range of the shared index pool, for the pretessellated path.
        pretess: Option<PropPrebuiltRange>,
    },
    /// Dynamic model.
    XModel {
        /// Which surface of the model to draw.
        surface: u32,
        /// The material asset's index.
        material: u32,
        /// Identifies the object across frames; the viewmodel has its own value.
        object_tag: u16,
        /// Where the object sits in the world.
        model_to_world: Mat4,
        /// Handle to the lighting data this object reads.
        illum_handle: u32,
        /// Baked lighting for this object, when there is some.
        compact_illum: Option<[u8; 4]>,
        /// The surface is a scope lens, which is drawn separately.
        is_scoped_view: bool,
        /// The scene entity that owns the object, if it has one.
        stage_actor_num: Option<u32>,
    },
    /// A mesh built on the CPU for this frame, with up to two argument vectors.
    CodeMesh {
        /// Drawn with the viewmodel's projection instead of the camera's.
        gun_view: bool,
        /// Index of the mesh in this frame's mesh pool.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// How many of `args` are meaningful.
        n_args: u8,
        /// Constants handed to the material.
        arg_rows: [[f32; 4]; 2],
    },
    /// Billboarded particles, three clouds per draw.
    SparkleSwarm {
        /// Index of the draw in this frame's effect output.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// The clouds' placement and size data.
        clouds: [VisualParticleCloud; 3],
    },
    /// Impact marks laid over world or model surfaces.
    MarkMesh {
        /// Glass decals are drawn in the late scene pass, with their receiver.
        glass: bool,
        /// Index of the draw in this frame's mark output.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// The vertices are in the packed format rather than the plain one.
        packed: bool,
        /// Handle to the lighting data the mark reads.
        illum_handle: u32,
    },
    /// Breakable glass.
    Glass {
        /// Index of the draw in this frame's glass output.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// Handle to the lighting data the pane reads.
        illum_handle: u32,
    },
}
