//! The colour grade the frame is drawn with: the map's film vision, the
//! vision set the scripts ask for (naked, night, thermal, missile), the pain
//! overlay, the player's brightness setting and any `r_film*` dvars the
//! scripts push, mixed into one `FilmVision` per frame.

use asset_world::FilmVision;
use bevy::prelude::*;
use hud_t6::vision_set::{self, VisionSetLerpStyle, VisionSetMode};
use lighting_t6::GlowViewInfo;
use sim::{ObjectiveMatch, ViewEffects, VisionChange};

use super::dof::GlowDvars;
use crate::prepare::scene::world::WorldScene;

/// A colour grade with no hue shift, unit gamma, no exposure and full colour.
const NEUTRAL_GRADING: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The vision with nothing switched on, for when there is none.
fn plain_vision() -> FilmVision {
    FilmVision::default()
}

/// Past this health fraction the pain overlay starts to show.
const PAIN_HEALTH: f32 = 0.5;

/// The vision of the frame being drawn and what it was mixed from.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct FilmVisionView {
    pub current: Option<FilmVision>,
    pub script_forced: bool,
    /// Hue in radians, gamma, exposure in stops, saturation.
    pub grading: [f32; 4],
    pub blur: f32,
    /// The lerp between vision sets; the film vision is its payload.
    mode: VisionSetMode<FilmVision>,
    last_map: Option<FilmVision>,
    override_vision: Option<FilmVision>,
}

impl Default for FilmVisionView {
    fn default() -> Self {
        Self {
            current: None,
            script_forced: false,
            grading: NEUTRAL_GRADING,
            blur: 0.0,
            mode: VisionSetMode::default(),
            last_map: None,
            override_vision: None,
        }
    }
}

impl FilmVisionView {
    /// Fade to `preset`, or back to `map` when there is none. The first
    /// selection starts from the map.
    pub fn select(
        &mut self,
        map: Option<FilmVision>,
        preset: Option<FilmVision>,
        now_ms: i32,
        duration_ms: i32,
    ) {
        if self.last_map.is_none() {
            self.mode.current = map.unwrap_or_else(plain_vision);
            self.mode.lerp.style = VisionSetLerpStyle::None;
        }
        self.mode.update(now_ms, lerp_film_vision);
        let target = preset.or(map).unwrap_or_else(plain_vision);
        self.mode
            .start_lerp_to(now_ms, VisionSetLerpStyle::ToSmooth, target, duration_ms);
        self.last_map = Some(target);
        self.override_vision = preset;
    }
}

/// Every field of a vision lerped by its type.
#[must_use]
pub fn lerp_film_vision(
    from: FilmVision,
    to: FilmVision,
    fraction: f32,
    style: VisionSetLerpStyle,
) -> FilmVision {
    let float = |a, b| vision_set::lerp_float(a, b, fraction, style);
    let flag = |a, b| vision_set::lerp_bool(a, b, fraction, style);
    let tint = |a, b| vision_set::lerp_vec3(a, b, fraction, style);
    FilmVision {
        enable: flag(from.enable, to.enable),
        contrast: float(from.contrast, to.contrast),
        brightness: float(from.brightness, to.brightness),
        desaturation: float(from.desaturation, to.desaturation),
        desaturation_dark: float(from.desaturation_dark, to.desaturation_dark),
        invert: flag(from.invert, to.invert),
        light_tint: tint(from.light_tint, to.light_tint),
        medium_tint: tint(from.medium_tint, to.medium_tint),
        dark_tint: tint(from.dark_tint, to.dark_tint),
        glow_enable: flag(from.glow_enable, to.glow_enable),
        glow_radius: float(from.glow_radius, to.glow_radius),
        glow_bloom_cutoff: float(from.glow_bloom_cutoff, to.glow_bloom_cutoff),
        glow_bloom_desaturation: float(from.glow_bloom_desaturation, to.glow_bloom_desaturation),
        glow_bloom_intensity: float(from.glow_bloom_intensity, to.glow_bloom_intensity),
    }
}

/// The map's vision, once there is a view to draw it in.
#[must_use]
pub fn presented_film_vision(
    view_ready: bool,
    map_vision: Option<FilmVision>,
) -> Option<FilmVision> {
    map_vision.filter(|_| view_ready)
}

/// `vision` with its glow settings swapped for the tweak dvars'.
fn with_glow(mut vision: FilmVision, glow: GlowViewInfo) -> FilmVision {
    vision.glow_enable = glow.enable;
    vision.glow_bloom_cutoff = glow.cutoff;
    vision.glow_bloom_desaturation = glow.desaturation;
    vision.glow_bloom_intensity = glow.intensity;
    vision.glow_radius = glow.radius;
    vision
}

