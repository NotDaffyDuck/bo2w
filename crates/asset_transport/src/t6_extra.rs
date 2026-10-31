//! bo2zm: extra read-only folders of Black Ops II files layered over the
//! install (`BO2MP_T6_EXTRA`, `;`-separated), e.g. a map pack kept outside
//! the game folder. Zones (`.ff`), image packs (`.ipak`) and sound banks
//! (`.sabl`/`.sabs`) are found anywhere under them; the install wins a name
//! both have.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The extra folders, in the order given.
pub fn extra_roots() -> &'static [PathBuf] {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        crate::install_scan::read_env_file();
        std::env::var_os("BO2MP_T6_EXTRA")
            .map(|v| {
                // `;`-separated folders; blanks and missing folders are dropped.
                let list = v.to_string_lossy();
                list.split(';').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_dir()).collect()
            })
            .unwrap_or_default()
    })
}

/// Every file under the extra folders, sorted within each folder.
fn extra_tree() -> &'static [PathBuf] {
    static FILES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut out = Vec::new();
        for root in extra_roots() {
            let mut pending = vec![root.clone()];
            let mut here = Vec::new();
            while let Some(dir) = pending.pop() {
                let Ok(read) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for path in read.flatten().map(|e| e.path()) {
                    if path.metadata().is_ok_and(|meta| meta.is_dir()) {
                        pending.push(path);
                    } else {
                        here.push(path);
                    }
                }
            }
            here.sort();
            out.extend(here);
        }
        out
    })
}

/// Every file under the extra folders whose extension is `ext` (any case).
pub fn extra_files(ext: &str) -> Vec<PathBuf> {
    extra_tree()
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
        .cloned()
        .collect()
}

/// The first extra file named `name` (any case).
pub fn extra_file(name: &str) -> Option<PathBuf> {
    extra_tree()
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .cloned()
}

/// `<dir>/<name>` if it is there, else the extra file of that name, else
/// `<dir>/<name>` (so callers' "is it a file" checks still say no).
pub fn file_in(dir: &Path, entry: &str) -> PathBuf {
    let local = dir.join(entry);
    if local.is_file() { local } else { extra_file(entry).unwrap_or(local) }
}

/// The install's `zone/all` for a zone at `zone_path`: its own folder when
/// that holds the common zones, else the install's under `BO2MP_GAMES` (a
/// map zone from an extra folder).
pub fn install_zone_dir(zone_path: &Path) -> PathBuf {
    let dir = zone_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    if dir.join("common_zm.ff").is_file() || dir.join("common_mp.ff").is_file() {
        return dir;
    }
    crate::install_scan::game_dir_from_environment()
        .ok()
        .and_then(|root| {
            crate::install_scan::search_roots(&root.0)
                .into_iter()
                .map(|r| r.join("zone").join("all"))
                .find(|d| d.join("common_zm.ff").is_file())
        })
        .unwrap_or(dir)
}
