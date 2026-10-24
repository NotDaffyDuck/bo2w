//! `UiPlugin`: installs every interface system of the game.

use bevy::prelude::*;
use frame::{ClientSet, register_ui_contracts};

use crate::classes::equip_txn::register_equip_systems;
use crate::classes::select::ClassSelectPlugin;
use crate::gap_hud::register_gap_hud_systems;
use crate::launch_report::publish_gap_hud;
use crate::layers::{GameUiFontPlugin, UiLayersPlugin};
use crate::loading::{poll_loading_preview, spawn_loading_screen, update_loading_screen};
use crate::menu::MenuPlugin;
use crate::screen::register_screen_systems;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        register_ui_contracts(app);

        app.add_plugins((
            GameUiFontPlugin,
            UiLayersPlugin,
            MenuPlugin,
            ClassSelectPlugin,
        ))
        .add_systems(Startup, spawn_loading_screen)
        .add_systems(
            Update,
            (
                (
                    spawn_loading_screen,
                    poll_loading_preview,
                    update_loading_screen,
                )
                    .chain(),
                publish_gap_hud,
            )
                .in_set(ClientSet::Ui),
        );

        // Screens and flows, then the in-game widgets.
        register_screen_systems(app);
        register_equip_systems(app);
        register_gap_hud_systems(app);
        crate::menu_load::register_menu_load_systems(app);

        crate::zm_hud::register_zm_hud_systems(app);
        crate::scope_hint::register(app);
        crate::rcxd_hint::register(app);
        crate::rcxd_screen::register(app);
        crate::target_highlights::register(app);
        crate::overhead_names::register(app);
        crate::lui_hud::register_lui_hud_systems(app);
        crate::bo2mp_damage::register_bo2mp_damage(app);
        crate::bo2mp_grenade::register_bo2mp_grenade(app);
    }
}
