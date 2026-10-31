//! What a T6 zone walk keeps. Assets whose header loads into TEMP vanish
//! when the asset finishes, so the sink copies what later stages need from
//! inside `asset_loaded`, while the header is still in place.
//!
//! References between assets: a pointer to another asset is either loaded
//! where it stands (the walk fixes the slot up to the body, and reports the
//! slot) or names earlier data: for TEMP assets an alias slot in VIRTUAL, for
//! the rest the body itself. The capture keeps every reported slot, alias
//! and body, so a later structure's asset pointer resolves to "material 12",
//! not to an address that TEMP has since reused.

use std::collections::{BTreeMap, HashMap};

use fastfile_t6::layout::{
    AddonMapEnts, BrushWrapper, ClipInfo, ClipSurfaceKind, CollBoxTree, CollTriGroup,
    Collmap, ComPrimaryLight, ComWorld, BonePoseMat, BreakableDef, DestructiblePiece,
    DestructibleStage, DpvsPlane, DynEntityDef, FontIcon, FontIconAlias, FontIconEntry,
    FontIconName, GameWorldMp, GfxAabbTree, GfxBrushModel, GfxCell, GfxCompressedLightGridCoeffs,
    GfxCompressedLightGridColors, GfxExposureVolume, GfxHeroLight, GfxHeroLightTree, GfxImage,
    GfxImageLoadDef, GfxLight, GfxLightGrid, GfxLightmapArray, GfxLutVolume, GfxPackedPlacement,
    GfxPackedVertex, GfxPixelShaderLoadDef, GfxPortal, GfxReflectionProbe, GfxSkyDynamicIntensity,
    GfxSkyGridVolume, GfxStateBits, GfxStaticModelDrawInst, GfxStaticModelLmapVertexInfo,
    GfxStreamedPartInfo, GfxDrawSurfRec, GfxVolumePlane, GfxWorld, GfxWorldDpvsPlanes,
    GfxWorldDpvsStatic, GfxWorldDraw, GfxWorldFog, GfxWorldFogModifierVolume, GfxWorldFogVolume,
    GfxWorldSun, GfxWorldVertexData0, GfxWorldVertexData1, Glass, GlassDef, Glasses, LocalizeEntry,
    MapEnts, Material, MaterialConstantDef, MaterialInfo, MaterialStage, MaterialPixelShader,
    MaterialPixelShaderProgram, MaterialShaderArgument, MaterialTechnique, MaterialTechniqueSet,
    MaterialTextureDef, PathData, PhysConstraint, PhysConstraints, PhysGeomInfo, PhysGeomList,
    PhysPreset, RawFile, ScriptParseTree, StringTable, StringTableCell, SunLightParseParams,
    VehicleDef, VehicleEngine, VehicleEngineSound, VehicleParameter, XAnimDeltaPart,
    XAnimDeltaPartQuat, XAnimDeltaPartQuatDataFrames, XAnimNotifyInfo, XAnimPartTrans,
    XAnimPartTransData, XAnimPartTransFrames, XAnimParts, XBoneInfo, XModel, XModelCollSurf_s,
    XModelCollTri_s, XModelLodInfo, XModelQuat, XRigidVertList, XSurface, XSurfaceVertexInfo,
    ZBarrierBoard, ZBarrierDef, cLeafBrushNodeChildren_t, cLeafBrushNodeLeaf_t, cLeafBrushNode_s,
    cLeaf_s, cNode_t, cStaticModel_s, cbrush_t, cbrushside_t, clipMap_t, cmodel_t, cplane_s,
    pathlink_s, pathnode_constant_t, pathnode_t, srfTriangles_t,
};
use fastfile_t6::{XAssetKind, Loaded, RawRef, Result, WalkSink, StreamRef, ZoneByteReader};

/// An asset this capture kept: its type and index into that type's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetIdentity {
    pub ty: XAssetKind,
    pub index: usize,
}

/// One GfxImage: where its pixels come from.
#[derive(Clone, Debug)]
pub struct ImageRef {
    pub name: String,
    /// `GfxImage::hash`, the pack index's name hash.
    pub label_digest: u32,
    pub map_kind: u8,
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub tier_total: u8,
    /// Streamed parts; on PC 0 or 1.
    pub streamed_parts: u8,
    /// The pack index's data hash of streamed part 0.
    pub part_hash: u32,
    /// Pixels carried in the zone itself (`GfxImageLoadDef`), with its DXGI
    /// format, when it has any.
    pub embedded: Option<EmbeddedImage>,
}

#[derive(Clone, Debug)]
pub struct EmbeddedImage {
    pub dxgi_format: u32,
    pub tier_total: u8,
    pub flags: u8,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ShaderSamplerImage {
    pub label_digest: u32,
    pub semantic: u8,
    pub texture_sampling_mode: u8,
    pub image: Option<AssetIdentity>,
}

#[derive(Clone, Debug)]
pub struct MaterialRef {
    pub name: String,
    pub sort_rank: u8,
    /// `MaterialInfo::gameFlags` (0x4: takes no marks).
    pub game_flags: u32,
    /// `MaterialInfo::surfaceTypeBits`: one bit per surface type.
    pub facet_type_bits: u32,
    pub surf_bits: u32,
    pub contents: u32,
    pub method_set: Option<AssetIdentity>,
    pub textures: Vec<ShaderSamplerImage>,
    /// `constantTable`: (name hash, name, literal) per constant.
    pub constants: Vec<(u32, String, [f32; 4])>,
    /// `stateBitsEntry`: per technique slot, its index into `state_bits`
    /// (0xff = the slot draws nothing).
    pub state_bits_entry: [u8; 36],
    /// `stateBitsTable`: each entry's `loadBits` (blend, alpha test, cull,
    /// colour write; depth write and test, polygon offset, stencil).
    pub state_words: Vec<[u32; 2]>,
    pub condition_bits: u8,
    pub camera_zone: u8,
    /// bo2mp: `thermalMaterial`, the material drawn in its place in the
    /// Dual Band's thermal view (None: keep this one).
    pub thermal_material: Option<AssetIdentity>,
}

/// One material draw state (`GfxStateBits::loadBits`), decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrawState {
    /// GFXS_BLEND_*: 0 disabled, 1 zero, 2 one, 3 src colour, 4 stow_kind src
    /// colour, 5 src alpha, 6 stow_kind src alpha, 7 dst alpha, 8 stow_kind dst alpha,
    /// 9 dst colour, 10 stow_kind dst colour.
    pub src_blend: u8,
    pub dst_blend: u8,
    /// GFXS_BLENDOP_*: 0 disabled, 1 add, 2 subtract, 3 rev subtract,
    /// 4 min, 5 max.
    pub blend_op: u8,
    /// `None` when alpha testing is off; else GFXS_ALPHA_TEST_* (0 = above
    /// 0, 1 = at least 128/255).
    pub alpha_test: Option<u8>,
    /// GFXS_CULL_*: 1 none, 2 back, 3 front.
    pub cull: u8,
    pub color_write_rgb: bool,
    pub color_write_alpha: bool,
    pub writes_depth: bool,
    /// `None` when the depth test is off; else GFXS_DEPTHTEST_* (0 always,
    /// 1 less, 2 equal, 3 less-equal).
    pub depth_test: Option<u8>,
    /// GFXS_POLYGON_OFFSET_*: 0 none, 1, 2, 3 = shadow map.
    pub polygon_offset: u8,
}

impl DrawState {
    /// Decode `loadBits` (bit positions per the T6 `GfxStateBitsLoadBits`
    /// layout).
    pub fn decode(bits: [u32; 2]) -> Self {
        let [a, b] = bits;
        let f = |v: u32, at: u32, n: u32| ((v >> at) & ((1 << n) - 1)) as u8;
        Self {
            src_blend: f(a, 0, 4),
            dst_blend: f(a, 4, 4),
            blend_op: f(a, 8, 3),
            alpha_test: (f(a, 11, 1) == 0).then(|| f(a, 12, 1)),
            cull: f(a, 14, 2),
            color_write_rgb: f(a, 27, 1) != 0,
            color_write_alpha: f(a, 28, 1) != 0,
            writes_depth: f(b, 0, 1) != 0,
            depth_test: (f(b, 1, 1) == 0).then(|| f(b, 2, 2)),
            polygon_offset: f(b, 4, 2),
        }
    }
}

impl MaterialRef {
    /// The draw state of technique slot `tech` (`MaterialTechniqueType`),
    /// when the material draws in it.
    pub fn draw_state(&self, tech: usize) -> Option<DrawState> {
        let entry = *self.state_bits_entry.get(tech)?;
        self.state_words
            .get(usize::from(entry))
            .map(|&bits| DrawState::decode(bits))
    }
}

/// One animation (`XAnimParts`): its frames, rate and, for a delta
/// animation, the root's translation keys (frame, xyz).
#[derive(Clone, Debug)]
pub struct XAnimRef {
    pub name: String,
    pub frame_total: u16,
    pub frame_rate_hz: f32,
    pub looping: bool,
    pub delta: bool,
    pub delta_trans: Vec<(u16, [f32; 3])>,
    /// `loopEntryTime`: where a looping animation starts again after its first lap (0..1).
    pub loop_entry_time: f32,
    /// `primedLength`.
    pub primed_length: f32,
    /// `assetType`: 0 a whole pose, 1 additive (laid over another anim).
    pub asset_type: u8,
    /// bo2zm: the root's turn keys (frame, quaternion x y z w): the delta
    /// part's `quat` (a whole turn) or else its `quat2` (about the up axis
    /// only, z and w). Empty when the animation does not turn its root.
    pub delta_quat: Vec<(u16, [f32; 4])>,
    /// bo2zm M2: the whole animation, as the engine's decoder reads it.
    pub joint_total: [u8; 10],
    /// Track (bone) names, `boneCount[PART_TYPE_ALL]` of them.
    pub names: Vec<String>,
    /// Notetracks: (name, time 0..1).
    pub notifies: Vec<(String, f32)>,
    pub payload_u8: Vec<u8>,
    pub payload_i16: Vec<u16>,
    pub payload_i32: Vec<u32>,
    pub noise_octet: Vec<u8>,
    pub noise_word16: Vec<u16>,
    pub noise_word32: Vec<u32>,
    /// Key frame indices (bytes when `numframes < 256`, widened).
    pub indices: Vec<u16>,
}

/// One primary light (`ComPrimaryLight`): the map's sun, omni and spot
/// lights.
#[derive(Clone, Debug)]
pub struct PrimaryLightRef {
    /// GFX_LIGHT_TYPE_*: 1 sun, 2 spot, 3 square spot, 4 round spot, 5 omni (0 none).
    pub ty: u8,
    pub supports_depth_shade: u8,
    pub falloff_pow: u8,
    pub priority: u8,
    pub hide_dist: i16,
    pub color: [f32; 3],
    pub heading: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub cos_half_fov_outer: f32,
    pub cos_half_fov_inner: f32,
    pub d_attenuation: f32,
    pub roundness: f32,
    pub diffuse_color: [f32; 4],
    pub falloff: [f32; 4],
    pub angle: [f32; 4],
    pub pair_ab_vs_b: [f32; 4],
}

/// One hero light (`GfxHeroLight`), as the map stores it.
#[derive(Clone, Debug)]
pub struct HeroLightRef {
    pub ty: u8,
    pub tint_rgb: [f32; 3],
    pub heading: [f32; 3],
    pub place_at: [f32; 3],
    pub reach: f32,
    pub cos_half_fov_outer: f32,
    pub cos_half_fov_inner: f32,
    pub exponent: i32,
}

/// One node of the hero light tree (`GfxHeroLightTree`): its box and the
/// indices of its two child nodes, as the map stores them.
#[derive(Clone, Debug)]
pub struct HeroLightTreeRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub left_node: i32,
    pub right_node: i32,
}

/// The world's static visibility lists (`GfxWorldDpvsStatic`): surface
/// ranges in draw order (indices into `sorted_facet_index`).
#[derive(Clone, Debug, Default)]
pub struct DpvsRanges {
    pub static_facet_tally: u32,
    pub lit: (u32, u32),
    pub lit_see_through: (u32, u32),
    pub emissive_opaque: (u32, u32),
    pub emissive_trans: (u32, u32),
}

/// One `GfxCell`: its bounds and the portals out of it.
#[derive(Clone, Debug, Default)]
pub struct CellRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub portals: Vec<PortalRef>,
}

/// One `GfxPortal`.
#[derive(Clone, Debug, Default)]
pub struct PortalRef {
    /// `plane.coeffs`: (normal, w) with n·p + w = 0 on the portal.
    pub plane: [f32; 4],
    /// The cell on the far side (`cell` points into `GfxWorld::cells`).
    pub cell: u16,
    pub vertices: Vec<[f32; 3]>,
    /// `hullAxis`: the 2D axes the walk projects the portal onto.
    pub hull_axes_dir: [[f32; 3]; 2],
}

/// One cell AABB tree node (`GfxAabbTree`): the surfaces (a run of
/// `sorted_facet_index`) and static models it draws.
#[derive(Clone, Debug)]
pub struct AabbTreeRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub sub_element_total: u16,
    pub facet_tally: u16,
    pub start_surf_index: u16,
    pub prop_indexes: Vec<u16>,
    pub sub_elements_start: i32,
}

#[derive(Clone, Debug)]
pub struct TechniqueSetRef {
    pub name: String,
    pub level_vertex_format: u8,
    /// The 36 technique slots (`MaterialTechniqueSet::techniques`).
    pub techniques: Vec<Option<TechniqueRef>>,
}

/// One technique: its passes.
#[derive(Clone, Debug)]
pub struct TechniqueRef {
    pub name: String,
    pub flags: u16,
    pub passes: Vec<PassRef>,
}

/// One pass: its shaders by name, the pixel shader's D3D11 bytecode (kept
/// only when the capture asks for shaders), and its arguments.
#[derive(Clone, Debug)]
pub struct PassRef {
    pub vertex_shader: String,
    pub pixel_shader: String,
    pub pixel_program: Vec<u8>,
    pub vertex_program: Vec<u8>,
    pub args: Vec<ShaderArgRef>,
}

/// One `MaterialShaderArgument`: what feeds a register.
#[derive(Clone, Copy, Debug)]
pub struct ShaderArgRef {
    pub ty: u16,
    pub location: u16,
    pub size: u16,
    pub buffer: u16,
    /// The union: a name hash, code constant or code sampler index (or a
    /// literal's address, meaningless here).
    pub u: u32,
}

#[derive(Clone, Debug)]
pub struct XModelRef {
    pub name: String,
    pub numsurfs: u8,
    pub materials: Vec<Option<AssetIdentity>>,
    pub skeleton_joint_total: u8,
    pub detail_tier_total: u16,
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub radius: f32,
    /// Where the model's light is read, from its origin
    /// (`lightingOriginOffset`), and how far that point may move before
    /// it is read again (`lightingOriginRange`).
    pub lighting_origin_offset: [f32; 3],
    pub lighting_origin_range: f32,
    /// The first (most detailed) LOD's surfaces.
    pub lod0: Vec<XSurfaceRef>,
    /// bo2zm M2: the skeleton. Bone names, the root bone count, each child
    /// bone's parent step, its local rotation (`XModelQuat`) and
    /// translation, every bone's bind matrix (`baseMat`: quat xyzw +
    /// shift_offset), and the hit-location class per bone.
    pub joint_labels: Vec<String>,
    pub top_joint_total: u8,
    pub ancestor_chain: Vec<u8>,
    pub rotation_quaternions: Vec<[i16; 4]>,
    pub shift_offset: Vec<[f32; 3]>,
    pub foundation_surface: Vec<([f32; 4], [f32; 3])>,
    pub body_part_category: Vec<u8>,
    /// bo2zm M3: each bone's hit box (`XBoneInfo`: bounds mins, maxs in the
    /// bone's space, radius squared; none when the bone has no box).
    pub bone_info: Vec<Option<([f32; 3], [f32; 3], f32)>>,
    /// Per LOD: (distance, surface count, first surface).
    pub detail_tiers: Vec<(f32, u16, u16)>,
    pub contents: u32,
    /// bo2zm fix list 2: the collision LOD (`collLod`, negative = none) and
    /// the collision surfaces (`collSurfs`) bullets and thrown things hit.
    pub collision_detail_tier: i16,
    pub collision_faces: Vec<XModelCollSurfRef>,
    /// codmp: the brushes of the model's collision maps (`collmaps`, each a
    /// `PhysGeomList` of brush geoms), in model space. A car's are what
    /// stops a player walking into it (player clip, not bullets).
    pub collmap_brushes: Vec<ClipBrushRef>,
    /// bo2mp: how the model flies when it comes loose (`physPreset`): a
    /// destructible's broken-off piece without one stays put.
    pub physics_template: Option<AssetIdentity>,
    /// bo2mp: the first physics collmap's pieces (`collmaps[0].geomList`),
    /// what a script model's movement traces hit (DObjGetCollmap), and the
    /// contents of every collmap (XModelGetContents adds them to
    /// `contents`).
    pub collmap: Vec<PhysGeomRef>,
    pub collmap_contents: u32,
    /// bo2mp: the corners of the first collmap piece when it is a brush
    /// (its `vertex_points`, or its box's 8 corners when it has no extra sides),
    /// what a vehicle that moves by its collmap (`useCollmap`) sweeps.
    pub collmap_brush_verts: Vec<[f32; 3]>,
}

/// bo2mp: one piece of a physics collmap (`PhysGeomInfo`), in model space:
/// its type (1 box, 2 brush, 3 cylinder, 5 capsule), its axes
/// (`orientation` rows), middle (`offset`) and half-size (`halfLengths`),
/// and for a brush its bounds and side count (`BrushWrapper`).
#[derive(Clone, Copy, Debug)]
pub struct PhysGeomRef {
    pub ty: i32,
    pub orientation: [[f32; 3]; 3],
    pub offset: [f32; 3],
    pub half_lengths: [f32; 3],
    pub brush: Option<([f32; 3], [f32; 3], u32)>,
}

/// One model collision surface (`XModelCollSurf_s`): its triangles (each a
/// plane + the two edge vectors of `XModelCollTri_s`), bounds in model
/// space, bone, contents and surface flags.
#[derive(Clone, Debug, Default)]
pub struct XModelCollSurfRef {
    pub triangle_list: Vec<[[f32; 4]; 3]>,
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub joint_position: i32,
    pub contents: u32,
    pub face_property_bits: u32,
}

/// One placed static model of the clip map (`cStaticModel_s`).
#[derive(Clone, Copy, Debug)]
pub struct ClipStaticModelRef {
    pub model: Option<AssetIdentity>,
    pub contents: u32,
    pub origin: [f32; 3],
    pub reverse_scaled_basis: [[f32; 3]; 3],
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
}

/// One model surface (`XSurface`) in model space.
#[derive(Clone, Debug, Default)]
pub struct XSurfaceRef {
    /// `GfxPackedVertex`, 32 bytes each: xyz f32x3 @0, binormal sign @12,
    /// colour @16, uv_semantic half2 @20, normal @24 and tangent @28 packed
    /// like the world's.
    pub vertex_points: Vec<u8>,
    pub vertex_tally: u16,
    pub indices: Vec<u16>,
    /// Rigid vertex lists: (bone offset, vert count, tri offset, tri count).
    pub vert_lists: Vec<(u16, u16, u16, u16)>,
    pub flags: u8,
    pub material: Option<AssetIdentity>,
    /// bo2zm M2: weighted vertices (`XSurfaceVertexInfo`): how many
    /// vertices have 1..4 influences, and their packed blend words.
    pub blend_counts: [i16; 4],
    pub blend: Vec<u16>,
    pub piece_mask: [u32; 5],
}

/// One world surface (`GfxSurface`).
#[derive(Clone, Debug)]
pub struct WorldSurface {
    /// `srfTriangles_t` bounds (zero on many retail surfaces).
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    /// `GfxSurface::bounds`.
    pub bounds: [[f32; 3]; 2],
    pub vertex_data_offset0: i32,
    pub vertex_data_offset1: i32,
    pub first_vertex: i32,
    pub vertex_count: u16,
    pub tri_tally: u16,
    pub base_index: i32,
    pub material: Option<AssetIdentity>,
    pub lightbake_index: i8,
    pub env_probe_index: i8,
    pub key_light_index: i8,
    pub flags: u8,
}

/// One static model placement (`GfxStaticModelDrawInst`).
#[derive(Clone, Debug)]
pub struct StaticModel {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub scale: f32,
    pub model: Option<AssetIdentity>,
    pub flags: i32,
    /// Baked per-vertex light for its first LOD
    /// (`lmapVertexInfo[0].lmapVertexColors`), one u32 per vertex; empty
    /// when the instance has none.
    pub lmap_colors: Vec<u32>,
    /// `lightingSH`, quantized (V0, V1, V2: four u16 each).
    pub lighting_sh: [u16; 12],
    /// `colorsIndex`: the light grid colour set the compiler picked for
    /// this placement (its light when it has no baked vertex light).
    pub vertex_color_slot: u16,
    pub key_light_index: u8,
    /// `visibility`: how much of its primary light reaches this placement,
    /// as the map's compiler measured it (0..255).
    pub visibility: u8,
    pub env_probe_index: u8,
    /// `cullDist`: beyond this distance the instance is not drawn.
    pub hide_dist: f32,
}

