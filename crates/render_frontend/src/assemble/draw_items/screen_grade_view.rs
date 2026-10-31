//! The colour grade the frame is drawn with: the map's film vision, the
//! vision set the scripts ask for (naked, night, thermal, missile), the pain
//! overlay, the player's brightness setting and any `r_film*` dvars the
//! scripts push, mixed into one `ScreenGradeLook` per frame.

use asset_world::ScreenGradeLook;
use bevy::prelude::*;
use hud_t6::vision_set::{self, VisionSetLerpStyle, VisionSetMode};
use lighting_t6::GlowSettings;
use sim::{MatchBoard, ViewEffects, VisionChange};

use super::dof::BloomTunables;
use crate::prepare::scene::world::LevelStage;

/// A colour grade with no hue shift, unit gamma, no exposure and full colour.
const NEUTRAL_GRADING: [f32; 4] = Vec4::new(0.0, 1.0, 0.0, 1.0).to_array();

/// The vision with nothing switched on, for when there is none.
fn plain_vision() -> ScreenGradeLook {
    ScreenGradeLook::default()
}

/// Past this health fraction the pain overlay starts to show.
const PAIN_HEALTH: f32 = 0.5;

/// The vision of the frame being drawn and what it was mixed from.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct GradeEye {
    pub current: Option<ScreenGradeLook>,
    pub script_forced: bool,
    /// Hue in radians, gamma, exposure in stops, saturation.
    pub grading: [f32; 4],
    pub blur: f32,
    /// The lerp between vision sets; the film vision is its payload.
    mode: VisionSetMode<ScreenGradeLook>,
    prior_map: Option<ScreenGradeLook>,
    override_vision: Option<ScreenGradeLook>,
}

impl Default for GradeEye {
    fn default() -> Self {
        let (mode, current) = (VisionSetMode::default(), None);
        Self { current, script_forced: false, grading: NEUTRAL_GRADING, blur: 0.0, mode, prior_map: None, override_vision: None }
    }
}

impl GradeEye {
    /// Fade to `preset`, or back to `map` when there is none. The first
    /// selection starts from the map.
    pub fn select(
        &mut self,
        map: Option<ScreenGradeLook>,
        preset: Option<ScreenGradeLook>,
        clock_millis: i32,
        span_ms: i32,
    ) {
        if self.prior_map.is_none() {
            self.mode.current = map.unwrap_or_else(plain_vision);
            self.mode.lerp.style = VisionSetLerpStyle::None;
        }
        self.mode.catch_up(clock_millis, lerp_film_vision);
        let target = preset.or(map).unwrap_or_else(plain_vision);
        self.mode
            .start_lerp_to(clock_millis, VisionSetLerpStyle::ToSmooth, target, span_ms);
        self.prior_map = Some(target);
        self.override_vision = preset;
    }
}

/// Every field of a vision lerped by its type.
#[must_use]
pub fn lerp_film_vision(
    from: ScreenGradeLook,
    to: ScreenGradeLook,
    frac: f32,
    style: VisionSetLerpStyle,
) -> ScreenGradeLook {
    let float = |a, b| vision_set::lerp_float(a, b, frac, style);
    let flag = |a, b| vision_set::lerp_bool(a, b, frac, style);
    let tint = |a, b| vision_set::lerp_vec3(a, b, frac, style);
    ScreenGradeLook {
        enable: flag(from.enable, to.enable),
        contrast: float(from.contrast, to.contrast),
        brightness: float(from.brightness, to.brightness),
        color_drain: float(from.color_drain, to.color_drain),
        fade_to_grey_shadow: float(from.fade_to_grey_shadow, to.fade_to_grey_shadow),
        flip_tones: flag(from.flip_tones, to.flip_tones),
        lit_tint: tint(from.lit_tint, to.lit_tint),
        midtone_hue: tint(from.midtone_hue, to.midtone_hue),
        shadow_hue: tint(from.shadow_hue, to.shadow_hue),
        bloom_enable: flag(from.bloom_enable, to.bloom_enable),
        bloom_radius: float(from.bloom_radius, to.bloom_radius),
        bloom_cutoff: float(from.bloom_cutoff, to.bloom_cutoff),
        bloom_desaturation: float(from.bloom_desaturation, to.bloom_desaturation),
        bloom_intensity: float(from.bloom_intensity, to.bloom_intensity),
    }
}

