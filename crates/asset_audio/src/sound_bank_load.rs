//! Putting a match's sound catalog together from its zones, and keeping
//! account of the zones that gave nothing.

use std::path::Path;

use asset_transport::{GamesRoot, find_runtime_zone, find_zone_file, zone_game_for_path};

use crate::{AssetNamespace, SoundCatalog};

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
pub fn namespace_for_zone(_games: &GamesRoot, _zone: &str) -> AssetNamespace {
    AssetNamespace::T6
}

/// A zone that gave no sound, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundZoneGap {
    pub namespace: AssetNamespace,
    pub zone: String,
    pub reason: String,
}

impl SoundZoneGap {
    fn line(&self) -> String {
        format!(
            "sound bank gap: {} `{}` — {}",
            self.namespace.as_str(),
            self.zone,
            self.reason
        )
    }
}

/// A finished catalog and the zones it is missing.
#[derive(Default)]
pub struct LoadedSoundBank {
    pub catalog: SoundCatalog,
    pub gaps: Vec<SoundZoneGap>,
}

impl LoadedSoundBank {
    pub fn gap_lines(&self) -> Vec<String> {
        self.gaps.iter().map(SoundZoneGap::line).collect()
    }
}

/// What the runtime zones gave before the map's own zone is added.
pub struct SoundSources {
    gaps: Vec<SoundZoneGap>,
}

/// Why a zone file's sound could not be read. Black Ops II sound banks are
/// not read from zone files yet, so a readable zone still gives nothing.
fn unread_reason(zone_path: &Path) -> String {
    match zone_game_for_path(zone_path) {
        Some(_) => format!(
            "{}: Black Ops II sound banks are not read yet (bo2zm)",
            zone_path.display()
        ),
        None => format!("{} is not a readable IWff envelope", zone_path.display()),
    }
}

fn gap_of(zone: &str, reason: String) -> SoundZoneGap {
    SoundZoneGap {
        namespace: AssetNamespace::T6,
        zone: zone.to_owned(),
        reason,
    }
}

/// Looks for each runtime zone beside the map's zone file `map`.
pub fn gather_sound_sources(games: &GamesRoot, map: &Path) -> SoundSources {
    let mut gaps = Vec::new();
    for (name, required) in RUNTIME_ZONES {
        match find_runtime_zone(games, map, name) {
            Ok(zone) => gaps.push(gap_of(name, unread_reason(&zone.path))),
            Err(error) if required => gaps.push(gap_of(name, error)),
            Err(error) => diag::info!(Zone, "sound bank: T6 {name} missing ({error})"),
        }
    }
    SoundSources { gaps }
}

/// The runtime zones' sound with the map zone's laid over what they lack.
/// `map` is what reading the map's zone gave.
pub fn compose_sound_bank(
    sources: SoundSources,
    zone: &str,
    namespace: AssetNamespace,
    map: Result<SoundCatalog, String>,
) -> LoadedSoundBank {
    let mut bank = LoadedSoundBank {
        catalog: SoundCatalog::default(),
        gaps: sources.gaps,
    };
    match map {
        Ok(map) => {
            diag::info!(
                Zone,
                "sound bank: map zone {zone} ({}) — {} rawfiles, {} aliases",
                namespace.as_str(),
                map.rawfiles.len(),
                map.sounds.len()
            );
            bank.catalog.absorb_missing_aliases(map);
        }
        Err(error) => bank.gaps.push(SoundZoneGap {
            namespace,
            zone: zone.to_owned(),
            reason: error,
        }),
    }
    bank.catalog.publish();
    for gap in &bank.gaps {
        diag::warn!(Zone, "{}", gap.line());
    }
    bank
}

/// Loads the sound of a multiplayer map's zones. Nothing is read from the
/// zone files yet, so the catalog comes back empty with every zone listed
/// as a gap.
pub fn load_mp_sound_bank(games: &GamesRoot, zone: &str) -> Result<LoadedSoundBank, String> {
    let found = find_zone_file(games, zone)?;
    let sources = gather_sound_sources(games, &found.path);
    let map = Err(unread_reason(&found.path));
    Ok(compose_sound_bank(sources, zone, AssetNamespace::T6, map))
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
            AssetNamespace::T6,
            Err("no such zone".to_owned()),
        );
        assert!(bank.catalog.sounds.is_empty());
        assert!(bank.catalog.revision() > 0);
        let namespace = AssetNamespace::T6.as_str();
        assert_eq!(
            bank.gap_lines(),
            [format!("sound bank gap: {namespace} `mp_x` — no such zone")]
        );
    }

    #[test]
    fn the_map_zone_fills_in_the_catalog() {
        let mut map = SoundCatalog::default();
        map.add_sound(crate::CapturedSound {
            name: "wind".to_owned(),
            aliases: Vec::new(),
        });
        let bank = compose_sound_bank(
            SoundSources { gaps: Vec::new() },
            "mp_x",
            AssetNamespace::T6,
            Ok(map),
        );
        assert!(bank.gaps.is_empty());
        assert!(bank.catalog.sound("wind").is_some());
    }
}
