// Interface crate: screens, layers, class selection and the in-game HUD.

// Public modules.
pub mod classes;
pub mod menu_flow;
pub mod lui_omni_row; // bo2zm: Controls > Move "Omni Movement" (10-10)

// Shell, loading and status pages.
mod cheat_menu; // bo2zm: the solo cheat menu (Num 1)
mod frame_rate; // bo2zm: Draw FPS, Max FPS
mod status_page;
mod boot_digest;
mod tiers;
mod load_table;
mod load_splash;
mod menu;
mod map_entry;
mod window_prefs;
mod ui_root;
mod screen_tiers;
mod screen_box;

// In-game widgets and the LUI front end.
mod bo2_font;
mod bo2_globe;
mod bo2mp_damage; // bo2mp lane C
mod bo2mp_grenade; // bo2mp: the grenade warning
mod lui_additive; // bo2mp scope: true additive HUD pictures
mod lui_frontend; // bo2mp lane C
mod lui_hud; // bo2zm M4
mod lui_scene; // bo2mp menus: the front end's 3D backdrop
mod name_tags; // bo2mp: names over heads in BO2's font
mod pickup_hint; // bo2mp: "Press F to swap for" a gun on the ground
mod stuck_pulse; // bo2mp: an enemy semtex or bolt stuck to him beats on his screen
mod rcxd_hint; // bo2mp RC-XD
mod rcxd_screen; // bo2mp RC-XD
mod scope_hint; // bo2mp scope lane
mod target_highlights; // bo2mp Dragonfire
mod zm_hud;

pub use classes::equip_txn::{
    LoadoutArmTracker, enact_queued_loadout_arm, settle_loadout_arm_deal,
    refresh_loadout_swap_permit,
};
pub use classes::icons::{
    LoadoutPickerThumbs, MenuMediaAnchor, loadout_addon_thumb, loadout_surface_pack_stem,
    loadout_gun_thumb, display_gun_title,
};
// What the console's class commands drive.
pub use classes::select::{
    LoadoutSwapPermit, LoadoutSwapDenial, LoadoutArmRejection, LoadoutPickerFocus, LoadoutPickerStage,
    LoadoutPickerReport, QueuedLoadoutArm, loadout_slot_named, finalize_loadout_arm,
};
pub use classes::setup::{LoadoutEditorLine, LoadoutRoster, ClassPickerFolder, LoadoutSlotStatus};
pub use classes::store::MatchLoadoutVault;
pub use frame::{ShellScreen, KitTemplate, StartupPersona, StartupReport, showcase_classes};
pub use status_page::InterimOverlay;
pub use boot_digest::share_interim_overlay;
pub use name_tags::{OverheadNameRow, OverheadNameRows};
pub use tiers::{
    OverlayTierPass, MenuTypeface, UiCamera, MenuPaintPass, OverlayTier, OverlayStackShown, OverlayTierSet,
    match_typeface,
};
pub use load_splash::{LoadCompletion, LoadBackdropOrigin, LoadBackdrop};
pub use lui_additive::AdditiveUi;
pub use lui_frontend::{Bo2mpFrontend, Bo2mpFrontendAssets};
pub use menu::ScreenArenaRoster;
pub use window_prefs::{KeymapSnapshot, SwapChainPacingChoice};
pub use ui_root::MenuHudPlugin;
pub use screen_tiers::{refresh_overlay_tiers, tiers_for_screen};
pub(crate) use screen_box::{
    ActiveView, CompositedWorldCameras, MainWindow, MainWindowMut, SceneCameras, ScreenBox, absolute_node, covering_node,
    every_ui_frame, pinned_node,
};

/// The HUD picture filed under this material name, once the packs are in.
pub(crate) fn hud_picture<'a>(icons: &'a assets::T6HudIcons, name: &str) -> Option<&'a std::sync::Arc<bevy::image::Image>> {
    icons.0.iter().find_map(|(material, image)| (material == name).then_some(image))
}
