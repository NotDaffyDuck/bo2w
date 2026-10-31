//! Switches to the load screen once the session grants a map, and back to
//! the main menu when the match is over.

use std::path::PathBuf;

use assets::{LoadBackdropOrigin, SessionLoadCancel, SessionLoadOrder};
use frame::{ShellScreen, ClientStep, StartupPersona, StartupReport, LevelOpenGranted, ReturnedToMenu};
use asset_transport::GameInstallDir;
use asset_transport::LoadCompletion;
use bevy::prelude::*;

use crate::MainWindowMut;
use crate::classes::select::LoadoutPickerShown;
use crate::load_splash::{LoadBackdrop, OverlayLens, SplashLens, SplashSceneAnchor, close_splash_overlay};
use crate::menu::ScreenArenaRoster;
use crate::tiers::{OverlayTier, OverlayTierSet};

/// Status-page note for as long as loading has not got as far as the clipmap.
const SIM_GAP_WHILE_LOADING: &str = "map loading has not reached clipmap yet";
const WALK_QUEUED: &str = "queued background zone walk";

/// Everything the lookup of one requested map produced.
#[derive(Clone)]
struct ResolvedZone {
    name: String,
    nickname_remark: Option<String>,
    zone_file: Result<PathBuf, String>,
    heading: String,
    common: Result<PathBuf, String>,
}

fn resolve_zone(games: &GameInstallDir, wanted: &str) -> ResolvedZone {
    let key = wanted.trim().to_ascii_lowercase();
    let (key_game, stem) = asset_transport::zone_key_parts(&key);
    let mut out = ResolvedZone {
        name: stem.to_owned(),
        nickname_remark: None,
        zone_file: Err(String::new()),
        heading: String::new(),
        common: Err(String::new()),
    };
    let mut path_game = None;
    match asset_transport::locate_zone_file(games, wanted) {
        Ok(located) => {
            out.name = located.zone_label.clone();
            out.nickname_remark = located.nickname_remark.clone();
            path_game = asset_transport::title_of_zone_path(&located.path);
            out.zone_file = Ok(located.path);
        }
        Err(why) => out.zone_file = Err(why),
    }
    out.common = match &out.zone_file {
        Ok(path) => asset_transport::locate_shared_mp_zone(games, path).map(|hit| hit.path),
        Err(why) => Err(why.clone()),
    };
    out.heading = asset_transport::loading_caption(&key, key_game.or(path_game));
    out
}

/// The status record to show until the map has finished loading.
fn pending_launch_report(zone: &ResolvedZone) -> StartupReport {
    let copy = zone.clone();
    StartupReport {
        level_digest: vec![WALK_QUEUED.to_owned()],
        pack_nickname: copy.nickname_remark,
        zone: copy.name,
        zone_archive_file: copy.zone_file,
        shared_zone: copy.common,
        stepping_lag: SIM_GAP_WHILE_LOADING,
        forecast_figures: None,
    }
}

fn show_loading_screen(
    cmd: &mut Commands,
    zone: &ResolvedZone,
    progress: LoadCompletion,
    ticket: u64,
    mode: sim::HostChosenMode,
) {
    diag::info!(Ui, "loading: map {} in mode {}", zone.name, mode.token());
    if let Ok(path) = &zone.zone_file {
        cmd.insert_resource(LoadBackdropOrigin {
            level_name: zone.name.clone(),
            path: path.clone(),
            ticket,
        });
    }
    let label = mode.shown_label().to_owned();
    cmd.insert_resource(LoadBackdrop::new(progress, zone.heading.clone(), label));
    let mut tiers = OverlayTierSet::default();
    tiers.raise_exactly([OverlayTier::Loading, OverlayTier::Overlay]);
    cmd.insert_resource(tiers);
}

/// Progress of the load this grant is for. A missing order, or one for an
/// older grant, means starting from zero.
fn progress_for(order: Option<&SessionLoadOrder>, ticket: u64) -> LoadCompletion {
    order
        .filter(|o| o.ticket == ticket)
        .map(|o| o.progress.clone())
        .unwrap_or_default()
}

