//! Which content trees are installed, and the loose sounds each one holds.

use std::path::{Path, PathBuf};

use asset_core::AssetCategoryScope;

use crate::install_scan::{GameInstallDir, zone_version_of, title_of_zone_path};
use crate::sound_pack::{SoundArchiveIndex, main_dir_for_zone};

/// Every namespace this module tracks; a namespace's position here is its
/// cell in the per-namespace arrays below.
const TRACKED: [AssetCategoryScope; 2] = [AssetCategoryScope::T5, AssetCategoryScope::T6];

/// Zone names, tried in order, that exist in every install of `ns`.
fn anchor_zone_names(ns: AssetCategoryScope) -> &'static [&'static str] {
    match ns {
        AssetCategoryScope::T5 => &["common_mp"],
        _ => &["common_mp", "common_zm"],
    }
}

/// The envelope version stamped on the zone files of `ns`.
fn envelope_version(ns: AssetCategoryScope) -> u32 {
    if ns == AssetCategoryScope::T5 {
        0x1d9
    } else {
        fastfile_t6::T6_PC_ZONE_REV
    }
}

fn slot(wanted: AssetCategoryScope) -> usize {
    TRACKED
        .iter()
        .position(|&ns| ns == wanted)
        .expect("every namespace has a cell")
}

/// The anchor zone file of `ns` under `root`: the first anchor name that
/// exists with the namespace's envelope version.
fn find_anchor(root: &GameInstallDir, ns: AssetCategoryScope) -> Option<PathBuf> {
    let version = envelope_version(ns);
    for zone_name in anchor_zone_names(ns) {
        if let Ok(zone) = zone_version_of(root, zone_name, version) {
            return Some(zone.path);
        }
    }
    None
}

/// One install: the zone that identified it and its `main/` directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryTree {
    pub root_dir: PathBuf,
    pub main: Option<PathBuf>,
}

impl CategoryTree {
    fn around(zone: PathBuf) -> Self {
        Self { main: main_dir_for_zone(&zone).ok(), root_dir: zone }
    }

    /// The report line for a namespace with (or without) an install.
    fn describe(label: &str, tree: Option<&Self>) -> String {
        let Some(Self { root_dir: anchor, main }) = tree else {
            return format!("namespace tree {label}: not installed");
        };
        let main = main.as_deref().map(|dir| dir.display().to_string());
        format!(
            "namespace tree {label}: main={} anchor={}",
            main.as_deref().unwrap_or("-"),
            anchor.display()
        )
    }
}

/// The install found for each namespace, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CategoryTreeSet {
    trees: [Option<CategoryTree>; TRACKED.len()],
}

impl CategoryTreeSet {
    pub fn discover(root: &GameInstallDir) -> Self {
        Self {
            trees: TRACKED.map(|ns| find_anchor(root, ns).map(CategoryTree::around)),
        }
    }

    /// Make the install holding `zone_archive_file` the one for its namespace.
    pub fn take_zone_root(&mut self, zone_archive_file: &Path) {
        let Some(kind) = title_of_zone_path(zone_archive_file) else {
            return;
        };
        let cell = slot(AssetCategoryScope::from_zone_game(kind));
        self.trees[cell] = CategoryTree::around(zone_archive_file.to_owned()).into();
    }

    pub fn get(&self, ns: AssetCategoryScope) -> Option<&CategoryTree> {
        self.trees[slot(ns)].as_ref()
    }

    pub fn is_empty(&self) -> bool {
        !self.trees.iter().any(Option::is_some)
    }

    /// One line per namespace for the load report.
    pub fn summary_rows(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(TRACKED.len());
        for ns in TRACKED {
            lines.push(CategoryTree::describe(ns.as_str(), self.get(ns)));
        }
        lines
    }
}

/// Open the sound index of one tree; the error is the full line to log.
fn open_tree_index(name: &str, tree: &CategoryTree) -> Result<SoundArchiveIndex, String> {
    let anchor = tree.root_dir.display();
    let archive_dir = tree
        .main
        .as_deref()
        .ok_or_else(|| format!("audio: IWD sounds {name}: tree {anchor} has no archive directory"))?;
    SoundArchiveIndex::open(archive_dir)
        .map_err(|error| format!("audio: IWD sounds {name} unavailable ({error})"))
}

