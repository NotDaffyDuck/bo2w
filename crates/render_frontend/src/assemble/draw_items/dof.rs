//! Depth of field and glow settings, plus each frame's blur ranges: derived
//! from the player's aim and weapon and from what the sight rests on, then
//! overridden by whatever a script or the server forces.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Contents the sights' autofocus line stops at (the sight trace's solid
/// world).
const AUTOFOCUS_HIT_MASK: u32 = 0x0080_6c31;

/// How far the autofocus sweep reaches.
const AUTOFOCUS_REACH: f32 = 8192.0;

/// Player-move types from this value up are the dead/spectating ones.
const PM_TYPE_FIRST_NON_PLAYING: i32 = 10;

/// Blur ranges, as distances from the camera. Everything between a start and
/// its end blends from sharp to fully blurred.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FocusBlur {
    pub hand_model_start: f32, pub hand_model_end: f32,
    pub close_begin: f32, pub close_stop: f32,
    pub distant_begin: f32, pub distant_stop: f32,
    pub close_soften: f32, pub distant_soften: f32,
}

impl FocusBlur {
    /// No blur anywhere: every range and strength is zero.
    pub const NONE: Self = Self::from_list([0.0; 8]);

    /// From `[view-model start, view-model end, near start, near end, far
    /// start, far end, near blur, far blur]`.
    const fn from_list(v: [f32; 8]) -> Self {
        Self {
            hand_model_start: v[0],
            hand_model_end: v[1],
            close_begin: v[2],
            close_stop: v[3],
            distant_begin: v[4],
            distant_stop: v[5],
            close_soften: v[6],
            distant_soften: v[7],
        }
    }

    /// Every field, in [`Self::from_list`] order.
    fn fields_mut(&mut self) -> [&mut f32; 8] {
        let Self {
            hand_model_start,
            hand_model_end,
            close_begin: near_start,
            close_stop: near_end,
            distant_begin: far_start,
            distant_stop: far_end,
            close_soften: near_blur,
            distant_soften: far_blur,
        } = self;
        [hand_model_start, hand_model_end, near_start, near_end, far_start, far_end, near_blur, far_blur]
    }

    /// Overwrites the near and far ranges and their blurs (everything but the
    /// view-model range), given in [`Self::from_list`] order.
    fn set_scene_ranges(&mut self, values: [f32; 6]) {
        for (field, value) in self.fields_mut().into_iter().skip(2).zip(values) {
            *field = value;
        }
    }

    /// Does any of the ranges blur anything? Ranges narrower than a unit do
    /// not count, and the far one also needs a blur strength.
    pub fn live_now(self) -> bool {
        let spans = |start: f32, end: f32| end > start + 1.0;
        let far = spans(self.distant_begin, self.distant_stop) && self.distant_soften > 0.0;
        far || spans(self.hand_model_start, self.hand_model_end) || spans(self.close_begin, self.close_stop)
    }
}

/// Console variables for depth of field.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BokehTunables {
    pub enable: bool,
    /// Use `values` instead of the computed ranges.
    pub tweak: bool, pub values: FocusBlur, pub bias: f32,
}

impl Default for BokehTunables {
    fn default() -> Self {
        let values = FocusBlur::from_list([2.0, 8.0, 10.0, 60.0, 1000.0, 7000.0, 6.0, 1.8]);
        let (tweak, enable) = (false, true);
        Self { values, bias: 0.5, enable, tweak }
    }
}

/// Console variables for the glow post effect.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BloomTunables {
    pub enable: bool,
    pub use_trims: bool,
    pub trim_enable: bool,
    pub trim_radius: f32,
    pub trim_intensity: f32,
    pub trim_cutoff: f32,
    pub trim_desaturation: f32,
}

impl Default for BloomTunables {
    fn default() -> Self {
        let defaults = lighting_t6::GlowSettings::tweak_register_defaults();
        Self {
            enable: lighting_t6::BLOOM_INITIAL_ON,
            use_trims: lighting_t6::BLOOM_CUSTOM_SETTINGS_INITIAL,
            trim_enable: defaults.on,
            trim_radius: defaults.radius,
            trim_intensity: defaults.strength,
            trim_cutoff: defaults.glow_floor,
            trim_desaturation: defaults.color_drain,
        }
    }
}

