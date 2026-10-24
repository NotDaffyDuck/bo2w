//! Fills the frame's code sources: each `produce_*` writes the constant rows or
//! binds the code texture one family of shaders reads.

use bevy::math::Mat4;
use bevy::prelude::*;

use super::super::material_runtime::{CodeSourceError, RuntimeCodeSources};
use super::super::{
    CODE_SHADOWMAP_LOOKUP_TRANSPOSE, CODE_SHADOWMAP_POLYGON_OFFSET, CODE_SHADOWMAP_SCALE,
    CODE_SHADOWMAP_SWITCH_PARTITION, CODE_SUN_SHADOWMAP_PIXEL_ADJUST, CODE_TEXTURE_SHADOWMAP_SPOT,
    CODE_TEXTURE_SHADOWMAP_SUN, SunShadowForcedFrame, SunShadowSamplingPath,
};
use super::{
    CODE_CLIP_SPACE_LOOKUP_OFFSET, CODE_CLIP_SPACE_LOOKUP_SCALE, CODE_DEPTH_FROM_CLIP, CODE_FOG,
    CODE_FOG_COLOR_GAMMA, CODE_FOG_COLOR_LINEAR, CODE_FOG_SUN_COLOR_GAMMA,
    CODE_FOG_SUN_COLOR_LINEAR, CODE_FOG_SUN_CONSTS, CODE_FOG_SUN_DIR, CODE_GAMETIME,
    CODE_INVERSE_TRANSPOSE_WORLD_VIEW0, CODE_INVERSE_WORLD_VIEW0, CODE_LIGHT_DIFFUSE,
    CODE_LIGHT_POSITION, CODE_LIGHT_SPECULAR, CODE_LIGHTING_LOOKUP_SCALE, CODE_MATERIAL_COLOR,
    CODE_RENDER_TARGET_SIZE, CODE_TEXTURE_FLOATZ, CODE_TEXTURE_FLOATZ_SAMPLER,
    CODE_TEXTURE_MODEL_LIGHTING, CODE_TEXTURE_MODEL_LIGHTING_SAMPLER, CODE_TEXTURE_UI3D,
    CODE_TEXTURE_UI3D_SAMPLER, CODE_TRANSPOSE_VIEW_PROJECTION,
    CODE_TRANSPOSE_WORLD_VIEW_PROJECTION0, CODE_TRANSPOSE_WORLD_VIEW0, CODE_TRANSPOSE_WORLD0,
    CODE_UI3D_UV_SETUP_5, CommandContextRefusal, GfxViewport, MapDirPrimaryLight,
    camera_relative_view_projection, code_transpose_matrix_row4, code_transpose_matrix_rows,
    float4_bits,
};

/// Two pi as the original game holds it, to the bit.
const GAMETIME_TWO_PI: f64 = f64::from_bits(0x4019_21FB_6000_0000);
/// The game time wraps at this many seconds.
const GAMETIME_WRAP: f64 = f64::from_bits(0x40E5_1800_0000_0000);

const VIEWPORT_ONE: f32 = 1.0;
const VIEWPORT_HALF: f32 = 0.5;

// ---- world transforms -----------------------------------------------------

/// The world matrix when nothing has posed it: a slide by minus the eye.
pub fn default_world_transpose_rows(view_origin: Vec3) -> Vec<[u32; 4]> {
    code_transpose_matrix_rows(Mat4::from_translation(-view_origin))
}

/// The world matrix a `set3d` call supplied, as transposed rows.
#[must_use]
pub fn world_transpose_rows_from_set3d(world: [f32; 16]) -> Vec<[u32; 4]> {
    code_transpose_matrix_rows(Mat4::from_cols_array(&world))
}

/// The inverse, transpose and inverse-transpose of world-view, from the one
/// world-view matrix.
pub fn produce_world_view_family(sources: &mut RuntimeCodeSources, world_view: Mat4) {
    let inverse = world_view.inverse();
    sources.set_constant_rows(
        CODE_INVERSE_WORLD_VIEW0,
        &code_transpose_matrix_row4(inverse.transpose()),
    );
    sources.set_constant_rows(
        CODE_TRANSPOSE_WORLD_VIEW0,
        &code_transpose_matrix_row4(world_view),
    );
    sources.set_constant_rows(
        CODE_INVERSE_TRANSPOSE_WORLD_VIEW0,
        &code_transpose_matrix_row4(inverse),
    );
}