/// The line to log for an index that opened.
fn opened_line(name: &str, index: &SoundArchiveIndex) -> String {
    let (dir, paths) = (index.numbered_folder().display(), index.audio_total());
    format!("audio: IWD sounds {name} from {dir} — {paths} paths")
}

/// The loose-sound index of each installed namespace.
#[derive(Debug, Default)]
pub struct CategorySoundArchive {
    indices: [Option<SoundArchiveIndex>; TRACKED.len()],
}

impl CategorySoundArchive {
    /// Index the sounds of every tree in `trees`. The second value is what to
    /// log about each one.
    pub fn open(trees: &CategoryTreeSet) -> (Self, Vec<String>) {
        let mut journal = Vec::new();
        let indices = TRACKED.map(|ns| {
            let tree = trees.get(ns)?;
            let opened = open_tree_index(ns.as_str(), tree);
            journal.push(match &opened {
                Ok(index) => opened_line(ns.as_str(), index),
                Err(line) => line.clone(),
            });
            opened.ok()
        });
        (Self { indices }, journal)
    }

    /// The sound at `relative` (below `sound/`) from the namespace's archives.
    pub fn fetch_audio_clip(
        &self,
        ns: AssetCategoryScope,
        clip_path: &str,
    ) -> Option<Result<Vec<u8>, String>> {
        let index = self.indices[slot(ns)].as_ref()?;
        index.fetch_audio_clip(clip_path)
    }

    pub fn is_empty(&self) -> bool {
        !self.indices.iter().any(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_outside_any_install_has_an_anchor_and_no_main() {
        let lone = std::env::temp_dir().join("bo2mp-lone-zone-anchor").join("x.ff");
        let tree = CategoryTree::around(lone.clone());
        assert_eq!(tree.root_dir, lone);
        assert_eq!(tree.main, main_dir_for_zone(&lone).ok());
    }

    /// A fresh scratch folder holding a T6 install: `game/main` plus one
    /// anchor zone stamped with the T6 envelope version.
    fn t6_install(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("bo2mp-trees-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let zone_dir = base.join("game/zone/all");
        for dir in [base.join("game/main"), zone_dir.clone()] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let stamp = [
            b"IWff0100".as_slice(),
            &fastfile_t6::T6_PC_ZONE_REV.to_le_bytes(),
        ]
        .concat();
        std::fs::write(zone_dir.join("common_zm.ff"), stamp).unwrap();
        base
    }

    #[test]
    fn only_the_installed_namespace_is_found() {
        let base = t6_install("found");
        let found = CategoryTreeSet::discover(&GameInstallDir(base.clone()));
        let t6 = found.get(AssetCategoryScope::T6).expect("t6 present");
        assert_eq!(t6.main, Some(base.join("game/main")));
        assert_eq!(
            (found.is_empty(), found.get(AssetCategoryScope::T5)),
            (false, None)
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_report_has_a_line_per_namespace() {
        let base = t6_install("report");
        let lines = CategoryTreeSet::discover(&GameInstallDir(base.clone())).summary_rows();
        assert_eq!(lines.len(), TRACKED.len());
        assert!(
            lines[slot(AssetCategoryScope::T5)].ends_with("t5: not installed"),
            "{lines:?}"
        );
        assert!(
            lines[slot(AssetCategoryScope::T6)].starts_with("namespace tree t6: main="),
            "{lines:?}"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_empty_tree_set_reports_missing_main() {
        let line =
            CategoryTree::describe("t6", Some(&CategoryTree::around(PathBuf::from("zone.ff"))));
        assert_eq!(line, "namespace tree t6: main=- anchor=zone.ff");
        assert!(CategoryTreeSet::default().is_empty());
        assert!(CategorySoundArchive::default().is_empty());
    }

    #[test]
    fn sound_indexing_logs_one_line_and_finds_nothing_loose() {
        let base = t6_install("sounds");
        let trees = CategoryTreeSet::discover(&GameInstallDir(base.clone()));
        let (audio, journal) = CategorySoundArchive::open(&trees);
        assert_eq!(journal.len(), 1);
        assert!(journal[0].contains("0 paths"), "{journal:?}");
        assert!(!audio.is_empty());
        for ns in TRACKED {
            assert!(audio.fetch_audio_clip(ns, "a.wav").is_none());
        }
        std::fs::remove_dir_all(&base).ok();
    }
}
