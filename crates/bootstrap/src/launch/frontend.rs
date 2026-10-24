//! Black Ops II's multiplayer front end with no map yet.
//!
//! The front end's own zones (scripts, pictures, fonts, tables, English text)
//! are read first; then the menu screen runs BO2's menus on the shell layer.
//! Starting a match from them swaps to the same screen `bo2mp map <zone>`
//! opens on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use asset_audio::SoundCatalog;
use asset_transport::{MapPack, NamespaceSoundIwd, find_zone_file, list_mp_map_packs};
use assets::{NamespaceTrees, T6Frontend};
use audio::{ClipStore, FrontendAudio, SoundBank, SoundIwd};
use bevy::prelude::*;
use master_protocol::ContentFlags;
use net::MasterLaunchIntent;
use ui::{AppScreen, Bo2mpFrontend, Bo2mpFrontendAssets, MenuMapList, UiAssetRoot, UiLayer};

use super::window;
use super::{Boot, Role, content_flags, fatal, insert_screen, intent_or_off, queue_env_capture};
use crate::plugins::{add_runtime_plugins, default_plugins_with_quiet_log};

/// The zone whose folder holds the front end's files.
const ANCHOR_ZONE: &str = "t6:patch_mp";

/// Reads the front end and runs the menus until the window closes.
pub(super) fn run(boot: Boot) {
    let patch = find_zone_file(&boot.games, ANCHOR_ZONE).unwrap_or_else(|error| {
        fatal(&format!(
            "Cannot start the Black Ops II multiplayer front end: its files were not found.\n\n{error}"
        ))
    });
    let zone_all = zone_folder(&patch.path);
    let started = std::time::Instant::now();
    let front = assets::load_t6_frontend(&zone_all);
    for line in &front.report {
        diag::info!(Launch, "{line}");
    }
    diag::info!(
        Launch,
        "frontend t6mp: read {} in {:.1} s",
        zone_all.display(),
        started.elapsed().as_secs_f64()
    );

    // The maps START MATCH and hosting check against.
    let packs = list_mp_map_packs(&boot.games);
    diag::info!(Launch, "{}", maps_report(&packs, &boot.games.0));
    let have = content_flags(&NamespaceTrees::discover(&boot.games));

    let mut app = App::new();
    app.insert_resource(browser_intent(have));
    app.add_plugins(default_plugins_with_quiet_log(window::window_plugin(Some(
        window::menu_window(),
    ))));
    add_menu_assets(&mut app, front, &boot.artifacts);
    app.insert_resource(boot.identity(Role::Listen, ""))
        .insert_resource(MenuMapList(packs))
        .insert_resource(UiAssetRoot(None));
    insert_screen(
        &mut app,
        AppScreen::MainMenu,
        [UiLayer::Shell, UiLayer::Overlay],
    );
    add_runtime_plugins(&mut app);
    queue_env_capture(&mut app, true);
    app.run();
}

/// The folder a zone file sits in.
fn zone_folder(zone_ff: &Path) -> PathBuf {
    zone_ff
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf)
}

/// What the menus list under "maps": how many, in how many packs, and where.
fn maps_report(packs: &[MapPack], games: &Path) -> String {
    let maps: usize = packs.iter().map(|pack| pack.maps.len()).sum();
    format!(
        "frontend t6mp: {maps} maps in {} packs under {}",
        packs.len(),
        games.display()
    )
}

/// The server browser the menus open, if there is a master server to ask.
fn browser_intent(have: ContentFlags) -> MasterLaunchIntent {
    if have.0 & net::CONTENT_T6 == 0 {
        diag::warn!(Net, "master browser is off: BO2 files are not installed");
        return MasterLaunchIntent::disabled();
    }
    intent_or_off("master browser", MasterLaunchIntent::browser_from_env(have))
}