impl BloomTunables {
    /// The tweak values in the form the glow pass takes.
    #[must_use]
    pub fn trim_eye_info(self) -> lighting_t6::GlowSettings {
        let Self {
            trim_enable: enable,
            trim_radius: radius,
            trim_intensity: intensity,
            trim_cutoff: cutoff,
            trim_desaturation: color_drain,
            ..
        } = self;
        lighting_t6::GlowSettings { on: enable, glow_floor: cutoff, color_drain, strength: intensity, radius }
    }
}

/// The depth of field to draw this frame.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BokehFrame {
    /// The focus ranges and blur amounts.
    pub focus: FocusBlur,
    /// `r_dof_bias`: the console value, or the server's when it forces one.
    pub bias: f32,
    /// The near plane the world is drawn with.
    pub stage_near: f32,
    /// The near plane the first-person gun is drawn with.
    pub hand_model_near: f32,
}

pub fn register(bevy_app: &mut App) {
    bevy_app.init_resource::<BokehTunables>()
        .init_resource::<BloomTunables>()
        .init_resource::<BokehFrame>()
        .add_systems(
            Update,
            update_bokeh
                .after(crate::prepare::scene::eye_parms::stamp_staged_stage_eye)
                .in_set(net::ClientStep::Present),
        );
}

/// Moves `current` toward `target` by half the remaining gap, but at least
/// one unit and at most `rate` per twentieth of a second, never passing it.
fn approach(now_val: f32, goal: f32, rate: f32, delta_t: f32) -> f32 {
    let limit = rate / 0.05 * delta_t;
    let half = 0.5 * (goal - now_val).abs();
    let step = match half {
        h if h > limit => limit,
        h => h.max(1.0),
    };
    if goal < now_val {
        return goal.max(now_val - step);
    }
    goal.min(now_val + step)
}

/// Where the ranges settle while aiming: a close near range and a far range
/// starting behind whatever the sight rests on. `focus` is how far that is,
/// when it was measured.
fn aim_target(focus: Option<f32>) -> FocusBlur {
    let open = FocusBlur::from_list([0.0, 0.0, 1.0, 256.0, 2500.0, 10_000.0, 6.0, 0.0]);
    let Some(hit) = focus else {
        return open;
    };
    let pulled_in = (hit < open.close_stop).then(|| (hit - 30.0).max(1.0));
    let far_from = open.distant_begin.max(hit);
    FocusBlur {
        close_stop: pulled_in.unwrap_or(open.close_stop),
        distant_begin: far_from,
        distant_stop: far_from * 4.0,
        ..open
    }
}

/// Between hip and aim (`aim` below 1) the ranges track the fraction itself
/// rather than easing.
fn follow_aim_fraction(scene: &mut FocusBlur, aim: f32, target: &FocusBlur) {
    let hip_far = (1.0 - aim) * 5000.0;
    scene.set_scene_ranges([
        aim,
        aim * target.close_stop,
        hip_far + aim * target.distant_begin,
        hip_far + aim * target.distant_stop,
        6.0,
        0.0,
    ]);
}

/// Fully aimed (or not playing): each range eases toward the target.
fn ease_to_target(scene: &mut FocusBlur, target: &FocusBlur, dt: f32) {
    let goals = [
        (target.close_begin, 50.0),
        (target.close_stop, 50.0),
        (target.distant_begin, 400.0),
        (target.distant_stop, 400.0),
        (6.0, 0.1),
        (0.0, 0.1),
    ];
    for (field, (goal, rate)) in scene.fields_mut().into_iter().skip(2).zip(goals) {
        *field = approach(*field, goal, rate, dt);
    }
}

/// The ranges a killcam uses, sharp on the killer `focus_distance` away.
fn killcam_ranges(focus_distance: f32) -> FocusBlur {
    let (far_from, far_to) = (focus_distance + 100.0, focus_distance + 400.0);
    FocusBlur::from_list([0.0, 0.0, 0.0, 100.0, far_from, far_to, 4.0, 2.0])
}

/// The `r_dof_*` settings that set one range field, in
/// [`DepthOfField::from_list`] order.
const FORCED_FIELDS: [&str; 8] = [
    "r_dof_viewmodelstart",
    "r_dof_viewmodelend",
    "r_dof_nearstart",
    "r_dof_nearend",
    "r_dof_farstart",
    "r_dof_farend",
    "r_dof_nearblur",
    "r_dof_farblur",
];

