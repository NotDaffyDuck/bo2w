//! The plugins every launch adds on top of Bevy's defaults, and the settings
//! they run under.
//!
//! The environment steers three choices, so a test run can change them
//! without a rebuild: how fast the window updates ([`Pacing`]), what the
//! graphics device is asked for ([`wgpu_settings`]), and what a process with no
//! window leaves out ([`without_devices`]). Each is a plain function of its
//! inputs; the code that reads the environment only gathers them.

use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::sync::OnceLock;
use std::time::Duration;

use assets::ZoneAssetPlugin;
use audio::SoundOutPlugin;
use bevy::app::{PluginGroupBuilder, ScheduleRunnerPlugin, SubApp};
use bevy::core_pipeline::{Core2d, Core3d};
use bevy::ecs::schedule::{InternedScheduleLabel, ScheduleLabel, SingleThreadedExecutor};
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::render::{
    ExtractSchedule, Render, RenderApp, RenderPlugin as BevyRenderPlugin,
    pipelined_rendering::PipelinedRenderingPlugin,
    renderer::RenderGraph,
    settings::{MemoryHints, RenderCreation, WgpuFeatures, WgpuSettings},
    sync_world::SyncWorldPlugin,
};
use bevy::window::ExitCondition;
use bevy::winit::{UpdateMode, WinitPlugin, WinitSettings};
use bots::AutomatedPlayersPlugin;
use console::PromptPanelPlugin;
use frame::{Headless, ProcessDuty};
use hud::OverlayModule;
use net::NetworkingPlugin;
use render::PixelsPlugin;
use replay::RecordingPlugin;
use session::LobbyLifecyclePlugin;
use ui::MenuHudPlugin;

/// `0` or `false` or `off` turns pipelined rendering off, anything else on.
const PIPELINED_RENDERING_ENV: &str = "BO2MP_PIPELINED_RENDERING";
/// A frame count: how far the CPU may run ahead of the GPU.
const FRAME_LATENCY_ENV: &str = "BO2MP_FRAME_LATENCY";
/// `performance` asks for the graphics memory in big blocks.
const GPU_MEMORY_ENV: &str = "BO2MP_GPU_MEMORY";
/// Frames a second the window updates at while another window has focus;
/// `0` is no limit.
const BACKGROUND_FPS_ENV: &str = "BO2MP_BACKGROUND_FPS";

/// The time between updates of a run nobody is looking at: about 30 a second.
/// The server keeps its own 20 Hz, so this costs the PC little.
const QUIET_TICK: Duration = Duration::from_millis(33);