pub(crate) fn start_boot_for_match(
    mut cmd: Commands,
    mut primary_win: MainWindowMut,
    mut on_screen: ResMut<ShellScreen>,
    mode: Option<Res<sim::HostChosenMode>>,
    mut grants: MessageReader<LevelOpenGranted>,
    persona: Option<ResMut<StartupPersona>>,
    mut picker: ResMut<LoadoutPickerShown>,
    cancel: Option<Res<SessionLoadCancel>>,
    splash: Option<Res<LoadBackdrop>>,
    order: Option<Res<SessionLoadOrder>>,
) {
    // Older grants in the queue are superseded by the last one.
    let Some(grant) = grants.read().last() else {
        return;
    };
    let ticket = grant.ask_number;
    if cancel.is_some_and(|c| c.0 == ticket) {
        return;
    }
    let Some(mut persona) = persona else {
        return;
    };

    let install = GameInstallDir(persona.titles_install_dir.clone());
    let zone = resolve_zone(&install, &grant.zone);
    // The load screen for this exact map is already up.
    if splash.is_some_and(|up| up.title() == zone.heading) {
        return;
    }

    let progress = progress_for(order.as_deref(), ticket);
    let mode = match mode {
        Some(picked) => *picked,
        None => sim::HostChosenMode::read_env(),
    };

    persona.zone.clone_from(&zone.name);
    if let Some(mut win) = primary_win.shown_mut() {
        win.title = format!("bo2mp — {}", zone.name);
    }
    *on_screen = ShellScreen::Loading;
    picker.0 = false;
    cmd.insert_resource(pending_launch_report(&zone));
    show_loading_screen(&mut cmd, &zone, progress, ticket, mode);
    diag::info!(Ui, "session: loading screen up for `{}` (load {})", zone.name, ticket);
}

pub(crate) fn restore_menu_on_return(
    mut cmd: Commands,
    mut back: MessageReader<ReturnedToMenu>,
    splash_parts: Query<Entity, Or<(With<SplashSceneAnchor>, With<SplashLens>)>>,
    mut roster: ResMut<ScreenArenaRoster>,
    persona: Option<Res<StartupPersona>>,
    mut picker: ResMut<LoadoutPickerShown>,
    lenses: Query<Entity, With<OverlayLens>>,
    mut on_screen: ResMut<ShellScreen>,
) {
    if back.read().next().is_none() {
        return;
    }
    picker.0 = false;
    *on_screen = ShellScreen::MainMenu;
    close_splash_overlay(&mut cmd, splash_parts.iter(), lenses.iter(), false);

    let needs_maps = roster.0.is_empty();
    if let Some(persona) = persona.filter(|_| needs_maps) {
        let install = GameInstallDir(persona.titles_install_dir.clone());
        roster.0 = asset_transport::list_mp_map_packs(&install);
    }
    diag::info!(Ui, "session: main menu turned on");
    diag::lifecycle_boundary("menu_interactive", "");
}

pub(crate) fn install_screen_boot_steps(host: &mut App) {
    let begin = start_boot_for_match;
    let restore = restore_menu_on_return.after(begin);
    let load_set = ClientStep::Load;
    host.add_message::<ReturnedToMenu>();
    let steps = (begin, restore).after(assets::MapLoadApproval).before(assets::MatchLoadDispatch);
    host.add_systems(Update, steps.in_set(load_set));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_report_carries_the_lookup_outcome() {
        let missing = || Err::<PathBuf, String>("no such zone".to_owned());
        let zone = ResolvedZone {
            name: "mp_demo".into(),
            nickname_remark: None,
            zone_file: missing(),
            heading: "Demo".into(),
            common: missing(),
        };
        let report = pending_launch_report(&zone);
        assert_eq!(report.zone, "mp_demo");
        assert_eq!(report.zone_archive_file, missing());
        assert!(report.pack_nickname.is_none());
        assert!(report.forecast_figures.is_none());
    }
}
