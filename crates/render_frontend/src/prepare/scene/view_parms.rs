//! Turns the world camera into the view the rest of the frame prepares for.

pub use render_scene::{
    LockPvsView, PreparedSceneView, RLockPvs, RZnearDepthhackDvar, RZnearDvar, SmEnableDvar,
    SmSunEnableDvar, host_clip_from_view, lens_world_from_parent_child, pack_live_view_parms,
    prepare_scene_view,
};

use bevy::camera::ClearColorConfig;
use bevy::prelude::*;

use crate::assemble::drawsurf::MapFrameFog;
use crate::assemble::drawsurf::fog::FogDvars;
use crate::prepare::scene::camera::{FlyCamera, FpvLens};
use crate::prepare::scene::smodel_geom_cache::{LodRampDvar, LodRampSkinnedDvar, update_lod_parms};

/// Aspect used when neither the projection nor the viewport has one.
const FALLBACK_ASPECT: f32 = 16.0 / 9.0;

/// The projection's aspect ratio, or the viewport's when the projection has
/// none yet, or widescreen as a last resort.
fn aspect_of(projection_aspect: f32, camera: &Camera) -> f32 {
    if projection_aspect >= 1e-3 {
        return projection_aspect;
    }
    let from_viewport = camera
        .logical_viewport_size()
        .filter(|size| size.y > 0.0)
        .map(|size| size.x / size.y);
    from_viewport
        .filter(|aspect| *aspect >= 1e-3)
        .unwrap_or(FALLBACK_ASPECT)
}

/// Fog hides the horizon only if the sky behind it takes the fog's colour, so
/// the target clears to that colour (quantised to 8 bits) while fog is on.
fn clear_for_fog(
    map_fog: Option<&MapFrameFog>,
    dvars: &FogDvars,
    clock: Option<&net::FrameClock>,
) -> ClearColorConfig {
    let Some(map_fog) = map_fog else {
        return ClearColorConfig::Default;
    };
    let sampled = map_fog.sample(clock.map_or(0, |c| c.time()));
    if dvars.zfar <= 0.0 || sampled.density() == 0.0 {
        return ClearColorConfig::Default;
    }
    let [r, g, b] = sampled
        .color_rgb
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0);
    ClearColorConfig::Custom(Color::linear_rgba(r, g, b, 1.0))
}

/// Builds this frame's [`PreparedSceneView`] from the fly camera (the pose)
/// and its lens (the projection), and updates what depends on it: the render
/// target's clear colour, the PVS lock, and the model level-of-detail ramps.
/// Leaves the default (empty) view when there is no camera to draw from.
pub(crate) fn stamp_prepared_scene_view(
    mut view: ResMut<PreparedSceneView>,
    mut lock_pvs: ResMut<RLockPvs>,
    mut lod: (ResMut<LodRampDvar>, ResMut<LodRampSkinnedDvar>),
    znear: Res<RZnearDvar>,
    fog: Res<FogDvars>,
    map_fog: Option<Res<MapFrameFog>>,
    clock: Option<Res<net::FrameClock>>,
    depthhack: Res<RZnearDepthhackDvar>,
    hosts: Query<&Transform, With<FlyCamera>>,
    mut lenses: Query<(&Transform, &Projection, &mut Camera), With<FpvLens>>,
) {
    *view = PreparedSceneView::default();
    let (Ok(host), Ok((lens, projection, mut camera))) = (hosts.single(), lenses.single_mut())
    else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        return;
    };

    let aspect = aspect_of(perspective.aspect_ratio, &camera);
    let (target_w, target_h) = camera
        .physical_target_size()
        .and_then(|size| Some((i32::try_from(size.x).ok()?, i32::try_from(size.y).ok()?)))
        .unwrap_or((0, 0));
    // The scene always fills the whole render target.
    camera.viewport = None;
    camera.clear_color = clear_for_fog(map_fog.as_deref(), &fog, clock.as_deref());

    let far = if fog.zfar > 0.0 { fog.zfar } else { perspective.far };
    *view = prepare_scene_view(
        lens_world_from_parent_child(*host, *lens),
        perspective.fov,
        aspect,
        perspective.near,
        far,
        target_w,
        target_h,
        hud_t6::view_projection::znear_from_refdef(0.0, znear.value),
        depthhack.value,
    );
    lock_pvs.apply_after_stamp(&view);

    let (rigid, skinned) = &mut lod;
    update_lod_parms(view.tan_half_fov_y(), rigid, skinned);
}

