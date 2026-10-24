//! Depth of field and glow settings, and the per-frame blur ranges worked out
//! from the player's weapon, what they are looking at, and anything the
//! server or script has forced.

use bevy::prelude::*;

/// What the ADS autofocus sweep is allowed to focus on: the same solid-world
/// contents the sight trace uses.
const AUTOFOCUS_CLIPMASK: u32 = 0x0080_6c31;

/// How far the autofocus sweep reaches.
const AUTOFOCUS_REACH: f32 = 8192.0;

/// Player-move types from this value up are the dead/spectating ones.
const PM_TYPE_FIRST_NON_PLAYING: i32 = 10;

/// Blur ranges, as distances from the camera. Everything between a start and
/// its end blends from sharp to fully blurred.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DepthOfField {
    pub view_model_start: f32,
    pub view_model_end: f32,
    pub near_start: f32,
    pub near_end: f32,
    pub far_start: f32,
    pub far_end: f32,
    pub near_blur: f32,
    pub far_blur: f32,
}

impl DepthOfField {
    /// Does any of the ranges blur anything? Ranges narrower than a unit do
    /// not count, and the far one also needs a blur strength.
    pub fn active(self) -> bool {
        let wide = |start: f32, end: f32| end > start + 1.0;
        wide(self.view_model_start, self.view_model_end)
            || wide(self.near_start, self.near_end)
            || (wide(self.far_start, self.far_end) && self.far_blur > 0.0)
    }
}

/// Console variables for depth of field.
#[derive(Resource, Clone, Copy, Debug)]
pub struct DofDvars {
    pub enable: bool,
    /// Use `values` instead of the computed ranges.
    pub tweak: bool,
    pub values: DepthOfField,
    pub bias: f32,
}

impl Default for DofDvars {
    fn default() -> Self {
        Self {
            enable: true,
            tweak: false,
            bias: 0.5,
            values: DepthOfField {
                view_model_start: 2.0,
                view_model_end: 8.0,
                near_start: 10.0,
                near_end: 60.0,
                far_start: 1000.0,
                far_end: 7000.0,
                near_blur: 6.0,
                far_blur: 1.8,
            },
        }
    }
}

/// Console variables for the glow post effect.
#[derive(Resource, Clone, Copy, Debug)]
pub struct GlowDvars {
    pub enable: bool,
    pub use_tweaks: bool,
    pub tweak_enable: bool,
    pub tweak_radius: f32,
    pub tweak_intensity: f32,
    pub tweak_cutoff: f32,
    pub tweak_desaturation: f32,
}

impl Default for GlowDvars {
    fn default() -> Self {
        let defaults = lighting_t6::GlowViewInfo::tweak_register_defaults();
        Self {
            enable: lighting_t6::R_GLOW_DEFAULT,
            use_tweaks: lighting_t6::R_GLOW_USE_TWEAKS_DEFAULT,
            tweak_enable: defaults.enable,
            tweak_radius: defaults.radius,
            tweak_intensity: defaults.intensity,
            tweak_cutoff: defaults.cutoff,
            tweak_desaturation: defaults.desaturation,
        }
    }
}

impl GlowDvars {
    /// The tweak values in the form the glow pass takes.
    #[must_use]
    pub fn tweak_view_info(self) -> lighting_t6::GlowViewInfo {
        lighting_t6::GlowViewInfo {
            enable: self.tweak_enable,
            cutoff: self.tweak_cutoff,
            desaturation: self.tweak_desaturation,
            intensity: self.tweak_intensity,
            radius: self.tweak_radius,
        }
    }
}

/// The depth of field to draw this frame.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct DofFrame {
    pub dof: DepthOfField,
    pub bias: f32,
    pub scene_near: f32,
    pub view_model_near: f32,
}

pub fn register(app: &mut App) {
    app.init_resource::<DofDvars>()
        .init_resource::<GlowDvars>()
        .init_resource::<DofFrame>()
        .add_systems(
            Update,
            update_dof
                .after(crate::prepare::scene::view_parms::stamp_prepared_scene_view)
                .in_set(net::ClientSet::Present),
        );
}

