//! Rebuilds the frame's code sources from the prepared scene view.

use bevy::prelude::*;

use super::super::frame_outputs::MaterialFrameFeed;
use super::super::material_live::LiveEngineSources;
use super::super::{
    DrawMethodDepthHaze, DaylightShadowCamera, forced_backup_frame, daylight_shadow_forward_from_light_dir,
    half_fov_tan_of_clip,
};
use super::{
    ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT, CommandContextRejection, VisualViewport, MapDirKeyLight,
    MapFrameHaze, MapOpenair, perspective_clip_matrix, produce_dir_key_light,
    produce_floatz_engine_texture, produce_game_time, produce_lighting_lookup_scale,
    produce_model_illum_engine_texture, produce_cone_shadow_engine_texture,
    produce_daylight_shadow_engine_texture, produce_daylight_shadow_receiver_constants,
    build_top_pair_command_context, produce_ui3d_code_sources, build_update_viewport,
    engine_transpose_matrix_rows,
};
use crate::assemble::draw_items::fog::HazeTunables;
use crate::prepare::scene::camera::FirstPersonLens;
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::eye_parms::StagedStageEye;
use crate::prepare::scene::world::LevelStage;
use hud_t6::set_2d::{GfxCmdBufSourceState, begin_view, gfx_scene_def};
use net::FrameClock;

/// Sampler state the resolved post-sun image is read with.
const RESOLVED_POST_SUN_SAMPLER: u8 = 0x62;

/// Rebuilds every code constant and code texture for the frame from the
/// prepared view. Nothing happens until a view has been prepared.
pub(crate) fn update_command_context_engine_sources(
    mut runtime: ResMut<MaterialFrameFeed>,
    fog: Option<Res<MapFrameHaze>>,
    haze_tunables: Res<HazeTunables>,
    mut depth_haze: ResMut<DrawMethodDepthHaze>,
    dir_lamp: Option<Res<MapDirKeyLight>>,
    outdoor: Option<Res<MapOpenair>>,
    lighting: Option<Res<LevelModelIllumAtlas>>,
    cg_clock: Option<Res<FrameClock>>,
    ready_set: Res<StagedStageEye>,
    cam_query: Query<&Camera, With<FirstPersonLens>>,
    scene: Option<Res<LevelStage>>,
) {
    if !ready_set.primed {
        return;
    }
    let frame = &mut *runtime;
    store_view(frame, &ready_set, outdoor.as_deref());

    let clock_stamp_ms = cg_clock.map(|clock| clock.time()).unwrap_or(0);
    frame.time_float = clock_stamp_ms as f32 / 1000.0;
    let sampled_haze = fog.as_ref().map(|fog| fog.fog_at(clock_stamp_ms));
    depth_haze.0 = sampled_haze.as_ref().is_some_and(|fog| fog.sun.is_some());

    let mut source = GfxCmdBufSourceState {
        canvas_columns: ready_set.target_pixel_width,
        canvas_rows: ready_set.target_pixel_height,
        stage_window_rect: ready_set.stage_window_rect,
        ..Default::default()
    };
    let stage_setup = gfx_scene_def(clock_stamp_ms, frame.time_float, [0.0; 3]);
    let view_time = begin_view(&mut source, &stage_setup, &ready_set.inputs);
    // The world matrix `set3d` provided, as transposed register rows.
    let level_zero = engine_transpose_matrix_rows(Mat4::from_cols_array(&source.world_matrix));

    // No fog means the fog rows are skipped; this call cannot refuse for any
    // other reason.
    let produced = build_top_pair_command_context(
        &mut frame.engine_sources,
        ready_set.screen_from_map,
        ready_set.eye_from_map,
        ready_set.eye,
        level_zero,
        sampled_haze.as_ref(),
        haze_tunables.enabled,
    );
    debug_assert!(
        matches!(produced, Ok(()) | Err(CommandContextRejection::MissingFrameHaze)),
        "unexpected refusal {produced:?}"
    );

    if let Ok(camera) = cam_query.single() {
        store_viewport(frame, camera);
    }
    if let Some(light) = dir_lamp.as_deref() {
        let _ = produce_dir_key_light(&mut frame.engine_sources, light);
    }
    produce_game_time(&mut frame.engine_sources, view_time);
    store_lighting_lookup(frame, lighting.as_deref());
    let _ = produce_floatz_engine_texture(&mut frame.engine_sources);
    let _ = produce_cone_shadow_engine_texture(&mut frame.engine_sources);
    let _ = frame
        .engine_sources
        .put_sampler_mode(ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT, RESOLVED_POST_SUN_SAMPLER);
    produce_ui3d_code_sources(&mut frame.engine_sources);

    if let Some(light) = dir_lamp.as_deref() {
        let _ = produce_daylight_shadow_engine_texture(&mut frame.engine_sources);
        let bounds = scene.as_deref().and_then(|scene| scene.level_bounds);
        if let Some(bounds) = bounds {
            store_sun_shadow(frame, &ready_set, light, bounds);
        }
    }
}

