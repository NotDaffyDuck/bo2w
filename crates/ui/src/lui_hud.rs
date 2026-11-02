//! bo2zm M4: Black Ops II's own Zombies HUD, run from the game's UI
//! scripts: the HavokScript VM (`hks_t6`) loads LUI and CoD's base, then
//! `ui_mp/t6/hud.lua`; the root opens its `HUD` menu as the engine does,
//! and the game's values reach it as the engine's events
//! (`hud_update_rounds_played`, ...). Each frame its elements (pictures and
//! text, in root units 720 high) are drawn as Bevy UI over the game.
//!
//! On unless `BO2ZM_LUI=0`; the hand-made HUD in `zm_hud` keeps what the
//! engine itself draws in BO2 (the use hint, script HUD text, the hurt
//! overlay).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bevy::prelude::*;
use hks_t6::host::{Drawn, Host};
use hks_t6::value::Value;
use net::{HostViewClient, ShownWorldFrame};

use crate::bo2_font::Bo2Fonts;
use crate::tiers::{OverlayTier, OverlayStackShown};
use crate::{ActiveView, MainWindow, SceneCameras, ScreenBox, absolute_node, covering_node, pinned_node};

/// Font metrics the scripts' text measure reads: font name -> (pixel
/// height, advance per letter, space advance).

/// The scripts' font files and the names `Bo2Fonts` keeps them by
/// (`ui/t6/codbase.lua`'s `fonts/<res>/<file>`).
const FONT_FILES: [(&str, &str); 7] = [
    ("normalFont", "Default"),
    ("smallFont", "Condensed"),
    ("bigFont", "Big"),
    ("extraBigFont", "Morris"),
    ("extraSmallFont", "ExtraSmall"),
    ("italicFont", "Italic"),
    ("smallItalicFont", "SmallItalic"),
];

/// Black Ops II's build number as its menus show it, and where (root
/// units, right-aligned).
const VERSION_TEXT: &str = "43.1701.1";
const VERSION_RECT: [f32; 4] = [1000.0, 70.0, 1125.0, 84.0];

fn font_name(file: &str) -> &'static str {
    let stem = file.rsplit('/').next().unwrap_or(file);
    FONT_FILES
        .iter()
        .find(|(f, _)| f.eq_ignore_ascii_case(stem))
        .map_or("Default", |(_, n)| n)
}

/// On unless `BO2ZM_LUI=0` (then the hand-made HUD in `zm_hud` draws it all).
pub(crate) fn lui_enabled() -> bool {
    std::env::var("BO2ZM_LUI").map_or(true, |v| v != "0")
}

struct LuiHud {
    host: Host,
    metrics: Rc<RefCell<HashMap<String, (f32, HashMap<char, f32>, f32)>>>,
    /// bo2zm M4: Black Ops II's front end (main menu, lobby), not the HUD.
    frontend: bool,
    /// When the front end last had its lobby update (LUI ms).
    lobby_update_at: f64,
    /// What the last lobby update was sent for (open menus, party), and
    /// whether one more goes a second later (a menu opening late).
    lobby_sig: String,
    lobby_again: bool,
    /// Its loading screen is up (a map loads).
    loading_open: bool,
    /// The lobby it opens on (back from a game), else the main menu.
    return_menu: Option<&'static str>,
    /// The left stick's menu step (its direction and when it last stepped).
    stick: Option<(&'static str, f64)>,
    /// BO2's scoreboard is open (Tab held, or the game-over shot).
    board_open: bool,
    /// The scripts it was built from (their count).
    built_from: usize,
    opened: bool,
    /// The mouse in root units, as last sent.
    mouse_at: Option<Vec2>,
    /// A bind row asked for a key and the capture has not finished.
    binding: bool,
    /// His settings' revision last written into BO2's names, and what.
    settings_rev: Option<u64>,
    written: Vec<(String, String)>,
    /// BO2ZM_LUI_DUMP: how many elements the last dump listed.
    dumped: usize,
    /// BO2ZM_LUI_PAUSE_AT's progress, and when the HUD opened.
    test_step: usize,
    /// BO2ZM_LUI_ASKED's one report went out.
    asked_logged: bool,
    /// BO2ZM_LUI_OPEN_MENU's event went out.
    open_menu_sent: bool,
    opened_at: f64,
    /// The values last sent (events go out when one changes).
    sent: Option<Values>,
    host_aspect: f32,
    /// bo2mp: the multiplayer front end's state (`lui_frontend`).
    mp: Option<hks_t6::mp::Mp>,
    /// bo2mp: built for a multiplayer match (BO2's MP HUD), and what its
    /// HUD was sent.
    match_mp: bool,
    seen: crate::lui_frontend::MatchSeen,
    ended_frames: u32,
    /// frames since the final killcam let go: the end scoreboard opened at once comes up empty
    calm_frames: u32,
    /// the end scoreboard's rows were seen drawn (logged once per opening)
    end_rows_logged: bool,
    scoreboard_bits: (bool, bool),
    /// bo2mp: the match flag `disableIngameMenu` is up (the host ended the
    /// game): Esc opens no in-game menu (BO2's CL_ShouldAllowInGameMenu).
    ingame_menu_off: bool,
    /// bo2mp: frames until CAMPAIGN's prompt is closed for him (as its NO
    /// closes it; not while its YES is still running).
    back_in: u8,
    /// bo2mp: when each client was last seen firing (an
    /// enemy firing shows on the minimap for a moment).
    shots: std::collections::HashMap<u32, f64>,
    /// bo2mp: his radar's sweep line and the enemies it pinged.
    sweep: RadarSweep,
    /// bo2zm H28: Public Match asked the relay for its games: the list's
    /// generation then, and when (LUI ms).
    public_search: Option<(u64, f64)>,
    /// bo2zm H28: the relay lobby's match started (the menu music stopped).
    relay_started: bool,
    /// bo2zm H28: the Friends list's lobbies, by the xuid each row shows
    /// (`joinplayersessionbyxuid <xuid>` joins that one).
    friend_ads: Vec<(String, net::DirectoryListing)>,
    /// The relay list's generation the Friends rows came from, and when
    /// (LUI ms) they last asked the relay again.
    friends_seen: (u64, f64),
    /// The last Friends row's xuid number (a lobby keeps its xuid).
    friend_next: u64,
    /// bo2zm H28: JOIN on a friend: his own relay lobby closes, then this
    /// one is joined; its lobby opens for him once he is in (LUI ms of the
    /// ask: it gives up after a while).
    pending_join: Option<(net::DirectoryListing, f64)>,
    /// bo2zm H28: his Custom Games lobby on the relay: the map and mode last
    /// asked for, and when (LUI ms).
    private_host: Option<(String, String, f64)>,
    /// bo2zm H28: the relay ids of the lobby's other players, in the order
    /// of `party` (row xuid N is the (N-2)th: `kickplayer <xuid>`).
    party_ids: Vec<net::RoomOccupantId>,
    /// bo2zm H28: his game and session modes before a Friends JOIN (where
    /// he goes back to if that lobby ends for him).
    pre_join_modes: Option<(Vec<f32>, Vec<f32>)>,
}

/// BO2's radar sweep: a line crosses the map while his radar is up, and
/// each enemy it passes (or who walks across it) is pinged where he stood;
/// the ping fades out.
#[derive(Default)]
struct RadarSweep {
    /// How far across the map the line is (0..1).
    progress: f32,
    last_ms: Option<f64>,
    /// Each enemy where he was last frame (to catch him walking across).
    last_pos: std::collections::HashMap<u32, [f32; 2]>,
    /// Each enemy's last ping: where, facing, when.
    pings: std::collections::HashMap<u32, ([f32; 2], f32, f64)>,
    /// The server's count of "show his enemies" moments (Lightning Strike).
    pulse: i32,
    /// Each enemy's Sensor Grenade sweeps so far (a new one pings him).
    sensor: std::collections::HashMap<u32, u32>,
}

/// One sweep across the map (ms): BO2's `compassRadarUpdateTime` 4 s, and
/// `compassRadarUpdateFastTime` 2 s when two UAVs are up (spy plane mode 2).
const RADAR_SWEEP_MS: f64 = 4000.0;
const RADAR_SWEEP_FAST_MS: f64 = 2000.0;
/// How long a radar ping takes to fade out (`compassRadarPingFadeTime`).
const RADAR_PING_FADE_MS: f64 = 4000.0;

impl LuiHud {
    /// The root's width in units (720 high).
    fn host_width(&self) -> f32 {
        self.host_aspect.max(0.5) * 720.0
    }
}

/// bo2mp: the test steps already run (front end, match).
static FRONT_STEPS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MATCH_STEPS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RETURN_STEPS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// How many times the front end was built (2 and more: back from a match).
static FRONT_BUILDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The HUD's VM lives on the main thread (its values are `Rc`); beside it,
/// the front end's lobby while a game it started runs.
#[derive(Default)]
pub(crate) struct LuiHudSlot(Option<LuiHud>, Option<FrontState>);

/// bo2zm M4: where the front end was when its game started (the playlist,
/// game and session modes, the playlist filter): after the game BO2 goes
/// back to that lobby.
struct FrontState {
    playlist: usize,
    game_modes: Vec<f32>,
    session_modes: Vec<f32>,
    filter: Option<String>,
}

/// The LUI root node (true: BO2's loading screen's, on the loading layer).
#[derive(Component)]
struct LuiRoot(bool);

/// What BO2's menus take in and hand out: his keys and mouse, the
/// classic-menu handshake, the pause, and the engine calls they make.
#[derive(bevy::ecs::system::SystemParam)]
struct LuiIo<'w, 's> {
    /// His monitors (BO2's display options count them).
    monitors: Query<'w, 's, &'static bevy::window::Monitor>,
    /// His controller (Start opens the pause menu; A, B, the d-pad and the
    /// shoulders work its menus).
    pads: Query<'w, 's, &'static bevy::input::gamepad::Gamepad>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    /// His key presses in the order they came: a hitch (a menu's slow
    /// build) must not fold two presses of a key into one.
    key_events: MessageReader<'w, 's, bevy::input::keyboard::KeyboardInput>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    menus: ResMut<'w, frame::LuiMenus>,
    paused: ResMut<'w, frame::GamePaused>,
    /// bo2zm: the cheat menu (Num 1) holds the keys and the solo pause.
    cheat: Res<'w, crate::cheat_menu::CheatMenu>,
    exec: MessageWriter<'w, frame::UiExecCommand>,
    sounds: MessageWriter<'w, frame::MenuCueStart>,
    /// The front end's music (`Engine.PlayMenuMusic`), stopped at Start.
    music: MessageWriter<'w, frame::MenuSongStart>,
    music_stop: MessageWriter<'w, frame::MenuSongHalt>,
    inbox: Option<ResMut<'w, net::PlayerActionIntake>>,
    ids: Option<ResMut<'w, net::ActionRequestCounter>>,
    role: Option<Res<'w, frame::ProcessDuty>>,
    /// The key capture BO2's bind rows start (`Engine.BindCommand`).
    bind: MessageWriter<'w, frame::UiBindRequest>,
    capture: Res<'w, frame::UiBindingCapture>,
    /// His settings, shown and changed by BO2's Settings and Controls.
    settings: ResMut<'w, frame::PlayerPrefsBundle>,
    /// How much the menus blur the world (`Engine.BlurWorld`).
    blur: ResMut<'w, frame::WorldBlur>,
    /// bo2mp: the black dims an in-match menu stacks over the world.
    dim: ResMut<'w, frame::WorldDim>,
    /// bo2mp: the same dims, counted however they are drawn.
    menu_dims: ResMut<'w, frame::MenuDims>,
    /// bo2mp: booted into BO2's multiplayer front end; its match swap.
    front: Option<Res<'w, crate::lui_frontend::Bo2mpFrontend>>,
    /// Where he left the front end for a match (its lobby, reopened).
    lobby_return: Option<ResMut<'w, crate::lui_frontend::LobbyReturn>>,
    launch_id: Option<Res<'w, frame::StartupPersona>>,
    swap: Option<ResMut<'w, session::SessionHandoverOrder>>,
    /// bo2mp: a match loading (BO2's own loading screen draws it).
    backdrop: Option<ResMut<'w, assets::LoadBackdrop>>,
    setup: Option<Res<'w, session::Bo2mpMatchSetup>>,
    /// bo2mp: since when the end scoreboard has been up (the match goes back
    /// to the front end only after BO2's 5 s intermission hold).
    end_board: Option<ResMut<'w, frame::EndBoardShown>>,
    /// His last input (pad or keys) and his pad's kind: BO2's prompts.
    devices: Option<Res<'w, frame::InputDevices>>,
    /// bo2zm H28: the relay's games (Public Match joins one or hosts its
    /// own), the lobby he is in there, and what the menus ask of it.
    master_browser: Option<Res<'w, net::DirectoryBrowser>>,
    dir_link: Option<Res<'w, net::DirectoryLink>>,
    master_action: Option<ResMut<'w, net::QueuedMenuChoice>>,
}

/// Frames after the final killcam lets go before the end scoreboard opens
/// (opened at once it comes up with no rows). Experiment: BO2MP_END_CALM.
/// The fire-mode picture for a weapon's own `weapFireType_t`: full auto (and the minigun and jet
/// gun, which hold the trigger), single or bolt (a stacked gun fires one at a pull), burst (the
/// burst types and the charge shot, BG_WeaponUsesBurstFire).
fn fire_material(t6_fire_type: i32, bolt_action: bool) -> &'static str {
    match t6_fire_type {
        1 | 6 if bolt_action => "hud_mp_firerate_bolt",
        1 | 6 => "hud_mp_firerate_single",
        2..=5 | 8 => "hud_mp_firerate_burst",
        _ => "hud_mp_firerate_auto",
    }
}

fn calm_limit() -> u32 {
    static SLOT_CEILING: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SLOT_CEILING.get_or_init(|| std::env::var("BO2MP_END_CALM").ok().and_then(|v| v.parse().ok()).unwrap_or(45))
}

/// The resolutions BO2's video mode offers (his own first if it is not
/// one of them).
const RESOLUTIONS: [&str; 6] = [
    "1280x720",
    "1366x768",
    "1600x900",
    "1920x1080",
    "2560x1440",
    "3840x2160",
];

/// The speakers this PC can play on, as Windows names them (Settings' Sound
/// Device row, "Speakers (Realtek USB Audio)"), the one it plays on first;
/// asked once.
fn sound_devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
            // Windows' full name ("Speakers (Realtek USB Audio)"), not
            // just "Speakers".
            let named = |d: &cpal::Device| {
                let desc = d.description().ok()?;
                Some(desc.extended().first().map_or(desc.name(), |f| f.as_str()).to_owned())
            };
            let host = cpal::default_host();
            let mut names: Vec<String> = host.default_output_device().as_ref().and_then(named).into_iter().collect();
            if let Ok(all) = host.output_devices() {
                for d in all {
                    if let Some(n) = named(&d)
                        && !names.contains(&n)
                    {
                        names.push(n);
                    }
                }
            }
            names
        })
        .clone()
}

/// His settings as BO2's profile and hardware-profile names.
fn profile_of(s: &frame::PlayerPrefsBundle) -> Vec<(String, String)> {
    let b = |v: bool| if v { "1" } else { "0" }.to_owned();
    vec![
        ("r_mode".into(), s.resolution.to_string()),
        ("r_fullscreen".into(), b(s.full_screen)),
        ("r_vsync".into(), b(s.sync_frames)),
        ("cg_fov_default".into(), s.view_angle.round().to_string()),
        ("sm_enable".into(), b(s.shadows)),
        // The Shadows row (off -1, low 0 .. high 2): ours are on or off.
        (
            "sm_spotQuality".into(),
            if s.shadows { "2" } else { "-1" }.to_owned(),
        ),
        ("cg_mature".into(), b(s.mature)),
        ("ai_corpseCount".into(), s.corpses.to_string()),
        (
            "cg_drawFPS".into(),
            if s.draw_fps { "Simple" } else { "Off" }.to_owned(),
        ),
        ("com_maxfps".into(), s.max_fps.to_string()),
        ("r_fxaa".into(), b(s.fxaa)),
        ("r_aaSamples".into(), s.aa_samples.to_string()),
        ("r_texFilterQuality".into(), s.tex_filter.to_string()),
        (
            "r_picmip".into(),
            if s.bo2_fast && s.tex_quality < 0 {
                3
            } else {
                s.tex_quality
            }
            .to_string(),
        ),
        // BO2 keeps whether texture quality was picked by hand (1) or is
        // automatic (0); its menu reads it.
        (
            "r_picmip_manual".into(),
            b(s.tex_quality >= 0 || s.bo2_fast),
        ),
        ("snd_menu_master".into(), format!("{:.2}", s.main_level)),
        // bo2mp: the Audio tab's other sliders.
        ("snd_menu_voice".into(), format!("{:.2}", s.voice_volume)),
        ("snd_menu_music".into(), format!("{:.2}", s.music_volume)),
        ("snd_menu_sfx".into(), format!("{:.2}", s.sfx_volume)),
        ("snd_menu_cinematic".into(), format!("{:.2}", s.cinematic_volume)),
        ("snd_menu_presets".into(), s.audio_preset.to_string()),
        // PC Controls' mouse slider (0.01 to 30, default 5: our scale).
        ("mouseSensitivity".into(), format!("{:.2}", s.sensitivity)),
        ("cl_freelook".into(), "1".to_owned()),
        // The pad's look sensitivity (BO2 0.4 to 4, default 1; ours x3).
        (
            "input_viewSensitivity".into(),
            format!("{:.2}", s.pad_sensitivity / 3.0),
        ),
        // PC Controls' invert mouse (yes -0.022, no 0.022).
        (
            "m_pitch".into(),
            if s.flip_look { "-0.022" } else { "0.022" }.to_owned(),
        ),
        // The pad's rows: look inversion, vibration.
        ("input_invertpitch".into(), b(s.pad_invert)),
        ("gpad_rumble".into(), b(s.pad_vibration)),
        // The brightness screen's slider (0.5 to 1.5): his brightness.
        ("r_gamma".into(), format!("{:.2}", 1.0 + s.brightness * 2.5)),
        // His name (BO2's `name` dvar: the lobby's player list).
        ("name".into(), s.shown_name.clone()),
        // His "Omni Movement" row (Controls > Move).
        crate::lui_omni_row::profile(s),
    ]
}

/// One of BO2's names, changed by its menus, into his settings.
fn apply_profile(s: &mut frame::PlayerPrefsBundle, key: &str, raw: &str) {
    let on = raw.trim() != "0" && !raw.trim().is_empty();
    let num = raw.trim().parse::<f32>().ok();
    match key {
        "r_mode" => {
            if let Some((cols, rows)) = raw.trim().split_once('x')
                && let (Ok(cols), Ok(rows)) = (cols.parse(), rows.parse())
            {
                s.resolution = frame::ScreenPixelSize::new(cols, rows);
            }
        }
        "r_fullscreen" => s.full_screen = on,
        "r_vsync" => s.sync_frames = on,
        "cg_fov_default" => s.view_angle = num.unwrap_or(s.view_angle),
        "sm_enable" => s.shadows = on,
        "snd_menu_master" => s.main_level = num.unwrap_or(s.main_level).clamp(0.0, 1.0),
        "snd_menu_music" => s.music_volume = num.unwrap_or(s.music_volume).clamp(0.0, 1.0),
        "snd_menu_sfx" => s.sfx_volume = num.unwrap_or(s.sfx_volume).clamp(0.0, 1.0),
        "snd_menu_voice" => s.voice_volume = num.unwrap_or(s.voice_volume).clamp(0.0, 1.0),
        "snd_menu_cinematic" => {
            s.cinematic_volume = num.unwrap_or(s.cinematic_volume).clamp(0.0, 1.0)
        }
        "snd_menu_presets" => s.audio_preset = num.map_or(s.audio_preset, |v| v.clamp(0.0, 4.0) as u8),
        "mouseSensitivity" => s.sensitivity = num.unwrap_or(s.sensitivity).clamp(0.1, 30.0),
        "input_viewSensitivity" => {
            s.pad_sensitivity = num.map_or(s.pad_sensitivity, |v| (v * 3.0).clamp(1.0, 10.0));
        }
        "m_pitch" => s.flip_look = num.is_some_and(|v| v < 0.0),
        "input_invertpitch" => s.pad_invert = on,
        "gpad_rumble" => s.pad_vibration = on,
        "sm_spotQuality" => s.shadows = num.is_none_or(|v| v >= 0.0),
        "cg_mature" => s.mature = on,
        "ai_corpseCount" => s.corpses = num.map_or(s.corpses, |v| v as u8),
        "cg_drawFPS" => {
            s.draw_fps = !raw.trim().eq_ignore_ascii_case("off") && raw.trim() != "0"
        }
        "com_maxfps" => s.max_fps = num.map_or(s.max_fps, |v| v.max(0.0) as u16),
        "r_fxaa" => s.fxaa = on,
        k if k == crate::lui_omni_row::KEY => s.omni_movement = on,
        "r_aaSamples" => s.aa_samples = num.map_or(s.aa_samples, |v| v as u8),
        "r_texFilterQuality" => {
            s.tex_filter = num.map_or(s.tex_filter, |v| v.clamp(0.0, 2.0) as u8)
        }
        "r_picmip_manual" if !on => {
            s.tex_quality = -1;
            s.bo2_fast = false;
        }
        // Texture quality: low is the fast quality (from the next start).
        "r_picmip" => {
            if let Some(v) = num {
                s.tex_quality = v.clamp(-1.0, 3.0) as i8;
                s.bo2_fast = s.tex_quality == 3;
            }
        }
        "r_gamma" => {
            if let Some(g) = num {
                s.brightness = ((g - 1.0) / 2.5).clamp(-0.2, 0.2);
            }
        }
        _ => {}
    }
}

/// His settings <-> BO2's names: his go in when they change (not by the
/// menus), and a name the menus changed comes back into them.
/// (Read through the `ResMut`, written only when a menu changed one: a
/// write marks his settings changed, and the volume, save and shadow
/// systems redo their work each frame they are.)
fn settings_bridge(hud: &mut LuiHud, settings: &mut ResMut<frame::PlayerPrefsBundle>) {
    if hud.settings_rev != Some(settings.revision) {
        // What the game's own code follows straight away: gore and corpses
        // TODO(rw-zm): sim::set_t6_profile comes with the sim/t6 delta (D:/bo2mp/rw-zm-deltas)
        // (the game's rules), texture filtering (the samplers).
        sim::set_t6_profile(settings.mature, settings.corpses);
        asset_core::t6_set_aniso(match settings.tex_filter {
            0 => 1,
            1 => 4,
            _ => 16,
        });
        let pairs = profile_of(&**settings);
        let mut v = hud.host.values.borrow_mut();
        // The name on his lobby row: a name his settings give (the family
        // launcher's profile), else the Steam account this PC signs into.
        v.gamertag = settings.shown_name();
        for (k, x) in &pairs {
            v.profile.insert(k.clone(), x.clone());
            v.dvars.insert(k.clone(), x.clone());
        }
        let mut modes: Vec<String> = RESOLUTIONS.iter().map(|r| (*r).to_owned()).collect();
        let mine = settings.resolution.to_string();
        if !modes.contains(&mine) {
            modes.insert(0, mine);
        }
        v.enums.insert("r_mode".to_owned(), modes);
        hud.written = pairs;
        hud.settings_rev = Some(settings.revision);
        return;
    }
    let changed: Vec<(String, String)> = {
        let v = hud.host.values.borrow();
        hud.written
            .iter()
            .filter_map(|(k, old)| {
                let now = v
                    .profile
                    .get(k)
                    .filter(|x| *x != old)
                    .or_else(|| v.dvars.get(k).filter(|x| *x != old))?;
                Some((k.clone(), now.clone()))
            })
            .collect()
    };
    if changed.is_empty() {
        return;
    }
    for (k, x) in &changed {
        diag::info!(Ui, "menus: option {k} set to {x}");
        apply_profile(settings, k, x);
    }
    (**settings).touch();
}

/// His keys as BO2's pad buttons (what its menus listen for).
/// (BO2 on PC turns menu tabs with D, the Settings and Controls pages; A goes back.)
const MENU_KEYS: [(KeyCode, &str); 9] = [
    (KeyCode::Escape, "secondary"),
    (KeyCode::Enter, "primary"),
    (KeyCode::NumpadEnter, "primary"),
    (KeyCode::ArrowUp, "up"),
    (KeyCode::ArrowDown, "down"),
    (KeyCode::ArrowLeft, "left"),
    (KeyCode::ArrowRight, "right"),
    (KeyCode::KeyA, "shoulderl"),
    (KeyCode::KeyD, "shoulderr"),
];

/// The pad button a key stands for in the menus, if any.
fn menu_button(key: KeyCode) -> Option<&'static str> {
    MENU_KEYS.iter().find_map(|&(code, button)| (code == key).then_some(button))
}

/// A key's name as BO2's prompts give their shortcut ("TAB", "F").
fn shortcut_key(key: KeyCode) -> Option<String> {
    let name = format!("{key:?}");
    if key == KeyCode::Tab {
        return Some("TAB".to_owned());
    }
    name.strip_prefix("Key").filter(|l| l.len() == 1).map(str::to_owned)
}

/// One key shortcut press into BO2's menus: the key, and the command his
/// binds give it (`bind1`; "" for none - a prompt with no bind of its own
/// compares it too, so it must not be nil).
fn key_shortcut(host: &mut Host, key: &str) {
    let bind = host
        .values
        .borrow()
        .binds
        .iter()
        .find(|(_, keys)| keys.iter().any(|k| k.eq_ignore_ascii_case(key)))
        .map(|(cmd, _)| cmd.clone())
        .unwrap_or_default();
    host.root_event(
        "gamepad_button",
        &[
            ("button", Value::str("key_shortcut")),
            ("key", Value::str(key)),
            ("bind1", Value::str(&bind)),
            ("down", Value::Boolean(true)),
            ("qualifier", Value::str("keyboard")),
            ("controller", Value::Num(0.0)),
        ],
    );
}

/// Keys and mouse into BO2's menus: Esc opens its pause menu (the HUD's
/// `open_ingame_menu`, menu `class`); with a menu open, keys become pad
/// buttons and the mouse its pointer (root units).
fn menu_input(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>, console_open: bool, cursor: Option<Vec2>, ui_scale: f32) {
    if console_open || io.cheat.is_up || io.cheat.ate_keys {
        return;
    }
    let key_events: Vec<_> = io.key_events.read().cloned().collect();
    use bevy::input::gamepad::GamepadButton;
    let pad_start = io.pads.iter().any(|p| p.just_pressed(GamepadButton::Start));
    // bo2mp: the front end is all menus (its main menu is not a `Menu.*`
    // child of the root), so his keys and mouse always work them there.
    // bo2zm: the Zombies front end always has a menu up, inside its black cover.
    let front_end = hud.frontend || (io.front.is_some() && !hud.match_mp);
    if !front_end && hud.host.open_menus().is_empty() {
        if (io.keys.just_pressed(KeyCode::Escape) || pad_start) && !hud.ingame_menu_off {
            hud.host.root_event("open_ingame_menu", &[("menuName", Value::str("class"))]);
        }
        return;
    }
    // (Each press and release as it came, so keys that arrive together
    // after a hitch all count; a held key's repeats do not.)
    for ev in &key_events {
        if let Some(button) = menu_button(ev.key_code) {
            match ev.state {
                bevy::input::ButtonState::Pressed if !ev.repeat => {
                    if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                        diag::info!(Ui, "menus: key {:?} pressed as {button}", ev.key_code);
                        for line in hud.host.menu_report() {
                            diag::info!(Ui, "menus: command {line}");
                        }
                    }
                    hud.host.button(button, true);
                }
                bevy::input::ButtonState::Released => hud.host.button(button, false),
                _ => {}
            }
        }
    }
    // BO2 PC's key shortcuts on its button prompts ("TAB After Action
    // Report", "F Friends", "S Search Preferences"): the engine sends
    // gamepad_button "key_shortcut" with the key's name.
    for key in io.keys.get_just_pressed() {
        if let Some(name) = shortcut_key(*key) {
            key_shortcut(&mut hud.host, &name);
        }
    }
    // Letter keys for the prompts that show one (F Friends).
    for key in io.keys.get_just_pressed() {
        let name = format!("{key:?}");
        if let Some(letter) = name.strip_prefix("Key").filter(|l| l.len() == 1) {
            hud.host.key_shortcut(letter);
        }
    }
    // The pad's buttons by BO2's names (Start backs out like Esc).
    for pad in &io.pads {
        for (b, name) in [
            (GamepadButton::South, "primary"),
            (GamepadButton::East, "secondary"),
            (GamepadButton::Start, "secondary"),
            (GamepadButton::West, "alt1"),
            (GamepadButton::North, "alt2"),
            (GamepadButton::DPadUp, "up"),
            (GamepadButton::DPadDown, "down"),
            (GamepadButton::DPadLeft, "left"),
            (GamepadButton::DPadRight, "right"),
            (GamepadButton::LeftTrigger, "shoulderl"),
            (GamepadButton::RightTrigger, "shoulderr"),
        ] {
            if pad.just_pressed(b) {
                hud.host.button(name, true);
            } else if pad.just_released(b) {
                hud.host.button(name, false);
            }
        }
    }
    if cursor.is_none() && io.mouse.get_just_pressed().next().is_some() {
        diag::info!(Ui, "menus: click, but the pointer is off the window");
    }
    // The left stick moves through the menus like the d-pad (one step per
    // push; it repeats while held, as BO2's menus do).
    let stick = io.pads.iter().find_map(|pad| {
        let x = pad
            .get(bevy::input::gamepad::GamepadAxis::LeftStickX)
            .unwrap_or(0.0);
        let y = pad
            .get(bevy::input::gamepad::GamepadAxis::LeftStickY)
            .unwrap_or(0.0);
        let dir = if y > 0.6 {
            Some("up")
        } else if y < -0.6 {
            Some("down")
        } else if x < -0.6 {
            Some("left")
        } else if x > 0.6 {
            Some("right")
        } else {
            None
        };
        dir.or((x.abs() > 0.3 || y.abs() > 0.3).then_some("hold"))
    });
    let now = hud.host.clock_millis();
    match stick {
        Some(dir) if dir != "hold" => {
            let fresh = hud.stick.is_none_or(|(d, _)| d != dir);
            let repeat = hud
                .stick
                .is_some_and(|(d, at)| d == dir && now - at > 220.0);
            if fresh || repeat {
                // The first repeat waits longer (a held stick, not a flick).
                let at = if fresh { now + 230.0 } else { now };
                hud.stick = Some((dir, at));
                hud.host.button(dir, true);
                hud.host.button(dir, false);
            }
        }
        Some(_) => (),
        None => hud.stick = None,
    }
    if let Some(c) = cursor {
        let p = c / ui_scale.max(1e-3);
        if io.mouse.just_pressed(MouseButton::Left) {
            diag::info!(Ui, "menus: click at {:.0},{:.0} (window {:.0},{:.0})", p.x, p.y, c.x, c.y);
        }
        if hud.mouse_at != Some(p) {
            hud.mouse_at = Some(p);
            hud.host.mouse("mousemove", p.x, p.y, "");
        }
        for (b, name) in [(MouseButton::Left, "left"), (MouseButton::Right, "right")] {
            if io.mouse.just_pressed(b) {
                hud.host.mouse("mousedown", p.x, p.y, name);
            }
            if io.mouse.just_released(b) {
                hud.host.mouse("mouseup", p.x, p.y, name);
            }
        }
    }
}

/// The main menu's ZOMBIES: this game started again in BO2's Zombies menus
/// (`frontend t6:zm_nuked`), from the same folder with the same settings
/// (its environment and working folder go with it; a test's front end steps
/// stay with the menu that ran them). Its process id.
fn zombies_menus() -> std::io::Result<u32> {
    restart_in(OtherFrontEnd::Zombies)
}

/// The other mode's front end, as BO2's main menus ask for it: ZOMBIES runs
/// `startZombies`, and the Zombies menu's MULTIPLAYER (YES on "launch
/// Multiplayer?") runs `startMultiplayer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OtherFrontEnd {
    Zombies,
    Multiplayer,
}

impl OtherFrontEnd {
    fn from_command(word: &str) -> Option<Self> {
        if word.eq_ignore_ascii_case("startZombies") {
            Some(Self::Zombies)
        } else if word.eq_ignore_ascii_case("startMultiplayer") {
            Some(Self::Multiplayer)
        } else {
            None
        }
    }

    /// The launch words that open it (`bo2mp frontend t6mp` is the
    /// multiplayer menu a double-click opens).
    fn launch_args(self) -> [&'static str; 2] {
        match self {
            Self::Zombies => ["frontend", "t6:zm_nuked"],
            Self::Multiplayer => ["frontend", "t6mp"],
        }
    }
}

/// This game started again in `front`'s menus (see [`zombies_menus`]). Its
/// process id.
fn restart_in(front: OtherFrontEnd) -> std::io::Result<u32> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    command.args(front.launch_args()).env_remove("BO2ZM_LUI_PAUSE_AT");
    // A hidden test run has no console to share, so the new game would
    // open one of its own on his screen (CREATE_NO_WINDOW).
    #[cfg(windows)]
    if matches!(std::env::var("BO2MP_WINDOW_HIDDEN").as_deref(), Ok("1")) {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let child = command.spawn()?;
    Ok(child.id())
}