/// The depth-from-clip row: the view-model flips the sign of w.
pub fn produce_depth_from_clip(sources: &mut RuntimeCodeSources, viewmodel: bool) {
    let w = if viewmodel { -1.0 } else { 1.0 };
    sources.set_constant_rows(CODE_DEPTH_FROM_CLIP, &[float4_bits([0.0, 0.0, 0.0, w])]);
}

/// The material color a fresh command buffer starts with: unset (all max).
pub fn produce_material_color_cmdbuf_init(sources: &mut RuntimeCodeSources) {
    sources.set_constant_rows(
        CODE_MATERIAL_COLOR,
        &[float4_bits([f32::MAX, f32::MAX, f32::MAX, 0.0])],
    );
}

/// The constants every frame starts with: the view-projection family, the
/// material color, depth-from-clip and fog.
pub fn produce_top_pair_command_context(
    sources: &mut RuntimeCodeSources,
    clip_from_world: Mat4,
    view_from_world: Mat4,
    view_origin: Vec3,
    world0: Vec<[u32; 4]>,
    fog: Option<&asset_world::ExpFog>,
    fog_enabled: bool,
) -> Result<(), CommandContextRefusal> {
    sources.set_constant_rows(
        CODE_TRANSPOSE_VIEW_PROJECTION,
        &code_transpose_matrix_row4(camera_relative_view_projection(
            clip_from_world,
            view_origin,
        )),
    );
    sources.set_constant(CODE_TRANSPOSE_WORLD0, world0);
    sources.set_constant_rows(
        CODE_TRANSPOSE_WORLD_VIEW_PROJECTION0,
        &code_transpose_matrix_row4(clip_from_world),
    );
    produce_world_view_family(sources, view_from_world);
    produce_material_color_cmdbuf_init(sources);
    produce_depth_from_clip(sources, false);
    let Some(frame_fog) = fog else {
        return Err(CommandContextRefusal::MissingFrameFog);
    };
    produce_frame_fog(sources, frame_fog, fog_enabled)
}

// ---- fog ------------------------------------------------------------------