/// The glow settings a vision carries.
fn glow_of(vision: &FilmVision) -> GlowViewInfo {
    GlowViewInfo {
        enable: vision.glow_enable,
        cutoff: vision.glow_bloom_cutoff,
        desaturation: vision.glow_bloom_desaturation,
        intensity: vision.glow_bloom_intensity,
        radius: vision.glow_radius,
    }
}

/// The map's vision with the glow tweak dvars laid over its glow, when they
/// are in use. The tweaks alone make a vision if the map has none.
#[must_use]
pub fn presented_film_vision_with_glow_tweaks(
    view_ready: bool,
    map_vision: Option<FilmVision>,
    use_tweaks: bool,
    tweaks: GlowViewInfo,
) -> Option<FilmVision> {
    if !view_ready {
        return None;
    }
    if !use_tweaks {
        return map_vision;
    }
    let vision = map_vision.unwrap_or_else(plain_vision);
    let glow = lighting_t6::select_glow_view_info(glow_of(&vision), true, tweaks);
    Some(with_glow(vision, glow))
}

/// The vision to draw: the map's, faded to over `slot`'s lerp whenever the
/// map's changes, then the glow tweaks over it. Without a view or a map
/// vision the slot starts over.
#[must_use]
pub fn presented_film_vision_with_lerp(
    view_ready: bool,
    map_vision: Option<FilmVision>,
    now_ms: i32,
    duration_ms: i32,
    slot: &mut FilmVisionView,
    use_tweaks: bool,
    tweaks: GlowViewInfo,
) -> Option<FilmVision> {
    let Some(map) = map_vision.filter(|_| view_ready) else {
        *slot = FilmVisionView::default();
        return None;
    };
    if slot.last_map != Some(map) {
        slot.mode
            .start_lerp_to(now_ms, VisionSetLerpStyle::ToLinear, map, duration_ms);
        slot.last_map = Some(map);
    }
    slot.mode.update(now_ms, lerp_film_vision);
    presented_film_vision_with_glow_tweaks(true, Some(slot.mode.current), use_tweaks, tweaks)
}

/// Which of the scripts' visions the local player sees through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisionKind {
    Missile,
    Thermal,
    Night,
    Naked,
}

impl VisionKind {
    /// A missile camera beats thermal, thermal beats night vision.
    fn pick(in_missile: bool, infrared: bool, night_vision: bool) -> Self {
        if in_missile {
            Self::Missile
        } else if infrared {
            Self::Thermal
        } else if night_vision {
            Self::Night
        } else {
            Self::Naked
        }
    }

    /// The client's own vision of this kind, else the match-wide one.
    fn wanted(self, effects: &ViewEffects, global: &ObjectiveMatch) -> Option<VisionChange> {
        let (own, shared) = match self {
            Self::Missile => (&effects.missile_vision, &global.missile_vision),
            Self::Thermal => (&effects.thermal_vision, &global.thermal_vision),
            Self::Night => (&effects.night_vision, &global.night_vision),
            Self::Naked => (&effects.naked_vision, &global.naked_vision),
        };
        own.clone().or_else(|| shared.clone())
    }
}

type Dvars = [(String, String)];

/// Is the `nightvision` dvar 1? Its latest setting counts, the client's
/// before the server's.
fn night_vision_dvar(client: &Dvars, server: &Dvars) -> bool {
    client
        .iter()
        .rev()
        .chain(server.iter().rev())
        .find(|(name, _)| name.eq_ignore_ascii_case("nightvision"))
        .is_some_and(|(_, value)| value == "1")
}

/// Does anything set a post effect by dvar?
fn sets_post_effect(client: &Dvars, server: &Dvars) -> bool {
    client
        .iter()
        .chain(server)
        .any(|(name, _)| sim::is_postfx_dvar(name))
}

/// The pain vision the client has, else the match's.
fn pain_vision(effects: Option<&ViewEffects>, global: &ObjectiveMatch) -> Option<VisionChange> {
    effects
        .and_then(|effects| effects.pain_vision.clone())
        .or_else(|| global.pain_vision.clone())
}

/// How long to fade for a change: no fade for the first thing applied or
/// for going back to nothing.
fn fade_ms(applied: &Option<Option<VisionChange>>, wanted: &Option<VisionChange>) -> i32 {
    match (applied, wanted) {
        (Some(_), Some(change)) => change.duration_ms,
        _ => 0,
    }
}

/// The share of health left, for a living player.
fn health_fraction(health: i32, max_health: i32, pm_type: i32) -> Option<f32> {
    (health > 0 && max_health > 0 && pm_type < 5).then(|| health as f32 / max_health as f32)
}