/// The party changed (`party_statechanged`, or a menu's
/// `SystemNeedsUpdate(_, "lobby")`): the lobbies get `partylobby_update` and
/// `gamelobby_update` with who is in the lobby (him alone).
/// `word` leaves the public lobby: its countdown and map vote end (true
/// when a countdown was running).
fn left_public_lobby(word: &str, m: &mut hks_t6::mp::MpState) -> bool {
    if !["xstopparty", "xstopallparties", "xstoppartykeeptogether", "xstartprivateparty"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
    {
        return false;
    }
    m.vote = None;
    m.public_start_ms.take().is_some() | m.public_start_in.take().is_some()
}

fn lobby_update(hud: &mut LuiHud) {
    let get = hud.host.field("Engine", "GetPlayersInLobby");
    let members = hud.host.call(get, vec![]).and_then(|r| r.into_iter().next()).unwrap_or(Value::Nil);
    for event in ["partylobby_update", "gamelobby_update"] {
        hud.host.root_event(
            event,
            &[
                ("controller", Value::Num(0.0)),
                ("actualPartyMemberCount", Value::Num(1.0)),
                ("members", members.clone()),
            ],
        );
    }
}

/// Carry out what the menus asked of the engine.
fn engine_calls(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>, local: Option<&HostViewClient>) {
    for call in hud.host.take_calls() {
        diag::info!(Ui, "menus: script calls the game: {call:?}");
        match call {
            hks_t6::host::EngineCall::MenuResponse(menu, response) => {
                let (Some(inbox), Some(ids), Some(local)) =
                    (io.inbox.as_mut(), io.ids.as_mut(), local)
                else {
                    continue;
                };
                let (Some(m), Some(r)) = (
                    sim::menu_response_field(&menu),
                    sim::menu_response_field(&response),
                ) else {
                    diag::warn!(
                        Ui,
                        "menus: reply {menu} {response} is too long"
                    );
                    continue;
                };
                if let Err(e) = inbox.push(
                    local.0,
                    sim::PlayerIntent::MenuResponse {
                        request_id: ids.allocate(),
                        menu: m,
                        response: r,
                    },
                ) {
                    diag::warn!(Ui, "menus: reply dropped: {e}");
                }
            }
            hks_t6::host::EngineCall::Exec(text) => {
                // BO2's config files (`exec default_private.cfg`) and dvar
                // sets, as ExecNow runs them.
                if hks_t6::host::run_command(&hud.host.values, &text) {
                    // (bo2mp: its stats commands on his stats.)
                    if let Some(mp) = hud.mp.as_ref() {
                        hks_t6::mp::apply_stat_commands(mp, &hud.host.values);
                    }
                    continue;
                }
                // BO2's commands as ours; the ones with nothing here to do
                // (pad rumble, party, on-screen keyboard) are dropped.
                let word = text.split_whitespace().next().unwrap_or("");
                // bo2mp: the main menu's prompts. ZOMBIES (YES runs
                // `startZombies`) opens the Zombies menus; CAMPAIGN
                // (`startSingleplayer`) is not built: its prompt closes as
                // NO closes it, and he stays at the main menu.
                // The file list (the Emblem Editor card's CODTv screen) asks
                // for his storage slots: offline they are all there; and
                // its online list of files is answered empty at once.
                if word.eq_ignore_ascii_case("fileshareGetSlots") {
                    hud.host.root_event("fileshare_slots_available", &[("valid", Value::Boolean(true)), ("controller", Value::Num(0.0))]);
                    continue;
                }
                if word.eq_ignore_ascii_case("bo2mpDwSearch") {
                    let folder = text.split_whitespace().nth(1).and_then(|n| n.parse::<f32>().ok()).unwrap_or(0.0);
                    hud.host.root_event(
                        "fileshare_search_complete",
                        &[
                            ("contextid", Value::Num(folder)),
                            ("numresults", Value::Num(0.0)),
                            ("startIndex", Value::Num(0.0)),
                            ("totalFiles", Value::Num(0.0)),
                            ("controller", Value::Num(0.0)),
                        ],
                    );
                    continue;
                }
                // ZOMBIES: as BO2 on PC quits multiplayer for its Zombies
                // program, this game starts again in the Zombies menus
                // (`frontend t6:zm_nuked`, where he picks the map and mode)
                // and this one quits. If it cannot start, Nuketown Zombies
                // starts here as before.
                // MULTIPLAYER in the Zombies menus: the same way back, to
                // the multiplayer menus (it did nothing: he stayed in Zombies).
                if OtherFrontEnd::from_command(word) == Some(OtherFrontEnd::Multiplayer) {
                    match restart_in(OtherFrontEnd::Multiplayer) {
                        Ok(pid) => {
                            diag::info!(Ui, "front end: multiplayer picked: the multiplayer menus open in a new game (`frontend t6mp`, pid {pid}); this one quits");
                            io.exec.write(frame::UiExecCommand { text: "quit".to_owned() });
                        }
                        Err(e) => diag::warn!(Ui, "front end: multiplayer picked, but the multiplayer menus could not start ({e}); he stays here"),
                    }
                    continue;
                }
                if word.eq_ignore_ascii_case("startZombies") {
                    match zombies_menus() {
                        Ok(pid) => {
                            diag::info!(Ui, "front end: zombies picked: the Zombies menus open in a new game (`frontend t6:zm_nuked`, pid {pid}); this one quits");
                            io.exec.write(frame::UiExecCommand { text: "quit".to_owned() });
                        }
                        Err(e) => {
                            diag::warn!(Ui, "front end: zombies picked, but the Zombies menus could not start ({e}); Nuketown Zombies starts here");
                            if let Some(mp) = hud.mp.as_ref() {
                                mp.borrow_mut().zombies_requested = true;
                            }
                        }
                    }
                    continue;
                }
                // The party changed (a lobby opened): the engine tells the
                // lobbies `partylobby_update` with its member count (his
                // party: him alone), which enables FIND MATCH.
                // Its members (him) go with it, and to the game lobby's
                // lists (`gamelobby_update`): BO2's lobbies list who is in
                // them from these events.
                if word.eq_ignore_ascii_case("party_statechanged") {
                    lobby_update(hud);
                    continue;
                }
                // `Engine.SystemNeedsUpdate`: a menu changed what the lobby
                // or the game options show (Change Map sets `ui_mapname`,
                // then asks for both), so the lobby's map picture and name
                // and its buttons redraw from the new values.
                if word.eq_ignore_ascii_case("bo2mpSystemNeedsUpdate") {
                    match text.split_whitespace().nth(1).unwrap_or("") {
                        "lobby" => lobby_update(hud),
                        "game_options" => hud.host.root_event("game_options_update", &[("controller", Value::Num(0.0))]),
                        _ => {}
                    }
                    continue;
                }
                // FIND MATCH (`xstartparty`, then the public game lobby):
                // a Combat Training playlist's lobby counts down to its
                // match (`lui_frontend::tick` starts the countdown and the
                // match).
                // (bo2zm's front end handles it below: it has no `mp` state.)
                if word.eq_ignore_ascii_case("xstartparty") && !hud.frontend {
                    if let Some(mp) = hud.mp.as_ref() {
                        let mut m = mp.borrow_mut();
                        if m.playlist.is_some() {
                            m.public_start_in = Some(hks_t6::playlists::COUNTDOWN_MS);
                            diag::info!(Ui, "front end: find match picked, Combat Training list {:?}", m.playlist);
                        }
                    }
                    continue;
                }
                // He left the public lobby (publicgamelobby.lua's LeaveLobby:
                // stop the party, back to a private one): its countdown and
                // map vote end with it, so Custom Games starts only on START
                // MATCH, with the map set then.
                if !hud.frontend
                    && let Some(mp) = hud.mp.as_ref()
                    && left_public_lobby(word, &mut mp.borrow_mut())
                {
                    diag::info!(Ui, "front end: left the public lobby, its countdown stopped");
                }
                // The public lobby's map voter: MINT_COUNTER, PREVIOUS, RANDOM
                // (`xpartyveto 1` / `2` / `3`).
                if word.eq_ignore_ascii_case("xpartyveto") {
                    let n = text.split_whitespace().nth(1).and_then(|n| n.parse::<i32>().ok()).unwrap_or(0);
                    if let Some(mp) = hud.mp.clone() {
                        crate::lui_frontend::map_vote(&mut hud.host, &mp, n);
                    }
                    continue;
                }
                // The Emblem Editor's commands change the emblem edited
                // (`MpState::emblem`; the editor reads it back through
                // `GetSelectedLayerIconID` / `GetSelectedLayerColor` /
                // `GetUsedLayerCount`).
                if word.get(..5).is_some_and(|w| w.eq_ignore_ascii_case("emble")) && word.len() > 5 {
                    let nums: Vec<f32> = text.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
                    if let Some(mp) = hud.mp.as_ref() {
                        let mut m = mp.borrow_mut();
                        let e = &mut m.emblem;
                        match word.to_ascii_lowercase().as_str() {
                            "emblemclearall" => e.wipe(),
                            "emblemselect" => e.select(nums.first().map_or(0, |n| n.max(0.0) as usize)),
                            "emblemicon" => e.set_icon(nums.first().map_or(-1, |n| *n as i32)),
                            "emblemclear" => e.set_icon(-1),
                            "emblemsetselectedlayericonid" => e.set_icon(nums.first().map_or(-1, |n| *n as i32)),
                            "emblemsetscalemode" => e.set_scale_mode(nums.first().map_or(0, |n| *n as i32)),
                            "emblempalette" if nums.len() >= 4 => e.set_color([nums[0], nums[1], nums[2], nums[3]]),
                            _ => diag::info!(Ui, "bo2zm lui: emblem command `{text}` is not built yet"),
                        }
                    }
                    continue;
                }
                if word.eq_ignore_ascii_case("startSingleplayer") {
                    diag::info!(Ui, "front end: campaign is missing, so its prompt shuts");
                    hud.back_in = 3;
                    continue;
                }
                // End Game as the host: BO2's hostmigration_start with no one
                // to migrate to ends the server (sv.c SV_MatchEnd, killServer),
                // so the player lands back in the menus, our disconnect. The
                // party disband after it: a public online game's lobby goes
                // (cl.c CL_Live_DisbandPartyAfterRound), so the way back
                // leaves it (lui_frontend::open).
                if word.eq_ignore_ascii_case("xpartydisbandafterround") {
                    if let Some(back) = io.lobby_return.as_deref_mut() {
                        back.disbanded = back.menu == "PublicGameLobby";
                        diag::info!(Ui, "front end: party breaks up once the round ends ({})", back.disbanded);
                    }
                    continue;
                }
                // A dvar the menus set as a command (`ui_mapname
                // zm_transit`, Custom Games' default): kept, the map one
                // this game plays.
                if (word.starts_with("ui_") || word.starts_with("party_"))
                    && let Some(value) = text
                        .split_once(' ')
                        .map(|(_, v)| v.trim())
                        .filter(|v| !v.is_empty())
                {
                    let mut v = hud.host.values.borrow_mut();
                    let value = if word.eq_ignore_ascii_case("ui_mapname")
                        && !v.maps.iter().any(|m| m == value)
                    {
                        v.maps.first().cloned().unwrap_or_else(|| value.to_owned())
                    } else {
                        value.to_owned()
                    };
                    let key = v
                        .dvars
                        .keys()
                        .find(|k| k.eq_ignore_ascii_case(word))
                        .cloned()
                        .unwrap_or_else(|| word.to_owned());
                    v.dvars.insert(key, value);
                    if word.eq_ignore_ascii_case("ui_mapname") {
                        hks_t6::host::fit_start(&mut v);
                    }
                    continue;
                }
                // Custom Games' options back to the game's defaults.
                if word.eq_ignore_ascii_case("resetCustomGametype") {
                    let mut v = hud.host.values.borrow_mut();
                    v.settings = v.default_settings.clone();
                    continue;
                }
                let ours = match word {
                    "fast_restart" | "map_restart" => Some("map_restart".to_owned()),
                    "disconnect" | "quit" => Some(word.to_owned()),
                    "hostmigration_start" => Some("disconnect".to_owned()),
                    // bo2zm M4: the playlist's party starts (Solo, Public
                    // Match): its rule is that everyone readies up (Start
                    // Match), as the engine sets from the playlist.
                    "xstartpartyhost" | "xstartparty" if hud.frontend => {
                        let now = hud.host.clock_millis();
                        let mut v = hud.host.values.borrow_mut();
                        v.dvars
                            .insert("party_readyPercentRequired".to_owned(), "1".to_owned());
                        // Public Match (Solo hosts its own): the lobby looks
                        // for players, counts down, then starts with him.
                        if word == "xstartparty" {
                            v.public_lobby_since = Some(now);
                            hks_t6::host::party_size(&mut v);
                            // bo2zm H28: with a relay, the search asks it
                            // for the games it has (public_match).
                            if let Some(finder) = io.master_browser.as_ref() {
                                finder.refresh();
                                hud.public_search = Some((finder.snapshot().generation, now));
                                hud.relay_started = false;
                            }
                        }
                        None
                    }
                    // bo2zm H28: JOIN on a friend's row (CoD.joinPlayer): out
                    // of his own relay lobby, then into the friend's.
                    "joinplayersessionbyxuid" if hud.frontend => {
                        let xuid = text.split_whitespace().nth(1).unwrap_or("");
                        let ad = hud
                            .friend_ads
                            .iter()
                            .find(|(x, _)| x == xuid)
                            .map(|(_, a)| a.clone());
                        let Some(ad) = ad else {
                            diag::warn!(Ui, "menus: friend {xuid} has no lobby to join");
                            continue;
                        };
                        diag::info!(
                            Ui,
                            "menus: joining friend {} ({}/{}) {} {}",
                            ad.name,
                            ad.players,
                            ad.seat_capacity,
                            ad.map,
                            ad.mode
                        );
                        hud.public_search = None;
                        hud.private_host = None;
                        hud.pending_join = Some((ad, hud.host.clock_millis()));
                        if let Some(action) = io.master_action.as_mut() {
                            action.0 = Some(net::DirectoryMenuChoice::ExitRoom);
                        }
                        continue;
                    }
                    // bo2zm H28: KICK on a player's row, Yes (KickPlayerPopup:
                    // BO2's UI_KickPlayer_f -> PartyHost_KickPlayer).
                    "kickplayer" if hud.frontend => {
                        let xuid = text.split_whitespace().nth(1).unwrap_or("");
                        let member = xuid
                            .parse::<usize>()
                            .ok()
                            .and_then(|n| n.checked_sub(2))
                            .and_then(|i| hud.party_ids.get(i).copied());
                        match (member, io.master_action.as_mut()) {
                            (Some(member_id), Some(action)) => {
                                diag::info!(Ui, "menus: {xuid} kicked out of the lobby");
                                action.0 = Some(net::DirectoryMenuChoice::Kick { member_id });
                            }
                            _ => diag::warn!(Ui, "bo2zm lui: kick: no player {xuid}"),
                        }
                        continue;
                    }
                    // The Friends list opened (or its tab): the relay's
                    // lobbies, asked again.
                    "updateInfoForInGameList" | "gameInvitesReceived" | "showParty"
                        if hud.frontend =>
                    {
                        if let Some(finder) = io.master_browser.as_ref() {
                            finder.refresh();
                            hud.friends_seen.1 = hud.host.clock_millis();
                        }
                        continue;
                    }
                    // He left the Public Match lobby: no countdown runs.
                    "xstopparty" | "xstopallparties" | "xstoppartykeeptogether"
                    | "xstartprivateparty"
                        if hud.frontend =>
                    {
                        let mut v = hud.host.values.borrow_mut();
                        if v.public_lobby_since.take().is_some() {
                            hks_t6::host::party_size(&mut v);
                        }
                        // bo2zm H28: and out of the relay's lobby.
                        hud.public_search = None;
                        hud.private_host = None;
                        hud.pending_join = None;
                        v.party.clear();
                        v.party_follower = false;
                        if io.dir_link.is_some()
                            && let Some(action) = io.master_action.as_mut()
                        {
                            action.0 = Some(net::DirectoryMenuChoice::ExitRoom);
                        }
                        None
                    }
                    // bo2zm M4: the front end's Start Match (the party is
                    // ready): load the map the game started for.
                    "xpartyready" | "xpartygo"
                        if hud.frontend
                            && (word == "xpartygo"
                                || text.split_whitespace().nth(1) == Some("1")) =>
                    {
                        hud.host.values.borrow_mut().public_lobby_since = None;
                        io.music_stop.write(frame::MenuSongHalt);
                        // The game's settings as the menus left them (Custom
                        // Games' starting round, difficulty, magic,
                        // headshots only; dog rounds stay off here).
                        let v = hud.host.values.borrow();
                        let picked = ["startRound", "zmDifficulty", "magic", "headshotsonly"]
                            .iter()
                            .filter_map(|k| {
                                v.settings.get(*k).map(|x| ((*k).to_owned(), *x as i32))
                            })
                            .collect();
                        // The map, start location and game type the menus
                        // picked (Tranzit's Survival: Town loads zm_transit;
                        // Turned at the Diner its own map, zm_transit_dr).
                        let dvar = |k: &str| hks_t6::host::dvar_get(&v.dvars, k).unwrap_or_default();
                        let (map, loc, mode) = (
                            dvar("ui_mapname"),
                            dvar("ui_zm_mapstartlocation"),
                            dvar("ui_gametype"),
                        );
                        let map = hks_t6::host::start_map(&map, &loc, &mode);
                        drop(v);
                        sim::set_t6_game_settings(picked);
                        // bo2zm H28: in a relay lobby its host starts
                        // everyone's match (the relay tells the others,
                        // who load it as he does); a player who joined
                        // waits for it.
                        let relay = io.dir_link.as_ref().map(|b| b.state());
                        match &relay {
                            Some(
                                net::DirectoryLinkStatus::Joining { .. }
                                | net::DirectoryLinkStatus::Joined { .. },
                            ) => {
                                diag::info!(Ui, "menus: start match is up to the lobby host");
                                continue;
                            }
                            Some(state @ net::DirectoryLinkStatus::Hosting { .. }) if !map.is_empty() => {
                                let party = 1 + state.other_names().len();
                                diag::info!(
                                    Ui,
                                    "menus: lobby host starts the match map={map} loc={loc} mode={mode} players={party}"
                                );
                                sim::set_t6_party_size(party);
                                frame::set_bo2_zm_start(&map, &loc, &mode);
                                let private = if state.mode().is_some_and(|m| m.ends_with(":private")) {
                                    ":private"
                                } else {
                                    ""
                                };
                                if let Some(action) = io.master_action.as_mut() {
                                    action.0 = Some(net::DirectoryMenuChoice::BeginGame {
                                        map: format!("t6:{map}"),
                                        mode: format!("{mode}:{loc}{private}"),
                                    });
                                }
                                continue;
                            }
                            _ => sim::set_t6_party_size(0),
                        }
                        frame::set_bo2_menu_started();
                        frame::bo2_frontend().map(|zone| {
                            if map.is_empty() {
                                return format!("map {zone}");
                            }
                            diag::info!(
                                Ui,
                                "menus: match starting map={map} loc={loc} mode={mode}"
                            );
                            frame::set_bo2_zm_start(&map, &loc, &mode);
                            let prefix = if zone.starts_with("t6:") { "t6:" } else { "" };
                            format!("map {prefix}{map}")
                        })
                    }
                    _ => None,
                };
                match ours {
                    None => diag::info!(Ui, "bo2zm lui: engine command `{text}` has nothing to do here"),
                    Some(line) => {
                        io.exec.write(frame::UiExecCommand { text: line });
                    }
                }
            }
            hks_t6::host::EngineCall::Sound(alias) => {
                io.sounds.write(frame::MenuCueStart { alias });
            }
            hks_t6::host::EngineCall::FetchLeagueTeams => {
                // Offline: the answer is at once, one result (his Solo card).
                hud.host.root_event(
                    "league_team_info_fetched",
                    &[("success", Value::Boolean(true)), ("numResults", Value::Num(1.0))],
                );
            }
            // PlayMusic("") is how the menus stop the music (the match starting).
            hks_t6::host::EngineCall::PlayMusic(alias) if alias.is_empty() => {
                io.music_stop.write(frame::MenuSongHalt);
            }
            hks_t6::host::EngineCall::PlayMusic(alias) => {
                io.music.write(frame::MenuSongStart { alias });
            }
            hks_t6::host::EngineCall::BlurWorld(amount) => {
                io.blur.0 = amount.max(0.0);
            }
            hks_t6::host::EngineCall::BindCommand(command, _index) => {
                // The game's own capture binds the next key he presses.
                io.bind.write(frame::UiBindRequest { command });
                hud.binding = true;
            }
            hks_t6::host::EngineCall::Event(name, fields) => {
                let fields: Vec<(&str, Value)> = fields
                    .iter()
                    .map(|(k, n)| (k.as_str(), Value::Num(*n)))
                    .collect();
                hud.host.root_event(&name, &fields);
            }
        }
    }
}

/// bo2zm H28: how long Public Match waits for the relay's list of games
/// (ms) before it hosts its own.
const PUBLIC_LOOKUP_MS: f64 = 3000.0;

/// bo2zm H28: Public Match through the relay. Its search joins the first
/// lobby there playing the same map, start and mode with room left (and
/// nothing he lacks), else he hosts his own; the lobby then lists who is in
/// it, and its countdown starts again when someone joins.
fn public_match(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>) {
    if !hud.frontend {
        return;
    }
    let now = hud.host.clock_millis();
    if let Some((generation, at)) = hud.public_search {
        let Some(browser) = io.master_browser.as_ref() else {
            hud.public_search = None;
            return;
        };
        let list = browser.snapshot();
        if (list.generation <= generation || list.loading) && now - at < PUBLIC_LOOKUP_MS {
            return;
        }
        hud.public_search = None;
        let v = hud.host.values.borrow();
        let dvar = |k: &str| hks_t6::host::dvar_get(&v.dvars, k).unwrap_or_default();
        let (map, loc, mode) = (
            dvar("ui_mapname"),
            dvar("ui_zm_mapstartlocation"),
            dvar("ui_gametype"),
        );
        drop(v);
        if map.is_empty() { return; }
        let map = format!("t6:{}", hks_t6::host::start_map(&map, &loc, &mode));
        let token = format!("{mode}:{loc}");
        let open = list.adverts.iter().find(|a| {
            !a.underway
                && !a.locked
                && a.players < a.seat_capacity
                && a.missing.0 == 0
                && a.map == map
                && a.mode == token
        });
        let action = match open {
            Some(a) => {
                diag::info!(
                    Ui,
                    "menus: joining open game {} ({}/{}) {map} {token}",
                    a.name,
                    a.players,
                    a.seat_capacity
                );
                net::DirectoryMenuChoice::Join {
                    listing_token: a.id,
                    map,
                    mode: token,
                }
            }
            None => {
                diag::info!(
                    Ui,
                    "menus: hosting open game {map} {token} ({} games listed)",
                    list.adverts.len()
                );
                net::DirectoryMenuChoice::Host { map, mode: token }
            }
        };
        if let Some(pending) = io.master_action.as_mut() {
            pending.0 = Some(action);
        }
        return;
    }
    // The host removed him from his lobby (BO2's PartyClient_HandleKicked).
    if hud.host.values.borrow().party_follower
        && io.dir_link.as_ref().is_some_and(|b| b.state().kicked())
    {
        party_ended(hud, io, "@PLATFORM_KICKEDFROMPARTY");
        return;
    }
    // The host of his Custom Games lobby left: BO2 moves a party to a new
    // host only in public games (PartyMigrate_IsEnabled); this one ends.
    if hud.host.values.borrow().party_follower
        && hud.host.values.borrow().game_modes.contains(&1.0)
        && io.dir_link.as_ref().is_some_and(|b| b.state().host_gone())
    {
        party_ended(hud, io, "@XBOXLIVE_PARTYENDED");
        return;
    }
    // Who is in his relay lobby (none once it closed or he left it).
    let (names, follower, in_match) = match io.dir_link.as_ref() {
        Some(link) => {
            let status = link.state();
            let follower = matches!(
                status,
                net::DirectoryLinkStatus::Joining { .. } | net::DirectoryLinkStatus::Joined { .. }
            );
            hud.party_ids = status.other_ids();
            (status.other_names(), follower, status.underway())
        }
        None => {
            hud.party_ids.clear();
            (Vec::new(), false, false)
        }
    };
    let (joined, host_changed) = {
        let mut v = hud.host.values.borrow_mut();
        if names != v.party || follower != v.party_follower {
            let joined = names.len() > v.party.len() || (follower && !v.party_follower);
            let host_changed = follower != v.party_follower;
            diag::info!(Ui, "menus: lobby members {names:?} (in a friend's lobby: {follower})");
            v.party = names;
            v.party_follower = follower;
            // Someone came in: the countdown starts over.
            if joined && v.public_lobby_since.is_some() {
                v.public_lobby_since = Some(now - hks_t6::host::PUBLIC_SEARCH_MS);
            }
            (joined, host_changed)
        } else {
            (false, false)
        }
    };
    // The lobby's buttons follow (BO2's party_joined / party_update_host:
    // a player who joined someone's has no Start Match).
    if host_changed {
        hud.host.root_event(
            "party_update_host",
            &[
                ("controller", Value::Num(0.0)),
                ("isHost", Value::Boolean(!follower)),
            ],
        );
    }
    if joined {
        hud.host
            .root_event("party_joined", &[("controller", Value::Num(0.0))]);
    }
    // The host started the match: the lobby's music stops as it loads.
    if in_match && !hud.relay_started {
        hud.relay_started = true;
        hud.host.values.borrow_mut().public_lobby_since = None;
        io.music_stop.write(frame::MenuSongHalt);
    } else if !in_match {
        hud.relay_started = false;
    }
}

/// bo2zm H28: his friend's lobby ended for him (BO2's
/// PartyClient_DisconnectFromHost): he is out of it, back where he joined
/// from, and the notice says why (popup_xboxlive_partyended shows the
/// partyend_reason dvar).
fn party_ended(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>, why: &str) {
    diag::info!(Ui, "menus: the friend's lobby closed ({why})");
    {
        let mut v = hud.host.values.borrow_mut();
        v.party.clear();
        v.party_follower = false;
        v.dvars
            .insert("partyend_reason".to_owned(), why.to_owned());
        match hud.pre_join_modes.take() {
            Some((game, session)) => {
                v.game_modes = game;
                v.session_modes = session;
            }
            None => v.game_modes.retain(|m| *m != 1.0),
        }
        hks_t6::host::party_size(&mut v);
    }
    hud.party_ids.clear();
    hud.pending_join = None;
    hud.private_host = None;
    if let Some(action) = io.master_action.as_mut() {
        action.0 = Some(net::DirectoryMenuChoice::ExitRoom);
    }
    for menu in ["MainLobby", "popup_xboxlive_partyended"] {
        hud.host.root_event(
            "open_menu",
            &[
                ("controller", Value::Num(0.0)),
                ("menuName", Value::str(menu)),
            ],
        );
    }
}

/// bo2zm H28: how long a Friends JOIN waits to be in the friend's lobby
/// (ms) before it gives up.
const JOIN_WAIT_MS: f64 = 15000.0;
/// ... how often the Friends rows ask the relay again (ms).
const FRIENDS_REFRESH_MS: f64 = 5000.0;
/// ... how long his Custom Games lobby waits for the relay (ms) before it
/// asks again.
const PRIVATE_HOST_RETRY_MS: f64 = 5000.0;

/// bo2zm H28: one Friends row for a lobby on the relay: its host, where it
/// is (the map, how full, started), and whether JOIN can take him there.
fn friend_row(v: &hks_t6::host::EngineValues, xuid: &str, a: &net::DirectoryListing) -> hks_t6::host::Friend {
    let load = a.map.strip_prefix("t6:").unwrap_or(&a.map);
    let mut status = format!(
        "{}  {}/{}",
        hks_t6::host::map_shown_name(v, load),
        a.players,
        a.seat_capacity
    );
    if a.underway {
        status.push_str("  In game");
    }
    hks_t6::host::Friend {
        xuid: xuid.to_owned(),
        name: a.name.clone(),
        status,
        open_to_join: !a.underway && !a.locked && a.players < a.seat_capacity && a.missing.0 == 0,
    }
}

/// bo2zm H28: the Friends rows: every other Zombies lobby on the relay.
fn friend_rows(
    hud: &mut LuiHud,
    finder: &net::DirectoryBrowser,
    state: Option<&net::DirectoryLinkStatus>,
    now: f64,
) {
    if now - hud.friends_seen.1 > FRIENDS_REFRESH_MS {
        finder.refresh();
        hud.friends_seen.1 = now;
    }
    let list = finder.snapshot();
    if list.generation != hud.friends_seen.0 {
        hud.friends_seen.0 = list.generation;
        let own = state
            .filter(|s| !s.is_terminal())
            .map(|s| s.identity().lobby_number);
        let mut ads = Vec::new();
        for a in list
            .adverts
            .iter()
            .filter(|a| Some(a.id) != own && a.map.starts_with("t6:zm_"))
        {
            let xuid = match hud.friend_ads.iter().find(|(_, o)| o.id == a.id) {
                Some((x, _)) => x.clone(),
                None => {
                    hud.friend_next += 1;
                    hud.friend_next.to_string()
                }
            };
            ads.push((xuid, a.clone()));
        }
        let rows: Vec<_> = {
            let v = hud.host.values.borrow();
            ads.iter().map(|(x, a)| friend_row(&v, x, a)).collect()
        };
        hud.friend_ads = ads;
        let changed = {
            let mut v = hud.host.values.borrow_mut();
            let changed = v.friends != rows;
            v.friends = rows;
            changed
        };
        if changed {
            let names: Vec<_> = hud
                .friend_ads
                .iter()
                .map(|(x, a)| format!("{x} {} {}", a.name, a.mode))
                .collect();
            diag::info!(Ui, "menus: friends list {names:?}");
            hud.host
                .root_event("friends_updated", &[("controller", Value::Num(0.0))]);
        }
    }
}

/// bo2zm H28: Custom Games over the relay, BO2's LAN party. His Custom
/// Games lobby (ONLINE > CUSTOM GAMES) is a relay lobby; the Friends list
/// shows the others' lobbies; JOIN on one closes his own, joins it, and
/// opens that lobby for him (the host's Start Match starts them all).
fn private_party(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>) {
    if !hud.frontend {
        return;
    }
    let now = hud.host.clock_millis();
    let state = io.dir_link.as_ref().map(|b| b.state());
    // The Friends rows (the relay's list is there only while he is in
    // no relay lobby; in a friend's, his lobby still follows it below).
    if let Some(browser) = io.master_browser.as_ref() {
        friend_rows(hud, browser, state.as_ref(), now);
    }
    let Some(pending) = io.master_action.as_mut() else {
        return;
    };
    // JOIN on a friend: once his own lobby is closed (the bridge gone and
    // the leave applied), join the friend's; once in, its lobby opens.
    if let Some((ad, at)) = hud.pending_join.clone() {
        match &state {
            None if pending.0.is_none() && hud.private_host.is_none() => {
                diag::info!(Ui, "menus: asking to join friend {}", ad.name);
                pending.0 = Some(net::DirectoryMenuChoice::Join {
                    listing_token: ad.id,
                    map: ad.map.clone(),
                    mode: ad.mode.clone(),
                });
                // (Asked once: the mark keeps it from asking again.)
                hud.private_host = Some((ad.map.clone(), ad.mode.clone(), now));
            }
            Some(net::DirectoryLinkStatus::Joined { map, mode: kind, .. }) => {
                hud.pending_join = None;
                hud.private_host = None;
                let map = map.strip_prefix("t6:").unwrap_or(map).to_owned();
                let mut parts = kind.split(':');
                let (gametype, loc) = (
                    parts.next().unwrap_or("").to_owned(),
                    parts.next().unwrap_or("").to_owned(),
                );
                {
                    let mut v = hud.host.values.borrow_mut();
                    hud.pre_join_modes = Some((v.game_modes.clone(), v.session_modes.clone()));
                    // The globe's map (Turned at the Diner loads its own,
                    // zm_transit_dr: the lobby shows TranZit's).
                    let globe = v
                        .maps
                        .iter()
                        .find(|m| map == **m || map.starts_with(m.as_str()))
                        .cloned()
                        .unwrap_or_else(|| map.clone());
                    v.dvars.insert("ui_mapname".to_owned(), globe);
                    v.dvars.insert("ui_zm_mapstartlocation".to_owned(), loc);
                    v.dvars.insert("ui_gametype".to_owned(), gametype);
                    // An online private match (SwitchToPrivateLobby's modes).
                    v.game_modes = vec![1.0];
                    for m in [2.0, 3.0] {
                        if !v.session_modes.contains(&m) {
                            v.session_modes.push(m);
                        }
                    }
                    hks_t6::host::party_size(&mut v);
                }
                diag::info!(Ui, "menus: joined friend {}, lobby opening", ad.name);
                hud.host.root_event(
                    "open_menu",
                    &[
                        ("controller", Value::Num(0.0)),
                        ("menuName", Value::str("PrivateOnlineGameLobby")),
                    ],
                );
            }
            Some(s) if s.is_terminal() || now - at > JOIN_WAIT_MS => {
                diag::warn!(Ui, "menus: could not join friend {} ({s:?})", ad.name);
                hud.pending_join = None;
                hud.private_host = None;
            }
            None if now - at > JOIN_WAIT_MS => {
                diag::warn!(Ui, "menus: join to friend {} did not begin", ad.name);
                hud.pending_join = None;
                hud.private_host = None;
            }
            _ => {}
        }
        return;
    }
    // In a friend's lobby: it follows the host's map, start and mode.
    if let Some(net::DirectoryLinkStatus::Joined { map, mode: kind, .. }) = &state
        && kind.ends_with(":private")
    {
        let map = map.strip_prefix("t6:").unwrap_or(map);
        let mut parts = kind.split(':');
        let (gametype, loc) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let mut v = hud.host.values.borrow_mut();
        let globe = v
            .maps
            .iter()
            .find(|m| map == m.as_str() || map.starts_with(m.as_str()))
            .cloned()
            .unwrap_or_else(|| map.to_owned());
        for (k, x) in [
            ("ui_mapname", globe.as_str()),
            ("ui_zm_mapstartlocation", loc),
            ("ui_gametype", gametype),
        ] {
            if hks_t6::host::dvar_get(&v.dvars, k).as_deref() != Some(x) {
                v.dvars.insert(k.to_owned(), x.to_owned());
            }
        }
        return;
    }
    // His Custom Games lobby (an online private match) is a relay lobby
    // the others' Friends lists show.
    let want = {
        let v = hud.host.values.borrow();
        let custom = v.game_modes.contains(&1.0)
            && v.session_modes.contains(&2.0)
            && !v.party_follower
            && v.public_lobby_since.is_none();
        let dvar = |k: &str| hks_t6::host::dvar_get(&v.dvars, k).unwrap_or_default();
        let (map, loc, mode) = (
            dvar("ui_mapname"),
            dvar("ui_zm_mapstartlocation"),
            dvar("ui_gametype"),
        );
        (custom && !map.is_empty()).then(|| {
            (
                format!("t6:{}", hks_t6::host::start_map(&map, &loc, &mode)),
                format!("{mode}:{loc}:private"),
            )
        })
    };
    let Some((map, mode)) = want else {
        hud.private_host = None;
        return;
    };
    if pending.0.is_some() {
        return;
    }
    let asked = hud
        .private_host
        .as_ref()
        .is_some_and(|(m, d, at)| *m == map && *d == mode && now - at < PRIVATE_HOST_RETRY_MS);
    match &state {
        Some(s @ net::DirectoryLinkStatus::Hosting { .. }) => {
            if (s.map(), s.mode()) != (Some(map.as_str()), Some(mode.as_str())) && !asked {
                diag::info!(Ui, "menus: custom game lobby set to {map} {mode}");
                hud.private_host = Some((map.clone(), mode.clone(), now));
                pending.0 = Some(net::DirectoryMenuChoice::RefreshRoom { map, mode });
            }
        }
        // A closed one goes first (a new lobby needs the relay's list).
        Some(s) if s.is_terminal() => {
            pending.0 = Some(net::DirectoryMenuChoice::ExitRoom);
        }
        Some(_) => (),
        None => {
            if !asked {
                diag::info!(Ui, "menus: hosting custom game {map} {mode}");
                hud.private_host = Some((map.clone(), mode.clone(), now));
                pending.0 = Some(net::DirectoryMenuChoice::Host { map, mode });
            }
        }
    }
}

/// One drawn element (its LUI id) and what it shows (rebuilt on change;
/// its alpha is set in place).
#[derive(Component)]
struct LuiNode {
    id: usize,
    shows: String,
    alpha: f32,
}

/// bo2mp: a multiplayer match's HUD: its game type, map, his stats file.
struct MatchSpec {
    gametype: String,
    map: String,
    stats: std::path::PathBuf,
}

fn build(
    ui: &assets::T6Ui,
    front: Option<&crate::lui_frontend::Bo2mpFrontend>,
    mp_match: Option<&MatchSpec>,
    frontend: bool,
) -> LuiHud {
    let metrics: Rc<RefCell<HashMap<String, (f32, HashMap<char, f32>, f32)>>> = Rc::default();
    let m = metrics.clone();
    let measure = Box::new(move |text: &str, font: &str, h: f32| {
        let m = m.borrow();
        let Some((px, adv, space)) = m.get(font_name(font)) else {
            return text.chars().count() as f32 * h * 0.5;
        };
        let w: f32 = text
            .chars()
            .map(|c| {
                if c == ' ' {
                    *space
                } else {
                    adv.get(&c).copied().unwrap_or(*space)
                }
            })
            .sum();
        w * h / px.max(1.0)
    });
    let scripts = ui
        .scripts
        .iter()
        .map(|(n, b)| (n.clone(), b.to_vec()))
        .collect();
    let mut host = Host::new(scripts, measure);
    if front.is_some() {
        FRONT_BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    // bo2mp: the multiplayer front end's answers go in before the base.
    let mp = match (front, mp_match) {
        (Some(f), _) => Some(crate::lui_frontend::install(&mut host, f)),
        (None, Some(m)) => Some(crate::lui_frontend::match_install(&mut host, &m.stats)),
        _ => None,
    };
    // The text and string tables first: CoD's base reads tables as it
    // loads (CoD.MAX_RANK, CoD.MAX_PRESTIGE from the rank tables).
    {
        let mut v = host.values.borrow_mut();
        v.localize = ui
            .strings
            .iter()
            .map(|(k, t)| (k.to_ascii_uppercase(), t.clone()))
            .collect();
        // BO2ZM_LUI_FINDSTR=<words,...>: every localized string whose text
        // holds one of the words (finding a key; debugging aid).
        if let Ok(words) = std::env::var("BO2ZM_LUI_FINDSTR") {
            let words: Vec<String> = words.split(',').map(str::to_ascii_lowercase).collect();
            let mut hits: Vec<_> = v
                .localize
                .iter()
                .filter(|(_, t)| {
                    let t = t.to_ascii_lowercase();
                    words.iter().any(|w| t.contains(w.as_str()))
                })
                .collect();
            hits.sort();
            for (k, t) in hits {
                diag::info!(Ui, "menus: text {k} is {t:?}");
            }
        }
        diag::info!(
            Ui,
            "menus: tables read {}",
            ui.tables
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        for (name, cols, _rows, cells) in ui.tables.iter() {
            let rows = cells
                .chunks((*cols).max(1))
                .map(<[String]>::to_vec)
                .collect();
            v.tables.insert(name.clone(), rows);
        }
        v.configs = ui.configs.iter().map(|(k, t)| (k.to_ascii_lowercase(), t.clone())).collect();
        // The game settings' defaults (Custom Games' rows start on them).
        for (name, value) in ui.game_settings.iter() {
            v.settings.insert(name.clone(), *value);
        }
    }
    host.load_base();
    if let (Some(m), Some(state), None) = (mp_match, mp.as_ref(), front) {
        crate::lui_frontend::match_load(&mut host, state, &m.gametype, &m.map);
    } else if mp.is_some() {
        crate::lui_frontend::load(&mut host);
    } else {
        zombies_values(&mut host);
        {
            let mut v = host.values.borrow_mut();
            v.front_end = frontend;
            v.default_settings = v.settings.clone();
        }
        // BO2's front end (its own script state, as BO2 keeps it apart from
        // the game's HUD) out of a game; test aid: BO2ZM_LUI_FRONTEND=1 builds
        // it over a game too.
        if frontend || std::env::var_os("BO2ZM_LUI_FRONTEND").is_some() {
            host.require("T6.Main");
            // Its loading screen (the engine opens `Loading` while a map loads).
            host.require("T6.HUD.Loading");
        } else {
            host.require("T6.HUD");
        }
    }
    // bo2zm: the "Omni Movement" row on the Controls menu (10-10).
    crate::lui_omni_row::add(&mut host);
    // Test aid: BO2ZM_LUI_REQUIRE=<module>[,<module>] loads more of BO2's UI
    // (the front end: `T6.Main`).
    if let Ok(more) = std::env::var("BO2ZM_LUI_REQUIRE") {
        for m in more.split(',').filter(|m| !m.is_empty()) {
            host.require(m.trim());
        }
    }
    LuiHud {
        host,
        metrics,
        frontend,
        lobby_update_at: 0.0,
        lobby_sig: String::new(),
        lobby_again: false,
        loading_open: false,
        return_menu: None,
        stick: None,
        built_from: ui.scripts.len(),
        opened: false,
        mouse_at: None,
        test_step: 0,
        asked_logged: false,
        open_menu_sent: false,
        dumped: 0,
        opened_at: 0.0,
        binding: false,
        settings_rev: None,
        written: Vec::new(),
        sent: None,
        host_aspect: Default::default(),
        match_mp: front.is_none() && mp.is_some(),
        mp,
        seen: crate::lui_frontend::MatchSeen::default(),
        board_open: false,
        ended_frames: 0,
        calm_frames: 0,
        end_rows_logged: false,
        scoreboard_bits: (false, false),
        ingame_menu_off: false,
        back_in: 0,
        shots: Default::default(),
        sweep: RadarSweep::default(),
        public_search: None,
        relay_started: false,
        friend_ads: Vec::new(),
        friends_seen: (0, 0.0),
        friend_next: 1000,
        pending_join: None,
        private_host: None,
        party_ids: Vec::new(),
        pre_join_modes: None,
    }
}

/// Nuketown Zombies as the engine reports it: survival, standard rules,
/// the HUD shown, one player on the allies.
/// The Black Ops maps on BO2's globe, in globe order, each near its place
/// (the maps table's globe columns: degrees west, degrees north, as BO2's
/// own rows give Mob of the Dead 122 35). Each sits at least as far from
/// its neighbours as BO2's own closest pair (Nuketown and Mob, 10.7
/// degrees), so every icon and NEW tag stands clear and can be picked.
const BO1_GLOBE: &[(&str, f32, f32)] = &[
    // Nacht der Untoten: west of Origins (11 east 49 north).
    ("zm_prototype", 6.0, 52.0),
    // Verrückt: north of Origins.
    ("zm_asylum", -13.0, 62.0),
    // Der Riese: Breslau, east of Origins.
    ("zm_factory", -28.0, 50.0),
    // Kino der Toten: south of Origins.
    ("zm_theater", -14.0, 37.0),
    // Shi No Numa: Japan.
    ("zm_sumpf", -138.0, 36.0),
    // Five: the Pentagon.
    ("zm_pentagon", 77.0, 39.0),
    // Ascension: Baikonur.
    ("zm_cosmodrome", -63.0, 46.0),
    // Shangri-La: the Himalayas.
    ("zm_temple", -86.0, 28.0),
    // Moon: south of Nuketown (110 40), clear of TranZit and Mob.
    ("zm_moon", 104.0, 27.0),
];

/// Where a map sits on the globe (longitude, latitude), if it is on it.
fn globe_spot(map: &str) -> Option<(f32, f32)> {
    BO1_GLOBE.iter().find_map(|&(name, lon, lat)| (name == map).then_some((lon, lat)))
}

fn zombies_values(host: &mut Host) {
    let hud_bit = host
        .field("CoD", "BIT_HUD_VISIBLE")
        .as_num()
        .map_or(0, |n| n as i32);
    let allies = host
        .field("CoD", "TEAM_ALLIES")
        .as_num()
        .map_or(1, |n| n as i32);
    // An offline game (Restart Level in the pause menu).
    let offline = host.field("CoD", "SESSIONMODE_OFFLINE").as_num();
    let mut v = host.values.borrow_mut();
    for (k, x) in [
        ("r_fontResolution", "720"),
        ("ui_gametype", "zstandard"),
        ("g_gametype", "zstandard"),
        ("ui_zm_gamemodegroup", "zsurvival"),
        ("ui_zm_mapstartlocation", "nuked"),
        ("ui_mapname", "zm_nuked"),
        ("mapname", "zm_nuked"),
        // He hosts his own game (CoD.isHost: the leave-game popup ends it).
        ("sv_running", "1"),
        // The engine's own (the front end reads them).
        ("developer", "0"),
        // His party: eight in the public lobby, four in a game (BO2's
        // zombies party; `party_size` switches it), one at this PC.
        ("party_maxplayers", "8"),
        ("party_maxlocalplayers", "1"),
        // Each lobby kind's local players (one: this PC) and party size.
        ("party_maxlocalplayers_mainlobby", "1"),
        ("party_maxlocalplayers_playermatch", "1"),
        ("party_maxlocalplayers_privatematch", "1"),
        ("party_maxlocalplayers_theater", "1"),
        ("party_maxlocalplayers_systemlink", "1"),
        ("party_maxlocalplayers_local_splitscreen", "1"),
        ("party_maxplayers_privatematch", "4"),
        ("party_maxplayers_theater", "4"),
        ("party_maxplayers_systemlink", "4"),
        ("party_maxplayers_local_splitscreen", "1"),
        // The public Zombies lobby: "1 Player (8 Max)" in the real game.
        ("party_maxplayers_partylobby", "8"),
        ("party_playerCount", "1"),
        // Everyone in the party must be ready (he is: Start Match).
        ("party_readyPercentRequired", "1"),
        // Settings' PlayingCue Chat page as the real game first opens it.
        ("cl_voice", "1"),
        // Settings' Sound Device row: the speakers this PC can play on.
        ("sd_can_switch_device", "1"),
    ] {
        v.dvars.insert(k.to_owned(), x.to_owned());
    }
    let speakers = sound_devices();
    if let Some(dev) = speakers.first() {
        v.dvars.insert("sd_xa2_device_name".to_owned(), dev.clone());
        v.dvars.insert("sd_xa2_num_devices".to_owned(), speakers.len().to_string());
        v.enums.insert("sd_xa2_device_name".to_owned(), speakers.clone());
    }
    // The volume sliders the profile keeps, as the real game first has
    // them (Codcaster's run 0 to 2).
    for (k, x) in [
        ("snd_shoutcast_game", "0.25"),
        ("snd_shoutcast_voip", "1"),
        ("snd_voicechat_volume", "1"),
        ("snd_voicechat_record_level", "1"),
    ] {
        v.profile.entry(k.to_owned()).or_insert_with(|| x.to_owned());
    }
    v.settings.insert("startRound".to_owned(), 1.0);
    // The maps this rebuild plays: every Zombies map on BO2's globe.
    v.maps = [
        "zm_transit",
        "zm_nuked",
        "zm_highrise",
        "zm_prison",
        "zm_buried",
        "zm_tomb",
    ]
    .map(str::to_owned)
    .to_vec();
    // The Black Ops maps, when their pack is there (its maps table rows join
    // BO2's): each on the globe at its own real-world place.
    let have: std::collections::BTreeSet<String> = v
        .tables
        .get("zm/mapstable.csv")
        .map(|rows| rows.iter().filter_map(|r| r.first().cloned()).collect())
        .unwrap_or_default();
    for (map, _, _) in BO1_GLOBE {
        if have.contains(*map) {
            v.maps.push((*map).to_owned());
        }
    }
    if let Some(rows) = v.tables.get_mut("zm/mapstable.csv") {
        for r in rows.iter_mut() {
            let Some((lon, lat)) = r.first().and_then(|map| globe_spot(map)) else {
                continue;
            };
            if r.len() > 17 {
                r[16] = lon.to_string();
                r[17] = lat.to_string();
            }
        }
    }
    // In a game the front end started: the map, location and mode it picked
    // (the game-over title names them).
    if let Some((map, loc, mode)) = frame::bo2_zm_choice() {
        for (k, x) in [
            ("ui_mapname", &map),
            ("mapname", &map),
            ("ui_zm_mapstartlocation", &loc),
            ("ui_gametype", &mode),
            ("g_gametype", &mode),
        ] {
            v.dvars.insert(k.to_owned(), x.clone());
        }
        // Its group too: the HUD and, after the game, the lobby picture read it.
        if let Some(g) = hks_t6::host::mode_group(&v.tables, &mode) {
            v.dvars.insert("ui_zm_gamemodegroup".to_owned(), g);
        }
    }
    // One team (the scoreboard draws a team's rows only when it counts
    // it), and Zombies' columns (its `setscoreboardcolumns`).
    v.settings.insert("teamCount".to_owned(), 1.0);
    v.columns = [
        "MPUI_SCORE",
        "MPUI_KILLS",
        "MPUI_DOWNS",
        "MPUI_REVIVES",
        "MPUI_HEADSHOTS",
    ]
    .map(str::to_owned)
    .to_vec();
    v.bits.insert(hud_bit);
    v.team = allies;
    v.session_modes = offline.into_iter().collect();
}

/// The values the HUD shows, from the presented snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
struct Values {
    round: i32,
    score: i32,
    weapon: i32,
    clip: i32,
    /// A dual-wield weapon's left clip.
    clip_left: Option<i32>,
    stock: i32,
    /// bo2mp: each hand's clip is low (BO2 draws that number red): at or
    /// under the gun's low-ammo share of a clip, never for a clip-only gun.
    low_clip: bool,
    low_clip_left: bool,
    /// His grenades and mines (`name:count` by `;`).
    offhand: String,
    /// His HUD client fields (`field:value` by `,`: perks, power-ups).
    icons: String,
    /// The world's HUD client fields (`field:value` by `,`: Mob's parts,
    /// Buried's Time Bomb).
    worldcf: String,
    /// His points counters (`score_cf_damage:3,...`).
    scorecf: String,
    /// His d-pad slots (`slot:weapon` by `;`, the scripts' setactionslot).
    slots: String,
    /// bo2mp: his scorestreaks (`icon:cost` by `;`) and his momentum.
    streaks: String,
    momentum: i32,
    /// The held weapon's name and display-name key (`ZOMBIE_WEAPON_...`).
    weapon_name: String,
    weapon_key: String,
    /// bo2mp: the held gun's fire-rate picture (`hud_mp_firerate_auto`).
    fire_material: &'static str,
    /// bo2mp: a multiplayer match's values.
    mp: Option<crate::lui_frontend::MatchValues>,
    /// The held weapon's inventory type (0 gun, 1 offhand, 2 item - a perk
    /// bottle, the knuckle crack -, 3 alt mode), as the game's HUD reads it.
    weapon_inventory: i32,
    /// The held weapon's Black Ops II `weapType` (7 melee: no ammo counter).
    weapon_type: i32,
    /// His team as the engine keeps it (1 axis, 2 allies), from the
    /// scripts' `sessionteam`: a Turned zombie is axis.
    team: i32,
    /// The game type the game runs (`zcleansed` Turned): a test run starts
    /// without the menus, so the HUD learns it from the game.
    gametype: String,
    /// The scoreboard's rows (`clientnum:name:score:kills:downs:revives:
    /// headshots` by `;`).
    board: String,
    /// The scripts hid his HUD (the game over).
    hud_off: bool,
    /// The scripts hid his ammo counter and points (the game over's
    /// setclientammocounterhide / setclientminiscoreboardhide).
    ammo_hide: bool,
    /// The game-over shot (he watches the intermission camera).
    game_over: bool,
    /// The held gun's heat (`percent:overheated`; empty for a gun that does not overheat).
    heat: String,
}

/// A d-pad weapon's icon.
fn slot_icon(weapon: &str) -> Option<&'static str> {
    Some(match weapon {
        "claymore_zm" => "hud_icon_claymore",
        _ => return None,
    })
}

/// Where a grenade goes in `hud_update_offhand` and the HUD icon it shows.
struct OffhandPick {
    slot: &'static str,
    icon: &'static str,
}

/// The HUD icon of a grenade the engine names in `hud_update_offhand`, and
/// its slot.
fn offhand_slot(weapon: &str) -> Option<OffhandPick> {
    let (slot, icon) = match weapon {
        "frag_grenade_zm" => ("lethal", "hud_us_grenade"),
        "sticky_grenade_zm" => ("lethal", "hud_icon_sticky_grenade"),
        "cymbal_monkey_zm" => ("tactical", "hud_cymbal_monkey"),
        // bo2mp: a multiplayer class's equipment.
        // The MP frag's own hudIcon / ammoCounterIcon / killIcon (common_mp's
        // weapon def) is `hud_grenadeicon`, the green camo pineapple; the
        // blue steel ball `hud_us_grenade` is Zombies' frag.
        "frag_grenade_mp" => ("lethal", "hud_grenadeicon"),
        "sticky_grenade_mp" => ("lethal", "hud_icon_sticky_grenade"),
        "hatchet_mp" => ("lethal", "hud_hatchet"),
        "claymore_mp" => ("lethal", "hud_icon_claymore"),
        "satchel_charge_mp" => ("lethal", "hud_icon_satchelcharge"),
        "bouncingbetty_mp" => ("lethal", "hud_bounce_betty"),
        "flash_grenade_mp" => ("tactical", "hud_us_flashgrenade"),
        "concussion_grenade_mp" => ("tactical", "hud_us_stungrenade"),
        "willy_pete_mp" => ("tactical", "hud_willy_pete"),
        "emp_grenade_mp" => ("tactical", "hud_empgrenade"),
        "trophy_system_mp" => ("tactical", "hud_trophy_system"),
        "tactical_insertion_mp" => ("tactical", "hud_tact_insert"),
        _ => return None,
    };
    Some(OffhandPick { slot, icon })
}

/// (field, value) pairs of `bo2zm_icons` (Mob's Afterlife mana is a
/// float, 0 to 1).
fn icon_fields(value: &str) -> Vec<(&str, f32)> {
    value
        .split(',')
        .filter_map(|e| {
            let (f, v) = e.split_once(':')?;
            Some((f, v.trim().parse().unwrap_or(0.0)))
        })
        .collect()
}

/// A clip from the player's clip table (rows of 12 bytes: the weapon, the
/// right hand's count, the left hand's).
fn clip_of(table: &[u8], weapon: i32, hand: usize) -> i32 {
    table
        .chunks_exact(12)
        .find(|row| i32::from_le_bytes([row[0], row[1], row[2], row[3]]) == weapon)
        .map_or(0, |row| {
            let o = 4 + hand.min(1) * 4;
            i32::from_le_bytes([row[o], row[o + 1], row[o + 2], row[o + 3]])
        })
}

fn read_values(presented: &ShownWorldFrame, local: &HostViewClient) -> Option<Values> {
    let snap = presented.snapshot()?;
    let dvars = snap.meta.script_dvars(local.0);
    // (bo2mp: a killcam still plays over the game-over shot: its widget
    // stays, the rest of the HUD goes with BIT_HUD_VISIBLE.)
    let killcam_on = dvars.string("bo2mp_killcam").is_some_and(|k| k.starts_with("1|") || k.starts_with("2|") || k.starts_with("4|"));
    let hidden = dvars.string("bo2zm_hud_hidden") == Some("1");
    // bo2mp: a multiplayer match has no round.
    let mp = crate::lui_frontend::MatchValues::read(&dvars).map(|mut m| {
        m.rows = crate::lui_frontend::MatchValues::rows_of(&snap.meta);
        m
    });
    // (bo2mp's HUD goes with the game-over shot; bo2zm's stays, flagged
    // `hidden`, for its game-over screen.)
    if hidden && !killcam_on && mp.is_some() {
        return None;
    }
    let round = match dvars.string("bo2zm_round") {
        Some(r) => r.trim().parse().unwrap_or(0),
        None => {
            if mp.is_none() {
                return None;
            }
            0
        }
    };
    let meta = snap.meta.for_client(local.0)?;
    let pl = presented.player(local.0);
    Some(Values {
        round,
        score: meta.score,
        weapon: pl.map_or(0, |p| p.weapon as i32),
        clip: meta.mag_rounds,
        clip_left: pl.filter(|p| p.previous_gun_hand == 1).map(|p| clip_of(&p.magazine_rows, p.weapon as i32, 1)),
        stock: meta.ammo_stock,
        low_clip: false,
        low_clip_left: false,
        offhand: dvars.string("bo2zm_offhand").unwrap_or_default().to_owned(),
        icons: dvars.string("bo2zm_icons").unwrap_or_default().to_owned(),
        worldcf: dvars.string("bo2zm_worldcf").unwrap_or_default().to_owned(),
        scorecf: dvars.string("bo2zm_scorecf").unwrap_or_default().to_owned(),
        slots: dvars.string("bo2zm_actionslots").unwrap_or_default().to_owned(),
        streaks: dvars.string("bo2mp_streaks").unwrap_or_default().to_owned(),
        momentum: dvars.string("bo2mp_momentum").and_then(|m| m.trim().parse().ok()).unwrap_or(0),
        weapon_name: String::new(),
        weapon_key: String::new(),
        fire_material: "",
        mp,
        weapon_inventory: 0,
        weapon_type: 0,
        team: meta.client_state_team,
        gametype: dvars.string("bo2zm_gametype").unwrap_or_default().to_owned(),
        board: dvars.string("bo2zm_scoreboard").unwrap_or_default().to_owned(),
        hud_off: hidden,
        ammo_hide: dvars.string("bo2zm_ammo_hide") == Some("1") || dvars.string("bo2zm_score_hide") == Some("1"),
        game_over: pl.is_some_and(|p| p.movement_mode == playerstate_t6::consts::movement_mode::INTERMISSION),
        heat: dvars.string("bo2zm_heat").unwrap_or_default().to_owned(),
    })
}

/// The engine's events for what changed since `old` (everything the first
/// time).
fn send_changes(host: &mut Host, old: Option<&Values>, new: &Values) {
    if old != Some(new) && std::env::var_os("BO2ZM_LUI_LOG").is_some() {
        diag::info!(Ui, "menus: hud numbers at {:.0} ms {new:?}", host.clock_millis());
    }
    let changed = |f: fn(&Values) -> i64| old.is_none_or(|o| f(o) != f(new));
    if old.is_none_or(|o| o.board != new.board) {
        let mut v = host.values.borrow_mut();
        v.players.clear();
        v.clients.clear();
        for row in new.board.split(';').filter(|r| !r.is_empty()) {
            let mut cells = row.split(':');
            let client = cells.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            v.clients.push(client);
            v.players.push(cells.map(str::to_owned).collect());
        }
    }
    if old.is_some_and(|o| o.ammo_hide != new.ammo_hide) {
        // BO2's HUD bit 12 (CoD.BIT_AMMO_COUNTER_HIDE): its ammo area and
        // score listen for hud_update_bit_12 and read the bit.
        let bit = host
            .field("CoD", "BIT_AMMO_COUNTER_HIDE")
            .as_num()
            .map_or(12, |n| n as i32);
        {
            let mut v = host.values.borrow_mut();
            if new.ammo_hide {
                v.bits.insert(bit);
            } else {
                v.bits.remove(&bit);
            }
        }
        host.root_event(
            &format!("hud_update_bit_{bit}"),
            &[("newValue", Value::Num(f32::from(u8::from(new.ammo_hide))))],
        );
    }
    if changed(|v| i64::from(v.round)) {
        host.root_event(
            "hud_update_rounds_played",
            &[
                ("roundsPlayed", Value::Num(new.round as f32)),
                ("wasDemoJump", Value::Boolean(false)),
            ],
        );
    }
    // His points counters first (as BO2 runs client fields before the
    // scoreboard's update): each step flies its own "+10", and the score's
    // change then flies only what they did not cover (spending, bonuses).
    if let Some(o) = old
        && o.scorecf != new.scorecf
    {
        let was = icon_fields(&o.scorecf);
        for (f, v) in icon_fields(&new.scorecf) {
            let before = was.iter().find(|(g, _)| *g == f).map_or(0.0, |(_, x)| *x);
            if before != v {
                if std::env::var_os("BO2MP_T6_HUDLOG").is_some() {
                    diag::info!(Ui, "menus: points count {f} from {before} to {v}");
                }
                host.root_event(
                    f,
                    &[
                        ("name", Value::str(f)),
                        ("entNum", Value::Num(0.0)),
                        ("newValue", Value::Num(v)),
                        ("oldValue", Value::Num(before)),
                    ],
                );
            }
        }
    }
    if changed(|v| i64::from(v.score)) {
        // One row per player (here the local one, client 0), ours first.
        let rows = hks_t6::value::LuaTable::new_ref();
        let row = hks_t6::value::LuaTable::new_ref();
        row.borrow_mut()
            .set_str("score", Value::Num(new.score as f32));
        row.borrow_mut().set_str("clientNum", Value::Num(0.0));
        rows.borrow_mut().store(Value::Num(1.0), Value::Table(row));
        host.root_event(
            "hud_update_competitive_scoreboard",
            &[
                ("competitivescores", Value::Table(rows)),
                ("selfindex", Value::Num(1.0)),
                ("bWasDemoJump", Value::Boolean(false)),
            ],
        );
    }
    // The game type the game runs: the HUD's Turned and Grief checks read
    // `ui_gametype` (BO2's zombie hides his score and d-pad area only in
    // Turned). Then his team again, now that the checks see it.
    let mode_changed =
        !new.gametype.is_empty() && old.is_none_or(|o| o.gametype != new.gametype);
    if mode_changed {
        let group = hks_t6::host::mode_group(&host.values.borrow().tables, &new.gametype);
        let mut v = host.values.borrow_mut();
        for k in ["ui_gametype", "g_gametype"] {
            v.dvars.insert(k.to_owned(), new.gametype.clone());
        }
        if let Some(g) = group {
            v.dvars.insert("ui_zm_gamemodegroup".to_owned(), g);
        }
    }
    // His team (BO2's `hud_update_team_change`): a Turned zombie (axis)
    // has no d-pad area or score; Grief's scoreboard knows his side.
    if mode_changed || changed(|v| i64::from(v.team)) {
        let name = match new.team {
            1 => Some("TEAM_AXIS"),
            2 => Some("TEAM_ALLIES"),
            _ => None,
        };
        if let Some(team) = name.and_then(|n| host.field("CoD", n).as_num()) {
            host.values.borrow_mut().team = team as i32;
            host.root_event("hud_update_team_change", &[("team", Value::Num(team))]);
        }
    }
    // A gun that overheats (the game sends its heat) shows the heat, not the ammo: the weapon
    // event goes again once the HUD knows.
    if changed(|v| i64::from(v.weapon) | (i64::from(!v.heat.is_empty()) << 40)) {
        let mut values = host.values.borrow_mut();
        values
            .weapon_types
            .insert(new.weapon_name.clone(), new.weapon_type);
        if new.heat.is_empty() {
            values.overheat_weapons.remove(&new.weapon_name);
        } else {
            values.overheat_weapons.insert(new.weapon_name.clone());
        }
        drop(values);
        let mut fields = vec![
            ("weapon", Value::str(&new.weapon_name)),
            ("inventorytype", Value::Num(new.weapon_inventory as f32)),
        ];
        if !new.fire_material.is_empty() {
            fields.push(("fireTypeMaterial", host.material(new.fire_material)));
        }
        host.root_event(
            "hud_update_weapon",
            &fields,
        );
        // A switch shows the new gun's name (not the first one he spawns with).
        if old.is_some() && !new.weapon_key.is_empty() {
            host.root_event(
                "hud_update_weapon_select",
                &[("weaponDisplayName", Value::str(&new.weapon_key))],
            );
        }
    }
    if changed(|v| {
        i64::from(v.clip) | (i64::from(v.stock) << 20) | ((i64::from(v.clip_left.unwrap_or(-1)) & 0xF_FFFF) << 40) | (i64::from(v.low_clip) << 61) | (i64::from(v.low_clip_left) << 62)
    }) {
        let mut fields = vec![
            ("ammoInClip", Value::Num(new.clip as f32)),
            ("ammoStock", Value::Num(new.stock as f32)),
            ("lowClip", Value::Boolean(new.low_clip)),
        ];
        if let Some(left) = new.clip_left {
            fields.push(("ammoInDWClip", Value::Num(left as f32)));
            fields.push(("lowDWClip", Value::Boolean(new.low_clip_left)));
        }
        host.root_event("hud_update_ammo", &fields);
    }
    // BO2's `hud_update_overheat`: the heat as a percentage, pulsing red once overheated.
    if old.is_none_or(|o| o.heat != new.heat) {
        if let Some((pct, over)) = new.heat.split_once(':') {
            host.root_event(
                "hud_update_overheat",
                &[
                    ("heatPercent", Value::Num(pct.parse().unwrap_or(0.0))),
                    ("overheat", Value::Boolean(over == "1")),
                ],
            );
        }
    }
    if old.is_none_or(|o| o.offhand != new.offhand) {
        // Each slot: { material = <the icon>, ammo = <count> }.
        let mut fields = Vec::new();
        for e in new.offhand.split(';') {
            let Some((w, n)) = e.split_once(':') else {
                continue;
            };
            let Some(OffhandPick { slot, icon }) = offhand_slot(w) else {
                continue;
            };
            let n: f32 = n.trim().parse().unwrap_or(0.0);
            if n <= 0.0 {
                continue;
            }
            let t = hks_t6::value::LuaTable::new_ref();
            t.borrow_mut().set_str("material", host.material(icon));
            t.borrow_mut().set_str("ammo", Value::Num(n));
            fields.push((slot, Value::Table(t)));
        }
        host.root_event("hud_update_offhand", &fields);
    }
    if old.is_none_or(|o| o.slots != new.slots || o.offhand != new.offhand) {
        // actionSlotData[slot] = { material, ammo, aspectRatio }.
        let data = hks_t6::value::LuaTable::new_ref();
        for e in new.slots.split(';') {
            let Some((name, w)) = e.split_once(':') else {
                continue;
            };
            let (Ok(n), Some(icon)) = (name.trim().parse::<f32>(), slot_icon(w)) else {
                continue;
            };
            let ammo: f32 = new
                .offhand
                .split(';')
                .find_map(|o| {
                    o.split_once(':')
                        .filter(|(k, _)| *k == w)
                        .and_then(|(_, c)| c.trim().parse().ok())
                })
                .unwrap_or(0.0);
            let t = hks_t6::value::LuaTable::new_ref();
            t.borrow_mut().set_str("material", host.material(icon));
            t.borrow_mut().set_str("ammo", Value::Num(ammo));
            t.borrow_mut().set_str("aspectRatio", Value::Num(1.0));
            data.borrow_mut().store(Value::Num(n), Value::Table(t));
        }
        host.root_event(
            "hud_update_actionslots",
            &[("actionSlotData", Value::Table(data))],
        );
    }
    if !new.streaks.is_empty() && old.is_none_or(|o| o.streaks != new.streaks || (o.momentum >= 0 && o.momentum != new.momentum)) {
        // bo2mp: the scorestreak column: each pick { slotIndex, material,
        // ammo (one once its cost is in), momentumCost }, then the bar.
        let mut column: Vec<Value> = Vec::new();
        for (i, e) in new.streaks.split(';').enumerate() {
            // "icon:cost" or "icon:cost:held" (held: his streak weapon has
            // one to use).
            let mut parts = e.split(':');
            let (Some(icon), Some(cost)) = (parts.next(), parts.next()) else { continue };
            let cost: i32 = cost.trim().parse().unwrap_or(0);
            let held = parts.next().map(|h| h.trim() == "1");
            let t = hks_t6::value::LuaTable::new_ref();
            t.borrow_mut().set_str("slotIndex", Value::Num((i + 1) as f32));
            t.borrow_mut().set_str("material", host.material(icon));
            let ready = held.unwrap_or(new.momentum >= cost);
            t.borrow_mut().set_str("ammo", Value::Num(if ready { 1.0 } else { 0.0 }));
            t.borrow_mut().set_str("momentumCost", Value::Num(cost as f32));
            column.extend([Value::Table(t)]);
        }
        host.root_event_list("hud_update_rewards", column);
        host.root_event("hud_update_momentum", &[("momentum", Value::Num(new.momentum as f32))]);
    }
    if old.is_none_or(|o| o.icons != new.icons) {
        field_events(host, old.map(|o| o.icons.as_str()), &new.icons);
    }
    if old.is_none_or(|o| o.worldcf != new.worldcf) {
        field_events(host, old.map(|o| o.worldcf.as_str()), &new.worldcf);
    }
}

/// A client field's own event (`perk_juggernaut`, ...) for each that
/// changed from `old` to `new` (`field:value` lists); one gone from the
/// list is back to 0.
fn field_events(host: &mut Host, old: Option<&str>, new: &str) {
    let now = icon_fields(new);
    if let Some(o) = old {
        for (f, _) in icon_fields(o) {
            if !now.iter().any(|(g, _)| *g == f) {
                host.root_event(f, &[("newValue", Value::Num(0.0))]);
            }
        }
    }
    for (f, v) in now {
        let was = old.and_then(|o| {
            icon_fields(o)
                .into_iter()
                .find(|(g, _)| *g == f)
                .map(|(_, x)| x)
        });
        if was != Some(v) {
            host.root_event(
                f,
                &[
                    ("newValue", Value::Num(v)),
                    ("oldValue", Value::Num(was.unwrap_or(0.0))),
                ],
            );
        }
    }
}

/// BO2's button token (`BUTTON_LUI_PRIMARY`, the `CoD.buttonStrings`) as
/// the pad button `frame::bo2_pad_glyph` names.
fn lui_button(token: &str) -> Option<&'static str> {
    Some(match token.strip_prefix("BUTTON_LUI_")? {
        "PRIMARY" => "south",
        "SECONDARY" => "east",
        "ALT1" => "west",
        "ALT2" => "north",
        "SELECT" => "select",
        "START" => "start",
        "SHOULDERL" => "lb",
        "SHOULDERR" => "rb",
        "LEFT_TRIGGER" => "lt",
        "RIGHT_TRIGGER" => "rt",
        "LEFT_STICK" | "LEFT_STICK_UP" => "ls",
        "RIGHT_STICK" => "rs",
        "DPAD_ALL" => "dpad_all",
        "DPAD_UD" => "dpad_ud",
        "DPAD_RL" | "DPAD_LR" => "dpad_rl",
        "DPAD_U" | "DPAD_UP" => "dpad_up",
        "DPAD_D" | "DPAD_DOWN" => "dpad_down",
        "DPAD_L" | "DPAD_LEFT" => "dpad_left",
        "DPAD_R" | "DPAD_RIGHT" => "dpad_right",
        _ => return None,
    })
}

/// `plain`, but with a pad's style BO2's button tokens become its button
/// pictures (the BO2 font draws them).
fn plain_with(text: &str, pad: Option<frame::PromptStyle>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '^' && it.peek().is_some_and(char::is_ascii_digit) {
            it.next();
            continue;
        }
        if c == '^' && it.peek() == Some(&'B') {
            // A button picture: with a pad's style (bo2zm) its button pictures;
            // else the tab arrows are drawn (one private-use letter each, see
            // `ARROW_ICONS`).
            let mut name = String::new();
            for d in it.by_ref() {
                if d == '^' {
                    break;
                }
                name.push(d);
            }
            let token = name.strip_prefix('B').unwrap_or(&name);
            let pad_glyph = pad.and_then(|style| {
                lui_button(token)
                    .and_then(|b| frame::bo2_pad_glyph(b, style))
                    .or_else(|| frame::bo2_cycle_glyph(token))
            });
            let arrow = match name.as_str() {
                "BBUTTON_CYCLE_LEFT" | "BBUTTON_CYCLE_LEFT_ACTIVE" => Some(0),
                "BBUTTON_CYCLE_RIGHT" | "BBUTTON_CYCLE_RIGHT_ACTIVE" => Some(1),
                _ => None,
            };
            if let Some(letter_shape) = pad_glyph {
                out.push_str(letter_shape);
            } else if let Some(i) = arrow {
                out.push(char::from_u32(0xE100 + i).unwrap_or('?'));
            } else if let Some(entry) = name.strip_prefix('B').filter(|n| !n.is_empty() && !n.contains('_')) {
                // A FontIcon entry's own name (the Controls rows' mouse
                // buttons): kept between two private-use letters for
                // `icon_pieces` to look up in BO2's icon set.
                out.push(GLYPH_OPEN);
                out.push_str(entry);
                out.push(GLYPH_CLOSE);
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// bo2mp: each entity container (BO2's objective waypoints) on its point in
/// the world this frame (a player's: his origin in the snapshot, plus its
/// offset), in root units. Off screen, a clamping one sits on the screen's
/// edge toward its point (behind him: toward its side of him); one that
/// does not clamp is not drawn.
fn place_entity_containers(
    host: &mut Host,
    presented: Option<&ShownWorldFrame>,
    cameras: &SceneCameras,
    window: Option<&Window>,
    width: f32,
) {
    let containers = host.entity_containers();
    if containers.is_empty() {
        return;
    }
    let Some(ActiveView { camera, at: transform, .. }) = cameras.active() else { return };
    let (sw, sh) = window.map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    let (eye, forward, right, up) = (transform.translation(), *transform.forward(), *transform.right(), *transform.up());
    // The box a clamped one keeps inside (root units), and its middle.
    let (edge_lo, edge_hi) = (Vec2::new(40.0, 72.0), Vec2::new(width - 40.0, 704.0));
    let mid = Vec2::new(width * 0.5, 360.0);
    for (u, entity, offset, clamp) in containers {
        let base = if entity >= 0 {
            u32::try_from(entity).ok().and_then(|id| presented?.player(sim::ClientId(id))).map(|ps| Vec3::from(ps.origin))
        } else {
            Some(Vec3::ZERO)
        };
        let Some(at) = base.map(|b| b + Vec3::from(offset)) else {
            host.place_entity_container(&u, None, false, 0.0);
            continue;
        };
        let d = at - eye;
        let pos = (d.dot(forward) > 0.0)
            .then(|| camera.world_to_viewport(transform, at).ok())
            .flatten()
            .map(|p| Vec2::new(p.x * width / sw, p.y * 720.0 / sh));
        let (lo, hi) = if clamp { (edge_lo, edge_hi) } else { (Vec2::ZERO, Vec2::new(width, 720.0)) };
        if let Some(p) = pos.filter(|p| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y) {
            host.place_entity_container(&u, Some([p.x, p.y]), false, 0.0);
            continue;
        }
        if !clamp {
            host.place_entity_container(&u, None, false, 0.0);
            continue;
        }
        // The way to it on the screen (y down): from the middle to where it
        // projects; behind him, its side of him and its height.
        let mut dir = match pos {
            Some(p) => p - mid,
            None => Vec2::new(d.dot(right), -d.dot(up)),
        };
        if dir.length_squared() < 1e-6 {
            dir = Vec2::Y;
        }
        let reach = |d: f32, lo: f32, hi: f32, m: f32| {
            if d > 0.0 {
                (hi - m) / d
            } else if d < 0.0 {
                (lo - m) / d
            } else {
                f32::INFINITY
            }
        };
        let t = reach(dir.x, lo.x, hi.x, mid.x).min(reach(dir.y, lo.y, hi.y, mid.y));
        let p = mid + dir * t;
        host.place_entity_container(&u, Some([p.x, p.y]), true, (-dir.y).atan2(dir.x).to_degrees());
    }
}

#[allow(clippy::too_many_arguments)]
fn lui_hud(
    mut cmds: Commands,
    mut slot: NonSendMut<LuiHudSlot>,
    (ui, screen): (Option<Res<assets::T6Ui>>, Res<frame::ShellScreen>),
    fonts: Option<Res<Bo2Fonts>>,
    icons: Option<Res<assets::T6HudIcons>>,
    (mut image_assets, mut globes, globe_nodes): (ResMut<Assets<Image>>, ResMut<Assets<crate::bo2_globe::GlobeMaterial>>, Query<&bevy::ui_render::prelude::MaterialNode<crate::bo2_globe::GlobeMaterial>>),
    seen_world: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    (window, time, world_cam, cameras): (
        MainWindow,
        Res<Time>,
        crate::CompositedWorldCameras,
        SceneCameras,
    ),
    (keys, weapons, actions, cursor, mut latest): (
        Option<Res<frame::OverlayControlSnapshot>>,
        Option<Res<assets::StagedWeapons>>,
        Option<Res<net::PlayerControlSnapshot>>,
        Option<Res<net::LocationCursor>>,
        Local<Option<(i32, i32, Option<i32>, i32)>>,
    ),
    mut io: LuiIo,
    roots: Query<(Entity, &LuiRoot, Option<&bevy::ui::UiTargetCamera>)>,
    mut nodes: Query<(Entity, &mut LuiNode, &mut Node, &mut ZIndex)>,
    (mut image_nodes, mut xforms): (Query<&mut ImageNode>, Query<&mut UiTransform>),
    (children, computed, mut backgrounds, mut scene_mats, mut scene_cards): (
        Query<&Children>,
        Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
        Query<&mut BackgroundColor>,
        ResMut<Assets<crate::lui_scene::SceneUi>>,
        Local<HashMap<&'static str, Handle<crate::lui_scene::SceneUi>>>,
    ),
    (mut pictures, mut logged, mut additive, mut add_names, mut color_add_names): (
        Local<PictureCache>,
        Local<usize>,
        ResMut<Assets<crate::lui_additive::AdditiveUi>>,
        Local<Option<std::collections::HashSet<String>>>,
        Local<std::collections::HashSet<String>>,
    ),
) {
    if !lui_enabled() {
        return;
    }
    let mut hud_t = std::time::Instant::now();
    // bo2mp scope: the pictures kept for additive drawing (see lui_additive).
    if add_names.is_none() && icons.is_some() {
        *add_names = Some(
            icons
                .iter()
                .flat_map(|i| i.0.iter())
                .filter_map(|(k, _)| k.strip_prefix("additive:").map(str::to_owned))
                .collect(),
        );
        // bo2mp vehicle screens: the ones BO2 draws with sw4_2d_color_add.
        *color_add_names = icons
            .iter()
            .flat_map(|i| i.0.iter())
            .filter_map(|(k, _)| k.strip_prefix("coloradd:").map(str::to_owned))
            .collect();
    }
    // bo2mp: the front end has no game to read; its menus run alone.
    let front = io.front.as_deref().cloned();
    // bo2mp: the match he started is loading: the front end's scripts run
    // BO2's own loading screen until the match's HUD takes over.
    let bo2_loading = front.is_none()
        && io.backdrop.is_some()
        && io.setup.is_some()
        && slot.0.as_ref().is_some_and(|h| h.mp.is_some() && !h.match_mp);
    let values = match (seen_world.as_deref(), local.as_deref()) {
        _ if front.is_some() || bo2_loading => Some(Values::default()),
        (Some(p), Some(l)) => read_values(p, l),
        _ => None,
    };
    // The held weapon's names from the weapon table.
    let values = values.map(|mut v| {
        if let Some(w) = weapons.as_deref() {
            // bo2mp: while a scorestreak's item is in his hands (the Hunter
            // Killer, RC-XD, care package marker: inventory type "item"),
            // BO2's HUD keeps the gun he had before it and that gun's ammo
            // (CG_SelectWeaponIndex's latest primary).
            if w.0.summary_for(v.weapon as u32).is_some_and(|f| f.carry_slot_kind == 2) {
                if let Some(l) = *latest {
                    (v.weapon, v.clip, v.clip_left, v.stock) = l;
                }
            } else if v.weapon != 0 {
                *latest = Some((v.weapon, v.clip, v.clip_left, v.stock));
            }
            // bo2mp: a gun with a sight attachment (`svu_mp+acog`) is its
            // base gun's name here.
            let name = w.0.label_for(v.weapon as u32);
            v.weapon_name = name.split('+').next().unwrap_or(name).trim_end_matches("_mp").to_owned();
            v.weapon_key = w.0.title_token_for(v.weapon as u32).unwrap_or_default().to_owned();
            v.weapon_inventory =
                w.0.summary_for(v.weapon as u32).map_or(0, |f| f.carry_slot_kind);
            // The Enchanted Stick (a Special Weapons guest on the Ray
            // Gun's entry) is swung, never fired: melee, no ammo counter.
            v.weapon_type = if v.weapon_name == "mcstick_zm" {
                7
            } else {
                w.0.summary_for(v.weapon as u32).map_or(0, |f| f.t6_weap_type)
            };
            if let Some(f) = w.0.summary_for(v.weapon as u32) {
                let low = |n: i32| !f.motion_clip_exclusive && n >= 0 && f.magazine_capacity > 0 && n.min(999) as f32 <= f.magazine_capacity.min(999) as f32 * f.scarce_rounds_alert_level;
                v.low_clip = low(v.clip);
                v.low_clip_left = v.clip_left.is_some_and(low);
            }
            // bo2mp: its fire mode's picture (FULL-AUTO, SINGLE, BURST, ..).
            v.fire_material = w.0.summary_for(v.weapon as u32).map_or("", |f| fire_material(f.t6_fire_type, f.manual_cycle));
            // bo2mp: looking through a scope (its overlay is up).
            if let (Some(m), Some(pl)) = (
                v.mp.as_mut(),
                seen_world.as_deref().zip(local.as_deref()).and_then(|(p, l)| p.player(l.0)),
            ) {
                m.scoped = pl.weapon_raise_fraction >= 1.0 && w.0.scope_is_iris(pl.weapon);
            }
        }
        v
    });
    let mut spent = [lap(&mut hud_t), 0.0, 0.0, 0.0, 0.0];
    // A Black Ops II game runs while its HUD has values: the pad's aim
    // assist is BO2's there.
    frame::set_bo2_game(values.is_some());
    // bo2zm M4: out of a game, on the menu screen, a game started on BO2's
    // own front end shows it (its scripts and pictures read at the start).
    // Its backdrop map has a world to read before he joins: the menus stay.
    let zm_front = (values.is_none() || frame::bo2_frontend_waits_on_menu())
        && frame::bo2_frontend().is_some()
        && matches!(
            *screen,
            frame::ShellScreen::MainMenu | frame::ShellScreen::Loading
        );
    // Trunk keeps the front end's scripts and pictures in the same resources.
    let ui: Option<&assets::T6Ui> = ui.as_deref();
    let icons: Option<&assets::T6HudIcons> = icons.as_deref();
    let values = if zm_front {
        Some(Values::default())
    } else {
        values
    };
    let Some((ui, values)) = ui.zip(values) else {
        for (e, ..) in &roots {
            cmds.entity(e).try_despawn();
        }
        let idle = frame::LuiMenus::default();
        if *io.menus != idle {
            *io.menus = idle;
        }
        if io.paused.0 && !io.cheat.is_up {
            io.paused.0 = false;
        }
        if io.menu_dims.0 != 0 {
            io.menu_dims.0 = 0;
        }
        // Test aid: a match's `shot:` steps keep their time while its HUD is
        // hidden (the outcome, before the scoreboard comes back).
        if let Some(hud) = slot.0.as_mut()
            && hud.match_mp
            && !MATCH_STEPS_DONE.load(std::sync::atomic::Ordering::Relaxed)
            && let Ok(plan) = std::env::var("BO2MP_MATCH_STEPS")
        {
            let now = time.elapsed_secs_f64() * 1000.0 - hud.opened_at;
            let count = plan.split(',').count();
            if let Some((button, at)) = plan.split(',').nth(hud.test_step).and_then(|s| s.split_once('@'))
                && hud.test_step >= 1
                && button.trim().starts_with("shot:")
                && at.trim().parse::<f64>().is_ok_and(|t| now >= t)
            {
                hud.test_step += 1;
                if hud.test_step >= count {
                    MATCH_STEPS_DONE.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                diag::info!(Ui, "menus: test key {button} (hud off)");
                let text = format!("screenshot bo2mp/{}", button.trim().trim_start_matches("shot:"));
                io.exec.write(frame::UiExecCommand { text });
            }
        }
        return;
    };
    // (Re)build when the map's scripts arrive.
    // (bo2mp: and when the front end hands over to a match.)
    let launch = io.launch_id.as_deref();
    let match_spec = values.mp.as_ref().filter(|_| front.is_none()).map(|m| MatchSpec {
        gametype: m.gametype.clone(),
        map: launch.map(|i| i.zone.rsplit(':').next().unwrap_or("").to_owned()).unwrap_or_default(),
        stats: crate::lui_frontend::stats_path(launch),
    });
    if !bo2_loading && slot.0.as_ref().is_none_or(|h| {
        h.built_from != ui.scripts.len()
            || (h.mp.is_some() && !h.match_mp) != front.is_some()
            // (a match HUD stays one: a moment without its values does
            // not rebuild it as the zombies HUD)
            || (match_spec.is_some() && !h.match_mp)
            || h.frontend != zm_front
    }) && !ui.scripts.is_empty()
    {
        let t = std::time::Instant::now();
        // The front end hands over to the game: keep its lobby.
        let slot = &mut *slot;
        if !zm_front && let Some(old) = slot.0.as_mut().filter(|h| h.frontend) {
            let filter = old
                .host
                .field("CoD", "PlaylistCategoryFilter")
                .as_str()
                .map(str::to_owned);
            let v = old.host.values.borrow();
            let state = FrontState {
                playlist: v.playlist,
                game_modes: v.game_modes.clone(),
                session_modes: v.session_modes.clone(),
                filter,
            };
            drop(v);
            slot.1 = Some(state);
        }
        let mut hud = build(ui, front.as_ref(), match_spec.as_ref(), zm_front);
        // Back from a game: the lobby it started from.
        if zm_front && let Some(st) = slot.1.take() {
            {
                let mut v = hud.host.values.borrow_mut();
                v.playlist = st.playlist;
                v.game_modes = st.game_modes;
                v.session_modes = st.session_modes;
            }
            if let Some(f) = &st.filter {
                hud.host
                    .set_global_field("CoD", "PlaylistCategoryFilter", Value::str(f));
            }
            let modes = hud.host.values.borrow().game_modes.clone();
            hud.return_menu = Some(if modes.contains(&0.0) {
                "PublicGameLobby"
            } else if modes.contains(&1.0) {
                "PrivateOnlineGameLobby"
            } else {
                "MainLobby"
            });
        }
        diag::info!(
            Ui,
            "menus: {} scripts, base and {} in {:.0} ms; {} script errors; modules not found {:?}",
            ui.scripts.len(),
            if zm_front {
                "T6.Main (front end)"
            } else {
                "T6.HUD"
            },
            t.elapsed().as_secs_f64() * 1000.0,
            hud.host.errors.len(),
            hud.host.missing.borrow()
        );
        slot.0 = Some(hud);
        pictures.forget_all();
        for (e, ..) in &roots {
            cmds.entity(e).try_despawn();
        }
        // (bo2mp: the old root goes this frame; draw from the next.)
        return;
    }
    let Some(hud) = slot.0.as_mut() else {
        return;
    };
    // Font metrics for the scripts' text measure, once the fonts load.
    if hud.metrics.borrow().is_empty()
        && let Some(f) = fonts.as_deref()
    {
        *hud.metrics.borrow_mut() = f.advances();
    }
    // The keys his commands are bound to (the d-pad's key prompts).
    if let Some(k) = keys.as_deref() {
        let mut v = hud.host.values.borrow_mut();
        if v.binds.len() != k.binding_keys_all.len()
            || k.binding_keys_all
                .iter()
                .any(|(c, keys)| v.binds.get(c) != Some(keys))
        {
            v.binds = k
                .binding_keys_all
                .iter()
                .map(|(c, keys)| (c.clone(), keys.clone()))
                .collect();
        }
    }
    // BO2MP_HEADLESS=1: no window. The scripts still run, on BO2's own
    // 1280x720 screen with no pointer, so their errors and engine calls
    // still reach the log; nothing is drawn.
    let headless = frame::Headless::requested();
    let (ww, wh, pointer) = match (window.shown(), headless) {
        (Some(win), _) => (win.width(), win.height(), win.cursor_position()),
        (None, true) => (1280.0, 720.0, None),
        (None, false) => return,
    };
    let aspect = ww / wh.max(1.0);
    let scale = wh / 720.0;
    let moved = (hud.host_aspect - aspect).abs() > 1e-3;
    if moved {
        hud.host_aspect = aspect;
        hud.host.root(aspect);
    }
    if !hud.opened {
        hud.opened = true;
        hud.opened_at = time.elapsed_secs_f64() * 1000.0;
        if front.is_some() {
            crate::lui_frontend::open(&mut hud.host, hud.mp.as_ref(), io.lobby_return.as_deref());
            cmds.remove_resource::<crate::lui_frontend::LobbyReturn>();
        } else if hud.match_mp {
            // (BO2's HUD builds its scoreboard now, column headers and team
            // blocks: the game type's columns and settings go in first.)
            if let (Some(m), Some(state)) = (values.mp.as_ref(), hud.mp.clone()) {
                crate::lui_frontend::match_prime(&mut hud.host, &state, m);
            }
            crate::lui_frontend::match_open(&mut hud.host);
        } else {
            // Test aid: BO2ZM_LUI_FIRST_MENU=<menu> opens that menu instead of
            // the HUD (the front end: `main`).
            let first = std::env::var("BO2ZM_LUI_FIRST_MENU")
                .unwrap_or_else(|_| if hud.frontend { "main" } else { "HUD" }.to_owned());
            hud.host.open_menu(&first);
            if hud.frontend {
                // The engine's `open_menu`: BO2's black cover (`main`) opens
                // the menu it names on it (after a game, its lobby).
                let menu = hud.return_menu.unwrap_or("MainMenu");
                hud.host.root_event(
                    "open_menu",
                    &[
                        ("controller", Value::Num(0.0)),
                        ("menuName", Value::str(menu)),
                    ],
                );
            } else {
                hud.host
                    .root_event("first_snapshot", &[("controller", Value::Num(0.0))]);
                hud.host.root_event("hud_update_refresh", &[]);
            }
        }
    }
    // The map loads after Start Match: BO2's loading screen over the menus.
    if hud.frontend && *screen == frame::ShellScreen::Loading && !hud.loading_open {
        hud.loading_open = true;
        // The backdrop map behind the Zombies menus is no match: no map's
        // name or place on its loading screen (it named Nuketown's).
        if frame::bo2_frontend_waits_on_menu() {
            hks_t6::host::clear_loading_dvars(&mut hud.host.values.borrow_mut());
        } else {
            hks_t6::host::set_loading_dvars(&mut hud.host.values.borrow_mut());
        }
        hud.host.open_menu("Loading");
        // The engine starts it once its screen is up: the map name, place,
        // game mode and picture fade in one after another
        // (ui_mp_t6_hud_loading.lua). BO2 loads on another thread so its
        // screen keeps drawing; ours stops drawing once the map installs
        // (about 2.5 s in, measured load146), which froze the screen before
        // the picture's turn. So all four fades start together and the
        // whole screen is up within a second.
        hud.host.root_event("start_loading", &[]);
        for event in ["fade_in_map_location", "fade_in_gametype", "fade_in_map_image"] {
            hud.host.root_event(event, &[]);
        }
    }
    // (The loading screen can arrive a frame after the menu opens.) The
    // backdrop load needs no hold: its screen has no picture to fade in, and
    // the hold kept it up 4.5 s past the load (load 10-29: held to 21.1 s).
    if hud.frontend
        && hud.loading_open
        && !frame::bo2_frontend_waits_on_menu()
        && *screen == frame::ShellScreen::Loading
        && let Some(loading) = io.backdrop.as_deref_mut()
    {
        // TODO(rw-zm): `hold_once` comes with the loading_screen delta (trunk moved it to assets/src/load_overlay.rs)
        loading.hold_once(std::time::Duration::from_millis(4500));
    }
    // The front end's backdrop map is in: BO2's loading screen goes away and
    // leaves the menus (it stayed up over them, its picture hiding them).
    if front_end_backdrop_loaded(hud.frontend, hud.loading_open, *screen) {
        hud.loading_open = false;
        let closed = hud.host.close_menu("Menu.Loading");
        diag::info!(Ui, "menus: backdrop loaded, loading screen closed: {closed}");
    }
    if bo2_loading && !hud.loading_open {
        hud.loading_open = true;
        if let Some(setup) = io.setup.as_deref() {
            crate::lui_frontend::open_loading(&mut hud.host, setup);
        }
    }
    if hud.frontend {
        hud.host.solo_lobby_start();
    }
    if !hud.frontend {
        if front.is_none() && !bo2_loading {
            send_changes(&mut hud.host, hud.sent.as_ref(), &values);
        }
    } else if hud.host.clock_millis() - hud.lobby_update_at > 1000.0
        && let sig = lobby_sig(&hud.host)
        && (sig != hud.lobby_sig || std::mem::take(&mut hud.lobby_again))
    {
        // The engine's lobby updates (it sends them as the party changes,
        // not on a clock: each one rebuilds the player rows, restarting
        // their "Searching for player" dots): the lobbies refresh their
        // titles and Start Match from them.
        hud.lobby_update_at = hud.host.clock_millis();
        if sig != hud.lobby_sig {
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                let v = hud.host.values.borrow();
                let game: Vec<_> = ["ui_mapname", "ui_zm_mapstartlocation", "ui_gametype"]
                    .iter()
                    .map(|k| hks_t6::host::dvar_get(&v.dvars, k))
                    .collect();
                diag::info!(Ui, "menus: lobby changed {game:?}");
            }
            hud.lobby_again = true;
            hud.lobby_sig = sig;
        }
        // Their members: him (the party panel's card).
        let members = hks_t6::host::lobby_members(&hud.host.values.borrow());
        hud.host.root_event(
            "gamelobby_update",
            &[
                ("controller", Value::Num(0.0)),
                ("members", members.clone()),
            ],
        );
        hud.host.root_event(
            "partylobby_update",
            &[("controller", Value::Num(0.0)), ("members", members)],
        );
    }
    // bo2mp: BO2's scoreboard open while his scores key (+scores, Tab) is
    // held (the engine opens it; its scripts fill it).
    if hud.match_mp {
        // (And up for good once the game has ended and his HUD is back, the
        // intermission: setmatchflag "game_ended".)
        let raw_ended = seen_world
            .as_deref()
            .zip(local.as_deref())
            .and_then(|(world, me)| world.snapshot().map(|s| s.meta.script_dvars(me.0).string("bo2mp_game_ended") == Some("1")))
            .unwrap_or(false);
        hud.ended_frames = if raw_ended { hud.ended_frames + 1 } else { 0 };
        // (No in-game menu once the host has ended the game: the match
        // flag `disableIngameMenu`, `setmatchflag` -> `bo2mp_disable_ingame_menu`.)
        hud.ingame_menu_off = seen_world
            .as_deref()
            .zip(local.as_deref())
            .and_then(|(world, me)| world.snapshot().map(|s| s.meta.script_dvars(me.0).string("bo2mp_disable_ingame_menu") == Some("1")))
            .unwrap_or(false);
        // (his board waits a moment for the HUD to settle after the script
        // gives it back; the rest of the HUD goes at once)
        let game_ended = raw_ended && hud.ended_frames > 12;
        if let Some(m) = hud.mp.as_ref() {
            m.borrow_mut().game_ended = game_ended;
        }
        // (bo2mp: the final killcam plays before the end scoreboard: BO2
        // shows only the killcam's widget over it, the board comes after.)
        let replaying_final_death = seen_world
            .as_deref()
            .zip(local.as_deref())
            .and_then(|(p, l)| {
                p.snapshot().map(|s| s.meta.script_dvars(l.0).string("bo2mp_killcam").is_some_and(|k| k.starts_with("1|") || k.starts_with("2|") || k.starts_with("4|")))
            })
            .unwrap_or(false);
        hud.calm_frames = if game_ended && !replaying_final_death { hud.calm_frames + 1 } else { 0 };
        let game_ended_board = hud.calm_frames > calm_limit();
        if game_ended_board && !hud.board_open {
            diag::info!(Ui, "match hud: match over, scores up");
        }
        if !game_ended_board {
            hud.end_rows_logged = false;
        }
        let want_board = game_ended_board || actions.as_deref().is_some_and(|a| a.client.pressed_key_latch.scores.active);
        if want_board != hud.board_open {
            hud.board_open = want_board;
            let event = if want_board { "open_scoreboard_menu" } else { "close_scoreboard_menu" };
            hud.host.root_event(event, &[]);
            // (bo2mp: the end scoreboard has no "Killed By" card under it:
            // real/62 shows the death view bare.)
            if want_board
                && game_ended_board
                && let Some(bit) = hud.host.field("CoD", "BIT_IN_KILLCAM").as_num().map(|n| n as i32)
            {
                hud.host.values.borrow_mut().bits.remove(&bit);
                hud.host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
            }
        }
        // (BO2's HUD parts key off the scoreboard's bit: minimap, ammo,
        // crest and the rest go while it is up; the game's end has its own.)
        // (The match goes back to the front end 5 s after the board is first
        // up: BO2's `wait 5.0` before `exitlevel`.)
        if let Some(shown) = io.end_board.as_deref_mut() {
            if game_ended_board && hud.board_open {
                shown.0.get_or_insert_with(std::time::Instant::now);
            } else {
                shown.0 = None;
            }
        }
        let bits = (want_board || raw_ended, raw_ended);
        if bits != hud.scoreboard_bits {
            hud.scoreboard_bits = bits;
            for (name, on) in [("BIT_SCOREBOARD_OPEN", bits.0), ("BIT_GAME_ENDED", bits.1)] {
                let Some(bit) = hud.host.field("CoD", name).as_num().map(|n| n as i32) else { continue };
                {
                    let mut v = hud.host.values.borrow_mut();
                    if on {
                        v.bits.insert(bit);
                    } else {
                        v.bits.remove(&bit);
                    }
                }
                hud.host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
            }
        }
    }
    // bo2mp: the weapons' kill icons for the kill feed, once.
    if let (Some(state), Some(icons)) = (hud.mp.as_ref().filter(|_| hud.match_mp), icons.as_deref())
        && !state.borrow().kill_icons_loaded
    {
        let mut m = state.borrow_mut();
        for (name, _) in &icons.0 {
            let Some(rest) = name.strip_prefix("killicon:") else { continue };
            let Some((weapon, ratio)) = rest.rsplit_once(':') else { continue };
            let wide = match ratio {
                "1" => 2.0,
                "2" => 4.0,
                _ => 1.0,
            };
            m.kill_icons.insert(weapon.to_owned(), (name.clone(), wide));
        }
        m.kill_icons_loaded = true;
    }
    if let Some(m) = values.mp.as_ref().filter(|_| hud.match_mp) {
        let old = hud.sent.as_ref().and_then(|v| v.mp.as_ref());
        if let Some(state) = hud.mp.clone() {
            crate::lui_frontend::match_events(&mut hud.host, &mut hud.seen, old, m, &state);
        }
    }
    // Test aid: BO2ZM_LUI_OPEN_MENU=<menu>: a second after the first menu
    // opens, the engine's `open_menu` event asks for that menu (BO2's front
    // end: its black cover opens `MainMenu` on it).
    if let Ok(menu) = std::env::var("BO2ZM_LUI_OPEN_MENU")
        && hud.host.clock_millis() - hud.opened_at > 1000.0
        && !hud.open_menu_sent
    {
        hud.open_menu_sent = true;
        hud.host.root_event(
            "open_menu",
            &[
                ("controller", Value::Num(0.0)),
                ("menuName", Value::str(&menu)),
            ],
        );
    }
    // Test aid: BO2ZM_LUI_ASKED=1 logs, 8 s after the menus open, every
    // engine field the scripts asked for that nothing answers (counted).
    // (BO2ZM_LUI_ASKED=<ms>: that long after instead.)
    if let Some(after) = std::env::var("BO2ZM_LUI_ASKED").ok().map(|v| {
        v.parse::<f64>()
            .ok()
            .filter(|ms| *ms > 1.0)
            .unwrap_or(8000.0)
    }) && hud.host.clock_millis() - hud.opened_at > after
        && !hud.asked_logged
    {
        hud.asked_logged = true;
        let asked = hud.host.asked.borrow();
        let tally: Vec<String> = asked.iter().map(|(k, n)| format!("{k} x{n}")).collect();
        diag::info!(Ui, "menus: script asked for {} things: {}", tally.len(), tally.join(", "));
        diag::info!(
            Ui,
            "menus: widget setups not found: {:?}",
            hks_t6::lui::missing_methods()
        );
    }
    // BO2's scoreboard: open while Tab (+scores) is held and through the
    // game-over shot (the engine opens it; its scripts refresh it).
    let scoreboard_held_open = actions
        .as_deref()
        .is_some_and(|a| a.client.pressed_key_latch.scores.active);
    let want_board = !hud.frontend && !hud.match_mp && (scoreboard_held_open || values.game_over);
    if want_board != hud.board_open {
        hud.board_open = want_board;
        let event = if want_board {
            "open_scoreboard_menu"
        } else {
            "close_scoreboard_menu"
        };
        hud.host
            .root_event(event, &[("controller", Value::Num(0.0))]);
    }
    let hidden = values.hud_off;
    hud.sent = Some(values);
    spent[1] = lap(&mut hud_t);
    // BO2ZM_LUI_PAUSE_AT=<ms after the HUD opened>[,<button>@<ms>...]: open
    // the pause menu then, and press pad buttons later (test aid). `-`
    // first: no pause menu (the front end's menus are up already).
    // bo2mp: a multiplayer match runs its own steps (BO2MP_MATCH_STEPS,
    // same form), so the front end's do not replay in it; each plan runs
    // once per process (coming back to the front end does not restart it).
    // BO2MP_RETURN_STEPS: the front end's once back from a match.
    let back = !hud.match_mp && FRONT_BUILDS.load(std::sync::atomic::Ordering::Relaxed) > 1;
    let (plan_var, plan_done) = if hud.match_mp {
        ("BO2MP_MATCH_STEPS", &MATCH_STEPS_DONE)
    } else if back {
        ("BO2MP_RETURN_STEPS", &RETURN_STEPS_DONE)
    } else {
        ("BO2ZM_LUI_PAUSE_AT", &FRONT_STEPS_DONE)
    };
    if let Ok(plan) = std::env::var(plan_var)
        && !plan_done.load(std::sync::atomic::Ordering::Relaxed)
    {
        let now = hud.host.clock_millis() - hud.opened_at;
        let count = plan.split(',').count();
        let mut steps = plan.split(',');
        let first = steps.next().map_or("", str::trim);
        let start_at = if first == "-" {
            Some(0.0)
        } else {
            first.parse::<f64>().ok()
        };
        if let Some(at) = start_at
            && now >= at
            && hud.test_step == 0
        {
            hud.test_step = 1;
            // (The front end's menus are open already; a match's test
            // opens menus by its steps.)
            if first != "-" && front.is_none() && !hud.match_mp {
                hud.host.root_event("open_ingame_menu", &[("menuName", Value::str("class"))]);
            }
        }
        for (i, step) in steps.enumerate() {
            let Some((button, at)) = step.split_once('@') else {
                continue;
            };
            if hud.test_step == i + 1 && at.trim().parse::<f64>().is_ok_and(|t| now >= t) {
                hud.test_step = i + 2;
                // The plan's last step: done for this process (START MATCH
                // leaves the front end before another frame could see it).
                if hud.test_step >= count {
                    plan_done.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                diag::info!(Ui, "menus: test key {button}");
                // `click:x:y` = the mouse there, pressed and let go (root
                // units); else a pad button.
                let mut c = button.trim().split(':');
                let kind = c.next();
                if kind == Some("menu") {
                    // `menu:<name>` = the engine opens that menu (Esc's
                    // `class`); `event:<name>` = that root event.
                    let name = c.next().unwrap_or("class");
                    hud.host.root_event("open_ingame_menu", &[("menuName", Value::str(name))]);
                } else if kind == Some("loading") {
                    // `loading` = BO2's loading screen for the lobby's map
                    // and game type, over the front end (its load is too
                    // quick to photograph).
                    let setup = {
                        let v = hud.host.values.borrow();
                        let d = |k: &str, x: &str| v.dvars.get(k).cloned().unwrap_or_else(|| x.to_owned());
                        session::Bo2mpMatchSetup {
                            map: d("ui_mapname", "mp_nuketown_2020"),
                            gametype: d("ui_gametype", "tdm"),
                            ..Default::default()
                        }
                    };
                    crate::lui_frontend::open_loading(&mut hud.host, &setup);
                } else if kind == Some("event") {
                    hud.host.root_event(c.next().unwrap_or(""), &[]);
                } else if kind == Some("shot") {
                    // `shot:<name>` = a screenshot then (bo2mp/<name>).
                    let text = format!("screenshot bo2mp/{}", c.next().unwrap_or("step"));
                    io.exec.write(frame::UiExecCommand { text });
                } else if kind == Some("key") {
                    // `key:TAB` = that key's shortcut (a prompt's key).
                    key_shortcut(&mut hud.host, c.next().unwrap_or(""));
                } else if kind == Some("notify") {
                    // `notify:<event>:<arg>:...` = a LUI notify the server
                    // sends (`luinotifyevent`): the event with its data.
                    let name = c.next().unwrap_or("").to_owned();
                    let data = hks_t6::value::LuaTable::new_ref();
                    for (i, a) in c.enumerate() {
                        let v = a.parse::<f32>().map_or_else(|_| Value::str(a), Value::Num);
                        data.borrow_mut().store(Value::Num((i + 1) as f32), v);
                    }
                    hud.host.root_event(&name, &[("data", Value::Table(data))]);
                } else if kind == Some("move") {
                    // `move:x:y` = the mouse there, no click.
                    let x: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let y: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    hud.host.mouse("mousemove", x, y, "");
                } else if kind == Some("click") {
                    let x: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let y: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    hud.host.mouse("mousemove", x, y, "");
                    hud.host.mouse("mousedown", x, y, "left");
                    hud.host.mouse("mouseup", x, y, "left");
                } else if let Some(b) = button.trim().strip_suffix('+') {
                    // `right+` presses and holds, `right-` lets go.
                    hud.host.button(b, true);
                } else if let Some(b) = button.trim().strip_suffix('-') {
                    hud.host.button(b, false);
                } else {
                    hud.host.button(button.trim(), true);
                    hud.host.button(button.trim(), false);
                }
            }
        }
    }
    // bo2mp: CAMPAIGN's prompt closed for him a few frames on.
    if hud.back_in > 0 {
        hud.back_in -= 1;
        // (BO2's confirm prompts are all `CoD.Menu.NewSmallPopup("SetDefaultPopup")`.)
        if hud.back_in == 0 && !hud.host.dismiss_popup("Menu.SetDefaultPopup") {
            diag::warn!(Ui, "front end: campaign prompt was already shut");
        }
    }
    // His keys and mouse, then what the menus asked of the engine. While
    // a bind row waits for a key, the key is the capture's; when it is
    // done the rows show the new keys (`key_bound`).
    let console_open = keys.as_deref().is_some_and(|k| k.console_open);
    let capturing = io.capture.command.is_some() || io.capture.consumed_input;
    if hud.binding && !capturing {
        hud.binding = false;
        hud.host.root_event("key_bound", &[]);
    }
    if !capturing {
        menu_input(hud, &mut io, console_open, pointer, scale);
    }
    // (Presses the menus did not take are not theirs a frame later.)
    io.key_events.clear();
    engine_calls(hud, &mut io, local.as_deref());
    if let (Some(f), Some(mp)) = (front.as_ref(), hud.mp.clone()) {
        let launch = io.launch_id.as_deref();
        if crate::lui_frontend::tick(&mut hud.host, &mp, f, launch, &mut cmds, io.swap.as_deref_mut()) {
            io.music_stop.write(frame::MenuSongHalt);
        }
    }
    // bo2zm: the relay's Public Match search and the party's own lobby.
    public_match(hud, &mut io);
    private_party(hud, &mut io);
    let monitors = io.monitors.iter().count().max(1);
    {
        let mut v = hud.host.values.borrow_mut();
        v.dvars.insert("r_monitorCount".to_owned(), monitors.to_string());
        v.dvars.entry("r_monitor".to_owned()).or_insert_with(|| "0".to_owned());
    }
    settings_bridge(hud, &mut io.settings);
    // bo2zm: his last input: BO2's prompts follow it (`Engine.LastInput_Gamepad`,
    // `input_source_changed`: source 0 the pad, 1 keys and mouse).
    let pad_style = io.devices.as_deref().filter(|d| d.pad_prompts).map(|d| d.style);
    let gamepad = pad_style.is_some();
    if hud.host.values.borrow().gamepad != gamepad {
        hud.host.values.borrow_mut().gamepad = gamepad;
        let source = if gamepad { 0.0 } else { 1.0 };
        hud.host.root_event(
            "input_source_changed",
            &[("controller", Value::Num(0.0)), ("source", Value::Num(source))],
        );
    }
    let open_names = hud.host.open_menus();
    // bo2zm: the solo cheat menu holds the pointer as a menu does.
    let open = !open_names.is_empty() || io.cheat.is_up;
    // bo2mp: the front end holds his keyboard and mouse (a free, shown
    // pointer) though its main menu is no `Menu.*` child of the root.
    let front_end = io.front.is_some() && !hud.match_mp;
    // (bo2mp: the loading screen holds nothing and pauses nothing: the
    // front end's menus under it are not open for the match.)
    let open = open && !bo2_loading;
    let state = frame::LuiMenus { active: true, open: open || front_end };
    if *io.menus != state {
        if io.menus.open != state.open {
            diag::info!(Ui, "menus: keys go to menus {} {open_names:?}", state.open);
        }
        *io.menus = state;
    }
    // A solo game pauses under its menus (Black Ops II's solo pause).
    // (bo2mp: a multiplayer match never pauses.)
    let pause = open
        && !hud.frontend
        && io.role.as_deref() == Some(&frame::ProcessDuty::Listen)
        && !hud.match_mp;

    if io.paused.0 != pause {
        io.paused.0 = pause;
        diag::info!(
            Ui,
            "menus: match {}",
            if pause { "paused" } else { "resumed" }
        );
    }
    spent[2] = lap(&mut hud_t);
    hud.host.frame(time.elapsed_secs_f64() * 1000.0);
    if hud.host.errors.len() > *logged {
        for e in &hud.host.errors[*logged..] {
            diag::warn!(Ui, "menus: script failed: {e}");
        }
        *logged = hud.host.errors.len();
    }
    // Each engine field a script asked that the host does not answer
    // (once per name): a gap the scripts read as nil.
    {
        static TOLD: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
        if let Ok(mut told) = TOLD.lock() {
            for k in hud.host.asked.borrow().keys() {
                if !told.contains(k) {
                    diag::info!(Ui, "menus: no answer for game field {k}");
                    told.insert(k.clone());
                }
            }
        }
    }
    spent[3] = lap(&mut hud_t);
    if headless {
        hud_spent(spent, [0.0; 3], 0, 0);
        return;
    }
    let mut sub = [0.0f64; 3];
    // bo2mp: the objectives' waypoints on their points in the world.
    if hud.match_mp {
        let width = hud.host_width();
        place_entity_containers(&mut hud.host, seen_world.as_deref(), &cameras, window.shown(), width);
    }
    // BO2ZM_LUI_DUMP=1: every element the scripts have, each time their
    // number changes (debugging aid).
    if std::env::var_os("BO2ZM_LUI_DUMP").is_some() {
        let all = hud.host.drawn();
        if all.len() != hud.dumped {
            hud.dumped = all.len();
            for d in &all {
                diag::info!(
                    Ui,
                    "menus: piece {}: {} {:?} box {:?} alpha {:.2} rgb {:?} words {:?} align {} font {:?} behind {} blur {} anchors {:?}",
                    d.id,
                    d.element_kind,
                    d.material,
                    d.rect,
                    d.opacity,
                    d.rgb,
                    d.text,
                    d.alignment,
                    d.font,
                    d.behind,
                    d.blur,
                    d.anchors
                );
            }
        }
    }
    // bo2mp: his minimap this frame (the engine draws BO2's compass).
    let clock_millis = hud.host.clock_millis();
    let mut shots = std::mem::take(&mut hud.shots);
    let mut sweep = std::mem::take(&mut hud.sweep);
    let mut scan_sound = false;
    let minimap = (hud.match_mp)
        .then(|| {
            let (p, l) = (seen_world.as_deref()?, local.as_deref()?);
            let m = hud.sent.as_ref()?.mp.as_ref()?;
            let pl = p.player(l.0)?;
            let me = ([pl.origin[0], pl.origin[1]], pl.look_angles[1]);
            let my_team = m.team.clone();
            // BO2 shows an enemy only while his radar is up, or for a moment
            // after he fires; a jammed map shows none. A UAV pings each
            // enemy as its line passes him; the satellite (Orbital VSAT)
            // shows them all the time. In a free-for-all (fourth value)
            // everyone else is an enemy: the team each player is filed
            // under there is no side. The fifth value counts the moments
            // the scripts showed him all his enemies (Lightning Strike).
            let radar: Vec<i32> = m.radar.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            let uav = radar.first().copied().unwrap_or(0);
            let sat = radar.get(1).copied().unwrap_or(0);
            let live = sat > 0;
            let jammed = radar.get(2).is_some_and(|v| *v > 0);
            let ffa = radar.get(3).is_some_and(|v| *v > 0);
            let pulse = radar.get(4).copied().unwrap_or(0);
            let killcam = p
                .snapshot()
                .is_some_and(|s| s.meta.script_dvars(l.0).string("bo2mp_killcam").is_some_and(|k| k.starts_with("1|") || k.starts_with("2|") || k.starts_with("4|")));
            let friends: Vec<([f32; 2], f32)> = m
                .teams
                .split(',')
                .filter_map(|e| e.split_once(':'))
                .filter(|(c, t)| !ffa && *t == my_team && c.parse::<u32>().ok() != Some(l.0.0))
                .filter_map(|(c, _)| p.player(sim::ClientId(c.parse().ok()?)))
                .map(|pl| ([pl.origin[0], pl.origin[1]], pl.look_angles[1]))
                .collect();
            let mut view = crate::lui_frontend::MinimapView::of(m, me, friends, Vec::new())?;
            // The line moves while a UAV or the satellite is up (a sweep a
            // `compassRadarUpdateTime`, the fast time in mode 2) and starts
            // over from the map's west edge when both are down.
            let dt = sweep.last_ms.map_or(0.0, |t| (clock_millis - t).max(0.0));
            sweep.last_ms = Some(clock_millis);
            let old = sweep.progress;
            if uav <= 0 && sat <= 0 {
                sweep.progress = 0.0;
            } else {
                let mode = if sat > 0 { sat } else { uav };
                let period = if mode == 2 { RADAR_SWEEP_FAST_MS } else { RADAR_SWEEP_MS };
                sweep.progress = (old + (dt / period) as f32).rem_euclid(1.0);
            }
            let wrapped = sweep.progress < old;
            let (was, line) = (view.line_at(old), view.line_at(sweep.progress));
            // The UAV's pings: an enemy the line passed this frame, or who
            // walked across it; none in a killcam. Each wrap of the line
            // plays the sweep sound (`mpl_minimap_scan`) instead.
            let sweeping = uav > 0 && !jammed && !killcam;
            if uav > 0 && wrapped {
                scan_sound = true;
            }
            let pulsed = pulse > sweep.pulse;
            sweep.pulse = pulse;
            // His Sensor Grenades: the enemies a blast tracks show the whole
            // time; each laser sweep pings the one it reaches.
            let (tracked, sensed) = m.sensor.split_once(' ').unwrap_or(("-", "-"));
            let tracked: std::collections::HashSet<u32> = tracked.split(',').filter_map(|c| c.parse().ok()).collect();
            let sensed: std::collections::HashMap<u32, u32> = sensed
                .split(',')
                .filter_map(|e| e.split_once(':'))
                .filter_map(|(c, n)| Some((c.parse().ok()?, n.parse().ok()?)))
                .collect();
            sweep.sensor.retain(|id, _| sensed.contains_key(id));
            let enemies: Vec<([f32; 2], f32, bool, f32)> = m
                .teams
                .split(',')
                .filter_map(|e| e.split_once(':'))
                .filter(|(c, t)| (ffa || *t != my_team) && matches!(*t, "allies" | "axis") && c.parse::<u32>().ok() != Some(l.0.0))
                .filter_map(|(c, _)| {
                    let id: u32 = c.parse().ok()?;
                    let es = p.living_participant(sim::ClientId(id))?;
                    // A shot: his weapon is in the firing state (WEAPON_FIRING),
                    // unless it is suppressed (BO2: a Suppressor's shots show no
                    // ping).
                    let seen = shots.entry(id).or_insert(f64::NEG_INFINITY);
                    let silenced = weapons
                        .as_deref()
                        .and_then(|w| w.0.summary_for(es.weapon as u32))
                        .is_some_and(|f| f.silenced);
                    // BO2's firing states: a shot, continuous fire, either dual-wield hand.
                    if matches!(es.weapon_state_t6, 6 | 39..=41 | 53 | 54) && !silenced {
                        *seen = clock_millis;
                    }
                    let firing = clock_millis - *seen < 2000.0;
                    let at = ([es.origin[0], es.origin[1]], es.look_angles[1]);
                    let before = sweep.last_pos.insert(id, at.0);
                    let passed = !wrapped && view.behind_line(was, at.0) != view.behind_line(line, at.0);
                    let crossed = before.is_some_and(|b| view.behind_line(line, b) != view.behind_line(line, at.0));
                    let swept = sensed.get(&id).is_some_and(|&n| sweep.sensor.insert(id, n).is_none_or(|was| n > was));
                    if (sweeping && (passed || crossed)) || pulsed || (swept && !killcam) {
                        sweep.pings.insert(id, (at.0, at.1, clock_millis));
                    }
                    if jammed {
                        None
                    } else if live || firing || tracked.contains(&id) {
                        Some((at.0, at.1, firing, 1.0))
                    } else {
                        let (pos, yaw, t) = *sweep.pings.get(&id)?;
                        let age = clock_millis - t;
                        (age < RADAR_PING_FADE_MS).then(|| (pos, yaw, false, (1.0 - age / RADAR_PING_FADE_MS).clamp(0.0, 1.0) as f32))
                    }
                })
                .collect();
            if jammed {
                sweep.pings.clear();
            }
            // BO2MP_RADARLOG=1 (test aid): his radar values and the enemy
            // marks, whenever they change.
            if std::env::var_os("BO2MP_RADARLOG").is_some() {
                static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
                let line = format!("radar \"{}\" pulsed {pulsed} team {my_team} pings {} shown {}", m.radar, sweep.pings.len(), enemies.len());
                if let Ok(mut last) = LAST.lock()
                    && *last != line
                {
                    diag::info!(Ui, "match hud: minimap {line} sides \"{}\"", m.teams);
                    *last = line;
                }
            }
            view.enemies = enemies;
            view.line = (uav > 0 && !jammed).then_some(line);
            // Jammed: the scramble's clock (seconds; kept small so its
            // hashes keep their precision).
            view.jam = jammed.then(|| (clock_millis / 1000.0).rem_euclid(600.0) as f32);
            Some(view)
        })
        .flatten();
    hud.shots = shots;
    hud.sweep = sweep;
    if scan_sound {
        io.sounds.write(frame::MenuCueStart { alias: "mpl_minimap_scan".to_owned() });
    }
    sub[0] = lap(&mut hud_t);
    // (The scripts' whole list once a frame: walking their tree is the dear
    // part. The blur flag sits on a menu container that is no picture or
    // text, so it is read before those are picked out.)
    // (Only what shows is copied out: most of the tree is hidden at a time.)
    let (all, elements) = hud.host.drawn_shown(0.004);
    let any_blur = all.iter().any(|d| d.blur && d.opacity > 0.004);
    let blur_amt = all.iter().filter(|d| d.blur).map(|d| d.opacity).fold(0.0f32, f32::max).clamp(0.0, 1.0);
    // His HUD hidden (the game over): only the scoreboard shows.
    let only = if hidden {
        Some(
            hud.host
                .menu_element_ids("Menu.Scoreboard")
                .unwrap_or_default(),
        )
    } else {
        None
    };
    let mut drawn: Vec<Drawn> = all
        .into_iter()
        .map(button_glyph)
        .filter(|d| {
            let asked = only.as_ref().is_none_or(|ids| ids.contains(&d.id));
            asked
                && d.opacity > 0.004
                && (d.blur
                    || d.material.is_some()
                    || d.element_kind == "image"
                    // bo2mp: `setupLoadingBar`: the engine's loading bar,
                    // two plain-colour strips under the tip box.
                    || d.element_kind == "loadingbar"
                    || d.text.as_deref().is_some_and(|t| !t.trim().is_empty())
                    || ((d.element_kind == "dashes" || d.element_kind == "meter") && d.dashes.0 > 0)
                    || (minimap.is_some() && d.element_kind.starts_with("compass_") && !d.element_kind.starts_with("compass_over")))
                // bo2mp: of the 3D widgets, the globe and the holotable
                // floor (its first grid stands for all three) are drawn by
                // `lui_scene`; the rest not.
                && !(front.is_some()
                    && crate::lui_frontend::has_depth_axis(d)
                    && !(d.element_kind == "globe" || d.material.as_deref() == Some("ui_holotable_grid")))
                // bo2mp: a menu under a blurring one (`Drawn::behind`) shows
                // blurred and dimmed: its pictures blurred, its text and
                // blocks as soft patches of their size (`soft_rect`).
                // bo2mp: no "Killed By" label under the end scoreboard.
                && !(hud.calm_frames > calm_limit() && d.text.as_deref().is_some_and(|t| t.trim().eq_ignore_ascii_case("killed by")))
        })
        .collect();
    sub[1] = lap(&mut hud_t);
    // BO2's loading screen for Die Rise, Mob of the Dead, Buried and Origins
    // sets `ui_zm_useloadingmovie` and plays the map's loading movie on a
    // `black` (then `webm_720p`) picture (ui_mp_t6_hud_loading.lua). Until
    // the movies play here, the map's own loading picture stands in for
    // them (it drew as a plain white sheet).
    if hud.loading_open {
        let v = hud.host.values.borrow();
        let dvar = |k: &str| hks_t6::host::dvar_get(&v.dvars, k).unwrap_or_default();
        let movie = dvar("ui_zm_useloadingmovie");
        if !movie.is_empty() && movie != "0" {
            let (map, loc, mode) = (dvar("ui_mapname"), dvar("ui_zm_mapstartlocation"), dvar("ui_gametype"));
            let still = [
                format!("loadscreen_{map}_{mode}_{loc}"),
                format!("loadscreen_{map}_{mode}"),
                format!("loadscreen_{map}"),
            ]
            .into_iter()
            .find(|w| icons.is_some_and(|i| i.0.iter().any(|(n, _)| n == w)));
            for d in drawn.iter_mut() {
                if matches!(d.material.as_deref(), Some("black" | "webm_720p")) {
                    d.material = still.clone().or(d.material.take());
                }
            }
        }
    }
    // The backdrop map's loading screen shows no map art: BO2 opens its
    // Zombies menus on a dark screen, not on Nuketown's (or any map's)
    // loading picture.
    if hud.loading_open && frame::bo2_frontend_waits_on_menu() {
        for d in drawn.iter_mut() {
            if d.material.as_deref().is_some_and(is_map_loading_picture) {
                d.material = Some("black".into());
            }
        }
    }
    // The engine's build number, small at the top right of every menu
    // screen (no script draws it; the loading screen has none).
    if hud.frontend && !hud.loading_open && !bo2_loading {
        drawn.push(Drawn {
            id: usize::MAX,
            element_kind: "text",
            rect: VERSION_RECT,
            opacity: 1.0,
            rgb: [1.0, 1.0, 1.0], material: None,
            text: Some(VERSION_TEXT.to_owned()),
            font: None,
            alignment: 3,
            anchors: (false, true),
            z_rot: 0.0,
            x_rot: 0.0,
            y_rot: 0.0,
            dashes: (0, 0, 0),
            dash_pitch: 0.0,
            tiles: 0.0,
            shader: [[0.0; 4]; 4],
            blur: false,
            behind: false,
            clip: None,
        });
    }
    // bo2mp: proof the end scoreboard drew its rows (the gate's check): every
    // player's name drawn on the board, logged once per opening.
    if hud.match_mp
        && hud.board_open
        && hud.calm_frames > calm_limit()
        && !hud.end_rows_logged
        && let Some(m) = hud.mp.as_ref()
    {
        let names: Vec<String> = m.borrow().board.iter().map(|r| r.name.clone()).collect();
        let seen = names
            .iter()
            .filter(|n| !n.is_empty() && drawn.iter().any(|d| d.text.as_deref().is_some_and(|t| t.contains(n.as_str()))))
            .count();
        if !names.is_empty() && seen == names.len() {
            diag::info!(Ui, "match hud: final scores, {seen} of {} rows shown", names.len());
            hud.end_rows_logged = true;
        }
    }
    // (bo2mp: the loading screen's root is the loading layer's, over the
    // engine's own loading screen; a root on another layer goes.)
    let root = match roots.iter().next() {
        Some((r, ..)) => r,
        None => cmds
            .spawn((
                LuiRoot(bo2_loading),
                OverlayStackShown,
                // (bo2zm: Zombies' front end draws over the menu and loading screens.)
                GlobalZIndex(if bo2_loading {
                    10_001
                } else if hud.frontend {
                    1_000_000
                } else {
                    0
                }),
                // bo2mp: the front end draws on the menu screen's layer.
                if bo2_loading {
                    OverlayTier::Loading
                } else if hud.frontend {
                    OverlayTier::Overlay
                } else if front.is_some() {
                    OverlayTier::Shell
                } else {
                    OverlayTier::Hud
                },
                covering_node(),
            ))
            .id(),
    };
    // (A root on the other layer goes with its nodes; draw from the next
    // frame, as nodes under it cannot be touched after its despawn.)
    if roots.iter().any(|(_, r, _)| r.0 != bo2_loading) {
        for (e, ..) in &roots {
            cmds.entity(e).try_despawn();
        }
        return;
    }
    let mut existing: HashMap<usize, Entity> = HashMap::new();
    for (e, n, ..) in &nodes {
        existing.insert(n.id, e);
    }
    let mut keep: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let now_s = time.elapsed_secs_f64();
    // bo2mp: a front-end menu over another blurs what is under it
    // (`setBlur`): the 3D backdrop is drawn blurred then.
    // (A menu is no picture or text: its flag is read before those are
    // picked out.)
    let blurred = front.is_some() && any_blur;
    // bo2mp: an in-match menu (a full-screen black dim is up) is drawn
    // with its colours as written: BO2's menu text and bars show the
    // script's 1.0 / 0.4 / 0.0 orange as 255 / 102 / 0, where the HUD's
    // linear colours go through the target's encode (G 170). The held Tab
    // scoreboard is drawn the same way (its rows' alpha 0.7 reads 0.46).
    let in_match_menu = front.is_none()
        && (hud.board_open || drawn.iter().any(|d| {
            d.element_kind == "image" && d.rgb == [0.0, 0.0, 0.0] && d.opacity > 0.3 && (d.rect[2] - d.rect[0]).abs() > 1200.0 && (d.rect[3] - d.rect[1]).abs() > 700.0
        }));
    // bo2mp: BO2 blends every multiplayer in-match picture and colour (the
    // match HUD as much as a menu over it) in the sRGB target's gamma space.
    let gamma_ui = in_match_menu || hud.match_mp;
    // bo2mp: BO2 blends its UI over the frame in the sRGB target's gamma
    // (encoded) space. The match world's camera keeps its frame in an
    // `Rgba8Unorm` target (`CompositingSpace::Srgb`), where a UI node's
    // values land as they are and blend as encoded values, so the match
    // UI is drawn onto that camera: colours and pictures go in as written
    // (as in the front end), with none of the linear-blend remaps below.
    // With no such camera (the overlay camera's sRGB target) the old
    // remaps stand.
    let lens = if gamma_ui { world_cam.first() } else { None };
    let enc_ui = lens.is_some();
    UI_ENCODED.store(enc_ui, std::sync::atomic::Ordering::Relaxed);
    if UI_ENCODED_LAST.swap(enc_ui, std::sync::atomic::Ordering::Relaxed) != enc_ui {
        // (The nodes were built for the other target: rebuilt next frame.)
        cmds.entity(root).try_despawn();
        return;
    }
    // (Only when it changes: a re-insert every frame marks the whole UI tree's
    // camera changed each frame.)
    let on = roots.get(root).ok().and_then(|(_, _, c)| c.map(|c| c.0));
    match lens {
        Some(cam) if on != Some(cam) => {
            cmds.entity(root).insert(bevy::ui::UiTargetCamera(cam));
        }
        None if on.is_some() => {
            cmds.entity(root).remove::<bevy::ui::UiTargetCamera>();
        }
        _ => {}
    }
    IN_MATCH_MENU_PICS.store(gamma_ui && !enc_ui, std::sync::atomic::Ordering::Relaxed);
    IN_FRONT_END_PICS.store(front.is_some(), std::sync::atomic::Ordering::Relaxed);
    let is_dim = |d: &Drawn| {
        d.element_kind == "image" && d.rgb == [0.0, 0.0, 0.0] && d.opacity > 0.3 && (d.rect[2] - d.rect[0]).abs() > 1200.0 && (d.rect[3] - d.rect[1]).abs() > 700.0
    };
    let dims = if front.is_none() && !enc_ui { drawn.iter().filter(|d| is_dim(d)).count() } else { 0 };
    // (The count the HUD's own pieces read: the encoded path draws the dims
    // as UI, so `dims` is 0 there and cannot say a popup is up.)
    let stacked = if front.is_none() { drawn.iter().filter(|d| is_dim(d)).count() as u32 } else { 0 };
    // bo2mp: two or more black dims stacked over the match world (the pause
    // menu's, then a popup's): the target keeps eight bits of linear light,
    // so the second dim leaves the world at 0 (BO2's gamma blend leaves it
    // near 3). The world pass (`bo2_bloom`) applies the dims itself; here the
    // dims are not drawn, and what lies under a dim (as BO2 draws it) is
    // scaled by what the dims over it let through, which is the same blend.
    if dims >= 2 && in_match_menu {
        let alphas: Vec<(usize, f32)> = drawn.iter().enumerate().filter(|(_, d)| is_dim(d)).map(|(i, d)| (i, d.opacity.clamp(0.0, 1.0))).collect();
        let mut kept: Vec<Drawn> = Vec::with_capacity(drawn.len());
        for (i, d) in drawn.iter().enumerate() {
            if is_dim(d) {
                continue;
            }
            let f: f32 = alphas.iter().filter(|(k, _)| *k > i).map(|(_, a)| 1.0 - a).product();
            let mut d = d.clone();
            if f < 1.0 {
                d.rgb = [d.rgb[0] * f, d.rgb[1] * f, d.rgb[2] * f];
            }
            kept.push(d);
        }
        drawn = kept;
    }
    if io.menu_dims.0 != stacked {
        io.menu_dims.0 = stacked;
    }
    if io.dim.0 != dims as f32 {
        diag::info!(Ui, "match hud: {dims} black screen covers on top of the world");
        io.dim.0 = dims as f32;
    }
    let behind_up = drawn.iter().any(|d| d.behind);
    // bo2mp: under a blurring menu the whole backdrop (picture, globe, floor)
    // shows at about a quarter of its level, flat and neutral: real's blurred
    // shot has no hex mesh, ring or warm tint, and its left and right edges
    // match. The menu's own black 0.15 dim is raised to leave that share;
    // the backdrop picture is blurred.
    // (The blur's amount, `blur_amt`, is read from the host's full list
    // above, not from `drawn`.)
    let is_backdrop_dim = |d: &Drawn| {
        d.element_kind == "image"
            && d.material.is_none()
            && d.rgb == [0.0, 0.0, 0.0]
            && d.opacity <= 0.3
            && (d.rect[2] - d.rect[0]).abs() > 1200.0
            && (d.rect[3] - d.rect[1]).abs() > 700.0
    };
    // bo2mp: real's rule for what a blurring menu leaves under itself (one
    // rule for every menu that covers another: `updateBlur` calls
    // `setBlur(true)` on each alike, no menu is named): everything below it,
    // the backdrop and the covered menu's content together, shows blurred at
    // one share of its own level (real's blurred shots read 0.20 to 0.25 of
    // the unblurred menu's, whether the Quit prompt or Setup Bots covers it).
    // The share is one dim over the whole covered stack, after the last
    // covered element (the menu's own pictures follow it); before this the
    // dim sat under the covered pictures, which then needed a scale per menu.
    let dim_at = if blurred && front.is_some() {
        drawn.iter().rposition(|d| d.behind).map(|i| i + 1).or_else(|| drawn.iter().position(|d| is_backdrop_dim(d)).map(|i| i + 1))
    } else {
        None
    };
    if let Some(at) = dim_at {
        drawn.insert(
            at,
            Drawn {
                id: usize::MAX - 20,
                element_kind: "image",
                rect: [0.0, 0.0, hud.host_width(), 720.0],
                // (The blur's intensity: `ui_blur_vignette` makes the picture from it.)
                opacity: blur_amt,
                rgb: [0.0, 0.0, 0.0], material: None, text: None,
                font: None,
                alignment: 2,
                anchors: (false, false),
                z_rot: 0.0,
                x_rot: 0.0,
                y_rot: 0.0,
                dashes: (0, 0, 0),
                dash_pitch: 0.0,
                tiles: 0.0,
                shader: [[0.0; 4]; 4],
                blur: false,
                behind: false,
                clip: None,
            },
        );
    }
    let first_blur = dim_at.unwrap_or(usize::MAX);
    // bo2mp: the whole map's square (FULL) is the underlay's: the picture is
    // drawn there, and the items widget (the pick cursor, the markers) has a
    // box of its own in BO2's location selector, so its own square left the
    // cursor off the map. Left, top, side in the scripts' units.
    let full_map = drawn.iter().find(|d| d.element_kind == "compass_under_full").map(|d| {
        let [x0, y0, x1, y1] = d.rect;
        let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
        let fit = w.min(h);
        [x0 + (w - fit) * 0.5, y0 + (h - fit) * 0.5, fit]
    });
    // BO2's popup blur smears the menu under it until its words are gone
    // (Theater under Select Film); the veil below only darkens, so words
    // behind the top blur are not drawn.
    let last_blur = drawn.iter().rposition(|d| d.blur && d.material.is_none());
    for (order, d) in drawn.iter().enumerate() {
        if last_blur.is_some_and(|b| order < b) && d.text.as_deref().is_some_and(|t| !t.trim().is_empty()) {
            continue;
        }
        let [x0, y0, x1, y1] = d.rect;
        // Clipped by a stencil ancestor (setUseStencil): wholly outside its
        // box is not drawn; a picture across its edge is cropped below.
        let crop = d.clip.map(|c| {
            [
                x0.min(x1).max(c[0]),
                y0.min(y1).max(c[1]),
                x0.max(x1).min(c[2]),
                y0.max(y1).min(c[3]),
            ]
        });
        if crop.is_some_and(|v| v[2] <= v[0] || v[3] <= v[1]) {
            continue;
        }
        // The scripts' colours are linear (BO2 draws into an sRGB target:
        // the chalk's 0.21 red shows as a mid blood red).
        let color = if enc_ui {
            Color::linear_rgba(d.rgb[0], d.rgb[1], d.rgb[2], d.opacity.clamp(0.0, 1.0))
        } else if gamma_ui {
            // bo2mp: BO2 blends in the sRGB target's gamma space; Bevy
            // blends in linear light, which lifts every see-through
            // thing (text at alpha 0.5 reads 186, BO2's 128; the dim
            // pips, the tile backs, the perk pictures, the focused
            // button's pulse low). Over the dark dimmed world the alpha
            // is remapped (a^2.2) so the encoded result is the gamma
            // blend's. The black dim (own remap), popups and the name
            // card's back (own scales) are left alone.
            let a = d.opacity.clamp(0.0, 1.0);
            let own = d.rgb == [0.0, 0.0, 0.0]
                || d.element_kind == "dashes"
                || d.material.as_deref().is_some_and(|m| m.starts_with("menu_mp_popup") || m.starts_with("menu_mp_dots") || m.starts_with("emblem_bg"));
            let a = if own { a } else { a.powf(2.2) };
            Color::srgba(d.rgb[0], d.rgb[1], d.rgb[2], a)
        } else {
            Color::linear_rgba(d.rgb[0], d.rgb[1], d.rgb[2], d.opacity.clamp(0.0, 1.0))
        };
        let color = if d.behind { color.with_alpha(color.alpha() * behind_dim()) } else { color };
        let backdrop_pic = blurred && front.is_some() && order < first_blur && d.element_kind == "image" && !d.behind && d.material.as_deref().is_some_and(|m| m.starts_with("lui_bkg"));
        let z = ZIndex(order as i32 + 1);
        if d.blur && d.material.is_none() && !hud.match_mp && front.is_none() {
            // A popup's blur of what is behind it: a dark veil over the
            // whole screen (BO2 blurs and darkens everything under the
            // popup, not just the popup's own box).
            keep.insert(d.id);
            let node = Node {
                // The whole screen.
                left: Val::ZERO, top: Val::ZERO,
                ..covering_node()
            };
            let veil = Color::linear_rgba(0.0, 0.0, 0.0, BLUR_VEIL * d.opacity.clamp(0.0, 1.0));
            let shows = "blur".to_owned();
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                cmds.entity(e).insert(BackgroundColor(veil));
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let e = cmds
                .spawn((LuiNode { id: d.id, shows, alpha: veil.alpha() }, node, BackgroundColor(veil), z))
                .id();
            cmds.entity(root).add_child(e);
        } else if front.is_some() && crate::lui_frontend::has_depth_axis(d) {
            // bo2mp: the 3D backdrop: the floor over the whole root, the
            // globe in its rectangle. Drawn on the graphics card each frame
            // (`lui_scene.wgsl`: BO2's own pixel shaders over its quads); the
            // CPU pictures they were (about two million texture reads every
            // quarter second for the floor, the globe ten times a second, each
            // sent to the card again) made the menus stutter. Under a blurring
            // menu the floor is the CPU picture, blurred, made once per width.
            let Some(icons) = icons.as_deref() else { continue };
            let globe = d.element_kind == "globe";
            // bo2mp: under Options / the Quit prompt the covered main menu's
            // soldiers stand over the backdrop: BO2's blurred shot has no
            // globe in it.
            // bo2mp: the globe is drawn once its map is in (the scripts
            // set its shader vector to 2 when it is: the main menu's stays
            // 0 and BO2 shows nothing there).
            if globe && (behind_up || blurred || d.shader[0][0] < 1.5) {
                continue;
            }
            // bo2mp: the real globe sits 24 units left of the widget's rectangle
            // (its right limb at 523, not 547) and a little smaller (R 354).
            let r = if globe {
                let (cx, cy) = ((x0 + x1) * 0.5 - 22.0, (y0 + y1) * 0.5);
                let (hw, hh) = ((x1 - x0).abs() * 0.5 * 0.985 * crate::lui_scene::GLOBE_PAD, (y1 - y0).abs() * 0.5 * 0.985 * crate::lui_scene::GLOBE_PAD);
                [cx - hw, cy - hh, cx + hw, cy + hh]
            } else {
                [0.0, 0.0, hud.host_width(), 720.0]
            };
            let node = ScreenBox {
                left_px: r[0] * scale,
                top_px: r[1] * scale,
                width_px: (r[2] - r[0]).abs() * scale,
                height_px: (r[3] - r[1]).abs() * scale,
            }.node();
            let (shows, card, pic) = if blurred {
                let tex = |name: &str| icons.0.iter().find(|(k, _)| k == name).map(|(_, i)| i.clone());
                let key = format!("\u{1}bo2mp floor {:.0} blurred", hud.host_width());
                if pictures.kept(&key).is_none_or(|h| image_assets.get(&h).is_none()) {
                    let (Some(a), Some(b), Some(c)) = (tex("scene:plus_tile"), tex("scene:plus_grid2"), tex("scene:plus_grid3")) else {
                        continue;
                    };
                    let (Some(a), Some(b), Some(c)) =
                        (crate::lui_scene::Tex::of(&a), crate::lui_scene::Tex::of(&b), crate::lui_scene::Tex::of(&c))
                    else {
                        continue;
                    };
                    let tv = tex("scene:tv_lookup");
                    let tv = tv.as_deref().and_then(crate::lui_scene::Tex::of);
                    let w = hud.host_width();
                    let (pw, ph) = ((w * 0.5) as usize, 270);
                    let mut px = crate::lui_scene::floor(w, pw, ph, now_s as f32, tv.as_ref(), &a, &b, &c);
                    crate::lui_scene::blur(&mut px, pw, ph, ((blur_sigma() * 0.5).round() as usize).max(1));
                    let h = image_assets.add(crate::lui_scene::image(pw, ph, px));
                    pictures.keep(key.clone(), &h);
                }
                let Some(handle) = pictures.kept(&key) else { continue };
                (format!("scene {key}"), None, Some(handle))
            } else {
                let which = if globe { "globe" } else { "floor" };
                let names = if globe {
                    ["scene:globe_map", "scene:globe_map_mesh", "scene:globe_map", "scene:globe_map"]
                } else {
                    ["scene:plus_tile", "scene:plus_grid3", "scene:plus_grid2", "scene:tv_lookup"]
                };
                let hs = names.map(|n| crate::lui_scene::gpu_texture(&mut pictures, icons, &mut image_assets, n));
                // (The floor needs its three grids, the globe its map; the
                // last picture, the globe's mesh, may be missing.)
                if hs[..if globe { 1 } else { 3 }].iter().any(Option::is_none) {
                    continue;
                }
                let has_aux = hs[if globe { 1 } else { 3 }].is_some();
                let Some(first) = hs[0].clone() else { continue };
                let [a, b, c, dd] = hs.map(|h| h.unwrap_or_else(|| first.clone()));
                static SPIN: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
                let spin = SPIN
                    .get_or_init(|| std::env::var("BO2ZM_GLOBE_SPIN").ok().and_then(|v| v.parse::<f32>().ok()))
                    .unwrap_or_else(|| (115.0 + now_s * 7.16).rem_euclid(360.0) as f32);
                let knobs = Vec4::new(now_s as f32, hud.host_width(), color.alpha(), if has_aux { 1.0 } else { 0.0 });
                let extra = if globe { Vec4::new(1.0, spin, 23.5, crate::lui_scene::GLOBE_PAD) } else { Vec4::ZERO };
                let h = match scene_cards.get(which).cloned() {
                    Some(h) if scene_mats.get(&h).is_some() => {
                        if let Some(mut m) = scene_mats.get_mut(&h) {
                            m.params = knobs;
                            m.extra = extra;
                        }
                        h
                    }
                    _ => {
                        let h = scene_mats.add(crate::lui_scene::SceneUi { params: knobs, a, b, c, d: dd, extra });
                        scene_cards.insert(which, h.clone());
                        h
                    }
                };
                (format!("scene gpu {which}"), Some(h), None)
            };
            keep.insert(d.id);
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                let c = Color::linear_rgba(1.0, 1.0, 1.0, color.alpha());
                if let Ok(mut img) = image_nodes.get_mut(e)
                    && img.color != c
                {
                    img.color = c;
                }
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let mut spawned = cmds.spawn((LuiNode { id: d.id, shows, alpha: color.alpha() }, node, z));
            if let Some(h) = card {
                spawned.insert(MaterialNode(h));
            } else if let Some(handle) = pic {
                spawned.insert(stretched(handle, Color::linear_rgba(1.0, 1.0, 1.0, color.alpha())));
            }
            let e = spawned.id();
            cmds.entity(root).add_child(e);
        } else if d.element_kind == "dashes" {
            // A slider's or attribute bar's pips, as BO2's engine draws them
            // from `setupDashes(count, filled, change, pitch)`: one engine
            // material per pip, the element's height square - the grey
            // filled pip (menu_mp_pip_blue) for each lit one, the hollow
            // outline (menu_mp_pip_outline) for the rest, and an
            // attachment's change in the green pip (better) or the red
            // pip (worse), the way CoD.AttributeBar reads its change
            // (PositiveBarColor / NegativeBarColor): positive adds that many
            // green pips after the lit ones, negative turns that many of the
            // lit ones' tail red.
            // A pip's picture is the element's own box (the slider's bar is
            // 16 wide by 32, the attribute bar 32 by 32), stepped along by
            // the pitch, as the script asks (sliders 8, Create-a-Class's bars
            // 18). Logged gap, not a rule: his real game's Create-a-Class
            // shots show about 14 for the 18 asked; no script, scale or
            // unit in the zone explains it (the engine's native
            // setupDashes is not in the scripts), so no fitted number here.
            // The first pip sits where the picture's own square does
            // (texel 10 of 32), no extra inset.
            let (count, lit, change) = d.dashes;
            let names = ["menu_mp_pip_blue", "menu_mp_pip_outline", "menu_mp_pip_green", "menu_mp_pip_red"];
            let handles: Vec<Handle<Image>> = names
                .into_iter()
                .map_while(|n| picture(&mut pictures, icons.as_deref(), &mut image_assets, n))
                .collect();
            if handles.len() < 4 {
                continue;
            }
            keep.insert(d.id);
            let pitch = d.dash_pitch;
            let qh = (y1 - y0).abs();
            let qw = (x1 - x0).abs();
            let (pitch, qw, qh) = (pitch * scale, qw * scale, qh * scale);
            let node = px_box(x0 * scale, y0.min(y1) * scale, pitch * count as f32, qh);
            let shows = format!("dashes {count} {lit} {change} {:.2} {:.2}", d.dash_pitch, d.opacity);
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let a = color.alpha();
            let e = cmds
                .spawn((LuiNode { id: d.id, shows, alpha: a }, node, z))
                .with_children(|c| {
                    // As BO2 draws them (measured, 720 high): 11 tall, 5
                    // wide, 8 apart. Lit: grey, its top 3 rows lighter and
                    // its bottom 3 a little; unlit: a faint outline. Not
                    // tinted by the bar's colour (the real ones look the
                    // same focused or not). The colours are the real
                    // screen's (94, 66, 76 of 255) through our output's
                    // curve (about a 1.2 power), so they come out the same.
                    // The level meter's are 11 square, 14 apart.
                    let a = color.alpha();
                    let unit = scale;
                    let wide = if d.element_kind == "meter" { 11.0 } else { 5.0 } * unit;
                    let grey = |v: f32| BackgroundColor(Color::linear_rgba(v, v, v, a));
                    for i in 0..count {
                        let which = if change >= 0 {
                            if i < lit {
                                0
                            } else if i < (lit + change).min(count) {
                                2
                            } else {
                                1
                            }
                        } else if i < lit + change {
                            0
                        } else if i < lit {
                            3
                        } else {
                            1
                        };
                        c.spawn((
                            px_box(pitch * i as f32, 0.0, qw, qh),
                            stretched(handles[which].clone(), Color::WHITE.with_alpha(a)),
                        ));
                    }
                })
                .id();
            cmds.entity(root).add_child(e);
        } else if d.element_kind == "meter" {
            // The voice level meter (Settings' Level Indicator): `count`
            // square dashes across its box, the lit ones filled (BO2's
            // engine draws them).
            keep.insert(d.id);
            let (count, lit, _) = d.dashes;
            // Dashes one pitch apart, as the real ones sit (measured):
            // centred across its box, 5 units up.
            let pitch = d.dash_pitch * scale;
            let inset = (((x1 - x0).abs() * scale - pitch * count as f32) / 2.0).max(0.0);
            let node = Node {
                align_items: AlignItems::Center,
                ..ScreenBox { left_px: x0 * scale + inset, top_px: (y0 - 5.0) * scale, width_px: pitch * count as f32, height_px: (y1 - y0).abs() * scale }.node()
            };
            let shows = format!("meter {count} {lit} {:.2} {:.2} {:.2} {:.2}", d.rgb[0], d.rgb[1], d.rgb[2], d.opacity);
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let e = cmds
                .spawn((LuiNode { id: d.id, shows, alpha: color.alpha() }, node, z))
                .with_children(|c| {
                    // As BO2 draws them (measured, 720 high): 11 square, 14
                    // apart. Lit: grey, its top 3 rows lighter and its bottom
                    // 3 a little; unlit: a faint outline. Not tinted by the
                    // bar's colour (the real ones look the same focused or
                    // not). The colours are the real screen's (94, 66, 76 of
                    // 255) through our output's curve (about a 1.2 power).
                    let a = color.alpha();
                    let unit = scale;
                    let wide = 11.0 * unit;
                    let grey = |v: f32| BackgroundColor(Color::linear_rgba(v, v, v, a));
                    for i in 0..count {
                        let dash = Node {
                            width: Val::Px(wide),
                            height: Val::Px(11.0 * unit),
                            margin: UiRect::right(Val::Px(pitch - wide)),
                            flex_direction: FlexDirection::Column, justify_content: JustifyContent::SpaceBetween, flex_shrink: 0.0,
                            ..default()
                        };
                        if i < lit {
                            c.spawn((dash, grey(0.33))).with_children(|d| {
                                // Two lit stripes, the upper one lighter.
                                for shade in [0.44, 0.37] {
                                    d.spawn((Node { width: Val::Percent(100.0), height: Val::Px(3.0 * unit), ..default() }, grey(shade)));
                                }
                            });
                        } else {
                            c.spawn((
                                Node { border: UiRect::all(Val::Px(unit.max(1.0))), ..dash },
                                BorderColor::all(Color::linear_rgba(1.0, 1.0, 1.0, 0.05 * a)),
                            ));
                        }
                    }
                })
                .id();
            cmds.entity(root).add_child(e);
        } else if d.element_kind == "globe" {
            // bo2zm M4: the Zombies globe, by its own shader (its mesh and
            // day map are both globe_map_zm, the material's picture; turned
            // by shader vector 2, revealed by vector 0).
            let Some(map) = d
                .material
                .as_deref()
                .and_then(|m| picture(&mut pictures, icons.as_deref(), &mut image_assets, m))
            else {
                continue;
            };
            let (mesh, day) = (map.clone(), map);
            keep.insert(d.id);
            let node = ScreenBox {
                left_px: x0 * scale,
                top_px: y0 * scale,
                width_px: (x1 - x0).abs() * scale,
                height_px: (y1 - y0).abs() * scale,
            }.node();
            let knobs = crate::bo2_globe::GlobeParams {
                v0: Vec4::from_array(d.shader[0]),
                v2: Vec4::from_array(d.shader[2]),
                color: Vec4::new(d.rgb[0], d.rgb[1], d.rgb[2], d.opacity.clamp(0.0, 1.0)),
                blur: Vec4::new(
                    if last_blur.is_some_and(|b| order < b) {
                        BLUR_UNITS * 2.0 / (x1 - x0).abs().max(1.0)
                    } else {
                        0.0
                    },
                    0.0,
                    0.0,
                    0.0,
                ),
            };
            let shows = "globe".to_owned();
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                if let Ok(m) = globe_nodes.get(e)
                    && let Some(mut mat) = globes.get_mut(&m.0)
                    && mat.params != knobs
                {
                    mat.params = knobs;
                }
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let e = cmds
                .spawn((
                    LuiNode { id: d.id, shows, alpha: color.alpha() },
                    node,
                    crate::bo2_globe::globe_node(&mut globes, knobs, mesh, day),
                    z,
                ))
                .id();
            cmds.entity(root).add_child(e);
        } else if let (Some(view), "compass", Some(material)) = (minimap.as_ref(), d.element_kind, d.material.as_deref()) {
            // bo2mp: the strip under the minimap: its picture (ticks and
            // N E S W, 360 degrees across) scrolled to his heading, 0.61 of
            // a pixel to the degree as BO2 draws it, the wrap in two pieces.
            keep.insert(d.id);
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let Some(handle) = picture(&mut pictures, icons.as_deref(), &mut image_assets, material) else { continue };
            let Some(size) = image_assets.get(&handle).map(|i| i.size_f32()) else { continue };
            let (w, h) = ((x1 - x0).abs() * scale, (y1 - y0).abs() * scale);
            let shown = (w / (0.61 * scale) / 360.0 * size.x).min(size.x);
            let centre = (view.heading(view.me.1).to_degrees().rem_euclid(360.0)) / 360.0 * size.x;
            let lo = centre - shown * 0.5;
            // The picture repeats (it is 360 degrees round): a slice that runs
            // past either end just reads on from the other, no wrap pieces.
            if let Some(img) = image_assets.get(&handle)
                && !matches!(&img.sampler, bevy::image::ImageSampler::Descriptor(d) if d.address_mode_u == bevy::image::ImageAddressMode::Repeat)
                && let Some(mut img) = image_assets.get_mut(&handle)
            {
                img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
                    ..bevy::image::ImageSamplerDescriptor::linear()
                });
            }
            let node = clipped_box(x0 * scale, y0 * scale, w, h);
            let e = cmds
                .spawn((LuiNode { id: d.id, shows: String::new(), alpha: color.alpha() }, node, z))
                .with_children(|c| {
                    // BO2 fades the strip's ticks and letters out towards both
                    // ends (measured on its shots: full to 35 units from the
                    // middle, none by 75, the strip being 148 across): drawn in
                    // 2-unit slices, each at its own alpha.
                    // Slices of whole screen pixels, each reading exactly the
                    // picture span under it (the old slices took the pieces' own
                    // fractional widths, which the layout rounded to pixels
                    // while their picture spans stayed exact: the letters
                    // landed 52 and 56 px apart, real is 55).
                    let step = (2.0 * scale).round().max(1.0);
                    let n = (w / step).ceil() as usize;
                    for i in 0..n {
                        let (x0s, x1s) = (i as f32 * step, ((i + 1) as f32 * step).min(w));
                        let d = ((x0s + x1s) * 0.5 - w * 0.5).abs() / scale;
                        let fade = (1.0 - (d - 35.0) / 40.0).clamp(0.0, 1.0);
                        if fade <= 0.0 {
                            continue;
                        }
                        let mut img = ImageNode::new(handle.clone()).with_color(color.with_alpha(color.alpha() * fade));
                        img.rect = Some(Rect::new(lo + x0s / w * shown, 0.0, lo + x1s / w * shown, size.y));
                        c.spawn((px_box(x0s, 0.0, x1s - x0s, h), img));
                    }
                })
                .id();
            cmds.entity(root).add_child(e);
        } else if let (Some(view), true) = (minimap.as_ref(), d.element_kind.starts_with("compass_")) {
            // bo2mp: BO2's minimap, redrawn every frame. The HUD's (type
            // PARTIAL): the map around him turned so he faces up (made on
            // the CPU: the 2D layer cannot clip a turned picture), his arrow
            // in the middle, his team's around him. The pause menu's (FULL):
            // the whole map, north up, everyone where they are.
            keep.insert(d.id);
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let full = d.element_kind.ends_with("_full");
            let (w, h) = ((x1 - x0).abs() * scale, (y1 - y0).abs() * scale);
            let px_per_unit = w / view.range;
            let node = clipped_box(x0 * scale, y0 * scale, w, h);
            let mut kids: Vec<(Node, ImageNode, UiTransform)> = Vec::new();
            let mut real_backing = false;
            // The whole map's square inside the widget (FULL).
            let (fx0, fy0, fit) = match full_map.filter(|_| full) {
                Some([mx, my, side]) => ((mx - x0) * scale, (my - y0) * scale, side * scale),
                None => {
                    let fit = w.min(h);
                    ((w - fit) * 0.5, (h - fit) * 0.5, fit)
                }
            };
            let flat = px_box;
            // A picture laid flat over a box, not turned.
            let unturned = |at: Node, shown: ImageNode| (at, shown, UiTransform::IDENTITY);
            // The backing and overlay pictures take only the widget's fade, not its colour.
            let faded = Color::WHITE.with_alpha(color.alpha());
            if d.element_kind.starts_with("compass_under") {
                // BO2's scrambled map's two pictures (jammed by an enemy
                // Counter-UAV).
                let raw = |name: &str| {
                    let (_, i) = icons.as_deref()?.0.iter().find(|(k, _)| *k == name)?;
                    Some((i.data.as_deref()?, i.texture_descriptor.size.width as usize, i.texture_descriptor.size.height as usize))
                };
                let scramble = view.jam.and_then(|_| Some(crate::lui_frontend::Scramble { noise: raw("sam_block_noise")?, tv: raw("tv_lookup")? }));
                if full {
                    // Jammed: the whole map scrambled in place of its picture.
                    let jammed = scramble.as_ref().zip(raw(&view.pic)).map(|(s, (d, sw, sh))| {
                        let n = (fit.round() as usize).clamp(32, 512);
                        let made = crate::lui_scene::image(n, n, view.render_scrambled(d, sw, sh, n, s));
                        pictures.refill(&mut image_assets, "\u{1}bo2mp map scrambled", made)
                    });
                    let Some(handle) = jammed.or_else(|| picture(&mut pictures, icons.as_deref(), &mut image_assets, &view.pic)) else {
                        continue;
                    };
                    kids.push(unturned(flat(fx0, fy0, fit, fit), ImageNode::new(handle).with_color(color)));
                } else {
                    let Some(src) = icons.as_deref().and_then(|i| i.0.iter().find(|(k, _)| *k == view.pic)).map(|(_, i)| i.clone())
                    else {
                        continue;
                    };
                    let (sw, sh) = (src.texture_descriptor.size.width as usize, src.texture_descriptor.size.height as usize);
                    let n = (w.round() as usize).clamp(32, 320);
                    // BO2's radar line material over the map while a UAV is up.
                    let line = view.line.and_then(|_| icons.as_deref()?.0.iter().find(|(k, _)| *k == "compass_radarline")).map(|(_, i)| i);
                    let line = line.and_then(|i| {
                        let (lw, lh) = (i.texture_descriptor.size.width as usize, i.texture_descriptor.size.height as usize);
                        Some((i.data.as_deref()?, lw, lh))
                    });
                    let pixels = src.data.as_deref().map(|d| view.render(d, sw, sh, n, line, scramble.as_ref())).unwrap_or_default();
                    // One picture, rewritten every frame.
                    let handle = pictures.refill(&mut image_assets, "\u{1}bo2mp minimap", crate::lui_scene::image(n, n, pixels));
                    // bo2mp: the box's backing is BO2's own compass materials, not a
                    // fitted grey: `compass_map_color_underlay2` (black, alpha 60/255)
                    // and `compass_map_color_underlay` (32x32 dark green-grey, alpha
                    // 1.0 at the top to 0.09 at the bottom), stretched over the box
                    // under the map, both at the material's own alpha (the
                    // materials carry no constants: technique trivial_9z33feqw,
                    // so the texture alpha is the whole rule; no strength factor).
                    for name in ["compass_map_color_underlay2", "compass_map_color_underlay"] {
                        if let Some(under) = picture(&mut pictures, icons.as_deref(), &mut image_assets, name) {
                            kids.push(unturned(flat(0.0, 0.0, w, h), ImageNode::new(under).with_color(faded)));
                            real_backing = true;
                        }
                    }
                    kids.push(unturned(flat(0.0, 0.0, w, h), ImageNode::new(handle).with_color(color)));
                    // BO2's minimap overlay (`setupCompassOverlay`): the box's
                    // own faint hex mesh, fading in towards the bottom, over
                    // the map (the empty area below the map shows it).
                    if let Some(over) = picture(&mut pictures, icons.as_deref(), &mut image_assets, "mp_minimap_overlay") {
                        kids.push(unturned(flat(0.0, 0.0, w, h), ImageNode::new(over).with_color(faded)));
                    }
                }
            } else {
                let arrow = if full { (fit * 0.085).max(8.0) } else { (w * 0.155).max(8.0) };
                // bo2mp: under the End Game popup the pause menu sits under two
                // black dims; the dims are not drawn (see above), the compass
                // widget's colour carries what they let through, and the map
                // markers take it too (BO2 shows them near 51 of 255 there,
                // not full bright).
                let ping_tint = if full && in_match_menu && d.rgb != [1.0, 1.0, 1.0] {
                    if enc_ui { Color::linear_rgba(d.rgb[0], d.rgb[1], d.rgb[2], 1.0) } else { Color::srgba(d.rgb[0], d.rgb[1], d.rgb[2], 1.0) }
                } else {
                    Color::WHITE
                };
                let mut ping = |name: &str, at: [f32; 2], turn: f32, alpha: f32| {
                    if at[0] < 0.0 || at[1] < 0.0 || at[0] > w || at[1] > h {
                        return;
                    }
                    let Some(handle) = picture(&mut pictures, icons.as_deref(), &mut image_assets, name) else { return };
                    kids.push((
                        flat(at[0] - arrow * 0.5, at[1] - arrow * 0.5, arrow, arrow),
                        ImageNode::new(handle).with_color(ping_tint.with_alpha(ping_tint.alpha() * alpha)),
                        UiTransform { rotation: Rot2::radians(turn), ..UiTransform::IDENTITY },
                    ));
                };
                // Where a world point sits in the widget, and which way a yaw
                // points there.
                let me_turn = view.heading(view.me.1);
                let at = |p: [f32; 2]| {
                    if full {
                        let [u, v] = view.uv(p);
                        [fx0 + u * fit, fy0 + v * fit]
                    } else {
                        let o = view.screen_offset(p);
                        [w * 0.5 + o[0] * px_per_unit, h * 0.5 + o[1] * px_per_unit]
                    }
                };
                let turn = |yaw: f32| if full { view.heading(yaw) } else { view.heading(yaw) - me_turn };
                for (p, yaw, firing, alpha) in &view.enemies {
                    ping(if *firing { "compassping_enemyfiring" } else { "compassping_enemy" }, at(*p), turn(*yaw), *alpha);
                }
                // Jammed (BO1 fades the compass icons to 0): no team, no arrow.
                if view.jam.is_none() {
                    for (p, yaw) in &view.friends {
                        ping("compassping_friendly_mp", at(*p), turn(*yaw), 1.0);
                    }
                    ping("compassping_player", at(view.me.0), turn(view.me.1), 1.0);
                }
                // bo2mp: the objectives the scripts give a compass picture
                // (`objective_icon`: the care package's
                // compass_supply_drop_green his team's, _red an enemy's), at
                // `compassObjectiveWidth` 32 (the ping's 0.155 of the box is
                // taken as BO2's 24, so 32 is 0.207). Past the edge a mark
                // sits on it (BO2 `compassClampIcons` 1): it points the way to
                // the crate. Jammed, none. A client script's own objective
                // (Express's train cars, the Hellstorm) gives its own width
                // and may be any picture, turn with its entity and be cut
                // off at the map's edge instead (objectiveMapInfo
                // rotateWithEntity, clipToMap). One coloured in code (the
                // Hellstorm's white ring: colorMaterialInCode) takes the
                // colour the sim gives it, BO2's relative team colour.
                let rows = hud.sent.as_ref().and_then(|v| v.mp.as_ref()).map(|m| m.objectives.clone()).unwrap_or_default();
                if view.jam.is_none() && !rows.is_empty() {
                    let (x0, y0, x1, y1) = if full { (fx0, fy0, fx0 + fit, fy0 + fit) } else { (0.0, 0.0, w, h) };
                    for o in hks_t6::mp::parse_objectives(&rows).values() {
                        if o.state != 1 || !(o.icon.starts_with("compass_") || o.icon_size.is_some()) {
                            continue;
                        }
                        let size = arrow * o.icon_size.unwrap_or(32.0) / 24.0;
                        let half = size * 0.5;
                        let p = at([o.spot[0], o.spot[1]]);
                        let c = if o.map_info & hks_t6::mp::OBJ_CLIP_TO_MAP != 0 {
                            if p[0] + half < x0 || p[0] - half > x1 || p[1] + half < y0 || p[1] - half > y1 {
                                continue;
                            }
                            p
                        } else {
                            [p[0].clamp(x0 + half, x1 - half), p[1].clamp(y0 + half, y1 - half)]
                        };
                        let spin = if o.map_info & hks_t6::mp::OBJ_ROTATE_WITH_ENTITY != 0 {
                            UiTransform { rotation: Rot2::radians(turn(o.facing)), ..UiTransform::IDENTITY }
                        } else {
                            UiTransform::IDENTITY
                        };
                        let Some(handle) = picture(&mut pictures, icons.as_deref(), &mut image_assets, &o.icon) else { continue };
                        let colour = match o.tint {
                            Some([r, g, b]) if o.map_info & hks_t6::mp::OBJ_COLOR_MATERIAL_IN_CODE != 0 => {
                                let t = if enc_ui { Color::linear_rgba(r, g, b, 1.0) } else { Color::srgba(r, g, b, 1.0) }.to_linear();
                                let d = ping_tint.to_linear();
                                Color::LinearRgba(LinearRgba::new(t.red * d.red, t.green * d.green, t.blue * d.blue, 1.0))
                            }
                            _ => ping_tint,
                        };
                        kids.push((flat(c[0] - half, c[1] - half, size, size), ImageNode::new(handle).with_color(colour), spin));
                    }
                }
                // Picking a spot (bo2mp_locsel "<material> <radius>"): the
                // selector where his cursor is, its radius to the map's scale.
                let locsel = hud.sent.as_ref().and_then(|v| v.mp.as_ref()).map(|m| m.locsel.clone()).unwrap_or_default();
                if full
                    && let Some((material, radius)) = locsel.trim().split_once(' ')
                    && let Some(c) = cursor.as_deref()
                    && let Some(handle) = picture(&mut pictures, icons.as_deref(), &mut image_assets, material)
                {
                    let radius: f32 = radius.trim().parse().unwrap_or(0.0);
                    let span = view.span_size();
                    let (sw, sh) = ((2.0 * radius / span[0]).max(0.02) * fit, (2.0 * radius / span[1]).max(0.02) * fit);
                    let (cx, cy) = (fx0 + c.at[0].clamp(0.0, 1.0) * fit, fy0 + c.at[1].clamp(0.0, 1.0) * fit);
                    // The part of the selector over the map: past the map's
                    // edge it is cut off, not drawn over the blurred world.
                    let (l, t) = (cx - sw * 0.5, cy - sh * 0.5);
                    let (x_lo, y_lo) = (l.max(fx0), t.max(fy0));
                    let (x_hi, y_hi) = ((l + sw).min(fx0 + fit), (t + sh).min(fy0 + fit));
                    if x_hi > x_lo && y_hi > y_lo {
                        let size = image_assets.get(&handle).map_or(Vec2::ONE, |i| i.size_f32());
                        let cut = Rect::new(
                            (x_lo - l) / sw * size.x,
                            (y_lo - t) / sh * size.y,
                            (x_hi - l) / sw * size.x,
                            (y_hi - t) / sh * size.y,
                        );
                        kids.push(unturned(flat(x_lo, y_lo, x_hi - x_lo, y_hi - y_lo), ImageNode::new(handle).with_rect(cut)));
                    }
                }
            }
            // bo2mp: the HUD map's dark square and thin border (the real
            // map shows them under and around the picture).
            let mut node = node;
            let backdrop = if d.element_kind.starts_with("compass_under") && !full {
                node.border = UiRect::all(Val::Px(scale.max(1.0)));
                if real_backing {
                    // The backing is the underlay pictures; only the thin box edge
                    // (no source found, fitted) is still drawn here.
                    (BackgroundColor(Color::NONE), BorderColor::all(if enc_ui { Color::linear_rgba(0.85, 0.88, 0.88, 0.085 * color.alpha()) } else { Color::srgba(0.85, 0.88, 0.88, 0.085 * color.alpha()) }))
                } else if enc_ui {
                    (BackgroundColor(Color::linear_rgba(0.12, 0.12, 0.12, 0.72 * color.alpha())), BorderColor::all(Color::linear_rgba(0.85, 0.88, 0.88, 0.085 * color.alpha())))
                } else {
                    (BackgroundColor(Color::srgba(0.12, 0.12, 0.12, 0.72 * color.alpha())), BorderColor::all(Color::srgba(0.85, 0.88, 0.88, 0.085 * color.alpha())))
                }
            } else {
                (BackgroundColor(Color::NONE), BorderColor::all(Color::NONE))
            };
            let e = cmds
                .spawn((LuiNode { id: d.id, shows: String::new(), alpha: color.alpha() }, node, z, backdrop))
                .with_children(|c| {
                    for k in kids {
                        c.spawn(k);
                    }
                })
                .id();
            cmds.entity(root).add_child(e);
        } else if (d.element_kind == "image" || d.element_kind == "loadingbar")
            && d.material.as_deref().is_none_or(|m| {
                m == "white" || (m == "black" && picture(&mut pictures, icons.as_deref(), &mut image_assets, m).is_none())
            })
        {
            // A picture with no material (or the engine's plain `white`) is
            // a block of its colour: BO2's dim behind its menus. (A named
            // picture that was not loaded draws nothing: a white block in
            // its place - Create-a-Class's perk icons - is never BO2's.)
            keep.insert(d.id);
            // bo2mp: a stencil above it (a map-vote tile's menu-sized
            // highlight) cuts the block to its rectangle, as it does a picture.
            let (bx0, by0, bx1, by1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
            let (bx0, by0, bx1, by1) = match d.clip {
                Some(c) => (bx0.max(c[0]), by0.max(c[1]), bx1.min(c[2]), by1.min(c[3])),
                None => (bx0, by0, bx1, by1),
            };
            if d.clip.is_some() && (bx1 <= bx0 || by1 <= by0) {
                continue;
            }
            let node = ScreenBox {
                left_px: bx0 * scale,
                top_px: by0 * scale,
                width_px: (bx1 - bx0) * scale,
                height_px: (by1 - by0) * scale,
            }.node();
            if d.id == usize::MAX - 20 {
                // bo2mp: the engine blur's vignette and darkening over the
                // covered stack (see `blur_vignette`): a black picture whose
                // alpha is one minus the shader's factor, stretched over the
                // whole screen.
                let q = (d.opacity.clamp(0.0, 1.0) * 100.0).round() as i32;
                let key = format!("\u{1}vignette {q}");
                let handle = pictures.kept_or_make(key.clone(), || image_assets.add(blur_vignette(q as f32 / 100.0)));
                let tint = Color::srgba(0.0, 0.0, 0.0, 1.0);
                if let Some(&e) = existing.get(&d.id)
                    && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                    && n.shows == key
                {
                    nn.set_if_neq(node);
                    zi.set_if_neq(z);
                    continue;
                }
                if let Some(&e) = existing.get(&d.id) {
                    cmds.entity(e).try_despawn();
                }
                let e = cmds
                    .spawn((
                        LuiNode { id: d.id, shows: key, alpha: 1.0 },
                        node,
                        stretched(handle, tint),
                        z,
                    ))
                    .id();
                cmds.entity(root).add_child(e);
                continue;
            }
            if d.behind {
                // A block under a blurring menu: blurred (a soft patch).
                let rect_u = [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()];
                soft_patch(&mut cmds, root, &mut pictures, &mut image_assets, &existing, &mut nodes, &mut image_nodes, d.id, rect_u, scale, color, z);
                continue;
            }
            let shows = "block".to_owned();
            let color = if d.material.as_deref() == Some("black") { Color::linear_rgba(0.0, 0.0, 0.0, color.alpha()) } else { color };
            // (The engine's loading bar has no material of its own in the
            // zones; `white` is plain alpha blend, src alpha / inverse src
            // alpha, image `$white` opaque, so its strips draw at the
            // script's colour and alpha as they are.)
            // bo2mp: BO2 blends its menus' black dim in the sRGB target's
            // gamma space; Bevy blends in linear light, where 0.8 black over
            // a bright world leaves it twice as bright. Over the match's
            // world (not the front end's tuned backdrop) the alpha is
            // remapped so the dimmed picture is the gamma blend's.
            let dim_over_world = !enc_ui && front.is_none() && d.rgb == [0.0, 0.0, 0.0] && color.alpha() > 0.3;
            // bo2mp: the killcam's top and bottom bands (a see-through red,
            // black in the final killcam; no material, 60-130 high at the
            // screen's edge) blend in BO2's gamma space too. Pure black gets
            // the exact remap; red the milder 1.5 power, which keeps its
            // green/blue right and its red from running away over the
            // world's dark parts (real/55's bands read pink, not grey).
            let band_h = (d.rect[3] - d.rect[1]).abs();
            let band_edge = d.rect[1].min(d.rect[3]) <= 1.0 || d.rect[1].max(d.rect[3]) >= 719.0;
            let killcam_band = !enc_ui && front.is_none() && !in_match_menu && d.material.is_none() && (60.0..=130.0).contains(&band_h) && band_edge;
            let color = if killcam_band && d.rgb == [1.0, 0.0, 0.0] {
                Color::linear_rgba(1.0, 0.0, 0.0, 1.0 - (1.0 - color.alpha()).powf(1.5))
            } else if killcam_band && d.rgb == [0.0, 0.0, 0.0] {
                Color::linear_rgba(0.0, 0.0, 0.0, 1.0 - (1.0 - color.alpha()).powf(2.2))
            } else if dim_over_world {
                Color::linear_rgba(0.0, 0.0, 0.0, 1.0 - (1.0 - color.alpha()).powf(2.2))
            } else {
                color
            };
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                match backgrounds.get_mut(e) {
                    Ok(mut bg) => {
                        bg.set_if_neq(BackgroundColor(color));
                    }
                    Err(_) => {
                        cmds.entity(e).insert(BackgroundColor(color));
                    }
                }
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let e = cmds
                .spawn((LuiNode { id: d.id, shows, alpha: color.alpha() }, node, BackgroundColor(color), z))
                .id();
            cmds.entity(root).add_child(e);
        } else if let Some(mat) = &d.material {
            // BO2ZM_LUI_FORCEMAT=<id>:<material> draws that element with
            // another picture (debugging aid).
            let forced = std::env::var("BO2ZM_LUI_FORCEMAT")
                .ok()
                .and_then(|v| {
                    v.split_once(':')
                        .map(|(i, m)| (i.parse::<usize>().ok(), m.to_owned()))
                })
                .filter(|(i, _)| *i == Some(d.id))
                .map(|(_, m)| m);
            let mat = forced.as_ref().unwrap_or(mat);
            // bo2mp: the popup panel is a solid dark grey in BO2 (33 on every
            // panel and dots picture; what lies under it does not show).
            // The textures read raw give ~0.107 linear; the target's encode
            // lifts 0.107 x 0.142 = 0.0152 to 33. Drawn opaque (alpha 1)
            // at that scale; the old alpha x 0.116 left it see-through and
            // darker (7-16).
            let color = if !enc_ui && front.is_none() && (mat.starts_with("menu_mp_popup") || mat.starts_with("menu_mp_dots")) {
                let l = color.to_linear();
                Color::linear_rgba(l.red * 0.142, l.green * 0.142, l.blue * 0.142, l.alpha)
            } else {
                color
            };
            let (pic, pad_share) = if d.behind || (backdrop_pic && blur_bkg()) {
                behind_picture(&mut pictures, icons.as_deref(), &mut image_assets, mat, (x1 - x0).abs(), d.behind && d.clip.is_none())
                    .map(|(h, px, py)| (Some(h), (px, py)))
                    .unwrap_or_default()
            } else {
                (picture(&mut pictures, icons.as_deref(), &mut image_assets, mat), (0.0, 0.0))
            };
            let Some(handle) = pic else { continue };
            // (The padded picture's border lies outside its rectangle.)
            let (pw, ph) = ((x1 - x0).abs() * pad_share.0, (y1 - y0).abs() * pad_share.1);
            let (x0, x1) = if x1 >= x0 { (x0 - pw, x1 + pw) } else { (x0 + pw, x1 - pw) };
            let (y0, y1) = if y1 >= y0 { (y0 - ph, y1 + ph) } else { (y0 + ph, y1 - ph) };
            // bo2mp: a stencil above it (the scorestreak bar) cuts the
            // picture to its rectangle: what shows, and the part of the
            // picture that lands there.
            let (nx0, ny0, nx1, ny1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
            let (mut vx0, mut vy0, mut vx1, mut vy1) = (nx0, ny0, nx1, ny1);
            let mut cut: Option<Rect> = None;
            if let Some(c) = d.clip {
                (vx0, vy0, vx1, vy1) = (nx0.max(c[0]), ny0.max(c[1]), nx1.min(c[2]), ny1.min(c[3]));
                if vx1 <= vx0 || vy1 <= vy0 {
                    continue;
                }
                if let Some(img) = image_assets.get(&handle)
                    && nx1 > nx0
                    && ny1 > ny0
                    && (vx0, vy0, vx1, vy1) != (nx0, ny0, nx1, ny1)
                {
                    let (tw, th) = (img.width() as f32, img.height() as f32);
                    cut = Some(Rect::new(
                        (vx0 - nx0) / (nx1 - nx0) * tw,
                        (vy0 - ny0) / (ny1 - ny0) * th,
                        (vx1 - nx0) / (nx1 - nx0) * tw,
                        (vy1 - ny0) / (ny1 - ny0) * th,
                    ));
                }
            }
            keep.insert(d.id);
            // Cropped to its clip: the visible part of the box, and the
            // same part of the picture (unturned pictures only).
            let (bx0, by0, bx1, by1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
            let (vis, part) = match crop {
                Some(v) if d.z_rot.abs() < 0.01 && v != [bx0, by0, bx1, by1] => {
                    let size = image_assets.get(&handle).map_or(Vec2::ONE, |i| i.size().as_vec2());
                    let (bw, bh) = ((bx1 - bx0).max(1e-3), (by1 - by0).max(1e-3));
                    let part = bevy::math::Rect::new(
                        (v[0] - bx0) / bw * size.x,
                        (v[1] - by0) / bh * size.y,
                        (v[2] - bx0) / bw * size.x,
                        (v[3] - by0) / bh * size.y,
                    );
                    (v, Some(part))
                }
                _ => ([x0, y0, x1, y1], None),
            };
            // A loader (`lui_loader_32`, the lobby's "Searching for player"
            // spinner) is a strip of frames its shader steps through, one
            // turn a second, started at shader vector 0's x: one frame. The
            // 32 one holds 21 square frames squeezed into 512 pixels.
            let part = if mat.starts_with("lui_loader")
                && let Some(size) = image_assets.get(&handle).map(|i| i.size().as_vec2())
                && size.x >= size.y * 2.0
            {
                let frames = if mat == "lui_loader_32" { 21.0 } else { (size.x / size.y).floor() };
                let wide = size.x / frames;
                let turn = hud.host.clock_millis() / 1000.0 + f64::from(d.shader[0][0]);
                let frame = ((turn.fract() * f64::from(frames)).floor() as f32).min(frames - 1.0);
                Some(bevy::math::Rect::new(frame * wide, 0.0, (frame + 1.0) * wide, size.y))
            } else {
                part
            };
            let node = ScreenBox {
                left_px: vx0 * scale,
                top_px: vy0 * scale,
                width_px: (vx1 - vx0) * scale,
                height_px: (vy1 - vy0) * scale,
            }.node();
            // bo2mp: a rectangle drawn right to left (or bottom to top) is
            // the picture turned over: the right-hand gun panel is the left
            // one mirrored.
            // bo2mp: the name card's dark backing is drawn at 0.85 over the
            // world; blended in linear light it comes out twice as bright
            // as BO2's gamma-space blend, so the alpha is remapped as the
            // menus' black dim is.
            let color = if front.is_none() && mat.starts_with("emblem_bg") && color.alpha() < 1.0 {
                color.with_alpha(1.0 - (1.0 - color.alpha()).powf(2.2))
            } else {
                color
            };
            let (flip_x, flip_y) = (x1 < x0, y1 < y0);
            // bo2mp: setupTiles(n): the picture repeats across the width in
            // tiles n units wide, stretched down (setTileVertically(false)):
            // the menus' dot grid is an 8-wide picture tiled at 8.
            let image_mode = match image_assets.get(&handle) {
                Some(img) if d.tiles >= 1.0 && cut.is_none() && img.width() > 0 => bevy::ui::widget::NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: false,
                    stretch_value: d.tiles * scale / img.width() as f32,
                },
                _ => bevy::ui::widget::NodeImageMode::Stretch,
            };
            // bo2mp scope: an additive picture in the match HUD is added onto
            // the scene (see lui_additive), not alpha-blended.
            let add_pic = (front.is_none() && cut.is_none() && !in_match_menu && add_names.as_ref().is_some_and(|n| n.contains(mat.as_str())))
                .then(|| format!("additive:{mat}"))
                .and_then(|n| picture(&mut pictures, icons.as_deref(), &mut image_assets, &n));
            if let Some(pic) = add_pic {
                let lin = color.to_linear();
                let tint = lin;
                // (The colour is in the key: a fade respawns the node.)
                let shows = format!("additive {mat} {:.2} {:.2} {:.2} {:.2}", lin.red, lin.green, lin.blue, lin.alpha);
                // (A node reaching past the screen keeps to it, the picture
                // cut to match: the material pass drops such a node.)
                let (cx0, cy0, cx1, cy1) = (vx0.max(0.0), vy0.max(0.0), vx1.min(720.0 * aspect), vy1.min(720.0));
                if cx1 <= cx0 || cy1 <= cy0 {
                    continue;
                }
                let (w, h) = ((vx1 - vx0).max(0.001), (vy1 - vy0).max(0.001));
                let uv = Vec4::new((cx0 - vx0) / w, (cy0 - vy0) / h, (cx1 - cx0) / w, (cy1 - cy0) / h);
                // (A flipped picture reads its window from the far side, backwards.)
                let uv = Vec4::new(if flip_x { 1.0 - uv.x } else { uv.x }, if flip_y { 1.0 - uv.y } else { uv.y }, if flip_x { -uv.z } else { uv.z }, if flip_y { -uv.w } else { uv.w });
                let node = Node {
                    left: Val::Px(cx0 * scale),
                    top: Val::Px(cy0 * scale),
                    width: Val::Px((cx1 - cx0) * scale),
                    height: Val::Px((cy1 - cy0) * scale),
                    ..node
                };
                if let Some(&e) = existing.get(&d.id)
                    && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                    && n.shows == shows
                {
                    nn.set_if_neq(node);
                    zi.set_if_neq(z);
                    continue;
                }
                if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                    diag::info!(Ui, "menus: glow piece #{} {shows} at {:?} node {:?} uv {:?}", d.id, d.rect, (cx0, cy0, cx1, cy1), uv);
                }
                if let Some(&e) = existing.get(&d.id) {
                    cmds.entity(e).try_despawn();
                }
                let e = cmds
                    .spawn((
                        LuiNode { id: d.id, shows, alpha: color.alpha() },
                        node,
                        MaterialNode(additive.add(crate::lui_additive::AdditiveUi { tint, picture: pic, uv, encoded: if enc_ui { 1.0 } else { 0.0 }, color_add: if color_add_names.contains(mat.as_str()) { 1.0 } else { 0.0 } })),
                        z,
                    ))
                    .id();
                cmds.entity(root).add_child(e);
                continue;
            }
            let shows = if d.behind { format!("image {mat} behind") } else { format!("image {mat}") };
            if std::env::var("BO2ZM_LUI_WATCH").ok().and_then(|v| v.parse::<usize>().ok()) == Some(d.id) {
                diag::info!(
                    Ui,
                    "menus: watching #{} at {:.0} ms: box {:?} colour {:?} entity {:?} z {} laid out {:?}",
                    d.id,
                    hud.host.clock_millis(),
                    d.rect,
                    color,
                    existing.get(&d.id),
                    order,
                    existing
                        .get(&d.id)
                        .and_then(|e| computed.get(*e).ok())
                        .map(|(c, t, v)| (c.size(), t.translation, v.get()))
                );
            }
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                if let Ok(mut img) = image_nodes.get_mut(e)
                    && (img.color != color
                        || img.flip_x != flip_x
                        || img.flip_y != flip_y
                        || img.rect != cut
                        || img.image_mode != image_mode)
                {
                    img.color = color;
                    img.flip_x = flip_x;
                    img.flip_y = flip_y;
                    img.rect = cut;
                    img.image_mode = image_mode.clone();
                }
                // setZRot(deg): a picture turned in its own plane (the
                // Selected Scorestreaks arrows: the up arrow at 270 reads
                // as a right arrow); LUI turns counter-clockwise.
                if d.z_rot.abs() > 0.01 {
                    cmds.entity(e).insert(UiTransform { rotation: Rot2::degrees(-d.z_rot), ..UiTransform::IDENTITY });
                }
                continue;
            }
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                diag::info!(Ui, "menus: picture #{} {shows} at {:?}", d.id, d.rect);
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let e = cmds
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    // BO2 stretches every picture over its element (an 8x32
                    // gradient fills a whole box); bevy's Auto mode would
                    // keep the picture's shape inside it.
                    ImageNode { flip_x, flip_y, rect: cut, ..ImageNode::new(handle).with_color(color).with_mode(image_mode) },
                    z,
                    UiTransform { rotation: Rot2::degrees(-d.z_rot), ..UiTransform::IDENTITY },
                ))
                .id();
            cmds.entity(root).add_child(e);
        } else if let (Some(text), Some(bo2)) = (&d.text, fonts.as_deref()) {
            keep.insert(d.id);
            let text = plain_with(text, pad_style);
            // bo2mp: black see-through text in the match HUD (the gun-name slab's
            // 0.5 black letters) is blended in gamma space by BO2: over a pale
            // slab the letters read dark (scene * (1 - a)). Bevy blends in linear
            // light and leaves them washed; the alpha is remapped as the black
            // dim's is.
            let color = if !enc_ui && front.is_none() && !in_match_menu && d.rgb == [0.0, 0.0, 0.0] && color.alpha() < 0.99 {
                color.with_alpha(1.0 - (1.0 - color.alpha()).powf(2.2))
            } else {
                color
            };
            let font = font_name(d.font.as_deref().unwrap_or("normalFont"));
            let px = (y1 - y0).abs() * scale;
            // A text with one of BO2's key pictures in it (the Controls rows'
            // mouse buttons: one private-use letter each) is a row of text
            // pieces and pictures.
            let pieces = {
                let values = hud.host.values.borrow();
                icon_pieces(&text, &|n| font_icon(&values.tables, n))
            };
            let has_icons = pieces.iter().any(|p| matches!(p, Piece::Icon(_)));
            let width = if has_icons {
                pieces.iter().fold(0.0, |sum, p| {
                    sum + match p {
                        Piece::Icon(i) => i.across * px,
                        Piece::Words(t) => bo2.line_width(font, t, px),
                    }
                })
            } else {
                bo2.line_width(font, &text, px)
            };
            // A text wider than its box (a box at least 100 units wide)
            // wraps at its words onto lines below, as BO2's does (Create-
            // a-Class's weapon descriptions); a box of 100 or less (the
            // lobby's "Game starting in N" is exactly 100) is a place,
            // not a width.
            let box_w = (x1 - x0).abs() * scale;
            let lines = if has_icons {
                vec![text.clone()]
            } else if box_w > 100.5 * scale && width > box_w + 1.0 { wrap_words(bo2, font, &text, px, box_w) } else { vec![text.clone()] };
            let wrapped = lines.len() > 1;
            // LUI.Alignment: 2 centre, 3 right, else left (a text with
            // none set reads from its box's anchors, see `Drawn`).
            let left = match (wrapped, d.alignment) {
                (true, _) => x0.min(x1) * scale,
                (_, 2) => (x0 + x1) * 0.5 * scale - width * 0.5,
                (_, 3) => x1 * scale - width,
                _ => x0 * scale,
            };
            if d.behind {
                // A text under a blurring menu: its letters (the font sheet's
                // coverage, laid out as the lines below) blurred as BO2's
                // engine blurs the menu layer it was drawn into.
                let px_u = px / scale;
                let lefts: Vec<(String, f32)> = lines
                    .iter()
                    .map(|l| {
                        let w = bo2.line_width(font, l, px) / scale;
                        let l_left = match (wrapped, d.alignment) {
                            (true, 2) => (x0 + x1) * 0.5 - w * 0.5,
                            (true, 3) => x1 - w,
                            (true, _) => x0.min(x1),
                            _ => left / scale,
                        };
                        (l.clone(), l_left)
                    })
                    .collect();
                let bl = lefts.iter().map(|(_, l)| *l).fold(f32::MAX, f32::min);
                let br = lefts.iter().map(|(l, x)| *x + bo2.line_width(font, l, px) / scale).fold(f32::MIN, f32::max);
                let rel: Vec<(String, f32)> = lefts.iter().map(|(l, x)| (l.clone(), x - bl)).collect();
                let sigma = blur_sigma();
                let radius = ((sigma * 0.5).round() as usize).max(1);
                // Texels to the unit so that the box passes' Gaussian is `sigma` units.
                let res = ((radius * (radius + 1)) as f32).sqrt() / sigma;
                let pad = (sigma * 2.0).ceil();
                let key = format!("\u{1}softtext {font}|{px_u:.2}|{sigma:.2}|{}", rel.iter().map(|(l, x)| format!("{l}@{x:.1}")).collect::<Vec<_>>().join("|"));
                let w_u = (br - bl).max(1.0);
                let shows = format!("softtext {text}|{font}|{px:.1}|{w_u:.0}");
                if let Some(h) = pictures.kept(&key) {
                    let rect_u = [bl, y0, w_u, bo2.coverage_height(font, px_u) * lines.len() as f32];
                    soft_patch_with(&mut cmds, root, &existing, &mut nodes, &mut image_nodes, d.id, rect_u, scale, color, z, h, pad, shows);
                    continue;
                }
                if let Some((mut cov, tw, th, bh)) = bo2.coverage(font, &rel, px_u, w_u, pad, res) {
                    crate::lui_scene::blur(&mut cov, tw, th, radius);
                    for p in cov.chunks_exact_mut(4) {
                        p[..3].fill(255);
                    }
                    let h = image_assets.add(crate::lui_scene::image(tw, th, cov));
                    pictures.keep(key, &h);
                    let rect_u = [bl, y0, w_u, bh];
                    soft_patch_with(&mut cmds, root, &existing, &mut nodes, &mut image_nodes, d.id, rect_u, scale, color, z, h, pad, shows);
                    continue;
                }
                // (No letters to read: the old patch of the text's extent.)
                let rect_u = [bl, y0 + 0.25 * (y1 - y0), w_u, ((lines.len() as f32 - 1.0) * 1.1 + 0.6) * (y1 - y0)];
                let tint = color.with_alpha(color.alpha() * behind_ink());
                soft_patch(&mut cmds, root, &mut pictures, &mut image_assets, &existing, &mut nodes, &mut image_nodes, d.id, rect_u, scale, tint, z);
                continue;
            }
            let node = Node {
                left: Val::Px(left), top: Val::Px(y0 * scale),
                width: if wrapped { Val::Px(box_w) } else { Val::Auto },
                flex_direction: if has_icons { FlexDirection::Row } else { FlexDirection::Column },
                align_items: match (wrapped, d.alignment) {
                    (true, 2) => AlignItems::Center,
                    (true, 3) => AlignItems::FlexEnd,
                    _ => AlignItems::FlexStart,
                },
                ..absolute_node()
            };
            let shows = format!("text {text}|{font}|{px:.1}|{:.2}|{:.2}|{:.2}|{}|{}", d.rgb[0], d.rgb[1], d.rgb[2], lines.len(), in_match_menu);
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, mut n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                nn.set_if_neq(node);
                zi.set_if_neq(z);
                // A fade: every letter's alpha.
                if (n.alpha - color.alpha()).abs() > 1e-3 {
                    n.alpha = color.alpha();
                    for g in children.iter_descendants(e) {
                        if let Ok(mut img) = image_nodes.get_mut(g) {
                            img.color.set_alpha(color.alpha());
                        }
                    }
                }
                continue;
            }
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                diag::info!(Ui, "menus: words #{} {shows} at {:?} align {} from {left} across {width}", d.id, d.rect, d.alignment);
            }
            if let Some(&e) = existing.get(&d.id) {
                cmds.entity(e).try_despawn();
            }
            let handles: Vec<Option<Handle<Image>>> = pieces
                .iter()
                .map(|p| match p {
                    Piece::Words(_) => None,
                    Piece::Icon(i) => picture(&mut pictures, icons.as_deref(), &mut image_assets, &i.pic_name),
                })
                .collect();
            let e = cmds
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    z,
                ))
                .with_children(|c| {
                    if has_icons {
                        for (p, h) in pieces.iter().zip(&handles) {
                            if let Piece::Words(t) = p {
                                bo2.spawn_line(c, font, &[(t.clone(), color)], px, 0.0);
                                continue;
                            }
                            let (Piece::Icon(i), Some(h)) = (p, h) else {
                                continue;
                            };
                            let tint = Color::linear_rgba(1.0, 1.0, 1.0, color.alpha());
                            let drop = -px * (i.up - 1.0) * 0.5;
                            let size = Node {
                                height: Val::Px(px * i.up),
                                width: Val::Px(px * i.across),
                                margin: UiRect::vertical(Val::Px(drop)),
                                ..default()
                            };
                            c.spawn((size, stretched(h.clone(), tint)));
                        }
                        return;
                    }
                    lines.iter().for_each(|line| {
                        bo2.spawn_line(c, font, &[(line.clone(), color)], px, 0.0);
                    });
                })
                .id();
            cmds.entity(root).add_child(e);
        }
    }
    // bo2mp: the build stamp the engine draws over every front-end screen
    // (not LUI: 43.1736.11, small and white, under the title bar's right end).
    // The engine's own stamp font and size live in the packed game, so
    // there is no data for them: the condensed HUD font (smallFont) at the
    // size that makes the string as wide as real's (40 px at 720p). The
    // normalFont this drew before was no font key at all, fell back to
    // Default, whose "3" overlaps the "6" at this size.
    // Checked 10-08: Condensed's pixelHeight is 25 and script text is drawn at
    // its element's own height (no pixelHeight rule), so no data gives 14.5.
    const STAMP: usize = usize::MAX - 7;
    // The Loading menu is not a front-end screen: the engine draws no stamp
    // over it (real BO2's loading screen has none).
    if front.is_some()
        && !hud.host.open_menus().iter().any(|m| m == "Menu.Loading")
        && let Some(bo2) = fonts.as_deref()
    {
        keep.insert(STAMP);
        if !existing.contains_key(&STAMP) {
            let node = pinned_node(1084.0 * scale, 69.5 * scale);
            let e = cmds
                .spawn((LuiNode { id: STAMP, shows: String::new(), alpha: 1.0 }, node, ZIndex(i32::MAX / 2)))
                .with_children(|c| {
                    bo2.spawn_line(c, font_name("smallFont"), &[("43.1736.11".to_owned(), Color::WHITE)], 14.5 * scale, 0.0);
                })
                .id();
            cmds.entity(root).add_child(e);
        }
    }
    for (id, e) in existing {
        if !keep.contains(&id) {
            cmds.entity(e).try_despawn();
        }
    }
    sub[2] = lap(&mut hud_t);
    spent[4] = sub.iter().sum();
    hud_spent(spent, sub, keep.len(), elements);
}

