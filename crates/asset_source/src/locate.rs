//! Finding the player's Source game files.
//!
//! In order: loose files already copied into the install (`guest/source`,
//! how the family PCs without Steam get them), `BO2MP_SOURCE_DIR`, every
//! Steam library's Half-Life 2 / Garry's Mod / Half-Life 2: Deathmatch, and
//! the free Garry's Mod Dedicated Server fetched into `guest/gmodds`.

use std::path::{Path, PathBuf};

use crate::vpk::{PackFile, folded, pack_path};

/// The VPKs that hold Half-Life 2's models, textures and sounds.
const HL2_VPKS: &[&str] = &["hl2_misc_dir.vpk", "hl2_textures_dir.vpk", "hl2_sound_misc_dir.vpk"];

/// Game folders under a library's `steamapps/common` that carry them.
const GAME_DIRS: &[&str] = &[
    "Half-Life 2/hl2",
    "GarrysMod/sourceengine",
    "Half-Life 2 Deathmatch/hl2",
];

/// Where Source files were found: loose folders first, then VPKs.
#[derive(Debug, Default)]
pub struct Source {
    pub loose: Vec<PathBuf>,
    pub vpks: Vec<PackFile>,
    /// What was searched and what was found, for the log.
    pub notes: Vec<String>,
}

/// Writes `bytes` to `to`, making its folder first.
fn write_with_parents(to: &Path, bytes: &[u8]) -> std::io::Result<()> {
    to.parent().map_or(Ok(()), std::fs::create_dir_all)?;
    std::fs::write(to, bytes)
}

impl Source {
    /// A file's bytes, from the first place that has it.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = pack_path(path);
        self.loose
            .iter()
            .find_map(|dir| std::fs::read(dir.join(&key)).ok())
            .or_else(|| self.vpks.iter().find_map(|v| v.read(&key)))
    }

    /// Whether every one of `paths` can be read.
    pub fn has_all(&self, paths: &[&str]) -> bool {
        paths.iter().all(|p| {
            let key = pack_path(p);
            self.loose.iter().any(|d| d.join(&key).is_file()) || self.vpks.iter().any(|v| v.contains(&key))
        })
    }

    /// Copies `paths` as loose files under `write_target`, so an install carries
    /// them to a PC without the game. Returns how many were written.
    pub fn copy_out(&self, paths: &[&str], write_target: &Path) -> std::io::Result<usize> {
        paths
            .iter()
            .filter_map(|p| Some((write_target.join(pack_path(p)), self.read(p)?)))
            .try_fold(0usize, |written, (to, bytes)| {
                write_with_parents(&to, &bytes).map(|()| written + 1)
            })
    }

    /// Searches everywhere, `install` being the folder the game runs from.
    pub fn find(install: &Path) -> Self {
        let mut source = Self::default();
        let mut notes = Vec::new();
        let guest = install.join("guest");
        let mut loose = vec![guest.join("source")];
        if let Some(dir) = std::env::var_os("BO2MP_SOURCE_DIR") {
            loose.push(PathBuf::from(dir));
        }
        for dir in loose {
            if dir.is_dir() {
                let shown = dir.display();
                notes.push(format!("loose {shown}"));
                source.loose.push(dir);
            }
        }
        let mut games: Vec<PathBuf> = steam_libraries()
            .iter()
            .flat_map(|lib| GAME_DIRS.iter().map(move |g| lib.join("steamapps/common").join(g)))
            .collect();
        games.push(guest.join("gmodds/sourceengine"));
        for game in games {
            for path in HL2_VPKS.iter().map(|name| game.join(name)).filter(|p| p.is_file()) {
                let shown = path.display();
                match PackFile::open(&path) {
                    Some(v) => {
                        notes.push(format!("vpk {shown}"));
                        source.vpks.push(v);
                    }
                    None => notes.push(format!("unreadable {shown}")),
                }
            }
        }
        source
    }
}

/// Steam's own folder: the registry's `SteamPath`, else the usual places.
fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
            .creation_flags(NO_WINDOW)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = text.lines().find(|l| l.contains("SteamPath")) {
                if let Some(at) = line.find("REG_SZ") {
                    roots.push(PathBuf::from(line[at + 6..].trim()));
                }
            }
        }
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(var) {
                roots.push(PathBuf::from(dir).join("Steam"));
            }
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".local/share/Steam"));
    }
    roots
}

/// Every Steam library: each root and the `"path"` lines of its
/// `libraryfolders.vdf`.
pub fn steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        let canon = folded(&p.to_string_lossy());
        if p.is_dir() && !libs.iter().any(|l| folded(&l.to_string_lossy()) == canon) {
            libs.push(p);
        }
    };
    for root in steam_roots() {
        add(root.clone());
        let Ok(vdf) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else { continue };
        for line in vdf.lines() {
            let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
            if parts.next() == Some("path") {
                if let Some(path) = parts.next() {
                    add(PathBuf::from(path.replace("\\\\", "\\")));
                }
            }
        }
    }
    libs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs where Half-Life 2 or Garry's Mod is installed; elsewhere it only
    /// checks the search does not fail.
    #[test]
    fn finds_and_reads_the_gravity_gun() {
        let source = Source::find(Path::new("."));
        for line in &source.notes {
            eprintln!("{line}");
        }
        if source.vpks.is_empty() && source.loose.is_empty() {
            return;
        }
        let missing: Vec<_> = crate::GRAVITY_GUN.iter().filter(|p| source.read(p).is_none()).collect();
        assert!(missing.is_empty(), "missing {missing:?}");
        let mdl = source.read("models/weapons/v_physcannon.mdl").unwrap();
        let vvd = source.read("models/weapons/v_physcannon.vvd").unwrap();
        let vtx = source.read("models/weapons/v_physcannon.dx90.vtx").unwrap();
        let model = crate::mdl::read(&mdl, &vvd, &vtx).expect("model reads");
        eprintln!(
            "bones {} sequences {:?} meshes {:?} textures {:?}",
            model.bones.len(),
            model.sequences.iter().map(|s| &s.name).collect::<Vec<_>>(),
            model.meshes.iter().map(|m| (m.texture, m.vertices.len(), m.tri_list.len())).collect::<Vec<_>>(),
            model.textures.iter().map(|t| &t.name).collect::<Vec<_>>(),
        );
        assert_eq!(model.bones.len(), 14);
        assert!(model.sequence("fire").is_some() && model.sequence("draw").is_some());
        assert!(model.meshes.iter().all(|m| !m.tri_list.is_empty()));
        let sheet = source.read("materials/models/weapons/v_physcannon/v_physcannon_sheet.vtf").unwrap();
        let rgba = crate::vtf::decode(&sheet).expect("texture decodes");
        assert!(rgba.width >= 256 && rgba.pixels.len() == (rgba.width * rgba.height * 4) as usize);
    }

    #[test]
    fn loose_read_and_copy_out_roundtrip() {
        let root = std::env::temp_dir().join(format!("bo2mp_locate_{}", std::process::id()));
        let (src, dst) = (root.join("src"), root.join("dst"));
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::write(src.join("a/one.bin"), b"one").unwrap();
        let source = Source { loose: vec![root.join("nothing"), src.clone()], ..Source::default() };
        assert_eq!(source.read("A\\One.bin").as_deref(), Some(&b"one"[..]));
        assert!(source.read("a/two.bin").is_none());
        assert_eq!(source.copy_out(&["a/one.bin", "a/two.bin"], &dst).unwrap(), 1);
        assert_eq!(std::fs::read(dst.join("a/one.bin")).unwrap(), b"one");
        std::fs::remove_dir_all(&root).ok();
    }
}