/// What the renderer needs from the graphics device beyond Bevy's defaults:
/// 16-bit normalised and block-compressed texture formats, line-mode
/// polygons, and the texture arrays it indexes per draw.
const REQUIRED_FEATURES: WgpuFeatures = WgpuFeatures::TEXTURE_FORMAT_16BIT_NORM
    .union(WgpuFeatures::TEXTURE_COMPRESSION_BC)
    .union(WgpuFeatures::POLYGON_MODE_LINE)
    .union(WgpuFeatures::TEXTURE_BINDING_ARRAY)
    .union(WgpuFeatures::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
    .union(WgpuFeatures::PARTIALLY_BOUND_BINDING_ARRAY);

/// The frame latency when the environment names none, or names something that
/// is not a count. Black Ops II's renderer never sets one and so gets Direct3D's
/// own default of three.
const DEFAULT_FRAME_LATENCY: NonZeroU32 = NonZeroU32::new(3).unwrap();

/// Adds everything a listen server (or a single-player match) runs.
pub fn install_gameplay_plugins(app: &mut App) {
    add_runtime_stack_as(app, ProcessDuty::Listen);
}

/// Adds the game's own plugins for a process that plays `duty`, registers
/// BO2's stock classes, and makes every stage of the frame run its systems one
/// after another.
pub fn add_runtime_stack_as(app: &mut App, duty: ProcessDuty) {
    weapon_t6::class_presets::register();
    insert_window_pacing(app);
    // Added in this order; later plugins read resources the earlier ones insert.
    let game = (ZoneAssetPlugin, MenuHudPlugin, PromptPanelPlugin, net_plugin(duty), (AutomatedPlayersPlugin, OverlayModule), SoundOutPlugin, RecordingPlugin, PixelsPlugin, LobbyLifecyclePlugin);
    app.add_plugins(game);
    run_on_one_thread(app);
}

/// An app with no window that runs the listen-server plugins.
pub fn build_hosting_app() -> App {
    let no_window = WindowPlugin { primary_window: None, ..default() };
    let mut app = App::new();
    app.add_plugins(stock_plugins_hushed_logging(no_window));
    install_gameplay_plugins(&mut app);
    app
}

/// Bevy's default plugins with `window` as the window, the game's graphics
/// settings, and a log that writes from its own thread.
///
/// A headless process (`BO2MP_HEADLESS=1`) gets no window, graphics device,
/// sound device or gamepads.
pub fn stock_plugins_hushed_logging(window: WindowPlugin) -> PluginGroupBuilder {
    let headless = Headless::requested();
    let window = window_for(window, headless, frame_latency());
    let big_blocks = wants_big_blocks(std::env::var(GPU_MEMORY_ENV).ok().as_deref());
    let render_creation = RenderCreation::Automatic(Box::new(wgpu_settings(headless, big_blocks)));
    let plugins = DefaultPlugins.set(window).set(log_plugin()).set(BevyRenderPlugin { render_creation, ..default() });
    if headless {
        return without_devices(plugins);
    }
    with_pipelining(plugins, pipelined_rendering())
}

fn net_plugin(duty: ProcessDuty) -> NetworkingPlugin {
    match duty {
        ProcessDuty::Listen => NetworkingPlugin::listen(),
        ProcessDuty::Dedicated => NetworkingPlugin::dedicated(),
        ProcessDuty::Client => NetworkingPlugin::client(),
        ProcessDuty::Replay => NetworkingPlugin::replay(),
    }
}

// ---- window pacing ----

/// How often the window updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pacing {
    /// A hidden test run: a fixed slow tick, focused or not.
    Quiet,
    /// A test run placed on another monitor. It is never focused there, and
    /// must not slow down for that.
    Continuous,
    /// The window the player uses: flat out while focused, and flat out
    /// unfocused too unless a cap is set. A cap on unfocused windows once
    /// dropped a game to 30 fps whenever another window had focus, which
    /// read as the match getting laggier.
    Player { background_fps: Option<NonZeroU32> },
    /// No window loop to pace.
    Headless,
}

impl Pacing {
    fn choose(hidden: bool, placed: bool, headless: bool, background_fps: Option<&str>) -> Self {
        if hidden {
            Self::Quiet
        } else if placed {
            Self::Continuous
        } else if headless {
            Self::Headless
        } else {
            Self::Player {
                background_fps: background_fps.and_then(|text| text.trim().parse().ok()),
            }
        }
    }

    /// The settings to insert, or `None` to leave Bevy's.
    fn settings(self) -> Option<WinitSettings> {
        Some(match self {
            Self::Quiet => {
                let tick = UpdateMode::reactive_low_power(QUIET_TICK);
                WinitSettings {
                    focused_mode: tick,
                    unfocused_mode: tick,
                }
            }
            Self::Continuous => WinitSettings::continuous(),
            Self::Player { background_fps } => WinitSettings {
                focused_mode: UpdateMode::Continuous,
                unfocused_mode: background_fps.map_or(UpdateMode::Continuous, |fps| {
                    UpdateMode::reactive_low_power(Duration::from_secs_f64(
                        1.0 / f64::from(fps.get()),
                    ))
                }),
            },
            Self::Headless => return None,
        })
    }
}

