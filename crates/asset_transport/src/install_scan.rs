//! Finding the games on disk: the root the user points at, the zone files
//! below it, and the maps they hold.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

use asset_core::ZoneTitleKind;

/// The directory holding the game installs (and shortcuts to more).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInstallDir(pub PathBuf);

/// A zone file that was looked for by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneArchive {
    pub path: PathBuf,
    pub zone_label: String,
    /// Set when the name asked for was not the file found.
    pub nickname_remark: Option<String>,
}

/// The maps of one install folder, labelled for a menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapPack {
    pub label: String,
    pub maps: Vec<String>,
}

const GAMES_ENV: &str = "BO2MP_GAMES";
const ARTIFACTS_DIR: &str = "bo2mp-artifacts";
const T6_VERSION: u32 = fastfile_t6::T6_PC_ZONE_REV;

fn dotenv_beside_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(".env");
    candidate.is_file().then_some(candidate)
}

fn dotenv_above_cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .map(|dir| dir.join(".env"))
        .find(|file| file.is_file())
}

/// Load the nearest `.env`: the one beside the executable, else the first
/// found walking up from the working directory.
pub fn read_env_file() {
    let Some(file) = dotenv_beside_exe().or_else(dotenv_above_cwd) else {
        return;
    };
    let _ = dotenvy::from_path(file);
}

fn games_env_value() -> Option<std::ffi::OsString> {
    read_env_file();
    std::env::var_os(GAMES_ENV)
}

fn require_dir(path: PathBuf) -> Result<PathBuf, String> {
    let shown = path.display().to_string();
    path.is_dir()
        .then_some(path)
        .ok_or_else(|| format!("{GAMES_ENV} is not a directory: {shown}"))
}

/// The games root named by `BO2MP_GAMES`; on Windows, when it is unset, the
/// directory the executable sits in.
pub fn game_dir_from_environment() -> Result<GameInstallDir, String> {
    let named = games_env_value().map_or_else(fallback_games_root, |v| Ok(PathBuf::from(v)));
    named.and_then(require_dir).map(GameInstallDir)
}

#[cfg(windows)]
fn fallback_games_root() -> Result<PathBuf, String> {
    let unavailable = || format!("{GAMES_ENV} is unset and the launcher directory is unavailable");
    let exe = std::env::current_exe().map_err(|_| unavailable())?;
    exe.parent().map(Path::to_path_buf).ok_or_else(unavailable)
}

#[cfg(not(windows))]
fn fallback_games_root() -> Result<PathBuf, String> {
    Err([
        GAMES_ENV,
        " is not set; copy .env.example to .env and set the games root",
    ]
    .concat())
}

/// The folders the `.lnk` shortcuts directly inside `base` lead to, in
/// shortcut-name order. Shortcuts that cannot be followed are logged.
#[cfg(windows)]
fn linked_folders(base: &Path) -> Vec<PathBuf> {
    let links: BTreeSet<PathBuf> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.path())
        .filter(|candidate| has_extension(candidate, "lnk"))
        .collect();
    let mut folders = Vec::with_capacity(links.len());
    for link in &links {
        match shortcut_target(link) {
            Ok(folder) => folders.push(folder),
            Err(why) => diag::warn!(Zone, "game shortcut {} ignored: {why}", link.display()),
        }
    }
    folders
}

#[cfg(not(windows))]
fn linked_folders(_base: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// The root itself, then (on Windows) the folder each `.lnk` shortcut inside
/// it points at.
pub(crate) fn search_roots(base: &Path) -> Vec<PathBuf> {
    linked_folders(base)
        .into_iter()
        .fold(vec![base.to_path_buf()], |mut roots, folder| {
            if !roots.contains(&folder) {
                roots.push(folder);
            }
            roots
        })
}

/// bo2zm: the roots to search for game files: the games root and its shortcuts, then the
/// extra read-only folders (`BO2MP_T6_EXTRA`).
fn game_roots(base: &Path) -> Vec<PathBuf> {
    let mut roots = search_roots(base);
    roots.extend(crate::t6_extra::extra_roots().iter().cloned());
    roots
}

/// The folder a shortcut destination stands for: itself when it is a folder,
/// else the folder holding it.
#[cfg(windows)]
fn folder_of(write_target: PathBuf) -> Result<PathBuf, String> {
    match (write_target.is_dir(), write_target.is_file()) {
        (true, _) => Ok(write_target),
        (false, true) => write_target
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("target has no folder: {}", write_target.display())),
        (false, false) => Err(format!("target does not exist: {}", write_target.display())),
    }
}

