//! Finding the games on disk: the root the user points at, the zone files
//! below it, and the maps they hold.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::Read;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use asset_core::ZoneGame;

/// The directory holding the game installs (and shortcuts to more).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamesRoot(pub PathBuf);

/// A zone file that was looked for by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneFile {
    pub path: PathBuf,
    pub zone_name: String,
    /// Set when the name asked for was not the file found.
    pub alias_note: Option<String>,
}

/// The maps of one install folder, labelled for a menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapPack {
    pub label: String,
    pub maps: Vec<String>,
}

const GAMES_ENV: &str = "BO2MP_GAMES";
const T6_VERSION: u32 = fastfile_t6::ZONE_VERSION_PC;

/// Load the nearest `.env`: the one beside the executable, else the first
/// found walking up from the working directory.
pub fn load_dotenv() {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(".env")))
        .filter(|candidate| candidate.is_file());
    if let Some(candidate) = beside_exe {
        let _ = dotenvy::from_path(candidate);
        return;
    }
    let Ok(start) = std::env::current_dir() else {
        return;
    };
    if let Some(candidate) = start
        .ancestors()
        .map(|dir| dir.join(".env"))
        .find(|candidate| candidate.is_file())
    {
        let _ = dotenvy::from_path(candidate);
    }
}

/// The games root named by `BO2MP_GAMES`; on Windows, when it is unset, the
/// directory the executable sits in.
pub fn games_root_from_env() -> Result<GamesRoot, String> {
    load_dotenv();
    let path = match std::env::var_os(GAMES_ENV) {
        Some(raw) => PathBuf::from(raw),
        None => default_games_root()?,
    };
    if !path.is_dir() {
        return Err(format!("{GAMES_ENV} is not a directory: {}", path.display()));
    }
    Ok(GamesRoot(path))
}

#[cfg(windows)]
fn default_games_root() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .ok_or_else(|| format!("{GAMES_ENV} is unset and the launcher directory is unavailable"))
}

#[cfg(not(windows))]
fn default_games_root() -> Result<PathBuf, String> {
    Err(format!(
        "{GAMES_ENV} is not set; copy .env.example to .env and set the games root"
    ))
}

/// The root itself, then (on Windows) the folder each `.lnk` shortcut inside
/// it points at.
pub(crate) fn search_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    #[cfg(windows)]
    if let Ok(listing) = std::fs::read_dir(root) {
        let mut links: Vec<PathBuf> = listing
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
            })
            .collect();
        links.sort();
        for link in links {
            match shortcut_target(&link) {
                Ok(target) if !roots.contains(&target) => roots.push(target),
                Ok(_) => {}
                Err(error) => {
                    diag::warn!(Zone, "game shortcut {} ignored: {error}", link.display());
                }
            }
        }
    }
    roots
}

#[cfg(windows)]
fn shortcut_target(link_path: &Path) -> Result<PathBuf, String> {
    let link = lnk::ShellLink::open(link_path, lnk::encoding::WINDOWS_1252)
        .map_err(|error| format!("cannot parse .lnk: {error}"))?;
    let info = link
        .link_info()
        .as_ref()
        .ok_or_else(|| ".lnk has no LinkInfo filesystem target".to_owned())?;
    if !info.link_info_flags().has_volume_id_and_local_base_path() {
        return Err(".lnk target is not on a local filesystem".to_owned());
    }
    let mut target = info
        .local_base_path_unicode()
        .as_deref()
        .or_else(|| info.local_base_path())
        .map(PathBuf::from)
        .ok_or_else(|| ".lnk has no local base path".to_owned())?;
    let suffix = info
        .common_path_suffix_unicode()
        .as_deref()
        .unwrap_or_else(|| info.common_path_suffix());
    if !suffix.is_empty() {
        target.push(suffix);
    }
    if target.is_dir() {
        Ok(target)
    } else if target.is_file() {
        target
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("target has no folder: {}", target.display()))
    } else {
        Err(format!("target does not exist: {}", target.display()))
    }
}

