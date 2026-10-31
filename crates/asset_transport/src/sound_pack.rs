//! IWD archives: zip files holding a game's loose sounds and config.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::install_scan::{has_extension, search_roots};

fn has_iwd_extension(path: &Path) -> bool {
    has_extension(path, "iwd")
}

/// Open `path` as a zip; `parse_label` prefixes the message when the zip
/// directory cannot be parsed.
fn open_reader(path: &Path, parse_label: &str) -> Result<zip::ZipArchive<BufReader<File>>, String> {
    File::open(path)
        .map_err(|why| format!("cannot open {}: {why}", path.to_string_lossy()))
        .and_then(|file| {
            zip::ZipArchive::new(BufReader::new(file))
                .map_err(|e| format!("{parse_label} {}: {e}", path.to_string_lossy()))
        })
}

/// Every readable child path of `directory`, in name order.
fn listing(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let children: BTreeSet<PathBuf> = std::fs::read_dir(directory)?
        .flatten()
        .map(|dirent| dirent.path())
        .collect();
    Ok(children.into_iter().collect())
}

fn archives_in(directory: &Path) -> Result<Vec<PathBuf>, String> {
    match listing(directory) {
        Ok(children) => Ok(children
            .into_iter()
            .filter(|p| has_iwd_extension(p))
            .collect()),
        Err(e) => Err(format!("cannot read {}: {e}", directory.display())),
    }
}

/// The `main/` directory of the game install a zone sits in.
pub fn main_dir_for_zone(zone_archive_file: &Path) -> Result<PathBuf, String> {
    let root_above = |dir: &Path| {
        let found = dir.ancestors().find(|above| above.join("main").is_dir());
        found.map(Path::to_path_buf)
    };
    // bo2zm: a map zone from an extra folder (`BO2MP_T6_EXTRA`) uses the install's game tree.
    let game_root = root_above(zone_archive_file)
        .or_else(|| root_above(&crate::t6_extra::install_zone_dir(zone_archive_file)));
    game_root
        .map(|root| root.join("main"))
        .ok_or_else(|| format!("no game tree with a `main/` directory above {}", zone_archive_file.display()))
}

/// The bytes of the member of the zip at `archive` whose name equals `wanted`
/// (already `/`-separated), ignoring case.
fn member_bytes(archive: &Path, wanted: &str) -> Option<Vec<u8>> {
    let mut zip_file = zip::ZipArchive::new(File::open(archive).ok()?).ok()?;
    let hit = (0..zip_file.len()).find(|&index| {
        zip_file.by_index(index).is_ok_and(|member| {
            member
                .name()
                .replace('\\', "/")
                .eq_ignore_ascii_case(wanted)
        })
    })?;
    let mut member = zip_file.by_index(hit).ok()?;
    let mut contents = Vec::with_capacity(member.size() as usize);
    member.read_to_end(&mut contents).ok().map(|_| contents)
}

/// Every IWD file under the search roots, depth-first: a folder's own
/// archives come out before anything in its subfolders.
fn iwd_files_below(titles_install_dir: &Path) -> impl Iterator<Item = PathBuf> {
    IwdWalk {
        folders: search_roots(titles_install_dir),
        ready: Vec::new(),
    }
}

/// The state of the depth-first walk behind `iwd_files_below`.
struct IwdWalk {
    folders: Vec<PathBuf>,
    /// The current folder's archives, last to come out first.
    ready: Vec<PathBuf>,
}

impl Iterator for IwdWalk {
    type Item = PathBuf;

    fn next(&mut self) -> Option<PathBuf> {
        while self.ready.is_empty() {
            let folder = self.folders.pop()?;
            let (subfolders, mut archives): (Vec<_>, Vec<_>) = listing(&folder)
                .unwrap_or_default()
                .into_iter()
                .partition(|child| child.is_dir());
            self.folders.extend(subfolders);
            archives.retain(|child| has_iwd_extension(child));
            // Taken from the back, so reverse to keep name order.
            archives.reverse();
            self.ready = archives;
        }
        self.ready.pop()
    }
}

/// The first file called `want` (a `/` path inside the archive, compared
/// without regard to case) in any IWD under the games root.
pub fn read_sound_pack_member(titles_install_dir: &Path, member_name: &str) -> Option<Vec<u8>> {
    let wanted = member_name.replace('\\', "/");
    iwd_files_below(titles_install_dir).find_map(|archive| member_bytes(&archive, &wanted))
}

