//! The zip players download: every input at the top level, wrapped in the old
//! zip password so mail and download scanners do not unpack the exe.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;

use crate::shell::Res;

pub const PASSWORD_VAR: &str = "BO2MP_ARCHIVE_PASSWORD";

/// `cargo xtask bundle-zip ARCHIVE FILE...`, password from
/// `BO2MP_ARCHIVE_PASSWORD`.
pub fn run(args: Vec<String>) -> Res<()> {
    let usage = || "usage: cargo xtask bundle-zip ARCHIVE FILE...".to_string();
    let (archive, files) = args.split_first().ok_or_else(usage)?;
    if files.is_empty() {
        return Err(usage());
    }
    let password = std::env::var(PASSWORD_VAR)
        .ok()
        .filter(|password| !password.is_empty())
        .ok_or_else(|| format!("set {PASSWORD_VAR}"))?;
    write_archive(Path::new(archive), files, password.as_bytes())
}

fn entry_name(path: &Path) -> Res<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", path.display()))
}

/// Entries are named by file name alone, so two inputs may not share one.
pub fn write_archive(archive: &Path, files: &[String], password: &[u8]) -> Res<()> {
    let mut seen = BTreeSet::new();
    for file in files {
        let path = Path::new(file);
        if !path.is_file() {
            return Err(format!("not a file: {file}"));
        }
        let name = entry_name(path)?;
        if !seen.insert(name) {
            return Err(format!("two inputs are both named {name}"));
        }
    }

    let out = File::create(archive)
        .map_err(|error| format!("creating {}: {error}", archive.display()))?;
    let mut zip = ZipWriter::new(out);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644)
        .with_deprecated_encryption(password);
    for file in files {
        let path = Path::new(file);
        zip.start_file(entry_name(path)?, options)
            .map_err(|error| format!("{}: {error}", archive.display()))?;
        let mut input = File::open(path).map_err(|error| format!("opening {file}: {error}"))?;
        std::io::copy(&mut input, &mut zip).map_err(|error| format!("adding {file}: {error}"))?;
    }
    zip.finish()
        .map_err(|error| format!("finishing {}: {error}", archive.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-zip-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_come_back_out_with_the_password() {
        let dir = scratch("roundtrip");
        let a = dir.join("a.txt");
        let b = dir.join("sub");
        std::fs::create_dir_all(&b).unwrap();
        let b = b.join("b.bin");
        std::fs::write(&a, b"alpha").unwrap();
        std::fs::write(&b, vec![7_u8; 200_000]).unwrap();
        let archive = dir.join("out.zip");
        let inputs = [a.display().to_string(), b.display().to_string()];
        write_archive(&archive, &inputs, b"pw").unwrap();

        let mut zip = zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        let mut text = Vec::new();
        zip.by_name_decrypt("a.txt", b"pw")
            .unwrap()
            .read_to_end(&mut text)
            .unwrap();
        assert_eq!(text, b"alpha");
        let mut blob = Vec::new();
        zip.by_name_decrypt("b.bin", b"pw")
            .unwrap()
            .read_to_end(&mut blob)
            .unwrap();
        assert_eq!(blob.len(), 200_000);
        assert!(zip.by_name_decrypt("a.txt", b"wrong").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_inputs_with_one_name_are_refused() {
        let dir = scratch("dupe");
        let one = dir.join("x");
        let two = dir.join("y");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("same.txt"), b"1").unwrap();
        std::fs::write(two.join("same.txt"), b"2").unwrap();
        let inputs = [
            one.join("same.txt").display().to_string(),
            two.join("same.txt").display().to_string(),
        ];
        let error = write_archive(&dir.join("out.zip"), &inputs, b"pw").unwrap_err();
        assert!(error.contains("same.txt"), "{error}");
        let missing = [dir.join("absent").display().to_string()];
        assert!(write_archive(&dir.join("out.zip"), &missing, b"pw").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