/// Move `current` toward `target`: at least one unit per call, half the
/// remaining gap if that is more, but never faster than `rate` per
/// twentieth of a second, and never past the target.
fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let ceiling = rate / 0.05 * dt;
    let half_gap = (current - target).abs() * 0.5;
    let step = if half_gap > ceiling {
        ceiling
    } else {
        half_gap.max(1.0)
    };
    if current > target {
        (current - step).max(target)
    } else {
        (current + step).min(target)
    }
}

/// Where the ranges settle when the player is aiming: a close near range and
/// a far range that starts behind whatever the sight is on. `focus` is how
/// far away that is, if it was measured.
fn aim_target(focus: Option<f32>) -> DepthOfField {
    let mut target = DepthOfField {
        near_start: 1.0,
        near_end: 256.0,
        far_start: 2500.0,
        far_end: 10000.0,
        near_blur: 6.0,
        ..Default::default()
    };
    if let Some(hit) = focus {
        if hit < target.near_end {
            target.near_end = (hit - 30.0).max(1.0);
        }
        target.far_start = target.far_start.max(hit);
        target.far_end = target.far_start * 4.0;
    }
    target
}

/// The ranges while moving between hip and aim (`aim` is the aim fraction,
/// below 1): they follow the fraction directly instead of easing.
fn follow_aim_fraction(scene: &mut DepthOfField, aim: f32, target: &DepthOfField) {
    let hip_far = (1.0 - aim) * 5000.0;
    scene.near_start = aim;
    scene.near_end = aim * target.near_end;
    scene.far_start = hip_far + aim * target.far_start;
    scene.far_end = hip_far + aim * target.far_end;
    scene.near_blur = 6.0;
    scene.far_blur = 0.0;
}

/// The ranges once fully aimed (or not playing): ease toward the target.
fn ease_to_target(scene: &mut DepthOfField, target: &DepthOfField, dt: f32) {
    scene.near_start = approach(scene.near_start, target.near_start, 50.0, dt);
    scene.near_end = approach(scene.near_end, target.near_end, 50.0, dt);
    scene.far_start = approach(scene.far_start, target.far_start, 400.0, dt);
    scene.far_end = approach(scene.far_end, target.far_end, 400.0, dt);
    scene.near_blur = approach(scene.near_blur, 6.0, 0.1, dt);
    scene.far_blur = approach(scene.far_blur, 0.0, 0.1, dt);
}

/// Ranges used while watching a killcam, focused on the killer.
fn killcam_ranges(focus_distance: f32) -> DepthOfField {
    DepthOfField {
        near_start: 0.0,
        near_end: 100.0,
        far_start: focus_distance + 100.0,
        far_end: focus_distance + 400.0,
        near_blur: 4.0,
        far_blur: 2.0,
        ..Default::default()
    }
}

/// Is this a `r_dof_*` setting?
fn is_dof_setting(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("r_dof")
}

/// Apply `r_dof_*` settings sent by the server or set for this client. Any
/// such setting at all first resets the ranges to what the scene computed;
/// then each valid number overrides its field, and `r_dof_enable 0` turns
/// the effect off whatever else was set.
fn apply_forced_settings<'a>(
    dof: &mut DepthOfField,
    bias: &mut f32,
    scene: DepthOfField,
    settings: impl Iterator<Item = (&'a str, &'a str)> + Clone,
) {
    if settings.clone().any(|(name, _)| is_dof_setting(name)) {
        *dof = scene;
    }
    let mut enabled = true;
    for (name, text) in settings {
        let Ok(value) = text.parse::<f32>() else {
            continue;
        };
        if !value.is_finite() {
            continue;
        }
        match name.to_ascii_lowercase().as_str() {
            "r_dof_enable" => enabled = value != 0.0,
            "r_dof_viewmodelstart" => dof.view_model_start = value,
            "r_dof_viewmodelend" => dof.view_model_end = value,
            "r_dof_nearstart" => dof.near_start = value,
            "r_dof_nearend" => dof.near_end = value,
            "r_dof_farstart" => dof.far_start = value,
            "r_dof_farend" => dof.far_end = value,
            "r_dof_nearblur" => dof.near_blur = value,
            "r_dof_farblur" => dof.far_blur = value,
            "r_dof_bias" => *bias = value,
            _ => {}
        }
    }
    if !enabled {
        *dof = DepthOfField::default();
    }
}

