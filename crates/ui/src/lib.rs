// Interface crate: screens, layers, class selection and the in-game HUD.

// Public modules.
pub mod classes;
pub mod frontend;
pub mod lui_omni_row; // bo2zm: Controls > Move "Omni Movement" (10-10)

// Shell, loading and status pages.
mod gap_hud;
mod launch_report;
mod layers;
mod load_table;
mod loading;
mod menu;
mod menu_load;
mod options;
mod plugin;
mod screen;

// In-game widgets and the LUI front end.
mod bo2_font;
mod bo2mp_damage; // bo2mp lane C
mod bo2mp_grenade; // bo2mp: the grenade warning
mod lui_additive; // bo2mp scope: true additive HUD pictures
mod lui_frontend; // bo2mp lane C
mod lui_hud; // bo2zm M4
mod lui_scene; // bo2mp menus: the front end's 3D backdrop
mod overhead_names; // bo2mp: names over heads in BO2's font
mod rcxd_hint; // bo2mp RC-XD
mod rcxd_screen; // bo2mp RC-XD
mod scope_hint; // bo2mp scope lane
mod target_highlights; // bo2mp Dragonfire
mod zm_hud;

pub use classes::equip_txn::{
    EquipTxnWatch, apply_pending_class_equip, resolve_class_equip_transaction,
    sync_class_change_allowed,
};
pub use classes::icons::{
    ClassSelectIconCache, UiAssetRoot, cac_attachment_image, cac_material_iwd_stem,
    cac_weapon_image, pretty_weapon_name,
};
pub use classes::select::{
    ClassChangeAllowed, ClassChangeBlockReason, ClassEquipRefusal, ClassEquipRequest,
    ClassSelectHighlight, ClassSelectOverlayOpen, ClassSelectPhase, ClassSelectStatus,
    PendingClassEquip, accept_class_equip, class_index_by_name, commit_class_equip,
    reject_class_equip,
};
pub use classes::setup::{ClassEditRow, ClassLoadoutCatalog, ClassPickerFolder, ClassSlotState};
pub use classes::store::SessionClassStore;
pub use frame::{AppScreen, ClassPreset, LaunchIdentity, LaunchReport, showcase_classes};
pub use gap_hud::GapHud;
pub use launch_report::publish_gap_hud;
pub use layers::{
    ApplyUiLayers, GameUiFont, UiCamera, UiDraw, UiLayer, UiLayerVisibility, UiLayers,
    game_text_font,
};
pub use loading::{LoadProgress, LoadingPreviewSource, LoadingScreen};
pub use lui_additive::AdditiveUi;
pub use lui_frontend::{Bo2mpFrontend, Bo2mpFrontendAssets};
pub use menu::MenuMapList;
pub use options::{BindingView, PresentModeOverride};
pub use overhead_names::{OverheadNameRow, OverheadNameRows};
pub use plugin::UiPlugin;
pub use screen::{layers_for_screen, sync_ui_layers};