/// Pictures the HUD made itself, filed by the key that describes them.
#[derive(Default, Clone)]
pub(crate) struct PictureCache(HashMap<String, Handle<Image>>);

impl PictureCache {
    /// The picture kept under `key`, replaced by `made` (one asset rewritten
    /// every frame, not a new one each frame).
    fn refill(&mut self, pics: &mut Assets<Image>, key: &str, made: Image) -> Handle<Image> {
        if let Some(kept) = self.kept(key).filter(|kept| pics.contains(kept)) {
            pics.insert(&kept, made).ok();
            return kept;
        }
        let fresh = pics.add(made);
        self.keep(key.to_owned(), &fresh);
        fresh
    }

    /// The picture filed under this key, if one was made.
    pub(crate) fn kept(&self, key: &str) -> Option<Handle<Image>> {
        self.0.get(key).cloned()
    }

    /// Whether no picture has been filed yet.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a picture is filed under this key.
    pub(crate) fn holds(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// The keys the pictures are filed under.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Files a picture under its key, replacing any older one.
    pub(crate) fn keep(&mut self, filed_as: String, made: &Handle<Image>) {
        self.0.insert(filed_as, made.clone());
    }

    /// The picture filed under this key, made and filed first when there is none.
    pub(crate) fn kept_or_make(&mut self, filed_as: String, make: impl FnOnce() -> Handle<Image>) -> Handle<Image> {
        self.0.entry(filed_as).or_insert_with(make).clone()
    }

    /// Drops every filed picture (the assets themselves stay).
    pub(crate) fn forget_all(&mut self) {
        self.0.clear();
    }
}

/// A box placed in pixels from its parent's top-left corner.
fn px_box(left: f32, top: f32, width: f32, height: f32) -> Node {
    ScreenBox { left_px: left, top_px: top, width_px: width, height_px: height }.node()
}

/// `px_box` that cuts off whatever lies outside it.
fn clipped_box(left: f32, top: f32, width: f32, height: f32) -> Node {
    Node { overflow: Overflow::clip(), ..px_box(left, top, width, height) }
}

/// A picture drawn stretched over its whole box.
fn stretched(pic: Handle<Image>, tint: Color) -> ImageNode {
    ImageNode::new(pic).with_color(tint).with_mode(bevy::ui::widget::NodeImageMode::Stretch)
}

/// The ms since `t`, and `t` moved on to now.
fn lap(t: &mut std::time::Instant) -> f64 {
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    *t = std::time::Instant::now();
    ms
}

/// Where the HUD's time goes, ms a frame over 10 s (logged): reading the
/// game's values, the engine's events to the scripts, his input and the
/// menus' engine calls, the scripts' own frame (timers, animations), and
/// drawing (split: waypoints and minimap, the scripts' tree, the nodes);
/// the pieces drawn and the tree's elements.
fn hud_spent(parts: [f64; 5], sub: [f64; 3], pieces: usize, elem_count: usize) {
    thread_local! {
        static SPENT: std::cell::RefCell<(Option<std::time::Instant>, u32, [f64; 8], usize, usize)> =
            const { std::cell::RefCell::new((None, 0, [0.0; 8], 0, 0)) };
    }
    SPENT.with_borrow_mut(|(since, frames, sum, drawn, all)| {
        let now = <std::time::Instant>::now();
        let start = *since.get_or_insert(now);
        *frames += 1;
        for (s, p) in sum.iter_mut().zip(parts.into_iter().chain(sub)) {
            *s += p;
        }
        *drawn += pieces;
        *all += elem_count;
        if now.duration_since(start).as_secs_f64() >= 10.0 {
            let n = f64::from(*frames);
            diag::info!(
                Ui,
                "match hud (10 s): ms each frame {:.2} (numbers {:.2}, events {:.2}, keys {:.2}, scripts {:.2}, drawing {:.2} = markers {:.2} + tree {:.2} + nodes {:.2}), {:.0} pieces of {:.0} parts",
                sum[..5].iter().sum::<f64>() / n,
                sum[0] / n,
                sum[1] / n,
                sum[2] / n,
                sum[3] / n,
                sum[4] / n,
                sum[5] / n,
                sum[6] / n,
                sum[7] / n,
                *drawn as f64 / n,
                *all as f64 / n
            );
            *since = Some(now);
            *frames = 0;
            *sum = [0.0; 8];
            *drawn = 0;
            *all = 0;
        }
    });
}

/// The marks `plain` puts round a FontIcon entry's name.
const GLYPH_OPEN: char = '\u{E200}';
const GLYPH_CLOSE: char = '\u{E201}';

/// The tab strips' arrows (`^BBUTTON_CYCLE_LEFT^` / `_RIGHT^`).
const ARROW_ICONS: [&str; 2] = ["ui_arrow_left", "ui_arrow_right"];

/// A picture in a line of text: its material and its size as a fraction of
/// the text's height (BO2's FontIcon entry: xScale across, yScale up).
#[derive(Clone)]
struct IconPic {
    pic_name: String,
    across: f32,
    up: f32,
}

enum Piece {
    Words(String),
    Icon(IconPic),
}

/// BO2's icon set (the FontIcon asset's `bo2mp/fonticon.csv`: name, hash,
/// material, size, xScale, yScale): the picture a letter_shape name draws.
fn font_icon(tables: &HashMap<String, Vec<Vec<String>>>, name: &str) -> Option<IconPic> {
    let row = tables.get("bo2mp/fonticon.csv")?.iter().find(|r| r.first().is_some_and(|n| n == name))?;
    let pic_name = row.get(2).filter(|m| !m.is_empty())?.clone();
    let across = row.get(4)?.parse().ok()?;
    let up = row.get(5)?.parse().ok()?;
    Some(IconPic { pic_name, across, up })
}

/// Moves the words gathered so far, if any, out as a piece.
fn flush_words(out: &mut Vec<Piece>, cur: &mut String) {
    if !cur.is_empty() {
        out.push(Piece::Words(std::mem::take(cur)));
    }
}

/// A text's pieces: its words and BO2's pictures (the tab arrows' private-use
/// letters, and a FontIcon entry's name between `GLYPH_OPEN` / `GLYPH_CLOSE`).
fn icon_pieces(text: &str, letter_shape: &dyn Fn(&str) -> Option<IconPic>) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut name: Option<String> = None;
    for c in text.chars() {
        if let Some(n) = name.as_mut() {
            if c == GLYPH_CLOSE {
                if let Some(pic) = letter_shape(n) {
                    flush_words(&mut out, &mut cur);
                    out.push(Piece::Icon(pic));
                }
                name = None;
            } else {
                n.push(c);
            }
            continue;
        }
        if c == GLYPH_OPEN {
            name = Some(String::new());
            continue;
        }
        match (c as u32).checked_sub(0xE100).and_then(|i| ARROW_ICONS.get(i as usize)) {
            Some(m) => {
                flush_words(&mut out, &mut cur);
                out.push(Piece::Icon(IconPic { up: 1.1, across: 0.8, pic_name: (*m).to_owned() }));
            }
            None => cur.push(c),
        }
    }
    flush_words(&mut out, &mut cur);
    out
}

