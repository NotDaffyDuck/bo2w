//! Distance fog as the frame sees it: the map's value, the blend toward a new
//! value when a script changes it, and the debug sliders that override it.

use asset_world::{ExpFog, SunFog};
use bevy::prelude::*;

/// Console switches for fog.
#[derive(Resource)]
pub struct FogDvars {
    pub enabled: bool,
    pub zfar: f32,
}

impl Default for FogDvars {
    fn default() -> Self {
        Self {
            enabled: true,
            zfar: 0.0,
        }
    }
}

/// Distance at which the "fog is off" value starts and is half-way: far
/// enough that nothing in a map reaches it.
const OFF_START_DIST: f32 = 100_000_000_000.0;

/// Sun fog used until a map supplies its own, so the sliders have something to
/// edit.
const PLACEHOLDER_SUN: SunFog = SunFog {
    color_rgb: [1.0, 0.0, 0.0],
    sun_dir: [1.0, 0.0, 0.0],
    begin_angle_deg: 0.0,
    end_angle_deg: 180.0,
    scale: 1.0,
};

/// A fog that hides nothing.
fn fog_that_is_off() -> ExpFog {
    ExpFog {
        start_dist: OFF_START_DIST,
        halfway_dist: OFF_START_DIST + 1.0,
        color_rgb: [0.0; 3],
        max_opacity: 0.0,
        transition_time: 0.0,
        sun: None,
        volumetric: None,
    }
}

/// The fog in effect on the map and what is needed to move it smoothly to a
/// new value.
#[derive(Resource, Clone, Copy, Debug)]
pub struct MapFrameFog {
    /// Where the fog is heading (or has arrived).
    pub fog: ExpFog,
    /// Values the debug sliders edit.
    pub sliders: ExpFog,
    pub slider_sun: SunFog,
    /// The last sun fog a script set, kept for read-outs.
    pub readonly_sun: SunFog,
    pub art_tweak: bool,
    pub script_disabled: bool,
    previous: ExpFog,
    start_ms: i32,
    duration_ms: i32,
}

impl MapFrameFog {
    pub fn new(fog: ExpFog) -> Self {
        let sun = fog.sun.unwrap_or(PLACEHOLDER_SUN);
        Self {
            fog,
            sliders: fog,
            slider_sun: sun,
            readonly_sun: sun,
            art_tweak: false,
            script_disabled: false,
            previous: fog,
            start_ms: 0,
            duration_ms: 0,
        }
    }

    /// Start moving to `fog` at `time_ms`, from wherever the fog is right
    /// now. Fog that is currently off jumps straight to the new value instead
    /// of fading in.
    pub fn set(&mut self, fog: ExpFog, time_ms: i32) {
        let here = self.sample(time_ms);
        self.duration_ms = match here.density() {
            d if d == 0.0 => 0,
            _ => (fog.transition_time * 1000.0).round() as i32,
        };
        self.previous = here;
        self.start_ms = time_ms;
        if let Some(sun) = fog.sun {
            self.readonly_sun = sun;
        }
        self.fog = fog;
    }

    /// Make the slider values the target, instantly, or switch the fog off if
    /// scripts have disabled it.
    pub fn apply_sliders(&mut self, time_ms: i32) {
        let target = if self.script_disabled {
            fog_that_is_off()
        } else {
            ExpFog {
                transition_time: 0.0,
                sun: self.sliders.sun.map(|_| self.slider_sun),
                ..self.sliders
            }
        };
        self.set(target, time_ms);
    }

    /// The fog at `time_ms`.
    pub fn sample(&self, time_ms: i32) -> ExpFog {
        let blend = sim::ScriptFog {
            from: to_script(self.previous),
            to: to_script(self.fog),
            start_ms: self.start_ms,
            duration_ms: self.duration_ms,
        };
        ExpFog {
            transition_time: self.fog.transition_time,
            volumetric: self.fog.volumetric,
            ..from_script(blend.sample(time_ms))
        }
    }
}

fn to_script_sun(sun: SunFog) -> sim::ScriptSunFog {
    sim::ScriptSunFog {
        color_rgb: sun.color_rgb,
        sun_dir: sun.sun_dir,
        begin_angle_deg: sun.begin_angle_deg,
        end_angle_deg: sun.end_angle_deg,
        scale: sun.scale,
    }
}

fn from_script_sun(sun: sim::ScriptSunFog) -> SunFog {
    SunFog {
        color_rgb: sun.color_rgb,
        sun_dir: sun.sun_dir,
        begin_angle_deg: sun.begin_angle_deg,
        end_angle_deg: sun.end_angle_deg,
        scale: sun.scale,
    }
}

fn to_script(fog: ExpFog) -> sim::ScriptFogParams {
    sim::ScriptFogParams {
        start_dist: fog.start_dist,
        halfway_dist: fog.halfway_dist,
        color_rgb: fog.color_rgb,
        max_opacity: fog.max_opacity,
        sun: fog.sun.map(to_script_sun),
    }
}

fn from_script(fog: sim::ScriptFogParams) -> ExpFog {
    ExpFog {
        start_dist: fog.start_dist,
        halfway_dist: fog.halfway_dist,
        color_rgb: fog.color_rgb,
        max_opacity: fog.max_opacity,
        transition_time: 0.0,
        volumetric: None,
        sun: fog.sun.map(from_script_sun),
    }
}

/// What the script fog presentation has applied so far, and the map fog to
/// return to when the script lets go.
#[derive(Resource, Default)]
pub(crate) struct ScriptFogPresentation {
    applied: Option<sim::ScriptFog>,
    baseline: Option<MapFrameFog>,
}

