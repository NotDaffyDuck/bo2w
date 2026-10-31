//! bo2zm: Black Ops II sound.
//!
//! A T6 zone carries sound *banks* (`SndBank`): alias names with their
//! variants (volume, pitch, distances, falloff curves, the file each one
//! plays, by the hash of its name). The audio itself lives beside the
//! zones in `sound/*.sabl` (loaded, 16-bit PCM) and `*.sabs` (streamed,
//! FLAC) files, read by `sab_t6`. Aliases become `SnapshotSoundEntry`s whose
//! variants name their clip as streamed `(bank file, entry id)`; the clip
//! store decodes a clip on first use through `decode_t6_clip`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{
    SnapshotSoundAlias, SnapshotAudioCurve, SnapshotSoundEntry, SndDuck, AudioRoster, StereoPan, VoicePriority,
};

/// Every bank file a set of bank names reads, indexed by entry id.
#[derive(Default)]
pub struct T6BankIndex {
    clips_by_id: HashMap<u32, (Arc<sab_t6::BankFile>, sab_t6::BankEntry)>,
    pub files: Vec<PathBuf>,
}

/// Whether `path` names one of `bank`'s loaded / streamed files in a language we play.
fn is_bank_file(path: &Path, bank: &str) -> bool {
    const TAILS: [&str; 4] = [".all.sabl", ".all.sabs", ".english.sabl", ".english.sabs"];
    let lower = match path.file_name().and_then(|n| n.to_str()) {
        Some(file) => file.to_ascii_lowercase(),
        None => return false,
    };
    lower
        .strip_prefix(bank)
        .is_some_and(|tail| TAILS.contains(&tail))
}

/// The banks found in a sound folder, with one line of news per folder gap, bad bank and the
/// final file and clip count.
pub struct OpenedBanks {
    pub index: T6BankIndex,
    pub notes: Vec<String>,
}

impl T6BankIndex {
    /// Read every `<bank>.<language>.sab*` in `folder` whose name starts
    /// with one of `wanted` (and the languages `all` / `english`). Earlier
    /// banks win an id they share with a later one.
    pub fn load(folder: &Path, wanted: &[&str]) -> OpenedBanks {
        let mut index = Self::default();
        let mut notes: Vec<String> = vec![];
        let Ok(read) = std::fs::read_dir(folder) else {
            let shown = folder.display();
            notes.push(format!("t6 sound: {shown} has no sound folder"));
            return OpenedBanks { index, notes };
        };
        let sorted: BTreeSet<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
        // bo2zm: then the extra folders' banks (`BO2MP_T6_EXTRA`), after the game's own.
        let paths: Vec<PathBuf> = sorted
            .into_iter()
            .chain(asset_transport::t6_extra::extra_files("sabl"))
            .chain(asset_transport::t6_extra::extra_files("sabs"))
            .collect();
        for bank in wanted {
            for path in paths.iter().filter(|p| is_bank_file(p, bank)) {
                match sab_t6::BankFile::open(path) {
                    Ok(file) => {
                        let file = Arc::new(file);
                        for e in &file.entries {
                            index
                                .clips_by_id
                                .entry(e.id)
                                .or_insert_with(|| (Arc::clone(&file), *e));
                        }
                        index.files.push(path.clone());
                    }
                    Err(e) => notes.push(format!("t6 sound bank gap: {e}")),
                }
            }
        }
        let (file_total, clip_total) = (index.files.len(), index.clips_by_id.len());
        notes.push(format!("t6 sound banks: {file_total} files hold {clip_total} clips"));
        OpenedBanks { index, notes }
    }

    /// Older spelling the assets lanes still call; the rename sweep moves them to `load`.
    pub fn open(folder: &Path, wanted: &[&str]) -> (Self, Vec<String>) {
        let OpenedBanks { index, notes } = Self::load(folder, wanted);
        (index, notes)
    }

    pub fn clip_by_id(&self, wanted: u32) -> Option<&(Arc<sab_t6::BankFile>, sab_t6::BankEntry)> {
        self.clips_by_id.get(&wanted)
    }

    pub fn len(&self) -> usize {
        self.clips_by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips_by_id.is_empty()
    }
}

