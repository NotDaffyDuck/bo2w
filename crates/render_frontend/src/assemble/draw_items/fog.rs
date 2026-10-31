//! Distance fog as the frame sees it: the map's value, the blend toward a new
//! value when a script changes it, and the debug sliders that override it.

use asset_world::{ExpHaze, SunlitMist};
use bevy::prelude::*;

/// Console switches for fog.
#[derive(Resource)]
pub struct HazeTunables {
    pub enabled: bool, pub far_clip: f32,
}

impl Default for HazeTunables {
    fn default() -> Self {
        let enabled = true;
        Self { enabled, far_clip: 0.0 }
    }
}

/// Distance at which the "fog is off" value starts and is half-way: far
/// enough that nothing in a map reaches it.
const OFF_START_DIST: f32 = 100_000_000_000.0;

/// Sun fog used until a map supplies its own, so the sliders have something to
/// edit.
const SUN_ARC_DEG: [f32; 2] = [0.0, 180.0];

const PLACEHOLDER_SUN: SunlitMist = SunlitMist {
    tint_rgb: Vec3::X.to_array(), toward_sun: Vec3::X.to_array(),
    inner_cone_deg: SUN_ARC_DEG[0], outer_cone_deg: SUN_ARC_DEG[1], scale: 1.0,
};

/// A fog that hides nothing.
fn fog_that_is_off() -> ExpHaze {
    let volumetric = None;
    let (black, clear) = ([0.0; 3], 0.0);
    ExpHaze {
        near_edge: OFF_START_DIST,
        half_density_at: OFF_START_DIST + 1.0,
        tint_rgb: black, opacity_cap: clear,
        fade_duration: 0.0,
        sun: None,
        volumetric,
    }
}

/// The fog in effect on the map and what is needed to move it smoothly to a
/// new value.
#[derive(Resource, Clone, Copy, Debug)]
pub struct MapFrameHaze {
    /// Where the fog is heading (or has arrived).
    pub fog: ExpHaze,
    /// Values the debug sliders edit.
    pub sliders: ExpHaze,
    pub slider_daylight: SunlitMist,
    /// The last sun fog a script set, kept for read-outs.
    pub readonly_daylight: SunlitMist,
    pub art_trim: bool,
    pub scripts_off: bool,
    previous: ExpHaze,
    begin_ms: i32,
    span_ms: i32,
}

impl MapFrameHaze {
    pub fn new(haze: ExpHaze) -> Self {
        let daylight = haze.sun.unwrap_or(PLACEHOLDER_SUN);
        Self {
            fog: haze, sliders: haze,
            slider_daylight: daylight, readonly_daylight: daylight,
            art_trim: false, scripts_off: false,
            previous: haze, begin_ms: 0, span_ms: 0,
        }
    }

    /// Start moving to `target` at `now_ms`, from wherever the fog is right now.
    /// Fog that is currently off jumps straight to the new value instead of
    /// fading in.
    pub fn set(&mut self, target: ExpHaze, now_ms: i32) {
        let here = self.fog_at(now_ms);
        self.span_ms = match here.thickness_at() {
            d if d == 0.0 => 0,
            _ => (target.fade_duration * 1000.0).round() as i32,
        };
        self.previous = here;
        self.begin_ms = now_ms;
        if let Some(daylight) = target.sun {
            self.readonly_daylight = daylight;
        }
        self.fog = target;
    }

    /// Make the slider values the target, instantly, or switch the fog off if
    /// scripts have disabled it.
    pub fn pull_tunables(&mut self, clock_stamp_ms: i32) {
        let target = if self.scripts_off {
            fog_that_is_off()
        } else {
            ExpHaze {
                fade_duration: 0.0,
                sun: self.sliders.sun.map(|_| self.slider_daylight),
                ..self.sliders
            }
        };
        self.set(target, clock_stamp_ms);
    }

