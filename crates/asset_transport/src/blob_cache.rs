//! A content-addressed blob cache under the artifacts directory.
//!
//! Entries live at `cache/<kind>/<first two key chars>/<key>`. Writes go to a
//! temp file in the same directory and are renamed into place, so a reader
//! never sees half a blob. The first successful write in a process starts one
//! background sweep that keeps the cache under a size budget.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::install_scan::prepare_output_folder;

const BUDGET_ENV: &str = "BO2MP_CACHE_BUDGET_MIB";
const DEFAULT_BUDGET_MIB: u64 = 4096;
/// A temp file older than this was left by a writer that died.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);
const MAX_KEY_LEN: usize = 200;

fn budget_bytes() -> u64 {
    static LIMIT_MIB: OnceLock<u64> = OnceLock::new();
    LIMIT_MIB.get_or_init(configured_budget_mib) << 20
}

fn configured_budget_mib() -> u64 {
    match std::env::var(BUDGET_ENV).map(|raw| raw.trim().parse::<u64>()) {
        Ok(Ok(mib)) => mib,
        _ => DEFAULT_BUDGET_MIB,
    }
}

fn kind_is_valid(kind: &str) -> bool {
    if kind.is_empty() {
        return false;
    }
    kind.bytes().all(|ch| ch == b'_' || ch.is_ascii_lowercase())
}

fn key_is_valid(key: &str) -> bool {
    let shaped = |ch: u8| matches!(ch, b'-' | b'_' | b'.') || ch.is_ascii_alphanumeric();
    if key.is_empty() || key.len() >= MAX_KEY_LEN {
        return false;
    }
    key.bytes().all(shaped)
}

fn entry_path(kind: &str, cache_key: &str) -> Result<PathBuf, String> {
    if !kind_is_valid(kind) || !key_is_valid(cache_key) {
        return Err("cache kind/key must be ascii [0-9a-z._-]".to_owned());
    }
    let prefix = &cache_key[..cache_key.len().min(2)];
    let base = prepare_output_folder()?;
    Ok(base.join("cache").join(kind).join(prefix).join(cache_key))
}

/// The bytes stored under `kind`/`key`, if any.
pub fn memo_fetch(kind: &str, cache_key: &str) -> Option<Vec<u8>> {
    fs::read(entry_path(kind, cache_key).ok()?).ok()
}

/// Store `bytes` under `kind`/`key`. Two writers racing on one key both
/// succeed; the first rename wins and the entries are the same bytes anyway.
pub fn memo_store(kind: &str, cache_key: &str, payload: &[u8]) -> Result<(), String> {
    let dest = entry_path(kind, cache_key)?;
    let Some(parent) = dest.parent() else {
        return Err(format!("cache path has no directory: {}", dest.display()));
    };
    fs::create_dir_all(parent).map_err(|e| format!("cache mkdir {}: {e}", parent.to_string_lossy()))?;

    let scratch = parent.join(scratch_name(cache_key));
    let drop_scratch = || {
        fs::remove_file(&scratch).ok();
    };
    fs::File::create_new(&scratch)
        .and_then(|mut handle| handle.write_all(payload))
        .map_err(|e| {
            drop_scratch();
            format!("cache write {}: {e}", scratch.display())
        })?;

    if let Err(e) = fs::rename(&scratch, &dest) {
        drop_scratch();
        // Windows will not rename onto an existing file; another writer got there first.
        if !dest.exists() {
            return Err(format!("cache rename {} -> {}: {e}", scratch.display(), dest.display()));
        }
    }
    start_sweep_once();
    Ok(())
}

/// A temp file name no other writer in any process picks for `key`.
fn scratch_name(key: &str) -> String {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let ticket = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{key}.{}.{ticket:x}.tmp", std::process::id())
}

fn start_sweep_once() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let Ok(artifacts) = prepare_output_folder() else {
            return;
        };
        let cache_dir = artifacts.join("cache");
        let builder = std::thread::Builder::new().name(String::from("bo2mp cache sweep"));
        drop(builder.spawn(move || trim_to_limit(&cache_dir, budget_bytes())));
    });
}

struct Stored {
    modified: SystemTime,
    size: u64,
    file: PathBuf,
}

/// Every blob under `cache`; temp files older than `stale_before` are
/// deleted on the way and the rest of them are left out.
fn survey(cache: &Path, stale_before: SystemTime) -> Vec<Stored> {
    let mut blobs: Vec<Stored> = vec![];
    visit_dir(cache, stale_before, &mut blobs);
    blobs
}

