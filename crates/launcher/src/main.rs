//! The `bo2mp` executable. It does the work that belongs to the process (the
//! allocator, the working folder, the log, the command line, the environment)
//! and hands the rest to `bootstrap`.

use std::path::Path;

#[global_allocator]
static PROCESS_ALLOCATOR: diag::ProcessCountingAllocator = diag::ProcessCountingAllocator;

fn main() {
    if let Err(error) = enter_install_folder() {
        diag::exit_launch_error(&error);
    }
    let artifacts = asset_transport::ensure_artifacts_dir()
        .unwrap_or_else(|error| diag::exit_launch_error(&error));
    announce_log(&diag::init_log(&artifacts));

    let (mode, acceptance) = bootstrap::parse_cli(std::env::args().skip(1))
        .unwrap_or_else(|error| diag::exit_launch_error(&error));
    let games = asset_transport::games_root_from_env()
        .unwrap_or_else(|error| diag::exit_launch_error(&error));

    bootstrap::launch(games, artifacts, mode, acceptance);
    // The log writer thread still holds the last lines.
    diag::flush();
}

/// On Windows the game is started from shortcuts and from the updater, so the
/// working folder is whatever they chose; relative paths mean the folder the
/// executable is in. Elsewhere the caller's folder is kept.
fn enter_install_folder() -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot find the running bo2mp.exe: {error}"))?;
    let folder = install_folder(&exe)?;
    std::env::set_current_dir(folder)
        .map_err(|error| format!("cannot enter {}: {error}", folder.display()))
}

fn install_folder(exe: &Path) -> Result<&Path, String> {
    exe.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .ok_or_else(|| format!("{} is not inside a folder", exe.display()))
}

fn announce_log(path: &Path) {
    diag::announce_log_stdout(path, diag::latest_log_path().as_deref());
    diag::info!(Launch, "log: {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_folder_is_where_the_exe_is() {
        let exe = Path::new("games").join("bo2mp").join("bo2mp.exe");
        assert_eq!(install_folder(&exe), Ok(exe.parent().unwrap()));
    }

    #[test]
    fn an_exe_with_no_folder_has_no_install_folder() {
        for exe in ["bo2mp.exe", ""] {
            let error = install_folder(Path::new(exe)).unwrap_err();
            assert!(
                error.ends_with("is not inside a folder"),
                "{exe:?}: {error}"
            );
        }
    }
}