    /// The fog at `clock_stamp_ms`.
    pub fn fog_at(&self, clock_stamp_ms: i32) -> ExpHaze {
        let blend = sim::ScriptFog {
            from: to_script(self.previous),
            to: to_script(self.fog),
            began_ms: self.begin_ms,
            lasts_ms: self.span_ms,
        };
        ExpHaze {
            fade_duration: self.fog.fade_duration,
            volumetric: self.fog.volumetric,
            ..from_script(blend.sample(clock_stamp_ms))
        }
    }
}

fn to_script_sun(sun: SunlitMist) -> sim::ScriptSunFog {
    sim::ScriptSunFog {
        tint_rgb: sun.tint_rgb,
        sun_way: sun.toward_sun,
        from_deg: sun.inner_cone_deg,
        to_deg: sun.outer_cone_deg,
        scale: sun.scale,
    }
}

fn from_script_sun(sun: sim::ScriptSunFog) -> SunlitMist {
    SunlitMist {
        tint_rgb: sun.tint_rgb,
        toward_sun: sun.sun_way,
        inner_cone_deg: sun.from_deg,
        outer_cone_deg: sun.to_deg,
        scale: sun.scale,
    }
}

fn to_script(fog: ExpHaze) -> sim::ScriptFogParams {
    sim::ScriptFogParams {
        from_dist: fog.near_edge,
        half_dist: fog.half_density_at,
        tint_rgb: fog.tint_rgb,
        thickest: fog.opacity_cap,
        sun: fog.sun.map(to_script_sun),
    }
}

fn from_script(fog: sim::ScriptFogParams) -> ExpHaze {
    let volumetric = None;
    ExpHaze {
        near_edge: fog.from_dist,
        half_density_at: fog.half_dist,
        tint_rgb: fog.tint_rgb,
        opacity_cap: fog.thickest,
        fade_duration: 0.0,
        volumetric,
        sun: fog.sun.map(from_script_sun),
    }
}

/// What the script fog presentation has applied so far, and the map fog to
/// return to when the script lets go.
#[derive(Resource, Default)]
pub(crate) struct ScriptFogPresentation {
    applied: Option<sim::ScriptFog>,
    baseline: Option<MapFrameHaze>,
}

