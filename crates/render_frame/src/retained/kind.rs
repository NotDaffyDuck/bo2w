//! What a retained draw is, with the data each sort of draw needs.

use fx_t6::runtime::GfxParticleCloud;
use glam::Mat4;
use lighting_t6::SmodelSurfPath;

use super::lane::BspCameraLane;
use crate::frontend::SmodelPretessRange;

/// One of the kinds of thing the frame can draw. Each variant carries only what
/// the packing stage needs to turn it into a source-list entry.
#[derive(Clone, Copy, Debug)]
pub enum RetainedDrawKind {
    /// Map geometry. A multi-surface run lists `run` surfaces from `run_off` in
    /// the producer's surface table.
    World {
        /// The lead surface of the run, as an index into the surface table.
        surf: u16,
        /// How many surfaces the run holds; 1 for a lone surface.
        run: u16,
        /// Where the run starts in the producer's run table.
        run_off: u32,
        /// The camera list this surface was sorted into, if it came from the map.
        bsp_kind: Option<BspCameraLane>,
        /// The first surface of the run in its sorted list.
        bsp_run_first: Option<u16>,
        /// Placement of the geometry; the identity for map surfaces.
        world_from_local: Mat4,
    },
    /// Static model.
    Smodel {
        /// Which surface of the model to draw.
        surface: u32,
        /// The material asset's index.
        material: u32,
        /// Where the instance sits in the world.
        world_from_local: Mat4,
        /// Handle to the lighting data this instance reads.
        lighting_handle: u32,
        /// Baked per-instance lighting, when the model has some.
        packed_lighting: Option<[u8; 4]>,
        /// The instance's index in the level's placement list.
        placement: u32,
        /// How the surface reaches the GPU, which decides the list it joins.
        stream: Option<SmodelSurfPath>,
        /// Slot in the cached-geometry pool, for the cached path.
        cache_index: Option<u16>,
        /// Range of the shared index pool, for the pretessellated path.
        pretess: Option<SmodelPretessRange>,
    },
    /// Dynamic model.
    XModel {
        /// Which surface of the model to draw.
        surface: u32,
        /// The material asset's index.
        material: u32,
        /// Identifies the object across frames; the viewmodel has its own value.
        object_id: u16,
        /// Where the object sits in the world.
        world_from_local: Mat4,
        /// Handle to the lighting data this object reads.
        lighting_handle: u32,
        /// Baked lighting for this object, when there is some.
        packed_lighting: Option<[u8; 4]>,
        /// The surface is a scope lens, which is drawn separately.
        is_scope: bool,
        /// The scene entity that owns the object, if it has one.
        scene_entnum: Option<u32>,
    },
    /// A mesh built on the CPU for this frame, with up to two argument vectors.
    CodeMesh {
        /// Drawn with the viewmodel's projection instead of the camera's.
        viewmodel: bool,
        /// Index of the mesh in this frame's mesh pool.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// How many of `args` are meaningful.
        arg_count: u8,
        /// Constants handed to the material.
        args: [[f32; 4]; 2],
    },
    /// Billboarded particles, three clouds per draw.
    ParticleCloud {
        /// Index of the draw in this frame's effect output.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// The clouds' placement and size data.
        clouds: [GfxParticleCloud; 3],
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
        lighting_handle: u32,
    },
    /// Breakable glass.
    Glass {
        /// Index of the draw in this frame's glass output.
        draw: u32,
        /// The material asset's index.
        material: u32,
        /// Handle to the lighting data the pane reads.
        lighting_handle: u32,
    },
}