/// The map's GfxWorld, in its own terms.
#[derive(Clone, Debug, Default)]
pub struct WorldRef {
    pub name: String,
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub vertex_count: u32,
    pub vd0: Vec<u8>,
    pub vd1: Vec<u8>,
    pub indices: Vec<u16>,
    pub surfaces: Vec<WorldSurface>,
    pub prop_set: Vec<StaticModel>,
    pub light_atlases: Vec<[Option<AssetIdentity>; 2]>,
    /// `GfxWorldDraw::reflectionProbes`: the cube maps surfaces reflect
    /// (`WorldSurface::env_probe_index` picks one).
    pub probe_list: Vec<ReflectionProbeRef>,
    pub sky_box_model: String,
    /// `GfxWorld::sunLight`: the sun the lit world shaders add to the
    /// lightmap (`sunDiffuse` times its baked visibility).
    pub sun: Option<SunRef>,
    /// `GfxWorld::exposureVolumes`: (exposure, increase scale, decrease
    /// scale, feather range) per volume.
    pub exposure_volumes: Vec<[f32; 4]>,
    /// bo2mp: `GfxWorld::sunParse.initWorldSun.exposure`, the map's own
    /// exposure where no exposure volume holds the eye.
    pub init_sun_exposure: f32,
    /// Per exposure volume its `control` word (which planes bound it).
    pub exposure_volume_controls: Vec<u32>,
    /// `GfxWorld::exposureVolumePlanes`: the volumes' bounding planes.
    pub exposure_volume_planes: Vec<[f32; 4]>,
    /// `GfxWorld::lightGrid`, the static-model lighting source.
    pub probe_grid: Option<LightGridRef>,
    /// `GfxWorld::lutMaterial`: the map's colour grading material.
    pub lut_material: Option<AssetIdentity>,
    /// `GfxWorld::lutVolumes`: per volume its box, control word, transition
    /// time and the band of the LUT image it picks.
    pub lut_volumes: Vec<([f32; 3], [f32; 3], u32, u32, u32)>,
    /// `GfxWorld::heroLights`: the map's hero lights.
    pub hero_lights: Vec<HeroLightRef>,
    /// `GfxWorld::heroLightTree`: the box tree over `hero_lights`.
    pub hero_light_tree: Vec<HeroLightTreeRef>,
    /// `GfxWorld::skyDynIntensity`: angle0, angle1, factor0, factor1.
    pub sky_dyn_intensity: [f32; 4],
    /// The static visibility lists.
    pub dpvs: DpvsRanges,
    /// `sortedSurfIndex`: draw order -> surface.
    pub sorted_facet_index: Vec<u16>,
    /// bo2zm M3: the brush models (`GfxBrushModel`, submodel n = entry n:
    /// doors, debris): their first surface and surface count in `surfaces`,
    /// and their bounds.
    pub solid_models: Vec<(u32, u32, [[f32; 3]; 2])>,
    /// Per cell, its AABB trees.
    pub cells: Vec<Vec<AabbTreeRef>>,
    /// `GfxWorld::dpvsPlanes.planes`: the cell tree's split planes as
    /// (normal, dist), `planeCount` of them.
    pub dpvs_planes: Vec<[f32; 4]>,
    /// `GfxWorld::dpvsPlanes.nodes`: the cell tree, `nodeCount` u16s.
    pub dpvs_nodes: Vec<u16>,
    /// Per cell, its bounds and portals (same order as `cells`).
    pub cell_bounds: Vec<CellRef>,
    /// `GfxWorld::primaryLightCount`.
    pub primary_light_count: u32,
    /// `GfxWorld::sunParse.initWorldFog`: the fog the map starts with.
    pub init_fog: Option<WorldFogRef>,
    /// `GfxWorld::sunParse.initWorldSun` as raw words (84 bytes; layout
    /// still being measured: it holds the map's starting sun and exposure).
    pub init_sun_raw: Vec<u32>,
    /// `GfxWorld::worldFogVolumes`: per volume its box, control words and
    /// fog.
    pub fog_volumes: Vec<([f32; 3], [f32; 3], u32, u32, WorldFogRef)>,
    /// `GfxWorld::worldFogVolumePlanes`: the fog volumes' bounding planes
    /// (their boxes are zero; the control word picks the planes, as for
    /// exposure volumes).
    pub fog_volume_planes: Vec<[f32; 4]>,
    /// `GfxWorld::worldFogModifierVolumes`: per volume its control word (as
    /// a fog volume's: banks, plane count, first plane), its transition
    /// time, depth scale and height scale, and its colour adjust.
    pub fog_modifier_volumes: Vec<(u32, [f32; 3], [f32; 4])>,
    /// `GfxWorld::worldFogModifierVolumePlanes`.
    pub fog_modifier_volume_planes: Vec<[f32; 4]>,
}

/// One `GfxWorldFog`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldFogRef {
    pub base_dist: f32,
    pub half_dist: f32,
    pub floor_elevation: f32,
    pub half_height: f32,
    pub sun_fog_pitch: f32,
    pub sun_fog_yaw: f32,
    pub sun_fog_inner: f32,
    pub sun_fog_outer: f32,
    pub fog_color: [f32; 3],
    pub fog_opacity: f32,
    pub sun_fog_color: [f32; 3],
    pub sun_fog_opacity: f32,
}

fn read_world_fog(s: &ZoneByteReader<'_>, f: RawRef) -> Result<WorldFogRef> {
    Ok(WorldFogRef {
        base_dist: s.float_from_bytes(f, GfxWorldFog::baseDist)?,
        half_dist: s.float_from_bytes(f, GfxWorldFog::halfDist)?,
        floor_elevation: s.float_from_bytes(f, GfxWorldFog::baseHeight)?,
        half_height: s.float_from_bytes(f, GfxWorldFog::halfHeight)?,
        sun_fog_pitch: s.float_from_bytes(f, GfxWorldFog::sunFogPitch)?,
        sun_fog_yaw: s.float_from_bytes(f, GfxWorldFog::sunFogYaw)?,
        sun_fog_inner: s.float_from_bytes(f, GfxWorldFog::sunFogInner)?,
        sun_fog_outer: s.float_from_bytes(f, GfxWorldFog::sunFogOuter)?,
        fog_color: vec3(s, f, GfxWorldFog::fogColor)?,
        fog_opacity: s.float_from_bytes(f, GfxWorldFog::fogOpacity)?,
        sun_fog_color: vec3(s, f, GfxWorldFog::sunFogColor)?,
        sun_fog_opacity: s.float_from_bytes(f, GfxWorldFog::sunFogOpacity)?,
    })
}

/// The world's light grid (`GfxLightGrid`): grid-space bounds, row axes,
/// the compressed rows, entries (4 bytes each) and colour sets (56
/// directions of RGB8, 168 bytes each).
#[derive(Clone, Debug, Default)]
pub struct LightGridRef {
    pub sunlight_lamp_slot: u32,
    pub lo_corner: [u16; 3],
    pub hi_corner: [u16; 3],
    pub offset: f32,
    pub line_direction: u32,
    pub column_direction: u32,
    pub line_bytes_origin: Vec<u8>,
    pub unparsed_line_bytes: Vec<u8>,
    pub entries: Vec<u8>,
    pub colors: Vec<u8>,
    pub tint_key_total: u32,
    /// `coeffs`: per set, 27 u16 (`GfxCompressedLightGridCoeffs`, 54 bytes);
    /// a grid with SH coefficients has no colour sets.
    pub coeffs: Vec<u8>,
    pub coeff_count: u32,
    /// `skyGridVolumes`: the light for places outside the grid (the
    /// distant terrain): bounds, the point the light was taken at, its
    /// coefficient set, primary light and that light's visibility.
    pub sky_volumes: Vec<SkyGridVolumeRef>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SkyGridVolumeRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub illum_origin: [f32; 3],
    pub vertex_color_slot: u16,
    pub key_light_index: u8,
    pub visibility: u8,
}

/// One reflection probe (`GfxReflectionProbe`): where it was captured, the
/// lighting there as BO2's shaders evaluate it per normal (`lightingSH`, three
/// vec4: colour scale + z^2 weight, linear + constant, quadratic terms), and
/// its cube map.
#[derive(Clone, Debug)]
pub struct ReflectionProbeRef {
    pub origin: [f32; 3],
    pub lighting_sh: [[f32; 4]; 3],
    pub image: Option<AssetIdentity>,
    pub mip_lod_bias: f32,
}

/// The world's sun light (`GfxLight`).
#[derive(Clone, Copy, Debug)]
pub struct SunRef {
    pub color: [f32; 3],
    pub heading: [f32; 3],
    pub diffuse_color: [f32; 4],
}

/// One collision brush (`cbrush_t`): its box, contents, axial side flags
/// ([min x, y, z], [max x, y, z]) and the extra side planes.
#[derive(Clone, Debug)]
pub struct ClipBrushRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub contents: i32,
    pub axial_cflags: [[i32; 3]; 2],
    pub axial_sflags: [[i32; 3]; 2],
    /// (normal, dist, surface flags) per non-axial side.
    pub sides: Vec<([f32; 3], f32, i32)>,
    /// bo2mp: each non-axial side's contents flags (`cbrushside_t::cflags`),
    /// in the order of `sides`.
    pub side_cflags: Vec<i32>,
    /// The brush's corner points (BO2's collision sweeps against these).
    pub vertex_points: Vec<[f32; 3]>,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipAabbRef {
    pub origin: [f32; 3],
    pub half_extent: [f32; 3],
    pub material_slot: u16,
    pub sub_element_total: u16,
    pub u: i32,
}

#[derive(Clone, Debug)]
pub struct ClipMaterialRef {
    pub name: String,
    pub surface_flags: i32,
    pub pack_presence_bits: i32,
}

/// One collision tree node (`cNode_t`): its split plane (normal, dist) and
/// two children; a negative child `c` is leaf `-1 - c`.
#[derive(Clone, Copy, Debug)]
pub struct ClipNodeRef {
    pub plane: [f32; 4],
    pub children: [i16; 2],
    /// bo2mp: the plane's `type` (its axis, 3 for none) and `signbits`.
    pub plane_type: u8,
    pub plane_signbits: u8,
}

/// One collision tree leaf (`cLeaf_s`): its mesh trees and the root of its
/// brush list in `leafbrush_nodes` (0 = no brushes).
#[derive(Clone, Copy, Debug, Default)]
pub struct ClipLeafRef {
    pub first_collision_box_slot: u16,
    pub collision_box_total: u16,
    pub solid_content_flags: i32,
    pub brush_leaf_link: i32,
    /// bo2mp: the leaf's mesh contents, box and cluster.
    pub ground_mask: i32,
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub cluster: i16,
}

/// One node of a leaf's brush list tree (`cLeafBrushNode_s`): brush indices
/// when `leaf_solid_total > 0`, else child offsets from this node.
#[derive(Clone, Debug)]
pub struct ClipLeafBrushNodeRef {
    pub leaf_solid_total: i16,
    pub brushes: Vec<u16>,
    pub child_offset: [u16; 2],
    /// bo2mp: the split axis, the contents below this node, and (for a
    /// split) where it splits and by how much it is widened.
    pub axis: u8,
    pub contents: i32,
    pub plane_dist: f32,
    pub range: f32,
}

/// One brush model (`cmodel_t`): model 0 is the world, the rest belong to
/// entities (doors, triggers, volumes) and are not world walls.
#[derive(Clone, Copy, Debug)]
pub struct ClipCmodelRef {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub radius: f32,
    pub leaf: ClipLeafRef,
    /// Its `ClipInfo` is the map's own (or none): its brush indices index
    /// the map's brushes.
    pub shares_map_info: bool,
}

/// The map's clipMap_t, in its own terms.
#[derive(Clone, Debug, Default)]
pub struct ClipRef {
    pub name: String,
    pub brushes: Vec<ClipBrushRef>,
    pub materials: Vec<ClipMaterialRef>,
    pub vertex_points: Vec<[f32; 3]>,
    pub tri_index_list: Vec<u16>,
    pub tri_edge_is_walkable: Vec<u8>,
    /// (triCount, firstTri) per partition.
    pub partitions: Vec<(u8, i32)>,
    pub box_trees: Vec<ClipAabbRef>,
    pub static_model_count: u32,
    pub nodes: Vec<ClipNodeRef>,
    pub leaves: Vec<ClipLeafRef>,
    pub leafbrush_nodes: Vec<ClipLeafBrushNodeRef>,
    /// bo2mp: `ClipInfo::brush_leaf_indices` (`numLeafBrushes` brush indices) and
    /// where it sits in the zone (block, offset).
    pub leaf_brushes: Vec<u16>,
    pub leaf_brushes_at: Option<(u8, u32)>,
    /// bo2mp: per leaf brush node, where its `brushes` pointer lands in the
    /// zone (block, offset); none for a split or an unresolved pointer.
    pub leafbrush_node_brushes_at: Vec<Option<(u8, u32)>>,
    /// `clipMap_t::box_model`'s leaf brush node: the box hull's own node,
    /// whose one brush index is `numBrushes`, naming `clipMap_t::box_brush`
    /// rather than a row of `brushes`.
    pub box_leaf_brush_node: i32,
    pub sub_models: Vec<ClipCmodelRef>,
    /// `ClipInfo::brushContents`: per-brush contents the world tree tests.
    pub solid_content_flags: Vec<i32>,
    /// bo2zm fix list 2: `staticModelList`, the placed models bullets and
    /// thrown things collide with.
    pub prop_set: Vec<ClipStaticModelRef>,
    /// codmp: the map's loose and breakable props (`dynEntDefList[0]`):
    /// the bottles, cones and crates a shot knocks over, the TVs and
    /// lamps that break.
    pub debris_pieces: Vec<ClipDynEntRef>,
}

/// One `DynEntityDef`, its asset pointers resolved to names.
#[derive(Clone, Debug, Default)]
pub struct ClipDynEntRef {
    /// 1 clutter (knocked about), 2 destruct (breaks), 3 hinge, 4 burst.
    pub kind: i32,
    /// `GfxPlacement`: quaternion (x, y, z, w) and origin.
    pub rotation: [f32; 4],
    pub origin: [f32; 3],
    pub model: String,
    pub destroyed_model: String,
    pub destroy_effect: String,
    pub destroy_sound: u32,
    pub physics_template: String,
    pub solid_model: u16,
    pub physics_solid_mesh: u16,
    pub health: i32,
    pub flags: u32,
    pub contents: u32,
}

fn read_leaf(s: &ZoneByteReader<'_>, p: RawRef) -> Result<ClipLeafRef> {
    Ok(ClipLeafRef {
        first_collision_box_slot: s.ushort_from_bytes(p, cLeaf_s::firstCollAabbIndex)?,
        collision_box_total: s.ushort_from_bytes(p, cLeaf_s::collAabbCount)?,
        solid_content_flags: s.int_from_bytes(p, cLeaf_s::brushContents)?,
        brush_leaf_link: s.int_from_bytes(p, cLeaf_s::leafBrushNode)?,
        ground_mask: s.int_from_bytes(p, cLeaf_s::terrainContents)?,
        lo_corner: vec3(s, p, cLeaf_s::mins)?,
        hi_corner: vec3(s, p, cLeaf_s::maxs)?,
        cluster: s.short_from_bytes(p, cLeaf_s::cluster)?,
    })
}

/// Follow a pointer field that the walk already resolved.
pub(crate) fn deref(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> Result<Option<RawRef>> {
    let StreamRef::At(target) = s.address_for(p, off)? else {
        return Ok(None);
    };
    Ok(Some(s.follow_entry_link(target)))
}

pub(crate) fn string_field(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> Result<String> {
    Ok(match deref(s, p, off)? {
        Some(q) => String::from_utf8_lossy(s.zero_ended_bytes(q)?).into_owned(),
        None => String::new(),
    })
}

fn le_u16s(raw: &[u8]) -> Vec<u16> {
    raw.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect()
}

fn le_u32s(raw: &[u8]) -> Vec<u32> {
    raw.chunks_exact(4).map(|quad| u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])).collect()
}

/// Pushes `item` and returns the index it landed at.
fn push_indexed<T>(list: &mut Vec<T>, item: T) -> usize {
    list.push(item);
    list.len() - 1
}

fn vec3(s: &ZoneByteReader<'_>, p: RawRef, base: usize) -> Result<[f32; 3]> {
    let axis = |n: usize| s.float_from_bytes(p, base + n * 4);
    Ok([axis(0)?, axis(1)?, axis(2)?])
}

/// Read `count` records of `stride` bytes from the array the pointer ptr_field `ptr_field` of
/// `owner` names; a null pointer is an empty table.
fn read_table<T>(
    s: &ZoneByteReader<'_>,
    owner: RawRef,
    ptr_field: usize,
    count: usize,
    stride: usize,
    read: impl FnMut(RawRef) -> Result<T>,
) -> Result<Vec<T>> {
    match deref(s, owner, ptr_field)? {
        Some(base) => read_run(base, count, stride, read),
        None => Ok(Vec::new()),
    }
}

/// Read `count` records laid `stride` bytes apart from `base`, one `read` call each.
fn read_run<T>(
    base: RawRef,
    count: usize,
    stride: usize,
    mut read: impl FnMut(RawRef) -> Result<T>,
) -> Result<Vec<T>> {
    (0..count).map(|n| read(base.shifted_by(n * stride))).collect()
}

fn bytes_field(s: &ZoneByteReader<'_>, p: RawRef, off: usize, len: usize) -> Result<Vec<u8>> {
    Ok(match deref(s, p, off)? {
        Some(q) if len > 0 => s.byte_window_from(q, 0, len)?.to_vec(),
        _ => Vec::new(),
    })
}

pub fn read_image(s: &ZoneByteReader<'_>, img: RawRef) -> Result<ImageRef> {
    let part = img.shifted_by(GfxImage::streamedParts);
    let embedded = match deref(s, img, GfxImage::texture)? {
        Some(load_def) => {
            let size = s.uint_from_bytes(load_def, GfxImageLoadDef::resourceSize)? as usize;
            if size == 0 {
                None
            } else {
                Some(EmbeddedImage {
                    dxgi_format: s.uint_from_bytes(load_def, GfxImageLoadDef::format)?,
                    tier_total: s.u8_at(load_def, GfxImageLoadDef::levelCount)?,
                    flags: s.u8_at(load_def, GfxImageLoadDef::flags)?,
                    data: s.byte_window_from(load_def, GfxImageLoadDef::data, size)?.to_vec(),
                })
            }
        }
        None => None,
    };
    Ok(ImageRef {
        name: string_field(s, img, GfxImage::name)?,
        label_digest: s.uint_from_bytes(img, GfxImage::hash)?,
        map_kind: s.u8_at(img, GfxImage::mapType)?,
        width: s.ushort_from_bytes(img, GfxImage::width)?,
        height: s.ushort_from_bytes(img, GfxImage::height)?,
        depth: s.ushort_from_bytes(img, GfxImage::depth)?,
        tier_total: s.u8_at(img, GfxImage::levelCount)?,
        streamed_parts: s.u8_at(img, GfxImage::streamedPartCount)?,
        part_hash: s.uint_from_bytes(part, GfxStreamedPartInfo::hash)?,
        embedded,
    })
}

/// bo2zm M3: one AI path node (GameWorldMp `pathnode_t`): its kind (path,
/// cover, negotiation begin/end, ...), names, where it is and its links.
#[derive(Clone, Debug, Default)]
pub struct PathNodeRef {
    pub ty: u32,
    pub spawnflags: u32,
    pub target_key: String,
    pub target: String,
    pub script_note: String,
    pub script_link: String,
    pub animscript: String,
    pub origin: [f32; 3],
    pub angle: f32,
    pub radius: f32,
    /// (node, distance, negotiation link).
    pub links: Vec<(u16, f32, bool)>,
}

/// bo2mp: the map compiler's baked path tables (GameWorldMp `PathData`):
/// which node pairs see each other (`pathVis`, bit `n * i + j` for i < j)
/// and which a straight walk joins (`smoothCache`, bit
/// `original * i + j`), as the engine's `Path_NodesVisible` and
/// `Path_NodesSmooth` read them.
#[derive(Clone, Debug, Default)]
pub struct PathTablesRef {
    pub node_count: u32,
    pub original_count: u32,
    pub vis: Vec<u8>,
    pub smooth: Vec<u8>,
}