/// Follow the fog the scripts have set in the presented snapshot. While one
/// is active it replaces the map's fog; when it goes away the map's fog
/// comes back (or, if there never was one, the fog is removed).
pub(crate) fn sync_script_fog(
    scene: Res<crate::prepare::scene::world::LevelStage>,
    presented: Res<net::ShownWorldFrame>,
    mut state: ResMut<ScriptFogPresentation>,
    mut map_fog: Option<ResMut<MapFrameHaze>>,
    mut commands: Commands,
) {
    if !scene.spawned {
        return;
    }
    let shown = presented.snapshot();
    let wanted = shown.and_then(|snap| snap.meta.objectives.fog);
    if wanted == state.applied {
        return;
    }
    if wanted.is_some() && state.applied.is_none() {
        state.baseline = map_fog.as_deref().copied();
    }
    let mut install = |fog: MapFrameHaze| match map_fog.as_deref_mut() {
        Some(current) => *current = fog,
        None => commands.insert_resource(fog),
    };
    match wanted {
        Some(script) => {
            install(MapFrameHaze {
                previous: from_script(script.from),
                begin_ms: script.began_ms,
                span_ms: script.lasts_ms,
                ..MapFrameHaze::new(from_script(script.to))
            });
        }
        None => match state.baseline.take() {
            Some(baseline) => install(baseline),
            None => commands.remove_resource::<MapFrameHaze>(),
        },
    }
    state.applied = wanted;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(halfway: f32, transition: f32) -> ExpHaze {
        let volumetric = None;
        ExpHaze {
            near_edge: 10.0,
            half_density_at: halfway,
            tint_rgb: Vec3::new(0.5, 0.25, 0.75).to_array(),
            opacity_cap: 0.9,
            fade_duration: transition,
            sun: None,
            volumetric,
        }
    }

    #[test]
    fn dvars_start_enabled_with_no_far_plane() {
        let dvars = HazeTunables::default();
        assert!(dvars.enabled);
        assert_eq!(dvars.far_clip, 0.0);
    }

    #[test]
    fn a_map_without_sun_fog_gets_the_placeholder_for_its_sliders() {
        let state = MapFrameHaze::new(fog(100.0, 0.0));
        assert_eq!(state.slider_daylight, PLACEHOLDER_SUN);
        assert_eq!(state.readonly_daylight, PLACEHOLDER_SUN);
    }

    #[test]
    fn a_map_with_sun_fog_shows_it_on_the_sliders() {
        let sun = SunlitMist {
            scale: 3.0,
            ..PLACEHOLDER_SUN
        };
        let state = MapFrameHaze::new(ExpHaze {
            sun: Some(sun),
            ..fog(100.0, 0.0)
        });
        assert_eq!(state.slider_daylight, sun);
    }

    #[test]
    fn fog_that_was_off_jumps_to_the_new_value() {
        let mut state = MapFrameHaze::new(fog(0.0, 0.0));
        state.set(fog(200.0, 4.0), 1000);
        assert_eq!(state.fog_at(1000).half_density_at, 200.0);
    }

    #[test]
    fn fog_that_was_on_fades_over_the_transition_time() {
        let mut state = MapFrameHaze::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.0), 5000);
        let start = state.fog_at(5000);
        let middle = state.fog_at(6000);
        let end = state.fog_at(7000);
        assert!((start.half_density_at - 100.0).abs() < 1e-3);
        assert!(middle.half_density_at > 100.0 && middle.half_density_at < 300.0);
        assert!((end.half_density_at - 300.0).abs() < 1e-3);
        assert_eq!(state.fog_at(9999).half_density_at, end.half_density_at);
    }

    #[test]
    fn a_new_target_starts_from_the_current_blend() {
        let mut state = MapFrameHaze::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.0), 0);
        let halfway_there = state.fog_at(1000);
        state.set(fog(50.0, 2.0), 1000);
        let right_after = state.fog_at(1000);
        assert!((right_after.half_density_at - halfway_there.half_density_at).abs() < 1e-3);
    }

    #[test]
    fn samples_report_the_targets_transition_time() {
        let mut state = MapFrameHaze::new(fog(100.0, 0.0));
        state.set(fog(300.0, 2.5), 0);
        assert_eq!(state.fog_at(10).fade_duration, 2.5);
    }

    #[test]
    fn a_sun_fog_set_by_a_script_is_remembered() {
        let sun = SunlitMist {
            scale: 9.0,
            ..PLACEHOLDER_SUN
        };
        let mut state = MapFrameHaze::new(fog(100.0, 0.0));
        state.set(
            ExpHaze {
                sun: Some(sun),
                ..fog(100.0, 0.0)
            },
            0,
        );
        assert_eq!(state.readonly_daylight, sun);
    }

    #[test]
    fn sliders_apply_at_once_with_the_slider_sun() {
        let sun = SunlitMist {
            scale: 4.0,
            ..PLACEHOLDER_SUN
        };
        let mut state = MapFrameHaze::new(ExpHaze {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(100.0, 0.0)
        });
        state.sliders = ExpHaze {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(80.0, 6.0)
        };
        state.slider_daylight = sun;
        state.pull_tunables(0);
        assert_eq!(state.fog.fade_duration, 0.0);
        assert_eq!(state.fog.sun, Some(sun));
        assert_eq!(state.fog.half_density_at, 80.0);
    }

    #[test]
    fn script_disabled_fog_is_switched_off() {
        let mut state = MapFrameHaze::new(fog(100.0, 0.0));
        state.scripts_off = true;
        state.pull_tunables(0);
        assert_eq!(state.fog.opacity_cap, 0.0);
        assert_eq!(state.fog.near_edge, OFF_START_DIST);
        assert!(state.fog.sun.is_none());
    }

    #[test]
    fn conversion_to_script_form_and_back_keeps_the_fog() {
        let original = ExpHaze {
            sun: Some(PLACEHOLDER_SUN),
            ..fog(120.0, 0.0)
        };
        assert_eq!(from_script(to_script(original)), original);
    }
}