/// The map's vision, once there is a view to draw it in.
#[must_use]
pub fn presented_grade(
    eye_ready: bool,
    map_vision: Option<ScreenGradeLook>,
) -> Option<ScreenGradeLook> {
    map_vision.filter(|_| eye_ready)
}

/// `vision` with its glow settings swapped for the tweak dvars'.
fn with_glow(mut vision: ScreenGradeLook, glow: GlowSettings) -> ScreenGradeLook {
    vision.bloom_enable = glow.on;
    vision.bloom_cutoff = glow.glow_floor;
    vision.bloom_desaturation = glow.color_drain;
    vision.bloom_intensity = glow.strength;
    vision.bloom_radius = glow.radius;
    vision
}

/// The glow settings a vision carries.
fn glow_of(vision: &ScreenGradeLook) -> GlowSettings {
    GlowSettings {
        on: vision.bloom_enable,
        glow_floor: vision.bloom_cutoff,
        color_drain: vision.bloom_desaturation,
        strength: vision.bloom_intensity,
        radius: vision.bloom_radius,
    }
}

/// The map's vision with the glow tweak dvars laid over its glow, when they
/// are in use. The tweaks alone make a vision if the map has none.
#[must_use]
pub fn presented_grade_with_bloom_trims(
    eye_ready: bool,
    authored: Option<ScreenGradeLook>,
    use_trims: bool,
    tweaks: GlowSettings,
) -> Option<ScreenGradeLook> {
    if !eye_ready {
        return None;
    }
    if !use_trims {
        return authored;
    }
    let vision = authored.unwrap_or_else(plain_vision);
    let glow = lighting_t6::select_glow_view_info(glow_of(&vision), true, tweaks);
    Some(with_glow(vision, glow))
}

/// The vision to draw: the map's, faded to over `slot`'s lerp whenever the
/// map's changes, then the glow tweaks over it. Without a view or a map
/// vision the slot starts over.
#[must_use]
pub fn presented_grade_with_lerp(
    eye_ready: bool,
    map_vision: Option<ScreenGradeLook>,
    clock_millis: i32,
    span_ms: i32,
    slot: &mut GradeEye,
    use_trims: bool,
    tweaks: GlowSettings,
) -> Option<ScreenGradeLook> {
    let Some(map) = map_vision.filter(|_| eye_ready) else {
        *slot = GradeEye::default();
        return None;
    };
    if slot.prior_map != Some(map) {
        slot.mode
            .start_lerp_to(clock_millis, VisionSetLerpStyle::ToLinear, map, span_ms);
        slot.prior_map = Some(map);
    }
    slot.mode.catch_up(clock_millis, lerp_film_vision);
    presented_grade_with_bloom_trims(true, Some(slot.mode.current), use_trims, tweaks)
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
    fn wanted(self, effects: &ViewEffects, global: &MatchBoard) -> Option<VisionChange> {
        let (own, shared) = match self {
            Self::Missile => (&effects.missile_vision, &global.missile_vision),
            Self::Thermal => (&effects.thermal_vision, &global.thermal_vision),
            Self::Night => (&effects.night_vision, &global.night_vision),
            Self::Naked => (&effects.naked_vision, &global.naked_vision),
        };
        own.clone().or_else(|| shared.clone())
    }
}


/// Is the `nightvision` dvar 1? Its latest setting counts, the client's
/// before the server's.
fn night_vision_dvar(client: &[(String, String)], server: &[(String, String)]) -> bool {
    client
        .iter()
        .rev()
        .chain(server.iter().rev())
        .find(|(name, _)| name.eq_ignore_ascii_case("nightvision"))
        .is_some_and(|pair| pair.1 == "1")
}

/// Does anything set a post effect by dvar?
fn sets_post_effect(client: &[(String, String)], server: &[(String, String)]) -> bool {
    client
        .iter()
        .chain(server)
        .any(|(name, _)| sim::is_postfx_dvar(name))
}

/// The pain vision the client has, else the match's.
fn pain_vision(effects: Option<&ViewEffects>, global: &MatchBoard) -> Option<VisionChange> {
    effects
        .and_then(|effects| effects.pain_vision.clone())
        .or_else(|| global.pain_vision.clone())
}