/// bo2zm M3: a string table: `rows` x `columns` cells, row-major.
#[derive(Clone, Debug, Default)]
pub struct StringTableRef {
    pub name: String,
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<String>,
}

/// bo2mp: a vehicle's own weapons (`VehicleDef::turretWeapon`,
/// `gunnerWeapon[4]`): what a scorestreak's helicopter fires.
#[derive(Clone, Debug, Default)]
pub struct VehicleRef {
    pub name: String,
    pub turret_weapon: String,
    pub gunner_weapons: Vec<String>,
    pub drive: VehicleDrive,
    /// The engine's sound (`VehicleDef::engine`).
    pub engine: VehicleEngineRef,
}

/// bo2mp: a vehicle's engine sound (`VehicleEngine`): two banks of five
/// loops, one heard on the throttle (`onload`) and one off it
/// (`offload`), faded and pitched by the engine's RPM.
#[derive(Clone, Debug, Default)]
pub struct VehicleEngineRef {
    pub idle_rpms: f32,
    pub max_rpms: f32,
    pub max_torque: f32,
    pub braking_coeff: f32,
    pub load_fade_params: [f32; 4],
    pub load_scale: f32,
    pub load_smoothing: f32,
    pub throttle_lag: f32,
    pub tilt_multiplier: f32,
    /// (`name`, `alias` (the alias name's hash), `params[5]`).
    pub onload: Vec<(String, u32, [f32; 5])>,
    pub offload: Vec<(String, u32, [f32; 5])>,
    pub num_gears: i32,
    pub loop_last_gear: i32,
    /// `gears[i]`: (`minRPM`, `maxRPM`, `ratio`).
    pub gears: Vec<[f32; 3]>,
}

/// bo2mp: a destructible's definition (`DestructibleDef`): the models it
/// stands for. A map entity names a destructible with `destructibledef`;
/// its own `model` is only the editor's stand-in.
#[derive(Clone, Debug, Default)]
pub struct DestructibleRef {
    pub name: String,
    /// The model the entity is built with.
    pub model: String,
    /// The pristine variant.
    pub pristine_model: String,
    /// The pieces that break off (`DestructiblePiece`), piece 0 the base.
    pub pieces: Vec<DestructiblePieceRef>,
    /// Only the clients break it (nothing the server tracks).
    pub local_machine_only: bool,
}

/// bo2mp: one piece of a destructible (`DestructiblePiece`): its health,
/// how much each kind of damage counts, the stages it breaks through and
/// the bones it starts with hidden.
#[derive(Clone, Debug, Default)]
pub struct DestructiblePieceRef {
    pub stages: [DestructibleStageRef; 5],
    pub owner_fragment: u8,
    pub inherited_harm_ratio: f32,
    pub round_damage_multiplier: f32,
    pub blast_damage_multiplier: f32,
    pub melee_damage_scale: f32,
    pub impact_damage_scale: f32,
    /// The share of a hit on the piece the entity's own health loses.
    pub entity_damage_transfer: f32,
    pub health: i32,
    /// Hide-part bits (`hideBones`), the model's bone order.
    pub conceal_joints: [u32; 5],
    /// The first of the piece's constraints (`physConstraints->data[0]`),
    /// the only one a break reads: a launch (type 7) aims what breaks off.
    pub constraint: Option<PhysConstraintRef>,
}

/// bo2mp: a physics preset (`PhysPreset`): how a loose piece weighs,
/// bounces and falls.
#[derive(Clone, Debug, Default)]
pub struct PhysPresetRef {
    pub name: String,
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    pub bullet_push: f32,
    pub blast_push: f32,
    pub shard_scatter_ratio: f32,
    pub shard_rise_speed: f32,
    pub gravity_scale: f32,
    pub center_of_mass_offset: [f32; 3],
}

/// bo2mp: one physics constraint (`PhysConstraint`), the fields a break's
/// launch reads: the push's strength (`power`), its direction as angles
/// added to the thing's own (`scale`) and how far off its centre it is
/// pushed (`spin_scale`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysConstraintRef {
    pub kind_code: i32,
    pub power: f32,
    pub size_mul: [f32; 3],
    pub spin_scale: f32,
}

/// bo2mp: a model's constraint set (`PhysConstraints`).
#[derive(Clone, Debug, Default)]
pub struct PhysConstraintsRef {
    pub name: String,
    pub data: Vec<PhysConstraintRef>,
}

/// bo2mp: one stage of a piece (`DestructibleStage`): the bone it shows,
/// the share of the piece's health it lasts to, and what breaking out of
/// it plays and tells the scripts. Empty strings are unset.
#[derive(Clone, Debug, Default)]
pub struct DestructibleStageRef {
    /// Empty: the stage shows nothing (the piece's last).
    pub reveal_joint: String,
    pub shatter_hitpoints: f32,
    /// A stage that breaks by itself after this many seconds.
    pub longest_duration: f32,
    pub flags: u32,
    pub shatter_visual: String,
    pub shatter_audio: String,
    /// What the scripts hear (`CodeCallback_DestructibleEvent` "broken").
    pub shatter_signal: String,
    pub repeating_audio: String,
    pub spawn_models: [String; 3],
    pub physics_template: bool,
}

/// bo2mp: how a vehicle drives and how its driver sees it (`VehicleDef`).
/// Speeds are units a second, angles degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleDrive {
    /// `cameraMode`: 0 = his view from the vehicle's `tag_player`, else the
    /// third-person camera below.
    pub camera_mode: i32,
    pub top_velocity: f32,
    pub max_speed_vertical: f32,
    pub speed_up_rate: f32,
    pub accel_vertical: f32,
    /// Reversing, a share of `top_velocity` (`nitrousVehParams.m_reverse_scale`).
    pub reverse_scale: f32,
    /// `thirdPersonCameraRange`, `thirdPersonCameraHeight[1]`.
    pub camera_range: f32,
    pub camera_height: f32,
    /// How far the third-person camera looks up and down
    /// (`thirdPersonCameraMinPitchClamp`, `...MaxPitchClamp`).
    pub camera_pitch: [f32; 2],
    /// `cameraFOV` (0: his own).
    pub camera_fov: f32,
    /// `thirdPersonCameraSpringDistance`, `...SpringTime`: the camera falls
    /// back this far at top speed, over about this long.
    pub camera_spring: [f32; 2],
    /// How far the driver looks up and down from its gun
    /// (`turretViewLimits[2]`, `[3]`).
    pub turret_pitch: [f32; 2],
    /// `thirdPersonDriver`: the driver's seat views from outside.
    pub third_person_driver: i32,
    /// The command each of the vehicle's buttons stands for, in BO2's
    /// default controller binds (players/bindings_mp.bdg: A +gostand, B
    /// +stance, X +usereload, Y +weapnext_inventory, LSHLDR +smoke, RSHLDR
    /// +frag, LSTICK +breath_sprint, RSTICK +melee, LTRIG +speed_throw, RTRIG
    /// +attack; "" for none): `moveUpButtonName`, `moveDownButtonName`,
    /// `switchSeatButtonName`, `attackButtonName`, `attackSecondaryButtonName`.
    /// BO2 puts the vehicle's own commands on the keys he has for these.
    pub buttons: [&'static str; 5],
    /// The commands of `boostButtonName` and `handBrakeButtonName`.
    pub boost_button: &'static str,
    pub hand_brake_button: &'static str,
    /// `boostAccelMultiplier` (an int in the def), `boostDuration` (seconds,
    /// one tank a vehicle), `boostSpeedIncrease` (top speed scale).
    pub boost_accel_mult: f32,
    pub boost_duration: f32,
    pub boost_speed_increase: f32,
    /// `nitrousVehParams`: `m_steer_angle_max` (radians), `m_steer_speed`
    /// (radians a second), tyre grip `m_tire_fric_fwd`, `_side`, `_brake`,
    /// `_hand_brake`, and `m_auto_hand_brake_min_speed`.
    pub steer_angle_max: f32,
    pub steer_speed: f32,
    pub tire_fric: [f32; 4],
    pub auto_hand_brake_min_speed: f32,
    /// `m_peel_out_max_speed` (it peels out on full throttle below it) and
    /// the def's `skidSpeedMin` (a wheel sliding faster than it skids).
    pub peel_out_max_speed: f32,
    pub skid_speed_min: f32,
    /// `isNitrous`: driven by BO2's nitrous physics, so its body tilts with
    /// the car itself (the pose's own pitch and roll stay 0).
    pub is_nitrous: i32,
    /// `wheelRotRate`: a wheel turns this many degrees a unit driven.
    pub wheel_rot_rate: f32,
    /// `maxBodyPitch`, `maxBodyRoll`, `suspensionTravel`.
    pub max_body_pitch: f32,
    pub max_body_roll: f32,
    pub suspension_travel: f32,
    /// `nitrousVehParams`: `m_wheel_radius`, `m_susp_hard_limit`,
    /// `m_susp_min_height`, `m_roll_stability`, `m_pitch_stability`.
    pub wheel_radius: f32,
    pub susp_hard_limit: f32,
    pub susp_min_height: f32,
    pub roll_stability: f32,
    pub pitch_stability: f32,
    /// What hurts it (`VEH_IsImmune`): `remoteControl` (fire burns a driven
    /// one), then whether bullets, armor-piercing bullets, grenades,
    /// projectiles, projectile splash and heavy explosives do
    /// (`bulletDamage`, `armorPiercingDamage`, `grenadeDamage`,
    /// `projectileDamage`, `projectileSplashDamage`, `heavyExplosiveDamage`),
    /// and `noDirectionalDamage` (no harder blast from underneath).
    pub remote_operated: i32,
    pub bullet_damage: i32,
    pub armor_piercing_damage: i32,
    pub grenade_damage: i32,
    pub projectile_damage: i32,
    pub projectile_splash_damage: i32,
    pub heavy_explosive_damage: i32,
    pub no_directional_damage: i32,
    /// Its kind (`type`: 0 four wheels, 2 a tank, 3 a plane, 6 a
    /// helicopter).
    pub vehicle_type: i32,
    /// How a helicopter flies.
    pub heli: HeliDrive,
    /// `collisionDamage` and `collisionSpeed` (units a second; the def holds
    /// it already scaled from miles an hour): what a moving vehicle's front
    /// does to whoever it touches.
    pub collision_damage: f32,
    pub collision_speed: f32,
}

/// bo2mp: how a helicopter flies (BG_VehicleHelicopterMove), from its
/// VehicleDef; its top speeds and accelerations are the vehicle's own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliDrive {
    /// `rotRate`, `rotAccel`: the most degrees a second it turns, and how
    /// fast it gets to that.
    pub rot_rate: f32,
    pub rot_accel: f32,
    /// `tiltFromAcceleration`, `tiltFromDeceleration`, `tiltFromVelocity`,
    /// `tiltSpeed`: forward, then sideways.
    pub tilt_from_accel: [f32; 2],
    pub tilt_from_decel: [f32; 2],
    pub tilt_from_vel: [f32; 2],
    pub tilt_speed: [f32; 2],
    /// `useCollmap`: it moves as its model's collision brush.
    pub use_collmap: i32,
    /// `vehHelicopterDecelerationFwd`, `Side`, `Up`.
    pub decel: [f32; 3],
    /// `vehHelicopterTiltFromControllerAxes`, `vehHelicopterTiltFromFwdAndYaw`,
    /// `vehHelicopterTiltFromFwdAndYaw_VelAtMaxTilt`,
    /// `vehHelicopterTiltMomentum`.
    pub tilt_from_axes: f32,
    pub tilt_from_fwd_yaw: f32,
    pub vel_at_max_tilt: f32,
    pub tilt_momentum: f32,
    /// `vehHelicopterAccelTwardsView`: forward climbs and dives with the
    /// view's pitch.
    pub accel_toward_view: i32,
    /// `vehHelicopterMaintainHeight`, `...HeightLimit`, `...HeightMinimum`,
    /// `...HeightMaximum` (how far down it looks for the ground),
    /// `vehHelicopterMaintainCeilingMinimum`.
    pub maintain_height: i32,
    pub height_limit: f32,
    pub height_min: f32,
    pub height_max: f32,
    pub ceiling_min: f32,
}

/// The command BO2's default controller binds put on a button name.
fn vehicle_button_command(name: &str) -> &'static str {
    match name {
        "BUTTON_A" => "+gostand",
        "BUTTON_B" => "+stance",
        "BUTTON_X" => "+usereload",
        "BUTTON_Y" => "+weapnext_inventory",
        "BUTTON_LSHLDR" => "+smoke",
        "BUTTON_RSHLDR" => "+frag",
        "BUTTON_LSTICK" => "+breath_sprint",
        "BUTTON_RSTICK" => "+melee",
        "BUTTON_LTRIG" => "+speed_throw",
        "BUTTON_RTRIG" => "+attack",
        _ => "",
    }
}

/// codmp: one breakable window (`Glass`): the brush model it stands on, its
/// outline in its own plane, and its def's health, materials and sounds.
#[derive(Clone, Debug, Default)]
pub struct GlassRef {
    /// The `*N` brush model the window's entity is (its clip brushes).
    pub solid_model: u32,
    /// `Glass::index`: the id a trace that meets the window reports.
    pub index: u32,
    /// Where the brush model stands and how it turns (`CM_TransformedBoxTrace`'s
    /// origin and angles; the model's brushes are its own space).
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// Outline corners in the window's plane (units along `axis[0]`, `axis[1]`).
    pub outline: Vec<[f32; 2]>,
    /// In-plane x, in-plane y, normal.
    pub axis: [[f32; 3]; 3],
    /// Where the outline's (0, 0) stands in the world.
    pub outline_origin: [f32; 3],
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    /// Texture units per outline unit.
    pub uv_scale: f32,
    pub definition_name: String,
    pub hit_point_ceiling: i32,
    pub wall_depth: f32,
    pub pristine_material: String,
    pub cracked_material: String,
    pub shard_material: String,
    pub crack_sound: String,
    pub shatter_sound: String,
    pub auto_shatter_sound: String,
    /// The def's shard limits (`GlassDef::minShardSize`, `maxShardSize`,
    /// `shardLifeProbablility`, `maxShards`).
    pub min_shard_size: f32,
    pub max_shard_size: f32,
    pub shard_life_probability: f32,
    pub max_shards: i32,
    /// The effects it plays as it cracks and as it shatters (`crackEffect`,
    /// `shatterEffect`), by name ("" when it has none).
    pub crack_effect: String,
    pub shatter_effect: String,
    /// bo2mp: what its shards are cut from (`Glass::thickness`,
    /// `Glass::isPlanar`: only a flat pane's border counts as frame).
    pub pane_thickness: f32,
    pub is_planar: bool,
    /// The map's shard pools (`Glasses::maxShards`, `maxPhysics`): the most
    /// shards, and the most falling ones without collision, at once.
    pub pool_shards: u32,
    pub pool_physics: u32,
}

/// bo2zm: a window/box barrier type (`ZBarrierDef`, named by a map
/// entity's `type` key): the zombie animation states it asks for.
#[derive(Clone, Debug, Default)]
pub struct ZBarrierDefRef {
    pub name: String,
    pub taunts: bool,
    pub reach_through: bool,
    /// Pieces torn off (state "open") stop drawing (`autoHideOpenPieces`).
    pub auto_hide_open_pieces: bool,
    pub taunt_state: String,
    pub reach_through_state: String,
    pub num_attack_slots: u32,
    pub attack_spot_horz_offset: f32,
    /// Per board: (tear anim state, tear anim substate).
    pub boards: Vec<(String, String)>,
    /// Per board: (model, opening clip, closing clip).
    pub looks: Vec<(String, String, String)>,
}

/// Everything one walk captured.
#[derive(Default)]
pub struct ZoneCapture {
    pub zone: String,
    pub images: Vec<ImageRef>,
    pub materials: Vec<MaterialRef>,
    pub technique_sets: Vec<TechniqueSetRef>,
    pub xmodels: Vec<XModelRef>,
    pub world: Option<WorldRef>,
    pub clip: Option<ClipRef>,
    /// `ComWorld::primaryLights`.
    pub key_lights: Vec<PrimaryLightRef>,
    /// Every RawFile (vision files, configs, Lua): (name, bytes).
    pub raw_files: Vec<(String, Vec<u8>)>,
    /// bo2zm M3: every StringTable (`mp/zombiemode.csv`, ...).
    pub string_tables: Vec<StringTableRef>,
    /// bo2zm: barrier types (window boards, the box).
    pub zbarriers: Vec<ZBarrierDefRef>,
    /// bo2zm M3: every localized string (`ZOMBIE_WEAPON_M14` -> its text),
    /// from a language zone (`zone/english/en_*.ff`).
    pub localize: Vec<(String, String)>,
    /// bo2zm M3: the map's AI path nodes and their links (GameWorldMp).
    pub path_nodes: Vec<PathNodeRef>,
    /// bo2mp: the map's baked node sight and smooth tables (GameWorldMp).
    pub path_tables: PathTablesRef,
    /// Every XAnimParts: its timing and root translation.
    pub animation_refs: Vec<XAnimRef>,
    /// Every MapEnts entity string, in load order (the clipmap's own first).
    pub map_ents: Vec<String>,
    /// bo2zm M2: the zone's script strings (bone names, notetracks).
    pub strings: Vec<String>,
    pub weapons: Vec<crate::m2::WeaponRef>,
    /// bo2mp: weapon attachments (`acog`, `vzoom`: what an attachment
    /// changes on every gun) and each gun's own take on one
    /// (`au_svu_acog`: its sight model, scope overlay, animations).
    pub attachments: Vec<crate::m2::AttachmentRef>,
    pub attachment_uniques: Vec<crate::m2::AttachmentUniqueRef>,
    /// bo2mp: vehicles and their weapons.
    pub vehicles: Vec<VehicleRef>,
    /// codmp: the map's breakable windows (`Glasses`).
    pub glasses: Vec<GlassRef>,
    /// bo2mp: destructible definitions (a map's cars, barrels, ...).
    pub destructibles: Vec<DestructibleRef>,
    /// bo2mp: physics presets and constraint sets (what a broken-off piece
    /// flies with).
    pub phys_presets: Vec<PhysPresetRef>,
    pub phys_constraints: Vec<PhysConstraintsRef>,
    /// bo2zm M3: weapon camos (Pack-a-Punch's look).
    pub weapon_camos: Vec<crate::m2::WeaponCamoRef>,
    /// bo2zm M3: fonts (the HUD's text).
    pub fonts: Vec<crate::m2::FontRef>,
    pub fx: Vec<crate::m2::FxEffectRef>,
    pub tracers: Vec<crate::m2::TracerRef>,
    pub impact_tables: Vec<crate::m2::ImpactTableRef>,
    pub footstep_tables: Vec<crate::m2::FootstepTableRef>,
    pub footstep_fx_tables: Vec<crate::m2::FootstepFxTableRef>,
    pub sound_banks: Vec<crate::m2::SndBankRef>,
    pub snd_globals: Option<crate::m2::SndDriverGlobalsRef>,
    pub loads: BTreeMap<XAssetKind, usize>,
    /// The slot each kept asset was loaded through, with the load's serial:
    /// TEMP addresses are reused, so a slot entry only counts for a parent
    /// that began before it.
    slots: HashMap<RawRef, (AssetIdentity, u64)>,
    /// Insert aliases (VIRTUAL, stable): what later references go through.
    insert_aliases: HashMap<RawRef, AssetIdentity>,
    /// `started` of the asset whose fields are being read.
    parent_started: u64,
    /// Asset pointers that named nothing this capture kept.
    pub unresolved_refs: usize,
    /// Keep every pass's shader bytecode (probes only; a game load does not).
    pub keep_shaders: bool,
}

impl ZoneCapture {
    /// The asset an asset-pointer slot of the asset being read names, if
    /// this capture kept it: a load through that very slot during this
    /// parent, else the alias the pointer goes through.
    pub fn asset_at(&mut self, s: &ZoneByteReader<'_>, slot: RawRef) -> Result<Option<AssetIdentity>> {
        let slot_word = s.uint_from_bytes(slot, 0)?;
        if slot_word == 0 {
            return Ok(None);
        }
        if let Some(&(key, serial)) = self.slots.get(&slot)
            && serial > self.parent_started
        {
            return Ok(Some(key));
        }
        if let StreamRef::At(p) = StreamRef::from_word(slot_word)
            && let Some(key) = self.insert_aliases.get(&p)
        {
            return Ok(Some(*key));
        }
        self.unresolved_refs += 1;
        Ok(None)
    }

    /// The zone string table entry for `id`, empty when it is out of range.
    fn string_for(&self, id: u16) -> String {
        self.strings.get(usize::from(id)).cloned().unwrap_or_default()
    }

