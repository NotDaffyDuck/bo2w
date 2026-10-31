//! Frame preparation: the world spawn, the camera, visibility culling, model
//! lighting and the gfx scene, scheduled in `ClientSet::Present`.

use bevy::prelude::*;
use frame::{LevelPresent, ModelLightPlaced, MatchSwapDone, JobQueueStage};
use net::ClientStep;

pub mod visual_stage;
pub mod scene;
pub mod daylight;
pub mod task_orders;

use crate::adapters::motion::{gun_present, scene_feed, script_prop, eye_kick};
use crate::assemble::draw_items;
use scene::{
    camera as cam, cull, model_illum_cache as dyn_light, prop_geom_memo as smc,
    prop_light_slots, spawn as world_spawn, eye_parms, world::LevelStage,
};

/// Adds every resource, set and system that prepares a frame for drawing.
pub struct DrawPreparePlugin;

impl bevy::app::Plugin for DrawPreparePlugin {
    fn build(&self, client: &mut App) {
        draw_items::dof::register(client);
        draw_items::screen_grade_view::register(client);
        seed_resources(client);
        order_host_scene(client);
        order_view_and_lists(client);
        order_dynamic_lighting(client);
        order_camera_and_visibility(client);
        order_session_swap(client);
        world_spawn::register_level_gpu_ready(client);
        scene::cell_cone_jobs::register_room_view_volume_orders(client);
    }
}

/// Inserts each listed resource's default, in the order given.
macro_rules! seed {
    ($app:ident: $($ty:ty),+ $(,)?) => {
        $( $app.init_resource::<$ty>(); )+
    };
}

fn seed_resources(client: &mut App) {
    seed!(client: LevelPresent, LevelStage, world_spawn::LevelSpawnJob, cull::SightFrameStats);
    seed!(client: draw_items::FacetDrawList, draw_items::StaticDrawTrack, draw_items::FigureDrawTrack, draw_items::EffectDrawTrack);
    seed!(client: draw_items::FrameRenderOutputs, draw_items::ColourPaintMethod);
    seed!(client: smc::PropCacheEnableTunable, smc::PrebuiltTunable, smc::DetailRampTunable, smc::DetailRampSkinnedTunable);
    seed!(client: smc::FrontendTaskOrders);
    seed!(client: draw_items::MapSunEffects, draw_items::SunEffectsFrameInput);
    seed!(client: draw_items::MapKeyLightTypes, draw_items::MapKeyLights, draw_items::DrawMethodDepthHaze);
    seed!(client: draw_items::fog::HazeTunables, draw_items::DaylightShadowMapPresent, draw_items::ConeShadowMapLights);
    seed!(client: draw_items::DaylightShadowCasterPlan, draw_items::ConeShadowCasterPlan);
    seed!(client: draw_items::MaterialEpoch, draw_items::MaterialFrameFeed, draw_items::FrameBuildFeed);
    seed!(client: draw_items::CameraOutputs, draw_items::WarpSettings, draw_items::DaylightOutput, draw_items::ConeOutput);
    seed!(client: draw_items::fog::ScriptFogPresentation);
    seed!(client: draw_items::FirstPersonPaintPlan, draw_items::OtherBodyPaintPlan, draw_items::ScriptedModelPaintPlan);
    seed!(client: draw_items::ThrownPaintPlan, draw_items::PickupPaintPlan, draw_items::FxModelDrawList);
    seed!(client: draw_items::DynEntPaintPlan, draw_items::FigureDrawPlan);
    seed!(client: frame::KitPickerPassover, frame::Retiring);
}

/// The host scene empties once the world is in, takes the frame's models between the
/// cell walk and the model workers, and is copied for the spot shadows after skinning.
fn order_host_scene(client: &mut App) {
    use scene::visual_stage::{RenderSceneInsert, RenderSceneWipe};

    let empty = RenderSceneWipe.before(RenderSceneInsert).after(world_spawn::spawn_level_world);
    let fill = RenderSceneInsert
        .before(JobQueueStage::SkinModel)
        .before(JobQueueStage::CellDynModel)
        .after(JobQueueStage::CellStatic)
        .after(dyn_light::begin_debris_model_illum_frame);
    client.configure_sets(Update, (empty, fill).in_set(ClientStep::Present));

    let submissions = scene::visual_stage::apply_anim_puppet_stage_submissions;
    let script_solids = scene::visual_stage::occupy_script_solid_stage
        .in_set(RenderSceneInsert)
        .after(submissions)
        .after(script_prop::ScriptPropDrawStage)
        .after(world_spawn::spawn_level_world);
    let shadow_copy = scene::visual_stage::capture_cone_shadow_fill
        .in_set(net::ClientStep::Present)
        .after(JobQueueStage::SkinModel)
        .after(RenderSceneInsert);
    client.add_systems(
        Update,
        (
            scene::visual_stage::reset_host_render_scene.in_set(RenderSceneWipe),
            submissions
                .in_set(RenderSceneInsert)
                .after(scene_feed::SkeletonSceneStage),
            script_solids,
            shadow_copy,
        ),
    );
}

