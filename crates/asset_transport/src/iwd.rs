//! IWD archives: zip files holding a game's loose sounds and config.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::discover::search_roots;

type Reader = zip::ZipArchive<BufReader<File>>;

fn has_iwd_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("iwd"))
}

fn archives_in(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let listing = std::fs::read_dir(directory)
        .map_err(|e| format!("cannot read {}: {e}", directory.display()))?;
    let mut archives: Vec<PathBuf> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| has_iwd_extension(path))
        .collect();
    archives.sort();
    Ok(archives)
}

/// The `main/` directory of the game install a zone sits in.
pub fn game_main_for_zone(zone_ff: &Path) -> Result<PathBuf, String> {
    zone_ff
        .ancestors()
        .map(|ancestor| ancestor.join("main"))
        .find(|main| main.is_dir())
        .ok_or_else(|| {
            format!(
                "no game tree with a `main/` directory above {}",
                zone_ff.display()
            )
        })
}

/// The first file called `want` (a `/` path inside the archive, compared
/// without regard to case) in any IWD under the games root.
pub fn read_iwd_named(games_root: &Path, want: &str) -> Option<Vec<u8>> {
    let want = want.replace('\\', "/");
    let mut pending = search_roots(games_root);
    while let Some(dir) = pending.pop() {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = listing.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !has_iwd_extension(&path) {
                continue;
            }
            let Ok(file) = File::open(&path) else {
                continue;
            };
            let Ok(mut archive) = zip::ZipArchive::new(file) else {
                continue;
            };
            for position in 0..archive.len() {
                let Ok(mut entry) = archive.by_index(position) else {
                    continue;
                };
                if !entry.name().replace('\\', "/").eq_ignore_ascii_case(&want) {
                    continue;
                }
                let mut bytes = Vec::new();
                if entry.read_to_end(&mut bytes).is_ok() {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

/// One IWD, with readers kept between calls: opening a zip means parsing its
/// directory, and sounds are asked for from several threads at once.
struct Archive {
    path: PathBuf,
    idle: Mutex<Vec<Reader>>,
}

/// Readers kept open per archive.
const IDLE_READERS: usize = 4;

impl Archive {
    fn read(&self, entry_name: &str) -> Result<Vec<u8>, String> {
        let parked = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let mut reader = match parked {
            Some(reader) => reader,
            None => {
                let file = File::open(&self.path)
                    .map_err(|e| format!("cannot open {}: {e}", self.path.display()))?;
                zip::ZipArchive::new(BufReader::new(file))
                    .map_err(|e| format!("cannot read {}: {e}", self.path.display()))?
            }
        };
        let bytes = {
            let mut entry = reader
                .by_name(entry_name)
                .map_err(|e| format!("cannot read {entry_name}: {e}"))?;
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read {entry_name}: {e}"))?;
            bytes
        };
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < IDLE_READERS {
            idle.push(reader);
        }
        Ok(bytes)
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
pub struct IwdSoundIndex {
    dir: PathBuf,
    archives: Vec<Arc<Archive>>,
    /// Sound path under `sound/` (lowercase, `/`) to its archive and entry.
    sounds: HashMap<String, (usize, String)>,
}

/// The language a `localized_<language>_<n>` archive carries; its entries sit
/// under a folder of that name.
fn archive_language(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let (language, _) = stem.strip_prefix("localized_")?.rsplit_once('_')?;
    Some(language.to_owned())
}

/// `sound/<rest>` entries as `<rest>`, with a leading language folder removed.
fn sound_path(entry_name: &str, language: Option<&str>) -> Option<String> {
    let lower = entry_name.replace('\\', "/").to_ascii_lowercase();
    let unrooted = language
        .and_then(|language| lower.strip_prefix(language)?.strip_prefix('/'))
        .unwrap_or(&lower);
    unrooted
        .strip_prefix("sound/")
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned)
}

impl IwdSoundIndex {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, String> {
        let directory = directory.as_ref();
        let mut index = Self {
            dir: directory.to_path_buf(),
            ..Self::default()
        };
        for path in archives_in(directory)? {
            let file = File::open(&path)
                .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
            let reader = zip::ZipArchive::new(BufReader::new(file))
                .map_err(|e| format!("zip {}: {e}", path.display()))?;
            let language = archive_language(&path);
            let slot = index.archives.len();
            for name in reader.file_names() {
                if let Some(rest) = sound_path(name, language.as_deref()) {
                    index
                        .sounds
                        .entry(rest)
                        .or_insert_with(|| (slot, name.to_owned()));
                }
            }
            index.archives.push(Arc::new(Archive {
                path,
                idle: Mutex::new(vec![reader]),
            }));
        }
        Ok(index)
    }

    pub fn indexed_dir(&self) -> &Path {
        &self.dir
    }

    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    /// The bytes of the sound at `relative` (below `sound/`), or `None` when
    /// no archive holds it.
    pub fn read_sound(&self, relative: &str) -> Option<Result<Vec<u8>, String>> {
        let key = relative.replace('\\', "/").to_ascii_lowercase();
        let (slot, entry) = self.sounds.get(&key)?;
        Some(self.archives[*slot].read(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_iwd(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bo2mp-iwd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sound_paths_drop_the_root_and_language_folder() {
        assert_eq!(sound_path("sound\\Misc\\Beep.wav", None).as_deref(), Some("misc/beep.wav"));
        assert_eq!(
            sound_path("English/sound/vo/hi.wav", Some("english")).as_deref(),
            Some("vo/hi.wav")
        );
        assert_eq!(sound_path("images/a.iwi", None), None);
        assert_eq!(sound_path("sound/", None), None);
        assert_eq!(
            archive_language(Path::new("localized_french_2.iwd")).as_deref(),
            Some("french")
        );
        assert_eq!(archive_language(Path::new("iw_00.iwd")), None);
    }

    #[test]
    fn the_first_archive_in_name_order_wins() {
        let dir = scratch("order");
        write_iwd(&dir.join("a.iwd"), &[("sound/x/one.wav", b"from-a")]);
        write_iwd(
            &dir.join("b.iwd"),
            &[("sound/x/one.wav", b"from-b"), ("sound/x/two.wav", b"two")],
        );
        let index = IwdSoundIndex::open(&dir).unwrap();
        assert_eq!(index.sound_count(), 2);
        assert_eq!(index.read_sound("X\\One.wav").unwrap().unwrap(), b"from-a");
        assert_eq!(index.read_sound("x/two.wav").unwrap().unwrap(), b"two");
        assert_eq!(index.read_sound("x/two.wav").unwrap().unwrap(), b"two");
        assert!(index.read_sound("x/none.wav").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_named_file_is_found_below_the_games_root() {
        let dir = scratch("named");
        std::fs::create_dir_all(dir.join("game/main")).unwrap();
        write_iwd(&dir.join("game/main/cfg.iwd"), &[("Config/Match.cfg", b"set x 1")]);
        assert_eq!(read_iwd_named(&dir, "config/match.cfg").unwrap(), b"set x 1");
        assert!(read_iwd_named(&dir, "config/other.cfg").is_none());
        let zone = dir.join("game/zone/all/mp_x.ff");
        assert_eq!(game_main_for_zone(&zone).unwrap(), dir.join("game/main"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