#[cfg(windows)]
fn shortcut_target(link_path: &Path) -> Result<PathBuf, String> {
    let shell_link = lnk::ShellLink::open(link_path, lnk::encoding::WINDOWS_1252)
        .map_err(|why| format!("cannot parse .lnk: {why}"))?;
    let info = match shell_link.link_info() {
        None => return Err(".lnk has no LinkInfo filesystem target".to_owned()),
        Some(info) if !info.link_info_flags().has_volume_id_and_local_base_path() => {
            return Err(".lnk target does not live on a local drive".to_owned());
        }
        Some(info) => info,
    };
    let wide_local = info.local_base_path_unicode();
    let Some(local) = wide_local.as_deref().or_else(|| info.local_base_path()) else {
        return Err(".lnk has no local base path".to_owned());
    };
    let wide_tail = info.common_path_suffix_unicode();
    let tail = wide_tail.as_deref().unwrap_or_else(|| info.common_path_suffix());
    folder_of([local, tail].into_iter().filter(|part| !part.is_empty()).collect())
}

/// The first twelve bytes of a file.
fn read_head(path: &Path) -> std::io::Result<[u8; 12]> {
    let mut head = [0u8; 12];
    std::fs::File::open(path)?.read_exact(&mut head)?;
    Ok(head)
}

/// The version number in the 12-byte head of a zone file.
fn envelope_version(path: &Path) -> Result<u32, String> {
    let [magic @ .., v0, v1, v2, v3] = read_head(path).map_err(|why| why.to_string())?;
    if magic.starts_with(b"IWff") || fastfile_t6::is_t6_magic(&magic) {
        return Ok(u32::from_le_bytes([v0, v1, v2, v3]));
    }
    Err(format!("not a zone envelope: {magic:02x?}"))
}

/// Which game a zone file belongs to, if it is one this engine reads.
pub fn title_of_zone_path(zone_path: &Path) -> Option<ZoneTitleKind> {
    envelope_version(zone_path)
        .ok()
        .filter(|version| *version == T6_VERSION)
        .map(|_| ZoneTitleKind::T6)
}

/// What a path turned out to be when the walk reached it.
enum Found {
    Nothing,
    File,
    Dir(BTreeSet<PathBuf>),
}

/// An error reporter for `map_err`: "cannot `what` `path`: why".
fn io_failure<'a>(what: &'a str, path: &'a Path) -> impl Fn(std::io::Error) -> String + 'a {
    move |why| format!("cannot {what} {}: {why}", path.to_string_lossy())
}

/// Every file below some roots, breadth first and in name order, each
/// directory once however many links lead to it. Unreadable places come
/// through as `Err` and the walk goes on.
struct FileWalk {
    queue: VecDeque<PathBuf>,
    visited: HashSet<PathBuf>,
}

impl FileWalk {
    fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            queue: roots.into(),
            visited: HashSet::new(),
        }
    }

    fn classify(&mut self, path: &Path) -> Result<Found, String> {
        let real = std::fs::canonicalize(path).map_err(io_failure("access", path))?;
        if !self.visited.insert(real) {
            return Ok(Found::Nothing);
        }
        let meta = std::fs::metadata(path).map_err(io_failure("stat", path))?;
        if !meta.is_dir() {
            return Ok(match meta.is_file() {
                true => Found::File,
                false => Found::Nothing,
            });
        }
        let kids = std::fs::read_dir(path)
            .map_err(io_failure("read", path))?
            .map(|item| item.map(|entry| entry.path()))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(io_failure("read entry in", path))?;
        Ok(Found::Dir(kids))
    }
}

impl Iterator for FileWalk {
    type Item = Result<PathBuf, String>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(path) = self.queue.pop_front() {
            match self.classify(&path) {
                Ok(Found::Nothing) => {}
                Ok(Found::File) => return Some(Ok(path)),
                Ok(Found::Dir(kids)) => self.queue.extend(kids),
                Err(why) => return Some(Err(why)),
            }
        }
        None
    }
}

pub(crate) fn has_extension(path: &Path, wanted: &str) -> bool {
    let ext = path.extension().and_then(|ext| ext.to_str());
    matches!(ext, Some(ext) if ext.eq_ignore_ascii_case(wanted))
}

fn is_ff(path: &Path) -> bool {
    has_extension(path, "ff")
}