fn insert_window_pacing(app: &mut App) {
    let headless = Headless::requested() || app.world().contains_resource::<Headless>();
    let pacing = Pacing::choose(
        crate::start::test_window_hidden(),
        crate::start::test_window_placed(),
        headless,
        std::env::var(BACKGROUND_FPS_ENV).ok().as_deref(),
    );
    if let Pacing::Player {
        background_fps: Some(fps),
    } = pacing
    {
        diag::info!(
            Launch,
            "window: {fps} fps while another window has focus ({BACKGROUND_FPS_ENV})"
        );
    }
    if let Some(settings) = pacing.settings() {
        app.insert_resource(settings);
    }
}

// ---- window, graphics device, log ----

/// `window` as this process opens it: none at all when headless, and
/// otherwise with `latency` frames of CPU lead asked for.
fn window_for(mut window: WindowPlugin, headless: bool, latency: NonZeroU32) -> WindowPlugin {
    if headless {
        window.primary_window = None;
        window.exit_condition = ExitCondition::DontExit;
    }
    for shown in window.primary_window.iter_mut() {
        shown.desired_maximum_frame_latency = Some(latency);
    }
    window
}

/// How many frames the surface is asked to let the CPU run ahead of the GPU.
///
/// wgpu treats this as a hint, and the backend may clamp it (Vulkan ties it to
/// the number of swapchain images), so the number granted can differ from the
/// one asked for and only a measurement says which. A value that is not a
/// frame count is the default, with one complaint. Read once, so every window
/// agrees.
fn frame_latency() -> NonZeroU32 {
    static FRAMES: OnceLock<NonZeroU32> = OnceLock::new();
    *FRAMES.get_or_init(|| {
        let Some(asked) = std::env::var_os(FRAME_LATENCY_ENV) else {
            return DEFAULT_FRAME_LATENCY;
        };
        parse_frame_latency(&asked).unwrap_or_else(|| {
            diag::warn!(
                Launch,
                "{FRAME_LATENCY_ENV}={} is not a frame count; using {DEFAULT_FRAME_LATENCY}",
                asked.to_string_lossy(),
            );
            DEFAULT_FRAME_LATENCY
        })
    })
}

fn parse_frame_latency(asked: &OsStr) -> Option<NonZeroU32> {
    asked.to_str()?.trim().parse().ok()
}

/// Whether the environment asks for graphics memory in the big blocks Bevy
/// uses by default. Without it the memory comes in small blocks, so a game
/// holds less it does not use; eight games can share one card.
fn wants_big_blocks(asked: Option<&str>) -> bool {
    asked.is_some_and(|text| text.trim().eq_ignore_ascii_case("performance"))
}

fn wgpu_settings(headless: bool, big_blocks: bool) -> WgpuSettings {
    let base = WgpuSettings::default();
    WgpuSettings {
        features: base.features | REQUIRED_FEATURES,
        // Small blocks unless asked: less memory held for a little more allocating.
        memory_hints: if big_blocks { base.memory_hints.clone() } else { MemoryHints::MemoryUsage },
        // With no backend Bevy makes no graphics device and no render world.
        backends: if headless { None } else { base.backends },
        ..base
    }
}

/// Bevy's log, written from the log writer's thread instead of the frame that
/// logged: a write Windows held up used to stall a frame for seconds.
fn log_lines(_: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::Layer::default().with_writer(diag::stderr_writer),
    ))
}

fn log_plugin() -> LogPlugin {
    #[cfg(feature = "span-stats")]
    return LogPlugin {
        level: Level::WARN, filter: crate::span_stats::FILTER.into(),
        custom_layer: crate::span_stats::layer,
        ..default()
    };
    #[cfg(not(feature = "span-stats"))]
    LogPlugin {
        level: Level::WARN, filter: "warn,bo2mp=info".into(),
        ..default()
    }
}