/// The version number in the 12-byte head of a zone file.
fn envelope_version(path: &Path) -> Result<u32, String> {
    let mut head = [0u8; 12];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .map_err(|error| error.to_string())?;
    if &head[..4] != b"IWff" && !fastfile_t6::is_t6_magic(&head[..8]) {
        return Err(format!("not a zone envelope: {:02x?}", &head[..8]));
    }
    Ok(u32::from_le_bytes([head[8], head[9], head[10], head[11]]))
}

/// Which game a zone file belongs to, if it is one this engine reads.
pub fn zone_game_for_path(path: &Path) -> Option<ZoneGame> {
    (envelope_version(path).ok()? == T6_VERSION).then_some(ZoneGame::T6)
}

/// Visit every file below the roots, breadth first and in name order, each
/// directory once however many links lead to it. Unreadable places come
/// through as `Err` and the walk goes on.
fn walk_files(
    roots: Vec<PathBuf>,
    mut visit: impl FnMut(Result<PathBuf, String>) -> ControlFlow<()>,
) {
    let mut pending: VecDeque<PathBuf> = roots.into();
    let mut seen = HashSet::new();
    while let Some(path) = pending.pop_front() {
        let step = (|| {
            let canonical = std::fs::canonicalize(&path)
                .map_err(|error| format!("cannot access {}: {error}", path.display()))?;
            if !seen.insert(canonical) {
                return Ok(None);
            }
            let metadata = std::fs::metadata(&path)
                .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
            if metadata.is_file() {
                return Ok(Some(None));
            }
            if !metadata.is_dir() {
                return Ok(None);
            }
            let listing = std::fs::read_dir(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let mut children = Vec::new();
            for entry in listing {
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(error) => {
                        return Err(format!("cannot read entry in {}: {error}", path.display()));
                    }
                }
            }
            children.sort();
            Ok(Some(Some(children)))
        })();
        let flow = match step {
            Ok(None) => ControlFlow::Continue(()),
            Ok(Some(None)) => visit(Ok(path)),
            Ok(Some(Some(children))) => {
                pending.extend(children);
                ControlFlow::Continue(())
            }
            Err(error) => visit(Err(error)),
        };
        if flow.is_break() {
            return;
        }
    }
}

fn is_ff(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ff"))
}

/// Every file called `file_name` under the roots with its envelope version,
/// until `stop` says one is enough. Also the walk's own errors.
fn candidates(
    roots: Vec<PathBuf>,
    file_name: &str,
    mut stop: impl FnMut(&Result<u32, String>) -> bool,
) -> (Vec<(PathBuf, Result<u32, String>)>, Vec<String>) {
    let (mut found, mut errors) = (Vec::new(), Vec::new());
    walk_files(roots, |entry| {
        match entry {
            Err(error) => errors.push(error),
            Ok(path) => {
                if path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(file_name))
                {
                    let version = envelope_version(&path);
                    let enough = stop(&version);
                    found.push((path, version));
                    if enough {
                        return ControlFlow::Break(());
                    }
                }
            }
        }
        ControlFlow::Continue(())
    });
    (found, errors)
}

fn zone_file(path: PathBuf, zone: &str) -> ZoneFile {
    ZoneFile {
        path,
        zone_name: zone.to_owned(),
        alias_note: None,
    }
}

fn none_or(list: &[String]) -> String {
    if list.is_empty() {
        "none".to_owned()
    } else {
        list.join("; ")
    }
}

