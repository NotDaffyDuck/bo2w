//! Black Ops II's multiplayer front end with no map yet.
//!
//! The front end's own zones (scripts, pictures, fonts, tables, English text)
//! are read first; then the menu screen runs BO2's menus on the shell layer.
//! Starting a match from them swaps to the same screen `bo2mp map <zone>`
//! opens on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use asset_audio::AudioRoster;
use asset_transport::{MapPack, CategorySoundArchive, list_mp_map_packs};
use assets::{CategoryTreeSet, T6Frontend};
use audio::{ClipRepository, FrontendAudio, AudioCatalogRes, SoundArchive};
use bevy::prelude::*;
use master_protocol::AssetCoverageBits;
use net::DirectoryLaunchRequest;
use ui::{ShellScreen, Bo2mpFrontend, Bo2mpFrontendAssets, ScreenArenaRoster, MenuMediaAnchor, OverlayTier};

use super::window;
use super::{
    Boot, CONTENT_T6, Role, content_catalog, pack_presence_bits, fatal, insert_screen, intent_or_off,
    queue_env_capture,
};
use crate::app_stack::{install_gameplay_plugins, stock_plugins_hushed_logging};
use asset_transport::locate_zone_file;

/// The zone whose folder holds the front end's files.
const ANCHOR_ZONE: &str = "t6:patch_mp";

/// Reads the front end and runs the menus until the window closes.
pub(super) fn run(boot: Boot) {
    let patch = locate_zone_file(&boot.games, ANCHOR_ZONE).unwrap_or_else(|error| {
        let why = format!(
            "Cannot start the Black Ops II multiplayer front end: its files were not found.\n\n{error}"
        );
        fatal(&why)
    });
    let zone_all = zone_folder(&patch.path);
    let clock = std::time::Instant::now();
    let front = assets::load_t6_frontend(&zone_all);
    for line in &front.report {
        diag::info!(Launch, "{line}");
    }
    diag::info!(
        Launch,
        "frontend t6mp: read {} in {:.1} s",
        zone_all.display(),
        clock.elapsed().as_secs_f64()
    );

    // The maps START MATCH and hosting check against.
    let packs = list_mp_map_packs(&boot.games);
    diag::info!(Launch, "{}", maps_report(&packs, &boot.games.0));
    let have = pack_presence_bits(&CategoryTreeSet::discover(&boot.games));

    let mut app = App::new();
    app.insert_resource(browser_intent(have));
    app.insert_resource(content_catalog());
    app.add_plugins(stock_plugins_hushed_logging(window::window_plugin(Some(
        window::menu_window(),
    ))));
    add_menu_assets(&mut app, front, &boot.writes_to);
    app.insert_resource(boot.identity(Role::Listen, ""))
        .insert_resource(ScreenArenaRoster(packs))
        .insert_resource(MenuMediaAnchor(None));
    insert_screen(
        &mut app,
        ShellScreen::MainMenu,
        [OverlayTier::Shell, OverlayTier::Overlay],
    );
    install_gameplay_plugins(&mut app);
    queue_env_capture(&mut app, true);
    app.run();
}

/// The folder a zone file sits in.
fn zone_folder(zone_archive_file: &Path) -> PathBuf {
    zone_archive_file
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
fn browser_intent(have: AssetCoverageBits) -> DirectoryLaunchRequest {
    if have.0 & CONTENT_T6 == 0 {
        diag::warn!(WireLink, "master browser is off: BO2 files are not installed");
        return DirectoryLaunchRequest::disabled();
    }
    intent_or_off("master browser", DirectoryLaunchRequest::browser_from_env(have))
}

/// The menus' own resources: what they draw from, where they write their
/// stats, and the sounds if the zones had any.
fn add_menu_assets(app: &mut App, front: T6Frontend, out_dir: &Path) {
    let T6Frontend {
        ui,
        icons,
        fonts,
        sound,
        ..
    } = front;
    add_menu_sound(app, sound);
    let shell = Bo2mpFrontend {
        stats_path: out_dir.join("bo2mp_stats.txt"),
    };
    app.insert_resource(Bo2mpFrontendAssets {
        ui: ui.clone(),
        icons: icons.clone(),
        fonts: fonts.clone(),
        front: shell.clone(),
    })
    .insert_resource(ui)
    .insert_resource(icons)
    .insert_resource(fonts)
    .insert_resource(shell);
}

/// The menu music and button sounds. The same bank is put back when a match
/// ends. Without one the menus are silent, and the log says why.
fn add_menu_sound(app: &mut App, sound: Result<AudioRoster, String>) {
    let catalog = match sound {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => {
            diag::warn!(Launch, "frontend t6mp: no menu sounds: {error}");
            return;
        }
    };
    let iwd = Arc::new(CategorySoundArchive::default());
    app.insert_resource(AudioCatalogRes(Arc::clone(&catalog)))
        .insert_resource(SoundArchive(Arc::clone(&iwd)))
        .insert_resource(ClipRepository::start(
            Arc::clone(&catalog),
            Some(Arc::clone(&iwd)),
        ))
        .insert_resource(FrontendAudio { bank: catalog, iwd });
}

#[cfg(test)]
mod tests {
    use super::super::CONTENT_T5;
    use super::*;

    fn pack(title: &str, maps: &[&str]) -> MapPack {
        MapPack {
            label: title.to_owned(),
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
        assert!(!browser_intent(AssetCoverageBits(0)).enabled());
        assert!(!browser_intent(AssetCoverageBits(CONTENT_T5)).enabled());
    }

    #[test]
    fn menu_sounds_that_failed_to_load_leave_the_menus_silent() {
        let mut app = App::new();
        add_menu_sound(&mut app, Err("no sound zones".into()));
        let world = app.world();
        assert!(!world.contains_resource::<AudioCatalogRes>());
        assert!(!world.contains_resource::<SoundArchive>());
        assert!(!world.contains_resource::<ClipRepository>());
        assert!(!world.contains_resource::<FrontendAudio>());
    }

    #[test]
    fn menu_sounds_share_one_bank_between_the_menus_and_the_matches() {
        let mut app = App::new();
        add_menu_sound(&mut app, Ok(AudioRoster::default()));
        let world = app.world();
        let bank = &world.resource::<AudioCatalogRes>().0;
        assert!(Arc::ptr_eq(bank, &world.resource::<FrontendAudio>().bank));
        let iwd = &world.resource::<SoundArchive>().0;
        assert!(Arc::ptr_eq(iwd, &world.resource::<FrontendAudio>().iwd));
        assert!(world.contains_resource::<ClipRepository>());
    }
}