/// Spawning the world, then what stamps the frame's view and what reads it: fog, the
/// sun, the draw-surface list, and the static models' lighting and cache.
fn order_view_and_lists(client: &mut App) {
    use world_spawn::{arm_level_spawn_on_install, spawn_level_world, spawn_level_finish};

    let view = eye_parms::stamp_staged_stage_eye;
    let world = (
        arm_level_spawn_on_install.before(spawn_level_world),
        spawn_level_world,
        spawn_level_finish.after(spawn_level_world),
        render_anim::prepare_fpv_compositions.after(spawn_level_finish),
        render_anim::prepare_model_materials.after(spawn_level_finish),
        draw_items::fog::sync_script_fog.before(view).after(spawn_level_finish),
    );
    let camera = (
        cam::free_camera,
        view.after(gun_present::tick_viewmodel)
            .after(gun_present::spawn_viewmodel)
            .after(eye_kick::sync_camera_from_shown)
            .after(cam::free_camera),
        draw_items::publish_sun_effects_frame.after(view),
    );
    let surfaces = (
        draw_items::take_drawsurf_list
            .after(JobQueueStage::CellDynBrush)
            .after(JobQueueStage::CellDynModel)
            .after(JobQueueStage::CellStatic),
        prop_light_slots::update_prop_illum
            .after(script_prop::ScriptPropSkinStage)
            .after(JobQueueStage::SkinModel)
            .after(JobQueueStage::CellStatic),
        smc::cache_visible_prop_facets.after(prop_light_slots::update_prop_illum),
    );
    let fpv_lighting = dyn_light::enqueue_first_person_model_illum
        .before(JobQueueStage::CellDynModel)
        .after(render_anim::ScriptPropDrawStage)
        .after(dyn_light::begin_debris_model_illum_frame)
        .after(render_anim::tick_viewmodel)
        .after(render_anim::spawn_viewmodel);
    let lighting = (
        dyn_light::begin_debris_model_illum_frame.after(spawn_level_world),
        fpv_lighting,
    );
    let reports = (
        cull::log_sight_stats_once.after(JobQueueStage::CellStatic),
        cull::log_script_model_gaps_one_time,
    );
    client.add_systems(
        Update,
        (world, camera, surfaces, lighting, reports).in_set(ClientStep::Present),
    );

    let skin_cached = smc::skin_cached_prop_order
        .after(smc::cache_visible_prop_facets)
        .in_set(JobQueueStage::SmodelCache);
    client.add_systems(Update, skin_cached);
}

/// Lighting for moving models: the dirty ones first, then glass, then effects, and the
/// effects' lit models are resolved once their lighting is in.
fn order_dynamic_lighting(client: &mut App) {
    use dyn_light::{refresh_dirty_model_lighting, refresh_fx_dyn_lighting, refresh_glass_dyn_lighting};

    let dirty_first = refresh_dirty_model_lighting
        .before(refresh_glass_dyn_lighting)
        .in_set(ModelLightPlaced)
        .after(dyn_light::enqueue_first_person_model_illum)
        .after(script_prop::ScriptPropSkinStage)
        .after(JobQueueStage::CellDynModel)
        .after(JobQueueStage::SkinModel)
        .after(dyn_light::begin_debris_model_illum_frame);
    client.add_systems(Update, dirty_first);

    let rest = (
        refresh_glass_dyn_lighting.before(JobQueueStage::FxVerts).after(refresh_dirty_model_lighting),
        refresh_fx_dyn_lighting.after(refresh_glass_dyn_lighting).after(JobQueueStage::FxVerts),
        draw_items::mesh_build::figure_mesh::apply_resolved_effect_model_illum.after(refresh_fx_dyn_lighting),
    );
    client.add_systems(Update, rest.in_set(ClientStep::Present));
}

fn order_camera_and_visibility(client: &mut App) {
    let view = eye_parms::stamp_staged_stage_eye;
    let look = cam::look_camera_override
        .before(view)
        .after(eye_kick::sync_camera_from_shown)
        .after(cam::free_camera);
    let visibility = cull::apply_sight_hide
        .in_set(JobQueueStage::CellStatic)
        .after(view)
        .after(cam::free_camera);
    client.add_systems(Update, look);
    client.add_systems(Update, visibility);
}

/// A session swap takes the old world down in a fixed order: its entities, its spawn
/// state, the world itself, and last the fly cameras.
fn order_session_swap(client: &mut App) {
    let shutdown = world_spawn::shutdown_level_on_teardown;
    let teardown = (
        world_spawn::despawn_level_entities_on_teardown,
        world_spawn::reset_level_spawn_on_teardown,
        shutdown,
        world_spawn::despawn_free_cameras_on_teardown,
    );
    client.add_systems(
        Update,
        teardown.chain().in_set(ClientStep::Load).after(MatchSwapDone),
    );
    client.add_systems(
        Update,
        fill_static_lookups_on_install
            .in_set(net::ClientStep::Load)
            .after(shutdown)
            .after(MatchSwapDone),
    );
}

/// The install message is sent once the static tables are whole, so that is the moment
/// to fill the moving-entity lookup and the world's cell list.
fn fill_static_lookups_on_install(
    mut installs: MessageReader<frame::GameSessionArmed>,
    world: Res<LevelStage>,
    mut at_point: ResMut<render_scene::LooseAtSpotLookup>,
    mut cell_list: ResMut<render_scene::LevelSightRooms>,
) {
    // Reading drains the messages, so one install fills the tables once.
    let any_installed = installs.read().last().is_some();
    if !any_installed {
        return;
    }
    world.publish_loose_at_spot(&mut at_point);
    world.publish_sight_rooms(&mut cell_list);
}