/// How strongly the pain overlay shows. It jumps up as health drops under
/// half, then fades out at one unit a second once health recovers. With no
/// pain vision or no living player it is gone.
fn pain_strength_after(strength: f32, health: Option<f32>, seconds: f32) -> f32 {
    let Some(health) = health else {
        return 0.0;
    };
    if health <= PAIN_HEALTH || strength > 0.0 {
        (1.0 - health).clamp(0.0, 1.0).max(strength - seconds)
    } else {
        strength
    }
}

/// The vision with the player's brightness setting added, unless the
/// scripts own the grade.
fn with_brightness(
    vision: Option<FilmVision>,
    ready: bool,
    scripted: bool,
    brightness: f32,
) -> Option<FilmVision> {
    if !ready || scripted || brightness == 0.0 {
        return vision;
    }
    let mut vision = vision.unwrap_or_else(plain_vision);
    vision.enable = true;
    vision.brightness += brightness;
    Some(vision)
}

/// A dvar's value as a finite number.
fn scalar(value: &str) -> Option<f32> {
    value
        .trim()
        .trim_matches('"')
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Three finite numbers.
fn tint(value: &str) -> Option<[f32; 3]> {
    let mut parts = value.trim().trim_matches('"').split_whitespace();
    let mut next = || parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite());
    let tint = [next()?, next()?, next()?];
    parts.next().is_none().then_some(tint)
}

/// The grade the `r_film*` dvars leave behind.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScriptGrade {
    vision: FilmVision,
    grading: [f32; 4],
    blur: f32,
    /// A dvar changed something.
    forced: bool,
}

/// Lay the post-effect dvars, in order, over `vision`. An enable dvar
/// (`r_filmenable`, `r_filmtweakenable`) has the last word on the enable
/// flag, whatever else switched it on.
fn apply_script_dvars<'a>(
    vision: FilmVision,
    blur: f32,
    dvars: impl Iterator<Item = &'a (String, String)>,
) -> ScriptGrade {
    let mut grade = ScriptGrade {
        vision,
        grading: NEUTRAL_GRADING,
        blur,
        forced: false,
    };
    let mut enable = None;
    for (name, value) in dvars {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "r_filmenable" | "r_filmtweakenable"
        ) && let Some(v) = scalar(value)
        {
            enable = Some(v != 0.0);
        }
        let changed = apply_script_film_dvar(
            &mut grade.vision,
            &mut grade.grading,
            &mut grade.blur,
            name,
            value,
        );
        grade.forced |= changed;
    }
    if let Some(enable) = enable {
        grade.vision.enable = enable;
    }
    grade
}

/// Apply one dvar from the scripts: whether it was one we know. Values come
/// from snapshot metadata, never from the console's permission path.
fn apply_script_film_dvar(
    vision: &mut FilmVision,
    grading: &mut [f32; 4],
    blur: &mut f32,
    name: &str,
    value: &str,
) -> bool {
    let name = name.to_ascii_lowercase().replace("r_filmtweak", "r_film");
    if let Some(slot) = match name.as_str() {
        "r_filmlighttint" => Some(&mut vision.light_tint),
        "r_filmmediumtint" => Some(&mut vision.medium_tint),
        "r_filmdarktint" => Some(&mut vision.dark_tint),
        _ => None,
    } {
        let Some(tint) = tint(value) else {
            return false;
        };
        *slot = tint;
        vision.enable = true;
        return true;
    }
    let Some(v) = scalar(value) else {
        return false;
    };
    let mut lit = |apply: &mut dyn FnMut(&mut FilmVision)| {
        vision.enable = true;
        apply(vision);
    };
    match name.as_str() {
        "r_filmenable" => vision.enable = v != 0.0,
        "r_filmusetweaks" | "r_glowusetweaks" => {}
        "r_filmbrightness" | "r_brightness" => lit(&mut |s| s.brightness = v),
        "r_filmcontrast" | "r_contrast" => lit(&mut |s| s.contrast = v),
        "r_filmdesaturation" => lit(&mut |s| s.desaturation = v),
        "r_filmdesaturationdark" => lit(&mut |s| s.desaturation_dark = v),
        "r_filminvert" => lit(&mut |s| s.invert = v != 0.0),
        "r_glow" | "r_glowtweakenable" => vision.glow_enable = v != 0.0,
        "r_glowradius0" | "r_glowtweakradius0" => vision.glow_radius = v,
        "r_glowbloomintensity0" | "r_glowtweakbloomintensity0" => vision.glow_bloom_intensity = v,
        "r_glowbloomcutoff" | "r_glowtweakbloomcutoff" => vision.glow_bloom_cutoff = v,
        "r_glowbloomdesaturation" | "r_glowtweakbloomdesaturation" => {
            vision.glow_bloom_desaturation = v;
        }
        "r_hue" | "r_filmhue" => grading[0] = v.to_radians(),
        "r_gamma" => grading[1] = v.max(0.001),
        "r_exposure" => grading[2] = v,
        "r_saturation" => grading[3] = v,
        "r_blur" => *blur = v.max(0.0),
        _ => return false,
    }
    true
}

