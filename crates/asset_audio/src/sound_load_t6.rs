//! bo2zm: Black Ops II sound.
//!
//! A T6 zone carries sound *banks* (`SndBank`): alias names with their
//! variants (volume, pitch, distances, falloff curves, the file each one
//! plays, by the hash of its name). The audio itself lives beside the
//! zones in `sound/*.sabl` (loaded, 16-bit PCM) and `*.sabs` (streamed,
//! FLAC) files, read by `sab_t6`. Aliases become `CapturedSound`s whose
//! variants name their clip as streamed `(bank file, entry id)`; the clip
//! store decodes a clip on first use through `decode_t6_clip`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{
    CapturedAlias, CapturedSndCurve, CapturedSound, SndDuck, SoundCatalog, StereoPan, VoicePriority,
};

/// Every bank file a set of bank names reads, indexed by entry id.
#[derive(Default)]
pub struct T6BankIndex {
    by_id: HashMap<u32, (Arc<sab_t6::BankFile>, sab_t6::BankEntry)>,
    pub files: Vec<PathBuf>,
}

impl T6BankIndex {
    /// Open every `<bank>.<language>.sab*` in `sound_dir` whose name starts
    /// with one of `banks` (and the languages `all` / `english`). Earlier
    /// banks win an id they share with a later one.
    pub fn open(sound_dir: &Path, banks: &[&str]) -> (Self, Vec<String>) {
        let mut index = Self::default();
        let mut report = Vec::new();
        let Ok(read) = std::fs::read_dir(sound_dir) else {
            report.push(format!("t6 sound: no sound folder at {}", sound_dir.display()));
            return (index, report);
        };
        let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for bank in banks {
            for path in &paths {
                let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let lower = file.to_ascii_lowercase();
                let wanted = [".all.sabl", ".all.sabs", ".english.sabl", ".english.sabs"]
                    .iter()
                    .any(|tail| lower == format!("{bank}{tail}"));
                if !wanted {
                    continue;
                }
                match sab_t6::BankFile::open(path) {
                    Ok(file) => {
                        let file = Arc::new(file);
                        for e in &file.entries {
                            index
                                .by_id
                                .entry(e.id)
                                .or_insert_with(|| (Arc::clone(&file), *e));
                        }
                        index.files.push(path.clone());
                    }
                    Err(e) => report.push(format!("t6 sound bank gap: {e}")),
                }
            }
        }
        report.push(format!(
            "t6 sound banks: {} files, {} clips",
            index.files.len(),
            index.by_id.len()
        ));
        (index, report)
    }

    pub fn get(&self, id: u32) -> Option<&(Arc<sab_t6::BankFile>, sab_t6::BankEntry)> {
        self.by_id.get(&id)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
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
fn curve(globals: Option<&asset_t6::SndDriverGlobalsRef>, index: u32) -> Option<CapturedSndCurve> {
    let c = globals?.curves.get(index as usize)?;
    // Curves end at the first point whose x falls back below the last.
    let mut knots: Vec<(f32, f32)> = Vec::with_capacity(8);
    for p in c.points {
        if knots.last().is_some_and(|&(x, _)| p[0] < x) {
            break;
        }
        knots.push((p[0], p[1]));
    }
    Some(CapturedSndCurve {
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
    Some(StereoPan::from_pan(p.front, p.back, p.left, p.right))
}

fn alias_variant(
    a: &asset_t6::SndAliasRef,
    index: &T6BankIndex,
    globals: Option<&asset_t6::SndDriverGlobalsRef>,
) -> CapturedAlias {
    // BO2 scales every variant by its volume group and the group's parents.
    let group_scalar = a.group_scalar(globals);
    let (vol_min, vol_max) = a.volume();
    let (vol_min, vol_max) = (vol_min * group_scalar, vol_max * group_scalar);
    let (pitch_min, pitch_max) = a.pitch();
    let streamed = index
        .get(a.asset_id)
        .map(|(file, _)| (file.path.to_string_lossy().into_owned(), format!("{:08x}", a.asset_id)));
    CapturedAlias {
        secondary: (!a.secondary.is_empty()).then(|| a.secondary.clone()),
        file_name: (!a.asset_file.is_empty()).then(|| a.asset_file.clone()),
        streamed,
        vol_min,
        vol_max,
        pitch_min: if pitch_min > 0.0 { pitch_min } else { 1.0 },
        pitch_max: if pitch_max > 0.0 { pitch_max } else { 1.0 },
        dist_min: f32::from(a.dist_min),
        dist_max: f32::from(a.dist_max),
        flags: Some(a.flags[0]),
        probability: f32::from(a.probability) / 255.0,
        volume_falloff: curve(globals, a.volume_falloff_curve()),
        near_falloff: curve(globals, a.volume_min_falloff_curve()),
        voice_priority: Some(VoicePriority {
            thresholds: [a.min_priority_threshold, a.max_priority_threshold],
            values: [a.min_priority, a.max_priority],
            distance_max: f32::from(a.dist_max),
        }),
        limit_count: Some(a.limit_count),
        entity_limit_count: Some(a.entity_limit_count),
        menu_category: Some(a.menu_category(globals) as u8),
        stereo_pan: stereo_pan(a, globals),
        duck: (a.duck != 0).then_some(a.duck),
        duck_group: Some(a.duck_group),
        ..CapturedAlias::default()
    }
}

/// What a T6 catalog build found.
#[derive(Clone, Debug, Default)]
pub struct T6SoundCensus {
    pub aliases: usize,
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
) -> (SoundCatalog, T6SoundCensus) {
    let mut catalog = SoundCatalog::default();
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
                    fade_in: d.fade_in,
                    fade_out: d.fade_out,
                    start_delay: d.start_delay,
                    distance: d.distance,
                    length: d.length,
                    update_while_paused: d.update_while_paused,
                    attenuation: per_group(&d.attenuation),
                    filter: per_group(&d.filter),
                },
            );
        }
        for list in &bank.aliases {
            if list.name.is_empty() {
                continue;
            }
            let aliases: Vec<CapturedAlias> = list
                .entries
                .iter()
                .map(|a| alias_variant(a, index, globals))
                .collect();
            census.aliases += 1;
            census.variants += aliases.len();
            let with = aliases.iter().filter(|a| a.streamed.is_some()).count();
            census.with_clip += with;
            census.without_clip += aliases.len() - with;
            catalog.add_sound(CapturedSound {
                name: list.name.clone(),
                aliases,
            });
        }
    }
    catalog.publish();
    (catalog, census)
}
