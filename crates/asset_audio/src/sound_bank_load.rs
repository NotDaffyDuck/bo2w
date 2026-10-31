//! Putting a match's sound catalog together from its zones, and keeping
//! account of the zones that gave nothing.

use std::path::Path;

use asset_core::AssetCategoryScope;
use asset_transport::GameInstallDir;
use asset_transport::locate_runtime_zone;
use asset_transport::title_of_zone_path;

use crate::sound_bank::AudioRoster;

/// The zones every match reads sound from, in order. A zone that is not
/// required may be absent without a word to the player.
const RUNTIME_ZONES: [(&str, bool); 5] = [
    ("code_post_gfx_mp", false),
    ("localized_code_post_gfx_mp", false),
    ("patch_mp", false),
    ("common_mp", true),
    ("localized_common_mp", true),
];

/// Every Black Ops II sound belongs to the one namespace.
pub fn zone_sound_namespace(_games: &GameInstallDir, _zone: &str) -> AssetCategoryScope {
    AssetCategoryScope::T6
}

/// A zone that gave no sound, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundZoneMiss {
    pub namespace: AssetCategoryScope,
    pub zone: String,
    pub why: String,
}

impl SoundZoneMiss {
    fn line(&self) -> String {
        let Self { namespace, zone, why } = self;
        let space = namespace.as_str();
        format!("sound bank gap: {space} `{zone}` — {why}")
    }
}

/// A finished catalog and the zones it is missing.
#[derive(Default)]
pub struct InMemoryAudioBank {
    pub catalog: AudioRoster,
    pub gaps: Vec<SoundZoneMiss>,
}

impl InMemoryAudioBank {
    pub fn hole_rows(&self) -> Vec<String> {
        Vec::from_iter(self.gaps.iter().map(|gap| gap.line()))
    }
}

/// What the runtime zones gave before the map's own zone is added.
pub struct SoundSources {
    gaps: Vec<SoundZoneMiss>,
}

/// Why a zone file's sound could not be read. Black Ops II sound banks are
/// not read from zone files yet, so a readable zone still gives nothing.
fn unread_reason(zone_path: &Path) -> String {
    match title_of_zone_path(zone_path) {
        Some(_) => format!(
            "{}: Black Ops II sound banks are not read yet (bo2zm)",
            zone_path.display()
        ),
        None => format!("{} is not a readable IWff envelope", zone_path.display()),
    }
}

fn gap_of(zone: &str, why: String) -> SoundZoneMiss {
    SoundZoneMiss { namespace: AssetCategoryScope::T6, zone: String::from(zone), why }
}

/// Looks for each runtime zone beside the map's zone file `map`.
pub fn gather_sound_sources(games: &GameInstallDir, map: &Path) -> SoundSources {
    let mut gaps: Vec<SoundZoneMiss> = vec![];
    for (name, required) in RUNTIME_ZONES {
        let found = locate_runtime_zone(games, map, name);
        match (found, required) {
            (Ok(zone), _) => gaps.push(gap_of(name, unread_reason(&zone.path))),
            (Err(error), true) => gaps.push(gap_of(name, error)),
            (Err(error), false) => diag::info!(Zone, "sound bank: T6 {name} missing ({error})"),
        }
    }
    SoundSources { gaps }
}

/// The runtime zones' sound with the map zone's laid over what they lack.
/// `map` is what reading the map's zone gave.
pub fn compose_sound_bank(
    sources: SoundSources,
    zone: &str,
    namespace: AssetCategoryScope,
    map: Result<AudioRoster, String>,
) -> InMemoryAudioBank {
    let mut bank = InMemoryAudioBank { catalog: Default::default(), gaps: sources.gaps };
    match map {
        Ok(from_map) => {
            let ns_name = namespace.as_str();
            let (raw_count, alias_count) = (from_map.rawfiles.len(), from_map.sounds.len());
            diag::info!(Zone, "sound bank: {zone} gave {raw_count} raw files and {alias_count} aliases ({ns_name})");
            bank.catalog.merge_missing(from_map);
        }
        Err(why) => bank.gaps.push(SoundZoneMiss { namespace, zone: zone.into(), why }),
    }
    bank.catalog.publish();
    bank.gaps.iter().for_each(|entry| diag::warn!(Zone, "{}", entry.line()));
    bank
}

/// Loads the sound of a multiplayer map's zones. Nothing is read from the
/// zone files yet, so the catalog comes back empty with every zone listed
/// as a gap.
pub fn read_mp_sound_bank(games: &GameInstallDir, zone: &str) -> Result<InMemoryAudioBank, String> {
    let found = asset_transport::locate_zone_file(games, zone)?;
    let sources = gather_sound_sources(games, &found.path);
    let map = Err(unread_reason(&found.path));
    Ok(compose_sound_bank(sources, zone, AssetCategoryScope::T6, map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_map_zone_is_a_gap_and_the_catalog_is_published() {
        let sources = SoundSources { gaps: Vec::new() };
        let bank = compose_sound_bank(
            sources,
            "mp_x",
            AssetCategoryScope::T6,
            Err("no such zone".to_owned()),
        );
        assert!(bank.catalog.sounds.is_empty());
        assert!(bank.catalog.generation() > 0);
        let namespace = AssetCategoryScope::T6.as_str();
        assert_eq!(
            bank.hole_rows(),
            [format!("sound bank gap: {namespace} `mp_x` — no such zone")]
        );
    }

    #[test]
    fn the_map_zone_fills_in_the_catalog() {
        let mut map = AudioRoster::default();
        map.add_sound(crate::SnapshotSoundEntry { name: "wind".to_owned(), aliases: vec![] });
        let bank = compose_sound_bank(
            SoundSources { gaps: Vec::new() },
            "mp_x",
            AssetCategoryScope::T6,
            Ok(map),
        );
        assert!(bank.gaps.is_empty());
        assert!(bank.catalog.entry_named("wind").is_some());
    }
}