fn is_named(path: &Path, wanted: &str) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    matches!(name, Some(name) if name.eq_ignore_ascii_case(wanted))
}

impl ZoneArchive {
    fn plain(path: PathBuf, zone: &str) -> Self {
        Self {
            path,
            zone_label: zone.into(),
            ..Self::default()
        }
    }
}

fn join_or_none(items: &[String]) -> String {
    Some(items)
        .filter(|list| !list.is_empty())
        .map_or_else(|| "none".to_owned(), |list| list.join("; "))
}

/// One file of the sought name, with what its head says.
struct Sighting {
    path: PathBuf,
    version: Result<u32, String>,
}

impl Sighting {
    fn reads_as(&self, version: u32) -> bool {
        self.version.as_ref().is_ok_and(|seen| *seen == version)
    }

    fn describe(&self) -> String {
        let shown = self.path.display();
        match &self.version {
            Ok(seen) => format!("{shown} (version {seen:#x})"),
            Err(why) => format!("{shown} ({why})"),
        }
    }
}

/// The files called `<zone>.ff` under some roots, collected until one reads
/// as the version wanted, plus the walk's own errors.
struct Hunt {
    zone_file: String,
    want: u32,
    sightings: Vec<Sighting>,
    scan_errors: Vec<String>,
}

impl Hunt {
    fn run(roots: Vec<PathBuf>, zone: &str, want: u32) -> Self {
        let mut hunt = Self {
            zone_file: format!("{zone}.ff"),
            want,
            sightings: Vec::new(),
            scan_errors: Vec::new(),
        };
        for item in FileWalk::new(roots) {
            let path = match item {
                Ok(path) if is_named(&path, &hunt.zone_file) => path,
                Ok(_) => continue,
                Err(why) => {
                    hunt.scan_errors.push(why);
                    continue;
                }
            };
            let version = envelope_version(&path);
            let sighting = Sighting { path, version };
            let enough = sighting.reads_as(want);
            hunt.sightings.push(sighting);
            if enough {
                break;
            }
        }
        hunt
    }

    fn winner(&self, zone: &str) -> Option<ZoneArchive> {
        let hit = self.sightings.iter().find(|s| s.reads_as(self.want))?;
        Some(ZoneArchive::plain(hit.path.clone(), zone))
    }
}

/// `zone` below `base` (and its shortcuts), preferring the version
/// this engine reads.
fn find_zone_under(base: &Path, zone: &str) -> Result<ZoneArchive, String> {
    let hunt = Hunt::run(game_roots(base), zone, T6_VERSION);
    if let Some(zone_file) = hunt.winner(zone) {
        return Ok(zone_file);
    }
    let name = &hunt.zone_file;
    let Some(first) = hunt.sightings.first() else {
        return Err(format!(
            "zone `{name}` not found under {}; scan errors: {}",
            base.display(),
            join_or_none(&hunt.scan_errors)
        ));
    };
    let at = first.path.display();
    Err(match first.version {
        Ok(version) => format!(
            "zone `{name}` found at {at} but version is {version:#x} (want {T6_VERSION:#x})"
        ),
        Err(_) => {
            format!("zone `{name}` at {at} is not a readable zone envelope (need {T6_VERSION:#x})")
        }
    })
}

/// `zone` anywhere under the games root, with exactly this envelope version.
pub(crate) fn zone_version_of(
    games: &GameInstallDir,
    zone: &str,
    wanted_version: u32,
) -> Result<ZoneArchive, String> {
    let hunt = Hunt::run(game_roots(&games.0), zone, wanted_version);
    hunt.winner(zone).ok_or_else(|| {
        let rejected: Vec<String> = (hunt.sightings.iter().map(Sighting::describe))
            .chain(hunt.scan_errors.iter().cloned())
            .collect();
        format!(
            "zone `{}` version {wanted_version:#x} not found; {}; other candidates: {}",
            hunt.zone_file,
            install_folders_summary(games),
            join_or_none(&rejected)
        )
    })
}

/// Split a `game:stem` key into the game and the stem. Anything else is
/// returned whole with no game.
pub fn zone_key_parts(key: &str) -> (Option<ZoneTitleKind>, &str) {
    match key.split_once(':') {
        Some((tag, rest)) if !rest.is_empty() => match ZoneTitleKind::from_prefix(tag) {
            Some(game) => (Some(game), rest),
            None => (None, key),
        },
        _ => (None, key),
    }
}

