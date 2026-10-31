//! Where each code constant and code texture lives. A shader names a constant
//! by its first register; a matrix then occupies the rows after it. Within each
//! group the entries run in register order.


// ---- constants set from the light pack -------------------------------------

pub const CODE_LIGHT_POSITION: u16 = lighting_t6::ENGINE_CONST_LAMP_ORIGIN;
pub const CODE_LIGHT_DIFFUSE: u16 = lighting_t6::ENGINE_CONST_LAMP_DIFFUSE;
pub const CODE_LIGHT_SPECULAR: u16 = lighting_t6::ENGINE_CONST_LAMP_GLOSS;
pub const CODE_LIGHT_SPOTDIR: u16 = lighting_t6::ENGINE_CONST_LAMP_AIM;
pub const CODE_LIGHT_SPOTFACTORS: u16 = lighting_t6::ENGINE_CONST_LAMP_CONE_TERMS;
pub const CODE_LIGHT_FALLOFF_PLACEMENT: u16 = lighting_t6::ENGINE_CONST_LAMP_FALLOFF_ORIGIN;

// ---- scalar-ish rows: time, target, material, lighting, fog ----------------

pub const ENGINE_GAMETIME: u16 = 0x07;
pub const ENGINE_RENDER_TARGET_SIZE: u16 = 0x16;
pub const ENGINE_ILLUM_LOOKUP_SCALE: u16 = 0x22;
pub const ENGINE_MATERIAL_COLOR: u16 = 0x24;
pub const ENGINE_HAZE: u16 = 0x25;
pub const ENGINE_HAZE_COLOR_LINEAR: u16 = 0x26;
pub const ENGINE_HAZE_COLOR_GAMMA: u16 = 0x27;
pub const ENGINE_HAZE_DAYLIGHT_CONSTS: u16 = 0x28;
pub const ENGINE_HAZE_DAYLIGHT_COLOR_LINEAR: u16 = 0x29;
pub const ENGINE_HAZE_DAYLIGHT_COLOR_GAMMA: u16 = 0x2a;
pub const ENGINE_HAZE_DAYLIGHT_DIR: u16 = 0x2b;
pub const ENGINE_BASE_ILLUM_COORDS: u16 = 0x3a;
pub const ENGINE_LIGHT_ENV_PROBE_AMBIENT: u16 = 0x3b;

// ---- clip space, depth and mesh arguments ---------------------------------

pub const ENGINE_CLIP_SPACE_LOOKUP_SCALE: u16 = 0x3f;
pub const ENGINE_CLIP_SPACE_LOOKUP_OFFSET: u16 = 0x40;
pub const ENGINE_DEPTH_FROM_CLIP: u16 = 0x49;
pub const ENGINE_MESH_ARG_0: u16 = 0x4a;
pub const ENGINE_MESH_ARG_1: u16 = 0x4b;
pub const R_ZNEAR_DEPTHHACK: f32 = hud_t6::view_projection::STOCK_HELD_GUN_FRONT_CLIP;

// ---- joint_transforms; each is kept transposed, one row per register --------------

pub const FIRST_ENGINE_MATRIX: u16 = 0x4c;
pub const ENGINE_TRANSPOSE_PROJECTION: u16 = 0x52;
pub const ENGINE_TRANSPOSE_EYE_PROJECTION: u16 = 0x56;
pub const ENGINE_TRANSPOSE_LEVEL_OPENAIR_LOOKUP: u16 = 0x5e;
pub const ENGINE_TRANSPOSE_WORLD0: u16 = 0x62;
pub const CODE_INVERSE_WORLD_VIEW0: u16 = 0x65;
pub const ENGINE_TRANSPOSE_LEVEL_VIEW0: u16 = 0x66;
pub const ENGINE_INVERSE_TRANSPOSE_LEVEL_VIEW0: u16 = 0x67;
pub const ENGINE_TRANSPOSE_LEVEL_EYE_PROJECTION0: u16 = 0x6a;
pub const ENGINE_TRANSPOSE_WORLD1: u16 = 0x6e;
pub const ENGINE_TRANSPOSE_LEVEL_VIEW1: u16 = 0x72;
pub const ENGINE_TRANSPOSE_LEVEL_EYE_PROJECTION1: u16 = 0x76;
pub const ENGINE_TRANSPOSE_WORLD2: u16 = 0x7a;
pub const ENGINE_TRANSPOSE_LEVEL_VIEW2: u16 = 0x7e;
pub const ENGINE_TRANSPOSE_LEVEL_EYE_PROJECTION2: u16 = 0x82;

// ---- Target Finder screen (UI3D) -------------------------------------------

/// Pixel-constant register holding the window-5 rectangle of the UI3D texture;
/// window 5 spans the whole 0..1 square. All 68 UI3D passes in common_mp.ff
/// (five materials, reticle_ui3d and ui3d5 techsets) read this one register,
/// as a single row at location 160 in buffer 2. Register 0x11 is touched only
/// by the two reticle_ui3d passes. Background: facts/target-finder.md, section 8.
pub const CODE_UI3D_UV_SETUP_5: u16 = 0xa3;

/// Slot of the UI3D code texture that `drawsurf/ui3d.rs` in render_gpu fills.
/// Taken from the rangefinder reticle material in common_mp.ff (technique 3,
/// pass 0), whose only code sampler argument (type 4, location 256) is 0x21.
pub const CODE_TEXTURE_UI3D: u32 = 0x21;

/// Linear filtering, one mip, clamped on U and V, matching the other
/// single-mip render targets (OUTDOOR, RESOLVED_POST_SUN).
// TODO(fact): unverified. The rangefinder reticle material's texture table has
// no UI3D entry to confirm it against; see facts/target-finder.md.
pub const CODE_TEXTURE_UI3D_SAMPLER: u8 = 0x62;

// ---- code textures, each with the sampler state it is read through ---------

pub const ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT: u32 = 9;
pub const ENGINE_TEXTURE_OPENAIR: u32 = 0x0e;
pub const ENGINE_TEXTURE_OPENAIR_SAMPLER: u8 = 0x62;
pub const ENGINE_TEXTURE_FLOATZ: u32 = 0x0f;
pub const ENGINE_TEXTURE_FLOATZ_SAMPLER: u8 = 0x61;
pub const ENGINE_TEXTURE_MODEL_ILLUM: u32 = lighting_t6::TEX_SLOT_ILLUM_ATLAS as u32;
pub const ENGINE_TEXTURE_MODEL_ILLUM_SAMPLER: u8 = 0xe2;
pub const CODE_TEXTURE_LIGHT_ATTENUATION: u32 = lighting_t6::TEX_SLOT_LAMP_FALLOFF as u32;
