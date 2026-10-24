//! The system that rebuilds the frame's code sources from the prepared view.

use bevy::prelude::*;

use super::super::frame_products::MaterialFrameInputs;
use super::super::material_runtime::RuntimeCodeSources;
use super::super::{
    DrawMethodDfog, SunShadowCamera, forced_fallback_frame, sun_shadow_forward_from_light_dir,
    tan_half_fov_from_clip,
};
use super::{
    CODE_TEXTURE_RESOLVED_POST_SUN, CommandContextRefusal, GfxViewport, MapDirPrimaryLight,
    MapFrameFog, MapOutdoor, host_clip_from_view, produce_dir_primary_light,
    produce_floatz_code_texture, produce_game_time, produce_lighting_lookup_scale,
    produce_model_lighting_code_texture, produce_spot_shadow_code_texture,
    produce_sun_shadow_code_texture, produce_sun_shadow_receiver_constants,
    produce_top_pair_command_context, produce_ui3d_code_sources, produce_update_viewport,
    world_transpose_rows_from_set3d,
};
use crate::assemble::drawsurf::fog::FogDvars;
use crate::prepare::scene::camera::FpvLens;
use crate::prepare::scene::model_lighting_atlas::WorldModelLightingAtlas;
use crate::prepare::scene::view_parms::PreparedSceneView;
use crate::prepare::scene::world::WorldScene;
use hud_t6::set_2d::{GfxCmdBufSourceState, begin_view, gfx_scene_def};
use net::FrameClock;

/// Sampler state the resolved post-sun image is read with.
const RESOLVED_POST_SUN_SAMPLER: u8 = 0x62;

/// Copies the prepared view into the frame inputs and rebuilds every code
/// constant and code texture from it. Does nothing until a view is prepared.
pub(crate) fn update_command_context_code_sources(
    mut runtime: ResMut<MaterialFrameInputs>,
    fog: Option<Res<MapFrameFog>>,
    fog_dvars: Res<FogDvars>,
    mut dfog: ResMut<DrawMethodDfog>,
    dir_light: Option<Res<MapDirPrimaryLight>>,
    outdoor: Option<Res<MapOutdoor>>,
    lighting: Option<Res<WorldModelLightingAtlas>>,
    cg_clock: Option<Res<FrameClock>>,
    prepared: Res<PreparedSceneView>,
    cameras: Query<&Camera, With<FpvLens>>,
    scene: Option<Res<WorldScene>>,
) {
    if !prepared.ready {
        return;
    }
    let view = prepared.view_from_world;
    let clip_from_view = prepared.clip_from_view;
    let clip_from_world = prepared.clip_from_world;
    let viewmodel_clip_from_world =
        host_clip_from_view(prepared.fov, prepared.aspect, prepared.depth_hack_near) * view;
    let eye = prepared.eye;

    let frame = &mut *runtime;
    frame.view_origin = eye;
    frame.clip_from_world = Some(clip_from_world);
    frame.view_from_world = Some(view);
    frame.clip_from_view = Some(clip_from_view);
    frame.viewmodel_clip_from_world = Some(viewmodel_clip_from_world);
    frame.viewmodel_near = Some(prepared.depth_hack_near);
    frame.outdoor = outdoor.as_deref().copied();

    frame.code_sources = RuntimeCodeSources::default();
    frame.sun_shadow = None;
    let float_time_ms = cg_clock.map(|c| c.time()).unwrap_or(0);
    frame.float_time = float_time_ms as f32 / 1000.0;
    let float_time = frame.float_time;
    let sampled_fog = fog.as_ref().map(|fog| fog.sample(float_time_ms));
    dfog.0 = sampled_fog.as_ref().is_some_and(|fog| fog.sun.is_some());

    let mut source = GfxCmdBufSourceState {
        render_target_width: prepared.rt_w,
        render_target_height: prepared.rt_h,
        scene_viewport: prepared.scene_viewport,
        ..Default::default()
    };
    let scene_def = gfx_scene_def(float_time_ms, float_time, [0.0; 3]);
    let view_float_time = begin_view(&mut source, &scene_def, &prepared.parms);
    let world0 = world_transpose_rows_from_set3d(source.world_matrix);

    // A frame without fog just skips the fog rows; the other refusals cannot
    // arise from this call.
    let produced = produce_top_pair_command_context(
        &mut frame.code_sources,
        clip_from_world,
        view,
        eye,
        world0,
        sampled_fog.as_ref(),
        fog_dvars.enabled,
    );
    debug_assert!(
        matches!(
            produced,
            Ok(()) | Err(CommandContextRefusal::MissingFrameFog)
        ),
        "unexpected refusal {produced:?}"
    );

    if let Ok(camera) = cameras.single()
        && let (Some(rt), Some(vp)) = (
            camera.physical_target_size(),
            camera.physical_viewport_rect(),
        )
    {
        let size = vp.size();
        if let (Ok(rt_w), Ok(rt_h), Ok(x), Ok(y), Ok(width), Ok(height)) = (
            i32::try_from(rt.x),
            i32::try_from(rt.y),
            i32::try_from(vp.min.x),
            i32::try_from(vp.min.y),
            i32::try_from(size.x),
            i32::try_from(size.y),
        ) {
            let _ = produce_update_viewport(
                &mut frame.code_sources,
                rt_w,
                rt_h,
                GfxViewport {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
    }
    if let Some(light) = dir_light.as_deref() {
        let _ = produce_dir_primary_light(&mut frame.code_sources, light);
    }
    produce_game_time(&mut frame.code_sources, view_float_time);
    if let Some(lighting) = lighting.as_deref()
        && let Some(inv_h) =
            lighting_t6::model_lighting_inv_image_height(lighting.dims.image_height)
    {
        produce_lighting_lookup_scale(&mut frame.code_sources, inv_h);
        let _ = produce_model_lighting_code_texture(&mut frame.code_sources);
    }
    let _ = produce_floatz_code_texture(&mut frame.code_sources);
    let _ = produce_spot_shadow_code_texture(&mut frame.code_sources);
    let _ = frame
        .code_sources
        .set_texture(CODE_TEXTURE_RESOLVED_POST_SUN, RESOLVED_POST_SUN_SAMPLER);
    produce_ui3d_code_sources(&mut frame.code_sources);

    if let Some(light) = dir_light.as_deref() {
        let _ = produce_sun_shadow_code_texture(&mut frame.code_sources);
        if let Some(bounds) = scene.as_deref().and_then(|scene| scene.world_bounds) {
            let world_mid = [bounds[0], bounds[1], bounds[2]];
            let world_half = [bounds[3], bounds[4], bounds[5]];
            let shadow_forward = sun_shadow_forward_from_light_dir(light.direction);
            let world = prepared.view_from_world.inverse();
            let forward = world.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
            let right = world.transform_vector3(Vec3::X).normalize_or_zero();
            let up = world.transform_vector3(Vec3::Y).normalize_or_zero();
            let (tx, ty) = tan_half_fov_from_clip(clip_from_view);
            let sun_frame = forced_fallback_frame(
                shadow_forward,
                SunShadowCamera {
                    origin: eye.to_array(),
                    forward: forward.to_array(),
                    right: right.to_array(),
                    up: up.to_array(),
                    tan_half_fov_x: tx,
                    tan_half_fov_y: ty,
                    z_near: prepared.near,
                },
                world_mid,
                world_half,
            );
            produce_sun_shadow_receiver_constants(&mut frame.code_sources, sun_frame);
            frame.sun_shadow = Some(sun_frame);
        }
    }
}