/// Follow the fog the scripts have set in the presented snapshot. While one
/// is active it replaces the map's fog; when it goes away the map's fog
/// comes back (or, if there never was one, the fog is removed).
pub(crate) fn sync_script_fog(
    scene: Res<crate::prepare::scene::world::WorldScene>,
    presented: Res<net::PresentedSnapshot>,
    mut state: ResMut<ScriptFogPresentation>,
    mut map_fog: Option<ResMut<MapFrameFog>>,
    mut commands: Commands,
) {
    if !scene.spawned {
        return;
    }
    let wanted = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.objectives.fog);
    if wanted == state.applied {
        return;
    }
    if wanted.is_some() && state.applied.is_none() {
        state.baseline = map_fog.as_deref().copied();
    }
    let mut install = |fog: MapFrameFog| match map_fog.as_deref_mut() {
        Some(current) => *current = fog,
        None => commands.insert_resource(fog),
    };
    match wanted {
        Some(script) => {
            install(MapFrameFog {
                previous: from_script(script.from),
                start_ms: script.start_ms,
                duration_ms: script.duration_ms,
                ..MapFrameFog::new(from_script(script.to))
            });
        }
        None => match state.baseline.take() {
            Some(baseline) => install(baseline),
            None => commands.remove_resource::<MapFrameFog>(),
        },
    }
    state.applied = wanted;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(halfway: f32, transition: f32) -> ExpFog {
        ExpFog {
            start_dist: 10.0,
            halfway_dist: halfway,
            color_rgb: [0.5, 0.25, 0.75],
            max_opacity: 0.9,
            transition_time: transition,
            sun: None,
            volumetric: None,
        }
    }

    #[test]
    fn dvars_start_enabled_with_no_far_plane() {
        let dvars = FogDvars::default();
        assert!(dvars.enabled);
        assert_eq!(dvars.zfar, 0.0);
    }

    #[test]
    fn a_map_without_sun_fog_gets_the_placeholder_for_its_sliders() {
        let state = MapFrameFog::new(fog(100.0, 0.0));
        assert_eq!(state.slider_sun, PLACEHOLDER_SUN);
        assert_eq!(state.readonly_sun, PLACEHOLDER_SUN);
    }

    #[test]
    fn a_map_with_sun_fog_shows_it_on_the_sliders() {
        let sun = SunFog {
            scale: 3.0,
            ..PLACEHOLDER_SUN
        };
        let state = MapFrameFog::new(ExpFog {
            sun: Some(sun),
            ..fog(100.0, 0.0)
        });
        assert_eq!(state.slider_sun, sun);
    }

    #[test]
    fn fog_that_was_off_jumps_to_the_new_value() {
        let mut state = MapFrameFog::new(fog(0.0, 0.0));
        state.set(fog(200.0, 4.0), 1000);
        assert_eq!(state.sample(1000).halfway_dist, 200.0);
    }

    #[test]
    fn fog_that_was_on_fades_over_the_transition_time() {
        let mut state = MapFrameFog::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.0), 5000);
        let start = state.sample(5000);
        let middle = state.sample(6000);
        let end = state.sample(7000);
        assert!((start.halfway_dist - 100.0).abs() < 1e-3);
        assert!(middle.halfway_dist > 100.0 && middle.halfway_dist < 300.0);
        assert!((end.halfway_dist - 300.0).abs() < 1e-3);
        assert_eq!(state.sample(9999).halfway_dist, end.halfway_dist);
    }

    #[test]
    fn a_new_target_starts_from_the_current_blend() {
        let mut state = MapFrameFog::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.0), 0);
        let halfway_there = state.sample(1000);
        state.set(fog(50.0, 2.0), 1000);
        let right_after = state.sample(1000);
        assert!((right_after.halfway_dist - halfway_there.halfway_dist).abs() < 1e-3);
    }

    #[test]
    fn samples_report_the_targets_transition_time() {
        let mut state = MapFrameFog::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.5), 0);
        assert_eq!(state.sample(10).transition_time, 2.5);
    }

    #[test]
    fn a_sun_fog_set_by_a_script_is_remembered() {
        let sun = SunFog {
            scale: 9.0,
            ..PLACEHOLDER_SUN
        };
        let mut state = MapFrameFog::new(fog(100.0, 0.0));
        state.set(
            ExpFog {
                sun: Some(sun),
                ..fog(100.0, 0.0)
            },
            0,
        );
        assert_eq!(state.readonly_sun, sun);
    }

    #[test]
    fn sliders_apply_at_once_with_the_slider_sun() {
        let sun = SunFog {
            scale: 4.0,
            ..PLACEHOLDER_SUN
        };
        let mut state = MapFrameFog::new(ExpFog {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(100.0, 0.0)
        });
        state.sliders = ExpFog {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(80.0, 6.0)
        };
        state.slider_sun = sun;
        state.apply_sliders(0);
        assert_eq!(state.fog.transition_time, 0.0);
        assert_eq!(state.fog.sun, Some(sun));
        assert_eq!(state.fog.halfway_dist, 80.0);
    }

    #[test]
    fn script_disabled_fog_is_switched_off() {
        let mut state = MapFrameFog::new(fog(100.0, 0.0));
        state.script_disabled = true;
        state.apply_sliders(0);
        assert_eq!(state.fog.max_opacity, 0.0);
        assert_eq!(state.fog.start_dist, OFF_START_DIST);
        assert!(state.fog.sun.is_none());
    }

    #[test]
    fn conversion_to_script_form_and_back_keeps_the_fog() {
        let original = ExpFog {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(120.0, 0.0)
        };
        assert_eq!(from_script(to_script(original)), original);
    }
}
