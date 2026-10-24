//! Finding the player's Source game files.
//!
//! In order: loose files already copied into the install (`guest/source`,
//! how the family PCs without Steam get them), `BO2MP_SOURCE_DIR`, every
//! Steam library's Half-Life 2 / Garry's Mod / Half-Life 2: Deathmatch, and
//! the free Garry's Mod Dedicated Server fetched into `guest/gmodds`.

use std::path::{Path, PathBuf};

use crate::vpk::{self, Vpk};

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
    pub vpks: Vec<Vpk>,
    /// What was searched and what was found, for the log.
    pub report: Vec<String>,
}

impl Source {
    /// A file's bytes, from the first place that has it.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let key = vpk::key(path);
        for dir in &self.loose {
            if let Ok(bytes) = std::fs::read(dir.join(&key)) {
                return Some(bytes);
            }
        }
        self.vpks.iter().find_map(|v| v.read(&key))
    }

    /// Whether every one of `paths` can be read.
    pub fn has_all(&self, paths: &[&str]) -> bool {
        paths.iter().all(|p| {
            let key = vpk::key(p);
            self.loose.iter().any(|d| d.join(&key).is_file()) || self.vpks.iter().any(|v| v.contains(&key))
        })
    }

    /// Copies `paths` as loose files under `dest`, so an install carries
    /// them to a PC without the game. Returns how many were written.
    pub fn copy_out(&self, paths: &[&str], dest: &Path) -> std::io::Result<usize> {
        let mut written = 0;
        for p in paths {
            let Some(bytes) = self.read(p) else { continue };
            let to = dest.join(vpk::key(p));
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&to, bytes)?;
            written += 1;
        }
        Ok(written)
    }

    /// Searches everywhere, `install` being the folder the game runs from.
    pub fn find(install: &Path) -> Self {
        let mut source = Self::default();
        let guest = install.join("guest");
        let mut loose = vec![guest.join("source")];
        if let Some(dir) = std::env::var_os("BO2MP_SOURCE_DIR") {
            loose.push(PathBuf::from(dir));
        }
        for dir in loose {
            if dir.is_dir() {
                source.report.push(format!("loose {}", dir.display()));
                source.loose.push(dir);
            }
        }
        let mut games: Vec<PathBuf> = steam_libraries()
            .iter()
            .flat_map(|lib| GAME_DIRS.iter().map(move |g| lib.join("steamapps/common").join(g)))
            .collect();
        games.push(guest.join("gmodds/sourceengine"));
        for game in games {
            for name in HL2_VPKS {
                let path = game.join(name);
                if !path.is_file() {
                    continue;
                }
                match Vpk::open(&path) {
                    Some(v) => {
                        source.report.push(format!("vpk {}", path.display()));
                        source.vpks.push(v);
                    }
                    None => source.report.push(format!("unreadable {}", path.display())),
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
        let canon = p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if p.is_dir() && !libs.iter().any(|l| l.to_string_lossy().replace('\\', "/").to_ascii_lowercase() == canon) {
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
        for line in &source.report {
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
            model.meshes.iter().map(|m| (m.texture, m.vertices.len(), m.indices.len())).collect::<Vec<_>>(),
            model.textures.iter().map(|t| &t.name).collect::<Vec<_>>(),
        );
        assert_eq!(model.bones.len(), 14);
        assert!(model.sequence("fire").is_some() && model.sequence("draw").is_some());
        assert!(model.meshes.iter().all(|m| !m.indices.is_empty()));
        let sheet = source.read("materials/models/weapons/v_physcannon/v_physcannon_sheet.vtf").unwrap();
        let rgba = crate::vtf::decode(&sheet).expect("texture decodes");
        assert!(rgba.width >= 256 && rgba.pixels.len() == (rgba.width * rgba.height * 4) as usize);
    }
}