/// A preset vision the map has loaded, by the script's name for it.
fn loaded_script_vision(scene: &WorldScene, change: &VisionChange) -> Option<FilmVision> {
    if change.name.is_empty() {
        return None;
    }
    let key = format!("vision/{}.vision", change.name.to_ascii_lowercase());
    match scene.film_visions.get(&key) {
        Some(Ok(preset)) => Some(*preset),
        Some(Err(error)) => {
            diag::warn!(World, "vision {key}: {error:?}");
            None
        }
        None => {
            diag::warn!(World, "vision {key} is not loaded");
            None
        }
    }
}

/// What the scripts last asked for, so a change is faded to only once.
#[derive(Resource, Default)]
struct AppliedVision {
    /// `None` until something was applied; then what was, which may be
    /// "no vision".
    vision: Option<Option<VisionChange>>,
    pain: Option<Option<VisionChange>>,
    pain_slot: FilmVisionView,
    pain_strength: f32,
}

pub fn register(app: &mut App) {
    app.init_resource::<AppliedVision>()
        .init_resource::<FilmVisionView>()
        .add_systems(
            Update,
            update_film_vision_view
                .after(crate::prepare::scene::view_parms::stamp_prepared_scene_view)
                .in_set(net::ClientSet::Present),
        );
}

/// Fade `slot` to `wanted` unless that is what it already fades to.
fn apply_change(
    slot: &mut FilmVisionView,
    applied: &mut Option<Option<VisionChange>>,
    wanted: Option<VisionChange>,
    scene: &WorldScene,
    now_ms: i32,
) {
    if applied.as_ref() == Some(&wanted) {
        return;
    }
    let preset = wanted
        .as_ref()
        .and_then(|change| loaded_script_vision(scene, change));
    slot.select(scene.film_vision, preset, now_ms, fade_ms(applied, &wanted));
    *applied = Some(wanted);
}