    fn keep(&mut self, loaded: Loaded, key: AssetIdentity) {
        self.slots.insert(loaded.bay, (key, loaded.serial));
        // Later references name either the insert alias or, when the asset
        // loaded through a slot outside TEMP (an asset-list entry, an array
        // of asset pointers), that slot itself; both stay put for the zone.
        if let Some(alias) = loaded.alias {
            self.insert_aliases.insert(alias, key);
        }
        if loaded.bay.seg as usize != fastfile_t6::ARCHIVE_SEGMENT_SCRATCH {
            self.insert_aliases.insert(loaded.bay, key);
        }
    }

    pub fn image(&self, key: AssetIdentity) -> Option<&ImageRef> {
        (key.ty == XAssetKind::Image)
            .then(|| self.images.get(key.index))
            .flatten()
    }

    pub fn material(&self, key: AssetIdentity) -> Option<&MaterialRef> {
        (key.ty == XAssetKind::Material)
            .then(|| self.materials.get(key.index))
            .flatten()
    }

    fn read_material(&mut self, s: &ZoneByteReader<'_>, m: RawRef) -> Result<MaterialRef> {
        let info = m.shifted_by(Material::info);
        let count = s.u8_at(m, Material::textureCount)? as usize;
        let mut textures = Vec::with_capacity(count);
        if let Some(table) = deref(s, m, Material::textureTable)? {
            for i in 0..count {
                let t = table.shifted_by(i * MaterialTextureDef::SIZE);
                textures.push(ShaderSamplerImage {
                    label_digest: s.uint_from_bytes(t, MaterialTextureDef::nameHash)?,
                    semantic: s.u8_at(t, MaterialTextureDef::semantic)?,
                    texture_sampling_mode: s.u8_at(t, MaterialTextureDef::samplerState)?,
                    image: self.asset_at(s, t.shifted_by(MaterialTextureDef::image))?,
                });
            }
        }
        let constant_count = s.u8_at(m, Material::constantCount)? as usize;
        let mut constants = Vec::with_capacity(constant_count);
        if let Some(table) = deref(s, m, Material::constantTable)? {
            for i in 0..constant_count {
                let c = table.shifted_by(i * MaterialConstantDef::SIZE);
                let name = s.byte_window_from(c, MaterialConstantDef::name, 12)?;
                let text_len = name.iter().take_while(|&&b| b != 0).count();
                constants.push((
                    s.uint_from_bytes(c, MaterialConstantDef::nameHash)?,
                    String::from_utf8_lossy(&name[..text_len]).into_owned(),
                    [
                        s.float_from_bytes(c, MaterialConstantDef::literal)?,
                        s.float_from_bytes(c, MaterialConstantDef::literal + 4)?,
                        s.float_from_bytes(c, MaterialConstantDef::literal + 8)?,
                        s.float_from_bytes(c, MaterialConstantDef::literal + 12)?,
                    ],
                ));
            }
        }
        let mut entry_bits = [0xffu8; 36];
        for (i, e) in entry_bits.iter_mut().enumerate() {
            *e = s.u8_at(m, Material::stateBitsEntry + i)?;
        }
        let state_count = usize::from(s.u8_at(m, Material::stateBitsCount)?);
        let mut bit_words = Vec::with_capacity(state_count);
        if let Some(table) = deref(s, m, Material::stateBitsTable)? {
            for i in 0..state_count {
                let e = table.shifted_by(i * GfxStateBits::SIZE + GfxStateBits::loadBits);
                bit_words.push([s.uint_from_bytes(e, 0)?, s.uint_from_bytes(e, 4)?]);
            }
        }
        Ok(MaterialRef {
            name: string_field(s, info, MaterialInfo::name)?,
            sort_rank: s.u8_at(info, MaterialInfo::sortKey)?,
            game_flags: s.uint_from_bytes(info, MaterialInfo::gameFlags)?,
            facet_type_bits: s.uint_from_bytes(info, MaterialInfo::surfaceTypeBits)?,
            surf_bits: s.uint_from_bytes(info, MaterialInfo::surfaceFlags)?,
            contents: s.uint_from_bytes(info, MaterialInfo::contents)?,
            method_set: self.asset_at(s, m.shifted_by(Material::techniqueSet))?,
            textures,
            constants,
            state_bits_entry: entry_bits,
            state_words: bit_words,
            condition_bits: s.u8_at(m, Material::stateFlags)?,
            camera_zone: s.u8_at(m, Material::cameraRegion)?,
            thermal_material: self.asset_at(s, m.shifted_by(Material::thermalMaterial))?,
        })
    }