/// `zone` below `search_root` (and its shortcuts), preferring the version
/// this engine reads.
fn find_zone_under(search_root: &Path, zone: &str) -> Result<ZoneFile, String> {
    let file_name = format!("{zone}.ff");
    let (found, errors) = candidates(search_roots(search_root), &file_name, |version| {
        *version == Ok(T6_VERSION)
    });
    if let Some((path, _)) = found
        .iter()
        .find(|(_, version)| *version == Ok(T6_VERSION))
    {
        return Ok(zone_file(path.clone(), zone));
    }
    match found.into_iter().next() {
        Some((path, Ok(version))) => Err(format!(
            "zone `{file_name}` found at {} but version is {version:#x} (want {T6_VERSION:#x})",
            path.display()
        )),
        Some((path, Err(_))) => Err(format!(
            "zone `{file_name}` at {} is not a readable zone envelope (need {T6_VERSION:#x})",
            path.display()
        )),
        None => Err(format!(
            "zone `{file_name}` not found under {}; scan errors: {}",
            search_root.display(),
            none_or(&errors)
        )),
    }
}

/// `zone` anywhere under the games root, with exactly this envelope version.
pub(crate) fn find_zone_file_version(
    root: &GamesRoot,
    zone: &str,
    version: u32,
) -> Result<ZoneFile, String> {
    let file_name = format!("{zone}.ff");
    let (found, errors) = candidates(search_roots(&root.0), &file_name, |seen| {
        *seen == Ok(version)
    });
    if let Some((path, _)) = found.iter().find(|(_, seen)| *seen == Ok(version)) {
        return Ok(zone_file(path.clone(), zone));
    }
    let rejected: Vec<String> = found
        .iter()
        .map(|(path, seen)| match seen {
            Ok(seen) => format!("{} (version {seen:#x})", path.display()),
            Err(error) => format!("{} ({error})", path.display()),
        })
        .chain(errors)
        .collect();
    Err(format!(
        "zone `{file_name}` version {version:#x} not found; {}; other candidates: {}",
        games_root_report(root),
        none_or(&rejected)
    ))
}

/// Split a `game:stem` key into the game and the stem. Anything else is
/// returned whole with no game.
pub fn split_zone_key(zone: &str) -> (Option<ZoneGame>, &str) {
    match zone.split_once(':') {
        Some((tag, rest)) if !rest.is_empty() => match ZoneGame::from_prefix(tag) {
            Some(game) => (Some(game), rest),
            None => (None, zone),
        },
        _ => (None, zone),
    }
}

/// The name a map is shown under: `game:stem`, with the game taken from the
/// key itself or else from `game`.
pub fn map_load_title(key: &str, game: Option<ZoneGame>) -> String {
    let (keyed, stem) = split_zone_key(key);
    match keyed.or(game) {
        Some(game) => format!("{}:{stem}", game.prefix()),
        None => stem.to_owned(),
    }
}

/// Find a zone by name or `game:name` key.
///
/// A bare name that is not already an `mp_` map is looked for as `mp_<name>`
/// first, and the alias is noted on the result.
pub fn find_zone_file(root: &GamesRoot, zone: &str) -> Result<ZoneFile, String> {
    let zone = zone.trim().to_ascii_lowercase();
    if zone.is_empty() {
        return Err("zone name is empty".to_owned());
    }
    let (game, stem) = split_zone_key(&zone);
    let find = |name: &str| match game {
        Some(ZoneGame::T6) => find_zone_file_version(root, name, T6_VERSION),
        None => find_zone_under(&root.0, name),
    };
    if stem.starts_with("mp_") || stem.ends_with("_mp") {
        return find(stem);
    }
    let prefixed = format!("mp_{stem}");
    match find(&prefixed) {
        Ok(mut found) => {
            let note = format!(
                "zone alias: `{stem}` -> `{prefixed}` (MP stem preferred over bare `{stem}.ff`)"
            );
            match std::fs::metadata(&found.path) {
                Ok(meta) => diag::info!(Zone, "{note} ({} bytes)", meta.len()),
                Err(_) => diag::info!(Zone, "{note}"),
            }
            found.alias_note = Some(note);
            Ok(found)
        }
        Err(mp_error) => find(stem).map_err(|bare_error| format!("{mp_error}; {bare_error}")),
    }
}

