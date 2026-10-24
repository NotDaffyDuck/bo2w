//! Writing the files commands leave in the artifacts folder.

use std::{fs, io::Write, path::Path};

/// Puts `body` at `path` so that nobody ever reads half of it, and so that a
/// file already there is never replaced. The body is written beside it under
/// a temporary name, then linked into place.
pub(super) fn write_new_file(path: &Path, body: &str) -> Result<(), String> {
    let folder = path
        .parent()
        .ok_or_else(|| format!("{} has no folder", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    fs::create_dir_all(folder).map_err(|e| format!("create {}: {e}", folder.display()))?;
    let scratch = folder.join(format!(".{}.tmp", name.to_string_lossy()));
    let linked = write_scratch(&scratch, body).and_then(|()| {
        fs::hard_link(&scratch, path)
            .map_err(|e| format!("link {} to {}: {e}", scratch.display(), path.display()))
    });
    let removed =
        fs::remove_file(&scratch).map_err(|e| format!("remove {}: {e}", scratch.display()));
    linked.and(removed)
}

fn write_scratch(scratch: &Path, body: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(scratch)
        .map_err(|e| format!("create {}: {e}", scratch.display()))?;
    file.write_all(body.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| format!("write {}: {e}", scratch.display()))
}

/// An empty folder name under the temp folder for a test to use; the test
/// removes it when done.
#[cfg(test)]
pub(super) fn fresh_folder(tag: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("bo2mp-console-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    folder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_lands_whole_and_no_scratch_file_is_left() {
        let folder = fresh_folder("files-whole");
        let path = folder.join("deep").join("out.txt");
        write_new_file(&path, "hello\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello\n");
        let left: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_already_there_is_kept_and_the_scratch_file_goes() {
        let folder = fresh_folder("files-kept");
        let path = folder.join("out.txt");
        write_new_file(&path, "first").unwrap();
        let error = write_new_file(&path, "second").unwrap_err();
        assert!(error.starts_with("link "), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_path_with_no_file_name_is_refused() {
        let error = write_new_file(Path::new(""), "x").unwrap_err();
        assert!(error.contains("has no"), "{error}");
    }
}
