//! BO1 code-constant numbers that the renderer's leftover bank still reads
//! (`render_frontend` `command_context.rs` and `t5_fog.rs`). A BO1 constant
//! `n` lives at `LEFTOVER_T5_CODE_BASE + n` in that bank.

/// First slot of the leftover bank.
pub const LEFTOVER_T5_CODE_BASE: u16 = 0x200;

// Where the view and the eye are.
pub const T5_CODE_VPOSX_TO_WORLD: u16 = 0x16;
pub const T5_CODE_VPOSY_TO_WORLD: u16 = 0x17;
pub const T5_CODE_VPOS1_TO_WORLD: u16 = 0x18;
pub const T5_CODE_EYEOFFSET: u16 = 0xbf;
pub const T5_CODE_EXTRA_CAM_PARAM: u16 = 0xc3;

// Fog.
pub const T5_CODE_FOG: u16 = 0x38;
pub const T5_CODE_FOG2: u16 = 0x39;
pub const T5_CODE_FOG_COLOR: u16 = 0x3a;
pub const T5_CODE_SUN_FOG: u16 = 0x3b;
pub const T5_CODE_SUN_FOG_DIR: u16 = 0x3c;
pub const T5_CODE_SUN_FOG_COLOR: u16 = 0x3d;

// Dynamic lights.
pub const T5_CODE_LIGHT_ATTENUATION: u16 = 0x5;
pub const T5_CODE_LIGHT_FALLOFF_A: u16 = 0x6;
pub const T5_CODE_LIGHT_FALLOFF_B: u16 = 0x7;
pub const T5_CODE_LIGHT_SPOT_MATRIX0: u16 = 0x8;
pub const T5_CODE_LIGHT_SPOT_MATRIX1: u16 = 0x9;
pub const T5_CODE_LIGHT_SPOT_MATRIX2: u16 = 0xa;
pub const T5_CODE_LIGHT_SPOT_MATRIX3: u16 = 0xb;
pub const T5_CODE_LIGHT_SPOT_AABB: u16 = 0xc;
pub const T5_CODE_LIGHT_CONE_CONTROL1: u16 = 0xd;
pub const T5_CODE_LIGHT_CONE_CONTROL2: u16 = 0xe;
pub const T5_CODE_LIGHT_SPOT_COOKIE_SLIDE: u16 = 0xf;

// The sun.
pub const T5_CODE_SUN_POSITION: u16 = 0x32;
pub const T5_CODE_SUN_DIFFUSE: u16 = 0x33;
pub const T5_CODE_SUN_SPECULAR: u16 = 0x34;

// Hero lighting.
pub const T5_CODE_HERO_LIGHTING_R: u16 = 0xa6;
pub const T5_CODE_HERO_LIGHTING_G: u16 = 0xa7;
pub const T5_CODE_HERO_LIGHTING_B: u16 = 0xa8;
pub const T5_CODE_LIGHT_HERO_SCALE: u16 = 0xa9;

// Sky, high dynamic range and post effects.
pub const T5_CODE_SKY_TRANSITION: u16 = 0x44;
pub const T5_CODE_POSTFX_CONTROL0: u16 = 0x79;
pub const T5_CODE_HDRCONTROL_0: u16 = 0x89;
pub const T5_CODE_HDRCONTROL_1: u16 = 0x8a;
pub const T5_CODE_SKY_COLOR_MULTIPLIER: u16 = 0xc2;

// Generic material parameters.
pub const T5_CODE_GENERIC_PARAM0: u16 = 0xb7;
pub const T5_CODE_GENERIC_PARAM1: u16 = 0xb8;
pub const T5_CODE_GENERIC_PARAM4: u16 = 0xbb;
pub const T5_CODE_GENERIC_PARAM5: u16 = 0xbc;
pub const T5_CODE_GENERIC_PARAM6: u16 = 0xbd;

// Wind.
pub const T5_CODE_WIND_DIRECTION: u16 = 0x4f;
pub const T5_CODE_GRASS_WIND_FORCE0: u16 = 0x54;
pub const T5_CODE_VARIANT_WIND_SPRING_0: u16 = 0x62;
pub const T5_CODE_TREECANOPY_PARMS: u16 = 0x77;
pub const T5_CODE_CUSTOMWIND_CENTER: u16 = 0xc0;
pub const T5_CODE_CUSTOMWIND_SPRING: u16 = 0xc1;
