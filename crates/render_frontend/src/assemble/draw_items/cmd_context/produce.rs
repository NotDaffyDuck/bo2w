//! Writers for the frame's code sources. Every `produce_*` fills the constant
//! rows, or binds the code texture, that one family of shaders reads.

use bevy::math::Mat4;
use bevy::prelude::*;

use super::super::material_live::{EngineSourceError, LiveEngineSources};
use super::super::{
    ENGINE_SHADOW_MAP_LOOKUP_TRANSPOSE, ENGINE_SHADOW_MAP_POLYGON_OFFSET, ENGINE_SHADOW_MAP_SCALE,
    ENGINE_SHADOW_MAP_SWITCH_PARTITION, ENGINE_DAYLIGHT_SHADOW_MAP_PIXEL_ADJUST, ENGINE_TEXTURE_SHADOW_MAP_CONE,
    ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT, DaylightShadowForcedFrame, DaylightShadowSamplingPath,
};
use super::ENGINE_CLIP_SPACE_LOOKUP_OFFSET;
use super::ENGINE_CLIP_SPACE_LOOKUP_SCALE;
use super::ENGINE_DEPTH_FROM_CLIP;
use super::ENGINE_HAZE;
use super::ENGINE_HAZE_COLOR_GAMMA;
use super::ENGINE_HAZE_COLOR_LINEAR;
use super::ENGINE_HAZE_DAYLIGHT_COLOR_GAMMA;
use super::ENGINE_HAZE_DAYLIGHT_COLOR_LINEAR;
use super::ENGINE_HAZE_DAYLIGHT_CONSTS;
use super::ENGINE_HAZE_DAYLIGHT_DIR;
use super::ENGINE_GAMETIME;
use super::ENGINE_INVERSE_TRANSPOSE_LEVEL_VIEW0;
use super::CODE_INVERSE_WORLD_VIEW0;
use super::CODE_LIGHT_DIFFUSE;
use super::CODE_LIGHT_POSITION;
use super::CODE_LIGHT_SPECULAR;
use super::ENGINE_ILLUM_LOOKUP_SCALE;
use super::ENGINE_MATERIAL_COLOR;
use super::ENGINE_RENDER_TARGET_SIZE;
use super::ENGINE_TEXTURE_FLOATZ;
use super::ENGINE_TEXTURE_FLOATZ_SAMPLER;
use super::ENGINE_TEXTURE_MODEL_ILLUM;
use super::ENGINE_TEXTURE_MODEL_ILLUM_SAMPLER;
use super::CODE_TEXTURE_UI3D;
use super::CODE_TEXTURE_UI3D_SAMPLER;
use super::ENGINE_TRANSPOSE_EYE_PROJECTION;
use super::ENGINE_TRANSPOSE_LEVEL_EYE_PROJECTION0;
use super::ENGINE_TRANSPOSE_LEVEL_VIEW0;
use super::ENGINE_TRANSPOSE_WORLD0;
use super::CODE_UI3D_UV_SETUP_5;
use super::CommandContextRejection;
use super::VisualViewport;
use super::MapDirKeyLight;
use super::eye_relative_clip;
use super::flip_matrix_rows_to_words;
use super::engine_transpose_matrix_rows;
use super::vec4_bits;

/// Bit pattern of the two-pi the shipped game multiplies by.
const CLOCK_TWO_PI: f64 = f64::from_bits(0x4019_21FB_6000_0000);
/// Seconds after which the game-time row starts over.
const GAME_CLOCK_WRAP: f64 = f64::from_bits(0x40E5_1800_0000_0000);

/// Multiplier for turning a 0..=255 byte back into a fraction.
const BYTE_TO_UNIT: f32 = 0.003_921_568_9;

/// Stores one four-float row at `register`.
fn put_row(sources: &mut LiveEngineSources, add_to: u16, row: [f32; 4]) {
    sources.store_constant_vectors(add_to, &[vec4_bits(row)]);
}

/// Stores a transposed matrix at `register`, one row per register.
fn put_matrix(sources: &mut LiveEngineSources, add_to: u16, matrix: Mat4) {
    sources.store_constant_vectors(add_to, &flip_matrix_rows_to_words(matrix));
}

// ---- code textures --------------------------------------------------------

