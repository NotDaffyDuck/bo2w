//! Whether a menu holds the keyboard and mouse. Black Ops II's menus are its
//! own UI (`ui::lui_hud`, published as `frame::LuiMenus`).

use bevy::prelude::*;
use frame::{AppScreen, LuiMenus};
use net::{LocalPresentClient, PresentedSnapshot};

#[derive(Resource, Default)]
pub struct ScriptMenus {
    lui_open: bool,
}

impl ScriptMenus {
    pub fn captures_input(&self) -> bool {
        self.lui_open
    }
}

/// Mirrors the LUI menu state into `ScriptMenus` and the HUD input view. A
/// menu only counts in the front end, or in a match once the local client
/// appears in the presented snapshot.
pub(crate) fn track_lui_menus(
    screen: Res<AppScreen>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    lui: Option<Res<LuiMenus>>,
    mut menus: ResMut<ScriptMenus>,
    hud_input: Option<ResMut<frame::HudInputView>>,
) {
    let local_in_snapshot = || {
        presented
            .snapshot()
            .is_some_and(|snap| snap.meta.for_client(local.0).is_some())
    };
    let menus_apply = match *screen {
        AppScreen::MainMenu => true,
        AppScreen::InGame | AppScreen::ClassSelect => local_in_snapshot(),
        _ => false,
    };
    let open = menus_apply && lui.is_some_and(|lui| lui.open);

    if menus.lui_open != open {
        menus.lui_open = open;
    }
    let Some(mut view) = hud_input else {
        return;
    };
    if view.script_menu_open != open {
        view.script_menu_open = open;
    }
}
