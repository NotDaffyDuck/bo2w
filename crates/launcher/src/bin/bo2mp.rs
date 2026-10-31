//! Entry point of `bo2mp.exe`. Process-wide setup lives here: the counting heap,
//! the current directory, the log file, the flags and the game folder. Everything
//! after that is `bootstrap`'s job.

use std::path::Path;

#[global_allocator] static COUNTED_HEAP: diag::TallyingHeap = diag::TallyingHeap;

/// Unwraps a start-up step, or stops the process with its message.
fn or_abort<T>(step: Result<T, String>) -> T {
    step.unwrap_or_else(|why| diag::abort_on_startup_fault(&why))
}

fn main() {
    or_abort(move_to_exe_folder());
    let output = or_abort(asset_transport::prepare_output_folder());
    let log_file = diag::init_log(&output);
    report_log_file(&log_file);

    let (run_mode, accept) = or_abort(bootstrap::read_flags(std::env::args().skip(1)));
    let game_root = or_abort(asset_transport::game_dir_from_environment());
    bootstrap::run_mode(game_root, output, run_mode, accept);

    // Lines queued after the last write are still with the log thread.
    diag::flush();
}

/// Shortcuts and the updater start the exe from any directory, so on Windows we
/// switch to the exe's own folder before relative paths are used. Other systems
/// keep the directory the caller gave.
fn move_to_exe_folder() -> Result<(), String> {
    if cfg!(not(windows)) {
        return Ok(());
    }
    let running = std::env::current_exe().map_err(|why| format!("cannot find the running bo2mp.exe: {why}"))?;
    let home = exe_folder(&running)?;
    std::env::set_current_dir(home).map_err(|why| format!("cannot enter {}: {why}", home.display()))
}

fn exe_folder(exe: &Path) -> Result<&Path, String> {
    let shown = exe.display();
    exe.parent().filter(|dir| !dir.as_os_str().is_empty()).ok_or_else(|| format!("{shown} is not inside a folder"))
}

fn report_log_file(log_file: &Path) {
    let previous = diag::newest_log_location();
    diag::print_log_location(log_file, previous.as_deref());
    diag::info!(Launch, "writing the log to {}", log_file.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_folder_is_the_parent_directory() {
        let exe = Path::new("games").join("bo2mp").join("bo2mp.exe");
        assert_eq!(exe_folder(&exe), Ok(exe.parent().unwrap()));
    }

    #[test]
    fn bare_or_empty_exe_paths_have_no_folder() {
        for name in ["bo2mp.exe", ""] {
            let why = exe_folder(Path::new(name)).unwrap_err();
            assert!(why.ends_with("is not inside a folder"), "{name:?}: {why}");
        }
    }
}