/// A zone that sits beside `zone_ff`, in the same game install.
pub fn find_zone_for_tree(zone_ff: &Path, zone: &str) -> Result<ZoneFile, String> {
    find_zone_under(&game_root_for_zone(zone_ff)?, zone)
}

/// The shared zone a map in `zone_ff`'s install loads with. A map of the game
/// this engine reads has none of its own to find.
pub fn find_runtime_zone(
    _root: &GamesRoot,
    zone_ff: &Path,
    zone: &str,
) -> Result<ZoneFile, String> {
    match zone_game_for_path(zone_ff) {
        Some(ZoneGame::T6) => Err(format!("{zone}: a Black Ops II map has no runtime zone")),
        None => find_zone_for_tree(zone_ff, zone),
    }
}

pub fn find_runtime_common_mp(root: &GamesRoot, zone_ff: &Path) -> Result<ZoneFile, String> {
    find_runtime_zone(root, zone_ff, "common_mp")
}

/// The install folder (the one with a `zone/` directory) above a zone file.
pub fn game_root_for_zone(zone_ff: &Path) -> Result<PathBuf, String> {
    zone_ff
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("zone").is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "no game tree with a `zone/` directory above {}",
                zone_ff.display()
            )
        })
}

/// One line saying where the games root is, and where its shortcuts lead.
pub fn games_root_report(root: &GamesRoot) -> String {
    let mut line = format!("games root: {}", root.0.display());
    for extra in search_roots(&root.0).iter().skip(1) {
        line.push_str(&format!("; shortcut root: {}", extra.display()));
    }
    line
}

/// Which folder of its install a zone is in: `DLC<n>` or the base game.
fn pack_folder(zone_ff: &Path) -> String {
    game_root_for_zone(zone_ff)
        .ok()
        .and_then(|tree| {
            let below = zone_ff.strip_prefix(tree.join("zone")).ok()?;
            below.components().next()?.as_os_str().to_str().map(str::to_ascii_uppercase)
        })
        .filter(|folder| folder.starts_with("DLC"))
        .unwrap_or_else(|| "BASE".to_owned())
}

/// The playable maps under the games root, grouped by install folder.
///
/// A map is any `mp_*` zone. Where an install ships `<map>_load` zones, only
/// maps that have one count, which leaves out the support zones that share
/// the `mp_` prefix.
pub fn list_mp_map_packs(root: &GamesRoot) -> Vec<MapPack> {
    let mut zones = Vec::new();
    walk_files(search_roots(&root.0), |entry| {
        let path = match entry {
            Ok(path) => path,
            Err(error) => {
                diag::warn!(Zone, "map discovery: {error}");
                return ControlFlow::Continue(());
            }
        };
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_ascii_lowercase);
        if let Some(stem) = stem.filter(|stem| is_ff(&path) && stem.starts_with("mp_"))
            && let Some(game) = zone_game_for_path(&path)
        {
            zones.push((game.prefix(), stem, pack_folder(&path)));
        }
        ControlFlow::Continue(())
    });

    let with_load: HashSet<(&str, &str)> = zones
        .iter()
        .filter_map(|(game, stem, _)| Some((*game, stem.strip_suffix("_load")?)))
        .collect();
    let mut packs: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for (game, stem, folder) in &zones {
        let has_load = with_load.contains(&(*game, stem.as_str()));
        let game_ships_loads = with_load.iter().any(|(g, _)| g == game);
        if stem.ends_with("_load") || (game_ships_loads && !has_load) {
            continue;
        }
        let map = format!("{game}:{stem}");
        if seen.insert(map.clone()) {
            packs.entry((game, folder)).or_default().push(map);
        }
    }
    packs
        .into_iter()
        .map(|((game, folder), mut maps)| {
            maps.sort();
            MapPack {
                label: format!("{} {folder}", game.to_ascii_uppercase()),
                maps,
            }
        })
        .collect()
}