    fn read_technique_set(&mut self, s: &ZoneByteReader<'_>, t: RawRef) -> Result<TechniqueSetRef> {
        let slots = (MaterialTechniqueSet::SIZE - MaterialTechniqueSet::techniques) / 4;
        let mut tech_list = Vec::with_capacity(slots);
        for i in 0..slots {
            let Some(tech) = deref(s, t, MaterialTechniqueSet::techniques + i * 4)? else {
                tech_list.push(None);
                continue;
            };
            let stage_total = usize::from(s.ushort_from_bytes(tech, MaterialTechnique::passCount)?);
            let mut passes = Vec::with_capacity(stage_total);
            for p in 0..stage_total {
                let pass = tech.shifted_by(MaterialTechnique::passArray + p * MaterialStage::SIZE);
                let shader = |off: usize, load: usize| -> Result<(String, Vec<u8>)> {
                    let Some(sh) = deref(s, pass, off)? else {
                        return Ok((String::new(), Vec::new()));
                    };
                    let name = string_field(s, sh, MaterialPixelShader::name)?;
                    let program = if self.keep_shaders {
                        let def = sh.shifted_by(load);
                        let size = s.uint_from_bytes(def, GfxPixelShaderLoadDef::programSize)? as usize;
                        bytes_field(s, def, GfxPixelShaderLoadDef::program, size)?
                    } else {
                        Vec::new()
                    };
                    Ok((name, program))
                };
                let load = MaterialPixelShader::prog + MaterialPixelShaderProgram::loadDef;
                let (pixel_shader, pixel_program) = shader(MaterialStage::pixelShader, load)?;
                let (vertex_shader, vertex_program) = shader(MaterialStage::vertexShader, load)?;
                let arg_count = usize::from(s.u8_at(pass, MaterialStage::perPrimArgCount)?)
                    + usize::from(s.u8_at(pass, MaterialStage::perObjArgCount)?)
                    + usize::from(s.u8_at(pass, MaterialStage::stableArgCount)?);
                let mut args = Vec::with_capacity(arg_count);
                if let Some(arr) = deref(s, pass, MaterialStage::arg_rows)? {
                    for a in 0..arg_count {
                        let arg = arr.shifted_by(a * MaterialShaderArgument::SIZE);
                        args.push(ShaderArgRef {
                            ty: s.ushort_from_bytes(arg, MaterialShaderArgument::r#type)?,
                            location: s.ushort_from_bytes(arg, MaterialShaderArgument::location)?,
                            size: s.ushort_from_bytes(arg, MaterialShaderArgument::size)?,
                            buffer: s.ushort_from_bytes(arg, MaterialShaderArgument::buffer)?,
                            u: s.uint_from_bytes(arg, MaterialShaderArgument::u)?,
                        });
                    }
                }
                passes.push(PassRef {
                    vertex_shader,
                    pixel_shader,
                    pixel_program,
                    vertex_program,
                    args,
                });
            }
            tech_list.push(Some(TechniqueRef {
                name: string_field(s, tech, MaterialTechnique::name)?,
                flags: s.ushort_from_bytes(tech, MaterialTechnique::flags)?,
                passes,
            }));
        }
        Ok(TechniqueSetRef {
            name: string_field(s, t, MaterialTechniqueSet::name)?,
            level_vertex_format: s.u8_at(t, MaterialTechniqueSet::worldVertFormat)?,
            techniques: tech_list,
        })
    }

    fn read_xmodel(&mut self, s: &ZoneByteReader<'_>, x: RawRef) -> Result<XModelRef> {
        let numsurfs = s.u8_at(x, XModel::numsurfs)?;
        let mut materials = Vec::with_capacity(numsurfs as usize);
        if let Some(handles) = deref(s, x, XModel::materialHandles)? {
            for i in 0..numsurfs as usize {
                materials.push(self.asset_at(s, handles.shifted_by(i * 4))?);
            }
        }
        let lod_info = x.shifted_by(XModel::lodInfo);
        let lod_surfs = usize::from(s.ushort_from_bytes(lod_info, XModelLodInfo::numsurfs)?);
        let lod_first = usize::from(s.ushort_from_bytes(lod_info, XModelLodInfo::surfIndex)?);
        let mut lod0 = Vec::with_capacity(lod_surfs);
        if let Some(surfs) = deref(s, x, XModel::surfs)? {
            for j in lod_first..(lod_first + lod_surfs).min(usize::from(numsurfs)) {
                let sf = surfs.shifted_by(j * XSurface::SIZE);
                let vertex_tally = s.ushort_from_bytes(sf, XSurface::vertCount)?;
                let tri_tally = usize::from(s.ushort_from_bytes(sf, XSurface::triCount)?);
                let vertex_points = bytes_field(
                    s,
                    sf,
                    XSurface::verts0,
                    usize::from(vertex_tally) * GfxPackedVertex::SIZE,
                )?;
                let indices = read_table(s, sf, XSurface::triIndices, tri_tally * 3, 2, |at| {
                    s.ushort_from_bytes(at, 0)
                })?;
                let list_count = usize::from(s.u8_at(sf, XSurface::vertListCount)?);
                let mut vert_lists = Vec::with_capacity(list_count);
                if let Some(vl) = deref(s, sf, XSurface::vertList)? {
                    for k in 0..list_count {
                        let e = vl.shifted_by(k * XRigidVertList::SIZE);
                        vert_lists.push((
                            s.ushort_from_bytes(e, XRigidVertList::boneOffset)?,
                            s.ushort_from_bytes(e, XRigidVertList::vertCount)?,
                            s.ushort_from_bytes(e, XRigidVertList::triOffset)?,
                            s.ushort_from_bytes(e, XRigidVertList::triCount)?,
                        ));
                    }
                }
                let vi = sf.shifted_by(XSurface::vertInfo);
                let mut blend_counts = [0i16; 4];
                for (k, c) in blend_counts.iter_mut().enumerate() {
                    *c = s.short_from_bytes(vi, XSurfaceVertexInfo::vertCount + k * 2)?;
                }
                // Words per vertex with k+1 influences: 1 + 2k.
                let blend_words: usize = blend_counts
                    .iter()
                    .enumerate()
                    .map(|(k, &c)| c.max(0) as usize * (1 + 2 * k))
                    .sum();
                let mut blend = Vec::new();
                if blend_words > 0
                    && let Some(b) = deref(s, vi, XSurfaceVertexInfo::vertsBlend)?
                {
                    blend.reserve(blend_words);
                    for k in 0..blend_words {
                        blend.push(s.ushort_from_bytes(b, k * 2)?);
                    }
                }
                let mut part_bits = [0_u32; 5];
                for (k, w) in part_bits.iter_mut().enumerate() {
                    *w = s.uint_from_bytes(sf, XSurface::partBits + k * 4)?;
                }
                lod0.push(XSurfaceRef {
                    vertex_points,
                    vertex_tally,
                    indices,
                    vert_lists,
                    flags: s.u8_at(sf, XSurface::flags)?,
                    material: materials.get(j).copied().flatten(),
                    blend_counts,
                    blend,
                    piece_mask: part_bits,
                });
            }
        }
        // The skeleton.
        let skeleton_joint_total = usize::from(s.u8_at(x, XModel::numBones)?);
        let root_count = s.u8_at(x, XModel::numRootBones)?;
        let num_child = skeleton_joint_total.saturating_sub(usize::from(root_count));
        let joint_labels = read_table(s, x, XModel::boneNames, skeleton_joint_total, 2, |at| {
            Ok(self.string_for(s.ushort_from_bytes(at, 0)?))
        })?;
        let mut ancestor_chain = Vec::with_capacity(num_child);
        if let Some(arr) = deref(s, x, XModel::parentList)? {
            for i in 0..num_child {
                ancestor_chain.push(s.u8_at(arr, i)?);
            }
        }
        let mut rotation_quaternions = Vec::with_capacity(num_child);
        if let Some(arr) = deref(s, x, XModel::rotation_quaternions)? {
            for i in 0..num_child {
                let q = arr.shifted_by(i * XModelQuat::SIZE);
                rotation_quaternions.push([
                    s.short_from_bytes(q, 0)?,
                    s.short_from_bytes(q, 2)?,
                    s.short_from_bytes(q, 4)?,
                    s.short_from_bytes(q, 6)?,
                ]);
            }
        }
        // Three floats per bone, packed (measured: the HAMR's tag_flash sits
        // 32.95 ahead of j_gun in `baseMat` and at floats 9..11 here).
        let mut shift_offset = Vec::with_capacity(num_child);
        if let Some(arr) = deref(s, x, XModel::shift_offset)? {
            for i in 0..num_child {
                shift_offset.push(vec3(s, arr, i * 12)?);
            }
        }
        let mut foundation_surface = Vec::with_capacity(skeleton_joint_total);
        if let Some(arr) = deref(s, x, XModel::baseMat)? {
            for i in 0..skeleton_joint_total {
                let m = arr.shifted_by(i * BonePoseMat::SIZE);
                foundation_surface.push((
                    [
                        s.float_from_bytes(m, 0)?,
                        s.float_from_bytes(m, 4)?,
                        s.float_from_bytes(m, 8)?,
                        s.float_from_bytes(m, 12)?,
                    ],
                    vec3(s, m, BonePoseMat::shift_offset)?,
                ));
            }
        }
        let mut part_classes = Vec::with_capacity(skeleton_joint_total);
        if let Some(arr) = deref(s, x, XModel::partClassification)? {
            for i in 0..skeleton_joint_total {
                part_classes.push(s.u8_at(arr, i)?);
            }
        }
        let mut bone_info = Vec::with_capacity(skeleton_joint_total);
        if let Some(arr) = deref(s, x, XModel::boneInfo)? {
            for i in 0..skeleton_joint_total {
                let b = arr.shifted_by(i * XBoneInfo::SIZE);
                let mins = vec3(s, b, XBoneInfo::bounds)?;
                let maxs = vec3(s, b, XBoneInfo::bounds + 12)?;
                let r2 = s.float_from_bytes(b, XBoneInfo::radiusSquared)?;
                let ok = r2 > 0.0
                    && r2.is_finite()
                    && (0..3).all(|k| mins[k].is_finite() && maxs[k].is_finite() && mins[k] <= maxs[k]);
                bone_info.push(ok.then_some((mins, maxs, r2)));
            }
        }
        let mut collision_faces = Vec::new();
        let coll_count = s.int_from_bytes(x, XModel::numCollSurfs)?.clamp(0, 4096) as usize;
        if let Some(arr) = deref(s, x, XModel::collSurfs)? {
            for i in 0..coll_count {
                let cs = arr.shifted_by(i * XModelCollSurf_s::SIZE);
                let tri_tally =
                    s.int_from_bytes(cs, XModelCollSurf_s::numCollTris)?.clamp(0, 1 << 20) as usize;
                let mut triangle_list = Vec::with_capacity(tri_tally);
                if let Some(t) = deref(s, cs, XModelCollSurf_s::collTris)? {
                    for j in 0..tri_tally {
                        let row = t.shifted_by(j * XModelCollTri_s::SIZE);
                        let v4 = |off: usize| -> Result<[f32; 4]> {
                            Ok([
                                s.float_from_bytes(row, off)?,
                                s.float_from_bytes(row, off + 4)?,
                                s.float_from_bytes(row, off + 8)?,
                                s.float_from_bytes(row, off + 12)?,
                            ])
                        };
                        triangle_list.push([
                            v4(XModelCollTri_s::plane)?,
                            v4(XModelCollTri_s::surface_tangent)?,
                            v4(XModelCollTri_s::surface_bitangent)?,
                        ]);
                    }
                }
                collision_faces.push(XModelCollSurfRef {
                    triangle_list,
                    lo_corner: vec3(s, cs, XModelCollSurf_s::mins)?,
                    hi_corner: vec3(s, cs, XModelCollSurf_s::maxs)?,
                    joint_position: s.int_from_bytes(cs, XModelCollSurf_s::boneIdx)?,
                    contents: s.uint_from_bytes(cs, XModelCollSurf_s::contents)?,
                    face_property_bits: s.uint_from_bytes(cs, XModelCollSurf_s::surfFlags)?,
                });
            }
        }
        let mut collmap = Vec::new();
        let mut collmap_contents = 0;
        let mut collmap_brush_verts = Vec::new();
        let collmap_count = usize::from(s.u8_at(x, XModel::numCollmaps)?);
        if let Some(arr) = deref(s, x, XModel::collmaps)? {
            for i in 0..collmap_count {
                let Some(list) = deref(s, arr.shifted_by(i * Collmap::SIZE), Collmap::geomList)?
                else {
                    continue;
                };
                collmap_contents |= s.uint_from_bytes(list, PhysGeomList::contents)?;
                if i > 0 {
                    continue;
                }
                let n = s.uint_from_bytes(list, PhysGeomList::count)?.min(256) as usize;
                let Some(geoms) = deref(s, list, PhysGeomList::geoms)? else {
                    continue;
                };
                for j in 0..n {
                    let g = geoms.shifted_by(j * PhysGeomInfo::SIZE);
                    let o = PhysGeomInfo::orientation;
                    let ty = s.int_from_bytes(g, PhysGeomInfo::r#type)?;
                    let brush = match deref(s, g, PhysGeomInfo::brush)? {
                        Some(b) => {
                            let mins = vec3(s, b, BrushWrapper::mins)?;
                            let maxs = vec3(s, b, BrushWrapper::maxs)?;
                            let numsides = s.uint_from_bytes(b, BrushWrapper::numsides)?;
                            if j == 0 && ty == 2 {
                                collmap_brush_verts = if numsides == 0 {
                                    (0..8)
                                        .map(|c| {
                                            std::array::from_fn(|k| {
                                                if c >> k & 1 == 0 { mins[k] } else { maxs[k] }
                                            })
                                        })
                                        .collect()
                                } else {
                                    let nv = s.uint_from_bytes(b, BrushWrapper::numverts)?.min(64) as usize;
                                    read_table(s, b, BrushWrapper::vertex_points, nv, 12, |at| {
                                        vec3(s, at, 0)
                                    })?
                                };
                            }
                            Some((mins, maxs, numsides))
                        }
                        None => None,
                    };
                    collmap.push(PhysGeomRef {
                        ty,
                        orientation: [vec3(s, g, o)?, vec3(s, g, o + 12)?, vec3(s, g, o + 24)?],
                        offset: vec3(s, g, PhysGeomInfo::offset)?,
                        half_lengths: vec3(s, g, PhysGeomInfo::halfLengths)?,
                        brush,
                    });
                }
            }
        }
        let mut collmap_brushes = Vec::new();
        let collmap_count = s.u8_at(x, XModel::numCollmaps)? as usize;
        if let Some(arr) = deref(s, x, XModel::collmaps)? {
            for i in 0..collmap_count {
                let Some(list) = deref(s, arr.shifted_by(i * Collmap::SIZE), Collmap::geomList)? else {
                    continue;
                };
                let count: usize = s.uint_from_bytes(list, PhysGeomList::count)?.min(256) as usize;
                let Some(geoms) = deref(s, list, PhysGeomList::geoms)? else { continue };
                for g in (0..count).map(|n| geoms.shifted_by(n * PhysGeomInfo::SIZE)) {
                    // PHYS_GEOM_BRUSH; the others (boxes, cylinders) are
                    // physics-only shapes.
                    if s.int_from_bytes(g, PhysGeomInfo::r#type)? != 2 {
                        continue;
                    }
                    if let Some(b) = deref(s, g, PhysGeomInfo::brush)? {
                        collmap_brushes.push(read_brush(s, b)?);
                    }
                }
            }
        }
        let mut detail_tiers = Vec::with_capacity(4);
        for i in 0..4 {
            let li = x.shifted_by(XModel::lodInfo + i * XModelLodInfo::SIZE);
            detail_tiers.push((
                s.float_from_bytes(li, XModelLodInfo::dist)?,
                s.ushort_from_bytes(li, XModelLodInfo::numsurfs)?,
                s.ushort_from_bytes(li, XModelLodInfo::surfIndex)?,
            ));
        }
        Ok(XModelRef {
            name: string_field(s, x, XModel::name)?,
            numsurfs,
            materials,
            skeleton_joint_total: skeleton_joint_total as u8,
            detail_tier_total: s.ushort_from_bytes(x, XModel::numLods)?,
            lo_corner: vec3(s, x, XModel::mins)?,
            hi_corner: vec3(s, x, XModel::maxs)?,
            radius: s.float_from_bytes(x, XModel::radius)?,
            lighting_origin_offset: vec3(s, x, XModel::lightingOriginOffset)?,
            lighting_origin_range: s.float_from_bytes(x, XModel::lightingOriginRange)?,
            lod0,
            joint_labels,
            top_joint_total: root_count,
            ancestor_chain,
            rotation_quaternions,
            shift_offset,
            foundation_surface,
            body_part_category: part_classes,
            bone_info,
            detail_tiers,
            contents: s.uint_from_bytes(x, XModel::contents)?,
            collision_detail_tier: s.short_from_bytes(x, XModel::collLod)?,
            collision_faces,
            collmap_brushes,
            physics_template: self
                .asset_at(s, x.shifted_by(XModel::physPreset))?
                .filter(|k| k.ty == XAssetKind::PhysPreset),
            collmap,
            collmap_contents,
            collmap_brush_verts,
        })
    }

    fn read_clip(&mut self, s: &ZoneByteReader<'_>, c: RawRef) -> Result<ClipRef> {
        let info = c.shifted_by(clipMap_t::info);
        let material_total = s.uint_from_bytes(info, ClipInfo::numMaterials)? as usize;
        let materials = read_table(
            s,
            info,
            ClipInfo::materials,
            material_total,
            ClipSurfaceKind::SIZE,
            |at| {
                Ok(ClipMaterialRef {
                    name: string_field(s, at, ClipSurfaceKind::name)?,
                    surface_flags: s.int_from_bytes(at, ClipSurfaceKind::surfaceFlags)?,
                    pack_presence_bits: s.int_from_bytes(at, ClipSurfaceKind::contentFlags)?,
                })
            },
        )?;

        let brush_total = s.ushort_from_bytes(info, ClipInfo::numBrushes)? as usize;
        let brushes = read_table(s, info, ClipInfo::brushes, brush_total, cbrush_t::SIZE, |at| {
            read_brush(s, at)
        })?;

        let vertex_tally = s.uint_from_bytes(c, clipMap_t::vertCount)? as usize;
        let vertex_points = read_table(s, c, clipMap_t::vertex_points, vertex_tally, 12, |at| vec3(s, at, 0))?;
        let tri_tally = s.uint_from_bytes(c, clipMap_t::triCount)? as usize;
        let tri_list = read_table(s, c, clipMap_t::triIndices, tri_tally * 3, 2, |at| {
            s.ushort_from_bytes(at, 0)
        })?;
        let walk_len = (3 * tri_tally).div_ceil(32) * 4;
        let tri_edge_is_walkable = bytes_field(s, c, clipMap_t::triEdgeIsWalkable, walk_len)?;
        let partition_total = s.uint_from_bytes(c, clipMap_t::partitionCount)? as usize;
        let part_list = read_table(
            s,
            c,
            clipMap_t::partitions,
            partition_total,
            CollTriGroup::SIZE,
            |at| {
                Ok((
                    s.u8_at(at, CollTriGroup::triCount)?,
                    s.int_from_bytes(at, CollTriGroup::firstTri)?,
                ))
            },
        )?;
        let tree_count = s.uint_from_bytes(c, clipMap_t::aabbTreeCount)? as usize;
        let tree_list = read_table(
            s,
            c,
            clipMap_t::aabbTrees,
            tree_count,
            CollBoxTree::SIZE,
            |at| {
                Ok(ClipAabbRef {
                    origin: vec3(s, at, CollBoxTree::origin)?,
                    half_extent: vec3(s, at, CollBoxTree::halfSize)?,
                    material_slot: s.ushort_from_bytes(at, CollBoxTree::materialIndex)?,
                    sub_element_total: s.ushort_from_bytes(at, CollBoxTree::childCount)?,
                    u: s.int_from_bytes(at, CollBoxTree::u)?,
                })
            },
        )?;

        let node_total = s.uint_from_bytes(c, clipMap_t::numNodes)? as usize;
        let nodes = read_table(s, c, clipMap_t::nodes, node_total, cNode_t::SIZE, |at| {
            let (plane, plane_type, plane_signbits) = match deref(s, at, cNode_t::plane)? {
                Some(p) => {
                    let normal = vec3(s, p, cplane_s::normal)?;
                    (
                        [normal[0], normal[1], normal[2], s.float_from_bytes(p, cplane_s::dist)?],
                        s.u8_at(p, cplane_s::r#type)?,
                        s.u8_at(p, cplane_s::signbits)?,
                    )
                }
                None => ([0.0; 4], 3, 0),
            };
            let kids = [
                s.short_from_bytes(at, cNode_t::children)?,
                s.short_from_bytes(at, cNode_t::children + 2)?,
            ];
            Ok(ClipNodeRef {
                plane,
                plane_type,
                plane_signbits,
                children: kids,
            })
        })?;
        let leaf_total = s.uint_from_bytes(c, clipMap_t::numLeafs)? as usize;
        let leaves = read_table(s, c, clipMap_t::leafs, leaf_total, cLeaf_s::SIZE, |at| {
            read_leaf(s, at)
        })?;
        let leaf_brush_total = s.uint_from_bytes(info, ClipInfo::numLeafBrushes)? as usize;
        let leaf_brush_array = deref(s, info, ClipInfo::brush_leaf_indices)?;
        let leaf_brushes = leaf_brush_array.map_or_else(
            || Ok(Vec::new()),
            |base| read_run(base, leaf_brush_total, 2, |at| s.ushort_from_bytes(at, 0)),
        )?;
        let leaf_brushes_at = leaf_brush_array.map(|base| (base.seg, base.offset));
        let lb_count = s.uint_from_bytes(info, ClipInfo::leafbrushNodesCount)? as usize;
        let mut lb_nodes = Vec::with_capacity(lb_count);
        let mut leafbrush_node_brushes_at = Vec::with_capacity(lb_count);
        if let Some(arr) = deref(s, info, ClipInfo::leafbrushNodes)? {
            for i in 0..lb_count {
                let n = arr.shifted_by(i * cLeafBrushNode_s::SIZE);
                let data = n.shifted_by(cLeafBrushNode_s::data);
                let leaf_solid_total = s.short_from_bytes(n, cLeafBrushNode_s::leafBrushCount)?;
                let mut brushes = Vec::new();
                let mut child_offset = [0u16; 2];
                let (mut dist, mut range) = (0.0, 0.0);
                let mut brushes_at = None;
                if leaf_solid_total <= 0 {
                    let o = cLeafBrushNodeChildren_t::childOffset;
                    child_offset = [s.ushort_from_bytes(data, o)?, s.ushort_from_bytes(data, o + 2)?];
                    dist = s.float_from_bytes(data, cLeafBrushNodeChildren_t::dist)?;
                    range = s.float_from_bytes(data, cLeafBrushNodeChildren_t::range)?;
                } else if let Some(b) = deref(s, data, cLeafBrushNodeLeaf_t::brushes)? {
                    brushes_at = Some((b.seg, b.offset));
                    brushes = read_run(b, leaf_solid_total as usize, 2, |at| s.ushort_from_bytes(at, 0))?;
                }
                leafbrush_node_brushes_at.push(brushes_at);
                lb_nodes.push(ClipLeafBrushNodeRef {
                    leaf_solid_total,
                    brushes,
                    child_offset,
                    axis: s.u8_at(n, cLeafBrushNode_s::axis)?,
                    contents: s.int_from_bytes(n, cLeafBrushNode_s::contents)?,
                    plane_dist: dist,
                    range,
                });
            }
        }
        let model_count = s.uint_from_bytes(c, clipMap_t::numSubModels)? as usize;
        let mut cmodels = Vec::with_capacity(model_count);
        if let Some(arr) = deref(s, c, clipMap_t::cmodels)? {
            for i in 0..model_count {
                let m = arr.shifted_by(i * cmodel_t::SIZE);
                let shares_map_info = match deref(s, m, cmodel_t::info)? {
                    Some(p) => p == info,
                    None => true,
                };
                cmodels.push(ClipCmodelRef {
                    lo_corner: vec3(s, m, cmodel_t::mins)?,
                    hi_corner: vec3(s, m, cmodel_t::maxs)?,
                    radius: s.float_from_bytes(m, cmodel_t::radius)?,
                    leaf: read_leaf(s, m.shifted_by(cmodel_t::leaf))?,
                    shares_map_info,
                });
            }
        }
        let contents_by_brush = read_table(s, info, ClipInfo::brushContents, brush_total, 4, |at| {
            s.int_from_bytes(at, 0)
        })?;
        let static_count = s.uint_from_bytes(c, clipMap_t::numStaticModels)? as usize;
        let clip_models = read_table(
            s,
            c,
            clipMap_t::staticModelList,
            static_count,
            cStaticModel_s::SIZE,
            |row| {
                let axis = cStaticModel_s::invScaledAxis;
                Ok(ClipStaticModelRef {
                    model: self.asset_at(s, row.shifted_by(cStaticModel_s::xmodel))?,
                    contents: s.uint_from_bytes(row, cStaticModel_s::contents)?,
                    origin: vec3(s, row, cStaticModel_s::origin)?,
                    reverse_scaled_basis: [
                        vec3(s, row, axis)?,
                        vec3(s, row, axis + 12)?,
                        vec3(s, row, axis + 24)?,
                    ],
                    absmin: vec3(s, row, cStaticModel_s::absmin)?,
                    absmax: vec3(s, row, cStaticModel_s::absmax)?,
                })
            },
        )?;
        let mut debris_pieces = Vec::new();
        let dyn_count = s.ushort_from_bytes(c, clipMap_t::dynEntCount)? as usize;
        if let Some(arr) = deref(s, c, clipMap_t::dynEntDefList)? {
            for i in 0..dyn_count {
                let d = arr.shifted_by(i * DynEntityDef::SIZE);
                let kind = s.int_from_bytes(d, DynEntityDef::r#type)?;
                if kind == 0 {
                    continue;
                }
                let model = self.asset_at(s, d.shifted_by(DynEntityDef::xModel))?;
                let destroyed = self.asset_at(s, d.shifted_by(DynEntityDef::destroyedxModel))?;
                let fx = self.asset_at(s, d.shifted_by(DynEntityDef::destroyFx))?;
                let preset = self.asset_at(s, d.shifted_by(DynEntityDef::physPreset))?;
                let pose = d.shifted_by(DynEntityDef::pose);
                debris_pieces.push(ClipDynEntRef {
                    kind,
                    rotation: [
                        s.float_from_bytes(pose, 0)?,
                        s.float_from_bytes(pose, 4)?,
                        s.float_from_bytes(pose, 8)?,
                        s.float_from_bytes(pose, 12)?,
                    ],
                    origin: vec3(s, pose, 16)?,
                    model: self.key_name(model),
                    destroyed_model: self.key_name(destroyed),
                    destroy_effect: self.key_name(fx),
                    destroy_sound: s.uint_from_bytes(d, DynEntityDef::destroySound)?,
                    physics_template: self.key_name(preset),
                    solid_model: s.ushort_from_bytes(d, DynEntityDef::brushModel)?,
                    physics_solid_mesh: s.ushort_from_bytes(d, DynEntityDef::physicsBrushModel)?,
                    health: s.int_from_bytes(d, DynEntityDef::health)?,
                    flags: s.uint_from_bytes(d, DynEntityDef::flags)?,
                    contents: s.uint_from_bytes(d, DynEntityDef::contents)?,
                });
            }
        }
        Ok(ClipRef {
            name: string_field(s, c, clipMap_t::name)?,
            brushes,
            materials,
            vertex_points,
            tri_index_list: tri_list,
            tri_edge_is_walkable,
            partitions: part_list,
            box_trees: tree_list,
            static_model_count: s.uint_from_bytes(c, clipMap_t::numStaticModels)?,
            nodes,
            leaves,
            leafbrush_nodes: lb_nodes,
            leaf_brushes,
            leaf_brushes_at,
            leafbrush_node_brushes_at,
            box_leaf_brush_node: read_leaf(s, c.shifted_by(clipMap_t::box_model).shifted_by(cmodel_t::leaf))?.brush_leaf_link,
            sub_models: cmodels,
            solid_content_flags: contents_by_brush,
            prop_set: clip_models,
            debris_pieces,
        })
    }

    /// The name of a kept model, effect or physics preset.
    fn key_name(&self, key: Option<AssetIdentity>) -> String {
        let Some(key) = key else { return String::new() };
        match key.ty {
            XAssetKind::XModel => self.xmodels.get(key.index).map(|m| m.name.clone()),
            XAssetKind::Fx => self.fx.get(key.index).map(|f| f.name.clone()),
            XAssetKind::PhysPreset => self.phys_presets.get(key.index).map(|p| p.name.clone()),
            _ => None,
        }
        .unwrap_or_default()
    }

    fn read_world(&mut self, s: &ZoneByteReader<'_>, w: RawRef) -> Result<WorldRef> {
        let draw = w.shifted_by(GfxWorld::draw);
        let vertex_count = s.uint_from_bytes(draw, GfxWorldDraw::vertexCount)?;
        let start_extent = s.uint_from_bytes(draw, GfxWorldDraw::vertexDataSize0)? as usize;
        let second_extent = s.uint_from_bytes(draw, GfxWorldDraw::vertexDataSize1)? as usize;
        let vd0 = bytes_field(
            s,
            draw.shifted_by(GfxWorldDraw::vd0),
            GfxWorldVertexData0::data,
            start_extent,
        )?;
        let vd1 = bytes_field(
            s,
            draw.shifted_by(GfxWorldDraw::vd1),
            GfxWorldVertexData1::data,
            second_extent,
        )?;
        let index_count = s.uint_from_bytes(draw, GfxWorldDraw::indexCount)? as usize;
        let index_bytes = bytes_field(s, draw, GfxWorldDraw::indices, index_count * 2)?;
        let indices = le_u16s(&index_bytes);

        let lightmap_total = s.int_from_bytes(draw, GfxWorldDraw::lightmapCount)?.max(0) as usize;
        let lightmaps = read_table(
            s,
            draw,
            GfxWorldDraw::light_atlases,
            lightmap_total,
            GfxLightmapArray::SIZE,
            |entry| {
                Ok([
                    self.asset_at(s, entry.shifted_by(GfxLightmapArray::primary))?,
                    self.asset_at(s, entry.shifted_by(GfxLightmapArray::secondary))?,
                ])
            },
        )?;

        let probe_total = s.uint_from_bytes(draw, GfxWorldDraw::reflectionProbeCount)? as usize;
        let probe_list = read_table(
            s,
            draw,
            GfxWorldDraw::reflectionProbes,
            probe_total,
            GfxReflectionProbe::SIZE,
            |probe| {
                let mut harmonics = [[0.0f32; 4]; 3];
                for (band, row) in harmonics.iter_mut().enumerate() {
                    for (lane, value) in row.iter_mut().enumerate() {
                        *value = s.float_from_bytes(
                            probe,
                            GfxReflectionProbe::lightingSH + band * 16 + lane * 4,
                        )?;
                    }
                }
                Ok(ReflectionProbeRef {
                    origin: vec3(s, probe, GfxReflectionProbe::origin)?,
                    lighting_sh: harmonics,
                    image: self.asset_at(s, probe.shifted_by(GfxReflectionProbe::reflectionImage))?,
                    mip_lod_bias: s.float_from_bytes(probe, GfxReflectionProbe::mipLodBias)?,
                })
            },
        )?;

        let dpvs = w.shifted_by(GfxWorld::dpvs);
        let surface_total = s.uint_from_bytes(w, GfxWorld::surfaceCount)? as usize;
        let surfaces = read_table(
            s,
            dpvs,
            GfxWorldDpvsStatic::surfaces,
            surface_total,
            GfxDrawSurfRec::SIZE,
            |g| {
                let t = g.shifted_by(GfxDrawSurfRec::triangle_list);
                Ok(WorldSurface {
                    lo_corner: vec3(s, t, srfTriangles_t::mins)?,
                    hi_corner: vec3(s, t, srfTriangles_t::maxs)?,
                    bounds: [
                        vec3(s, g, GfxDrawSurfRec::bounds)?,
                        vec3(s, g, GfxDrawSurfRec::bounds + 12)?,
                    ],
                    vertex_data_offset0: s.int_from_bytes(t, srfTriangles_t::vertexDataOffset0)?,
                    vertex_data_offset1: s.int_from_bytes(t, srfTriangles_t::vertexDataOffset1)?,
                    first_vertex: s.int_from_bytes(t, srfTriangles_t::firstVertex)?,
                    vertex_count: s.ushort_from_bytes(t, srfTriangles_t::vertexCount)?,
                    tri_tally: s.ushort_from_bytes(t, srfTriangles_t::triCount)?,
                    base_index: s.int_from_bytes(t, srfTriangles_t::baseIndex)?,
                    material: self.asset_at(s, g.shifted_by(GfxDrawSurfRec::material))?,
                    lightbake_index: s.u8_at(g, GfxDrawSurfRec::lightmapIndex)? as i8,
                    env_probe_index: s.u8_at(g, GfxDrawSurfRec::reflectionProbeIndex)? as i8,
                    key_light_index: s.u8_at(g, GfxDrawSurfRec::primaryLightIndex)? as i8,
                    flags: s.u8_at(g, GfxDrawSurfRec::flags)?,
                })
            },
        )?;

        let smodel_total = s.uint_from_bytes(dpvs, GfxWorldDpvsStatic::smodelCount)? as usize;
        let fixed_props = read_table(
            s,
            dpvs,
            GfxWorldDpvsStatic::smodelDrawInsts,
            smodel_total,
            GfxStaticModelDrawInst::SIZE,
            |d| {
                let p = d.shifted_by(GfxStaticModelDrawInst::placement);
                let axis = p.shifted_by(GfxPackedPlacement::axis);
                let info = d.shifted_by(GfxStaticModelDrawInst::lmapVertexInfo);
                let color_total =
                    s.uint_from_bytes(info, GfxStaticModelLmapVertexInfo::numLmapVertexColors)? as usize;
                let lmap_colors = read_table(
                    s,
                    info,
                    GfxStaticModelLmapVertexInfo::lmapVertexColors,
                    color_total,
                    4,
                    |word| s.uint_from_bytes(word, 0),
                )?;
                let dominant_light = s.u8_at(d, GfxStaticModelDrawInst::primaryLightIndex)?;
                let probe_slot = s.u8_at(d, GfxStaticModelDrawInst::reflectionProbeIndex)?;
                let sh = d.shifted_by(GfxStaticModelDrawInst::lightingSH);
                let mut lighting_sh = [0u16; 12];
                for (slot, coeff) in lighting_sh.iter_mut().enumerate() {
                    *coeff = s.ushort_from_bytes(sh, slot * 2)?;
                }
                Ok(StaticModel {
                    origin: vec3(s, p, GfxPackedPlacement::origin)?,
                    axis: [vec3(s, axis, 0)?, vec3(s, axis, 12)?, vec3(s, axis, 24)?],
                    scale: s.float_from_bytes(p, GfxPackedPlacement::scale)?,
                    model: self.asset_at(s, d.shifted_by(GfxStaticModelDrawInst::model))?,
                    flags: s.int_from_bytes(d, GfxStaticModelDrawInst::flags)?,
                    lmap_colors,
                    lighting_sh,
                    vertex_color_slot: s.ushort_from_bytes(d, GfxStaticModelDrawInst::colorsIndex)?,
                    key_light_index: dominant_light,
                    visibility: s.u8_at(d, GfxStaticModelDrawInst::visibility)?,
                    env_probe_index: probe_slot,
                    hide_dist: s.float_from_bytes(d, GfxStaticModelDrawInst::cullDist)?,
                })
            },
        )?;

        let range = |b: usize, e: usize| -> Result<(u32, u32)> {
            Ok((s.uint_from_bytes(dpvs, b)?, s.uint_from_bytes(dpvs, e)?))
        };
        let dpvs_ranges = DpvsRanges {
            static_facet_tally: s.uint_from_bytes(dpvs, GfxWorldDpvsStatic::staticSurfaceCount)?,
            lit: range(
                GfxWorldDpvsStatic::litSurfsBegin,
                GfxWorldDpvsStatic::litSurfsEnd,
            )?,
            lit_see_through: range(
                GfxWorldDpvsStatic::litTransSurfsBegin,
                GfxWorldDpvsStatic::litTransSurfsEnd,
            )?,
            emissive_opaque: range(
                GfxWorldDpvsStatic::emissiveOpaqueSurfsBegin,
                GfxWorldDpvsStatic::emissiveOpaqueSurfsEnd,
            )?,
            emissive_trans: range(
                GfxWorldDpvsStatic::emissiveTransSurfsBegin,
                GfxWorldDpvsStatic::emissiveTransSurfsEnd,
            )?,
        };
        let brush_model_count = s.uint_from_bytes(w, GfxWorld::modelCount)? as usize;
        let bmodels = read_table(
            s,
            w,
            GfxWorld::models,
            brush_model_count.min(4096),
            GfxBrushModel::SIZE,
            |m| {
                Ok((
                    s.uint_from_bytes(m, GfxBrushModel::startSurfIndex)?,
                    s.uint_from_bytes(m, GfxBrushModel::surfaceCount)?,
                    [
                        vec3(s, m, GfxBrushModel::bounds)?,
                        vec3(s, m, GfxBrushModel::bounds + 12)?,
                    ],
                ))
            },
        )?;
        let sorted_surfs = read_table(
            s,
            dpvs,
            GfxWorldDpvsStatic::sortedSurfIndex,
            dpvs_ranges.static_facet_tally as usize,
            2,
            |entry| s.ushort_from_bytes(entry, 0),
        )?;
        let cell_total = s.uint_from_bytes(
            w.shifted_by(GfxWorld::dpvsPlanes),
            GfxWorldDpvsPlanes::cellCount,
        )? as usize;
        let (cell_bounds, cells): (Vec<CellRef>, Vec<Vec<AabbTreeRef>>) =
            match deref(s, w, GfxWorld::cells)? {
                None => (Vec::new(), Vec::new()),
                Some(cell_base) => read_run(cell_base, cell_total, GfxCell::SIZE, |cell| {
                    let portal_total = s.int_from_bytes(cell, GfxCell::portalCount)?.max(0) as usize;
                    let kept = read_table(
                        s,
                        cell,
                        GfxCell::portals,
                        portal_total,
                        GfxPortal::SIZE,
                        |p| {
                            // The neighbour is a pointer into the cells array.
                            let Some(to) = deref(s, p, GfxPortal::cell)? else {
                                return Ok(None);
                            };
                            let delta = to.offset.wrapping_sub(cell_base.offset) as usize;
                            let in_array = to.seg == cell_base.seg
                                && delta % GfxCell::SIZE == 0
                                && delta / GfxCell::SIZE < cell_total;
                            if !in_array {
                                return Ok(None);
                            }
                            let coeffs = p.shifted_by(GfxPortal::plane + DpvsPlane::coeffs);
                            let corner_total = s.u8_at(p, GfxPortal::vertexCount)? as usize;
                            Ok(Some(PortalRef {
                                plane: [
                                    s.float_from_bytes(coeffs, 0)?,
                                    s.float_from_bytes(coeffs, 4)?,
                                    s.float_from_bytes(coeffs, 8)?,
                                    s.float_from_bytes(coeffs, 12)?,
                                ],
                                cell: (delta / GfxCell::SIZE) as u16,
                                vertices: read_table(
                                    s,
                                    p,
                                    GfxPortal::vertices,
                                    corner_total,
                                    12,
                                    |corner| vec3(s, corner, 0),
                                )?,
                                hull_axes_dir: [
                                    vec3(s, p, GfxPortal::hullAxis)?,
                                    vec3(s, p, GfxPortal::hullAxis + 12)?,
                                ],
                            }))
                        },
                    )?;
                    let bounds = CellRef {
                        lo_corner: vec3(s, cell, GfxCell::mins)?,
                        hi_corner: vec3(s, cell, GfxCell::maxs)?,
                        portals: kept.into_iter().flatten().collect(),
                    };
                    let tree_total = s.int_from_bytes(cell, GfxCell::aabbTreeCount)?.max(0) as usize;
                    let trees = read_table(
                        s,
                        cell,
                        GfxCell::aabbTree,
                        tree_total,
                        GfxAabbTree::SIZE,
                        |n| {
                            let model_total =
                                usize::from(s.ushort_from_bytes(n, GfxAabbTree::smodelIndexCount)?);
                            Ok(AabbTreeRef {
                                lo_corner: vec3(s, n, GfxAabbTree::mins)?,
                                hi_corner: vec3(s, n, GfxAabbTree::maxs)?,
                                sub_element_total: s.ushort_from_bytes(n, GfxAabbTree::childCount)?,
                                facet_tally: s.ushort_from_bytes(n, GfxAabbTree::surfaceCount)?,
                                start_surf_index: s.ushort_from_bytes(n, GfxAabbTree::startSurfIndex)?,
                                prop_indexes: read_table(
                                    s,
                                    n,
                                    GfxAabbTree::smodelIndexes,
                                    model_total,
                                    2,
                                    |slot| s.ushort_from_bytes(slot, 0),
                                )?,
                                sub_elements_start: s.int_from_bytes(n, GfxAabbTree::childrenOffset)?,
                            })
                        },
                    )?;
                    Ok((bounds, trees))
                })?
                .into_iter()
                .unzip(),
            };
        let plane_total = s.uint_from_bytes(w, GfxWorld::planeCount)? as usize;
        let plane_list = read_table(
            s,
            w.shifted_by(GfxWorld::dpvsPlanes),
            GfxWorldDpvsPlanes::planes,
            plane_total,
            cplane_s::SIZE,
            |p| {
                let [facing_x, facing_y, facing_z] = vec3(s, p, cplane_s::normal)?;
                Ok([facing_x, facing_y, facing_z, s.float_from_bytes(p, cplane_s::dist)?])
            },
        )?;
        let dpvs_nodes = read_table(
            s,
            w.shifted_by(GfxWorld::dpvsPlanes),
            GfxWorldDpvsPlanes::nodes,
            s.uint_from_bytes(w, GfxWorld::nodeCount)? as usize,
            2,
            |slot| s.ushort_from_bytes(slot, 0),
        )?;

        Ok(WorldRef {
            name: string_field(s, w, GfxWorld::name)?,
            lo_corner: vec3(s, w, GfxWorld::mins)?,
            hi_corner: vec3(s, w, GfxWorld::maxs)?,
            dpvs: dpvs_ranges,
            sorted_facet_index: sorted_surfs,
            solid_models: bmodels,
            cells,
            dpvs_planes: plane_list,
            dpvs_nodes,
            cell_bounds,
            primary_light_count: s.uint_from_bytes(w, GfxWorld::primaryLightCount)?,
            init_fog: Some(read_world_fog(
                s,
                w.shifted_by(GfxWorld::sunParse + SunLightParseParams::initWorldFog),
            )?),
            init_sun_raw: {
                let base = GfxWorld::sunParse + SunLightParseParams::initWorldSun;
                let mut v = Vec::with_capacity(21);
                for i in 0..21 {
                    v.push(s.uint_from_bytes(w, base + i * 4)?);
                }
                v
            },
            fog_volumes: {
                let n = s.uint_from_bytes(w, GfxWorld::worldFogVolumeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::worldFogVolumes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxWorldFogVolume::SIZE);
                        v.push((
                            vec3(s, e, GfxWorldFogVolume::mins)?,
                            vec3(s, e, GfxWorldFogVolume::maxs)?,
                            s.uint_from_bytes(e, GfxWorldFogVolume::control)?,
                            s.uint_from_bytes(e, GfxWorldFogVolume::controlEx)?,
                            read_world_fog(s, e.shifted_by(GfxWorldFogVolume::volumeWorldFog))?,
                        ));
                    }
                }
                v
            },
            fog_volume_planes: {
                let n = s.uint_from_bytes(w, GfxWorld::worldFogVolumePlaneCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::worldFogVolumePlanes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxVolumePlane::SIZE);
                        v.push([
                            s.float_from_bytes(e, 0)?,
                            s.float_from_bytes(e, 4)?,
                            s.float_from_bytes(e, 8)?,
                            s.float_from_bytes(e, 12)?,
                        ]);
                    }
                }
                v
            },
            fog_modifier_volumes: {
                let n = s.uint_from_bytes(w, GfxWorld::worldFogModifierVolumeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::worldFogModifierVolumes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxWorldFogModifierVolume::SIZE);
                        v.push((
                            s.uint_from_bytes(e, GfxWorldFogModifierVolume::control)?,
                            [
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::transitionTime)?,
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::depthScale)?,
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::heightScale)?,
                            ],
                            [
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::colorAdjust)?,
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::colorAdjust + 4)?,
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::colorAdjust + 8)?,
                                s.float_from_bytes(e, GfxWorldFogModifierVolume::colorAdjust + 12)?,
                            ],
                        ));
                    }
                }
                v
            },
            fog_modifier_volume_planes: {
                let n = s.uint_from_bytes(w, GfxWorld::worldFogModifierVolumePlaneCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::worldFogModifierVolumePlanes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxVolumePlane::SIZE);
                        v.push([
                            s.float_from_bytes(e, 0)?,
                            s.float_from_bytes(e, 4)?,
                            s.float_from_bytes(e, 8)?,
                            s.float_from_bytes(e, 12)?,
                        ]);
                    }
                }
                v
            },
            vertex_count,
            vd0,
            vd1,
            indices,
            surfaces,
            prop_set: fixed_props,
            light_atlases: lightmaps,
            probe_list,
            sky_box_model: string_field(s, w, GfxWorld::skyBoxModel)?,
            lut_material: self.asset_at(s, w.shifted_by(GfxWorld::lutMaterial))?,
            lut_volumes: {
                let n = s.uint_from_bytes(w, GfxWorld::lutVolumeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::lutVolumes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxLutVolume::SIZE);
                        v.push((
                            vec3(s, e, GfxLutVolume::mins)?,
                            vec3(s, e, GfxLutVolume::maxs)?,
                            s.uint_from_bytes(e, GfxLutVolume::control)?,
                            s.uint_from_bytes(e, GfxLutVolume::lutTransitionTime)?,
                            s.uint_from_bytes(e, GfxLutVolume::lutIndex)?,
                        ));
                    }
                }
                v
            },
            hero_lights: {
                let n = s.uint_from_bytes(w, GfxWorld::heroLightCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::heroLights)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxHeroLight::SIZE);
                        v.push(HeroLightRef {
                            ty: s.u8_at(e, GfxHeroLight::r#type)?,
                            tint_rgb: vec3(s, e, GfxHeroLight::color)?,
                            heading: vec3(s, e, GfxHeroLight::dir)?,
                            place_at: vec3(s, e, GfxHeroLight::origin)?,
                            reach: s.float_from_bytes(e, GfxHeroLight::radius)?,
                            cos_half_fov_outer: s.float_from_bytes(e, GfxHeroLight::cosHalfFovOuter)?,
                            cos_half_fov_inner: s.float_from_bytes(e, GfxHeroLight::cosHalfFovInner)?,
                            exponent: s.int_from_bytes(e, GfxHeroLight::exponent)?,
                        });
                    }
                }
                v
            },
            hero_light_tree: {
                let n = s.uint_from_bytes(w, GfxWorld::heroLightTreeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::heroLightTree)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxHeroLightTree::SIZE);
                        v.push(HeroLightTreeRef {
                            lo_corner: vec3(s, e, GfxHeroLightTree::mins)?,
                            hi_corner: vec3(s, e, GfxHeroLightTree::maxs)?,
                            left_node: s.int_from_bytes(e, GfxHeroLightTree::leftNode)?,
                            right_node: s.int_from_bytes(e, GfxHeroLightTree::rightNode)?,
                        });
                    }
                }
                v
            },
            sky_dyn_intensity: {
                let d = w.shifted_by(GfxWorld::skyDynIntensity);
                [
                    s.float_from_bytes(d, GfxSkyDynamicIntensity::angle0)?,
                    s.float_from_bytes(d, GfxSkyDynamicIntensity::angle1)?,
                    s.float_from_bytes(d, GfxSkyDynamicIntensity::factor0)?,
                    s.float_from_bytes(d, GfxSkyDynamicIntensity::factor1)?,
                ]
            },
            probe_grid: {
                let g = w.shifted_by(GfxWorld::lightGrid);
                let u16x3 = |off: usize| -> Result<[u16; 3]> {
                    Ok([
                        s.ushort_from_bytes(g, off)?,
                        s.ushort_from_bytes(g, off + 2)?,
                        s.ushort_from_bytes(g, off + 4)?,
                    ])
                };
                let mins = u16x3(GfxLightGrid::mins)?;
                let maxs = u16x3(GfxLightGrid::maxs)?;
                let line_direction = s.uint_from_bytes(g, GfxLightGrid::rowAxis)?;
                let rows = if line_direction < 3 {
                    usize::from(maxs[line_direction as usize].saturating_sub(mins[line_direction as usize])) + 1
                } else {
                    0
                };
                let raw_len = s.uint_from_bytes(g, GfxLightGrid::rawRowDataSize)? as usize;
                let entry_tally = s.uint_from_bytes(g, GfxLightGrid::entryCount)? as usize;
                let tint_key_total = s.uint_from_bytes(g, GfxLightGrid::colorCount)?;
                let grid = LightGridRef {
                    sunlight_lamp_slot: s.uint_from_bytes(g, GfxLightGrid::sunPrimaryLightIndex)?,
                    lo_corner: mins,
                    hi_corner: maxs,
                    offset: s.float_from_bytes(g, GfxLightGrid::offset)?,
                    line_direction,
                    column_direction: s.uint_from_bytes(g, GfxLightGrid::colAxis)?,
                    line_bytes_origin: bytes_field(s, g, GfxLightGrid::rowDataStart, rows * 2)?,
                    unparsed_line_bytes: bytes_field(s, g, GfxLightGrid::rawRowData, raw_len)?,
                    entries: bytes_field(s, g, GfxLightGrid::entries, entry_tally * 4)?,
                    colors: bytes_field(
                        s,
                        g,
                        GfxLightGrid::colors,
                        tint_key_total as usize * GfxCompressedLightGridColors::SIZE,
                    )?,
                    tint_key_total,
                    coeffs: {
                        let n = s.uint_from_bytes(g, GfxLightGrid::coeffCount)? as usize;
                        bytes_field(
                            s,
                            g,
                            GfxLightGrid::coeffs,
                            n * GfxCompressedLightGridCoeffs::SIZE,
                        )?
                    },
                    coeff_count: s.uint_from_bytes(g, GfxLightGrid::coeffCount)?,
                    sky_volumes: {
                        let n = s.uint_from_bytes(g, GfxLightGrid::skyGridVolumeCount)? as usize;
                        let mut v = Vec::with_capacity(n);
                        if let Some(arr) = deref(s, g, GfxLightGrid::skyGridVolumes)? {
                            for i in 0..n {
                                let e = arr.shifted_by(i * GfxSkyGridVolume::SIZE);
                                v.push(SkyGridVolumeRef {
                                    lo_corner: vec3(s, e, GfxSkyGridVolume::mins)?,
                                    hi_corner: vec3(s, e, GfxSkyGridVolume::maxs)?,
                                    illum_origin: vec3(
                                        s,
                                        e,
                                        GfxSkyGridVolume::lightingOrigin,
                                    )?,
                                    vertex_color_slot: s.ushort_from_bytes(e, GfxSkyGridVolume::colorsIndex)?,
                                    key_light_index: s.u8_at(
                                        e,
                                        GfxSkyGridVolume::primaryLightIndex,
                                    )?,
                                    visibility: s.u8_at(e, GfxSkyGridVolume::visibility)?,
                                });
                            }
                        }
                        v
                    },
                };
                Some(grid)
            },
            init_sun_exposure: s.float_from_bytes(
                w,
                GfxWorld::sunParse
                    + SunLightParseParams::initWorldSun
                    + GfxWorldSun::exposure,
            )?,
            exposure_volumes: {
                let n = s.uint_from_bytes(w, GfxWorld::exposureVolumeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::exposureVolumes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxExposureVolume::SIZE);
                        v.push([
                            s.float_from_bytes(e, GfxExposureVolume::exposure)?,
                            s.float_from_bytes(e, GfxExposureVolume::luminanceIncreaseScale)?,
                            s.float_from_bytes(e, GfxExposureVolume::luminanceDecreaseScale)?,
                            s.float_from_bytes(e, GfxExposureVolume::featherRange)?,
                        ]);
                    }
                }
                v
            },
            exposure_volume_controls: {
                let n = s.uint_from_bytes(w, GfxWorld::exposureVolumeCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::exposureVolumes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxExposureVolume::SIZE);
                        v.push(s.uint_from_bytes(e, GfxExposureVolume::control)?);
                    }
                }
                v
            },
            exposure_volume_planes: {
                let n = s.uint_from_bytes(w, GfxWorld::exposureVolumePlaneCount)? as usize;
                let mut v = Vec::with_capacity(n);
                if let Some(arr) = deref(s, w, GfxWorld::exposureVolumePlanes)? {
                    for i in 0..n {
                        let e = arr.shifted_by(i * GfxVolumePlane::SIZE);
                        v.push([
                            s.float_from_bytes(e, 0)?,
                            s.float_from_bytes(e, 4)?,
                            s.float_from_bytes(e, 8)?,
                            s.float_from_bytes(e, 12)?,
                        ]);
                    }
                }
                v
            },
            sun: match deref(s, w, GfxWorld::sunLight)? {
                Some(light) => Some(SunRef {
                    color: vec3(s, light, GfxLight::color)?,
                    heading: vec3(s, light, GfxLight::dir)?,
                    diffuse_color: [
                        s.float_from_bytes(light, GfxLight::diffuseColor)?,
                        s.float_from_bytes(light, GfxLight::diffuseColor + 4)?,
                        s.float_from_bytes(light, GfxLight::diffuseColor + 8)?,
                        s.float_from_bytes(light, GfxLight::diffuseColor + 12)?,
                    ],
                }),
                None => None,
            },
        })
    }
}