// ---- what a headless or unpipelined process leaves out ----

/// `plugins` without anything that needs a machine: no window loop, sound
/// device, gamepads or render thread. A plain loop at the quiet tick runs the
/// app.
///
/// With no render world Bevy also leaves out its entity sync, yet the
/// components it syncs still report their removal to it, so a despawned camera
/// or mesh would panic without the sync plugin. Nothing drains its queue here;
/// the records are a few bytes each.
fn without_devices(plugins: PluginGroupBuilder) -> PluginGroupBuilder {
    plugins
        .disable::<WinitPlugin>()
        .disable::<bevy::audio::AudioPlugin>()
        .disable::<bevy::gilrs::GilrsPlugin>().disable::<PipelinedRenderingPlugin>()
        .add(SyncWorldPlugin)
        .add(ScheduleRunnerPlugin::run_loop(QUIET_TICK))
}

/// Whether rendering overlaps the next main frame. Extraction stays the
/// ownership boundary, and the render channel holds one outstanding frame.
///
/// On by default, except on macOS: AppKit lets only the main thread touch the
/// view behind the Metal surface, and with the single-threaded `Render`
/// schedule the render thread would create it and panic.
fn pipelined_rendering() -> bool {
    pipelined_rendering_from(
        std::env::var_os(PIPELINED_RENDERING_ENV).as_deref(),
        cfg!(target_os = "macos"),
    )
}

fn pipelined_rendering_from(asked: Option<&OsStr>, macos: bool) -> bool {
    let Some(asked) = asked else {
        return !macos;
    };
    !matches!(
        asked
            .to_str()
            .map(|text| text.trim().to_ascii_lowercase())
            .as_deref(),
        None | Some("" | "0" | "false" | "off")
    )
}

fn with_pipelining(plugins: PluginGroupBuilder, on: bool) -> PluginGroupBuilder {
    if on {
        plugins
    } else {
        plugins.disable::<PipelinedRenderingPlugin>()
    }
}

// ---- executors ----

fn one_thread(schedule: &mut Schedule) {
    schedule.set_executor(SingleThreadedExecutor::new());
}

fn main_schedules() -> [InternedScheduleLabel; 5] {
    [
        First.intern(),
        PreUpdate.intern(),
        Update.intern(),
        PostUpdate.intern(),
        Last.intern(),
    ]
}

fn render_schedules() -> [InternedScheduleLabel; 5] {
    [
        RenderGraph.intern(),
        Core3d.intern(),
        Core2d.intern(),
        Render.intern(),
        ExtractSchedule.intern(),
    ]
}

fn main_on_one_thread(app: &mut App) {
    for label in main_schedules() {
        app.edit_schedule(label, one_thread);
    }
}

fn render_on_one_thread(render: &mut SubApp) {
    for label in render_schedules() {
        render.edit_schedule(label, one_thread);
    }
}

