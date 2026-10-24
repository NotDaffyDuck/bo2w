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

use crate::discover::ensure_artifacts_dir;

const BUDGET_ENV: &str = "BO2MP_CACHE_BUDGET_MIB";
const DEFAULT_BUDGET_MIB: u64 = 4096;
/// A temp file older than this was left by a writer that died.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);
const MAX_KEY_LEN: usize = 200;

fn budget_bytes() -> u64 {
    static BUDGET: OnceLock<u64> = OnceLock::new();
    let mib = *BUDGET.get_or_init(|| {
        std::env::var(BUDGET_ENV)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(DEFAULT_BUDGET_MIB)
    });
    mib << 20
}

fn kind_is_valid(kind: &str) -> bool {
    !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn key_is_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() < MAX_KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn entry_path(kind: &str, key: &str) -> Result<PathBuf, String> {
    if !kind_is_valid(kind) || !key_is_valid(key) {
        return Err("cache kind/key must be ascii [0-9a-z._-]".to_owned());
    }
    let shard = &key[..key.len().min(2)];
    Ok(ensure_artifacts_dir()?
        .join("cache")
        .join(kind)
        .join(shard)
        .join(key))
}

/// The bytes stored under `kind`/`key`, if any.
pub fn cache_get(kind: &str, key: &str) -> Option<Vec<u8>> {
    fs::read(entry_path(kind, key).ok()?).ok()
}

/// Store `bytes` under `kind`/`key`. Two writers racing on one key both
/// succeed; the first rename wins and the entries are the same bytes anyway.
pub fn cache_put(kind: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let target = entry_path(kind, key)?;
    let dir = target
        .parent()
        .ok_or_else(|| format!("cache path has no directory: {}", target.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("cache mkdir {}: {e}", dir.display()))?;

    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!("{key}.{}.{serial:x}.tmp", std::process::id()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| file.write_all(bytes));
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(format!("cache write {}: {e}", temp.display()));
    }

    if let Err(e) = fs::rename(&temp, &target) {
        let _ = fs::remove_file(&temp);
        // Windows refuses to rename over an existing file; someone else stored it.
        if !target.exists() {
            return Err(format!(
                "cache rename {} -> {}: {e}",
                temp.display(),
                target.display()
            ));
        }
    }
    start_sweep_once();
    Ok(())
}

fn start_sweep_once() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let Ok(root) = ensure_artifacts_dir() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("bo2mp cache sweep".to_owned())
            .spawn(move || sweep(&root.join("cache"), budget_bytes()));
    });
}

struct Stored {
    modified: SystemTime,
    len: u64,
    path: PathBuf,
}

/// Delete stale temp files, then the oldest entries until the cache fits.
fn sweep(cache: &Path, budget: u64) {
    let cutoff = SystemTime::now() - STALE_TEMP;
    let mut stored = Vec::new();
    let mut total = 0u64;
    let mut pending = vec![cache.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(listing) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in listing.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            if meta.is_dir() {
                pending.push(path);
                continue;
            }
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            if path.extension().is_some_and(|ext| ext == "tmp") {
                if modified < cutoff {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            total += meta.len();
            stored.push(Stored {
                modified,
                len: meta.len(),
                path,
            });
        }
    }
    if total <= budget {
        return;
    }

    stored.sort_by_key(|entry| entry.modified);
    let (mut freed, mut evicted) = (0u64, 0usize);
    for entry in stored {
        if total - freed <= budget {
            break;
        }
        if fs::remove_file(&entry.path).is_ok() {
            freed += entry.len;
            evicted += 1;
        }
    }
    diag::info!(
        Zone,
        "artifact cache sweep: {} MiB over {} MiB budget - evicted {evicted} files ({} MiB)",
        total >> 20,
        budget >> 20,
        freed >> 20
    );
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_more(FNV_OFFSET, bytes)
}

/// Continue an FNV-1a hash with more bytes.
pub fn fnv1a64_more(hash: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(hash, |h, &b| (h ^ u64::from(b)).wrapping_mul(FNV_PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a64_more(fnv1a64(b"foo"), b"bar"), fnv1a64(b"foobar"));
    }

    #[test]
    fn kinds_and_keys_are_checked() {
        assert!(kind_is_valid("image_blob"));
        assert!(!kind_is_valid("Image"));
        assert!(!kind_is_valid(""));
        assert!(key_is_valid("ab-12_x.9"));
        assert!(!key_is_valid("a/b"));
        assert!(!key_is_valid(&"a".repeat(MAX_KEY_LEN)));
        assert!(cache_get("Bad Kind", "ok").is_none());
        assert!(cache_put("ok", "a/b", b"x").is_err());
    }

    #[test]
    fn sweep_evicts_oldest_until_the_budget_fits() {
        let root = std::env::temp_dir().join(format!("bo2mp-sweep-{}", std::process::id()));
        let shard = root.join("kind").join("aa");
        fs::create_dir_all(&shard).unwrap();
        let old = shard.join("old");
        let new = shard.join("new");
        fs::write(&old, [0u8; 100]).unwrap();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(10))
            .unwrap();
        fs::write(&new, [0u8; 100]).unwrap();

        sweep(&root, 150);
        assert!(!old.exists());
        assert!(new.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