/// Every playable map under the games root, sorted.
pub fn list_mp_maps(root: &GamesRoot) -> Vec<String> {
    let mut maps: Vec<String> = list_mp_map_packs(root)
        .into_iter()
        .flat_map(|pack| pack.maps)
        .collect();
    maps.sort();
    maps
}

/// The folder generated files go in, made if it is not there.
pub fn ensure_artifacts_dir() -> Result<PathBuf, String> {
    let dir = PathBuf::from("bo2mp-artifacts");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bo2mp-find-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An empty `IWff` envelope of `version`.
    fn put_zone(dir: &Path, name: &str, version: u32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let mut head = b"IWff0100".to_vec();
        head.extend_from_slice(&version.to_le_bytes());
        let path = dir.join(name);
        std::fs::write(&path, head).unwrap();
        path
    }

    #[test]
    fn keys_split_into_game_and_stem() {
        assert_eq!(split_zone_key("t6:mp_x"), (Some(ZoneGame::T6), "mp_x"));
        assert_eq!(split_zone_key("t6:"), (None, "t6:"));
        assert_eq!(split_zone_key("zz:mp_x"), (None, "zz:mp_x"));
        assert_eq!(split_zone_key("mp_x"), (None, "mp_x"));
        assert_eq!(map_load_title("mp_x", Some(ZoneGame::T6)), "t6:mp_x");
        assert_eq!(map_load_title("t6:mp_x", None), "t6:mp_x");
        assert_eq!(map_load_title("mp_x", None), "mp_x");
    }

    #[test]
    fn the_mp_stem_wins_over_the_bare_one() {
        let dir = scratch("alias");
        put_zone(&dir.join("g/zone/all"), "mp_hub.ff", T6_VERSION);
        put_zone(&dir.join("g/zone/all"), "hub.ff", T6_VERSION);
        let root = GamesRoot(dir.clone());

        let found = find_zone_file(&root, " HUB ").unwrap();
        assert!(found.path.ends_with("mp_hub.ff"));
        assert!(found.alias_note.is_some());
        let exact = find_zone_file(&root, "t6:hub").unwrap();
        assert!(exact.path.ends_with("mp_hub.ff"));
        assert!(find_zone_file(&root, "").is_err());
        assert!(find_zone_file(&root, "t6:nope").unwrap_err().contains("not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zone_of_another_version_is_reported_not_returned() {
        let dir = scratch("version");
        put_zone(&dir.join("g/zone"), "old.ff", 0x1d9);
        let error = find_zone_file(&GamesRoot(dir.clone()), "old").unwrap_err();
        assert!(error.contains("0x1d9"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packs_group_by_folder_and_skip_support_zones() {
        let dir = scratch("packs");
        let base = dir.join("g/zone/all");
        put_zone(&base, "mp_a.ff", T6_VERSION);
        put_zone(&base, "mp_a_load.ff", T6_VERSION);
        put_zone(&base, "mp_support.ff", T6_VERSION);
        put_zone(&dir.join("g/zone/dlc1"), "mp_b.ff", T6_VERSION);
        put_zone(&dir.join("g/zone/dlc1"), "mp_b_load.ff", T6_VERSION);
        let root = GamesRoot(dir.clone());

        let packs = list_mp_map_packs(&root);
        let labels: Vec<_> = packs.iter().map(|pack| pack.label.as_str()).collect();
        assert_eq!(labels, ["T6 BASE", "T6 DLC1"]);
        assert_eq!(packs[0].maps, ["t6:mp_a"]);
        assert_eq!(list_mp_maps(&root), ["t6:mp_a", "t6:mp_b"]);
        assert_eq!(
            game_root_for_zone(&base.join("mp_a.ff")).unwrap(),
            dir.join("g")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
