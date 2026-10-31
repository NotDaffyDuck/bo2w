//! Putting a manifest's files into the install folder. A file is checked
//! before it is written and written so that a failure leaves the old one.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::{FileEntry, hash_digits};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The folder already had exactly this file.
    Current,
    /// The file was fetched and written.
    Installed,
}

/// Makes `dir` hold `entry`. `fetch` is asked, only if needed, for the bytes
/// the channel has under a name.
pub fn bring_in(
    dir: &Path,
    entry: &FileEntry,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
) -> Result<Outcome, String> {
    let target = dir.join(&entry.path);
    if is_current(&target, entry)? {
        return Ok(Outcome::Current);
    }
    let shipped = fetch(entry.remote_name())?;
    let bytes = unpack(entry, shipped)?;
    replace_file(&target, &bytes)?;
    Ok(Outcome::Installed)
}

fn is_current(path: &Path, entry: &FileEntry) -> Result<bool, String> {
    let wanted = hash_digits(&entry.sha256)?;
    let right_size = match fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.len() == entry.size,
        Err(_) => false,
    };
    // Only a file of the right size is worth hashing.
    Ok(right_size && matches!(file_digest(path), Ok(found) if found == wanted))
}

/// `what size <got> != <want>` when a length is off.
fn check_len(what: &str, got: usize, want: u64) -> Result<(), String> {
    if got as u64 == want { Ok(()) } else { Err(format!("{what} size {got} != {want}")) }
}

/// The file's bytes, once the download has been checked and, if it came
/// compressed, expanded. Every number the manifest gives is held to.
fn unpack(entry: &FileEntry, shipped: Vec<u8>) -> Result<Vec<u8>, String> {
    let remote = entry.remote_name();
    if let Some(want) = entry.download_size {
        check_len(remote, shipped.len(), want)?;
    }
    if let Some(want_hash) = entry.download_sha256.as_deref() {
        let want = hash_digits(want_hash)?;
        if digest(&shipped) != want {
            return Err(format!("{remote}: download sha256 mismatch"));
        }
    }
    let installed = entry.path.as_str();
    let plain = match remote.strip_suffix(".zst") {
        Some(_) => expand(remote, &shipped, entry.size)?,
        None => shipped,
    };
    check_len(installed, plain.len(), entry.size)?;
    let want = hash_digits(&entry.sha256)?;
    match digest(&plain) == want {
        true => Ok(plain),
        false => Err(format!("{installed} sha256 mismatch")),
    }
}

/// Expands zstd data, reading no more than one byte past the size the
/// manifest promises, so a small download cannot become an enormous file.
fn expand(name: &str, shipped: &[u8], promised: u64) -> Result<Vec<u8>, String> {
    let limit = promised.saturating_add(1);
    let mut out = Vec::new();
    zstd::stream::read::Decoder::new(shipped)
        .and_then(|decoder| decoder.take(limit).read_to_end(&mut out))
        .map_err(|why| format!("decode {name}: {why}"))?;
    Ok(out)
}

/// Writes `bytes` to `destination` without ever leaving it half written: the
/// bytes go to a `.part` file first, the old file steps aside as `.bak`
/// (a running `.exe` can be renamed, not overwritten) and comes back if the
/// new one cannot be moved in.
pub fn replace_file(target: &Path, contents: &[u8]) -> Result<(), String> {
    let [staged, prior_copy] = [".part", ".bak"].map(|suffix| with_suffix(target, suffix));
    stage(&staged, contents)?;
    let had_old = target.exists();
    let shown = target.display();
    // Cleanup below is best effort: the error that matters is already known.
    if had_old {
        if let Err(why) = fs::rename(target, &prior_copy) {
            fs::remove_file(&staged).ok();
            return Err(format!("rename {shown} to backup: {why}"));
        }
    }
    match fs::rename(&staged, target) {
        Ok(()) => {
            if had_old {
                fs::remove_file(&prior_copy).ok();
            }
            Ok(())
        }
        Err(why) => {
            if had_old {
                fs::rename(&prior_copy, target).ok();
            }
            fs::remove_file(&staged).ok();
            Err(format!("install {shown}: {why}"))
        }
    }
}