/// Applies `r_dof_*` settings the server sent or set for this client. Any
/// such setting first puts back the ranges the scene computed; then every
/// finite number overrides its field, and `r_dof_enable 0` switches the
/// effect off whatever else was set.
fn apply_forced_settings<'a>(
    dof: &mut FocusBlur,
    bias: &mut f32,
    scene: FocusBlur,
    settings: impl Iterator<Item = (&'a str, &'a str)> + Clone,
) {
    let lowered = settings.map(|(name, text)| (name.to_ascii_lowercase(), text));
    if lowered.clone().any(|(name, _)| name.starts_with("r_dof")) {
        *dof = scene;
    }
    let mut switched_off = false;
    let numbers = lowered.filter_map(|(key, text)| {
        let value = text.parse::<f32>().ok().filter(|v| v.is_finite())?;
        Some((key, value))
    });
    for (name, value) in numbers {
        match name.as_str() {
            "r_dof_enable" => switched_off = value == 0.0,
            "r_dof_bias" => *bias = value,
            other => {
                if let Some(at) = FORCED_FIELDS.iter().position(|field| *field == other) {
                    *dof.fields_mut()[at] = value;
                }
            }
        }
    }
    if switched_off {
        *dof = FocusBlur::NONE;
    }
}

/// The ranges the console variables pick when no script forced any.
fn console_choice(dvars: &BokehTunables, scene: FocusBlur) -> FocusBlur {
    match (dvars.tweak, dvars.enable) {
        (true, _) => dvars.values,
        (false, true) => scene,
        (false, false) => FocusBlur::NONE,
    }
}

/// What [`update_bokeh`] reads.
#[derive(SystemParam)]
struct DofInputs<'w> {
    view: Res<'w, crate::prepare::scene::eye_parms::StagedStageEye>,
    near_plane: Res<'w, crate::prepare::scene::eye_parms::NearClipSetting>,
    config: Res<'w, BokehTunables>,
    snapshot: Res<'w, net::ShownWorldFrame>,
    me: Res<'w, net::HostViewClient>,
    weapon_table: Option<Res<'w, assets::StagedWeapons>>,
    time: Res<'w, net::FrameClock>,
    world_clip: Res<'w, crate::adapters::motion::debris_piece::PropPhysicsShape>,
    level_generation: Res<'w, frame::LevelGeneration>,
    view_subject: Res<'w, frame::EyeSubject>,
    eye_kick: Option<Res<'w, render_anim::occupancy::eye_kick::CameraRecoilState>>,
}

impl DofInputs<'_> {
    /// How far ahead the sight's line hits the world, when there is a world
    /// to hit.
    fn autofocus(&self) -> Option<f32> {
        let clip = self.world_clip.0.as_ref()?;
        let eye = self.view.eye;
        let reach = eye + self.view.forward * AUTOFOCUS_REACH;
        let hit = clip.sweep_box(eye.to_array(), reach.to_array(), [0.0; 3], [0.0; 3], AUTOFOCUS_HIT_MASK);
        Some(hit.fraction * AUTOFOCUS_REACH)
    }

    /// The weapon's own view-model blur range, scaled by `aim`, when the view
    /// is first person and the weapon has one.
    fn view_model_range(&self, player: &playerstate_t6::game::AvatarState, aim: f32) -> Option<(f32, f32)> {
        let me = self.me.0;
        let replaying_final_death = self.view_subject.replaying_final_death();
        if crate::adapters::motion::other_view::shown_in_third_person(&self.snapshot, me, replaying_final_death) {
            return None;
        }
        let weapon = playerstate_t6::game::get_viewmodel_weapon_index(player);
        let facts = self.weapon_table.as_ref()?.0.summary_for(weapon)?;
        let [start, end] = facts.aim_blur_depth?;
        Some((start * aim, end * aim))
    }

    /// The ranges a script forced on this client, if it forced any.
    fn script_ranges(&self) -> Option<sim::ScriptDepthOfField> {
        let meta = self.snapshot.snapshot()?.meta.for_client(self.me.0)?;
        Some(meta.view_effects.depth_of_field).filter(sim::ScriptDepthOfField::overrides_scene)
    }
}

/// What the system keeps between frames: the eased scene ranges, and the
/// world they were eased in (a new world starts from nothing).
#[derive(Default)]
struct DofMemory {
    scene: FocusBlur,
    world: Option<u64>,
}

