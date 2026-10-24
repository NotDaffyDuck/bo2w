use bevy::prelude::*;

use hud_t6::vision_set::{self, VisionSetLerpStyle, VisionSetMode};

use super::dof::GlowDvars;

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct FilmVisionView {
    pub current: Option<asset_world::FilmVision>,
    pub script_forced: bool,
    /// Hue in radians, gamma, exposure in stops, saturation.
    pub grading: [f32; 4],
    pub blur: f32,
    /// BO2 vision-set lerp; FilmVision stays the payload until the BO2 postfx exists.
    mode: VisionSetMode<asset_world::FilmVision>,
    last_map: Option<asset_world::FilmVision>,
    override_vision: Option<asset_world::FilmVision>,
}

impl Default for FilmVisionView {
    fn default() -> Self {
        Self {
            current: None,
            script_forced: false,
            grading: [0.0, 1.0, 0.0, 1.0],
            blur: 0.0,
            mode: VisionSetMode::default(),
            last_map: None,
            override_vision: None,
        }
    }
}

impl FilmVisionView {
    /// Select a preset, or restore the map with None. Call only for a ready view.
    pub fn select(
        &mut self,
        map: Option<asset_world::FilmVision>,
        preset: Option<asset_world::FilmVision>,
        now_ms: i32,
        duration_ms: i32,
    ) {
        if self.last_map.is_none() {
            self.mode.current = map.unwrap_or_default();
            self.mode.lerp.style = VisionSetLerpStyle::None;
        }
        self.mode.update(now_ms, lerp_film_vision);
        let target = preset.or(map).unwrap_or_default();
        self.mode
            .start_lerp_to(now_ms, VisionSetLerpStyle::ToSmooth, target, duration_ms);
        self.last_map = Some(target);
        self.override_vision = preset;
    }
}

/// Every field lerped by its type, as BO2's `UpdateVarsLerp` does.
#[must_use]
pub fn lerp_film_vision(
    from: asset_world::FilmVision,
    to: asset_world::FilmVision,
    fraction: f32,
    style: VisionSetLerpStyle,
) -> asset_world::FilmVision {
    let float = |a, b| vision_set::lerp_float(a, b, fraction, style);
    let boolean = |a, b| vision_set::lerp_bool(a, b, fraction, style);
    let vec3 = |a, b| vision_set::lerp_vec3(a, b, fraction, style);
    asset_world::FilmVision {
        enable: boolean(from.enable, to.enable),
        contrast: float(from.contrast, to.contrast),
        brightness: float(from.brightness, to.brightness),
        desaturation: float(from.desaturation, to.desaturation),
        desaturation_dark: float(from.desaturation_dark, to.desaturation_dark),
        invert: boolean(from.invert, to.invert),
        light_tint: vec3(from.light_tint, to.light_tint),
        medium_tint: vec3(from.medium_tint, to.medium_tint),
        dark_tint: vec3(from.dark_tint, to.dark_tint),
        glow_enable: boolean(from.glow_enable, to.glow_enable),
        glow_radius: float(from.glow_radius, to.glow_radius),
        glow_bloom_cutoff: float(from.glow_bloom_cutoff, to.glow_bloom_cutoff),
        glow_bloom_desaturation: float(from.glow_bloom_desaturation, to.glow_bloom_desaturation),
        glow_bloom_intensity: float(from.glow_bloom_intensity, to.glow_bloom_intensity),
    }
}

#[must_use]
pub fn presented_film_vision(
    view_ready: bool,
    map_vision: Option<asset_world::FilmVision>,
) -> Option<asset_world::FilmVision> {
    if view_ready { map_vision } else { None }
}

#[must_use]
pub fn presented_film_vision_with_glow_tweaks(
    view_ready: bool,
    map_vision: Option<asset_world::FilmVision>,
    use_tweaks: bool,
    tweaks: lighting_t6::GlowViewInfo,
) -> Option<asset_world::FilmVision> {
    if !view_ready {
        return None;
    }
    if !use_tweaks {
        return map_vision;
    }
    let mut vision = map_vision.unwrap_or_default();
    let selected = lighting_t6::select_glow_view_info(
        lighting_t6::GlowViewInfo {
            enable: vision.glow_enable,
            cutoff: vision.glow_bloom_cutoff,
            desaturation: vision.glow_bloom_desaturation,
            intensity: vision.glow_bloom_intensity,
            radius: vision.glow_radius,
        },
        true,
        tweaks,
    );
    vision.glow_enable = selected.enable;
    vision.glow_bloom_cutoff = selected.cutoff;
    vision.glow_bloom_desaturation = selected.desaturation;
    vision.glow_bloom_intensity = selected.intensity;
    vision.glow_radius = selected.radius;
    Some(vision)
}