/// Writes `bytes` to `path` and makes sure they reached the disk.
fn stage(path: &Path, contents: &[u8]) -> Result<(), String> {
    let step = |verb: &str| {
        let shown = path.display().to_string();
        let verb = verb.to_owned();
        move |why: io::Error| format!("{verb} {shown}: {why}")
    };
    let result = (|| {
        let mut out = File::create(path).map_err(step("create"))?;
        out.write_all(contents).map_err(step("write"))?;
        out.sync_all().map_err(step("sync"))
    })();
    if result.is_err() {
        fs::remove_file(path).ok();
    }
    result
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut full = OsString::from(path.as_os_str());
    full.push(suffix);
    full.into()
}

/// Lowercase hex of a finished hash.
fn hex(state: Sha256) -> String {
    state.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The SHA-256 of `bytes` as lowercase hex.
pub fn digest(bytes: &[u8]) -> String {
    let mut state = Sha256::new();
    state.update(bytes);
    hex(state)
}

/// The SHA-256 of a file's contents as lowercase hex.
pub fn file_digest(path: &Path) -> io::Result<String> {
    let mut source = File::open(path)?;
    let mut state = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut filled = source.read(&mut buffer)?;
    while filled > 0 {
        state.update(&buffer[..filled]);
        filled = source.read(&mut buffer)?;
    }
    Ok(hex(state))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    fn entry_for(path: &str, content: &[u8]) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            sha256: digest(content),
            size: content.len() as u64,
            download: None,
            download_sha256: None,
            download_size: None,
        }
    }

    /// An entry whose file is shipped zstd-compressed under `<path>.zst`.
    fn shipped_entry(path: &str, content: &[u8]) -> (FileEntry, Vec<u8>) {
        let blob = zstd::encode_all(content, 3).unwrap();
        let entry = FileEntry {
            download: Some(format!("{path}.zst")),
            download_sha256: Some(digest(&blob)),
            download_size: Some(blob.len() as u64),
            ..entry_for(path, content)
        };
        (entry, blob)
    }

    fn never(name: &str) -> Result<Vec<u8>, String> {
        panic!("nothing should have been fetched, but {name} was")
    }

    #[test]
    fn digests_are_lowercase_hex() {
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_file_hashes_the_same_as_its_bytes() {
        let dir = TestDir::new("digest");
        let big: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        fs::write(dir.join("big"), &big).unwrap();
        assert_eq!(file_digest(&dir.join("big")).unwrap(), digest(&big));
        assert!(file_digest(&dir.join("missing")).is_err());
    }

    #[test]
    fn a_plain_file_is_fetched_by_its_own_name() {
        let dir = TestDir::new("plain");
        let entry = entry_for("bo2mp-ca.pem", b"cert");
        let outcome = bring_in(dir.path(), &entry, |name| {
            assert_eq!(name, "bo2mp-ca.pem");
            Ok(b"cert".to_vec())
        });
        assert_eq!(outcome, Ok(Outcome::Installed));
        assert_eq!(fs::read(dir.join("bo2mp-ca.pem")).unwrap(), b"cert");
    }

    #[test]
    fn a_compressed_download_is_fetched_by_its_download_name_and_expanded() {
        let dir = TestDir::new("zst");
        let game: Vec<u8> = b"MZ game bytes ".repeat(500);
        let (entry, blob) = shipped_entry("bo2mp.exe", &game);
        let outcome = bring_in(dir.path(), &entry, |name| {
            assert_eq!(name, "bo2mp.exe.zst");
            Ok(blob.clone())
        });
        assert_eq!(outcome, Ok(Outcome::Installed));
        assert_eq!(fs::read(dir.join("bo2mp.exe")).unwrap(), game);
    }

    #[test]
    fn a_file_that_is_already_right_is_left_alone() {
        let dir = TestDir::new("current");
        fs::write(dir.join("bo2mp-ca.pem"), b"cert").unwrap();
        let entry = entry_for("bo2mp-ca.pem", b"cert");
        assert_eq!(bring_in(dir.path(), &entry, never), Ok(Outcome::Current));
    }

    #[test]
    fn an_old_file_is_replaced_and_leaves_no_litter() {
        let dir = TestDir::new("replace");
        fs::write(dir.join("bo2mp.exe"), b"old").unwrap();
        let entry = entry_for("bo2mp.exe", b"newer");
        let outcome = bring_in(dir.path(), &entry, |_| Ok(b"newer".to_vec()));
        assert_eq!(outcome, Ok(Outcome::Installed));
        assert_eq!(fs::read(dir.join("bo2mp.exe")).unwrap(), b"newer");
        assert_eq!(dir.entry_names(), ["bo2mp.exe"]);
    }

    #[test]
    fn same_size_but_different_bytes_is_not_current() {
        let dir = TestDir::new("samesize");
        fs::write(dir.join("f"), b"aaaa").unwrap();
        let entry = entry_for("f", b"bbbb");
        let outcome = bring_in(dir.path(), &entry, |_| Ok(b"bbbb".to_vec()));
        assert_eq!(outcome, Ok(Outcome::Installed));
        assert_eq!(fs::read(dir.join("f")).unwrap(), b"bbbb");
    }

    #[test]
    fn a_bad_download_leaves_the_old_file_in_place() {
        let dir = TestDir::new("bad");
        fs::write(dir.join("bo2mp.exe"), b"old").unwrap();
        let game = b"the real game".repeat(40);
        let (entry, blob) = shipped_entry("bo2mp.exe", &game);

        let mut flipped = blob.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        let tampered = bring_in(dir.path(), &entry, |_| Ok(flipped)).unwrap_err();
        assert!(tampered.contains("download sha256 mismatch"), "{tampered}");

        let mut longer = blob.clone();
        longer.push(0);
        let wrong_size = bring_in(dir.path(), &entry, |_| Ok(longer)).unwrap_err();
        assert!(wrong_size.contains("bo2mp.exe.zst size"), "{wrong_size}");

        let failed = bring_in(dir.path(), &entry, |_| Err("GET: 404".to_owned()));
        assert_eq!(failed, Err("GET: 404".to_owned()));

        assert_eq!(fs::read(dir.join("bo2mp.exe")).unwrap(), b"old");
        assert_eq!(dir.entry_names(), ["bo2mp.exe"]);
    }

    #[test]
    fn the_installed_bytes_must_match_the_manifest() {
        let dir = TestDir::new("final");
        let entry = entry_for("f", b"expected");
        let wrong_hash = bring_in(dir.path(), &entry, |_| Ok(b"EXPECTED".to_vec())).unwrap_err();
        assert_eq!(wrong_hash, "f sha256 mismatch");
        let wrong_size = bring_in(dir.path(), &entry, |_| Ok(b"short".to_vec())).unwrap_err();
        assert_eq!(wrong_size, "f size 5 != 8");
        assert!(dir.entry_names().is_empty());
    }

    #[test]
    fn a_download_that_expands_past_its_size_is_stopped() {
        let dir = TestDir::new("bomb");
        let huge = vec![0u8; 8 * 1024 * 1024];
        let blob = zstd::encode_all(&huge[..], 3).unwrap();
        assert!(blob.len() < 4096, "zeros should compress hard");
        let promised = b"only a few bytes";
        let entry = FileEntry {
            download: Some("f.zst".to_owned()),
            ..entry_for("f", promised)
        };
        let error = bring_in(dir.path(), &entry, |_| Ok(blob)).unwrap_err();
        assert_eq!(
            error,
            format!("f size {} != {}", promised.len() + 1, promised.len())
        );
        assert!(dir.entry_names().is_empty());
    }

    #[test]
    fn a_download_that_is_not_zstd_is_a_decode_error() {
        let dir = TestDir::new("notzstd");
        let entry = FileEntry {
            download: Some("f.zst".to_owned()),
            ..entry_for("f", b"content")
        };
        let error = bring_in(dir.path(), &entry, |_| Ok(b"plain text".to_vec())).unwrap_err();
        assert!(error.starts_with("decode f.zst:"), "{error}");
    }

    #[test]
    fn replacing_in_a_folder_that_is_gone_is_a_create_error() {
        let dir = TestDir::new("gone");
        let error = replace_file(&dir.join("no/such/dir/f"), b"x").unwrap_err();
        assert!(error.starts_with("create "), "{error}");
    }

    #[test]
    fn a_file_that_cannot_step_aside_keeps_its_place() {
        let dir = TestDir::new("stuck");
        fs::write(dir.join("bo2mp.exe"), b"old").unwrap();
        // A folder already sits where the prior_copy would go.
        fs::create_dir(dir.join("bo2mp.exe.bak")).unwrap();
        fs::write(dir.join("bo2mp.exe.bak/inner"), b"x").unwrap();
        let error = replace_file(&dir.join("bo2mp.exe"), b"new").unwrap_err();
        assert!(error.contains("to backup"), "{error}");
        assert_eq!(fs::read(dir.join("bo2mp.exe")).unwrap(), b"old");
        assert!(!dir.join("bo2mp.exe.part").exists());
    }
}
