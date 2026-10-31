//! Writing the files commands leave in the artifacts folder.

use std::{fs, io::Write, path::Path};

/// Puts `contents` at `target` so that nobody ever reads half of it, and so that a
/// file already there is never replaced. The body is written beside it under
/// a temporary name, then linked into place.
pub(super) fn write_new_file(target: &Path, contents: &str) -> Result<(), String> {
    let shown = target.display();
    let folder = target
        .parent()
        .ok_or_else(|| format!("{shown} has no folder"))?;
    let name = Path::file_name(target)
        .ok_or_else(|| format!("{shown} has no file name"))?;
    fs::create_dir_all(folder).map_err(|e| format!("create {}: {e}", folder.display()))?;
    let scratch = folder.join(format!(".{}.tmp", name.to_string_lossy()));
    let linked = write_scratch(&scratch, contents).and_then(|()| {
        fs::hard_link(&scratch, target)
            .map_err(|why| format!("link {} to {shown}: {why}", scratch.display()))
    });
    let removed =
        fs::remove_file(&scratch).map_err(|e| format!("remove {}: {e}", scratch.display()));
    linked.and(removed)
}

fn write_scratch(spare: &Path, text: &str) -> Result<(), String> {
    let fresh = true;
    let mut out = std::fs::File::options()
        .write(true)
        .create_new(fresh)
        .open(spare)
        .map_err(|e| format!("create {}: {e}", spare.display()))?;
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("write {}: {e}", spare.display()))
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
    use crate::tool_cmds::files::*;

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