/// The name a map is shown under: `game:stem`, with the game taken from the
/// key itself or else from `game`.
pub fn loading_caption(zone_key: &str, game: Option<ZoneTitleKind>) -> String {
    let (from_key, base_name) = zone_key_parts(zone_key);
    let tag = from_key.or(game).map(ZoneTitleKind::prefix);
    [tag, Some(base_name)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(":")
}

/// One lookup by name, pinned to the game the key named (if any).
fn lookup_zone(games: &GameInstallDir, game: Option<ZoneTitleKind>, name: &str) -> Result<ZoneArchive, String> {
    match game {
        Some(ZoneTitleKind::T6) => zone_version_of(games, name, T6_VERSION),
        None => find_zone_under(&games.0, name),
    }
}

/// Record on `hit` that it was found under the `mp_` spelling of `stem`.
fn with_alias_note(mut hit: ZoneArchive, stem: &str, mp_name: &str) -> ZoneArchive {
    let note =
        format!("zone alias: `{stem}` -> `{mp_name}` (MP stem preferred over bare `{stem}.ff`)");
    let size = std::fs::metadata(&hit.path)
        .map(|meta| format!(" ({} bytes)", meta.len()))
        .unwrap_or_default();
    diag::info!(Zone, "{note}{size}");
    hit.nickname_remark = Some(note);
    hit
}

/// Find a zone by name or `game:name` key.
///
/// A bare name that is not already an `mp_` map is looked for as `mp_<name>`
/// first, and the alias is noted on the result.
pub fn locate_zone_file(games: &GameInstallDir, name: &str) -> Result<ZoneArchive, String> {
    let key = name.trim().to_ascii_lowercase();
    let (game, stem) = match zone_key_parts(&key) {
        (_, "") => return Err("zone name is empty".to_owned()),
        parsed => parsed,
    };
    if stem.starts_with("mp_") || stem.ends_with("_mp") {
        return lookup_zone(games, game, stem);
    }
    let mp_name = format!("mp_{stem}");
    lookup_zone(games, game, &mp_name)
        .map(|hit| with_alias_note(hit, stem, &mp_name))
        .or_else(|mp_error| {
            lookup_zone(games, game, stem).map_err(|bare_error| format!("{mp_error}; {bare_error}"))
        })
}

/// A zone that sits beside `sibling`, in the same game install.
pub fn locate_tree_zone(sibling: &Path, zone: &str) -> Result<ZoneArchive, String> {
    let tree = install_root_of_zone(sibling)?;
    find_zone_under(&tree, zone)
}

/// The shared zone a map in `map_zone`'s install loads with. A map of the game
/// this engine reads has none of its own to find.
pub fn locate_runtime_zone(
    _games: &GameInstallDir,
    map_zone: &Path,
    zone: &str,
) -> Result<ZoneArchive, String> {
    if title_of_zone_path(map_zone).is_some() {
        return Err(format!("{zone}: a Black Ops II map has no runtime zone"));
    }
    locate_tree_zone(map_zone, zone)
}

pub fn locate_shared_mp_zone(games: &GameInstallDir, map_zone: &Path) -> Result<ZoneArchive, String> {
    locate_runtime_zone(games, map_zone, "common_mp")
}

/// The install folder (the one with a `zone/` directory) above a zone file.
pub fn install_root_of_zone(file: &Path) -> Result<PathBuf, String> {
    let mut above = std::iter::successors(file.parent(), |dir| dir.parent());
    match above.find(|dir| dir.join("zone").is_dir()) {
        Some(root) => Ok(root.to_path_buf()),
        None => Err(format!("no game tree with a `zone/` directory above {}", file.display())),
    }
}

/// One line saying where the games root is, and where its shortcuts lead.
pub fn install_folders_summary(games: &GameInstallDir) -> String {
    let roots = search_roots(&games.0);
    roots
        .iter()
        .enumerate()
        .map(|(n, dir_v)| {
            let label = if n == 0 {
                "games root"
            } else {
                "; shortcut root"
            };
            format!("{label}: {}", dir_v.display())
        })
        .collect()
}

/// The first folder name below `dir` on the way to `path`, upper-cased.
fn first_folder_below(path: &Path, dir: &Path) -> Option<String> {
    let below = path.strip_prefix(dir).ok()?;
    let first = below.components().next()?;
    first.as_os_str().to_str().map(str::to_ascii_uppercase)
}

/// Which folder of its install a zone is in: `DLC<n>` or the base game.
fn pack_folder(zone_archive_file: &Path) -> String {
    let folder = install_root_of_zone(zone_archive_file)
        .ok()
        .and_then(|tree| first_folder_below(zone_archive_file, &tree.join("zone")));
    match folder {
        Some(name) if name.starts_with("DLC") => name,
        _ => "BASE".to_owned(),
    }
}

/// An `mp_*` zone file found on disk.
struct MapZone {
    game: &'static str,
    base_name: String,
    folder: String,
}

fn scan_mp_zones(root: &GameInstallDir) -> Vec<MapZone> {
    FileWalk::new(game_roots(&root.0))
        .filter_map(|entry| {
            entry
                .inspect_err(|why| diag::warn!(Zone, "map scan skipped an entry: {why}"))
                .ok()
        })
        .filter(|path| is_ff(path))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
            if !stem.starts_with("mp_") {
                return None;
            }
            let game = title_of_zone_path(&path)?.prefix();
            Some(MapZone {
                game,
                folder: pack_folder(&path),
                base_name: stem,
            })
        })
        .collect()
}

