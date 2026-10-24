//! The sun's sprite, flare and the blinding/glare overlays, worked out once
//! per frame from where the camera is looking and handed to the renderer.

use bevy::prelude::*;
use net::FrameClock;
use render_frame::{SunEffectsDef, SunEffectsFrame, angular_lerp};

use super::command_context::MapSunEffects;
use crate::prepare::scene::camera::FpvLens;
use crate::prepare::scene::view_parms::PreparedSceneView;

/// What the sun effects need this frame, or `None` when the map has none.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SunEffectsFrameInput {
    pub frame: Option<SunEffectsFrame>,
}

/// The camera as seen last frame, to tell a jump from ordinary motion.
#[derive(Default)]
pub(crate) struct SunEffectsCameraTrack {
    world_generation: Option<u64>,
    camera_key: u64,
    origin: Vec3,
    armed: bool,
}

/// Moving the camera this far in one frame counts as a cut.
const CAMERA_CUT_ORIGIN: f32 = 512.0;

/// Frame length assumed when the clock has none to give.
const FALLBACK_FRAME_MS: i32 = 10;

impl SunEffectsCameraTrack {
    /// Record this frame's camera and say whether it is a cut from the last
    /// one: the first frame, a new world, a different camera, or a big jump.
    fn observe(&mut self, generation: Option<u64>, camera_key: u64, origin: Vec3) -> bool {
        let cut = !self.armed
            || self.world_generation != generation
            || self.camera_key != camera_key
            || origin.distance(self.origin) > CAMERA_CUT_ORIGIN;
        *self = Self {
            world_generation: generation,
            camera_key,
            origin,
            armed: true,
        };
        cut
    }
}

/// Everything about this frame's view that the effects depend on.
struct SunShot {
    world_generation: Option<u64>,
    camera_key: u64,
    camera_cut: bool,
    dt_ms: i32,
    /// Cosine of the angle between the view direction and the sun.
    view_dot: f32,
    /// The sun direction in clip space (a point at infinity).
    clip: Vec4,
    viewport: (f32, f32),
}

