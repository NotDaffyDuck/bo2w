//! Which content trees are installed, and the loose sounds each one holds.

use std::path::{Path, PathBuf};

use asset_core::AssetNamespace;

use crate::discover::{GamesRoot, find_zone_file_version, zone_game_for_path};
use crate::iwd::{IwdSoundIndex, game_main_for_zone};

/// How one namespace is recognised on disk.
struct Namespace {
    ns: AssetNamespace,
    /// Zones that sit in every install of it, tried in order.
    anchors: &'static [&'static str],
    /// The zone envelope version of its files.
    version: u32,
}

const NAMESPACES: [Namespace; 2] = [
    Namespace {
        ns: AssetNamespace::T5,
        anchors: &["common_mp"],
        version: 0x1d9,
    },
    Namespace {
        ns: AssetNamespace::T6,
        anchors: &["common_mp", "common_zm"],
        version: fastfile_t6::ZONE_VERSION_PC,
    },
];

fn slot(ns: AssetNamespace) -> usize {
    NAMESPACES
        .iter()
        .position(|row| row.ns == ns)
        .expect("every namespace has a row")
}

/// One install: the zone that identified it and its `main/` directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceTree {
    pub anchor: PathBuf,
    pub main: Option<PathBuf>,
}

impl NamespaceTree {
    fn from_anchor(anchor: PathBuf) -> Self {
        let main = game_main_for_zone(&anchor).ok();
        Self { anchor, main }
    }
}

/// The install found for each namespace, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamespaceTrees {
    trees: [Option<NamespaceTree>; NAMESPACES.len()],
}

impl NamespaceTrees {
    pub fn discover(root: &GamesRoot) -> Self {
        let mut found = Self::default();
        for row in &NAMESPACES {
            found.trees[slot(row.ns)] = row
                .anchors
                .iter()
                .find_map(|anchor| find_zone_file_version(root, anchor, row.version).ok())
                .map(|zone| NamespaceTree::from_anchor(zone.path));
        }
        found
    }

    /// Make the install holding `zone_ff` the one for its namespace.
    pub fn adopt_zone(&mut self, zone_ff: &Path) {
        if let Some(game) = zone_game_for_path(zone_ff) {
            let ns = AssetNamespace::from_zone_game(game);
            self.trees[slot(ns)] = Some(NamespaceTree::from_anchor(zone_ff.to_path_buf()));
        }
    }

    pub fn get(&self, ns: AssetNamespace) -> Option<&NamespaceTree> {
        self.trees[slot(ns)].as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.trees.iter().all(Option::is_none)
    }

    /// One line per namespace for the load report.
    pub fn report_lines(&self) -> Vec<String> {
        NAMESPACES
            .iter()
            .map(|row| match self.get(row.ns) {
                Some(tree) => format!(
                    "namespace tree {}: main={} anchor={}",
                    row.ns.as_str(),
                    tree.main
                        .as_deref()
                        .map_or_else(|| "-".to_owned(), |main| main.display().to_string()),
                    tree.anchor.display()
                ),
                None => format!("namespace tree {}: not installed", row.ns.as_str()),
            })
            .collect()
    }
}

/// The loose-sound index of each installed namespace.
#[derive(Debug, Default)]
pub struct NamespaceSoundIwd {
    indices: [Option<IwdSoundIndex>; NAMESPACES.len()],
}

impl NamespaceSoundIwd {
    /// Index the sounds of every tree in `trees`. The second value is what to
    /// log about each one.
    pub fn open(trees: &NamespaceTrees) -> (Self, Vec<String>) {
        let mut opened = Self::default();
        let mut lines = Vec::new();
        for row in &NAMESPACES {
            let name = row.ns.as_str();
            let Some(tree) = trees.get(row.ns) else {
                continue;
            };
            let Some(main) = tree.main.as_deref() else {
                lines.push(format!(
                    "audio: IWD sounds {name}: tree {} has no archive directory",
                    tree.anchor.display()
                ));
                continue;
            };
            match IwdSoundIndex::open(main) {
                Ok(index) => {
                    lines.push(format!(
                        "audio: IWD sounds {name} from {} — {} paths",
                        index.indexed_dir().display(),
                        index.sound_count()
                    ));
                    opened.indices[slot(row.ns)] = Some(index);
                }
                Err(error) => lines.push(format!("audio: IWD sounds {name} unavailable ({error})")),
            }
        }
        (opened, lines)
    }

    /// The sound at `relative` (below `sound/`) from the namespace's archives.
    pub fn read_sound(
        &self,
        ns: AssetNamespace,
        relative: &str,
    ) -> Option<Result<Vec<u8>, String>> {
        self.indices[slot(ns)].as_ref()?.read_sound(relative)
    }

    pub fn is_empty(&self) -> bool {
        self.indices.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_zone(dir: &Path, name: &str, version: u32) {
        std::fs::create_dir_all(dir).unwrap();
        let mut head = b"IWff0100".to_vec();
        head.extend_from_slice(&version.to_le_bytes());
        std::fs::write(dir.join(name), head).unwrap();
    }

    #[test]
    fn a_tree_is_found_by_its_anchor_zone_and_reported() {
        let dir = std::env::temp_dir().join(format!("bo2mp-trees-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/main")).unwrap();
        put_zone(&dir.join("game/zone/all"), "common_zm.ff", fastfile_t6::ZONE_VERSION_PC);

        let trees = NamespaceTrees::discover(&GamesRoot(dir.clone()));
        assert!(!trees.is_empty());
        assert!(trees.get(AssetNamespace::T5).is_none());
        let tree = trees.get(AssetNamespace::T6).unwrap();
        assert_eq!(tree.main.as_deref(), Some(dir.join("game/main").as_path()));
        let lines = trees.report_lines();
        assert!(lines[0].ends_with("t5: not installed"), "{lines:?}");
        assert!(lines[1].starts_with("namespace tree t6: main="), "{lines:?}");

        let (sounds, log) = NamespaceSoundIwd::open(&trees);
        assert!(!sounds.is_empty());
        assert!(log[0].contains("0 paths"), "{log:?}");
        assert!(sounds.read_sound(AssetNamespace::T6, "a.wav").is_none());
        assert!(sounds.read_sound(AssetNamespace::T5, "a.wav").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