/// Opened bank files by path, shared by every clip decode.
fn open_banks() -> &'static Mutex<HashMap<String, Arc<sab_t6::BankFile>>> {
    static BANKS: OnceLock<Mutex<HashMap<String, Arc<sab_t6::BankFile>>>> = OnceLock::new();
    BANKS.get_or_init(Default::default)
}

/// Decode one clip: `bank` is the bank file's path, `id` the entry id as
/// eight hex digits (what a T6 alias variant names as its streamed clip).
pub fn decode_t6_clip(bank: &str, id: &str) -> Result<sab_t6::Pcm16, String> {
    let id = u32::from_str_radix(id, 16).map_err(|e| format!("clip id `{id}`: {e}"))?;
    let file = {
        let mut banks = open_banks().lock().map_err(|_| "bank cache poisoned".to_owned())?;
        match banks.get(bank) {
            Some(file) => Arc::clone(file),
            None => {
                let file = Arc::new(sab_t6::BankFile::open(Path::new(bank))?);
                banks.insert(bank.to_owned(), Arc::clone(&file));
                file
            }
        }
    };
    let entry = file
        .entries
        .iter()
        .find(|e| e.id == id)
        .copied()
        .ok_or_else(|| format!("{bank}: no clip {id:08x}"))?;
    file.decode(&entry)
}

/// The curve name a falloff curve index stands for in the driver globals.
fn curve(globals: Option<&asset_t6::SndDriverGlobalsRef>, index: u32) -> Option<SnapshotAudioCurve> {
    let c = globals?.curves.get(index as usize)?;
    // Curves end at the first point whose x falls back below the last.
    let mut knots: Vec<(f32, f32)> = Vec::with_capacity(8);
    for p in c.points {
        if knots.last().is_some_and(|&(x, _)| p[0] < x) {
            break;
        }
        knots.push((p[0], p[1]));
    }
    Some(SnapshotAudioCurve {
        name: format!("t6/curve/{}", c.name),
        knots,
    })
}

/// The fixed stereo gains BO2 gives a variant that does not pan by direction: a 2D one
/// (flags bit 1 clear), or a 3D one whose pan is not the `default` pan.
fn stereo_pan(
    a: &asset_t6::SndAliasRef,
    globals: Option<&asset_t6::SndDriverGlobalsRef>,
) -> Option<StereoPan> {
    let pans = &globals?.pans;
    let default = pans.iter().position(|p| p.name == "default").unwrap_or(0);
    let spatialized = a.flags[0] & 2 != 0;
    if spatialized && usize::from(a.pan) == default {
        return None;
    }
    let p = pans.get(usize::from(a.pan))?;
    Some(StereoPan::from_weights([p.front, p.back, p.left, p.right]))
}

fn alias_variant(
    a: &asset_t6::SndAliasRef,
    index: &T6BankIndex,
    globals: Option<&asset_t6::SndDriverGlobalsRef>,
) -> SnapshotSoundAlias {
    // BO2 scales every variant by its volume group and the group's parents.
    let group_scalar = a.group_scalar(globals);
    let (vol_min, vol_max) = a.volume();
    let (vol_min, vol_max) = (vol_min * group_scalar, vol_max * group_scalar);
    let (pitch_lower_bound, pitch_upper_bound) = a.pitch();
    let streamed = index
        .clip_by_id(a.asset_id)
        .map(|(file, _)| (file.path.to_string_lossy().into_owned(), format!("{:08x}", a.asset_id)));
    SnapshotSoundAlias {
        layered_alias: (!a.secondary.is_empty()).then(|| a.secondary.clone()),
        clip_file: (!a.asset_file.is_empty()).then(|| a.asset_file.clone()),
        streamed,
        volume_low: vol_min,
        volume_high: vol_max,
        pitch_lower_bound: if pitch_lower_bound > 0.0 { pitch_lower_bound } else { 1.0 },
        pitch_upper_bound: if pitch_upper_bound > 0.0 { pitch_upper_bound } else { 1.0 },
        reach_lower: f32::from(a.reach_lower),
        reach_upper: f32::from(a.reach_upper),
        alias_bits: Some(a.flags[0]),
        chance_weight: f32::from(a.chance_weight) / 255.0,
        volume_falloff: curve(globals, a.volume_falloff_curve()),
        near_falloff: curve(globals, a.volume_min_falloff_curve()),
        voice_priority: Some(VoicePriority {
            thresholds: [a.min_priority_threshold, a.max_priority_threshold],
            values: [a.min_priority, a.max_priority],
            distance_max: f32::from(a.reach_upper),
        }),
        reverb_send: f32::from(a.reverb_send) / 65535.0,
        cap_total: Some(a.cap_total),
        actor_cap_total: Some(a.actor_cap_total),
        menu_category: Some(a.menu_category(globals) as u8),
        stereo_pan: stereo_pan(a, globals),
        duck: (a.duck != 0).then_some(a.duck),
        duck_group: Some(a.duck_group),
        ..SnapshotSoundAlias::default()
    }
}