/// The `mp_*` zones that count as playable maps: loads are not maps, and in a
/// game that ships loads only the maps that have one count.
fn playable(zones: &[MapZone]) -> Vec<&MapZone> {
    let loads: HashSet<(&str, &str)> = zones
        .iter()
        .filter_map(|zone| Some((zone.game, zone.base_name.strip_suffix("_load")?)))
        .collect();
    let games_with_loads: HashSet<&str> = loads.iter().map(|(game, _)| *game).collect();
    zones
        .iter()
        .filter(|zone| !zone.base_name.ends_with("_load"))
        .filter(|zone| {
            !games_with_loads.contains(zone.game)
                || loads.contains(&(zone.game, zone.base_name.as_str()))
        })
        .collect()
}

/// The playable maps under the games root, grouped by install folder.
///
/// A map is any `mp_*` zone. Where an install ships `<map>_load` zones, only
/// maps that have one count, which leaves out the support zones that share
/// the `mp_` prefix.
pub fn list_mp_map_packs(root: &GameInstallDir) -> Vec<MapPack> {
    let zones = scan_mp_zones(root);
    let mut claimed = HashSet::new();
    let by_pack = playable(&zones)
        .into_iter()
        .filter(|zone| claimed.insert((zone.game, zone.base_name.as_str())))
        .fold(
            BTreeMap::<_, BTreeSet<String>>::new(),
            |mut by_pack, zone| {
                let map = format!("{}:{}", zone.game, zone.base_name);
                by_pack
                    .entry((zone.game, &zone.folder))
                    .or_default()
                    .insert(map);
                by_pack
            },
        );
    by_pack
        .into_iter()
        .map(|((title_tag, folder), maps)| MapPack {
            label: format!("{} {folder}", title_tag.to_ascii_uppercase()),
            maps: maps.into_iter().collect(),
        })
        .collect()
}