/// Copies the prepared view's joint_transforms into the frame and starts the frame's
/// code sources and sun shadow from scratch.
fn store_view(
    frame: &mut MaterialFrameFeed,
    ready_set: &StagedStageEye,
    outdoor: Option<&MapOpenair>,
) {
    let viewmodel_projection =
        perspective_clip_matrix(ready_set.view_deg, ready_set.wide_by_tall, ready_set.weapon_depth_near);
    frame.eye_origin = ready_set.eye;
    frame.world_to_clip = Some(ready_set.screen_from_map);
    frame.world_to_view = Some(ready_set.eye_from_map);
    frame.view_to_clip = Some(ready_set.screen_from_eye);
    frame.hand_model_clip_from_level = Some(viewmodel_projection * ready_set.eye_from_map);
    frame.hand_near_plane = Some(ready_set.weapon_depth_near);
    frame.outdoor = outdoor.copied();
    frame.engine_sources = LiveEngineSources::default();
    frame.sun_shadow.take();
}

/// Publishes the camera's viewport, if its sizes are known and fit in `i32`.
fn store_viewport(frame: &mut MaterialFrameFeed, cam_xf: &Camera) {
    let (Some(goal), Some(rect)) = (
        cam_xf.physical_target_size(),
        cam_xf.physical_viewport_rect(),
    ) else {
        return;
    };
    let size = rect.size();
    let converted = [goal.x, goal.y, rect.min.x, rect.min.y, size.x, size.y]
        .map(|value| i32::try_from(value).ok());
    let [Some(target_pixel_width), Some(target_pixel_height), Some(x), Some(y), Some(width), Some(height)] = converted else {
        return;
    };
    let viewport = VisualViewport {
        width,
        height,
        x,
        y,
    };
    let _ = build_update_viewport(&mut frame.engine_sources, target_pixel_width, target_pixel_height, viewport);
}

/// Publishes the model-lighting lookup scale and texture when the atlas has a
/// usable image height.
fn store_lighting_lookup(
    frame: &mut MaterialFrameFeed,
    atlas: Option<&LevelModelIllumAtlas>,
) {
    let inv_height = atlas
        .and_then(|atlas| lighting_t6::light_atlas_reciprocal_height(atlas.dims.rows_tall));
    if let Some(inv_height) = inv_height {
        produce_lighting_lookup_scale(&mut frame.engine_sources, inv_height);
        let _ = produce_model_illum_engine_texture(&mut frame.engine_sources);
    }
}

/// Fits the sun shadow around the world bounds `[mid.xyz, half.xyz]` as seen
/// from the camera and publishes its receiver constants.
fn store_sun_shadow(
    frame: &mut MaterialFrameFeed,
    prepared: &StagedStageEye,
    light: &MapDirKeyLight,
    bbox: [f32; 6],
) {
    let camera_to_world = prepared.eye_from_map.inverse();
    let axis = |direction: Vec3| {
        let dir = camera_to_world.transform_vector3(direction).normalize_or_zero();
        dir.to_array()
    };
    let (fov_half_tan_x, fov_half_tan_y) = half_fov_tan_of_clip(prepared.screen_from_eye);
    let camera = DaylightShadowCamera {
        at_point: prepared.eye.to_array(),
        fwd: axis(Vec3::NEG_Z),
        right_v: axis(Vec3::X),
        up_v: axis(Vec3::Y),
        fov_half_tan_x,
        fov_half_tan_y,
        front_clip_depth: prepared.near_cut,
    };
    let sun_frame = forced_backup_frame(
        daylight_shadow_forward_from_light_dir(light.direction),
        camera,
        [bbox[0], bbox[1], bbox[2]],
        [bbox[3], bbox[4], bbox[5]],
    );
    produce_daylight_shadow_receiver_constants(&mut frame.engine_sources, sun_frame);
    frame.sun_shadow = Some(sun_frame);
}