/// An overlay that only exists when its peak strength is above zero.
fn overlay_strength(peak: f32, dot: f32, min_dot: f32, max_dot: f32) -> f32 {
    if peak > 0.0 {
        (angular_lerp(dot, min_dot, max_dot) * peak).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn describe(def: &SunEffectsDef, shot: &SunShot) -> SunEffectsFrame {
    let flare_t = angular_lerp(shot.view_dot, def.flare_min_dot, def.flare_max_dot);
    SunEffectsFrame {
        world_generation: shot.world_generation,
        camera_key: shot.camera_key,
        camera_cut: shot.camera_cut,
        dt_ms: shot.dt_ms,
        view_dot: shot.view_dot,
        clip: shot.clip.to_array(),
        viewport_w: shot.viewport.0,
        viewport_h: shot.viewport.1,
        sprite_size: def.sprite_size.max(0.0),
        flare_size: def.flare_min_size + flare_t * def.flare_max_size,
        flare_alpha: (flare_t * def.flare_max_alpha).clamp(0.0, 1.0),
        flare_fade_in_ms: def.flare_fade_in_ms.max(0),
        flare_fade_out_ms: def.flare_fade_out_ms.max(0),
        blind_goal: overlay_strength(
            def.blind_max_darken,
            shot.view_dot,
            def.blind_min_dot,
            def.blind_max_dot,
        ),
        blind_fade_in_ms: def.blind_fade_in_ms.max(0),
        blind_fade_out_ms: def.blind_fade_out_ms.max(0),
        glare_goal: overlay_strength(
            def.glare_max_lighten,
            shot.view_dot,
            def.glare_min_dot,
            def.glare_max_dot,
        ),
        glare_fade_in_ms: def.glare_fade_in_ms.max(0),
        glare_fade_out_ms: def.glare_fade_out_ms.max(0),
        behind_camera: !(shot.clip.w.is_finite() && shot.clip.w > 0.0),
    }
}

/// Work out the sun effects for the frame about to be drawn. With no sun
/// effects on the map, or no ready view, the published frame is cleared; the
/// camera history is dropped only in the first case.
pub(crate) fn publish_sun_effects_frame(
    mut published: ResMut<SunEffectsFrameInput>,
    mut track: Local<SunEffectsCameraTrack>,
    sun: Option<Res<MapSunEffects>>,
    prepared: Option<Res<PreparedSceneView>>,
    clock: Option<Res<FrameClock>>,
    world_generation: Option<Res<frame::WorldGeneration>>,
    cameras: Query<(Entity, &GlobalTransform), (With<Camera3d>, Without<FpvLens>)>,
) {
    let Some(def) = sun.and_then(|sun| sun.def) else {
        *published = SunEffectsFrameInput::default();
        *track = SunEffectsCameraTrack::default();
        return;
    };
    let Some(view) = prepared.filter(|view| view.ready) else {
        *published = SunEffectsFrameInput::default();
        return;
    };
    let generation = world_generation.and_then(|generation| generation.0);
    let (camera_key, origin) = match cameras.iter().next() {
        Some((entity, transform)) => (entity.to_bits(), transform.translation()),
        None => (0, view.eye),
    };
    let camera_cut = track.observe(generation, camera_key, origin);

    let direction = Vec3::from_array(def.direction);
    let shot = SunShot {
        world_generation: generation,
        camera_key,
        camera_cut,
        dt_ms: clock
            .map(|clock| clock.frametime())
            .filter(|dt| *dt > 0)
            .unwrap_or(FALLBACK_FRAME_MS),
        view_dot: direction.dot(view.forward),
        clip: view.clip_from_world * direction.extend(0.0),
        viewport: (view.rt_w.max(1) as f32, view.rt_h.max(1) as f32),
    };
    published.frame = Some(describe(&def, &shot));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> SunEffectsDef {
        SunEffectsDef {
            sprite_size: 4.0,
            flare_min_size: 10.0,
            flare_min_dot: 0.5,
            flare_max_size: 30.0,
            flare_max_dot: 1.0,
            flare_max_alpha: 0.8,
            flare_fade_in_ms: 100,
            flare_fade_out_ms: 200,
            blind_min_dot: 0.9,
            blind_max_dot: 1.0,
            blind_max_darken: 0.6,
            blind_fade_in_ms: 50,
            blind_fade_out_ms: 60,
            glare_min_dot: 0.0,
            glare_max_dot: 1.0,
            glare_max_lighten: 0.0,
            glare_fade_in_ms: 5,
            glare_fade_out_ms: 6,
            direction: [0.0, 1.0, 0.0],
            ..Default::default()
        }
    }

    fn shot(view_dot: f32, clip_w: f32) -> SunShot {
        SunShot {
            world_generation: Some(2),
            camera_key: 7,
            camera_cut: false,
            dt_ms: 16,
            view_dot,
            clip: Vec4::new(0.0, 0.0, 0.0, clip_w),
            viewport: (1920.0, 1080.0),
        }
    }

    #[test]
    fn the_first_frame_is_a_cut() {
        let mut track = SunEffectsCameraTrack::default();
        assert!(track.observe(Some(1), 3, Vec3::ZERO));
        assert!(!track.observe(Some(1), 3, Vec3::ONE));
    }

    #[test]
    fn a_new_world_or_camera_is_a_cut() {
        let mut track = SunEffectsCameraTrack::default();
        track.observe(Some(1), 3, Vec3::ZERO);
        assert!(track.observe(Some(2), 3, Vec3::ZERO));
        assert!(track.observe(Some(2), 4, Vec3::ZERO));
        assert!(!track.observe(Some(2), 4, Vec3::ZERO));
    }

    #[test]
    fn only_a_big_jump_counts_as_a_cut() {
        let mut track = SunEffectsCameraTrack::default();
        track.observe(None, 1, Vec3::ZERO);
        assert!(!track.observe(None, 1, Vec3::new(CAMERA_CUT_ORIGIN, 0.0, 0.0)));
        assert!(track.observe(None, 1, Vec3::new(2.0 * CAMERA_CUT_ORIGIN + 1.0, 0.0, 0.0)));
    }

    #[test]
    fn looking_away_gives_no_flare_but_keeps_the_sprite() {
        let frame = describe(&def(), &shot(0.0, 1.0));
        assert_eq!(frame.flare_alpha, 0.0);
        assert_eq!(frame.flare_size, 10.0);
        assert_eq!(frame.sprite_size, 4.0);
    }

    #[test]
    fn looking_straight_at_the_sun_gives_the_full_flare() {
        let frame = describe(&def(), &shot(1.0, 1.0));
        assert_eq!(frame.flare_alpha, 0.8);
        assert_eq!(frame.flare_size, 40.0);
        assert_eq!(frame.blind_goal, 0.6);
    }

    #[test]
    fn an_overlay_with_no_strength_never_shows() {
        let frame = describe(&def(), &shot(1.0, 1.0));
        assert_eq!(frame.glare_goal, 0.0);
    }

    #[test]
    fn a_sun_behind_the_camera_is_flagged() {
        assert!(describe(&def(), &shot(1.0, -1.0)).behind_camera);
        assert!(describe(&def(), &shot(1.0, 0.0)).behind_camera);
        assert!(describe(&def(), &shot(1.0, f32::NAN)).behind_camera);
        assert!(!describe(&def(), &shot(1.0, 0.5)).behind_camera);
    }

    #[test]
    fn negative_authored_times_and_sizes_are_clamped() {
        let odd = SunEffectsDef {
            sprite_size: -3.0,
            flare_fade_in_ms: -1,
            blind_fade_out_ms: -9,
            ..def()
        };
        let frame = describe(&odd, &shot(1.0, 1.0));
        assert_eq!(frame.sprite_size, 0.0);
        assert_eq!(frame.flare_fade_in_ms, 0);
        assert_eq!(frame.blind_fade_out_ms, 0);
    }

    #[test]
    fn the_shot_is_carried_through() {
        let frame = describe(&def(), &shot(0.7, 2.0));
        assert_eq!(frame.world_generation, Some(2));
        assert_eq!(frame.camera_key, 7);
        assert_eq!(frame.dt_ms, 16);
        assert_eq!((frame.viewport_w, frame.viewport_h), (1920.0, 1080.0));
        assert_eq!(frame.clip[3], 2.0);
    }
}
