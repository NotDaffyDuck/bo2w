//! The zip players download: every input at the top level, wrapped in the old
//! zip password so mail and download scanners do not unpack the exe.

use std::fs::File;
use std::path::Path;

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;

pub const PASSWORD_VAR: &str = "BO2MP_ARCHIVE_PASSWORD";

/// `cargo xtask bundle-zip ARCHIVE FILE...`, password from
/// `BO2MP_ARCHIVE_PASSWORD`.
pub fn launch(cli_words: Vec<String>) -> Result<(), String> {
    let usage = "usage: cargo xtask bundle-zip ARCHIVE FILE...";
    let (zip_path, files) = match cli_words.as_slice() {
        [zip_path, files @ ..] if !files.is_empty() => (zip_path, files),
        _ => return Err(usage.to_string()),
    };
    let password = match std::env::var(PASSWORD_VAR) {
        Ok(password) if !password.is_empty() => password,
        _ => return Err(format!("set {PASSWORD_VAR}")),
    };
    pack_zip(Path::new(zip_path), files, password.as_ref())
}

fn entry_name(file_at: &Path) -> Result<&str, String> {
    let title = file_at.file_name().and_then(|title| title.to_str());
    title.ok_or_else(|| format!("{} has no UTF-8 file name", file_at.display()))
}

/// Each input paired with its entry name. Entries are named by file name
/// alone, so two inputs may not share one; the first problem found wins.
fn plan_entries(files: &[String]) -> Result<Vec<(&str, &str)>, String> {
    files.iter().try_fold(Vec::new(), |mut plan, file| {
        let file_at = Path::new(file);
        let title = match file_at.is_file() {
            true => entry_name(file_at)?,
            false => return Err(format!("not a file: {file}")),
        };
        if plan.iter().any(|&(_, taken)| taken == title) {
            return Err(format!("two inputs are both named {title}"));
        }
        plan.push((file.as_str(), title));
        Ok(plan)
    })
}

const ENTRY_MODE: u32 = 0o644;

fn entry_options(password: &[u8]) -> SimpleFileOptions {
    let method = CompressionMethod::Deflated;
    let plain = SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(ENTRY_MODE);
    plain.with_deprecated_encryption(password)
}

pub fn pack_zip(zip_path: &Path, inputs: &[String], lock_word: &[u8]) -> Result<(), String> {
    let plan = plan_entries(inputs)?;
    let zip_shown = zip_path.display();
    let zip_file = File::create(zip_path).map_err(|e| format!("creating {zip_shown}: {e}"))?;
    let mut zip = ZipWriter::new(zip_file);
    let options = entry_options(lock_word);
    plan.into_iter().try_for_each(|(file, title)| {
        zip.start_file(title, options)
            .map_err(|e| format!("{zip_shown}: {e}"))?;
        let mut source = File::open(file).map_err(|error| format!("opening {file}: {error}"))?;
        std::io::copy(&mut source, &mut zip)
            .map(drop)
            .map_err(|error| format!("adding {file}: {error}"))
    })?;
    zip.finish()
        .map(drop)
        .map_err(|error| format!("finishing {}: {error}", zip_path.display()))
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn scratch(git_label: &str) -> std::path::PathBuf {
        let base_dir = std::env::temp_dir().join(format!("xtask-zip-{git_label}-{}", std::process::id()));
        std::fs::create_dir_all(&base_dir).unwrap();
        base_dir
    }

    #[test]
    fn files_come_back_out_with_the_password() {
        let base_dir = scratch("roundtrip");
        let a = base_dir.join("a.txt");
        let b = base_dir.join("sub");
        std::fs::create_dir_all(&b).unwrap();
        let b = b.join("b.bin");
        std::fs::write(&a, b"alpha").unwrap();
        std::fs::write(&b, vec![7_u8; 200_000]).unwrap();
        let zip_path = base_dir.join("out.zip");
        let inputs = [a.display().to_string(), b.display().to_string()];
        pack_zip(&zip_path, &inputs, b"pw").unwrap();

        let mut zip = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        let mut text = Vec::new();
        zip.by_name_decrypt("a.txt", b"pw")
            .unwrap()
            .read_to_end(&mut text)
            .unwrap();
        assert_eq!(text, b"alpha");
        let mut zst_blob = Vec::new();
        zip.by_name_decrypt("b.bin", b"pw")
            .unwrap()
            .read_to_end(&mut zst_blob)
            .unwrap();
        assert_eq!(zst_blob.len(), 200_000);
        assert!(zip.by_name_decrypt("a.txt", b"wrong").is_err());
        std::fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn two_inputs_with_one_name_are_refused() {
        let base_dir = scratch("dupe");
        let one = base_dir.join("x");
        let two = base_dir.join("y");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("same.txt"), b"1").unwrap();
        std::fs::write(two.join("same.txt"), b"2").unwrap();
        let inputs = [
            one.join("same.txt").display().to_string(),
            two.join("same.txt").display().to_string(),
        ];
        let error = pack_zip(&base_dir.join("out.zip"), &inputs, b"pw").unwrap_err();
        assert!(error.contains("same.txt"), "{error}");
        let missing = [base_dir.join("absent").display().to_string()];
        assert!(pack_zip(&base_dir.join("out.zip"), &missing, b"pw").is_err());
        std::fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn run_wants_an_archive_and_at_least_one_file() {
        for bad in [vec![], vec!["only.zip".to_string()]] {
            let error = launch(bad).unwrap_err();
            assert_eq!(error, "usage: cargo xtask bundle-zip ARCHIVE FILE...");
        }
    }

    #[test]
    fn the_first_problem_in_input_order_is_the_one_reported() {
        let base_dir = scratch("order");
        std::fs::write(base_dir.join("a.txt"), b"1").unwrap();
        let a = base_dir.join("a.txt").display().to_string();
        let gone = base_dir.join("gone.txt").display().to_string();
        let error = plan_entries(&[a.clone(), a.clone(), gone.clone()]).unwrap_err();
        assert_eq!(error, "two inputs are both named a.txt");
        let error = plan_entries(&[a, gone.clone()]).unwrap_err();
        assert_eq!(error, format!("not a file: {gone}"));
        std::fs::remove_dir_all(&base_dir).unwrap();
    }
}