#[allow(clippy::too_many_arguments)]
fn update_film_vision_view(
    view: Res<crate::prepare::scene::view_parms::PreparedSceneView>,
    scene: Res<WorldScene>,
    glow: Res<GlowDvars>,
    clock: Res<net::FrameClock>,
    mut film: ResMut<FilmVisionView>,
    presented: Res<net::PresentedSnapshot>,
    local: Res<net::LocalPresentClient>,
    mut applied: ResMut<AppliedVision>,
    settings: Res<frame::GameSettings>,
) {
    let now = clock.time();
    let snapshot = presented.snapshot();
    let meta = snapshot.and_then(|snapshot| snapshot.meta.for_client(local.0));
    let global = snapshot.map(|snapshot| &snapshot.meta.objectives);
    let player = presented.player(local.0);

    if !view.ready {
        applied.vision = None;
    } else {
        let wanted = meta.zip(global).and_then(|(meta, global)| {
            let infrared = player.is_some_and(|ps| {
                ps.other_flags & playerstate_t6::game::other_flags::INFRARED != 0
            });
            let night = player.is_some_and(|ps| {
                ps.weap_flags & playerstate_t6::game::weap_flags::NIGHT_VISION != 0
            }) || night_vision_dvar(&meta.client_dvars, &global.server_info);
            VisionKind::pick(meta.remote_missile.is_some(), infrared, night)
                .wanted(&meta.view_effects, global)
        });
        film.script_forced = wanted.is_some()
            || meta.zip(global).is_some_and(|(meta, global)| {
                sets_post_effect(&meta.client_dvars, &global.server_info)
            });
        apply_change(&mut film, &mut applied.vision, wanted, &scene, now);
    }

    let scripted = film.script_forced;
    let map_vision = film.override_vision.or(scene.film_vision);
    let mut mixed = presented_film_vision_with_lerp(
        view.ready,
        map_vision,
        now,
        0,
        &mut film,
        glow.use_tweaks && !scripted,
        glow.tweak_view_info(),
    );

    if !view.ready {
        *applied = AppliedVision::default();
    } else if let Some(global) = global {
        let applied = &mut *applied;
        let wanted = pain_vision(meta.map(|meta| &meta.view_effects), global);
        apply_change(
            &mut applied.pain_slot,
            &mut applied.pain,
            wanted.clone(),
            &scene,
            now,
        );
        let health = player
            .and_then(|ps| health_fraction(ps.health, ps.max_health, ps.pm_type))
            .filter(|_| wanted.is_some());
        applied.pain_strength =
            pain_strength_after(applied.pain_strength, health, clock.frametime_secs());
        let pain_map = applied.pain_slot.override_vision.or(scene.film_vision);
        let pain = presented_film_vision_with_lerp(
            true,
            pain_map,
            now,
            0,
            &mut applied.pain_slot,
            false,
            glow.tweak_view_info(),
        );
        if let Some(pain) = pain.filter(|_| applied.pain_strength > 0.0) {
            mixed = Some(lerp_film_vision(
                mixed.unwrap_or_else(plain_vision),
                pain,
                applied.pain_strength,
                VisionSetLerpStyle::ToLinear,
            ));
            film.script_forced = true;
        }
    }

    film.current = with_brightness(mixed, view.ready, film.script_forced, settings.brightness);
    film.grading = NEUTRAL_GRADING;
    film.blur = 0.0;
    if !view.ready {
        film.script_forced = false;
        return;
    }
    let (Some(meta), Some(global)) = (meta, global) else {
        return;
    };
    let blur = meta.view_effects.blur.map_or(0.0, |blur| blur.sample(now));
    let grade = apply_script_dvars(
        film.current.unwrap_or_else(plain_vision),
        blur,
        global.server_info.iter().chain(&meta.client_dvars),
    );
    film.script_forced |= grade.forced;
    film.grading = grade.grading;
    film.blur = grade.blur;
    film.current = Some(grade.vision);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dvars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn change(name: &str, duration_ms: i32) -> VisionChange {
        VisionChange {
            name: name.to_owned(),
            duration_ms,
            set_ms: 0,
        }
    }

    fn contrast(contrast: f32) -> FilmVision {
        FilmVision {
            contrast,
            ..FilmVision::default()
        }
    }

    const NO_TWEAKS: GlowViewInfo = GlowViewInfo {
        enable: true,
        cutoff: 1.0,
        desaturation: 2.0,
        intensity: 3.0,
        radius: 4.0,
    };

    #[test]
    fn a_vision_lerps_each_field_by_its_own_type() {
        let from = FilmVision {
            contrast: 1.0,
            light_tint: [0.0; 3],
            enable: false,
            ..FilmVision::default()
        };
        let to = FilmVision {
            contrast: 3.0,
            light_tint: [1.0; 3],
            enable: true,
            ..FilmVision::default()
        };
        let halfway = lerp_film_vision(from, to, 0.5, VisionSetLerpStyle::ToLinear);
        assert_eq!(halfway.contrast, 2.0);
        assert_eq!(halfway.light_tint, [0.5; 3]);
        assert!(halfway.enable, "a flag takes the target");
        let there_and_back = lerp_film_vision(from, to, 0.75, VisionSetLerpStyle::BackforthLinear);
        assert!(!there_and_back.enable, "past half a back-and-forth returns");
    }

    #[test]
    fn the_map_vision_needs_a_view() {
        let vision = Some(contrast(2.0));
        assert_eq!(presented_film_vision(true, vision), vision);
        assert_eq!(presented_film_vision(false, vision), None);
    }

    #[test]
    fn glow_tweaks_replace_only_the_glow() {
        let map = FilmVision {
            contrast: 2.0,
            glow_radius: 9.0,
            ..FilmVision::default()
        };
        assert_eq!(
            presented_film_vision_with_glow_tweaks(true, Some(map), false, NO_TWEAKS),
            Some(map)
        );
        assert_eq!(
            presented_film_vision_with_glow_tweaks(false, Some(map), true, NO_TWEAKS),
            None
        );
        let tweaked =
            presented_film_vision_with_glow_tweaks(true, Some(map), true, NO_TWEAKS).unwrap();
        assert_eq!(tweaked.contrast, 2.0);
        assert_eq!(glow_of(&tweaked), NO_TWEAKS);
    }

    #[test]
    fn tweaks_alone_make_a_vision_when_the_map_has_none() {
        let tweaked = presented_film_vision_with_glow_tweaks(true, None, true, NO_TWEAKS).unwrap();
        assert_eq!(tweaked, with_glow(FilmVision::default(), NO_TWEAKS));
        assert_eq!(
            presented_film_vision_with_glow_tweaks(true, None, false, NO_TWEAKS),
            None
        );
    }

    #[test]
    fn a_new_slot_snaps_to_the_map_vision() {
        let mut slot = FilmVisionView::default();
        let map = contrast(2.0);
        let shown =
            presented_film_vision_with_lerp(true, Some(map), 0, 1000, &mut slot, false, NO_TWEAKS);
        assert_eq!(shown, Some(map));
    }

    #[test]
    fn a_changed_map_vision_fades_over_the_duration() {
        let mut slot = FilmVisionView::default();
        let _ = presented_film_vision_with_lerp(
            true,
            Some(contrast(1.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        let target = Some(contrast(3.0));
        let at = |slot: &mut FilmVisionView, now| {
            presented_film_vision_with_lerp(true, target, now, 1000, slot, false, NO_TWEAKS)
                .unwrap()
                .contrast
        };
        let start = at(&mut slot, 0);
        let middle = at(&mut slot, 500);
        let end = at(&mut slot, 1100);
        assert!(start < middle && middle < end, "{start} {middle} {end}");
        assert_eq!(end, 3.0);
    }

    #[test]
    fn without_a_view_or_a_map_vision_the_slot_starts_over() {
        let mut slot = FilmVisionView::default();
        let _ = presented_film_vision_with_lerp(
            true,
            Some(contrast(2.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        assert_ne!(slot, FilmVisionView::default());
        assert_eq!(
            presented_film_vision_with_lerp(
                false,
                Some(contrast(2.0)),
                0,
                0,
                &mut slot,
                false,
                NO_TWEAKS
            ),
            None
        );
        assert_eq!(slot, FilmVisionView::default());
        let _ = presented_film_vision_with_lerp(
            true,
            Some(contrast(2.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        assert_eq!(
            presented_film_vision_with_lerp(true, None, 0, 0, &mut slot, false, NO_TWEAKS),
            None
        );
        assert_eq!(slot, FilmVisionView::default());
    }

    #[test]
    fn selecting_a_preset_without_a_fade_shows_it_at_once() {
        let mut view = FilmVisionView::default();
        let (map, preset) = (contrast(1.0), contrast(5.0));
        view.select(Some(map), Some(preset), 0, 0);
        assert_eq!(view.mode.current, preset);
        assert_eq!(view.override_vision, Some(preset));
        view.select(Some(map), None, 10, 0);
        assert_eq!(view.mode.current, map);
        assert_eq!(view.override_vision, None);
    }

    #[test]
    fn the_first_selection_starts_from_the_map() {
        let mut view = FilmVisionView::default();
        let (map, preset) = (contrast(1.0), contrast(5.0));
        view.select(Some(map), Some(preset), 0, 1000);
        assert_eq!(view.mode.current, map, "fading, not yet there");
        view.mode.update(2000, lerp_film_vision);
        assert_eq!(view.mode.current, preset);
    }

    #[test]
    fn a_missile_camera_beats_thermal_beats_night_vision() {
        assert_eq!(VisionKind::pick(true, true, true), VisionKind::Missile);
        assert_eq!(VisionKind::pick(false, true, true), VisionKind::Thermal);
        assert_eq!(VisionKind::pick(false, false, true), VisionKind::Night);
        assert_eq!(VisionKind::pick(false, false, false), VisionKind::Naked);
    }

    #[test]
    fn the_clients_own_vision_beats_the_matchs() {
        let mut effects = ViewEffects::default();
        let global = ObjectiveMatch {
            night_vision: Some(change("match_night", 100)),
            ..ObjectiveMatch::default()
        };
        assert_eq!(
            VisionKind::Night.wanted(&effects, &global),
            Some(change("match_night", 100))
        );
        effects.night_vision = Some(change("own_night", 50));
        assert_eq!(
            VisionKind::Night.wanted(&effects, &global),
            Some(change("own_night", 50))
        );
        assert_eq!(VisionKind::Thermal.wanted(&effects, &global), None);
    }

    #[test]
    fn each_kind_reads_its_own_slot() {
        let effects = ViewEffects {
            naked_vision: Some(change("naked", 1)),
            thermal_vision: Some(change("thermal", 2)),
            missile_vision: Some(change("missile", 3)),
            night_vision: Some(change("night", 4)),
            ..ViewEffects::default()
        };
        let global = ObjectiveMatch::default();
        for (kind, name) in [
            (VisionKind::Naked, "naked"),
            (VisionKind::Thermal, "thermal"),
            (VisionKind::Missile, "missile"),
            (VisionKind::Night, "night"),
        ] {
            assert_eq!(kind.wanted(&effects, &global).unwrap().name, name);
        }
    }

    #[test]
    fn the_pain_vision_falls_back_to_the_matchs() {
        let global = ObjectiveMatch {
            pain_vision: Some(change("match_pain", 10)),
            ..ObjectiveMatch::default()
        };
        assert_eq!(pain_vision(None, &global), global.pain_vision);
        let effects = ViewEffects {
            pain_vision: Some(change("own_pain", 20)),
            ..ViewEffects::default()
        };
        assert_eq!(pain_vision(Some(&effects), &global), effects.pain_vision);
        assert_eq!(
            pain_vision(Some(&ViewEffects::default()), &global),
            global.pain_vision
        );
    }

    #[test]
    fn the_latest_nightvision_dvar_wins_and_the_clients_comes_first() {
        let on = dvars(&[("NightVision", "1")]);
        let off = dvars(&[("nightvision", "0")]);
        assert!(night_vision_dvar(&on, &[]));
        assert!(night_vision_dvar(&[], &on));
        assert!(
            !night_vision_dvar(&off, &on),
            "the client's setting is seen first"
        );
        assert!(!night_vision_dvar(
            &dvars(&[("nightvision", "1"), ("nightvision", "0")]),
            &[]
        ));
        assert!(!night_vision_dvar(&[], &[]));
        assert!(!night_vision_dvar(&dvars(&[("nightvision", "yes")]), &[]));
    }

    #[test]
    fn a_post_effect_dvar_from_either_side_takes_the_grade() {
        let film = dvars(&[("r_filmbrightness", "0.1")]);
        let other = dvars(&[("cg_fov", "65")]);
        assert!(sets_post_effect(&film, &[]));
        assert!(sets_post_effect(&other, &film));
        assert!(!sets_post_effect(&other, &[]));
    }

    #[test]
    fn the_fade_is_the_changes_own_only_after_something_was_applied() {
        let wanted = Some(change("night", 750));
        assert_eq!(fade_ms(&None, &wanted), 0);
        assert_eq!(fade_ms(&Some(None), &wanted), 750);
        assert_eq!(fade_ms(&Some(wanted.clone()), &None), 0);
    }

    #[test]
    fn only_a_living_player_has_a_health_fraction() {
        assert_eq!(health_fraction(50, 100, 0), Some(0.5));
        assert_eq!(health_fraction(0, 100, 0), None);
        assert_eq!(health_fraction(50, 0, 0), None);
        assert_eq!(health_fraction(50, 100, 5), None);
    }

    #[test]
    fn pain_grows_below_half_health_and_fades_a_unit_a_second() {
        assert_eq!(pain_strength_after(0.0, Some(0.8), 0.1), 0.0);
        assert_eq!(pain_strength_after(0.0, Some(0.25), 0.1), 0.75);
        assert_eq!(
            pain_strength_after(0.75, Some(0.8), 0.25),
            0.5,
            "recovering fades it"
        );
        let floor = pain_strength_after(0.1, Some(0.8), 5.0);
        assert!(
            (floor - 0.2).abs() < 1e-6,
            "never below the health's own: {floor}"
        );
        assert_eq!(pain_strength_after(0.9, None, 0.1), 0.0);
    }

    #[test]
    fn the_brightness_setting_lights_the_vision_unless_scripts_own_it() {
        let base = Some(FilmVision {
            brightness: 0.25,
            ..FilmVision::default()
        });
        let lit = with_brightness(base, true, false, 0.5).unwrap();
        assert!(lit.enable);
        assert_eq!(lit.brightness, 0.75);
        assert_eq!(with_brightness(base, true, true, 0.5), base);
        assert_eq!(with_brightness(base, false, false, 0.5), base);
        assert_eq!(with_brightness(base, true, false, 0.0), base);
        assert_eq!(with_brightness(None, true, false, 0.0), None);
        assert!(with_brightness(None, true, false, 0.5).unwrap().enable);
    }

    #[test]
    fn scalars_are_trimmed_unquoted_and_finite() {
        assert_eq!(scalar(" \"1.5\" "), Some(1.5));
        assert_eq!(scalar("nan"), None);
        assert_eq!(scalar("inf"), None);
        assert_eq!(scalar("x"), None);
    }

    #[test]
    fn a_tint_is_exactly_three_finite_numbers() {
        assert_eq!(tint("1 0.5 0"), Some([1.0, 0.5, 0.0]));
        assert_eq!(tint("\"1 0.5 0\""), Some([1.0, 0.5, 0.0]));
        assert_eq!(tint("1 0.5"), None);
        assert_eq!(tint("1 0.5 0 0"), None);
        assert_eq!(tint("1 nan 0"), None);
        assert_eq!(tint("a b c"), None);
    }

    fn apply(name: &str, value: &str) -> (FilmVision, [f32; 4], f32, bool) {
        let (mut vision, mut grading, mut blur) = (FilmVision::default(), NEUTRAL_GRADING, 0.0);
        let known = apply_script_film_dvar(&mut vision, &mut grading, &mut blur, name, value);
        (vision, grading, blur, known)
    }

    #[test]
    fn colour_dvars_set_their_field_and_switch_the_grade_on() {
        let (vision, _, _, known) = apply("r_filmContrast", "1.5");
        assert!(known && vision.enable);
        assert_eq!(vision.contrast, 1.5);
        assert_eq!(apply("r_filmtweakbrightness", "0.2").0.brightness, 0.2);
        assert_eq!(apply("r_contrast", "2").0.contrast, 2.0);
        assert_eq!(apply("r_filmdesaturation", "0.3").0.desaturation, 0.3);
        assert_eq!(
            apply("r_filmdesaturationdark", "0.4").0.desaturation_dark,
            0.4
        );
        assert!(apply("r_filminvert", "1").0.invert);
    }

    #[test]
    fn tint_dvars_set_their_tint() {
        let (vision, _, _, known) = apply("r_filmtweaklighttint", "1 0 0");
        assert!(known && vision.enable);
        assert_eq!(vision.light_tint, [1.0, 0.0, 0.0]);
        assert_eq!(
            apply("r_filmmediumtint", "0 1 0").0.medium_tint,
            [0.0, 1.0, 0.0]
        );
        assert_eq!(
            apply("r_filmdarktint", "0 0 1").0.dark_tint,
            [0.0, 0.0, 1.0]
        );
        let (vision, _, _, known) = apply("r_filmdarktint", "0 0");
        assert!(!known && !vision.enable && vision.dark_tint == [1.0; 3]);
    }

    #[test]
    fn glow_dvars_set_the_glow_without_switching_the_grade_on() {
        let (vision, _, _, known) = apply("r_glowtweakradius0", "6");
        assert!(known && !vision.enable);
        assert_eq!(vision.glow_radius, 6.0);
        assert!(apply("r_glow", "1").0.glow_enable);
        assert_eq!(
            apply("r_glowbloomintensity0", "2").0.glow_bloom_intensity,
            2.0
        );
        assert_eq!(apply("r_glowbloomcutoff", "0.9").0.glow_bloom_cutoff, 0.9);
        assert_eq!(
            apply("r_glowtweakbloomdesaturation", "0.1")
                .0
                .glow_bloom_desaturation,
            0.1
        );
    }

    #[test]
    fn grading_dvars_fill_the_grading() {
        let hue = apply("r_hue", "180").1[0];
        assert!((hue - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(
            apply("r_gamma", "0").1[1],
            0.001,
            "gamma never reaches zero"
        );
        assert_eq!(apply("r_gamma", "2.2").1[1], 2.2);
        assert_eq!(apply("r_exposure", "1").1[2], 1.0);
        assert_eq!(apply("r_saturation", "0.5").1[3], 0.5);
        assert_eq!(apply("r_blur", "-3").2, 0.0);
        assert_eq!(apply("r_blur", "3").2, 3.0);
    }

    #[test]
    fn unknown_or_malformed_dvars_change_nothing() {
        for (name, value) in [
            ("cg_fov", "65"),
            ("r_filmcontrast", "high"),
            ("r_filmcontrast", "nan"),
            ("r_filmusetweaks", "1"),
        ] {
            let (vision, grading, blur, known) = apply(name, value);
            assert_eq!(vision, FilmVision::default(), "{name}");
            assert_eq!((grading, blur), (NEUTRAL_GRADING, 0.0));
            assert_eq!(known, name == "r_filmusetweaks", "{name}");
        }
    }

    #[test]
    fn a_dvar_pass_applies_in_order_and_the_enable_dvar_has_the_last_word() {
        let list = dvars(&[
            ("r_filmenable", "0"),
            ("r_filmcontrast", "2"),
            ("r_blur", "4"),
        ]);
        let grade = apply_script_dvars(FilmVision::default(), 1.0, list.iter());
        assert_eq!(grade.vision.contrast, 2.0);
        assert!(
            !grade.vision.enable,
            "contrast switched it on, the enable dvar off"
        );
        assert_eq!(grade.blur, 4.0);
        assert!(grade.forced);
    }

    #[test]
    fn a_dvar_pass_with_nothing_for_it_leaves_the_vision_and_the_blur() {
        let list = dvars(&[("cg_fov", "65")]);
        let start = contrast(2.0);
        let grade = apply_script_dvars(start, 0.5, list.iter());
        assert_eq!(grade.vision, start);
        assert_eq!(
            (grade.grading, grade.blur, grade.forced),
            (NEUTRAL_GRADING, 0.5, false)
        );
    }

    #[test]
    fn an_unparsable_enable_dvar_does_not_override() {
        let list = dvars(&[("r_filmcontrast", "2"), ("r_filmenable", "maybe")]);
        let grade = apply_script_dvars(FilmVision::default(), 0.0, list.iter());
        assert!(grade.vision.enable);
    }

    #[test]
    fn the_default_view_is_neutral() {
        let view = FilmVisionView::default();
        assert_eq!(view.current, None);
        assert!(!view.script_forced);
        assert_eq!(view.grading, NEUTRAL_GRADING);
        assert_eq!(view.blur, 0.0);
    }
}
