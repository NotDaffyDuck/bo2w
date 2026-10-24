//! A scratch folder for tests, deleted when it goes out of scope.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct TestDir(PathBuf);

impl TestDir {
    pub fn new(tag: &str) -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let number = COUNT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "bo2mp-updater-test-{}-{number}-{tag}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("the temp folder can be made");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// The names of everything directly inside, in order.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("the folder can be listed")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