/// The menus' own resources: what they draw from, where they write their
/// stats, and the sounds if the zones had any.
fn add_menu_assets(app: &mut App, front: T6Frontend, artifacts: &Path) {
    let T6Frontend {
        ui,
        icons,
        fonts,
        sound,
        ..
    } = front;
    add_menu_sound(app, sound);
    let menus = Bo2mpFrontend {
        stats_path: artifacts.join("bo2mp_stats.txt"),
    };
    app.insert_resource(Bo2mpFrontendAssets {
        ui: ui.clone(),
        icons: icons.clone(),
        fonts: fonts.clone(),
        front: menus.clone(),
    })
    .insert_resource(ui)
    .insert_resource(icons)
    .insert_resource(fonts)
    .insert_resource(menus);
}

/// The menu music and button sounds. The same bank is put back when a match
/// ends. Without one the menus are silent, and the log says why.
fn add_menu_sound(app: &mut App, sound: Result<SoundCatalog, String>) {
    let catalog = match sound {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => {
            diag::warn!(Launch, "frontend t6mp: no menu sounds: {error}");
            return;
        }
    };
    let iwd = Arc::new(NamespaceSoundIwd::default());
    app.insert_resource(SoundBank(Arc::clone(&catalog)))
        .insert_resource(SoundIwd(Arc::clone(&iwd)))
        .insert_resource(ClipStore::start(
            Arc::clone(&catalog),
            Some(Arc::clone(&iwd)),
        ))
        .insert_resource(FrontendAudio { bank: catalog, iwd });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(label: &str, maps: &[&str]) -> MapPack {
        MapPack {
            label: label.to_owned(),
            maps: maps.iter().map(|map| (*map).to_owned()).collect(),
        }
    }

    #[test]
    fn the_zone_folder_is_where_the_file_sits() {
        assert_eq!(
            zone_folder(Path::new("games/zone/all/patch_mp.ff")),
            PathBuf::from("games/zone/all")
        );
        assert_eq!(zone_folder(Path::new("patch_mp.ff")), PathBuf::new());
        assert_eq!(zone_folder(Path::new("")), PathBuf::new());
    }

    #[test]
    fn the_maps_report_counts_maps_across_packs() {
        let packs = [
            pack("base", &["mp_raid", "mp_slums"]),
            pack("dlc", &["mp_turbine"]),
        ];
        assert_eq!(
            maps_report(&packs, Path::new("games")),
            "frontend t6mp: 3 maps in 2 packs under games"
        );
        assert_eq!(
            maps_report(&[], Path::new("games")),
            "frontend t6mp: 0 maps in 0 packs under games"
        );
    }

    #[test]
    fn without_bo2_files_the_menus_have_no_server_browser() {
        assert!(!browser_intent(ContentFlags(0)).enabled());
        assert!(!browser_intent(ContentFlags(net::CONTENT_T5)).enabled());
    }

    #[test]
    fn menu_sounds_that_failed_to_load_leave_the_menus_silent() {
        let mut app = App::new();
        add_menu_sound(&mut app, Err("no sound zones".into()));
        let world = app.world();
        assert!(!world.contains_resource::<SoundBank>());
        assert!(!world.contains_resource::<SoundIwd>());
        assert!(!world.contains_resource::<ClipStore>());
        assert!(!world.contains_resource::<FrontendAudio>());
    }

    #[test]
    fn menu_sounds_share_one_bank_between_the_menus_and_the_matches() {
        let mut app = App::new();
        add_menu_sound(&mut app, Ok(SoundCatalog::default()));
        let world = app.world();
        let bank = &world.resource::<SoundBank>().0;
        assert!(Arc::ptr_eq(bank, &world.resource::<FrontendAudio>().bank));
        let iwd = &world.resource::<SoundIwd>().0;
        assert!(Arc::ptr_eq(iwd, &world.resource::<FrontendAudio>().iwd));
        assert!(world.contains_resource::<ClipStore>());
    }
}