/// One IWD, with readers kept between calls: opening a zip means parsing its
/// directory, and sounds are asked for from several threads at once.
struct Archive {
    path: PathBuf,
    idle: Mutex<Vec<zip::ZipArchive<BufReader<File>>>>,
}

/// Readers kept open per archive.
const IDLE_READERS: usize = 4;

impl Archive {
    /// Run `act` on the idle-reader pool while holding its lock.
    fn with_pool<T>(&self, act: impl FnOnce(&mut Vec<zip::ZipArchive<BufReader<File>>>) -> T) -> T {
        act(&mut self.idle.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn checkout(&self) -> Result<zip::ZipArchive<BufReader<File>>, String> {
        match self.with_pool(Vec::pop) {
            None => open_reader(&self.path, "cannot read"),
            Some(reader) => Ok(reader),
        }
    }

    fn checkin(&self, reader: zip::ZipArchive<BufReader<File>>) {
        self.with_pool(|pool| {
            if pool.len() < IDLE_READERS {
                pool.push(reader);
            }
        });
    }

    fn read(&self, member: &str) -> Result<Vec<u8>, String> {
        let mut reader = self.checkout()?;
        let bytes = extract_member(&mut reader, member)
            .map_err(|why| format!("cannot read {member}: {why}"))?;
        // A reader that failed is dropped rather than returned to the pool.
        self.checkin(reader);
        Ok(bytes)
    }
}

/// The whole contents of the member `entry_name` of an open zip.
fn extract_member(reader: &mut zip::ZipArchive<BufReader<File>>, member: &str) -> Result<Vec<u8>, String> {
    let mut member = reader.by_name(member).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(member.size() as usize);
    match std::io::copy(&mut member, &mut bytes) {
        Ok(_) => Ok(bytes),
        Err(e) => Err(e.to_string()),
    }
}

impl fmt::Debug for Archive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Archive").field(&self.path).finish()
    }
}

/// Where each loose sound of one `main/` directory lives.
///
/// Archives are taken in name order and the first one holding a sound wins.
#[derive(Debug, Default, Clone)]
pub struct SoundArchiveIndex {
    base_dir: PathBuf,
    archives: Vec<Arc<Archive>>,
    /// Sound path under `sound/` (lowercase, `/`) to where it is stored.
    by_path: HashMap<String, SoundHome>,
}

/// Which archive holds a sound, and the member name inside it.
#[derive(Debug, Clone)]
struct SoundHome {
    archive: usize,
    member: String,
}