impl DofInputs<'_> {
    /// This frame's depth of field; `None` (nothing drawn) while there is no
    /// ready view of a local player.
    fn frame(&self, memory: &mut DofMemory) -> Option<BokehFrame> {
        let world = self.level_generation.0;
        if std::mem::replace(&mut memory.world, world) != world {
            memory.scene = FocusBlur::NONE;
        }
        let me = self.me.0;
        let Some(player) = self.snapshot.player(me).filter(|_| self.view.primed) else {
            memory.scene = FocusBlur::NONE;
            return None;
        };
        let scene = &mut memory.scene;

        let aim = player.weapon_raise_fraction;
        let playing = player.movement_mode < PM_TYPE_FIRST_NON_PLAYING;
        let measured = (aim != 0.0 || !playing).then(|| self.autofocus()).flatten();
        let target = aim_target(measured);
        if playing && aim < 1.0 {
            follow_aim_fraction(scene, aim, &target);
        } else {
            ease_to_target(scene, &target, self.time.step_duration_seconds());
        }

        let (vm_start, vm_end) = self.view_model_range(player, aim).unwrap_or((0.0, 0.0));
        (scene.hand_model_start, scene.hand_model_end) = (vm_start, vm_end);
        let killcam_focus = self.eye_kick.as_ref().and_then(|kick| kick.killcam_focus_distance);
        if let Some(distance) = killcam_focus {
            *scene = killcam_ranges(distance);
        }

        let mut dof = match self.script_ranges() {
            Some(forced) => {
                scene.set_scene_ranges([
                    forced.near_from,
                    forced.near_to,
                    forced.far_from,
                    forced.far_to,
                    forced.near_smear,
                    forced.far_smear,
                ]);
                *scene
            }
            None => console_choice(&self.config, *scene),
        };

        let mut bias = self.config.bias;
        let snapshot = self.snapshot.snapshot();
        if let Some((snapshot, meta)) = snapshot.and_then(|s| Some((s, s.meta.for_client(me)?))) {
            let server = snapshot.meta.objectives.server_info.iter();
            let settings = server
                .chain(meta.client_dvars.iter())
                .map(|pair| (pair.0.as_str(), pair.1.as_str()));
            apply_forced_settings(&mut dof, &mut bias, *scene, settings);
        }
        let stage_near = self.near_plane.value;
        let hand_model_near = self.view.weapon_depth_near;
        Some(BokehFrame { focus: dof, bias, stage_near, hand_model_near })
    }
}

