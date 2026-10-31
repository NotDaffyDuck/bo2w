//! Frame ordering sets, session identity keys and the shared UI/settings
//! contracts. Plain data and set declarations; no systems.

pub mod class_presets;
pub mod pad;
pub mod retire;
mod schedule;
mod script_entity_notify;
mod script_notify;
mod run_state;
mod settings;
pub mod ui;

pub use class_presets::KitTemplate;
pub use class_presets::pick_showcase;
pub use class_presets::register_showcase;
pub use class_presets::showcase_classes;
pub use pad::{
    ActivePad, BO2_XBOX_GLYPHS, InputDevices, PromptStyle, TestControllerRumble, bo2_cycle_glyph,
    bo2_frontend, bo2_frontend_waits_on_menu, bo2_game, bo2_pad_glyph, bo2_zm_choice, bo2_zm_start, set_bo2_frontend, set_bo2_menu_started,
    set_bo2_game, set_bo2_zm_start,
};
pub use retire::Retiring;
pub use schedule::SERVER_STAGES;
pub use schedule::ServerLedgerMark;
pub use schedule::ServerHandoffLink;
pub use schedule::ServerStage;
pub use schedule::CLIENT_STAGES;
pub use schedule::KitEquipDone;
pub use schedule::PlayerHandoffLink;
pub use schedule::ClientStep;
pub use schedule::EffectAudioPosted;
pub use schedule::LifeStatePosted;
pub use schedule::ModelLightPlaced;
pub use schedule::ShownStatePosted;
pub use schedule::DrawPassStage;
pub use schedule::MatchSwapDone;
pub use schedule::WORKER_CMD_AFTER;
pub use schedule::JOB_QUEUE_TAIL_BARRIER;
pub use schedule::WORKER_CMD_NOT_RENDER_THREAD;
pub use schedule::WORKER_CMD_RETAIL_NAMES;
pub use schedule::WORKER_CMD_TOC;
pub use schedule::JobQueueStage;
pub use schedule::server_stage_name;
pub use schedule::client_stage_name;
pub use schedule::render_set_name;
pub use schedule::order_server_side_stages;
pub use schedule::order_player_side_stages;
pub use schedule::order_draw_stages;
pub use schedule::order_job_queue_stages;
pub use schedule::worker_cmd_name;
pub use script_entity_notify::AbortKillcam;
pub use script_entity_notify::BeginKillcam;
pub use script_entity_notify::KillReplayDone;
pub use script_entity_notify::SpawnedPlayer;
pub use script_notify::EndBoardShown;
pub use script_notify::LevelQuitRequested;
pub use script_notify::hook_script_signal;
pub use run_state::EntryPassToken;
pub use run_state::ShellScreen;
pub use run_state::BotNavigationReady;
pub use run_state::CustomClassGunChoice;
pub use run_state::KitPickerPassover;
pub use run_state::LevelPresent;
pub use run_state::Headless;
pub use run_state::ServerKitRoster;
pub use run_state::ServerKitSlot;
pub use run_state::OverlayControlSnapshot;
pub use run_state::StartupPersona;
pub use run_state::StartupReport;
pub use run_state::DeathCircumstance;
pub use run_state::SpawnLifeOver;
pub use run_state::SpawnCause;
pub use run_state::SpawnLifeBegun;
pub use run_state::OwnLevelLoadTicket;
pub use run_state::LevelOpenGranted;
pub use run_state::LevelOpenRejected;
pub use run_state::GameSessionArmed;
pub use run_state::GameSessionStamp;
pub use run_state::GameSessionClosed;
pub use run_state::ReturnedToMenu;
pub use run_state::ProcessDuty;
pub use run_state::ShutdownCause;
pub use run_state::UiCamera;
pub use run_state::MenuPaintPass;
pub use run_state::EyeSubject;
pub use run_state::LevelGeneration;
pub use run_state::WorldProducts;
pub use settings::ScreenPixelSize;
pub use settings::PlayerPrefsBundle;
pub use ui::{
    GamePaused, HostMatchRules, LuiMenus, MenuDims, ScriptScreenFilter, UiBindRequest,
    UiBindingCapture, UiExecCommand, UiMenuDvars, UiPartyState, MenuSongStart, MenuCueStart,
    MenuSongHalt, WorldBlur, WorldDim, register_ui_contracts,
};