pub fn produce_model_illum_engine_texture(
    sources: &mut LiveEngineSources,
) -> Result<(), EngineSourceError> {
    let (image, sampler) = (
        ENGINE_TEXTURE_MODEL_ILLUM,
        ENGINE_TEXTURE_MODEL_ILLUM_SAMPLER,
    );
    sources.put_sampler_mode(image, sampler)
}

pub fn produce_floatz_engine_texture(
    sources: &mut LiveEngineSources,
) -> Result<(), EngineSourceError> {
    sources.put_sampler_mode(ENGINE_TEXTURE_FLOATZ, ENGINE_TEXTURE_FLOATZ_SAMPLER)
}

pub fn produce_daylight_shadow_engine_texture(
    sources: &mut LiveEngineSources,
) -> Result<(), EngineSourceError> {
    let sampler = DaylightShadowSamplingPath::Backup.engine_image_sampler();
    sources.put_sampler_mode(ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT, sampler)
}

pub fn produce_cone_shadow_engine_texture(
    sources: &mut LiveEngineSources,
) -> Result<(), EngineSourceError> {
    let sampler = DaylightShadowSamplingPath::Backup.engine_image_sampler();
    sources.put_sampler_mode(ENGINE_TEXTURE_SHADOW_MAP_CONE, sampler)
}

/// Binds the UI3D texture and sets its window-5 rectangle to the full 0..1
/// square, so the Target Finder glass reads its own screen.
pub fn produce_ui3d_code_sources(sources: &mut LiveEngineSources) {
    put_row(sources, CODE_UI3D_UV_SETUP_5, [0.0, 0.0, 1.0, 1.0]);
    let _ = sources.put_sampler_mode(CODE_TEXTURE_UI3D, CODE_TEXTURE_UI3D_SAMPLER);
}

// ---- joint_transforms and the first constants of a frame ---------------------------

/// Transposed rows of the world matrix used when nothing has posed it: a
/// translation by the negated eye position.
pub fn default_world_transpose_rows(eye_origin: Vec3) -> Vec<[u32; 4]> {
    engine_transpose_matrix_rows(Mat4::from_translation(-eye_origin))
}

/// Writes world-view, its inverse and its inverse-transpose, all derived from
/// the single `world_view` matrix.
pub fn produce_level_eye_family(sources: &mut LiveEngineSources, level_view: Mat4) {
    let inverse = level_view.inverse();
    put_matrix(sources, ENGINE_TRANSPOSE_LEVEL_VIEW0, level_view);
    put_matrix(sources, CODE_INVERSE_WORLD_VIEW0, inverse.transpose());
    put_matrix(sources, ENGINE_INVERSE_TRANSPOSE_LEVEL_VIEW0, inverse);
}

/// Depth-from-clip: only w varies, and the view model negates it.
pub fn produce_depth_from_clip(sources: &mut LiveEngineSources, view_model_ent: bool) {
    let depth_sign = if view_model_ent { -1.0 } else { 1.0 };
    put_row(sources, ENGINE_DEPTH_FROM_CLIP, [0.0, 0.0, 0.0, depth_sign]);
}

/// Material color as a new command buffer sees it before anything sets one:
/// three channels at the float maximum, alpha zero.
pub fn produce_material_color_cmdbuf_init(sources: &mut LiveEngineSources) {
    let unset = f32::MAX;
    put_row(sources, ENGINE_MATERIAL_COLOR, [unset, unset, unset, 0.0]);
}

/// Everything a frame begins with: the view-projection joint_transforms, the material
/// color, depth-from-clip and the fog. Fails when no fog was supplied.
pub fn build_top_pair_command_context(
    inputs_src: &mut LiveEngineSources, world_to_clip: Mat4, world_to_view: Mat4, eye_origin: Vec3,
    level_zero: Vec<[u32; 4]>, fog: Option<&asset_world::ExpHaze>, has_fog: bool,
) -> Result<(), CommandContextRejection> {
    let eye_relative = eye_relative_clip(world_to_clip, eye_origin);
    put_matrix(inputs_src, ENGINE_TRANSPOSE_EYE_PROJECTION, eye_relative);
    inputs_src.store_shader_constant(ENGINE_TRANSPOSE_WORLD0, level_zero);
    put_matrix(inputs_src, ENGINE_TRANSPOSE_LEVEL_EYE_PROJECTION0, world_to_clip);
    produce_level_eye_family(inputs_src, world_to_view);
    produce_material_color_cmdbuf_init(inputs_src); produce_depth_from_clip(inputs_src, false);

    match fog {
        Some(frame_fog) => produce_frame_haze(inputs_src, frame_fog, has_fog),
        None => Err(CommandContextRejection::MissingFrameHaze),
    }
}

