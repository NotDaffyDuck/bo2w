//! Where demos live and what they are called.
//!
//! A demo is `<artifacts>/demos/<name>.bo2mpdemo`. Names are typed at the
//! console, so every one is cut down to a safe file name before it touches the
//! disk.

use std::path::{Path, PathBuf};

const EXTENSION: &str = ".bo2mpdemo";
const DEMO_FOLDER: &str = "demos";
const MAX_NAME_LEN: usize = 64;
const AUTO_NAMES: u32 = 10_000;

/// The file a demo called `stem` is stored in.
pub fn recording_file(artifacts: &Path, stem: &str) -> PathBuf {
    artifacts.join(DEMO_FOLDER).join(format!("{stem}{EXTENSION}"))
}

/// `name` with everything but letters, digits, `_` and `-` removed and at most
/// 64 characters kept; `None` when nothing is left.
pub fn tidy_clip_name(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(MAX_NAME_LEN)
        .collect();
    (!kept.is_empty()).then_some(kept)
}

/// What a player typed for a demo, reduced to the name itself: no folders, no
/// `.bo2mpdemo`.
pub fn recording_stem(requested: &str) -> &str {
    let file = requested.rsplit(['/', '\\']).next().unwrap_or(requested);
    file.strip_suffix(EXTENSION).unwrap_or(file)
}

/// Older spelling bootstrap still calls; the rename sweep moves it over.

/// The name the `slot`th unnamed recording gets.
pub fn generated_recording_name(slot: u32) -> String {
    format!("demo{slot:04}")
}

/// The first unnamed-recording name with no demo behind it yet.
pub fn first_unused_demo_name(artifacts: &Path) -> String {
    let stamp = std::time::UNIX_EPOCH.elapsed().map_or(0, |since| since.as_secs());
    first_free_name(|name| recording_file(artifacts, name).exists(), stamp)
}

/// The first `demoNNNN` that `taken` does not claim; when all 10,000 are
/// taken, `demo` and `stamp` instead.
fn first_free_name(taken: impl Fn(&str) -> bool, stamp: u64) -> String {
    (0..AUTO_NAMES)
        .map(generated_recording_name)
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("demo{stamp}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn a_demo_lives_under_demos_with_the_extension() {
        let path = recording_file(Path::new("art"), "round1");
        assert_eq!(
            path,
            Path::new("art").join("demos").join("round1.bo2mpdemo")
        );
    }

    #[test]
    fn sanitizing_keeps_letters_digits_underscore_and_dash() {
        assert_eq!(
            tidy_clip_name("Nuke_Town-2"),
            Some("Nuke_Town-2".to_owned())
        );
    }

    #[test]
    fn sanitizing_drops_everything_else() {
        assert_eq!(
            tidy_clip_name("my demo!.. /x"),
            Some("mydemox".to_owned())
        );
        assert_eq!(tidy_clip_name("../../etc"), Some("etc".to_owned()));
        assert_eq!(tidy_clip_name("naïve"), Some("nave".to_owned()));
    }

    #[test]
    fn sanitizing_cuts_at_64_characters() {
        let long = "a".repeat(100);
        assert_eq!(tidy_clip_name(&long), Some("a".repeat(64)));
    }

    #[test]
    fn sanitizing_nothing_gives_none() {
        for empty in ["", "..", "!!! ", "/\\"] {
            assert_eq!(tidy_clip_name(empty), None, "{empty:?}");
        }
    }

    #[test]
    fn the_stem_drops_folders_and_the_extension() {
        assert_eq!(recording_stem("round1"), "round1");
        assert_eq!(recording_stem("round1.bo2mpdemo"), "round1");
        assert_eq!(recording_stem("demos/round1.bo2mpdemo"), "round1");
        assert_eq!(recording_stem("C:\\games\\demos\\round1"), "round1");
        assert_eq!(recording_stem("a/b\\c.bo2mpdemo"), "c");
    }

    #[test]
    fn the_stem_of_a_folder_is_empty() {
        assert_eq!(recording_stem("demos/"), "");
        assert_eq!(recording_stem(""), "");
    }

    #[test]
    fn the_stem_removes_only_one_trailing_extension() {
        assert_eq!(recording_stem("x.bo2mpdemo.bo2mpdemo"), "x.bo2mpdemo");
        assert_eq!(recording_stem("x.bo2mpdemo.bak"), "x.bo2mpdemo.bak");
    }

    #[test]
    fn automatic_names_are_four_digits_wide() {
        assert_eq!(generated_recording_name(0), "demo0000");
        assert_eq!(generated_recording_name(42), "demo0042");
        assert_eq!(generated_recording_name(12345), "demo12345");
    }

    #[test]
    fn the_first_free_name_skips_the_taken_ones() {
        let name = first_free_name(|name| matches!(name, "demo0000" | "demo0001"), 7);
        assert_eq!(name, "demo0002");
    }

    #[test]
    fn a_gap_is_filled_before_the_numbers_go_on() {
        let name = first_free_name(|name| name != "demo0001", 7);
        assert_eq!(name, "demo0001");
    }

    #[test]
    fn with_every_automatic_name_taken_the_clock_names_it() {
        assert_eq!(first_free_name(|_| true, 1_700_000_000), "demo1700000000");
    }

    #[test]
    fn the_next_free_name_follows_the_demos_on_disk() {
        let dir = TestDir::new("names");
        assert_eq!(first_unused_demo_name(dir.path()), "demo0000");
        let demos = dir.path().join("demos");
        std::fs::create_dir_all(&demos).unwrap();
        std::fs::write(demos.join("demo0000.bo2mpdemo"), b"").unwrap();
        std::fs::write(demos.join("demo0001.bo2mpdemo"), b"").unwrap();
        assert_eq!(first_unused_demo_name(dir.path()), "demo0002");
    }
}