/// A path in the form the index keys use: `/` separators, lowercase.
fn fold_path(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '\\' => '/',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// The language a `localized_<language>_<n>` archive carries; its entries sit
/// under a folder of that name.
fn archive_language(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    stem.strip_prefix("localized_")
        .and_then(|tail| tail.rsplit_once('_'))
        .map(|(language, _serial)| language.to_owned())
}

/// `sound/<rest>` entries as `<rest>`, with a leading language folder removed.
fn sound_path(entry_name: &str, language: Option<&str>) -> Option<String> {
    let folded = fold_path(entry_name);
    let mut path = folded.as_str();
    if let Some(inside) = language
        .and_then(|lang| path.strip_prefix(lang))
        .and_then(|after| after.strip_prefix('/'))
    {
        path = inside;
    }
    match path.strip_prefix("sound/") {
        Some("") | None => None,
        Some(rest) => Some(rest.to_owned()),
    }
}

impl SoundArchiveIndex {
    pub fn open(install_root: impl AsRef<Path>) -> Result<Self, String> {
        let base_dir = install_root.as_ref().to_path_buf();
        let found = archives_in(&base_dir)?;
        let mut index = Self { base_dir, ..Self::default() };
        index.archives.reserve(found.len());
        found.into_iter().try_for_each(|archive_path| index.add_archive(archive_path))?;
        Ok(index)
    }

    /// Index one archive's sounds (earlier archives keep any path they hold)
    /// and keep its already-parsed reader for the first read.
    fn add_archive(&mut self, archive_path: PathBuf) -> Result<(), String> {
        let parsed = open_reader(&archive_path, "zip")?;
        let language = archive_language(&archive_path);
        let archive = self.archives.len();
        for name in parsed.file_names() {
            let Some(rest) = sound_path(name, language.as_deref()) else {
                continue;
            };
            self.by_path.entry(rest).or_insert_with(|| SoundHome {
                archive,
                member: name.to_owned(),
            });
        }
        self.archives.push(Arc::new(Archive {
            path: archive_path,
            idle: Mutex::new(vec![parsed]),
        }));
        Ok(())
    }

    pub fn numbered_folder(&self) -> &Path {
        &self.base_dir
    }

    pub fn audio_total(&self) -> usize {
        self.by_path.len()
    }

    /// The bytes of the sound at `relative` (below `sound/`), or `None` when
    /// no archive holds it.
    pub fn fetch_audio_clip(&self, clip_path: &str) -> Option<Result<Vec<u8>, String>> {
        let home = self.by_path.get(&fold_path(clip_path))?;
        Some(self.archives[home.archive].read(&home.member))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A zip at `path` holding the given `(member name, payload)` pairs.
    fn write_iwd(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        entries.iter().for_each(|&(member, payload)| {
            writer.start_file(member, options).unwrap();
            writer.write_all(payload).unwrap();
        });
        writer.finish().unwrap();
    }

    /// A fresh empty folder, removed again when the guard drops.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bo2mp-iwd-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("scratch folder");
            Self(dir)
        }

        fn join(&self, tail: &str) -> PathBuf {
            self.0.join(tail)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn sound_paths_drop_the_root_and_language_folder() {
        let cases = [
            ("sound\\Misc\\Beep.wav", None, Some("misc/beep.wav")),
            (
                "English/sound/vo/hi.wav",
                Some("english"),
                Some("vo/hi.wav"),
            ),
            ("images/a.iwi", None, None),
            ("sound/", None, None),
        ];
        for (entry, language, expected) in cases {
            assert_eq!(sound_path(entry, language).as_deref(), expected, "{entry}");
        }
    }

    #[test]
    fn localized_archives_name_their_language() {
        let language_of = |file: &str| archive_language(Path::new(file));
        assert_eq!(
            language_of("localized_french_2.iwd").as_deref(),
            Some("french")
        );
        assert_eq!(language_of("iw_00.iwd"), None);
    }

    #[test]
    fn index_keys_use_forward_slashes_and_lowercase() {
        assert_eq!(fold_path("X\\One.WAV"), "x/one.wav");
        assert_eq!(fold_path(""), "");
    }

    #[test]
    fn the_first_archive_in_name_order_wins() {
        let dir = TempTree::new("order");
        write_iwd(&dir.join("a.iwd"), &[("sound/x/one.wav", b"from-a")]);
        write_iwd(
            &dir.join("b.iwd"),
            &[("sound/x/one.wav", b"from-b"), ("sound/x/two.wav", b"two")],
        );
        let index = SoundArchiveIndex::open(&dir.0).unwrap();
        assert_eq!(index.audio_total(), 2);
        let read = |name: &str| index.fetch_audio_clip(name).map(Result::unwrap);
        assert_eq!(read("X\\One.wav"), Some(b"from-a".to_vec()));
        // Twice, so the second read comes through a pooled reader.
        assert_eq!(read("x/two.wav"), Some(b"two".to_vec()));
        assert_eq!(read("x/two.wav"), Some(b"two".to_vec()));
        assert_eq!(read("x/none.wav"), None);
    }

    #[test]
    fn a_named_file_is_found_below_the_games_root() {
        let dir = TempTree::new("named");
        std::fs::create_dir_all(dir.join("game/main")).unwrap();
        write_iwd(
            &dir.join("game/main/cfg.iwd"),
            &[("Config/Match.cfg", b"set x 1")],
        );
        assert_eq!(
            read_sound_pack_member(&dir.0, "config/match.cfg"),
            Some(b"set x 1".to_vec())
        );
        assert_eq!(read_sound_pack_member(&dir.0, "config/other.cfg"), None);
    }

    #[test]
    fn the_main_directory_is_looked_for_above_the_zone() {
        let dir = TempTree::new("main");
        std::fs::create_dir_all(dir.join("game/main")).unwrap();
        let zone = dir.join("game/zone/all/mp_x.ff");
        assert_eq!(main_dir_for_zone(&zone), Ok(dir.join("game/main")));
    }
}