#[must_use]
pub fn presented_film_vision_with_lerp(
    view_ready: bool,
    map_vision: Option<asset_world::FilmVision>,
    now_ms: i32,
    duration_ms: i32,
    slot: &mut FilmVisionView,
    use_tweaks: bool,
    tweaks: lighting_t6::GlowViewInfo,
) -> Option<asset_world::FilmVision> {
    if !view_ready {
        *slot = FilmVisionView::default();
        return None;
    }
    let Some(map) = map_vision else {
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

#[derive(Resource, Default)]
struct AppliedVision {
    vision: Option<Option<sim::VisionChange>>,
    pain: Option<Option<sim::VisionChange>>,
    pain_slot: FilmVisionView,
    pain_strength: f32,
}

pub fn register(app: &mut App) {
    app.init_resource::<AppliedVision>();
    app.init_resource::<FilmVisionView>().add_systems(
        Update,
        update_film_vision_view
            .after(crate::prepare::scene::view_parms::stamp_prepared_scene_view)
            .in_set(net::ClientSet::Present),
    );
}

#[allow(clippy::too_many_arguments)]
fn update_film_vision_view(
    view: Res<crate::prepare::scene::view_parms::PreparedSceneView>,
    scene: Res<crate::prepare::scene::world::WorldScene>,
    glow: Res<GlowDvars>,
    clock: Res<net::FrameClock>,
    mut film: ResMut<FilmVisionView>,
    presented: Res<net::PresentedSnapshot>,
    local: Res<net::LocalPresentClient>,
    mut applied: ResMut<AppliedVision>,
    settings: Res<frame::GameSettings>,
) {
    if !view.ready {
        applied.vision = None;
    } else {
        let wanted = presented.snapshot().and_then(|snapshot| {
            let meta = snapshot.meta.for_client(local.0)?;
            let effects = &meta.view_effects;
            let global = &snapshot.meta.objectives;
            if meta.remote_missile.is_some() {
                effects
                    .missile_vision
                    .clone()
                    .or_else(|| global.missile_vision.clone())
            } else if presented
                .player(local.0)
                .is_some_and(|ps| ps.other_flags & playerstate_t6::game::other_flags::INFRARED != 0)
            {
                effects
                    .thermal_vision
                    .clone()
                    .or_else(|| global.thermal_vision.clone())
            } else if presented
                .player(local.0)
                .is_some_and(|ps| ps.weap_flags & playerstate_t6::game::weap_flags::NIGHT_VISION != 0)
                || meta
                    .client_dvars
                    .iter()
                    .rev()
                    .chain(global.server_info.iter().rev())
                    .find(|(name, _)| name.eq_ignore_ascii_case("nightvision"))
                    .is_some_and(|(_, value)| value == "1")
            {
                effects
                    .night_vision
                    .clone()
                    .or_else(|| global.night_vision.clone())
            } else {
                effects
                    .naked_vision
                    .clone()
                    .or_else(|| global.naked_vision.clone())
            }
        });
        film.script_forced = wanted.is_some()
            || presented
                .snapshot()
                .and_then(|s| {
                    s.meta.for_client(local.0).map(|meta| {
                        meta.client_dvars
                            .iter()
                            .chain(s.meta.objectives.server_info.iter())
                            .any(|(name, _)| sim::is_postfx_dvar(name))
                    })
                })
                .unwrap_or(false);
        if applied.vision.as_ref() != Some(&wanted) {
            let preset = wanted
                .as_ref()
                .and_then(|vision| loaded_script_vision(&scene, vision));
            let duration_ms = match (&applied.vision, &wanted) {
                (Some(_), Some(vision)) => vision.duration_ms,
                _ => 0,
            };
            film.select(
                scene.film_vision,
                preset,
                clock.time(),
                duration_ms,
            );
            applied.vision = Some(wanted);
        }
    }
    let script_forced = film.script_forced;
    let mixed = presented_film_vision_with_lerp(
        view.ready,
        film.override_vision.or(scene.film_vision),
        clock.time(),
        0,
        &mut film,
        glow.use_tweaks && !script_forced,
        glow.tweak_view_info(),
    );
    let mut mixed = mixed;
    if !view.ready {
        *applied = AppliedVision::default();
    } else if let Some(snapshot) = presented.snapshot() {
        let wanted = snapshot
            .meta
            .for_client(local.0)
            .and_then(|meta| meta.view_effects.pain_vision.clone())
            .or_else(|| snapshot.meta.objectives.pain_vision.clone());
        if applied.pain.as_ref() != Some(&wanted) {
            let preset = wanted
                .as_ref()
                .and_then(|vision| loaded_script_vision(&scene, vision));
            let duration = if applied.pain.is_some() {
                wanted.as_ref().map_or(0, |v| v.duration_ms)
            } else {
                0
            };
            applied.pain_slot.select(
                scene.film_vision,
                preset,
                clock.time(),
                duration,
            );
            applied.pain = Some(wanted.clone());
        }
        let health = presented
            .player(local.0)
            .filter(|ps| ps.health > 0 && ps.max_health > 0 && ps.pm_type < 5)
            .map(|ps| ps.health as f32 / ps.max_health as f32);
        if let Some(health) = health.filter(|_| wanted.is_some()) {
            if health <= 0.5 || applied.pain_strength > 0.0 {
                let target = (1.0 - health).clamp(0.0, 1.0);
                applied.pain_strength = target.max(applied.pain_strength - clock.frametime_secs());
            }
        } else {
            applied.pain_strength = 0.0;
        }
        let pain_map = applied.pain_slot.override_vision.or(scene.film_vision);
        let pain = presented_film_vision_with_lerp(
            true,
            pain_map,
            clock.time(),
            0,
            &mut applied.pain_slot,
            false,
            glow.tweak_view_info(),
        );
        if let Some(pain) = pain.filter(|_| applied.pain_strength > 0.0) {
            mixed = Some(lerp_film_vision(
                mixed.unwrap_or_default(),
                pain,
                applied.pain_strength,
                VisionSetLerpStyle::ToLinear,
            ));
            film.script_forced = true;
        }
    }
    film.current = if view.ready && !film.script_forced && settings.brightness != 0.0 {
        let mut vision = mixed.unwrap_or(asset_world::FilmVision::default());
        vision.enable = true;
        vision.brightness += settings.brightness;
        Some(vision)
    } else {
        mixed
    };
    film.grading = [0.0, 1.0, 0.0, 1.0];
    film.blur = 0.0;
    if !view.ready {
        film.script_forced = false;
        return;
    }
    if let Some(snapshot) = presented.snapshot() {
        if let Some(meta) = snapshot.meta.for_client(local.0) {
            film.blur = meta
                .view_effects
                .blur
                .map_or(0.0, |blur| blur.sample(clock.time()));
            let mut vision = match film.current {
                Some(vision) => vision,
                None => asset_world::FilmVision::default(),
            };
            let mut grading = film.grading;
            let mut blur = film.blur;
            let mut enable_override = None;
            for (name, value) in snapshot
                .meta
                .objectives
                .server_info
                .iter()
                .chain(meta.client_dvars.iter())
            {
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "r_filmenable" | "r_filmtweakenable"
                ) {
                    if let Ok(value) = value.parse::<f32>() {
                        if value.is_finite() {
                            enable_override = Some(value != 0.0);
                        }
                    }
                }
                if apply_script_film_dvar(&mut vision, &mut grading, &mut blur, name, value) {
                    film.script_forced = true;
                }
            }
            if let Some(enable) = enable_override {
                vision.enable = enable;
            }
            film.grading = grading;
            film.blur = blur;
            film.current = Some(vision);
        }
    }
}

fn loaded_script_vision(
    scene: &crate::prepare::scene::world::WorldScene,
    vision: &sim::VisionChange,
) -> Option<asset_world::FilmVision> {
    if vision.name.is_empty() {
        return None;
    }
    let key = format!("vision/{}.vision", vision.name.to_ascii_lowercase());
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

/// Values come from snapshot metadata, never from the console permission path.
fn apply_script_film_dvar(
    vision: &mut asset_world::FilmVision,
    grading: &mut [f32; 4],
    blur: &mut f32,
    name: &str,
    value: &str,
) -> bool {
    let name = name.to_ascii_lowercase().replace("r_filmtweak", "r_film");
    let value = value.trim().trim_matches('"');
    if matches!(
        name.as_str(),
        "r_filmlighttint" | "r_filmmediumtint" | "r_filmdarktint"
    ) {
        let Ok(values) = value
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
        else {
            return false;
        };
        if values.len() != 3 || !values.iter().all(|v| v.is_finite()) {
            return false;
        }
        let tint = [values[0], values[1], values[2]];
        vision.enable = true;
        match name.as_str() {
            "r_filmlighttint" => vision.light_tint = tint,
            "r_filmmediumtint" => vision.medium_tint = tint,
            _ => vision.dark_tint = tint,
        }
        return true;
    }
    let Ok(v) = value.parse::<f32>() else {
        return false;
    };
    if !v.is_finite() {
        return false;
    }
    match name.as_str() {
        "r_filmenable" => vision.enable = v != 0.0,
        "r_filmusetweaks" | "r_glowusetweaks" => {}
        "r_filmbrightness" | "r_brightness" => {
            vision.enable = true;
            vision.brightness = v;
        }
        "r_filmcontrast" | "r_contrast" => {
            vision.enable = true;
            vision.contrast = v;
        }
        "r_filmdesaturation" => {
            vision.enable = true;
            vision.desaturation = v;
        }
        "r_filmdesaturationdark" => {
            vision.enable = true;
            vision.desaturation_dark = v;
        }
        "r_filminvert" => {
            vision.enable = true;
            vision.invert = v != 0.0;
        }
        "r_glow" | "r_glowtweakenable" => vision.glow_enable = v != 0.0,
        "r_glowradius0" | "r_glowtweakradius0" => vision.glow_radius = v,
        "r_glowbloomintensity0" | "r_glowtweakbloomintensity0" => vision.glow_bloom_intensity = v,
        "r_glowbloomcutoff" | "r_glowtweakbloomcutoff" => vision.glow_bloom_cutoff = v,
        "r_glowbloomdesaturation" | "r_glowtweakbloomdesaturation" => {
            vision.glow_bloom_desaturation = v
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
