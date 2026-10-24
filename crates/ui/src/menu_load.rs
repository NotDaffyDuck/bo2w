//! Entering the loading screen when the session approves a map, and coming
//! back to the main menu when the match ends.

use std::path::PathBuf;

use asset_transport::{
    GamesRoot, LoadProgress, find_runtime_common_mp, find_zone_file, list_mp_map_packs,
};
use assets::{LoadingPreviewSource, MatchLoadAbort, MatchLoadRequest};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::{AppScreen, ClientSet, LaunchIdentity, LaunchReport, MapLoadApproved, ReturnedToMenu};

use crate::classes::select::ClassSelectOverlayOpen;
use crate::layers::{UiLayer, UiLayers};
use crate::loading::{
    LoadingCamera, LoadingRoot, LoadingScreen, OverlayUiCamera, dismiss_loading_overlay,
};
use crate::menu::MenuMapList;

/// What looking up a requested map turned up.
struct ResolvedZone {
    name: String,
    zone_ff: Result<PathBuf, String>,
    common_mp: Result<PathBuf, String>,
    alias: Option<String>,
    title: String,
}

fn resolve_zone(games: &GamesRoot, requested: &str) -> ResolvedZone {
    let found = find_zone_file(games, requested);
    let requested = requested.trim().to_ascii_lowercase();
    let (key_game, stem) = asset_transport::split_zone_key(&requested);

    let alias = found.as_ref().ok().and_then(|zone| zone.alias_note.clone());
    let name = found
        .as_ref()
        .map_or_else(|_| stem.to_owned(), |zone| zone.zone_name.clone());
    let zone_ff = found
        .as_ref()
        .map(|zone| zone.path.clone())
        .map_err(Clone::clone);
    let common_mp = match &zone_ff {
        Ok(path) => find_runtime_common_mp(games, path).map(|zone| zone.path),
        Err(error) => Err(error.clone()),
    };
    let game = key_game.or_else(|| {
        let zone = found.as_ref().ok()?;
        asset_transport::zone_game_for_path(&zone.path)
    });

    ResolvedZone {
        title: asset_transport::map_load_title(&requested, game),
        name,
        zone_ff,
        common_mp,
        alias,
    }
}

/// The status record shown while the map is still loading.
fn pending_launch_report(zone: &ResolvedZone) -> LaunchReport {
    LaunchReport {
        zone: zone.name.clone(),
        common_mp: zone.common_mp.clone(),
        zone_ff: zone.zone_ff.clone(),
        zone_alias: zone.alias.clone(),
        sim_gap: "map loading has not reached clipmap yet",
        prediction_metrics: None,
        world_report: vec!["queued background zone walk".into()],
    }
}

fn show_loading_screen(
    commands: &mut Commands,
    zone: &ResolvedZone,
    progress: LoadProgress,
    request_id: u64,
    mode: sim::HostGameModeSelection,
) {
    diag::info!(Ui, "loading: map={} mode={}", zone.name, mode.token());
    commands.insert_resource(LoadingScreen::new(
        progress,
        zone.title.clone(),
        mode.display_name().to_owned(),
    ));
    if let Ok(path) = &zone.zone_ff {
        commands.insert_resource(LoadingPreviewSource {
            path: path.clone(),
            map_name: zone.name.clone(),
            request_id,
        });
    }
    let mut layers = UiLayers::default();
    layers.show_only([UiLayer::Loading, UiLayer::Overlay]);
    commands.insert_resource(layers);
}

pub(crate) fn begin_load_from_session(
    mut commands: Commands,
    mut approved: MessageReader<MapLoadApproved>,
    mode: Option<Res<sim::HostGameModeSelection>>,
    mut screen: ResMut<AppScreen>,
    mut class_overlay: ResMut<ClassSelectOverlayOpen>,
    identity: Option<ResMut<LaunchIdentity>>,
    loading: Option<Res<LoadingScreen>>,
    request: Option<Res<MatchLoadRequest>>,
    abort: Option<Res<MatchLoadAbort>>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Only the newest approval matters.
    let Some(approval) = approved.read().last() else {
        return;
    };
    if abort.is_some_and(|abort| abort.0 == approval.request_id) {
        return;
    }
    let Some(mut identity) = identity else {
        return;
    };

    let zone = resolve_zone(&GamesRoot(identity.games_root.clone()), &approval.zone);
    // Already showing this very map's loading screen.
    if loading.is_some_and(|screen| screen.title() == zone.title) {
        return;
    }

    let progress = request
        .filter(|request| request.request_id == approval.request_id)
        .map(|request| request.progress.clone())
        .unwrap_or_default();
    let mode = mode
        .as_deref()
        .copied()
        .unwrap_or_else(sim::HostGameModeSelection::from_env);

    identity.zone = zone.name.clone();
    if let Ok(mut window) = window.single_mut() {
        window.title = format!("bo2mp — {}", zone.name);
    }
    class_overlay.0 = false;
    *screen = AppScreen::Loading;
    commands.insert_resource(pending_launch_report(&zone));
    show_loading_screen(&mut commands, &zone, progress, approval.request_id, mode);
    diag::info!(
        Ui,
        "session: loading overlay `{}` (swap #{})",
        zone.name,
        approval.request_id
    );
}

pub(crate) fn restore_menu_on_return(
    mut commands: Commands,
    mut returned: MessageReader<ReturnedToMenu>,
    mut class_overlay: ResMut<ClassSelectOverlayOpen>,
    mut maps: ResMut<MenuMapList>,
    identity: Option<Res<LaunchIdentity>>,
    mut screen: ResMut<AppScreen>,
    loading_parts: Query<Entity, Or<(With<LoadingRoot>, With<LoadingCamera>)>>,
    overlay_cameras: Query<Entity, With<OverlayUiCamera>>,
) {
    if returned.read().next().is_none() {
        return;
    }
    class_overlay.0 = false;
    *screen = AppScreen::MainMenu;
    dismiss_loading_overlay(
        &mut commands,
        loading_parts.iter(),
        overlay_cameras.iter(),
        false,
    );

    if maps.0.is_empty()
        && let Some(identity) = identity
    {
        maps.0 = list_mp_map_packs(&GamesRoot(identity.games_root.clone()));
    }
    diag::info!(Ui, "session: main menu enabled");
    diag::lifecycle_boundary("menu_interactive", "");
}

pub(crate) fn register_menu_load_systems(app: &mut App) {
    app.add_message::<ReturnedToMenu>().add_systems(
        Update,
        (
            begin_load_from_session,
            restore_menu_on_return.after(begin_load_from_session),
        )
            .after(assets::MapLoadApproval)
            .before(assets::MatchLoadDispatch)
            .in_set(ClientSet::Load),
    );
}
