//! Register numbers of the code constants and code textures a shader can ask
//! for. A constant is named by its first register; a matrix covers the rows
//! that follow.

use lighting_t6 as lt;

// ---- lights ---------------------------------------------------------------

pub const CODE_LIGHT_POSITION: u16 = lt::CONST_SRC_CODE_LIGHT_POSITION;
pub const CODE_LIGHT_DIFFUSE: u16 = lt::CONST_SRC_CODE_LIGHT_DIFFUSE;
pub const CODE_LIGHT_SPECULAR: u16 = lt::CONST_SRC_CODE_LIGHT_SPECULAR;
pub const CODE_LIGHT_SPOTDIR: u16 = lt::CONST_SRC_CODE_LIGHT_SPOTDIR;
pub const CODE_LIGHT_SPOTFACTORS: u16 = lt::CONST_SRC_CODE_LIGHT_SPOTFACTORS;
pub const CODE_LIGHT_FALLOFF_PLACEMENT: u16 = lt::CONST_SRC_CODE_LIGHT_FALLOFF_PLACEMENT;

// ---- time and fog ---------------------------------------------------------

pub const CODE_GAMETIME: u16 = 0x07;
pub const CODE_FOG: u16 = 0x25;
pub const CODE_FOG_COLOR_LINEAR: u16 = 0x26;
pub const CODE_FOG_COLOR_GAMMA: u16 = 0x27;
pub const CODE_FOG_SUN_CONSTS: u16 = 0x28;
pub const CODE_FOG_SUN_COLOR_LINEAR: u16 = 0x29;
pub const CODE_FOG_SUN_COLOR_GAMMA: u16 = 0x2a;
pub const CODE_FOG_SUN_DIR: u16 = 0x2b;

// ---- render target and clip space -----------------------------------------

pub const CODE_RENDER_TARGET_SIZE: u16 = 0x16;
pub const CODE_CLIP_SPACE_LOOKUP_SCALE: u16 = 0x3f;
pub const CODE_CLIP_SPACE_LOOKUP_OFFSET: u16 = 0x40;
pub const CODE_DEPTH_FROM_CLIP: u16 = 0x49;
pub const R_ZNEAR_DEPTHHACK: f32 = hud_t6::view_projection::R_ZNEAR_DEPTHHACK_DEFAULT;

// ---- UI3D (the Target Finder screen) --------------------------------------

/// The window-5 rect inside the UI3D texture; window 5 is the full 0..1 rect.
/// Measured: every UI3D pass in common_mp.ff (the 5 UI3D materials, 68 passes,
/// both the reticle_ui3d and the ui3d5 techsets) reads code pixel const 0xa3,
/// 1 row, location 160, buffer 2. 0x11 is read only by the two reticle_ui3d
/// passes. See facts/target-finder.md section 8.
pub const CODE_UI3D_UV_SETUP_5: u16 = 0xa3;

/// The UI3D code texture, drawn by render_gpu `drawsurf/ui3d.rs`. Measured:
/// common_mp.ff `mc/mtl_t6_attach_optic_rangefinder_reticle`, technique 3
/// pass 0; its one code sampler argument (type 4, location 256) is 0x21.
pub const CODE_TEXTURE_UI3D: u32 = 0x21;

/// Linear, no mips, clamp U and V: the same state as the other one-mip render
/// targets here (OUTDOOR, RESOLVED_POST_SUN).
// TODO(fact): not measured yet. The UI3D lens material
// (common_mp.ff mc/mtl_t6_attach_optic_rangefinder_reticle) has no UI3D entry
// in its texture table; see facts/target-finder.md.
pub const CODE_TEXTURE_UI3D_SAMPLER: u8 = 0x62;

// ---- lighting lookups -----------------------------------------------------

pub const CODE_LIGHTING_LOOKUP_SCALE: u16 = 0x22;
pub const CODE_BASE_LIGHTING_COORDS: u16 = 0x3a;
pub const CODE_LIGHT_PROBE_AMBIENT: u16 = 0x3b;
pub const CODE_MATERIAL_COLOR: u16 = 0x24;
pub const CODE_MESH_ARG_0: u16 = 0x4a;
pub const CODE_MESH_ARG_1: u16 = 0x4b;

// ---- matrices (each is stored transposed, row by row) ---------------------

pub const FIRST_CODE_MATRIX: u16 = 0x4c;
pub const CODE_TRANSPOSE_PROJECTION: u16 = 0x52;
pub const CODE_TRANSPOSE_VIEW_PROJECTION: u16 = 0x56;
pub const CODE_TRANSPOSE_WORLD_OUTDOOR_LOOKUP: u16 = 0x5e;
pub const CODE_TRANSPOSE_WORLD0: u16 = 0x62;
pub const CODE_TRANSPOSE_WORLD1: u16 = 0x6e;
pub const CODE_TRANSPOSE_WORLD2: u16 = 0x7a;
pub const CODE_TRANSPOSE_WORLD_VIEW0: u16 = 0x66;
pub const CODE_TRANSPOSE_WORLD_VIEW1: u16 = 0x72;
pub const CODE_TRANSPOSE_WORLD_VIEW2: u16 = 0x7e;
pub const CODE_TRANSPOSE_WORLD_VIEW_PROJECTION0: u16 = 0x6a;
pub const CODE_TRANSPOSE_WORLD_VIEW_PROJECTION1: u16 = 0x76;
pub const CODE_TRANSPOSE_WORLD_VIEW_PROJECTION2: u16 = 0x82;
pub const CODE_INVERSE_WORLD_VIEW0: u16 = 0x65;
pub const CODE_INVERSE_TRANSPOSE_WORLD_VIEW0: u16 = 0x67;

// ---- code textures and the sampler state each is read with ----------------

pub const CODE_TEXTURE_OUTDOOR: u32 = 0x0e;
pub const CODE_TEXTURE_OUTDOOR_SAMPLER: u8 = 0x62;
pub const CODE_TEXTURE_FLOATZ: u32 = 0x0f;
pub const CODE_TEXTURE_FLOATZ_SAMPLER: u8 = 0x61;
pub const CODE_TEXTURE_RESOLVED_POST_SUN: u32 = 9;
pub const CODE_TEXTURE_MODEL_LIGHTING: u32 = lt::TEXTURE_SRC_CODE_MODEL_LIGHTING as u32;
pub const CODE_TEXTURE_MODEL_LIGHTING_SAMPLER: u8 = 0xe2;
pub const CODE_TEXTURE_LIGHT_ATTENUATION: u32 = lt::TEXTURE_SRC_CODE_LIGHT_ATTENUATION as u32;