#[cfg(test)]
mod tests {
    use asset_world::ExpFog;
    use bevy::camera::Viewport;
    use bevy::ecs::system::RunSystemOnce;

    use super::*;
    use crate::prepare::scene::camera::{FlyCamera, FpvLens};

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    fn exp_fog(halfway_dist: f32, color_rgb: [f32; 3]) -> ExpFog {
        ExpFog {
            start_dist: 0.0,
            halfway_dist,
            color_rgb,
            max_opacity: 1.0,
            transition_time: 0.0,
            sun: None,
            volumetric: None,
        }
    }

    fn fog_on(zfar: f32) -> FogDvars {
        FogDvars {
            enabled: true,
            zfar,
        }
    }

    #[test]
    fn the_projections_own_aspect_ratio_wins() {
        assert_eq!(aspect_of(2.0, &Camera::default()), 2.0);
        assert_eq!(aspect_of(1e-3, &Camera::default()), 1e-3);
    }

    #[test]
    fn a_projection_with_no_aspect_and_no_viewport_falls_back_to_widescreen() {
        assert_eq!(aspect_of(0.0, &Camera::default()), FALLBACK_ASPECT);
        assert_eq!(aspect_of(-1.0, &Camera::default()), FALLBACK_ASPECT);
    }

    #[test]
    fn the_target_clears_normally_without_map_fog() {
        assert!(matches!(
            clear_for_fog(None, &fog_on(1000.0), None),
            ClearColorConfig::Default
        ));
    }

    #[test]
    fn the_target_clears_normally_when_the_fog_has_no_far_plane_or_no_density() {
        let fog = MapFrameFog::new(exp_fog(500.0, [0.5, 0.5, 0.5]));
        assert!(matches!(
            clear_for_fog(Some(&fog), &fog_on(0.0), None),
            ClearColorConfig::Default
        ));
        let thin = MapFrameFog::new(exp_fog(0.0, [0.5, 0.5, 0.5]));
        assert!(matches!(
            clear_for_fog(Some(&thin), &fog_on(1000.0), None),
            ClearColorConfig::Default
        ));
    }

    #[test]
    fn with_fog_on_the_target_clears_to_the_fog_colour_in_eight_bits() {
        let fog = MapFrameFog::new(exp_fog(500.0, [0.5, -1.0, 1.5]));
        let ClearColorConfig::Custom(color) = clear_for_fog(Some(&fog), &fog_on(1000.0), None)
        else {
            panic!("fog did not set the clear colour");
        };
        let color = color.to_linear();
        assert!((color.red - 128.0 / 255.0).abs() < 1e-5);
        assert_eq!(color.green, 0.0);
        assert_eq!(color.blue, 1.0);
        assert_eq!(color.alpha, 1.0);
    }

    fn stamp_world(host: Option<Transform>, lens: Option<(Transform, Projection)>) -> World {
        let mut world = World::new();
        world.init_resource::<PreparedSceneView>();
        world.init_resource::<RLockPvs>();
        world.init_resource::<LodRampDvar>();
        world.insert_resource(LodRampSkinnedDvar::default());
        world.init_resource::<RZnearDvar>();
        world.insert_resource(fog_on(0.0));
        world.init_resource::<RZnearDepthhackDvar>();
        if let Some(host) = host {
            world.spawn((
                host,
                FlyCamera {
                    yaw: 0.0,
                    pitch: 0.0,
                    speed: 1.0,
                },
            ));
        }
        if let Some((transform, projection)) = lens {
            world.spawn((transform, projection, Camera::default(), FpvLens));
        }
        world
    }

    fn perspective(aspect_ratio: f32) -> Projection {
        Projection::Perspective(PerspectiveProjection {
            fov: 1.2,
            aspect_ratio,
            near: 2.0,
            far: 9000.0,
            ..Default::default()
        })
    }

    fn stamp(world: &mut World) -> PreparedSceneView {
        world.run_system_once(stamp_prepared_scene_view).unwrap();
        world.resource::<PreparedSceneView>().clone()
    }