/// Makes every stage of the frame, and of the render world's, run its systems
/// one after another on the thread that runs the stage. The render world is
/// there only when the render plugin made one.
fn run_on_one_thread(app: &mut App) {
    main_on_one_thread(app);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render_on_one_thread(render);
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::schedule::Schedules;

    use super::*;

    const TWO: NonZeroU32 = NonZeroU32::new(2).unwrap();

    fn fps(count: u32) -> Option<NonZeroU32> {
        NonZeroU32::new(count)
    }

    fn windowed() -> WindowPlugin {
        WindowPlugin { primary_window: Some(Window::default()), ..default() }
    }

    #[test]
    fn a_hidden_run_ticks_slowly_whatever_else_is_set() {
        let pacing = Pacing::choose(true, true, true, Some("60"));
        assert_eq!(pacing, Pacing::Quiet);
        let settings = pacing.settings().unwrap();
        let tick = UpdateMode::reactive_low_power(Duration::from_millis(33));
        assert_eq!(settings.focused_mode, tick);
        assert_eq!(settings.unfocused_mode, tick);
    }

    #[test]
    fn a_placed_run_never_sleeps_for_being_unfocused() {
        let pacing = Pacing::choose(false, true, true, Some("30"));
        assert_eq!(pacing, Pacing::Continuous);
        let settings = pacing.settings().unwrap();
        assert_eq!(settings.focused_mode, UpdateMode::Continuous);
        assert_eq!(settings.unfocused_mode, UpdateMode::Continuous);
    }

    #[test]
    fn a_headless_run_has_no_window_loop_to_pace() {
        let pacing = Pacing::choose(false, false, true, Some("30"));
        assert_eq!(pacing, Pacing::Headless);
        assert!(pacing.settings().is_none());
    }

    #[test]
    fn the_players_window_runs_flat_out_unless_capped() {
        let uncapped = Pacing::choose(false, false, false, None);
        assert_eq!(
            uncapped,
            Pacing::Player {
                background_fps: None
            }
        );
        let settings = uncapped.settings().unwrap();
        assert_eq!(settings.focused_mode, UpdateMode::Continuous);
        assert_eq!(settings.unfocused_mode, UpdateMode::Continuous);
    }

    #[test]
    fn a_background_cap_slows_only_the_unfocused_window() {
        let capped = Pacing::choose(false, false, false, Some(" 30 "));
        assert_eq!(
            capped,
            Pacing::Player {
                background_fps: fps(30)
            }
        );
        let settings = capped.settings().unwrap();
        assert_eq!(settings.focused_mode, UpdateMode::Continuous);
        assert_eq!(
            settings.unfocused_mode,
            UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 30.0))
        );
    }

    #[test]
    fn a_background_cap_that_is_not_a_positive_count_is_no_cap() {
        for text in ["0", "", "-5", "fast", "2.5", "30fps"] {
            assert_eq!(
                Pacing::choose(false, false, false, Some(text)),
                Pacing::Player {
                    background_fps: None
                },
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_frame_latency_is_a_positive_whole_number() {
        assert_eq!(parse_frame_latency(OsStr::new("1")), fps(1));
        assert_eq!(parse_frame_latency(OsStr::new(" 4 ")), fps(4));
        for text in ["0", "", "-1", "two", "2.5", "3 frames"] {
            assert_eq!(parse_frame_latency(OsStr::new(text)), None, "{text:?}");
        }
    }

    #[test]
    fn an_unset_frame_latency_is_black_ops_twos_three() {
        assert_eq!(DEFAULT_FRAME_LATENCY.get(), 3);
    }

    #[test]
    fn graphics_memory_comes_in_big_blocks_only_when_asked() {
        assert!(wants_big_blocks(Some("performance")));
        assert!(wants_big_blocks(Some(" Performance ")));
        for text in [None, Some(""), Some("memory"), Some("perf")] {
            assert!(!wants_big_blocks(text), "{text:?}");
        }
    }

    #[test]
    fn the_device_is_asked_for_what_the_renderer_needs() {
        for big_blocks in [false, true] {
            let wgpu = wgpu_settings(false, big_blocks);
            assert!(wgpu.features.contains(REQUIRED_FEATURES));
            assert!(wgpu.backends.is_some());
        }
    }

    #[test]
    fn small_blocks_are_the_default_and_big_ones_leave_bevys_hint() {
        let small = wgpu_settings(false, false);
        assert!(matches!(small.memory_hints, MemoryHints::MemoryUsage));
        let big = wgpu_settings(false, true);
        assert!(!matches!(big.memory_hints, MemoryHints::MemoryUsage));
    }

    #[test]
    fn a_headless_process_asks_for_no_graphics_backend() {
        assert!(wgpu_settings(true, false).backends.is_none());
        assert!(wgpu_settings(true, true).backends.is_none());
    }

    #[test]
    fn a_windowed_process_asks_its_window_for_the_frame_latency() {
        let plugin = window_for(windowed(), false, TWO);
        let primary = plugin.primary_window.unwrap();
        assert_eq!(primary.desired_maximum_frame_latency, Some(TWO));
        assert!(matches!(plugin.exit_condition, ExitCondition::OnAllClosed));
    }

    #[test]
    fn a_headless_process_has_no_window_and_never_exits_for_lack_of_one() {
        let plugin = window_for(windowed(), true, TWO);
        assert!(plugin.primary_window.is_none());
        assert!(matches!(plugin.exit_condition, ExitCondition::DontExit));
    }

    #[test]
    fn a_launch_with_no_window_keeps_its_exit_rule() {
        let none = WindowPlugin { primary_window: None, ..default() };
        let plugin = window_for(none, false, TWO);
        assert!(plugin.primary_window.is_none());
        assert!(matches!(plugin.exit_condition, ExitCondition::OnAllClosed));
    }

    #[test]
    #[cfg(not(feature = "span-stats"))]
    fn the_log_shows_warnings_and_the_games_own_info() {
        let plugin = log_plugin();
        assert_eq!(plugin.filter, "warn,bo2mp=info");
        assert_eq!(plugin.level, Level::WARN);
    }

    #[test]
    fn pipelined_rendering_is_on_unless_turned_off_or_on_macos() {
        assert!(pipelined_rendering_from(None, false));
        assert!(!pipelined_rendering_from(None, true));
        for text in ["1", "on", "yes", "true", "2"] {
            assert!(pipelined_rendering_from(Some(OsStr::new(text)), false));
            // Asking for it on macOS is allowed; the default is what holds back.
            assert!(pipelined_rendering_from(Some(OsStr::new(text)), true));
        }
        for text in ["", "0", "false", "off", " OFF ", "False"] {
            assert!(
                !pipelined_rendering_from(Some(OsStr::new(text)), false),
                "{text:?}"
            );
        }
    }

    #[test]
    fn turning_pipelining_off_removes_its_plugin() {
        let defaults = DefaultPlugins.build();
        assert!(defaults.enabled::<PipelinedRenderingPlugin>());
        assert!(
            with_pipelining(DefaultPlugins.build(), true).enabled::<PipelinedRenderingPlugin>()
        );
        assert!(
            !with_pipelining(DefaultPlugins.build(), false).enabled::<PipelinedRenderingPlugin>()
        );
    }

    #[test]
    fn a_process_without_devices_runs_on_a_plain_loop() {
        let plugins = without_devices(DefaultPlugins.build());
        assert!(!plugins.enabled::<WinitPlugin>());
        assert!(!plugins.enabled::<bevy::audio::AudioPlugin>());
        assert!(!plugins.enabled::<bevy::gilrs::GilrsPlugin>());
        assert!(!plugins.enabled::<PipelinedRenderingPlugin>());
        assert!(plugins.enabled::<ScheduleRunnerPlugin>());
        assert!(plugins.enabled::<SyncWorldPlugin>());
    }

    #[test]
    fn the_five_main_stages_get_their_executor() {
        let mut app = App::default();
        main_on_one_thread(&mut app);
        let schedules = app.world().resource::<Schedules>();
        for label in main_schedules() {
            assert!(schedules.contains(label), "{label:?}");
        }
    }

    #[test]
    fn the_five_render_stages_get_their_executor() {
        let mut render = SubApp::new();
        render_on_one_thread(&mut render);
        let schedules = render.world().resource::<Schedules>();
        for label in render_schedules() {
            assert!(schedules.contains(label), "{label:?}");
        }
    }

    #[test]
    fn without_a_render_world_only_the_main_stages_change() {
        let mut app = App::default();
        run_on_one_thread(&mut app);
        assert!(app.get_sub_app(RenderApp).is_none());
        assert!(app.world().resource::<Schedules>().contains(Update));
    }
}