fn update_bokeh(inputs: DofInputs, mut memory: Local<DofMemory>, mut out: ResMut<BokehFrame>) {
    *out = inputs.frame(&mut memory).unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(near: (f32, f32), far: (f32, f32), far_blur: f32) -> FocusBlur {
        FocusBlur::from_list([0.0, 0.0, near.0, near.1, far.0, far.1, 6.0, far_blur])
    }

    fn forced(settings: &[(&str, &str)], scene: FocusBlur) -> (FocusBlur, f32) {
        let (mut dof, mut bias) = (scene, 0.5);
        apply_forced_settings(&mut dof, &mut bias, scene, settings.iter().copied());
        (dof, bias)
    }

    fn sample_scene() -> FocusBlur {
        ranges((1.0, 50.0), (900.0, 5000.0), 1.0)
    }

    #[test]
    fn no_ranges_means_inactive() {
        assert!(!FocusBlur::NONE.live_now());
    }

    #[test]
    fn a_range_must_be_wider_than_a_unit() {
        assert!(!ranges((10.0, 11.0), (0.0, 0.0), 0.0).live_now());
        assert!(ranges((10.0, 11.5), (0.0, 0.0), 0.0).live_now());
    }

    #[test]
    fn the_far_range_needs_blur_strength() {
        assert!(!ranges((0.0, 0.0), (100.0, 900.0), 0.0).live_now());
        assert!(ranges((0.0, 0.0), (100.0, 900.0), 1.0).live_now());
    }

    #[test]
    fn the_viewmodel_range_alone_can_make_it_active() {
        let dof = FocusBlur::from_list([1.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!(dof.live_now());
    }

    #[test]
    fn dvar_defaults_are_enabled_and_untweaked() {
        let dvars = BokehTunables::default();
        assert!(dvars.enable && !dvars.tweak);
        assert_eq!(dvars.bias, 0.5);
        assert!(dvars.values.live_now());
    }

    #[test]
    fn glow_tweaks_round_trip_into_view_info() {
        let mut glow = BloomTunables::default();
        glow.trim_enable = true;
        (glow.trim_radius, glow.trim_intensity) = (1.0, 2.0);
        (glow.trim_cutoff, glow.trim_desaturation) = (3.0, 4.0);
        let info = glow.trim_eye_info();
        assert!(info.on);
        assert_eq!([info.radius, info.strength, info.glow_floor, info.color_drain], [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn approach_never_overshoots() {
        for (from, to) in [(10.0, 10.5), (10.0, 9.5), (5.0, 5.0)] {
            assert_eq!(approach(from, to, 50.0, 1.0), to);
        }
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

    fn near_end_and_far(goal: FocusBlur) -> [f32; 3] {
        [goal.close_stop, goal.distant_begin, goal.distant_stop]
    }

    #[test]
    fn an_unmeasured_aim_target_is_the_open_field_default() {
        assert_eq!(near_end_and_far(aim_target(None)), [256.0, 2500.0, 10_000.0]);
    }

    #[test]
    fn a_near_focus_pulls_the_near_range_in() {
        assert_eq!(near_end_and_far(aim_target(Some(100.0))), [70.0, 2500.0, 10_000.0]);
        assert_eq!(aim_target(Some(10.0)).close_stop, 1.0);
    }

    #[test]
    fn a_distant_focus_pushes_the_far_range_out() {
        assert_eq!(near_end_and_far(aim_target(Some(4000.0))), [256.0, 4000.0, 16_000.0]);
    }

    #[test]
    fn following_the_aim_fraction_runs_between_hip_and_target() {
        let target = aim_target(None);
        let mut scene = FocusBlur::NONE;
        follow_aim_fraction(&mut scene, 0.0, &target);
        assert_eq!([scene.close_begin, scene.close_stop], [0.0, 0.0]);
        assert_eq!([scene.distant_begin, scene.distant_stop], [5000.0, 5000.0]);
        follow_aim_fraction(&mut scene, 0.5, &target);
        assert_eq!(scene.close_stop, 128.0);
        assert_eq!(scene.distant_begin, 2500.0 + 1250.0);
        assert_eq!(scene.distant_soften, 0.0);
    }

    #[test]
    fn easing_reaches_the_target_given_time() {
        let target = aim_target(None);
        let mut scene = FocusBlur::NONE;
        (0..200).for_each(|_| ease_to_target(&mut scene, &target, 0.05));
        assert_eq!([scene.close_stop, scene.distant_stop], [target.close_stop, target.distant_stop]);
        assert_eq!(scene.distant_soften, 0.0);
    }

    #[test]
    fn a_killcam_focuses_just_behind_the_killer() {
        let dof = killcam_ranges(500.0);
        assert_eq!([dof.distant_begin, dof.distant_stop, dof.close_stop], [600.0, 900.0, 100.0]);
    }

    #[test]
    fn the_console_choice_follows_tweak_then_enable() {
        let scene = sample_scene();
        let mut dvars = BokehTunables::default();
        assert_eq!(console_choice(&dvars, scene), scene);
        dvars.enable = false;
        assert_eq!(console_choice(&dvars, scene), FocusBlur::NONE);
        dvars.tweak = true;
        assert_eq!(console_choice(&dvars, scene), dvars.values);
    }

    #[test]
    fn forced_values_override_fields_and_bias() {
        let settings = [("r_dof_nearend", "99"), ("R_DOF_Bias", "0.25"), ("sv_x", "1")];
        let (dof, bias) = forced(&settings, sample_scene());
        assert_eq!([dof.close_stop, dof.distant_begin, bias], [99.0, 900.0, 0.25]);
    }

    #[test]
    fn forced_values_that_are_not_finite_numbers_are_ignored() {
        let settings = [("r_dof_nearend", "abc"), ("r_dof_farend", "inf"), ("r_dof_farstart", "NaN")];
        let (dof, _) = forced(&settings, sample_scene());
        assert_eq!(dof, sample_scene());
    }

    #[test]
    fn a_forced_disable_beats_the_other_settings() {
        let (dof, _) = forced(&[("r_dof_nearend", "99"), ("r_dof_enable", "0")], sample_scene());
        assert_eq!(dof, FocusBlur::NONE);
        let (dof, _) = forced(&[("r_dof_enable", "1")], sample_scene());
        assert_eq!(dof, sample_scene());
    }

    #[test]
    fn any_forced_setting_resets_to_the_scene_ranges_first() {
        let scene = sample_scene();
        let mut bias = 0.5;
        let mut dof = FocusBlur::NONE;
        apply_forced_settings(&mut dof, &mut bias, scene, [("r_dof_nearblur", "2")].into_iter());
        assert_eq!([dof.close_stop, dof.close_soften], [50.0, 2.0]);
        // unrelated names leave the incoming ranges alone
        let mut untouched = FocusBlur::NONE;
        apply_forced_settings(&mut untouched, &mut bias, scene, [("sv_hostname", "1")].into_iter());
        assert_eq!(untouched, FocusBlur::NONE);
    }
}