// ---- fog ------------------------------------------------------------------

/// Turns a fog color into its `(linear, gamma)` four-vectors. Each channel
/// first goes through an 8-bit quantisation, as the shipped game does, so the
/// result matches what it would draw to the bit.
pub fn haze_color_linear_and_gamma(rgb: [f32; 3], opacity: f32) -> ([f32; 4], [f32; 4]) {
    let quantise = |channel: f32| f32::from((channel.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);

    let [r, g, b] = rgb;
    let encoded = [r, g, b, opacity].map(|channel| quantise(channel) * BYTE_TO_UNIT);
    let to_linear = lighting_t6::gamma_encoded_to_linear;
    let decoded = [to_linear(encoded[0]), to_linear(encoded[1]), to_linear(encoded[2]), encoded[3]];
    (decoded, encoded)
}

/// Writes the fog rows for the frame. When fog is off the main fog row is
/// replaced by a pass-through, whatever the map asked for.
pub fn produce_frame_haze(
    sources: &mut LiveEngineSources,
    haze: &asset_world::ExpHaze,
    r_haze_enabled: bool,
) -> Result<(), CommandContextRejection> {
    let thickness = haze.thickness_at();
    put_row(
        sources,
        ENGINE_HAZE,
        [0.0, 1.0 - haze.opacity_cap, -thickness, haze.near_edge * thickness],
    );

    let (lin, gamma) = haze_color_linear_and_gamma(haze.tint_rgb, 1.0);
    put_row(sources, ENGINE_HAZE_COLOR_LINEAR, lin);
    put_row(sources, ENGINE_HAZE_COLOR_GAMMA, gamma);

    if let Some(sun) = &haze.sun {
        produce_sun_fog(sources, thickness, sun);
    }
    if !r_haze_enabled {
        put_row(sources, ENGINE_HAZE, [0.0, 1.0, 0.0, 0.0]);
    }
    Ok(())
}

/// Writes the sun's fog contribution, a cone about the sun direction.
fn produce_sun_fog(sources: &mut LiveEngineSources, thick_v: f32, sun: &asset_world::SunlitMist) {
    let cos_begin = sun.inner_cone_deg.to_radians().cos();
    let cos_end = sun.outer_cone_deg.to_radians().cos();
    let falloff = match cos_end < cos_begin {
        true => 1.0 / (cos_begin - cos_end),
        false => 100.0,
    };
    put_row(
        sources,
        ENGINE_HAZE_DAYLIGHT_CONSTS,
        [-thick_v * sun.scale, cos_end, falloff, -thick_v],
    );

    let (lin, gamma) = haze_color_linear_and_gamma(sun.tint_rgb, 1.0);
    put_row(sources, ENGINE_HAZE_DAYLIGHT_COLOR_LINEAR, lin);
    put_row(sources, ENGINE_HAZE_DAYLIGHT_COLOR_GAMMA, gamma);

    let magnitude = game_math::vec3_norm(sun.toward_sun);
    let [step_along_x, dy, dz] = match magnitude > 0.0 {
        true => sun.toward_sun.map(|axis| axis / magnitude),
        false => sun.toward_sun,
    };
    put_row(sources, ENGINE_HAZE_DAYLIGHT_DIR, [step_along_x, dy, dz, 0.0]);
}

// ---- viewport, time, lights -----------------------------------------------

/// Writes the render-target size row and the pair of rows mapping clip space
/// to target lookups. A target without area is refused.
pub fn build_update_viewport(
    sources: &mut LiveEngineSources, target_width: i32, target_height: i32, viewport: VisualViewport,
) -> Result<(), CommandContextRejection> {
    if target_width <= 0 || target_height <= 0 {
        return Err(CommandContextRejection::ViewportRenderTargetIsEmpty);
    }
    let (width, height) = (target_width as f32, target_height as f32);
    let (inv_w, inv_h) = (1.0 / width, 1.0 / height);
    let half_span_x = inv_w * viewport.width as f32 * 0.5;
    let half_span_y = inv_h * viewport.height as f32 * 0.5;

    put_row(sources, ENGINE_RENDER_TARGET_SIZE, [width, height, inv_w, inv_h]);
    put_row(
        sources,
        ENGINE_CLIP_SPACE_LOOKUP_SCALE,
        [half_span_x, -half_span_y, 0.0, 1.0],
    );
    let offset_x = 0.5 * inv_w + viewport.x as f32 * inv_w + half_span_x;
    let offset_y = 0.5 * inv_h + half_span_y + viewport.y as f32 * inv_h;
    put_row(
        sources,
        ENGINE_CLIP_SPACE_LOOKUP_OFFSET,
        [offset_x, offset_y, 0.0, 0.0],
    );
    Ok(())
}

/// Writes the game-time row `[sin, cos, fraction, wrapped time]`, where the
/// angle is the fractional second scaled to a full turn.
pub fn produce_game_time(sources: &mut LiveEngineSources, sim_time: f32) {
    let fraction = sim_time - sim_time.floor();
    let turn = (f64::from(fraction) * CLOCK_TWO_PI) as f32;
    let wrapped = (f64::from(sim_time) % GAME_CLOCK_WRAP) as f32;
    let (sine, cosine) = turn.sin_cos();
    put_row(sources, ENGINE_GAMETIME, [sine, cosine, fraction, wrapped]);
}

/// Writes position, diffuse and specular rows for the directional primary
/// light. Colors the map gives explicitly win; otherwise they are derived from
/// the light color and its scales. Non-directional lights are refused.
pub fn produce_dir_key_light(
    sources: &mut LiveEngineSources,
    light: &MapDirKeyLight,
) -> Result<(), CommandContextRejection> {
    if light.light_kind != lighting_t6::LAMP_KIND_PARALLEL {
        return Err(CommandContextRejection::KeyLightNotDir);
    }
    let opaque = |rgba: [f32; 4]| [rgba[0], rgba[1], rgba[2], 1.0];

    put_row(
        sources,
        CODE_LIGHT_POSITION,
        lighting_t6::sun_point_at_infinity(light.direction),
    );
    let diffuse = match light.classic_diffuse_color {
        Some(rgb) => opaque(rgb),
        None => lighting_t6::lamp_diffuse_rgba(light.color, light.diffuse_gain),
    };
    put_row(sources, CODE_LIGHT_DIFFUSE, diffuse);
    let specular = match light.classic_specular_color {
        Some(rgb) => opaque(rgb),
        None => lighting_t6::lamp_gloss_rgba(light.color, light.specular_gain),
    };
    put_row(sources, CODE_LIGHT_SPECULAR, specular);
    Ok(())
}

/// Writes the model-lighting lookup scale for an image whose height is the
/// reciprocal of `recip_image_height`.
pub fn produce_lighting_lookup_scale(sources: &mut LiveEngineSources, recip_image_height: f32) {
    let scale = lighting_t6::atlas_lookup_scale(recip_image_height);
    put_row(sources, ENGINE_ILLUM_LOOKUP_SCALE, scale.as_array());
}

// ---- sun shadow -------------------------------------------------------------

/// Writes the constants a receiving surface needs to sample the sun shadow map.
pub fn produce_daylight_shadow_receiver_constants(
    sources: &mut LiveEngineSources,
    frame: DaylightShadowForcedFrame,
) {
    put_row(
        sources,
        ENGINE_SHADOW_MAP_SWITCH_PARTITION,
        frame.receiver.switch_split,
    );
    put_row(sources, ENGINE_SHADOW_MAP_SCALE, frame.receiver.shadow_map_scale);
    put_row(
        sources,
        ENGINE_DAYLIGHT_SHADOW_MAP_PIXEL_ADJUST,
        frame.texel_offset_correction,
    );
    put_matrix(
        sources,
        ENGINE_SHADOW_MAP_LOOKUP_TRANSPOSE,
        frame.lookup.transpose(),
    );
    put_row(
        sources,
        ENGINE_SHADOW_MAP_POLYGON_OFFSET,
        frame.partitions[0].depth_bias,
    );
}