/// What a T6 catalog build found.
#[derive(Clone, Debug, Default)]
pub struct T6SoundCensus {
    pub alias_count: usize,
    pub variants: usize,
    pub with_clip: usize,
    pub without_clip: usize,
}

/// Build the engine's sound catalog from T6 banks (later banks override an
/// alias an earlier one defines). `index` says where each clip lives.
pub fn build_t6_sound_catalog(
    banks: &[&asset_t6::SndBankRef],
    globals: Option<&asset_t6::SndDriverGlobalsRef>,
    index: &T6BankIndex,
) -> (AudioRoster, T6SoundCensus) {
    let mut catalog = AudioRoster::default();
    let mut census = T6SoundCensus::default();
    if let Some(g) = globals {
        catalog.duck_groups = g.duck_groups.iter().map(|(name, _)| name.clone()).collect();
        let band = |b: &asset_t6::SndEqBandRef| crate::SndEqBand {
            enabled: b.enabled,
            gain_db: b.gain,
            freq: b.freq,
            q: b.q,
        };
        catalog.masters = g
            .masters
            .iter()
            .map(|m| crate::SndMaster {
                name: m.name.clone(),
                low: band(&m.low),
                peak1: band(&m.peak1),
                peak2: band(&m.peak2),
                hi: band(&m.hi),
                eq_gain: m.eq_gain,
            })
            .collect();
    }
    for bank in banks {
        // A later bank's duck wins (SND_GetDuckById searches the last loaded first).
        for d in &bank.ducks {
            let per_group = |v: &[f32]| {
                let mut out = [1.0f32; 32];
                for (o, x) in out.iter_mut().zip(v) {
                    *o = *x;
                }
                out
            };
            catalog.ducks.insert(
                d.id,
                SndDuck {
                    name: d.name.clone(),
                    id: d.id,
                    ease_on: d.ease_on,
                    ease_off: d.ease_off,
                    start_delay: d.start_delay,
                    fade_range: d.distance,
                    length: d.length,
                    update_while_paused: d.update_while_paused,
                    attenuation: per_group(&d.attenuation),
                    filter: per_group(&d.filter),
                },
            );
        }
        for r in &bank.radverbs {
            catalog.radverbs.insert(r.name.clone(), r.values);
        }
        for list in &bank.aliases {
            if list.name.is_empty() {
                continue;
            }
            let aliases: Vec<SnapshotSoundAlias> = list
                .entries
                .iter()
                .map(|a| alias_variant(a, index, globals))
                .collect();
            census.alias_count += 1;
            census.variants += aliases.len();
            let with = aliases.iter().filter(|a| a.streamed.is_some()).count();
            census.with_clip += with;
            census.without_clip += aliases.len() - with;
            catalog.add_sound(SnapshotSoundEntry {
                name: list.name.clone(),
                aliases,
            });
        }
    }
    catalog.publish();
    (catalog, census)
}

#[cfg(test)]
mod bank_file_tests {
    use super::*;

    #[test]
    fn bank_files_match_exact_bank_and_language() {
        let yes = |f: &str| is_bank_file(Path::new(f), "zm_transit");
        assert!(yes("zm_transit.all.sabl"));
        assert!(yes("ZM_Transit.English.SABS".to_ascii_lowercase().as_str()));
        assert!(yes("zm_transit.english.sabl"));
        assert!(!yes("zm_transit.german.sabl"));
        assert!(!yes("zm_transit_patch.all.sabl"));
        assert!(!yes("zm_transit.all.sab"));
        assert!(!is_bank_file(Path::new("ZM_Transit.all.sabl"), "ZM_Transit"));
    }
}