/// How long to fade for a change: no fade for the first thing applied or
/// for going back to nothing.
fn fade_ms(applied: &Option<Option<VisionChange>>, wanted: &Option<VisionChange>) -> i32 {
    match (applied, wanted) {
        (Some(_), Some(change)) => change.lasts_ms,
        _ => 0,
    }
}

/// The share of health left, for a living player.
fn health_fraction(health: i32, hit_point_ceiling: i32, movement_mode: i32) -> Option<f32> {
    (health > 0 && hit_point_ceiling > 0 && movement_mode < 5).then(|| health as f32 / hit_point_ceiling as f32)
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
    vision: Option<ScreenGradeLook>,
    prepped: bool,
    scripted: bool,
    brightness: f32,
) -> Option<ScreenGradeLook> {
    if !prepped || scripted || brightness == 0.0 {
        return vision;
    }
    let mut lifted = vision.unwrap_or_else(plain_vision);
    lifted.enable = true;
    lifted.brightness += brightness;
    Some(lifted)
}

/// A dvar's value as a finite number.
fn scalar(value: &str) -> Option<f32> {
    value.trim().trim_matches('"').parse::<f32>().ok().filter(|v| v.is_finite())
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
    vision: ScreenGradeLook,
    grading: [f32; 4],
    blur: f32,
    /// A dvar changed something.
    forced: bool,
}