    #[test]
    fn the_view_is_the_lens_in_the_host_with_the_projections_numbers() {
        let mut world = stamp_world(
            Some(Transform::from_xyz(100.0, 0.0, 0.0)),
            Some((Transform::from_xyz(0.0, 5.0, 0.0), perspective(2.0))),
        );
        let view = stamp(&mut world);
        assert!(view.ready);
        assert!(near(view.eye, Vec3::new(100.0, 5.0, 0.0)));
        assert!(near(view.forward, Vec3::NEG_Z));
        assert_eq!(
            (view.fov, view.aspect, view.near, view.far),
            (1.2, 2.0, 2.0, 9000.0)
        );
        assert_eq!(view.depth_hack_near, RZnearDepthhackDvar::default().value);
    }

    #[test]
    fn the_view_faces_where_the_host_faces() {
        let host = Transform::IDENTITY.looking_to(Vec3::Y, Vec3::Z);
        let mut world = stamp_world(Some(host), Some((Transform::IDENTITY, perspective(2.0))));
        let view = stamp(&mut world);
        assert!(near(view.forward, Vec3::Y));
        // A point ahead lands in the middle of the screen.
        let ahead = view.clip_from_world.project_point3(Vec3::new(0.0, 50.0, 0.0));
        assert!(ahead.x.abs() < 1e-4 && ahead.y.abs() < 1e-4);
    }

    #[test]
    fn a_projection_without_an_aspect_uses_widescreen() {
        let mut world = stamp_world(
            Some(Transform::IDENTITY),
            Some((Transform::IDENTITY, perspective(0.0))),
        );
        assert_eq!(stamp(&mut world).aspect, FALLBACK_ASPECT);
    }

    #[test]
    fn the_fogs_far_plane_replaces_the_projections_when_it_is_set() {
        let mut world = stamp_world(
            Some(Transform::IDENTITY),
            Some((Transform::IDENTITY, perspective(2.0))),
        );
        world.insert_resource(fog_on(750.0));
        assert_eq!(stamp(&mut world).far, 750.0);
    }

    #[test]
    fn the_scene_always_fills_the_whole_render_target() {
        let mut world = stamp_world(
            Some(Transform::IDENTITY),
            Some((Transform::IDENTITY, perspective(2.0))),
        );
        let lens = world
            .query_filtered::<Entity, With<FpvLens>>()
            .single(&world)
            .unwrap();
        world.get_mut::<Camera>(lens).unwrap().viewport = Some(Viewport::default());
        stamp(&mut world);
        assert!(world.get::<Camera>(lens).unwrap().viewport.is_none());
    }

    #[test]
    fn no_camera_leaves_the_empty_view() {
        for (host, lens) in [
            (None, Some((Transform::IDENTITY, perspective(2.0)))),
            (Some(Transform::IDENTITY), None),
            (None, None),
        ] {
            let mut world = stamp_world(host, lens);
            // A stale view from the last frame does not survive.
            world.resource_mut::<PreparedSceneView>().ready = true;
            let view = stamp(&mut world);
            assert!(!view.ready);
            assert!(view.frustum_planes.is_empty());
        }
    }

    #[test]
    fn an_orthographic_lens_gives_no_view() {
        let mut world = stamp_world(
            Some(Transform::IDENTITY),
            Some((
                Transform::IDENTITY,
                Projection::Orthographic(OrthographicProjection::default_3d()),
            )),
        );
        assert!(!stamp(&mut world).ready);
    }

    #[test]
    fn a_locked_pvs_freezes_the_first_view_it_sees() {
        let mut world = stamp_world(
            Some(Transform::from_xyz(1.0, 2.0, 3.0)),
            Some((Transform::IDENTITY, perspective(2.0))),
        );
        world.resource_mut::<RLockPvs>().enabled = true;
        stamp(&mut world);
        let frozen = world.resource::<RLockPvs>().frozen.clone().expect("frozen");
        assert!(near(frozen.eye, Vec3::new(1.0, 2.0, 3.0)));

        // The camera moves on; the frozen view stays.
        let host = world
            .query_filtered::<Entity, With<FlyCamera>>()
            .single(&world)
            .unwrap();
        world.get_mut::<Transform>(host).unwrap().translation = Vec3::new(50.0, 0.0, 0.0);
        stamp(&mut world);
        let still = world.resource::<RLockPvs>().frozen.clone().expect("frozen");
        assert!(near(still.eye, Vec3::new(1.0, 2.0, 3.0)));

        world.resource_mut::<RLockPvs>().enabled = false;
        stamp(&mut world);
        assert!(world.resource::<RLockPvs>().frozen.is_none());
    }
}