/// A fog color as both the linear and the gamma vector. The color goes through
/// a round trip of four bytes, as the original does, so the two agree to the
/// bit with what it would draw.
pub fn fog_color_linear_and_gamma(rgb: [f32; 3], alpha: f32) -> ([f32; 4], [f32; 4]) {
    let pack = |c: f32| -> u8 { (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };

    let bytes = [pack(rgb[2]), pack(rgb[1]), pack(rgb[0]), pack(alpha)];
    const INV_255: f32 = 0.003_921_568_9;
    let gamma = [
        f32::from(bytes[2]) * INV_255,
        f32::from(bytes[1]) * INV_255,
        f32::from(bytes[0]) * INV_255,
        f32::from(bytes[3]) * INV_255,
    ];
    let linear = [
        lighting_t6::color_srgb_to_linear(gamma[0]),
        lighting_t6::color_srgb_to_linear(gamma[1]),
        lighting_t6::color_srgb_to_linear(gamma[2]),
        gamma[3],
    ];
    (linear, gamma)
}

/// The frame's fog rows. With fog switched off the fog row passes everything,
/// whatever the map asked for.
pub fn produce_frame_fog(
    sources: &mut RuntimeCodeSources,
    fog: &asset_world::ExpFog,
    r_fog_enabled: bool,
) -> Result<(), CommandContextRefusal> {
    let density = fog.density();
    let fog_row = [
        0.0,
        1.0 - fog.max_opacity,
        -density,
        fog.start_dist * density,
    ];
    sources.set_constant_rows(CODE_FOG, &[float4_bits(fog_row)]);

    let (linear, gamma) = fog_color_linear_and_gamma(fog.color_rgb, 1.0);
    sources.set_constant_rows(CODE_FOG_COLOR_LINEAR, &[float4_bits(linear)]);
    sources.set_constant_rows(CODE_FOG_COLOR_GAMMA, &[float4_bits(gamma)]);

    if let Some(sun) = fog.sun.as_ref() {
        produce_sun_fog(sources, fog.density(), sun);
    }
    if !r_fog_enabled {
        sources.set_constant_rows(CODE_FOG, &[float4_bits([0.0, 1.0, 0.0, 0.0])]);
    }
    Ok(())
}

/// The extra fog the sun adds: a cone around the sun's direction.
fn produce_sun_fog(sources: &mut RuntimeCodeSources, density: f32, sun: &asset_world::SunFog) {
    let begin = sun.begin_angle_deg.to_radians().cos();
    let end = sun.end_angle_deg.to_radians().cos();
    let slope = if end < begin {
        1.0 / (begin - end)
    } else {
        100.0
    };
    let sun_consts = [-density * sun.scale, end, slope, -density];
    sources.set_constant_rows(CODE_FOG_SUN_CONSTS, &[float4_bits(sun_consts)]);

    let (sun_linear, sun_gamma) = fog_color_linear_and_gamma(sun.color_rgb, 1.0);
    sources.set_constant_rows(CODE_FOG_SUN_COLOR_LINEAR, &[float4_bits(sun_linear)]);
    sources.set_constant_rows(CODE_FOG_SUN_COLOR_GAMMA, &[float4_bits(sun_gamma)]);

    let length = game_math::vec3_length(sun.sun_dir);
    let dir = if length > 0.0 {
        sun.sun_dir.map(|v| v / length)
    } else {
        sun.sun_dir
    };
    sources.set_constant_rows(
        CODE_FOG_SUN_DIR,
        &[float4_bits([dir[0], dir[1], dir[2], 0.0])],
    );
}

// ---- viewport, time, lights -----------------------------------------------

/// The render-target size and the two rows that turn clip space into a lookup
/// into the target. A target with no area is refused.
pub fn produce_update_viewport(
    sources: &mut RuntimeCodeSources,
    rt_width: i32,
    rt_height: i32,
    viewport: GfxViewport,
) -> Result<(), CommandContextRefusal> {
    if rt_width <= 0 || rt_height <= 0 {
        return Err(CommandContextRefusal::ViewportHasZeroRenderTarget);
    }
    let inv_w = VIEWPORT_ONE / rt_width as f32;
    let inv_h = VIEWPORT_ONE / rt_height as f32;
    let scale_x = inv_w * viewport.width as f32 * VIEWPORT_HALF;
    let scale_y = inv_h * viewport.height as f32 * VIEWPORT_HALF;
    sources.set_constant_rows(
        CODE_RENDER_TARGET_SIZE,
        &[float4_bits([
            rt_width as f32,
            rt_height as f32,
            inv_w,
            inv_h,
        ])],
    );
    sources.set_constant_rows(
        CODE_CLIP_SPACE_LOOKUP_SCALE,
        &[float4_bits([scale_x, -scale_y, 0.0, 1.0])],
    );
    sources.set_constant_rows(
        CODE_CLIP_SPACE_LOOKUP_OFFSET,
        &[float4_bits([
            VIEWPORT_HALF * inv_w + viewport.x as f32 * inv_w + scale_x,
            VIEWPORT_HALF * inv_h + scale_y + viewport.y as f32 * inv_h,
            0.0,
            0.0,
        ])],
    );
    Ok(())
}

/// The game-time row: sine and cosine of the fraction of a second, the
/// fraction itself, and the time wrapped.
pub fn produce_game_time(sources: &mut RuntimeCodeSources, game_time: f32) {
    let frac = game_time - game_time.floor();
    let angle = (f64::from(frac) * GAMETIME_TWO_PI) as f32;
    let (sin, cos) = angle.sin_cos();
    let wrapped = (f64::from(game_time) % GAMETIME_WRAP) as f32;
    sources.set_constant_rows(CODE_GAMETIME, &[float4_bits([sin, cos, frac, wrapped])]);
}

/// The directional primary light's position, diffuse and specular rows. A map
/// may supply the colors itself; otherwise they come from the light's color and
/// scales. Any other light type is refused.
pub fn produce_dir_primary_light(
    sources: &mut RuntimeCodeSources,
    light: &MapDirPrimaryLight,
) -> Result<(), CommandContextRefusal> {
    if light.light_type != lighting_t6::GFX_LIGHT_TYPE_DIR {
        return Err(CommandContextRefusal::PrimaryLightNotDir);
    }
    let position = lighting_t6::dir_light_position(light.direction);
    let diffuse = light
        .t5_diffuse_color
        .map(|v| [v[0], v[1], v[2], 1.0])
        .unwrap_or_else(|| lighting_t6::light_diffuse(light.color, light.diffuse_color_scale));
    let specular = light
        .t5_specular_color
        .map(|v| [v[0], v[1], v[2], 1.0])
        .unwrap_or_else(|| lighting_t6::light_specular(light.color, light.specular_color_scale));
    sources.set_constant_rows(CODE_LIGHT_POSITION, &[float4_bits(position)]);
    sources.set_constant_rows(CODE_LIGHT_DIFFUSE, &[float4_bits(diffuse)]);
    sources.set_constant_rows(CODE_LIGHT_SPECULAR, &[float4_bits(specular)]);
    Ok(())
}

/// The scale of the model-lighting lookup, from the reciprocal of the image
/// height.
pub fn produce_lighting_lookup_scale(sources: &mut RuntimeCodeSources, inv_image_height: f32) {
    let scale = lighting_t6::model_lighting_lookup_scale(inv_image_height);
    sources.set_constant_rows(CODE_LIGHTING_LOOKUP_SCALE, &[float4_bits(scale.as_array())]);
}

// ---- code textures --------------------------------------------------------

pub fn produce_model_lighting_code_texture(
    sources: &mut RuntimeCodeSources,
) -> Result<(), CodeSourceError> {
    sources.set_texture(
        CODE_TEXTURE_MODEL_LIGHTING,
        CODE_TEXTURE_MODEL_LIGHTING_SAMPLER,
    )
}

pub fn produce_floatz_code_texture(
    sources: &mut RuntimeCodeSources,
) -> Result<(), CodeSourceError> {
    sources.set_texture(CODE_TEXTURE_FLOATZ, CODE_TEXTURE_FLOATZ_SAMPLER)
}

/// The UI3D texture and its window-5 rect, so the Target Finder glass samples
/// its own screen instead of whatever was bound.
pub fn produce_ui3d_code_sources(sources: &mut RuntimeCodeSources) {
    sources.set_constant_rows(CODE_UI3D_UV_SETUP_5, &[float4_bits([0.0, 0.0, 1.0, 1.0])]);
    let _ = sources.set_texture(CODE_TEXTURE_UI3D, CODE_TEXTURE_UI3D_SAMPLER);
}

pub fn produce_sun_shadow_code_texture(
    sources: &mut RuntimeCodeSources,
) -> Result<(), CodeSourceError> {
    sources.set_texture(
        CODE_TEXTURE_SHADOWMAP_SUN,
        SunShadowSamplingPath::Fallback.code_image_sampler(),
    )
}

pub fn produce_spot_shadow_code_texture(
    sources: &mut RuntimeCodeSources,
) -> Result<(), CodeSourceError> {
    sources.set_texture(
        CODE_TEXTURE_SHADOWMAP_SPOT,
        SunShadowSamplingPath::Fallback.code_image_sampler(),
    )
}

/// The rows a surface needs to look up the sun's shadow map.
pub fn produce_sun_shadow_receiver_constants(
    sources: &mut RuntimeCodeSources,
    frame: SunShadowForcedFrame,
) {
    sources.set_constant_rows(
        CODE_SHADOWMAP_SWITCH_PARTITION,
        &[float4_bits(frame.receiver.switch_partition)],
    );
    sources.set_constant_rows(
        CODE_SHADOWMAP_SCALE,
        &[float4_bits(frame.receiver.shadowmap_scale)],
    );
    sources.set_constant_rows(
        CODE_SUN_SHADOWMAP_PIXEL_ADJUST,
        &[float4_bits(frame.pixel_adjust)],
    );
    sources.set_constant_rows(
        CODE_SHADOWMAP_LOOKUP_TRANSPOSE,
        &code_transpose_matrix_row4(frame.lookup.transpose()),
    );
    sources.set_constant_rows(
        CODE_SHADOWMAP_POLYGON_OFFSET,
        &[float4_bits(frame.partitions[0].polygon_offset)],
    );
}
