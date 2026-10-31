//! The one plugin the app adds for its interface: sub-modules first, then the load screen
//! and status page systems, then each screen flow and in-game widget in turn.

use bevy::prelude::*;
use frame::ClientStep;

pub struct MenuHudPlugin;

/// Systems that run each frame in the interface set: the load-screen chain, plus the
/// status page refresh beside it.
fn add_frame_systems(host: &mut App) {
    let splash = (
        crate::load_splash::build_splash_scene,
        crate::load_splash::check_splash_thumb,
        crate::load_splash::advance_splash_scene,
    );
    let frame = (splash.chain(), crate::boot_digest::share_interim_overlay);
    host.add_systems(Update, frame.in_set(ClientStep::Ui));
}

/// Every widget's own registration, run in this order.
const WIDGET_HOOKS: [fn(&mut App); 12] = [
    crate::zm_hud::register_zm_hud_systems,
    crate::scope_hint::register,
    crate::rcxd_hint::register,
    crate::rcxd_screen::register,
    crate::target_highlights::register,
    crate::name_tags::register,
    crate::frame_rate::register_frame_rate_systems,
    crate::cheat_menu::register_cheat_menu_systems,
    crate::bo2_globe::register_globe,
    crate::lui_hud::register_lui_hud_systems,
    crate::bo2mp_damage::register_bo2mp_damage,
    crate::bo2mp_grenade::register_bo2mp_grenade,
];

impl Plugin for MenuHudPlugin {
    fn build(&self, app: &mut App) {
        frame::register_ui_contracts(app);
        app.add_plugins((
            crate::tiers::MenuTypefaceModule,
            crate::tiers::OverlayTierModule,
            crate::menu::MenuStateModule,
            crate::classes::select::LoadoutPickerModule,
        ));
        app.add_systems(Startup, crate::load_splash::build_splash_scene);
        add_frame_systems(app);

        crate::screen_tiers::wire_screen_tiers(app);
        crate::classes::equip_txn::install_loadout_arm_steps(app);
        crate::status_page::install_interim_overlay_steps(app);
        crate::map_entry::install_screen_boot_steps(app);
        for hook in WIDGET_HOOKS {
            hook(app);
        }
    }
}