/// Lay the post-effect dvars, in order, over `vision`. An enable dvar
/// (`r_filmenable`, `r_filmtweakenable`) has the last word on the enable
/// flag, whatever else switched it on.
fn apply_script_dvars<'a>(
    vision: ScreenGradeLook,
    blur: f32,
    dvars: impl Iterator<Item = &'a (String, String)>,
) -> ScriptGrade {
    let mut grade = ScriptGrade {
        vision,
        grading: NEUTRAL_GRADING,
        blur,
        forced: false,
    };
    let mut switched = None;
    for (name, value) in dvars {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "r_filmenable" | "r_filmtweakenable"
        ) && let Some(v) = scalar(value)
        {
            switched = Some(v != 0.0);
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
    if let Some(on) = switched {
        grade.vision.enable = on;
    }
    grade
}

/// Apply one dvar from the scripts: whether it was one we know. Values come
/// from snapshot metadata, never from the console's permission path.
fn apply_script_film_dvar(
    vision: &mut ScreenGradeLook,
    grading: &mut [f32; 4],
    blur: &mut f32,
    name: &str,
    text: &str,
) -> bool {
    let name = name.to_ascii_lowercase().replace("r_filmtweak", "r_film");
    if let Some(slot) = match name.as_str() {
        "r_filmlighttint" => Some(&mut vision.lit_tint),
        "r_filmmediumtint" => Some(&mut vision.midtone_hue),
        "r_filmdarktint" => Some(&mut vision.shadow_hue),
        _ => None,
    } {
        let Some(tint) = tint(text) else {
            return false;
        };
        *slot = tint;
        vision.enable = true;
        return true;
    }
    let Some(v) = scalar(text) else {
        return false;
    };
    let mut lit = |apply: &mut dyn FnMut(&mut ScreenGradeLook)| {
        vision.enable = true;
        apply(vision);
    };
    match name.as_str() {
        "r_filmenable" => vision.enable = v != 0.0,
        "r_filmusetweaks" | "r_glowusetweaks" => {}
        "r_filmbrightness" | "r_brightness" => lit(&mut |s| s.brightness = v),
        "r_filmcontrast" | "r_contrast" => lit(&mut |s| s.contrast = v),
        "r_filmdesaturation" => lit(&mut |s| s.color_drain = v),
        "r_filmdesaturationdark" => lit(&mut |s| s.fade_to_grey_shadow = v),
        "r_filminvert" => lit(&mut |s| s.flip_tones = v != 0.0),
        "r_glow" | "r_glowtweakenable" => vision.bloom_enable = v != 0.0,
        "r_glowradius0" | "r_glowtweakradius0" => vision.bloom_radius = v,
        "r_glowbloomintensity0" | "r_glowtweakbloomintensity0" => vision.bloom_intensity = v,
        "r_glowbloomcutoff" | "r_glowtweakbloomcutoff" => vision.bloom_cutoff = v,
        "r_glowbloomdesaturation" | "r_glowtweakbloomdesaturation" => {
            vision.bloom_desaturation = v;
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
fn loaded_script_vision(scene: &LevelStage, change: &VisionChange) -> Option<ScreenGradeLook> {
    if change.name.is_empty() {
        return None;
    }
    let key = format!("vision/{}.vision", change.name.to_ascii_lowercase());
    match scene.film_visions.get(&key) {
        Some(Ok(preset)) => Some(*preset),
        Some(Err(fault)) => {
            diag::warn!(World, "vision {key}: {fault:?}");
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
    pain_slot: GradeEye,
    pain_strength: f32,
}

pub fn register(client: &mut App) {
    client.init_resource::<AppliedVision>()
        .init_resource::<GradeEye>()
        .add_systems(
            Update,
            update_grade_eye
                .after(crate::prepare::scene::eye_parms::stamp_staged_stage_eye)
                .in_set(net::ClientStep::Present),
        );
}

/// Fade `slot` to `wanted` unless that is what it already fades to.
fn apply_change(
    slot: &mut GradeEye,
    applied: &mut Option<Option<VisionChange>>,
    wanted: Option<VisionChange>,
    scene: &LevelStage,
    clock_millis: i32,
) {
    if applied.as_ref() == Some(&wanted) {
        return;
    }
    let preset = wanted
        .as_ref()
        .and_then(|change| loaded_script_vision(scene, change));
    slot.select(scene.colour_grade, preset, clock_millis, fade_ms(applied, &wanted));
    *applied = Some(wanted);
}

#[allow(clippy::too_many_arguments)]
fn update_grade_eye(
    view_pos: Res<crate::prepare::scene::eye_parms::StagedStageEye>,
    scene: Res<LevelStage>,
    glow: Res<BloomTunables>,
    clock: Res<net::FrameClock>,
    mut film: ResMut<GradeEye>,
    presented: Res<net::ShownWorldFrame>,
    local: Res<net::HostViewClient>,
    mut applied: ResMut<AppliedVision>,
    settings: Res<frame::PlayerPrefsBundle>,
) {
    let now = clock.time();
    let shown = presented.snapshot();
    let meta = shown.and_then(|frame| frame.meta.for_client(local.0));
    let global = shown.map(|frame| &frame.meta.objectives);
    let player = presented.player(local.0);

    if !view_pos.primed {
        applied.vision = None;
    } else {
        let wanted = meta.zip(global).and_then(|(meta, global)| {
            let infrared = player.is_some_and(|ps| {
                ps.secondary_state_bits & playerstate_t6::game::secondary_state_bits::INFRARED != 0
            });
            let night = player.is_some_and(|ps| {
                ps.weapon_state_bits & playerstate_t6::game::weapon_state_bits::NIGHT_VISION != 0
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
    let map_vision = film.override_vision.or(scene.colour_grade);
    let mut mixed = presented_grade_with_lerp(
        view_pos.primed,
        map_vision,
        now,
        0,
        &mut film,
        glow.use_trims && !scripted,
        glow.trim_eye_info(),
    );

    if !view_pos.primed {
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
            .and_then(|ps| health_fraction(ps.health, ps.hit_point_ceiling, ps.movement_mode))
            .filter(|_| wanted.is_some());
        applied.pain_strength =
            pain_strength_after(applied.pain_strength, health, clock.step_duration_seconds());
        let pain_map = applied.pain_slot.override_vision.or(scene.colour_grade);
        let pain = presented_grade_with_lerp(
            true,
            pain_map,
            now,
            0,
            &mut applied.pain_slot,
            false,
            glow.trim_eye_info(),
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

    film.current = with_brightness(mixed, view_pos.primed, film.script_forced, settings.brightness);
    film.grading = NEUTRAL_GRADING;
    film.blur = 0.0;
    if !view_pos.primed {
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
            .map(|&(key, text)| (key.to_owned(), text.to_owned()))
            .collect()
    }

    fn change(vision: &str, span_ms: i32) -> VisionChange {
        VisionChange { name: vision.to_owned(), lasts_ms: span_ms, set_ms: 0 }
    }

    fn contrast(contrast: f32) -> ScreenGradeLook {
        ScreenGradeLook {
            contrast,
            ..ScreenGradeLook::default()
        }
    }

    const BLOOM_ON: bool = true;
    const NO_TWEAKS: GlowSettings =
        GlowSettings { on: BLOOM_ON, glow_floor: 1.0, color_drain: 2.0, strength: 3.0, radius: 4.0 };

    #[test]
    fn a_vision_lerps_each_field_by_its_own_type() {
        let mut from = ScreenGradeLook::default();
        (from.contrast, from.lit_tint, from.enable) = (1.0, [0.0; 3], false);
        let mut to = ScreenGradeLook::default();
        (to.contrast, to.lit_tint, to.enable) = (3.0, [1.0; 3], true);
        let halfway = lerp_film_vision(from, to, 0.5, VisionSetLerpStyle::ToLinear);
        assert_eq!(halfway.contrast, 2.0);
        assert_eq!(halfway.lit_tint, [0.5; 3]);
        assert!(halfway.enable, "a flag takes the target");
        let there_and_back = lerp_film_vision(from, to, 0.75, VisionSetLerpStyle::BackforthLinear);
        assert!(!there_and_back.enable, "past half a back-and-forth returns");
    }

    #[test]
    fn the_map_vision_needs_a_view() {
        let vision = Some(contrast(2.0));
        assert_eq!(presented_grade(true, vision), vision);
        assert_eq!(presented_grade(false, vision), None);
    }

    #[test]
    fn glow_tweaks_replace_only_the_glow() {
        let map = ScreenGradeLook {
            contrast: 2.0,
            bloom_radius: 9.0,
            ..ScreenGradeLook::default()
        };
        assert_eq!(
            presented_grade_with_bloom_trims(true, Some(map), false, NO_TWEAKS),
            Some(map)
        );
        assert_eq!(
            presented_grade_with_bloom_trims(false, Some(map), true, NO_TWEAKS),
            None
        );
        let tweaked =
            presented_grade_with_bloom_trims(true, Some(map), true, NO_TWEAKS).unwrap();
        assert_eq!(tweaked.contrast, 2.0);
        assert_eq!(glow_of(&tweaked), NO_TWEAKS);
    }

    #[test]
    fn tweaks_alone_make_a_vision_when_the_map_has_none() {
        let tweaked = presented_grade_with_bloom_trims(true, None, true, NO_TWEAKS).unwrap();
        assert_eq!(tweaked, with_glow(ScreenGradeLook::default(), NO_TWEAKS));
        assert_eq!(
            presented_grade_with_bloom_trims(true, None, false, NO_TWEAKS),
            None
        );
    }

    #[test]
    fn a_new_slot_snaps_to_the_map_vision() {
        let mut slot = GradeEye::default();
        let map = contrast(2.0);
        let shown =
            presented_grade_with_lerp(true, Some(map), 0, 1000, &mut slot, false, NO_TWEAKS);
        assert_eq!(shown, Some(map));
    }

    #[test]
    fn a_changed_map_vision_fades_over_the_duration() {
        let mut slot = GradeEye::default();
        let _ = presented_grade_with_lerp(
            true,
            Some(contrast(1.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        let target = Some(contrast(3.0));
        let at = |slot: &mut GradeEye, now| {
            presented_grade_with_lerp(true, target, now, 1000, slot, false, NO_TWEAKS)
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
        let mut slot = GradeEye::default();
        let _ = presented_grade_with_lerp(
            true,
            Some(contrast(2.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        assert_ne!(slot, GradeEye::default());
        assert_eq!(
            presented_grade_with_lerp(
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
        assert_eq!(slot, GradeEye::default());
        let _ = presented_grade_with_lerp(
            true,
            Some(contrast(2.0)),
            0,
            0,
            &mut slot,
            false,
            NO_TWEAKS,
        );
        assert_eq!(
            presented_grade_with_lerp(true, None, 0, 0, &mut slot, false, NO_TWEAKS),
            None
        );
        assert_eq!(slot, GradeEye::default());
    }

    #[test]
    fn selecting_a_preset_without_a_fade_shows_it_at_once() {
        let mut view = GradeEye::default();
        let (map, wake_preset) = (contrast(1.0), contrast(5.0));
        view.select(Some(map), Some(wake_preset), 0, 0);
        assert_eq!(view.mode.current, wake_preset);
        assert_eq!(view.override_vision, Some(wake_preset));
        view.select(Some(map), None, 10, 0);
        assert_eq!(view.mode.current, map);
        assert_eq!(view.override_vision, None);
    }

    #[test]
    fn the_first_selection_starts_from_the_map() {
        let mut view = GradeEye::default();
        let (map, wake_preset) = (contrast(1.0), contrast(5.0));
        view.select(Some(map), Some(wake_preset), 0, 1000);
        assert_eq!(view.mode.current, map, "fading, not yet there");
        view.mode.catch_up(2000, lerp_film_vision);
        assert_eq!(view.mode.current, wake_preset);
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
        let global = MatchBoard {
            night_vision: Some(change("match_night", 100)),
            ..MatchBoard::default()
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
        let global = MatchBoard::default();
        for (name, kind) in [
            ("naked", VisionKind::Naked),
            ("thermal", VisionKind::Thermal),
            ("missile", VisionKind::Missile),
            ("night", VisionKind::Night),
        ] {
            assert_eq!(kind.wanted(&effects, &global).unwrap().name, name);
        }
    }

    #[test]
    fn the_pain_vision_falls_back_to_the_matchs() {
        let global = MatchBoard {
            pain_vision: Some(change("match_pain", 10)),
            ..MatchBoard::default()
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
        let base = Some(ScreenGradeLook {
            brightness: 0.25,
            ..ScreenGradeLook::default()
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

    fn apply(name: &str, value: &str) -> (ScreenGradeLook, [f32; 4], f32, bool) {
        let (mut vision, mut grading, mut blur) = (ScreenGradeLook::default(), NEUTRAL_GRADING, 0.0);
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
        assert_eq!(apply("r_filmdesaturation", "0.3").0.color_drain, 0.3);
        assert_eq!(
            apply("r_filmdesaturationdark", "0.4").0.fade_to_grey_shadow,
            0.4
        );
        assert!(apply("r_filminvert", "1").0.flip_tones);
    }

    #[test]
    fn tint_dvars_set_their_tint() {
        let (vision, _, _, known) = apply("r_filmtweaklighttint", "1 0 0");
        assert!(known && vision.enable);
        assert_eq!(vision.lit_tint, [1.0, 0.0, 0.0]);
        assert_eq!(apply("r_filmmediumtint", "0 1 0").0.midtone_hue, [0.0, 1.0, 0.0]);
        assert_eq!(apply("r_filmdarktint", "0 0 1").0.shadow_hue, [0.0, 0.0, 1.0]);
        let (vision, _, _, known) = apply("r_filmdarktint", "0 0");
        assert!(!known && !vision.enable && vision.shadow_hue == [1.0; 3]);
    }

    #[test]
    fn glow_dvars_set_the_glow_without_switching_the_grade_on() {
        let (vision, _, _, known) = apply("r_glowtweakradius0", "6");
        assert!(known && !vision.enable);
        assert_eq!(vision.bloom_radius, 6.0);
        assert!(apply("r_glow", "1").0.bloom_enable);
        assert_eq!(
            apply("r_glowbloomintensity0", "2").0.bloom_intensity,
            2.0
        );
        assert_eq!(apply("r_glowbloomcutoff", "0.9").0.bloom_cutoff, 0.9);
        assert_eq!(
            apply("r_glowtweakbloomdesaturation", "0.1")
                .0
                .bloom_desaturation,
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
            assert_eq!(vision, ScreenGradeLook::default(), "{name}");
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
        let grade = apply_script_dvars(ScreenGradeLook::default(), 1.0, list.iter());
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
        let grade = apply_script_dvars(ScreenGradeLook::default(), 0.0, list.iter());
        assert!(grade.vision.enable);
    }

    #[test]
    fn the_default_view_is_neutral() {
        let view = GradeEye::default();
        assert_eq!(view.current, None);
        assert!(!view.script_forced);
        assert_eq!(view.grading, NEUTRAL_GRADING);
        assert_eq!(view.blur, 0.0);
    }
}