fn visit_dir(dir: &Path, stale_before: SystemTime, blobs: &mut Vec<Stored>) {
    let entries = fs::read_dir(dir).into_iter().flatten().flatten();
    for entry in entries {
        let Ok(stat) = entry.metadata() else { continue };
        let path = entry.path();
        let modified = stat.modified().unwrap_or(UNIX_EPOCH);
        let is_temp = path.extension().is_some_and(|ext| ext == "tmp");
        match (stat.is_dir(), is_temp) {
            (true, _) => visit_dir(&path, stale_before, blobs),
            (false, true) if modified < stale_before => {
                fs::remove_file(&path).ok();
            }
            (false, true) => {}
            (false, false) => blobs.push(Stored { modified, size: stat.len(), file: path }),
        }
    }
}

/// Delete stale temp files, then the oldest entries until the cache fits.
fn trim_to_limit(cache_dir: &Path, byte_limit: u64) {
    let mut kept = survey(cache_dir, SystemTime::now() - STALE_TEMP);
    let used: u64 = kept.iter().map(|item| item.size).sum();
    let Some(over) = used.checked_sub(byte_limit).filter(|over| *over > 0) else {
        return;
    };

    kept.sort_by_key(|item| item.modified);
    let removed_sizes: Vec<u64> = kept
        .iter()
        .scan(over, |still_over, item| {
            if *still_over == 0 {
                return None;
            }
            let removed = fs::remove_file(&item.file).is_ok();
            if removed {
                *still_over = still_over.saturating_sub(item.size);
            }
            Some(removed.then_some(item.size))
        })
        .flatten()
        .collect();
    let freed: u64 = removed_sizes.iter().sum();
    let evicted = removed_sizes.len();
    diag::info!(
        Zone,
        "artifact cache sweep: {} MiB over {} MiB budget - evicted {evicted} files ({} MiB)",
        used >> 20,
        byte_limit / (1 << 20),
        freed >> 20
    );
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a of `bytes`.
pub fn hash_text_64(bytes: &[u8]) -> u64 {
    fnv1a64_extend(FNV_OFFSET, bytes)
}

/// Continue an FNV-1a hash with more bytes.
pub fn fnv1a64_extend(hash: u64, data: &[u8]) -> u64 {
    data
        .iter()
        .fold(hash, |acc, &byte| (acc ^ u64::from(byte)).wrapping_mul(FNV_PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_published_vectors() {
        let vectors: [(&[u8], u64); 3] = [
            (b"", 0xcbf2_9ce4_8422_2325),
            (b"a", 0xaf63_dc4c_8601_ec8c),
            (b"foobar", 0x8594_4171_f739_67e8),
        ];
        for (input, expected) in vectors {
            assert_eq!(hash_text_64(input), expected);
        }
        let split = fnv1a64_extend(hash_text_64(b"foo"), b"bar");
        assert_eq!(split, hash_text_64(b"foobar"));
    }

    #[test]
    fn kinds_and_keys_are_checked() {
        assert!(kind_is_valid("image_blob"));
        assert!(!kind_is_valid("Image"));
        assert!(!kind_is_valid(""));
        assert!(key_is_valid("ab-12_x.9"));
        assert!(!key_is_valid("a/b"));
        assert!(!key_is_valid(&"a".repeat(MAX_KEY_LEN)));
        assert!(memo_fetch("Bad Kind", "ok").is_none());
        assert!(memo_store("ok", "a/b", b"x").is_err());
    }

    #[test]
    fn survey_lists_blobs_and_drops_only_stale_temp_files() {
        let base = std::env::temp_dir().join(format!("bo2mp-survey-{}", std::process::id()));
        let bucket = base.join("kind").join("bb");
        fs::create_dir_all(&bucket).unwrap();
        for (name, len) in [("blob", 7usize), ("old.tmp", 3), ("fresh.tmp", 3)] {
            fs::write(bucket.join(name), vec![1u8; len]).unwrap();
        }
        let aged = SystemTime::now() - Duration::from_secs(100);
        fs::File::options().write(true).open(bucket.join("old.tmp")).unwrap().set_modified(aged).unwrap();

        let found = survey(&base, SystemTime::now() - Duration::from_secs(50));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].size, 7);
        assert!(!bucket.join("old.tmp").exists());
        assert!(bucket.join("fresh.tmp").exists());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn sweep_evicts_oldest_until_the_budget_fits() {
        let base = std::env::temp_dir().join(format!("bo2mp-sweep-{}", std::process::id()));
        let bucket = base.join("kind").join("aa");
        fs::create_dir_all(&bucket).unwrap();
        let [older, newer] = ["old", "new"].map(|name| bucket.join(name));
        for file in [&older, &newer] {
            fs::write(file, [0u8; 100]).unwrap();
        }
        let aged = SystemTime::now() - Duration::from_secs(10);
        fs::File::options().write(true).open(&older).unwrap().set_modified(aged).unwrap();

        trim_to_limit(&base, 150);
        assert!(!older.exists());
        assert!(newer.exists());
        fs::remove_dir_all(&base).ok();
    }
}