impl WalkSink for ZoneCapture {
    fn asset_loaded(&mut self, s: &ZoneByteReader<'_>, loaded: Loaded) -> Result<()> {
        *self.loads.entry(loaded.ty).or_default() += 1;
        self.parent_started = loaded.started;
        let body = loaded.body;
        let key = match loaded.ty {
            XAssetKind::Image => Some(push_indexed(&mut self.images, read_image(s, body)?)),
            XAssetKind::Material => self
                .read_material(s, body)
                .map(|m| push_indexed(&mut self.materials, m))
                .map(Some)?,
            XAssetKind::TechniqueSet => self
                .read_technique_set(s, body)
                .map(|t| push_indexed(&mut self.technique_sets, t))
                .map(Some)?,
            XAssetKind::XModel => {
                let model = self.read_xmodel(s, body)?;
                self.xmodels.push(model);
                Some(self.xmodels.len() - 1)
            }
            XAssetKind::GfxWorld => {
                self.world = Some(self.read_world(s, body)?);
                None
            }
            XAssetKind::ClipMap | XAssetKind::ClipMapPvs => {
                self.clip = Some(self.read_clip(s, body)?);
                None
            }
            XAssetKind::XAnimParts => {
                let numframes = s.ushort_from_bytes(body, XAnimParts::numframes)?;
                let mut trans_deltas = Vec::new();
                if let Some(dp) = deref(s, body, XAnimParts::deltaPart)?
                    && let Some(t) = deref(s, dp, XAnimDeltaPart::shift_offset)?
                {
                    let size = usize::from(s.ushort_from_bytes(t, XAnimPartTrans::size)?);
                    let small = s.u8_at(t, XAnimPartTrans::smallTrans)? != 0;
                    let u = t.shifted_by(XAnimPartTrans::u);
                    if size == 0 {
                        trans_deltas.push((0, vec3(s, u, XAnimPartTransData::frame0)?));
                    } else {
                        let mins = vec3(s, u, XAnimPartTransFrames::mins)?;
                        let span = vec3(s, u, XAnimPartTransFrames::size)?;
                        let keys = size + 1;
                        let frames = deref(s, u, XAnimPartTransFrames::frames)?;
                        let idx = u.shifted_by(XAnimPartTransFrames::indices);
                        for k in 0..keys {
                            let frame = if keys == usize::from(numframes) + 1 {
                                k as u16
                            } else if numframes >= 256 {
                                s.ushort_from_bytes(idx, k * 2)?
                            } else {
                                u16::from(s.u8_at(idx, k)?)
                            };
                            let Some(f) = frames else { break };
                            // mins + size * the raw quantized value (size is
                            // the step per unit, as in the IW channels).
                            let v = |axis: usize| -> Result<f32> {
                                Ok(if small {
                                    f32::from(s.u8_at(f, k * 3 + axis)?)
                                } else {
                                    f32::from(s.ushort_from_bytes(f, (k * 3 + axis) * 2)?)
                                })
                            };
                            trans_deltas.push((
                                frame,
                                [
                                    mins[0] + span[0] * v(0)?,
                                    mins[1] + span[1] * v(1)?,
                                    mins[2] + span[2] * v(2)?,
                                ],
                            ));
                        }
                    }
                }
                // The root's turn: the same keys as the translation (key
                // count less one, one value or a frames table, frame indices
                // unless every frame has a key), each value a quaternion in
                // shorts (32767 = 1): `quat` all four, `quat2` z and w.
                let mut delta_quat = Vec::new();
                if let Some(dp) = deref(s, body, XAnimParts::deltaPart)? {
                    let full = deref(s, dp, XAnimDeltaPart::quat)?;
                    let part =
                        full.map(|q| (q, 4usize))
                            .or(deref(s, dp, XAnimDeltaPart::quat2)?.map(|q| (q, 2usize)));
                    if let Some((q, n)) = part {
                        let size = usize::from(s.ushort_from_bytes(q, XAnimDeltaPartQuat::size)?);
                        let u = q.shifted_by(XAnimDeltaPartQuat::u);
                        let quat = |at: RawRef, k: usize| -> Result<[f32; 4]> {
                            let c = |i: usize| -> Result<f32> {
                                Ok(f32::from(s.short_from_bytes(at, (k * n + i) * 2)?) / 32767.0)
                            };
                            Ok(if n == 4 {
                                [c(0)?, c(1)?, c(2)?, c(3)?]
                            } else {
                                [0.0, 0.0, c(0)?, c(1)?]
                            })
                        };
                        if size == 0 {
                            delta_quat.push((0, quat(u, 0)?));
                        } else if let Some(frames) =
                            deref(s, u, XAnimDeltaPartQuatDataFrames::frames)?
                        {
                            let keys = size + 1;
                            let idx = u.shifted_by(XAnimDeltaPartQuatDataFrames::indices);
                            for k in 0..keys {
                                let frame = if keys == usize::from(numframes) + 1 {
                                    k as u16
                                } else if numframes >= 256 {
                                    s.ushort_from_bytes(idx, k * 2)?
                                } else {
                                    u16::from(s.u8_at(idx, k)?)
                                };
                                delta_quat.push((frame, quat(frames, k)?));
                            }
                        }
                    }
                }
                let mut bone_tally = [0u8; 10];
                for (k, c) in bone_tally.iter_mut().enumerate() {
                    *c = s.u8_at(body, XAnimParts::boneCount + k)?;
                }
                let names = read_table(
                    s,
                    body,
                    XAnimParts::names,
                    usize::from(bone_tally[9]),
                    2,
                    |at| Ok(self.string_for(s.ushort_from_bytes(at, 0)?)),
                )?;
                let notify_count = usize::from(s.u8_at(body, XAnimParts::notifyCount)?);
                let mut notifies = Vec::with_capacity(notify_count);
                if let Some(arr) = deref(s, body, XAnimParts::notify)? {
                    for k in 0..notify_count {
                        let e = arr.shifted_by(k * XAnimNotifyInfo::SIZE);
                        let id = s.ushort_from_bytes(e, XAnimNotifyInfo::name)?;
                        notifies.push((
                            self.strings.get(usize::from(id)).cloned().unwrap_or_default(),
                            s.float_from_bytes(e, XAnimNotifyInfo::time)?,
                        ));
                    }
                }
                let bytes = |field: usize, n: usize| -> Result<Vec<u8>> {
                    Ok(match deref(s, body, field)? {
                        Some(p) if n > 0 => s.byte_window_from(p, 0, n)?.to_vec(),
                        _ => Vec::new(),
                    })
                };
                let shorts = |field: usize, n: usize| -> Result<Vec<u16>> {
                    let raw = bytes(field, n * 2)?;
                    Ok(le_u16s(&raw))
                };
                let ints = |field: usize, n: usize| -> Result<Vec<u32>> {
                    let raw = bytes(field, n * 4)?;
                    Ok(le_u32s(&raw))
                };
                let payload_u8 = bytes(
                    XAnimParts::dataByte,
                    usize::from(s.ushort_from_bytes(body, XAnimParts::dataByteCount)?),
                )?;
                let payload_i16 = shorts(
                    XAnimParts::dataShort,
                    usize::from(s.ushort_from_bytes(body, XAnimParts::dataShortCount)?),
                )?;
                let payload_i32 = ints(
                    XAnimParts::dataInt,
                    usize::from(s.ushort_from_bytes(body, XAnimParts::dataIntCount)?),
                )?;
                let rand_shorts = shorts(
                    XAnimParts::randomDataShort,
                    s.uint_from_bytes(body, XAnimParts::randomDataShortCount)? as usize,
                )?;
                let noise_octet = bytes(
                    XAnimParts::randomDataByte,
                    usize::from(s.ushort_from_bytes(body, XAnimParts::randomDataByteCount)?),
                )?;
                let rand_ints = ints(
                    XAnimParts::randomDataInt,
                    usize::from(s.ushort_from_bytes(body, XAnimParts::randomDataIntCount)?),
                )?;
                let index_total = s.uint_from_bytes(body, XAnimParts::indexCount)? as usize;
                let indices = if numframes >= 256 {
                    shorts(XAnimParts::indices, index_total)?
                } else {
                    let narrow = bytes(XAnimParts::indices, index_total)?;
                    narrow.iter().map(|&b| u16::from(b)).collect()
                };
                self.animation_refs.push(XAnimRef {
                    name: string_field(s, body, XAnimParts::name)?,
                    frame_total: numframes,
                    frame_rate_hz: s.float_from_bytes(body, XAnimParts::framerate)?,
                    looping: s.u8_at(body, XAnimParts::bLoop)? != 0,
                    delta: s.u8_at(body, XAnimParts::bDelta)? != 0,
                    delta_trans: trans_deltas,
                    loop_entry_time: s.float_from_bytes(body, XAnimParts::loopEntryTime)?,
                    primed_length: s.float_from_bytes(body, XAnimParts::primedLength)?,
                    asset_type: s.u8_at(body, XAnimParts::assetType)?,
                    joint_total: bone_tally,
                    names,
                    notifies,
                    payload_u8,
                    payload_i16,
                    payload_i32,
                    noise_octet,
                    noise_word16: rand_shorts,
                    noise_word32: rand_ints,
                    indices,
                    delta_quat,
                });
                None
            }
            XAssetKind::RawFile => {
                let len = s.uint_from_bytes(body, RawFile::len)? as usize;
                let name = string_field(s, body, RawFile::name)?;
                let bytes = bytes_field(s, body, RawFile::buffer, len)?;
                self.raw_files.push((name, bytes));
                None
            }
            XAssetKind::GameWorldMp => {
                // PathData is inline in GameWorldMp (at `path`).
                let pd = body.shifted_by(GameWorldMp::path);
                let count = s.uint_from_bytes(pd, PathData::nodeCount)? as usize;
                if let Some(nodes) = deref(s, pd, PathData::nodes)? {
                    let sstr = |i: u16| -> String {
                        self.strings.get(usize::from(i)).cloned().unwrap_or_default()
                    };
                    for i in 0..count {
                        let n = nodes.shifted_by(i * pathnode_t::SIZE);
                        let c = n.shifted_by(pathnode_t::constant);
                        let link_count = s.short_from_bytes(c, pathnode_constant_t::totalLinkCount)?.max(0) as usize;
                        let mut links = Vec::with_capacity(link_count);
                        if let Some(lp) = deref(s, c, pathnode_constant_t::Links)? {
                            for k in 0..link_count {
                                let lk = lp.shifted_by(k * pathlink_s::SIZE);
                                links.push((
                                    s.ushort_from_bytes(lk, pathlink_s::nodeNum)?,
                                    s.float_from_bytes(lk, pathlink_s::fDist)?,
                                    s.u8_at(lk, pathlink_s::negotiationLink)? != 0,
                                ));
                            }
                        }
                        self.path_nodes.push(PathNodeRef {
                            ty: s.uint_from_bytes(c, pathnode_constant_t::r#type)?,
                            // Four bytes (targetname follows at 8): the
                            // negotiation kinds sit above bit 15 (jump 0x800000).
                            spawnflags: s.uint_from_bytes(c, pathnode_constant_t::spawnflags)?,
                            target_key: sstr(s.ushort_from_bytes(c, pathnode_constant_t::targetname)?),
                            target: sstr(s.ushort_from_bytes(c, pathnode_constant_t::target)?),
                            script_note: sstr(s.ushort_from_bytes(c, pathnode_constant_t::script_noteworthy)?),
                            script_link: sstr(s.ushort_from_bytes(c, pathnode_constant_t::script_linkName)?),
                            animscript: sstr(s.ushort_from_bytes(c, pathnode_constant_t::animscript)?),
                            origin: vec3(s, c, pathnode_constant_t::vOrigin)?,
                            angle: s.float_from_bytes(c, pathnode_constant_t::fAngle)?,
                            radius: s.float_from_bytes(c, pathnode_constant_t::fRadius)?,
                            links,
                        });
                    }
                }
                let vis_len = s.uint_from_bytes(pd, PathData::visBytes)? as usize;
                let smooth_bytes = s.uint_from_bytes(pd, PathData::smoothBytes)? as usize;
                self.path_tables = PathTablesRef {
                    node_count: count as u32,
                    original_count: s.uint_from_bytes(pd, PathData::originalNodeCount)?,
                    vis: bytes_field(s, pd, PathData::pathVis, vis_len)?,
                    smooth: bytes_field(s, pd, PathData::smoothCache, smooth_bytes)?,
                };
                None
            }
            XAssetKind::Localize => {
                let name = string_field(s, body, LocalizeEntry::name)?;
                let value = string_field(s, body, LocalizeEntry::value)?;
                if !name.is_empty() {
                    self.localize.push((name, value));
                }
                None
            }
            XAssetKind::FontIcon => {
                // bo2mp: BO2's button pictures (`^BBUTTON_MOUSE_LEFT^`): each icon's
                // name, picture and its size scales, and the aliases that name
                // one icon for another. Published as two tables so the UI
                // reads them like any BO2 table.
                let entries = s.uint_from_bytes(body, FontIcon::numEntries)? as usize;
                let aliases = s.uint_from_bytes(body, FontIcon::numAliasEntries)? as usize;
                let mut cells = Vec::with_capacity(entries * 6);
                if let Some(arr) = deref(s, body, FontIcon::fontIconEntry)? {
                    for i in 0..entries {
                        let e = arr.shifted_by(i * FontIconEntry::SIZE);
                        // Each entry's picture was loaded through its own slot while
                        // this icon set was read (TEMP addresses are reused, so
                        // the slot is how it is found, not its address).
                        let material = match self.asset_at(s, e.shifted_by(FontIconEntry::fontIconMaterialHandle))? {
                            Some(k) => self.material(k).map(|m| m.name.clone()).unwrap_or_default(),
                            None => String::new(),
                        };
                        cells.push(string_field(s, e.shifted_by(FontIconEntry::fontIconName), FontIconName::string)?);
                        cells.push(s.uint_from_bytes(e.shifted_by(FontIconEntry::fontIconName), FontIconName::hash)?.to_string());
                        cells.push(material);
                        cells.push(s.int_from_bytes(e, FontIconEntry::fontIconSize)?.to_string());
                        cells.push(s.float_from_bytes(e, FontIconEntry::xScale)?.to_string());
                        cells.push(s.float_from_bytes(e, FontIconEntry::yScale)?.to_string());
                    }
                }
                self.string_tables.push(StringTableRef { name: "bo2mp/fonticon.csv".to_owned(), columns: 6, rows: cells.len() / 6, cells });
                let mut cells = Vec::with_capacity(aliases * 2);
                if let Some(arr) = deref(s, body, FontIcon::fontIconAlias)? {
                    for i in 0..aliases {
                        let a = arr.shifted_by(i * FontIconAlias::SIZE);
                        cells.push(s.uint_from_bytes(a, FontIconAlias::aliasHash)?.to_string());
                        cells.push(s.uint_from_bytes(a, FontIconAlias::buttonHash)?.to_string());
                    }
                }
                self.string_tables.push(StringTableRef { name: "bo2mp/fonticonalias.csv".to_owned(), columns: 2, rows: cells.len() / 2, cells });
                None
            }
            XAssetKind::StringTable => {
                let name = string_field(s, body, StringTable::name)?;
                let columns = s.uint_from_bytes(body, StringTable::columnCount)? as usize;
                let rows = s.uint_from_bytes(body, StringTable::rowCount)? as usize;
                let mut cells = Vec::with_capacity(columns * rows);
                if let Some(arr) = deref(s, body, StringTable::values)? {
                    for i in 0..columns * rows {
                        cells.push(string_field(s, arr.shifted_by(i * StringTableCell::SIZE), StringTableCell::string)?);
                    }
                }
                self.string_tables.push(StringTableRef {
                    name,
                    columns,
                    rows,
                    cells,
                });
                None
            }
            XAssetKind::ZBarrier => {
                // Each piece's model and its opening and closing clips: the
                // engine builds the pieces from these, not from the map
                // entity's keys (Origins' boxes name Mob of the Dead's clips,
                // which its zones lack; its type names its own).
                let n = (s.uint_from_bytes(body, ZBarrierDef::numBoardsInBarrier)? as usize).min(6);
                let mut looks = Vec::with_capacity(n);
                for i in 0..n {
                    let b = body.shifted_by(ZBarrierDef::boards + i * ZBarrierBoard::SIZE);
                    looks.push((
                        self.asset_name_at(s, b.shifted_by(ZBarrierBoard::pBoardModel), XAssetKind::XModel)?,
                        string_field(s, b, ZBarrierBoard::pTearAnim)?,
                        string_field(s, b, ZBarrierBoard::pBoardAnim)?,
                    ));
                }
                let sl = |off: usize| -> Result<String> {
                    Ok(self.script_string(s.ushort_from_bytes(body, off)?).to_owned())
                };
                let n = (s.uint_from_bytes(body, ZBarrierDef::numBoardsInBarrier)? as usize).min(6);
                let mut boards = Vec::with_capacity(n);
                for i in 0..n {
                    let b = ZBarrierDef::boards + i * ZBarrierBoard::SIZE;
                    boards.push((
                        sl(b + ZBarrierBoard::zombieBoardTearStateName)?,
                        sl(b + ZBarrierBoard::zombieBoardTearSubStateName)?,
                    ));
                }
                let def = ZBarrierDefRef {
                    name: string_field(s, body, ZBarrierDef::name)?,
                    taunts: s.uint_from_bytes(body, ZBarrierDef::taunts)? != 0,
                    reach_through: s.uint_from_bytes(body, ZBarrierDef::reachThroughAttacks)? != 0,
                    auto_hide_open_pieces: s.uint_from_bytes(body, ZBarrierDef::autoHideOpenPieces)? != 0,
                    taunt_state: sl(ZBarrierDef::zombieTauntAnimState)?,
                    reach_through_state: sl(ZBarrierDef::zombieReachThroughAnimState)?,
                    num_attack_slots: s.uint_from_bytes(body, ZBarrierDef::numAttackSlots)?,
                    attack_spot_horz_offset: s.float_from_bytes(body, ZBarrierDef::attackSpotHorzOffset)?,
                    boards,
                    looks,
                };
                self.zbarriers.push(def);
                None
            }
            // Compiled scripts are kept with the rawfiles, named `script:`.
            XAssetKind::ScriptParseTree => {
                let len = s.uint_from_bytes(body, ScriptParseTree::len)? as usize;
                let name = string_field(s, body, ScriptParseTree::name)?;
                let bytes = bytes_field(s, body, ScriptParseTree::buffer, len)?;
                self.raw_files.push((format!("script:{name}"), bytes));
                None
            }
            XAssetKind::ComWorld => {
                let n = s.uint_from_bytes(body, ComWorld::primaryLightCount)? as usize;
                if let Some(arr) = deref(s, body, ComWorld::primaryLights)? {
                    for i in 0..n {
                        let p = arr.shifted_by(i * ComPrimaryLight::SIZE);
                        let v4 = |off: usize| -> Result<[f32; 4]> {
                            let lane = |n: usize| s.float_from_bytes(p, off + n * 4);
                            Ok([lane(0)?, lane(1)?, lane(2)?, lane(3)?])
                        };
                        self.key_lights.push(PrimaryLightRef {
                            ty: s.u8_at(p, ComPrimaryLight::r#type)?,
                            supports_depth_shade: s.u8_at(p, ComPrimaryLight::canUseShadowMap)?,
                            falloff_pow: s.u8_at(p, ComPrimaryLight::exponent)?,
                            priority: s.u8_at(p, ComPrimaryLight::priority)?,
                            hide_dist: s.short_from_bytes(p, ComPrimaryLight::cullDist)?,
                            color: vec3(s, p, ComPrimaryLight::color)?,
                            heading: vec3(s, p, ComPrimaryLight::dir)?,
                            origin: vec3(s, p, ComPrimaryLight::origin)?,
                            radius: s.float_from_bytes(p, ComPrimaryLight::radius)?,
                            cos_half_fov_outer: s.float_from_bytes(p, ComPrimaryLight::cosHalfFovOuter)?,
                            cos_half_fov_inner: s.float_from_bytes(p, ComPrimaryLight::cosHalfFovInner)?,
                            d_attenuation: s.float_from_bytes(p, ComPrimaryLight::dAttenuation)?,
                            roundness: s.float_from_bytes(p, ComPrimaryLight::roundness)?,
                            diffuse_color: v4(ComPrimaryLight::diffuseColor)?,
                            falloff: v4(ComPrimaryLight::falloff)?,
                            angle: v4(ComPrimaryLight::angle)?,
                            pair_ab_vs_b: v4(ComPrimaryLight::aAbB)?,
                        });
                    }
                }
                None
            }
            // bo2zm M2: guns, effects, sounds.
            XAssetKind::Weapon => {
                let weapon = self.read_weapon(s, body)?;
                self.weapons.push(weapon);
                None
            }
            XAssetKind::Attachment => {
                let a = self.read_attachment(s, body)?;
                self.attachments.push(a);
                None
            }
            XAssetKind::AttachmentUnique => {
                let a = self.read_attachment_unique(s, body)?;
                self.attachment_uniques.push(a);
                None
            }
            XAssetKind::WeaponCamo => {
                let camo = self.read_weapon_camo(s, body)?;
                self.weapon_camos.push(camo);
                Some(self.weapon_camos.len() - 1)
            }
            XAssetKind::Font => {
                let font = self.read_font(s, body)?;
                self.fonts.push(font);
                None
            }
            XAssetKind::Fx => {
                let fx = self.read_fx(s, body)?;
                self.fx.push(fx);
                Some(self.fx.len() - 1)
            }
            XAssetKind::Tracer => {
                let tracer = self.read_tracer(s, body)?;
                self.tracers.push(tracer);
                Some(self.tracers.len() - 1)
            }
            XAssetKind::ImpactFx => {
                let table = self.read_impact_table(s, body)?;
                self.impact_tables.push(table);
                None
            }
            XAssetKind::FootstepTable => {
                let table = self.read_footstep_table(s, body)?;
                self.footstep_tables.push(table);
                None
            }
            XAssetKind::FootstepFxTable => {
                let table = self.read_footstep_fx_table(s, body)?;
                self.footstep_fx_tables.push(table);
                None
            }
            XAssetKind::Sound => {
                let bank = self.read_sound_bank(s, body)?;
                self.sound_banks.push(bank);
                None
            }
            XAssetKind::SndDriverGlobals => {
                self.snd_globals = Some(self.read_snd_driver_globals(s, body)?);
                None
            }
            XAssetKind::VehicleDef => {
                // Its guns are named, not pointed at (`turretWeapon`,
                // `gunnerWeapon[4]` are strings in BO2's vehicle def).
                let name_at = |off: usize| string_field(s, body, off).unwrap_or_default();
                let gunner_weapons = (0..4)
                    .map(|i| name_at(VehicleDef::gunnerWeapon + i * 4))
                    .filter(|gun| !gun.is_empty())
                    .collect();
                let f = |off: usize| s.float_from_bytes(body, off).unwrap_or(0.0_f32);
                let i = |off: usize| s.int_from_bytes(body, off).unwrap_or(0_i32);
                let v = VehicleDef::nitrousVehParams;
                let tp = VehicleDef::thirdPersonCameraHeight;
                let tv = VehicleDef::turretViewLimits;
                let drive = VehicleDrive {
                    camera_mode: s.int_from_bytes(body, VehicleDef::cameraMode).unwrap_or(0),
                    top_velocity: f(VehicleDef::maxSpeed),
                    max_speed_vertical: f(VehicleDef::maxSpeedVertical),
                    speed_up_rate: f(VehicleDef::speed_up_rate),
                    accel_vertical: f(VehicleDef::accelVertical),
                    reverse_scale: f(v + VehicleParameter::m_reverse_scale),
                    camera_range: f(VehicleDef::thirdPersonCameraRange),
                    camera_height: f(tp + 4),
                    camera_pitch: [
                        f(VehicleDef::thirdPersonCameraMinPitchClamp),
                        f(VehicleDef::thirdPersonCameraMaxPitchClamp),
                    ],
                    camera_fov: f(VehicleDef::cameraFOV),
                    camera_spring: [
                        f(VehicleDef::thirdPersonCameraSpringDistance),
                        f(VehicleDef::thirdPersonCameraSpringTime),
                    ],
                    turret_pitch: [f(tv + 8), f(tv + 12)],
                    third_person_driver: s.int_from_bytes(body, VehicleDef::thirdPersonDriver).unwrap_or(0),
                    buttons: [
                        VehicleDef::moveUpButtonName,
                        VehicleDef::moveDownButtonName,
                        VehicleDef::switchSeatButtonName,
                        VehicleDef::attackButtonName,
                        VehicleDef::attackSecondaryButtonName,
                    ]
                    .map(|o| vehicle_button_command(&name_at(o))),
                    boost_button: vehicle_button_command(&name_at(VehicleDef::boostButtonName)),
                    hand_brake_button: vehicle_button_command(&name_at(VehicleDef::handBrakeButtonName)),
                    boost_accel_mult: s.int_from_bytes(body, VehicleDef::boostAccelMultiplier).unwrap_or(0) as f32,
                    boost_duration: f(VehicleDef::boostDuration),
                    boost_speed_increase: f(VehicleDef::boostSpeedIncrease),
                    steer_angle_max: f(v + VehicleParameter::m_steer_angle_max),
                    steer_speed: f(v + VehicleParameter::m_steer_speed),
                    tire_fric: [
                        f(v + VehicleParameter::m_tire_fric_fwd),
                        f(v + VehicleParameter::m_tire_fric_side),
                        f(v + VehicleParameter::m_tire_fric_brake),
                        f(v + VehicleParameter::m_tire_fric_hand_brake),
                    ],
                    auto_hand_brake_min_speed: f(v + VehicleParameter::m_auto_hand_brake_min_speed),
                    peel_out_max_speed: f(v + VehicleParameter::m_peel_out_max_speed),
                    skid_speed_min: f(VehicleDef::skidSpeedMin),
                    is_nitrous: s.int_from_bytes(body, VehicleDef::isNitrous).unwrap_or(0),
                    wheel_rot_rate: f(VehicleDef::wheelRotRate),
                    max_body_pitch: f(VehicleDef::maxBodyPitch),
                    max_body_roll: f(VehicleDef::maxBodyRoll),
                    suspension_travel: f(VehicleDef::suspensionTravel),
                    wheel_radius: f(v + VehicleParameter::m_wheel_radius),
                    susp_hard_limit: f(v + VehicleParameter::m_susp_hard_limit),
                    susp_min_height: f(v + VehicleParameter::m_susp_min_height),
                    roll_stability: f(v + VehicleParameter::m_roll_stability),
                    pitch_stability: f(v + VehicleParameter::m_pitch_stability),
                    remote_operated: i(VehicleDef::remoteControl),
                    bullet_damage: i(VehicleDef::bulletDamage),
                    armor_piercing_damage: i(VehicleDef::armorPiercingDamage),
                    grenade_damage: i(VehicleDef::grenadeDamage),
                    projectile_damage: i(VehicleDef::projectileDamage),
                    projectile_splash_damage: i(VehicleDef::projectileSplashDamage),
                    heavy_explosive_damage: i(VehicleDef::heavyExplosiveDamage),
                    no_directional_damage: i(VehicleDef::noDirectionalDamage),
                    // `type` is a short; the two bytes after it are padding.
                    vehicle_type: i32::from(s.short_from_bytes(body, VehicleDef::r#type).unwrap_or(0)),
                    collision_damage: f(VehicleDef::collisionDamage),
                    collision_speed: f(VehicleDef::collisionSpeed),
                    heli: {
                        let f2 = |off: usize| [f(off), f(off + 4)];
                        HeliDrive {
                            rot_rate: f(VehicleDef::rotRate),
                            rot_accel: f(VehicleDef::rotAccel),
                            tilt_from_accel: f2(VehicleDef::tiltFromAcceleration),
                            tilt_from_decel: f2(VehicleDef::tiltFromDeceleration),
                            tilt_from_vel: f2(VehicleDef::tiltFromVelocity),
                            tilt_speed: f2(VehicleDef::tiltSpeed),
                            use_collmap: i(VehicleDef::useCollmap),
                            decel: [
                                f(VehicleDef::vehHelicopterDecelerationFwd),
                                f(VehicleDef::vehHelicopterDecelerationSide),
                                f(VehicleDef::vehHelicopterDecelerationUp),
                            ],
                            tilt_from_axes: f(VehicleDef::vehHelicopterTiltFromControllerAxes),
                            tilt_from_fwd_yaw: f(VehicleDef::vehHelicopterTiltFromFwdAndYaw),
                            vel_at_max_tilt: f(VehicleDef::vehHelicopterTiltFromFwdAndYaw_VelAtMaxTilt),
                            tilt_momentum: f(VehicleDef::vehHelicopterTiltMomentum),
                            accel_toward_view: i(VehicleDef::vehHelicopterAccelTwardsView),
                            maintain_height: i(VehicleDef::vehHelicopterMaintainHeight),
                            height_limit: f(VehicleDef::vehHelicopterMaintainHeightLimit),
                            height_min: f(VehicleDef::vehHelicopterMaintainHeightMinimum),
                            height_max: f(VehicleDef::vehHelicopterMaintainHeightMaximum),
                            ceiling_min: f(VehicleDef::vehHelicopterMaintainCeilingMinimum),
                        }
                    },
                };
                let e = VehicleDef::engine;
                let bank = |at: usize| -> Vec<(String, u32, [f32; 5])> {
                    (0..5)
                        .map(|i| {
                            let o = at + i * VehicleEngineSound::SIZE;
                            (
                                name_at(o + VehicleEngineSound::name),
                                s.uint_from_bytes(body, o + VehicleEngineSound::alias).unwrap_or(0),
                                std::array::from_fn(|k| f(o + VehicleEngineSound::params + k * 4)),
                            )
                        })
                        .collect()
                };
                let num_gears = s.int_from_bytes(body, e + VehicleEngine::numGears).unwrap_or(0);
                let engine = VehicleEngineRef {
                    idle_rpms: f(e + VehicleEngine::idleRpms),
                    max_rpms: f(e + VehicleEngine::maxRpms),
                    max_torque: f(e + VehicleEngine::maxTorque),
                    braking_coeff: f(e + VehicleEngine::brakingCoeff),
                    load_fade_params: std::array::from_fn(|k| f(e + VehicleEngine::loadFadeParams + k * 4)),
                    load_scale: f(e + VehicleEngine::loadScale),
                    load_smoothing: f(e + VehicleEngine::loadSmoothing),
                    throttle_lag: f(e + VehicleEngine::throttleLag),
                    tilt_multiplier: f(e + VehicleEngine::pitchScale),
                    onload: bank(e + VehicleEngine::onload),
                    offload: bank(e + VehicleEngine::offload),
                    num_gears,
                    loop_last_gear: s.int_from_bytes(body, e + VehicleEngine::loopLastGear).unwrap_or(0),
                    gears: (0..num_gears.clamp(0, 10) as usize)
                        .map(|g| std::array::from_fn(|k| f(e + VehicleEngine::gears + g * 12 + k * 4)))
                        .collect(),
                };
                self.vehicles.push(VehicleRef {
                    name: name_at(VehicleDef::name),
                    turret_weapon: name_at(VehicleDef::turretWeapon),
                    gunner_weapons,
                    drive,
                    engine,
                });
                None
            }
            XAssetKind::PhysPreset => {
                self.phys_presets.push(PhysPresetRef {
                    name: string_field(s, body, PhysPreset::name)?,
                    mass: s.float_from_bytes(body, PhysPreset::mass)?,
                    bounce: s.float_from_bytes(body, PhysPreset::bounce)?,
                    friction: s.float_from_bytes(body, PhysPreset::friction)?,
                    bullet_push: s.float_from_bytes(body, PhysPreset::bulletForceScale)?,
                    blast_push: s.float_from_bytes(body, PhysPreset::explosiveForceScale)?,
                    shard_scatter_ratio: s.float_from_bytes(body, PhysPreset::piecesSpreadFraction)?,
                    shard_rise_speed: s.float_from_bytes(body, PhysPreset::piecesUpwardVelocity)?,
                    gravity_scale: s.float_from_bytes(body, PhysPreset::gravityScale)?,
                    center_of_mass_offset: vec3(s, body, PhysPreset::centerOfMassOffset)?,
                });
                Some(self.phys_presets.len() - 1)
            }
            XAssetKind::PhysConstraints => {
                let count = s.uint_from_bytes(body, PhysConstraints::count)?.min(16) as usize;
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    let c = body.shifted_by(PhysConstraints::data + i * PhysConstraint::SIZE);
                    data.push(PhysConstraintRef {
                        kind_code: s.int_from_bytes(c, PhysConstraint::r#type)?,
                        power: s.float_from_bytes(c, PhysConstraint::power)?,
                        size_mul: vec3(s, c, PhysConstraint::scale)?,
                        spin_scale: s.float_from_bytes(c, PhysConstraint::spin_scale)?,
                    });
                }
                self.phys_constraints.push(PhysConstraintsRef {
                    name: string_field(s, body, PhysConstraints::name)?,
                    data,
                });
                Some(self.phys_constraints.len() - 1)
            }
            XAssetKind::Destructible => {
                let name = string_field(s, body, BreakableDef::name).unwrap_or_default();
                let model_of = |this: &mut Self, off: usize| -> String {
                    this.asset_at(s, body.shifted_by(off))
                        .ok()
                        .flatten()
                        .filter(|k| k.ty == XAssetKind::XModel)
                        .and_then(|k| this.xmodels.get(k.index))
                        .map(|m| m.name.trim_start_matches(',').to_owned())
                        .unwrap_or_default()
                };
                let model = model_of(self, BreakableDef::model);
                let pristine_model = model_of(self, BreakableDef::pristineModel);
                let piece_total =
                    s.int_from_bytes(body, BreakableDef::numPieces)?.clamp(0, 64) as usize;
                let mut pieces = Vec::with_capacity(piece_total);
                if let Some(first) = deref(s, body, BreakableDef::pieces)? {
                    for i in 0..piece_total {
                        let p = first.shifted_by(i * DestructiblePiece::SIZE);
                        let mut stages: [DestructibleStageRef; 5] = Default::default();
                        for (j, stage) in stages.iter_mut().enumerate() {
                            let q = p.shifted_by(DestructiblePiece::stages + j * DestructibleStage::SIZE);
                            let bone = s.ushort_from_bytes(q, DestructibleStage::showBone)?;
                            let effect_label = self
                                .asset_at(s, q.shifted_by(DestructibleStage::breakEffect))?
                                .filter(|k| k.ty == XAssetKind::Fx)
                                .and_then(|k| self.fx.get(k.index))
                                .map(|f| f.name.trim_start_matches(',').to_owned())
                                .unwrap_or_default();
                            let mut spawned: [String; 3] = Default::default();
                            for (k, m) in spawned.iter_mut().enumerate() {
                                *m = self
                                    .asset_at(s, q.shifted_by(DestructibleStage::spawnModel + k * 4))?
                                    .filter(|k| k.ty == XAssetKind::XModel)
                                    .and_then(|k| self.xmodels.get(k.index))
                                    .map(|m| m.name.trim_start_matches(',').to_owned())
                                    .unwrap_or_default();
                            }
                            let shown_bone = if bone == 0 {
                                String::new()
                            } else {
                                self.string_for(bone)
                            };
                            *stage = DestructibleStageRef {
                                reveal_joint: shown_bone,
                                shatter_hitpoints: s.float_from_bytes(q, DestructibleStage::breakHealth)?,
                                longest_duration: s.float_from_bytes(q, DestructibleStage::maxTime)?,
                                flags: s.uint_from_bytes(q, DestructibleStage::flags)?,
                                shatter_visual: effect_label,
                                shatter_audio: string_field(s, q, DestructibleStage::breakSound)?,
                                shatter_signal: string_field(s, q, DestructibleStage::breakNotify)?,
                                repeating_audio: string_field(s, q, DestructibleStage::loopSound)?,
                                spawn_models: spawned,
                                physics_template: s.uint_from_bytes(q, DestructibleStage::physPreset)? != 0,
                            };
                        }
                        let mut conceal_joints = [0u32; 5];
                        for (slot, mask) in conceal_joints.iter_mut().enumerate() {
                            *mask = s.uint_from_bytes(p, DestructiblePiece::hideBones + slot * 4)?;
                        }
                        let constraint = self
                            .asset_at(s, p.shifted_by(DestructiblePiece::physConstraints))?
                            .filter(|k| k.ty == XAssetKind::PhysConstraints)
                            .and_then(|k| self.phys_constraints.get(k.index))
                            .and_then(|c| c.data.first().copied());
                        pieces.push(DestructiblePieceRef {
                            stages,
                            owner_fragment: s.byte_window_from(p, DestructiblePiece::parentPiece, 1)?[0],
                            inherited_harm_ratio: s.float_from_bytes(p, DestructiblePiece::parentDamagePercent)?,
                            round_damage_multiplier: s.float_from_bytes(p, DestructiblePiece::bulletDamageScale)?,
                            blast_damage_multiplier: s.float_from_bytes(p, DestructiblePiece::explosiveDamageScale)?,
                            melee_damage_scale: s.float_from_bytes(p, DestructiblePiece::meleeDamageScale)?,
                            impact_damage_scale: s.float_from_bytes(p, DestructiblePiece::impactDamageScale)?,
                            entity_damage_transfer: s.float_from_bytes(p, DestructiblePiece::entityDamageTransfer)?,
                            health: s.int_from_bytes(p, DestructiblePiece::health)?,
                            conceal_joints,
                            constraint,
                        });
                    }
                }
                let local_machine_only = s.int_from_bytes(body, BreakableDef::clientOnly)? != 0;
                self.destructibles.push(DestructibleRef {
                    name,
                    model,
                    pristine_model,
                    pieces,
                    local_machine_only,
                });
                None
            }
            XAssetKind::MapEnts => {
                let len = s.uint_from_bytes(body, MapEnts::numEntityChars)? as usize;
                let text = bytes_field(s, body, MapEnts::entityString, len)?;
                let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
                let entities = String::from_utf8_lossy(&text[..end]).into_owned();
                self.map_ents.push(entities);
                None
            }
            XAssetKind::Glasses => {
                let count = s.uint_from_bytes(body, Glasses::numGlasses)?.min(4096) as usize;
                if let Some(first) = deref(s, body, Glasses::glasses)? {
                    // codmp: a def's material is either inline or a
                    // reference to one loaded before (Raid's pristine
                    // material names its asset-list slot, not the body).
                    let material = |this: &mut Self, d: RawRef, off: usize| -> Result<String> {
                        if let Some(key) = this.asset_at(s, d.shifted_by(off))?
                            && let Some(m) = this.material(key)
                        {
                            return Ok(m.name.clone());
                        }
                        let name = match deref(s, d, off)? {
                            Some(m) => string_field(s, m, 0_usize).unwrap_or_default(),
                            None => String::new(),
                        };
                        Ok(if name.bytes().all(|b| b.is_ascii_graphic()) { name } else { String::new() })
                    };
                    for i in 0..count {
                        let p = first.shifted_by(i * Glass::SIZE);
                        let vertex_points = usize::from(s.u8_at(p, Glass::numOutlineVerts)?);
                        let mut outline = Vec::with_capacity(vertex_points);
                        if let Some(o) = deref(s, p, Glass::outline)? {
                            for k in 0..vertex_points {
                                outline.push([s.float_from_bytes(o, k * 8)?, s.float_from_bytes(o, k * 8 + 4)?]);
                            }
                        }
                        let mut glass = GlassRef {
                            solid_model: s.uint_from_bytes(p, Glass::brushModel)?,
                            index: s.uint_from_bytes(p, Glass::index)?,
                            origin: vec3(s, p, Glass::origin)?,
                            angles: vec3(s, p, Glass::angles)?,
                            outline,
                            axis: [
                                vec3(s, p, Glass::outlineAxis)?,
                                vec3(s, p, Glass::outlineAxis + 12)?,
                                vec3(s, p, Glass::outlineAxis + 24)?,
                            ],
                            outline_origin: vec3(s, p, Glass::outlineOrigin)?,
                            absmin: vec3(s, p, Glass::absmin)?,
                            absmax: vec3(s, p, Glass::absmax)?,
                            uv_scale: s.float_from_bytes(p, Glass::uvScale)?,
                            pane_thickness: s.float_from_bytes(p, Glass::thickness)?,
                            is_planar: s.u8_at(p, Glass::isPlanar)? != 0,
                            pool_shards: s.uint_from_bytes(body, Glasses::maxShards)?,
                            pool_physics: s.uint_from_bytes(body, Glasses::maxPhysics)?,
                            ..GlassRef::default()
                        };
                        if let Some(d) = deref(s, p, Glass::glassDef)? {
                            glass.definition_name = string_field(s, d, GlassDef::name)?;
                            glass.hit_point_ceiling = s.int_from_bytes(d, GlassDef::maxHealth)?;
                            glass.wall_depth = s.float_from_bytes(d, GlassDef::thickness)?;
                            glass.pristine_material = material(self, d, GlassDef::pristineMaterial)?;
                            glass.cracked_material = material(self, d, GlassDef::crackedMaterial)?;
                            glass.shard_material = material(self, d, GlassDef::shardMaterial)?;
                            glass.crack_sound = string_field(s, d, GlassDef::crackSound)?;
                            glass.shatter_sound = string_field(s, d, GlassDef::shatterShound)?;
                            glass.auto_shatter_sound = string_field(s, d, GlassDef::autoShatterShound)?;
                            glass.min_shard_size = s.float_from_bytes(d, GlassDef::minShardSize)?;
                            glass.max_shard_size = s.float_from_bytes(d, GlassDef::maxShardSize)?;
                            glass.shard_life_probability = s.float_from_bytes(d, GlassDef::shardLifeProbablility)?;
                            glass.max_shards = s.int_from_bytes(d, GlassDef::maxShards)?;
                            let fx = |this: &mut Self, at: usize| -> Result<String> {
                                Ok(this
                                    .asset_at(s, d.shifted_by(at))?
                                    .filter(|k| k.ty == XAssetKind::Fx)
                                    .and_then(|k| this.fx.get(k.index))
                                    .map(|f| f.name.trim_start_matches(',').to_owned())
                                    .unwrap_or_default())
                            };
                            glass.crack_effect = fx(self, GlassDef::crackEffect)?;
                            glass.shatter_effect = fx(self, GlassDef::shatterEffect)?;
                        }
                        self.glasses.push(glass);
                    }
                }
                None
            }
            // A mode zone's extra entities (`so_zclassic_zm_buried.ff`'s
            // `maps/mp/so_zclassic_zm_buried.mapents`), after the map's.
            XAssetKind::AddonMapEnts => {
                let len = s.uint_from_bytes(body, AddonMapEnts::numEntityChars)? as usize;
                let text = bytes_field(s, body, AddonMapEnts::entityString, len)?;
                let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
                let ents = String::from_utf8_lossy(&text[..end]).into_owned();
                self.map_ents.push(ents);
                None
            }
            _ => None,
        };
        if let Some(index) = key {
            self.keep(
                loaded,
                AssetIdentity {
                    ty: loaded.ty,
                    index,
                },
            );
        }
        Ok(())
    }
}

/// One brush (`cbrush_t`; a collision map's `BrushWrapper` starts the same):
/// its box, contents, the box sides' flags and every extra side.
fn read_brush(s: &ZoneByteReader<'_>, b: RawRef) -> Result<ClipBrushRef> {
    let mut axial_cflags = [[0i32; 3]; 2];
    let mut axial_sflags = [[0i32; 3]; 2];
    for side in 0..2 {
        for axis in 0_usize..3 {
            let o = (side * 3 + axis) * 4;
            axial_cflags[side][axis] = s.int_from_bytes(b, cbrush_t::axial_cflags + o)?;
            axial_sflags[side][axis] = s.int_from_bytes(b, cbrush_t::axial_sflags + o)?;
        }
    }
    let numsides = s.uint_from_bytes(b, cbrush_t::numsides)?.min(1024) as usize;
    let mut sides = Vec::with_capacity(numsides);
    let mut side_cflags = Vec::with_capacity(numsides);
    if let Some(arr) = deref(s, b, cbrush_t::sides)? {
        for side_no in 0..numsides {
            let side = arr.shifted_by(side_no * cbrushside_t::SIZE);
            if let Some(plane) = deref(s, side, cbrushside_t::plane)? {
                sides.push((
                    vec3(s, plane, cplane_s::normal)?,
                    s.float_from_bytes(plane, cplane_s::dist)?,
                    s.int_from_bytes(side, cbrushside_t::sflags)?,
                ));
                side_cflags.push(s.int_from_bytes(side, cbrushside_t::cflags)?);
            }
        }
    }
    let numverts = s.uint_from_bytes(b, cbrush_t::numverts)?.min(1024) as usize;
    let mut vertex_points = Vec::with_capacity(numverts);
    if let Some(arr) = deref(s, b, cbrush_t::vertex_points)? {
        for j in 0..numverts {
            vertex_points.push(vec3(s, arr.shifted_by(j * 12), 0)?);
        }
    }
    Ok(ClipBrushRef {
        lo_corner: vec3(s, b, cbrush_t::mins)?,
        hi_corner: vec3(s, b, cbrush_t::maxs)?,
        contents: s.int_from_bytes(b, cbrush_t::contents)?,
        axial_cflags,
        axial_sflags,
        sides,
        side_cflags,
        vertex_points,
    })
}