/// Which ranges the console variables ask for when no script has forced any.
fn console_choice(dvars: &DofDvars, scene: DepthOfField) -> DepthOfField {
    if dvars.tweak {
        dvars.values
    } else if dvars.enable {
        scene
    } else {
        DepthOfField::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn update_dof(
    view: Res<crate::prepare::scene::view_parms::PreparedSceneView>,
    znear: Res<crate::prepare::scene::view_parms::RZnearDvar>,
    dvars: Res<DofDvars>,
    presented: Res<net::PresentedSnapshot>,
    local: Res<net::LocalPresentClient>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    clock: Res<net::FrameClock>,
    mut frame: ResMut<DofFrame>,
    mut scene: Local<DepthOfField>,
    clip: Res<crate::adapters::anim::dyn_ent::DynEntPhysClip>,
    world: Res<frame::WorldGeneration>,
    subject: Res<frame::ViewSubject>,
    camera: Option<Res<render_anim::occupancy::view_kick::SessionViewKick>>,
    mut generation: Local<Option<u64>>,
) {
    *frame = DofFrame::default();
    if *generation != world.0 {
        *generation = world.0;
        *scene = DepthOfField::default();
    }
    let Some(player) = presented.player(local.0).filter(|_| view.ready) else {
        *scene = DepthOfField::default();
        return;
    };
    let aim = player.f_weapon_pos_frac;
    let playing = player.pm_type < PM_TYPE_FIRST_NON_PLAYING;

    let focus = if aim != 0.0 || !playing {
        clip.0.as_ref().map(|clip| {
            let trace = clip.sweep_box(
                view.eye.to_array(),
                (view.eye + view.forward * AUTOFOCUS_REACH).to_array(),
                [0.0; 3],
                [0.0; 3],
                AUTOFOCUS_CLIPMASK,
            );
            trace.fraction * AUTOFOCUS_REACH
        })
    } else {
        None
    };
    let target = aim_target(focus);
    if aim < 1.0 && playing {
        follow_aim_fraction(&mut scene, aim, &target);
    } else {
        ease_to_target(&mut scene, &target, clock.frametime_secs());
    }

    scene.view_model_start = 0.0;
    scene.view_model_end = 0.0;
    let third_person = crate::adapters::anim::third_person::presented_is_third_person(
        &presented,
        local.0,
        subject.in_killcam(),
    );
    if !third_person {
        let weapon = playerstate_t6::game::get_viewmodel_weapon_index(player);
        let range = weapons
            .as_ref()
            .and_then(|weapons| weapons.0.facts_of(weapon))
            .and_then(|facts| facts.ads_dof);
        if let Some([start, end]) = range {
            scene.view_model_start = start * aim;
            scene.view_model_end = end * aim;
        }
    }
    if let Some(distance) = camera.as_ref().and_then(|c| c.killcam_focus_distance) {
        *scene = killcam_ranges(distance);
    }

    let script = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .map(|meta| meta.view_effects.depth_of_field)
        .filter(sim::ScriptDepthOfField::overrides_scene);
    if let Some(script) = script {
        scene.near_start = script.near_start;
        scene.near_end = script.near_end;
        scene.far_start = script.far_start;
        scene.far_end = script.far_end;
        scene.near_blur = script.near_blur;
        scene.far_blur = script.far_blur;
    }

    let mut dof = if script.is_some() {
        *scene
    } else {
        console_choice(&dvars, *scene)
    };
    let mut bias = dvars.bias;
    if let Some(snapshot) = presented.snapshot()
        && let Some(meta) = snapshot.meta.for_client(local.0)
    {
        let settings = snapshot
            .meta
            .objectives
            .server_info
            .iter()
            .chain(meta.client_dvars.iter())
            .map(|(name, value)| (name.as_str(), value.as_str()));
        apply_forced_settings(&mut dof, &mut bias, *scene, settings);
    }
    *frame = DofFrame {
        dof,
        bias,
        scene_near: znear.value,
        view_model_near: view.depth_hack_near,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(near: (f32, f32), far: (f32, f32), far_blur: f32) -> DepthOfField {
        DepthOfField {
            near_start: near.0,
            near_end: near.1,
            far_start: far.0,
            far_end: far.1,
            near_blur: 6.0,
            far_blur,
            ..Default::default()
        }
    }

    fn forced(settings: &[(&str, &str)], scene: DepthOfField) -> (DepthOfField, f32) {
        let mut dof = scene;
        let mut bias = 0.5;
        apply_forced_settings(&mut dof, &mut bias, scene, settings.iter().copied());
        (dof, bias)
    }

    #[test]
    fn no_ranges_means_inactive() {
        assert!(!DepthOfField::default().active());
    }

    #[test]
    fn a_range_must_be_wider_than_a_unit() {
        assert!(!ranges((10.0, 11.0), (0.0, 0.0), 0.0).active());
        assert!(ranges((10.0, 11.5), (0.0, 0.0), 0.0).active());
    }

    #[test]
    fn the_far_range_needs_blur_strength() {
        assert!(!ranges((0.0, 0.0), (100.0, 900.0), 0.0).active());
        assert!(ranges((0.0, 0.0), (100.0, 900.0), 1.0).active());
    }

    #[test]
    fn the_viewmodel_range_alone_can_make_it_active() {
        let dof = DepthOfField {
            view_model_start: 1.0,
            view_model_end: 9.0,
            ..Default::default()
        };
        assert!(dof.active());
    }

    #[test]
    fn dvar_defaults_are_enabled_and_untweaked() {
        let dvars = DofDvars::default();
        assert!(dvars.enable && !dvars.tweak);
        assert_eq!(dvars.bias, 0.5);
        assert!(dvars.values.active());
    }

    #[test]
    fn glow_tweaks_round_trip_into_view_info() {
        let glow = GlowDvars {
            tweak_enable: true,
            tweak_radius: 1.0,
            tweak_intensity: 2.0,
            tweak_cutoff: 3.0,
            tweak_desaturation: 4.0,
            ..GlowDvars::default()
        };
        let info = glow.tweak_view_info();
        assert!(info.enable);
        assert_eq!(
            (info.radius, info.intensity, info.cutoff, info.desaturation),
            (1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn approach_never_overshoots() {
        assert_eq!(approach(10.0, 10.5, 50.0, 1.0), 10.5);
        assert_eq!(approach(10.0, 9.5, 50.0, 1.0), 9.5);
        assert_eq!(approach(5.0, 5.0, 50.0, 1.0), 5.0);
    }

    #[test]
    fn approach_closes_half_the_gap_when_slow_enough() {
        assert_eq!(approach(0.0, 100.0, 1000.0, 1.0), 50.0);
        assert_eq!(approach(100.0, 0.0, 1000.0, 1.0), 50.0);
    }

    #[test]
    fn approach_is_rate_limited_for_big_gaps() {
        // 50 per 0.05 s at dt = 0.05 allows 50 per call.
        assert_eq!(approach(0.0, 10_000.0, 50.0, 0.05), 50.0);
    }

    #[test]
    fn approach_always_moves_at_least_a_unit() {
        assert_eq!(approach(0.0, 1.5, 50.0, 1.0), 1.0);
    }

    #[test]
    fn an_unmeasured_aim_target_is_the_open_field_default() {
        let target = aim_target(None);
        assert_eq!(target.near_end, 256.0);
        assert_eq!(target.far_start, 2500.0);
        assert_eq!(target.far_end, 10_000.0);
    }

    #[test]
    fn a_near_focus_pulls_the_near_range_in() {
        let target = aim_target(Some(100.0));
        assert_eq!(target.near_end, 70.0);
        assert_eq!(target.far_start, 2500.0);
        assert_eq!(target.far_end, 10_000.0);
        assert_eq!(aim_target(Some(10.0)).near_end, 1.0);
    }

    #[test]
    fn a_distant_focus_pushes_the_far_range_out() {
        let target = aim_target(Some(4000.0));
        assert_eq!(target.near_end, 256.0);
        assert_eq!(target.far_start, 4000.0);
        assert_eq!(target.far_end, 16_000.0);
    }

    #[test]
    fn following_the_aim_fraction_runs_between_hip_and_target() {
        let target = aim_target(None);
        let mut scene = DepthOfField::default();
        follow_aim_fraction(&mut scene, 0.0, &target);
        assert_eq!((scene.near_start, scene.near_end), (0.0, 0.0));
        assert_eq!((scene.far_start, scene.far_end), (5000.0, 5000.0));
        follow_aim_fraction(&mut scene, 0.5, &target);
        assert_eq!(scene.near_end, 128.0);
        assert_eq!(scene.far_start, 2500.0 + 1250.0);
        assert_eq!(scene.far_blur, 0.0);
    }

    #[test]
    fn easing_reaches_the_target_given_time() {
        let target = aim_target(None);
        let mut scene = DepthOfField::default();
        for _ in 0..200 {
            ease_to_target(&mut scene, &target, 0.05);
        }
        assert_eq!(scene.near_end, target.near_end);
        assert_eq!(scene.far_end, target.far_end);
        assert_eq!(scene.far_blur, 0.0);
    }

    #[test]
    fn a_killcam_focuses_just_behind_the_killer() {
        let dof = killcam_ranges(500.0);
        assert_eq!((dof.far_start, dof.far_end), (600.0, 900.0));
        assert_eq!(dof.near_end, 100.0);
    }

    #[test]
    fn the_console_choice_follows_tweak_then_enable() {
        let scene = ranges((1.0, 50.0), (900.0, 5000.0), 1.0);
        let mut dvars = DofDvars::default();
        assert_eq!(console_choice(&dvars, scene), scene);
        dvars.enable = false;
        assert_eq!(console_choice(&dvars, scene), DepthOfField::default());
        dvars.tweak = true;
        assert_eq!(console_choice(&dvars, scene), dvars.values);
    }

    #[test]
    fn forced_values_override_fields_and_bias() {
        let scene = ranges((1.0, 50.0), (900.0, 5000.0), 1.0);
        let (dof, bias) = forced(
            &[
                ("r_dof_nearend", "99"),
                ("R_DOF_Bias", "0.25"),
                ("sv_x", "1"),
            ],
            scene,
        );
        assert_eq!(dof.near_end, 99.0);
        assert_eq!(dof.far_start, 900.0);
        assert_eq!(bias, 0.25);
    }

    #[test]
    fn forced_values_that_are_not_finite_numbers_are_ignored() {
        let scene = ranges((1.0, 50.0), (900.0, 5000.0), 1.0);
        let (dof, _) = forced(
            &[
                ("r_dof_nearend", "abc"),
                ("r_dof_farend", "inf"),
                ("r_dof_farstart", "NaN"),
            ],
            scene,
        );
        assert_eq!(dof, scene);
    }

    #[test]
    fn a_forced_disable_beats_the_other_settings() {
        let scene = ranges((1.0, 50.0), (900.0, 5000.0), 1.0);
        let (dof, _) = forced(&[("r_dof_nearend", "99"), ("r_dof_enable", "0")], scene);
        assert_eq!(dof, DepthOfField::default());
        let (dof, _) = forced(&[("r_dof_enable", "1")], scene);
        assert_eq!(dof, scene);
    }

    #[test]
    fn any_forced_setting_resets_to_the_scene_ranges_first() {
        let scene = ranges((1.0, 50.0), (900.0, 5000.0), 1.0);
        let mut dof = DepthOfField::default();
        let mut bias = 0.5;
        apply_forced_settings(
            &mut dof,
            &mut bias,
            scene,
            [("r_dof_nearblur", "2")].into_iter(),
        );
        assert_eq!(dof.near_end, 50.0);
        assert_eq!(dof.near_blur, 2.0);
        // unrelated names leave the incoming ranges alone
        let mut dof = DepthOfField::default();
        apply_forced_settings(
            &mut dof,
            &mut bias,
            scene,
            [("sv_hostname", "1")].into_iter(),
        );
        assert_eq!(dof, DepthOfField::default());
    }
}