/// A text that is one button letter_shape (`^BBUTTON_CYCLE_LEFT^`) drawn as the
/// picture BO2 PC shows for it (CHOOSE CLASS's class set arrows, the
/// settings tabs' arrows: ui_arrow_left / ui_arrow_right).
fn button_glyph(mut d: Drawn) -> Drawn {
    let picture = match d.text.as_deref().map(str::trim) {
        Some("^BBUTTON_CYCLE_LEFT^") => "ui_arrow_left",
        Some("^BBUTTON_CYCLE_RIGHT^") => "ui_arrow_right",
        Some("^BBUTTON_MOUSE_CLICK^") => "mouse_click",
        Some("^BBUTTON_MOUSE_CLICK_ACTIVE^") => "mouse_click_active",
        Some("^BBUTTON_MOUSE_EDIT^") => "mouse_edit",
        Some("^BBUTTON_MOUSE_EDIT_ACTIVE^") => "mouse_edit_active",
        _ => return d,
    };
    d.element_kind = "image";
    d.material = Some(picture.to_owned());
    d.text = None;
    d.rgb = [1.0, 1.0, 1.0];
    d
}

/// `text` broken at its spaces into lines no wider than `max`, each word
/// measured with the space that follows it, as BO2's text layout counts it
/// (a word longer than that stands alone on its line).
fn wrap_words(bo2: &crate::bo2_font::Bo2Fonts, font: &str, text: &str, px: f32, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let next = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
        if !cur.is_empty() && bo2.line_width(font, &format!("{next} "), px) > max {
            lines.push(std::mem::replace(&mut cur, word.to_owned()));
        } else {
            cur = next;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// What the front end's lobby updates depend on: the open menus and the
/// party (its size, team and mode settings).
fn lobby_sig(host: &Host) -> String {
    let v = host.values.borrow();
    let mut party: Vec<_> = v
        .dvars
        .iter()
        .filter(|(k, _)| k.starts_with("party") || k.starts_with("lobby"))
        .collect();
    // And the game it is set to: the engine sends an update when the map,
    // start or mode changes (Grief picked after its start: the lobby's
    // picture is the start's Grief one, `menu_zm_prison_zencounter_cellblock`).
    let game: Vec<_> = ["ui_mapname", "ui_zm_mapstartlocation", "ui_gametype"]
        .iter()
        .map(|k| hks_t6::host::dvar_get(&v.dvars, k))
        .collect();
    party.sort();
    // And who is in his relay lobby (bo2zm H28).
    format!(
        "{:?}|{party:?}|{game:?}|{}|{:?}|{}|{:?}|{}",
        host.open_menus(),
        v.team,
        v.game_modes,
        v.gamertag,
        v.party,
        v.party_follower
    )
}

/// `text` broken at spaces into lines no wider than `width` pixels (a word
/// wider than that keeps a line to itself).
fn wrap_lines(bo2: &Bo2Fonts, font: &str, text: &str, px: f32, width: f32) -> Vec<String> {
    let (mut lines, mut line): (Vec<String>, String) = Default::default();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && bo2.line_width(font, &candidate, px) > width {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// How dark a popup's blur leaves the menu behind it (0 clear, 1 black).
const BLUR_VEIL: f32 = 0.6;
/// How far a popup's blur smears the menu behind it, in the menus'
/// 1280-wide units.
const BLUR_UNITS: f32 = 14.0;

/// A material's picture smeared as BO2's popup blur smears it, for a box
/// `size` units across: shrunk until one pixel is about a blur's width,
/// softened, and drawn stretched (made once per picture and strength).
/// Returns the image, the name it is kept under and how far its border
/// reaches past the box, as a share of the box.
fn blurred_picture(
    cache: &mut PictureCache,
    icons: Option<&assets::T6HudIcons>,
    pics: &mut Assets<Image>,
    name: &str,
    size: (f32, f32),
) -> Option<(Handle<Image>, String, Vec2)> {
    let img = icons?.0.iter().find(|(k, _)| k == name)?.1.clone();
    let (w, h) = (img.texture_descriptor.size.width as usize, img.texture_descriptor.size.height as usize);
    let f = (blur_step(w, size.0) / 2).max(1);
    let pad = Vec2::new(4.0 / w.div_ceil(f).max(1) as f32, 4.0 / h.div_ceil(f).max(1) as f32);
    let step = blur_step(w, size.0);
    let key = format!("{name}#blur{step}");
    if let Some(h) = cache.kept(&key) {
        return Some((h, key, pad));
    }
    let rgba = picture_rgba(&img)?;
    // Shrink by half the blur's width (alpha-weighted, so see-through
    // pixels do not darken the edges), then three soft passes.
    let (sw, sh) = (w.div_ceil(f).max(1), h.div_ceil(f).max(1));
    let mut small = vec![[0f32; 4]; sw * sh];
    for y in 0..h {
        for x in 0..w {
            let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let a = f32::from(p[3]) / 255.0;
            let c = &mut small[(y / f) * sw + x / f];
            for i in 0..3 {
                c[i] += f32::from(p[i]) / 255.0 * a;
            }
            c[3] += a;
        }
    }
    for (i, c) in small.iter_mut().enumerate() {
        let (sx, sy) = (i % sw, i / sw);
        let n = ((w - sx * f).min(f) * (h - sy * f).min(f)) as f32;
        for v in c.iter_mut() {
            *v /= n;
        }
    }
    // A border the smear spreads into, so a picture's box edge is soft too:
    // see-through, or the edge carried on for a picture solid to its edge
    // (a backdrop does not fade at the screen's edge).
    const PAD: usize = 4;
    let border: Vec<f32> = (0..sw)
        .flat_map(|x| [small[x][3], small[(sh - 1) * sw + x][3]])
        .chain((0..sh).flat_map(|y| [small[y * sw][3], small[y * sw + sw - 1][3]]))
        .collect();
    let solid = border.iter().sum::<f32>() / border.len().max(1) as f32 > 0.95;
    let (iw, ih) = (sw, sh);
    let (sw, sh) = (iw + PAD * 2, ih + PAD * 2);
    let mut small = {
        let inner = small;
        let mut out = vec![[0f32; 4]; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let (ix, iy) = (x as i32 - PAD as i32, y as i32 - PAD as i32);
                let inside = (0..iw as i32).contains(&ix) && (0..ih as i32).contains(&iy);
                if inside || solid {
                    let (cx, cy) = (ix.clamp(0, iw as i32 - 1) as usize, iy.clamp(0, ih as i32 - 1) as usize);
                    out[y * sw + x] = inner[cy * iw + cx];
                }
            }
        }
        out
    };
    let soften = |px: &mut Vec<[f32; 4]>, horizontal: bool| {
        let src = px.clone();
        for y in 0..sh {
            for x in 0..sw {
                let mut acc = [0f32; 4];
                for (d, wt) in [(-1i32, 0.25f32), (0, 0.5), (1, 0.25)] {
                    let (facing_x, facing_y) = if horizontal {
                        ((x as i32 + d).clamp(0, sw as i32 - 1) as usize, y)
                    } else {
                        (x, (y as i32 + d).clamp(0, sh as i32 - 1) as usize)
                    };
                    let s = src[facing_y * sw + facing_x];
                    for i in 0..4 {
                        acc[i] += s[i] * wt;
                    }
                }
                px[y * sw + x] = acc;
            }
        }
    };
    for _ in 0..3 {
        soften(&mut small, true);
        soften(&mut small, false);
    }
    let mut out = vec![0u8; sw * sh * 4];
    for (i, c) in small.iter().enumerate() {
        let a = c[3].max(1e-6);
        for k in 0..3 {
            out[i * 4 + k] = ((c[k] / a).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        out[i * 4 + 3] = (c[3].clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{TextureDimension, TextureFormat};
    // Stored values shown as they are, as `picture` does.
    let small = Image::new(
        crate::lui_scene::picture_size(sw, sh),
        TextureDimension::D2,
        out,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    let h = pics.add(small);
    cache.keep(key.clone(), &h);
    Some((h, key, pad))
}

/// The popup blur's width in a picture's own pixels (`w` wide, drawn
/// `drawn` units across), in steps so a picture that grows or shrinks is
/// not remade every frame.
fn blur_step(w: usize, drawn: f32) -> usize {
    let r = BLUR_UNITS * w as f32 / drawn.max(1.0);
    [1usize, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64]
        .into_iter()
        .min_by(|left, right| (*left as f32 - r).abs().total_cmp(&(*right as f32 - r).abs()))
        .unwrap_or(1)
}

/// A picture's top level as plain RGBA bytes (its stored values).
fn picture_rgba(image: &Image) -> Option<Vec<u8>> {
    use bevy::render::render_resource::TextureFormat;
    let w = image.texture_descriptor.size.width as usize;
    let h = image.texture_descriptor.size.height as usize;
    let data = image.data.as_ref()?;
    let (block, decode): (usize, fn(&[u8], &mut [u8], usize)) = match image.texture_descriptor.format {
        TextureFormat::Bc1RgbaUnormSrgb | TextureFormat::Bc1RgbaUnorm => (8, bcdec_rs::bc1),
        TextureFormat::Bc2RgbaUnormSrgb | TextureFormat::Bc2RgbaUnorm => (16, bcdec_rs::bc2),
        TextureFormat::Bc3RgbaUnormSrgb | TextureFormat::Bc3RgbaUnorm => (16, bcdec_rs::bc3),
        TextureFormat::Bc7RgbaUnormSrgb | TextureFormat::Bc7RgbaUnorm => (16, bcdec_rs::bc7),
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => return data.get(..w * h * 4).map(<[u8]>::to_vec),
        _ => return None,
    };
    let mut rgba = vec![0u8; w * h * 4];
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    // One decoded 4x4 block, RGBA.
    let mut tile: [u8; 64] = [0; 64];
    for by in 0..bh {
        for bx in 0..bw {
            let at = (by * bw + bx) * block;
            decode(data.get(at..at + block)?, &mut tile, 16);
            for y in 0..4 {
                for x in 0..4 {
                    let (px, py) = (bx * 4 + x, by * 4 + y);
                    if px < w && py < h {
                        let d = (py * w + px) * 4;
                        let s = (y * 4 + x) * 4;
                        rgba[d..d + 4].copy_from_slice(&tile[s..s + 4]);
                    }
                }
            }
        }
    }
    Some(rgba)
}

/// A material's picture as an image handle (made once).
/// bo2mp: how much of a covered menu's pictures show before the blur's dim
/// falls on them (they are drawn as written: the dim is the one rule);
/// `BO2MP_BEHIND_DIM`.
fn behind_dim() -> f32 {
    std::env::var("BO2MP_BEHIND_DIM").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

/// bo2mp: the blur's sigma in screen units (the front end is 720 high). Black
/// Ops 1's engine (BO2's parent) blurs the menu layer by
/// `ui_blurAmount * intensity` virtual units of a 480-high screen (default
/// 4.0), and runs its Gaussian twice, so the two passes add in quadrature:
/// sigma = sqrt(2) * 4 * 720 / 480 = 8.5 units at full intensity.
/// `BO2MP_BEHIND_BLUR` overrides it (units).
fn blur_sigma() -> f32 {
    std::env::var("BO2MP_BEHIND_BLUR").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0f32.sqrt() * 4.0 * 720.0 / 480.0)
}

/// bo2mp: what the engine's UI blur leaves of the covered stack, per screen
/// position. BO2's blur material `ui_blur` (zone `code_post_gfx_mp`,
/// techniqueset `sw4_2d_ui_mp_blur`) holds `Radius` 0.42 and `Hardness` 1.0,
/// and its pixel shader (disassembled) computes, with `I` the blur
/// intensity, `D` the dvar `ui_blurDarkenAmount`, `d2` the squared distance
/// of the pixel's screen position from the screen's middle (both axes 0 to 1):
///   out = blurred * (1 - I * d2^Hardness / (1 - (1 - Radius) * I^Hardness)) * (1 - D * I)
/// so the middle keeps `1 - D*I` of its level and the corners fall off. The
/// blend there is replace, so the factor multiplies the whole covered stack.
const BLUR_RADIUS: f32 = 0.42;
const BLUR_HARDNESS: f32 = 1.0;
/// `ui_blurDarkenAmount`. The engine registers its default in code (not in the
/// zones, the scripts or the cfgs; the executable is packed), so this one
/// number is MEASURED, not read: BO2's own Quit shot, blurred main menu over
/// the unblurred one, reads 0.50 at the middle of the screen (Black Ops 1's
/// default is 0.28). `BO2MP_BLUR_DARKEN` overrides it.
fn blur_darken() -> f32 {
    std::env::var("BO2MP_BLUR_DARKEN").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

/// bo2mp: the black picture whose alpha is one minus the blur shader's factor
/// (see `BLUR_RADIUS`) at intensity `i`; 128 x 72 texels over the whole screen.
fn blur_vignette(i: f32) -> Image {
    let (w, h) = (128usize, 72usize);
    let mut px = vec![0u8; w * h * 4];
    let d = blur_darken();
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let d2 = (0.5 - u).powi(2) + (0.5 - v).powi(2);
            let vig = (1.0 - i * d2.powf(BLUR_HARDNESS) / (1.0 - (1.0 - BLUR_RADIUS) * i.powf(BLUR_HARDNESS))).max(0.0);
            let f = (vig * (1.0 - d * i)).clamp(0.0, 1.0);
            px[(y * w + x) * 4 + 3] = ((1.0 - f) * 255.0).round() as u8;
        }
    }
    crate::lui_scene::image(w, h, px)
}

/// bo2mp: blur the backdrop picture under a blurring menu (`BO2MP_BLUR_BKG=0` turns it off).
fn blur_bkg() -> bool {
    std::env::var("BO2MP_BLUR_BKG").map(|v| v != "0").unwrap_or(true)
}

/// bo2mp: the share of a covered menu's text box that shows as ink once it is
/// blurred away (the sheet's letters cover about a third of their box);
/// `BO2MP_BEHIND_INK`.
fn behind_ink() -> f32 {
    std::env::var("BO2MP_BEHIND_INK").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

/// bo2mp: a white rectangle of `w` x `h` screen units blurred as the covered
/// menu's pictures are (`BO2MP_BEHIND_BLUR` units), padded so the blur has
/// room: BO2's blur of a text or a block that is not a picture. Returns the
/// picture and the padding (units). Two texels to the unit.
fn soft_rect(cache: &mut PictureCache, pics: &mut Assets<Image>, w: f32, h: f32) -> (Handle<Image>, f32) {
    let screen: f32 = blur_sigma();
    let pad = (screen * 2.0).ceil();
    let (tw, th) = (((w + 2.0 * pad) * 0.5).ceil().max(1.0) as usize, ((h + 2.0 * pad) * 0.5).ceil().max(1.0) as usize);
    let key = format!("\u{1}soft {tw}x{th} {screen}");
    if let Some(k) = cache.kept(&key) {
        return (k, pad);
    }
    let mut px = vec![0u8; tw * th * 4];
    let (x0, y0) = ((pad * 0.5) as usize, (pad * 0.5) as usize);
    let (x1, y1) = ((x0 + (w * 0.5).ceil() as usize).min(tw), (y0 + (h * 0.5).ceil() as usize).min(th));
    for y in y0..y1 {
        for x in x0..x1 {
            px[(y * tw + x) * 4..(y * tw + x) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    crate::lui_scene::blur(&mut px, tw, th, ((screen * 0.5).round() as usize).max(1));
    for p in px.chunks_exact_mut(4) {
        p[..3].fill(255);
    }
    let handle = pics.add(crate::lui_scene::image(tw, th, px));
    cache.keep(key, &handle);
    (handle, pad)
}

/// bo2mp: draw `rect` (x, y, w, h in units) as a soft patch tinted `tint`,
/// keeping the element's node between frames.
#[allow(clippy::too_many_arguments)]
fn soft_patch(
    spawner: &mut Commands, parent: Entity, cache: &mut PictureCache,
    pics: &mut Assets<Image>, existing: &HashMap<usize, Entity>,
    nodes: &mut Query<(Entity, &mut LuiNode, &mut Node, &mut ZIndex)>,
    img_nodes: &mut Query<&mut ImageNode>, id: usize, rect: [f32; 4], scale: f32, tint: Color, z: ZIndex,
) {
    let (pic, pad) = soft_rect(cache, pics, rect[2].max(1.0), rect[3].max(1.0));
    let shows = format!("soft {:.0}x{:.0}", rect[2], rect[3]);
    soft_patch_with(spawner, parent, existing, nodes, img_nodes, id, rect, scale, tint, z, pic, pad, shows);
}

/// bo2mp: `soft_patch` with its blurred picture (`handle`, clear border
/// `pad` units) already made; `shows` names what it draws.
#[allow(clippy::too_many_arguments)]
fn soft_patch_with(
    cmds: &mut Commands, parent: Entity, existing: &HashMap<usize, Entity>,
    nodes: &mut Query<(Entity, &mut LuiNode, &mut Node, &mut ZIndex)>,
    image_nodes: &mut Query<&mut ImageNode>, id: usize, rect: [f32; 4], scale: f32, shade: Color, z: ZIndex,
    picture: Handle<Image>, pad: f32, shows: String,
) {
    let node = px_box(
        (rect[0] - pad) * scale,
        (rect[1] - pad) * scale,
        (rect[2].max(1.0) + 2.0 * pad) * scale,
        (rect[3].max(1.0) + 2.0 * pad) * scale,
    );
    if let Some(&e) = existing.get(&id)
        && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
        && n.shows == shows
    {
        nn.set_if_neq(node);
        zi.set_if_neq(z);
        if let Ok(mut img) = image_nodes.get_mut(e)
            && img.color != shade
        {
            img.color = shade;
        }
        return;
    }
    if let Some(&e) = existing.get(&id) {
        cmds.entity(e).try_despawn();
    }
    let e = cmds
        .spawn((
            LuiNode { id, shows, alpha: shade.alpha() },
            node,
            stretched(picture, shade),
            z,
        ))
        .id();
    cmds.entity(parent).add_child(e);
}

/// bo2mp: `name`'s picture blurred (premultiplied, so its clear edges do not
/// darken), for the covered main menu: about `BO2MP_BEHIND_BLUR` screen units
/// of blur over a `rect_w`-wide rectangle. With `pad` the picture gets a
/// clear border of twice the blur radius first, so its glow spills past its
/// rectangle as BO2's blur of the whole screen does (the BLACK OPS logo is a
/// soft glow beyond its box, not a cut-off block); returns the picture and
/// the border as a share of its width and height.
fn behind_picture(
    cache: &mut PictureCache, icons: Option<&assets::T6HudIcons>,
    pics: &mut Assets<Image>, name: &str, box_w: f32, pad: bool,
) -> Option<(Handle<Image>, f32, f32)> {
    use bevy::render::render_resource::TextureFormat;
    // (The pixels come from the loaded picture, not from the uploaded copy:
    // the GPU copy has given its data up.)
    let Some(src) = icons.and_then(|i| i.0.iter().find(|(k, _)| k == name).map(|(_, i)| i.clone())) else {
        return picture(cache, icons, pics, name).map(|h| (h, 0.0, 0.0));
    };
    let w = src.width() as usize;
    let screen: f32 = blur_sigma();
    let radius = ((screen * w as f32 / box_w.max(1.0)).round() as usize).clamp(1, 64);
    let key = format!("{name}|behind{radius}{}", if pad { "|pad" } else { "" });
    let border = if pad { radius * 2 } else { 0 };
    let shares = (border as f32 / w.max(1) as f32, border as f32 / (src.height() as usize).max(1) as f32);
    if let Some(k) = cache.kept(&key) {
        return Some((k, shares.0, shares.1));
    }
    let mut out = if matches!(src.texture_descriptor.format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb) {
        (*src).clone()
    } else if let Some(dec) = assets::plain_rgba8(&src) {
        dec
    } else {
        return picture(cache, icons, pics, name).map(|h| (h, 0.0, 0.0));
    };
    raw_texels(&mut out);
    // Only its top level: the blur and the clear border are made on that, and
    // a mip chain left on a grown picture no longer matches its size (wgpu
    // panicked on the Barracks' Prestige card behind the ConfirmPrestige
    // popup: 61680 bytes wanted, 49344 given).
    let top = out.width() as usize * out.height() as usize * 4;
    if let Some(d) = out.data.as_mut() {
        d.truncate(top);
    }
    out.texture_descriptor.mip_level_count = 1;
    let shares = if border > 0 && pad_clear(&mut out, border) { shares } else { (0.0, 0.0) };
    out.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });
    let (w, h) = (out.width() as usize, out.height() as usize);
    let plain = matches!(out.texture_descriptor.format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb);
    if plain && let Some(px) = out.data.as_mut().filter(|p| p.len() >= w * h * 4) {
        for p in px.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            for c in &mut p[..3] {
                *c = (u32::from(*c) * a / 255) as u8;
            }
        }
        crate::lui_scene::blur(px, w, h, radius);
        for p in px.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            if a > 0 {
                for c in &mut p[..3] {
                    *c = (u32::from(*c) * 255 / a).min(255) as u8;
                }
            }
        }
    }
    let handle = pics.add(out);
    cache.keep(key, &handle);
    Some((handle, shares.0, shares.1))
}

/// bo2mp: `img` (plain RGBA8) grown by a clear border of `border` texels on
/// every side. False (untouched) when it is not plain RGBA8.
fn pad_clear(img: &mut Image, border: usize) -> bool {
    use bevy::render::render_resource::TextureFormat;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if !matches!(img.texture_descriptor.format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb) {
        return false;
    }
    let Some(src) = img.data.as_ref().filter(|p| p.len() >= w * h * 4) else { return false };
    let (nw, nh) = (w + 2 * border, h + 2 * border);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..h {
        let (a, b) = (y * w * 4, ((y + border) * nw + border) * 4);
        out[b..b + w * 4].copy_from_slice(&src[a..a + w * 4]);
    }
    img.data = Some(out);
    img.texture_descriptor.size = crate::lui_scene::picture_size(nw, nh);
    true
}

/// bo2mp: a BC3 picture's alpha raised to the 2.2 power (every 16-byte block
/// is independent, so the whole mip chain goes through one loop).
fn gamma_alpha_bc3(img: &mut Image) {
    use bevy::render::render_resource::TextureFormat;
    let fmt = img.texture_descriptor.format;
    let Some(data) = img.data.as_mut() else { return };
    let f = |a: u8| -> u8 { ((f32::from(a) / 255.0).powf(2.2) * 255.0).round() as u8 };
    match fmt {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => {
            for px in data.chunks_exact_mut(4) {
                px[3] = f(px[3]);
            }
            return;
        }
        // BC2: sixteen 4-bit alphas, then the colour block.
        TextureFormat::Bc2RgbaUnorm | TextureFormat::Bc2RgbaUnormSrgb => {
            for blk in data.chunks_exact_mut(16) {
                for b in &mut blk[..8] {
                    let lo = f((*b & 15) * 17) / 17;
                    let hi = f((*b >> 4) * 17) / 17;
                    *b = lo.min(15) | (hi.min(15) << 4);
                }
            }
            return;
        }
        TextureFormat::Bc3RgbaUnorm | TextureFormat::Bc3RgbaUnormSrgb => {}
        other => {
            diag::info!(Ui, "menus: picture alpha curve not done, format {other:?}");
            return;
        }
    }
    for blk in data.chunks_exact_mut(16) {
        let (a0, a1) = (blk[0], blk[1]);
        let mut pal = [0u8; 8];
        pal[0] = a0;
        pal[1] = a1;
        let (p0, p1) = (u32::from(a0), u32::from(a1));
        if a0 > a1 {
            for k in 1..7u32 {
                pal[(k + 1) as usize] = (((7 - k) * p0 + k * p1) / 7) as u8;
            }
        } else {
            for k in 1..5u32 {
                pal[(k + 1) as usize] = (((5 - k) * p0 + k * p1) / 5) as u8;
            }
            pal[6] = 0;
            pal[7] = 255;
        }
        let bits = blk[2..8].iter().zip(0..).fold(0u64, |acc, (b, i)| acc | (u64::from(*b) << (8 * i)));
        let mut vals = [0u8; 16];
        for (i, v) in vals.iter_mut().enumerate() {
            *v = f(pal[((bits >> (3 * i)) & 7) as usize]);
        }
        let (lo, hi) = (*vals.iter().min().unwrap_or(&0), *vals.iter().max().unwrap_or(&0));
        // 8-value mode: a0 = hi > a1 = lo; a flat block keeps a0 == a1 (index 0).
        let q: [u32; 8] = std::array::from_fn(|k| {
            let k = k as u32;
            match k {
                0 => u32::from(hi),
                1 => u32::from(lo),
                _ => ((8 - k) * u32::from(hi) + (k - 1) * u32::from(lo)) / 7,
            }
        });
        let mut nb = 0u64;
        for (i, v) in vals.iter().enumerate() {
            let best = (0..8).min_by_key(|k| (q[*k] as i32 - i32::from(*v)).abs()).unwrap_or(0);
            nb |= (best as u64) << (3 * i);
        }
        blk[0] = hi;
        blk[1] = lo;
        for i in 0..6 {
            blk[2 + i] = ((nb >> (8 * i)) & 0xff) as u8;
        }
    }
}


fn picture(
    cache: &mut PictureCache, icons: Option<&assets::T6HudIcons>,
    pics: &mut Assets<Image>, name: &str,
) -> Option<Handle<Image>> {
    // bo2mp: in an in-match menu BO2 blends in the target's gamma space, so
    // a picture's texels show darker than the front end's raw read (the
    // class screen's perk icons: real = ours^2.2). Such a picture keeps its
    // sRGB read, cached under its own key.
    let srgb = IN_MATCH_MENU_PICS.load(std::sync::atomic::Ordering::Relaxed)
        && !matches!(name, "white" | "black")
        && !name.starts_with("menu_mp_popup")
        && !name.starts_with("menu_mp_dots");
    // bo2mp: a card background (a calling card) in the front end is read
    // raw like every other front-end picture; only the match HUD's name
    // card keeps its sRGB read (see below).
    let front_card = (IN_FRONT_END_PICS.load(std::sync::atomic::Ordering::Relaxed) || UI_ENCODED.load(std::sync::atomic::Ordering::Relaxed))
        && name.starts_with("emblem_bg");
    let key: std::borrow::Cow<str> = if srgb {
        format!("{name}|srgb").into()
    } else if front_card {
        format!("{name}|front").into()
    } else {
        name.into()
    };
    if let Some(h) = cache.kept(key.as_ref()) {
        return Some(h);
    }
    // The engine draws a team's emblem into a picture; the Solo team's is
    // the green shield with the BO2 mark (made here, no material holds it).
    if name == LEAGUE_EMBLEM_SOLO {
        let h = pics.add(league_emblem_solo());
        cache.keep(key.into_owned(), &h);
        return Some(h);
    }
    let Some(img) = icons?.0.iter().find(|(k, _)| k == name).map(|(_, i)| i.clone()) else {
        // (Once per name: a picture the scripts name that was not loaded.)
        static MISSING: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
        if MISSING.lock().is_ok_and(|mut m| m.insert(name.to_owned())) {
            diag::info!(Ui, "menus: picture `{name}` is not loaded");
        }
        return None;
    };
    // Clamped at its edges: a picture is drawn once over its element, so a
    // repeating sampler only bleeds the far edge in (a dark line along the
    // bottom of the loading screen's `gradient_top`).
    let mut img = (*img).clone();
    // (bo2mp: a card's background keeps its sRGB read, or it shows far too
    // bright over the world.)
    if (!name.starts_with("emblem_bg") || front_card) && !srgb {
        raw_texels(&mut img);
    }
    // bo2mp: BO2 blends in gamma space in an in-match menu, so a picture's
    // see-through texels (a weapon's soft shadow) show as texel * alpha;
    // this linear blend would show them brighter (texel * alpha^(1/2.2)).
    // The alpha takes the same 2.2 power the element colours' alpha takes.
    if srgb {
        gamma_alpha_bc3(&mut img);
    }
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });
    let h = pics.add(img);
    cache.keep(key.into_owned(), &h);
    Some(h)
}

/// The Solo team's emblem picture (see `picture`).
pub(crate) const LEAGUE_EMBLEM_SOLO: &str = "\u{1}bo2mp league emblem solo";

/// The Barracks' League Teams card art for the Solo team: a green shield
/// (outline, half disc, the two bars of the BO2 mark) on nothing, centred in
/// a picture as wide as the card (260 x 172).
fn league_emblem_solo() -> Image {
    use bevy::render::render_resource::{TextureDimension, TextureFormat};
    // Shield drawn in a 640 x 680 box (x, y): its outer edge and inner edge.
    const OUTER: [(f32, f32); 11] = [
        (105.0, 65.0), (550.0, 65.0), (570.0, 170.0), (520.0, 222.0), (560.0, 375.0), (540.0, 478.0),
        (325.0, 625.0), (115.0, 480.0), (95.0, 375.0), (135.0, 222.0), (80.0, 160.0),
    ];
    const INNER: [(f32, f32); 11] = [
        (125.0, 90.0), (528.0, 90.0), (545.0, 172.0), (497.0, 226.0), (535.0, 378.0), (518.0, 458.0),
        (325.0, 585.0), (136.0, 458.0), (118.0, 378.0), (158.0, 226.0), (104.0, 170.0),
    ];
    fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
        let mut c = false;
        let mut j = poly.len() - 1;
        for i in 0..poly.len() {
            let (xi, yi) = poly[i];
            let (xj, yj) = poly[j];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                c = !c;
            }
            j = i;
        }
        c
    }
    let (w, h) = (256usize, 262usize);
    // The card draws it in a square of about 116 x 119 (the picture is
    // stretched to it): 640 x 680 box -> 288 x 306 px, centred (its drawn part is narrower).
    let scale = 306.0 / 680.0;
    let (ox, oy) = (w as f32 / 2.0 - 320.0 * scale, h as f32 / 2.0 - 340.0 * scale);
    let green = [107u8, 145, 14];
    let ss = 3usize;
    let mut data = vec![0u8; w * h * 4];
    for py in 0..h {
        for px in 0..w {
            let mut hit = 0usize;
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = (px as f32 + (sx as f32 + 0.5) / ss as f32 - ox) / scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / ss as f32 - oy) / scale;
                    let in_outer = inside(&OUTER, x, y);
                    let in_inner = inside(&INNER, x, y);
                    let disc = (x - 322.0).powi(2) + (y - 465.0).powi(2) < 180.0f32.powi(2);
                    let bar = (y > 342.0 && y < 502.0) && ((x > 278.0 && x < 320.0) || (x > 326.0 && x < 372.0));
                    if (in_outer && !in_inner) || (in_inner && disc && !bar) {
                        hit += 1;
                    }
                }
            }
            if hit > 0 {
                let i = (py * w + px) * 4;
                data[i..i + 3].copy_from_slice(&green);
                data[i + 3] = (hit * 255 / (ss * ss)) as u8;
            }
        }
    }
    Image::new(
        crate::lui_scene::picture_size(w, h),
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        Default::default(),
    )
}