/// Every playable map under the games root, sorted.
pub fn multiplayer_map_names(games: &GameInstallDir) -> Vec<String> {
    list_mp_map_packs(games)
        .into_iter()
        .flat_map(|pack| pack.maps)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The folder generated files go in, made if it is not there.
pub fn prepare_output_folder() -> Result<PathBuf, String> {
    let dir_v = PathBuf::from(ARTIFACTS_DIR);
    match std::fs::create_dir_all(&dir_v) {
        Ok(()) => Ok(dir_v),
        Err(error) => Err(format!("cannot create {}: {error}", dir_v.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bo2mp-find-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An empty `IWff` envelope of `version`.
    fn put_zone(dir: &Path, name: &str, version: u32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let mut head = b"IWff0100".to_vec();
        head.extend_from_slice(&version.to_le_bytes());
        let zone_file = dir.join(name);
        std::fs::write(&zone_file, head).unwrap();
        zone_file
    }

    #[test]
    fn keys_split_into_game_and_stem() {
        assert_eq!(zone_key_parts("t6:mp_x"), (Some(ZoneTitleKind::T6), "mp_x"));
        assert_eq!(zone_key_parts("t6:"), (None, "t6:"));
        assert_eq!(zone_key_parts("zz:mp_x"), (None, "zz:mp_x"));
        assert_eq!(zone_key_parts("mp_x"), (None, "mp_x"));
        assert_eq!(loading_caption("mp_x", Some(ZoneTitleKind::T6)), "t6:mp_x");
        assert_eq!(loading_caption("t6:mp_x", None), "t6:mp_x");
        assert_eq!(loading_caption("mp_x", None), "mp_x");
    }

    #[test]
    fn the_mp_stem_wins_over_the_bare_one() {
        let dir = scratch("alias");
        put_zone(&dir.join("g/zone/all"), "mp_hub.ff", T6_VERSION);
        put_zone(&dir.join("g/zone/all"), "hub.ff", T6_VERSION);
        let root = GameInstallDir(dir.clone());

        let found = locate_zone_file(&root, " HUB ").unwrap();
        assert!(found.path.ends_with("mp_hub.ff"));
        assert!(found.nickname_remark.is_some());
        let exact = locate_zone_file(&root, "t6:hub").unwrap();
        assert!(exact.path.ends_with("mp_hub.ff"));
        assert!(locate_zone_file(&root, "").is_err());
        assert!(
            locate_zone_file(&root, "t6:nope")
                .unwrap_err()
                .contains("not found")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zone_of_another_version_is_reported_not_returned() {
        let dir = scratch("version");
        put_zone(&dir.join("g/zone"), "old.ff", 0x1d9);
        let error = locate_zone_file(&GameInstallDir(dir.clone()), "old").unwrap_err();
        assert!(error.contains("0x1d9"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
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
        let root = GameInstallDir(dir.clone());

        let packs = list_mp_map_packs(&root);
        let labels: Vec<_> = packs.iter().map(|pack| pack.label.as_str()).collect();
        assert_eq!(labels, ["T6 BASE", "T6 DLC1"]);
        assert_eq!(packs[0].maps, ["t6:mp_a"]);
        assert_eq!(multiplayer_map_names(&root), ["t6:mp_a", "t6:mp_b"]);
        assert_eq!(
            install_root_of_zone(&base.join("mp_a.ff")).unwrap(),
            dir.join("g")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_walk_goes_breadth_first_in_name_order() {
        let dir = scratch("walk");
        put_zone(&dir.join("b"), "deep.ff", T6_VERSION);
        put_zone(&dir.join("a"), "one.ff", T6_VERSION);
        put_zone(&dir, "top.ff", T6_VERSION);
        let order: Vec<PathBuf> = FileWalk::new(vec![dir.clone(), dir.clone()])
            .map(|item| item.unwrap().strip_prefix(&dir).unwrap().to_path_buf())
            .collect();
        let expected: Vec<PathBuf> = ["top.ff", "a/one.ff", "b/deep.ff"]
            .map(PathBuf::from)
            .into();
        assert_eq!(order, expected);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_start_comes_through_as_an_error() {
        let dir = scratch("missing");
        let mut walk = FileWalk::new(vec![dir.join("nowhere")]);
        assert!(
            walk.next()
                .unwrap()
                .unwrap_err()
                .starts_with("cannot access")
        );
        assert!(walk.next().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn heads_that_are_not_envelopes_are_refused() {
        let dir = scratch("heads");
        let ok = put_zone(&dir, "ok.ff", 7);
        assert_eq!(envelope_version(&ok), Ok(7));
        let junk = dir.join("junk.ff");
        std::fs::write(&junk, b"NOPE00000000").unwrap();
        assert!(
            envelope_version(&junk)
                .unwrap_err()
                .contains("not a zone envelope")
        );
        let short = dir.join("short.ff");
        std::fs::write(&short, b"IWff").unwrap();
        assert!(envelope_version(&short).is_err());
        assert_eq!(
            title_of_zone_path(&put_zone(&dir, "t6.ff", T6_VERSION)),
            Some(ZoneTitleKind::T6)
        );
        assert_eq!(title_of_zone_path(&ok), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn small_name_helpers_ignore_case() {
        assert!(is_ff(Path::new("a/B.FF")));
        assert!(!is_ff(Path::new("a/b.fff")));
        assert!(is_named(Path::new("x/Mp_Hub.FF"), "mp_hub.ff"));
        assert!(!is_named(Path::new("x/hub.ff"), "mp_hub.ff"));
        assert_eq!(join_or_none(&[]), "none");
        assert_eq!(join_or_none(&["a".to_owned(), "b".to_owned()]), "a; b");
    }

    #[test]
    fn the_report_lists_the_games_root_first() {
        let dir = scratch("report");
        let report = install_folders_summary(&GameInstallDir(dir.clone()));
        assert!(report.starts_with("games root: "), "{report}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