/// Set each frame: the front end is the one drawing (see `picture`).
static IN_FRONT_END_PICS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set each frame: the match UI is drawn onto the world camera, whose target
/// holds encoded values (pictures and colours go in as written).
static UI_ENCODED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static UI_ENCODED_LAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set each frame: a multiplayer in-match menu or the match HUD is up (see `picture`).
static IN_MATCH_MENU_PICS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Read a picture's texels as stored, as the scripts' colours are (BO2's UI
/// writes both as they are): an sRGB format would turn the texels linear
/// first and every picture would show darker than BO2's (the main menu's
/// soldiers at a third of their brightness; the lobby's backdrop black).
pub(crate) fn raw_texels(img: &mut Image) {
    use bevy::render::render_resource::TextureFormat;
    let f = &mut img.texture_descriptor.format;
    *f = match *f {
        TextureFormat::Rgba8UnormSrgb => TextureFormat::Rgba8Unorm,
        TextureFormat::Bgra8UnormSrgb => TextureFormat::Bgra8Unorm,
        TextureFormat::Bc1RgbaUnormSrgb => TextureFormat::Bc1RgbaUnorm,
        TextureFormat::Bc2RgbaUnormSrgb => TextureFormat::Bc2RgbaUnorm,
        TextureFormat::Bc3RgbaUnormSrgb => TextureFormat::Bc3RgbaUnorm,
        TextureFormat::Bc7RgbaUnormSrgb => TextureFormat::Bc7RgbaUnorm,
        rest => rest,
    };
}

pub(crate) fn register_lui_hud_systems(app: &mut App) {
    app.insert_non_send(LuiHudSlot::default());
    crate::lui_additive::attach(app);
    crate::lui_scene::attach(app);
    crate::every_ui_frame(app, lui_hud.after(crate::bo2_font::load_bo2_fonts));
    crate::every_ui_frame(app, crate::lui_frontend::restore_after_match);
}

/// The front end's loading screen is up but the map behind the menus is
/// in: the screen is back on the menus (a Start Match load goes on into the
/// game instead, with the game's HUD).
fn front_end_backdrop_loaded(frontend: bool, loading_open: bool, screen: frame::ShellScreen) -> bool {
    frontend && loading_open && screen == frame::ShellScreen::MainMenu
}

/// A map's loading picture (`loadscreen_zm_nuked_zstandard_nuked`), kept
/// off the backdrop map's loading screen.
fn is_map_loading_picture(material: &str) -> bool {
    material.starts_with("loadscreen_")
}

#[cfg(test)]
mod other_front_end_tests {
    use super::OtherFrontEnd;

    #[test]
    fn each_main_menu_opens_the_other_modes_menus() {
        let zm = OtherFrontEnd::from_command("startZombies");
        let mp = OtherFrontEnd::from_command("startMultiplayer");
        assert_eq!(zm.map(OtherFrontEnd::launch_args), Some(["frontend", "t6:zm_nuked"]));
        assert_eq!(mp.map(OtherFrontEnd::launch_args), Some(["frontend", "t6mp"]));
        assert_eq!(OtherFrontEnd::from_command("startSingleplayer"), None);
    }
}

#[cfg(test)]
mod backdrop_loading_tests {
    use super::{front_end_backdrop_loaded, is_map_loading_picture};

    #[test]
    fn the_backdrop_loading_screen_shows_no_map_art() {
        assert!(is_map_loading_picture("loadscreen_zm_nuked_zstandard_nuked"));
        assert!(is_map_loading_picture("loadscreen_zm_transit"));
        // The spinner and the menu's own pictures stay.
        assert!(!is_map_loading_picture("lui_loader"));
        assert!(!is_map_loading_picture("lui_bkg_zm"));
        assert!(!is_map_loading_picture("black"));
    }
    use frame::ShellScreen;

    #[test]
    fn the_loading_screen_leaves_once_the_backdrop_is_in() {
        // The backdrop load ends on the menus: the loading screen goes.
        assert!(front_end_backdrop_loaded(true, true, ShellScreen::MainMenu));
        // Still loading, or no loading screen up: nothing to close.
        assert!(!front_end_backdrop_loaded(true, true, ShellScreen::Loading));
        assert!(!front_end_backdrop_loaded(true, false, ShellScreen::MainMenu));
        // A match's HUD (not the front end) keeps its own loading screen.
        assert!(!front_end_backdrop_loaded(false, true, ShellScreen::MainMenu));
    }
}

#[cfg(test)]
mod fire_material_tests {
    use super::fire_material;

    #[test]
    fn the_hud_picture_follows_the_weapons_own_fire_type() {
        assert_eq!(fire_material(0, false), "hud_mp_firerate_auto");
        assert_eq!(fire_material(7, false), "hud_mp_firerate_auto");
        assert_eq!(fire_material(9, false), "hud_mp_firerate_auto");
        assert_eq!(fire_material(1, false), "hud_mp_firerate_single");
        assert_eq!(fire_material(1, true), "hud_mp_firerate_bolt");
        assert_eq!(fire_material(6, false), "hud_mp_firerate_single");
        assert_eq!(fire_material(3, false), "hud_mp_firerate_burst");
        assert_eq!(fire_material(5, false), "hud_mp_firerate_burst");
        assert_eq!(fire_material(8, false), "hud_mp_firerate_burst");
    }
}

#[cfg(test)]
mod public_lobby_tests {
    use super::left_public_lobby;

    #[test]
    fn leaving_the_public_lobby_stops_its_countdown() {
        let mut m = hks_t6::mp::MpState::default();
        m.playlist = Some(0);
        m.public_start_ms = Some(25_000.0);
        // In the public lobby: its own commands keep the countdown.
        assert!(!left_public_lobby("xpartyveto", &mut m));
        assert_eq!(m.public_start_ms, Some(25_000.0));
        // Back to a private party (Custom Games): no countdown, so its
        // match starts only on START MATCH, with the map set then.
        assert!(left_public_lobby("xstartprivateparty", &mut m));
        assert_eq!((m.public_start_ms, m.public_start_in), (None, None));
        // The countdown waiting for the front end's first frame goes too.
        m.public_start_in = Some(25_000.0);
        assert!(left_public_lobby("xstopparty", &mut m));
        assert_eq!(m.public_start_in, None);
        assert!(!left_public_lobby("xstopparty", &mut m));
    }
}
