//! Clips: the last 45 seconds of a match, kept in memory so `clip` can save
//! them after the fact, and the folders they are saved to.
//!
//! A saved clip is `<artifacts>/clips/<id>/` holding a demo of those ticks
//! ([`CLIP_DEMO_FILE`]), a text dump of the state ([`CLIP_DUMP_FILE`]) and a
//! small description ([`CLIP_MANIFEST_FILE`]). `<artifacts>/clips/LATEST`
//! points at the newest one.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use net::AUTHORITY_MS;
use playerstate_t6::game::{PlayerState, UserCmd};
use sim::{Snapshot, TickInput};

use crate::error::ReplayError;
use crate::names::{demo_path, demo_stem, sanitize_demo_name};

/// How much of the match a clip holds.
pub const CLIP_MS: u32 = 45_000;
/// The length of one tick; the ring counts in these.
pub const CLIP_TICK_MS: u32 = 50;
pub const CLIP_TICKS: usize = (CLIP_MS / CLIP_TICK_MS) as usize;

/// The name of the link to the newest clip.
pub const CLIP_LATEST: &str = "LATEST";

pub const CLIP_DEMO_FILE: &str = "clip.bo2mpdemo";
pub const CLIP_DUMP_FILE: &str = "dump.txt";
pub const CLIP_MANIFEST_FILE: &str = "manifest.toml";

/// What the ring may hold, by the estimate in [`ClipRing::bytes`], however
/// short the ticks are of detail.
pub const CLIP_BYTE_BUDGET: usize = 32 * 1024 * 1024;

const _: () = assert!(CLIP_TICKS * CLIP_TICK_MS as usize == CLIP_MS as usize);
const _: () = assert!(CLIP_TICK_MS as i32 == AUTHORITY_MS);

#[derive(Clone, Debug)]
struct HeldTick {
    input: TickInput,
    snapshot: Snapshot,
    bytes: usize,
}

/// A rough memory cost of keeping one tick: enough to keep a crowded match
/// inside the budget, not an exact count.
fn estimate_bytes(input: &TickInput, snapshot: &Snapshot) -> usize {
    const SNAPSHOT_OVERHEAD: usize = 64;
    const PROJECTILE: usize = 64;
    const ENTITY: usize = 256;
    const ACTION: usize = 32;
    SNAPSHOT_OVERHEAD
        + snapshot.players.len() * size_of::<PlayerState>()
        + snapshot.projectiles.len() * PROJECTILE
        + snapshot.meta.entities.len() * ENTITY
        + input.cmds.len() * size_of::<UserCmd>()
        + input.actions.len() * ACTION
}

/// The most recent ticks of the match, oldest dropped first.
///
/// It keeps at most `max_ticks` ticks, at most `byte_budget` estimated bytes
/// and at most [`CLIP_MS`] of match time, but never fewer than one tick.
#[derive(Resource, Debug)]
pub struct ClipRing {
    held: VecDeque<HeldTick>,
    bytes: usize,
    max_ticks: usize,
    byte_budget: usize,
}

impl Default for ClipRing {
    fn default() -> Self {
        Self::with_limits(CLIP_TICKS, CLIP_BYTE_BUDGET)
    }
}

impl ClipRing {
    pub fn with_limits(max_ticks: usize, byte_budget: usize) -> Self {
        Self {
            held: VecDeque::new(),
            bytes: 0,
            max_ticks: max_ticks.max(1),
            byte_budget,
        }
    }

    /// Adds a tick. One that is not later than the newest held is ignored, so
    /// a tick offered twice (or after a rewind) is kept once.
    pub fn push(&mut self, input: TickInput, snapshot: Snapshot) {
        let is_new = self
            .held
            .back()
            .is_none_or(|newest| snapshot.tick > newest.snapshot.tick);
        if !is_new {
            return;
        }
        let bytes = estimate_bytes(&input, &snapshot);
        self.bytes = self.bytes.saturating_add(bytes);
        self.held.push_back(HeldTick {
            input,
            snapshot,
            bytes,
        });
        while self.held.len() > 1 && self.over_limits() {
            if let Some(oldest) = self.held.pop_front() {
                self.bytes = self.bytes.saturating_sub(oldest.bytes);
            }
        }
    }

    fn over_limits(&self) -> bool {
        self.held.len() > self.max_ticks
            || self.bytes > self.byte_budget
            || self.duration_ms() > CLIP_MS
    }

    pub fn clear(&mut self) {
        self.held.clear();
        self.bytes = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Match time from the first held tick to the end of the last. Ticks
    /// missing in between count, as time passed all the same.
    pub fn duration_ms(&self) -> u32 {
        match (self.held.front(), self.held.back()) {
            (Some(first), Some(last)) => last
                .snapshot
                .tick
                .0
                .saturating_sub(first.snapshot.tick.0)
                .saturating_add(1)
                .saturating_mul(CLIP_TICK_MS),
            _ => 0,
        }
    }

    /// The estimated memory held; what [`CLIP_BYTE_BUDGET`] limits.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The held ticks, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = (&TickInput, &Snapshot)> {
        self.held.iter().map(|held| (&held.input, &held.snapshot))
    }
}

pub fn clips_dir(artifacts_root: &Path) -> PathBuf {
    artifacts_root.join("clips")
}

pub fn clip_dir(artifacts_root: &Path, id: &str) -> PathBuf {
    clips_dir(artifacts_root).join(id)
}

pub fn clip_demo_path(artifacts_root: &Path, id: &str) -> PathBuf {
    clip_dir(artifacts_root, id).join(CLIP_DEMO_FILE)
}

/// Makes the folder for clip `id`. It is an error (`AlreadyExists`) for the
/// folder to be there already, so one clip never overwrites another.
pub fn create_clip_dir(artifacts_root: &Path, id: &str) -> Result<PathBuf, ReplayError> {
    fs::create_dir_all(clips_dir(artifacts_root))?;
    let dir = clip_dir(artifacts_root, id);
    fs::create_dir(&dir)?;
    Ok(dir)
}

/// Points `clips/LATEST` at clip `id`, replacing where it pointed before.
pub fn rewrite_latest_symlink(artifacts_root: &Path, id: &str) -> Result<(), ReplayError> {
    let clips = clips_dir(artifacts_root);
    fs::create_dir_all(&clips)?;
    let latest = clips.join(CLIP_LATEST);
    match remove_link(&latest) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    Ok(make_link(id, &latest)?)
}

#[cfg(unix)]
fn remove_link(link: &Path) -> io::Result<()> {
    fs::remove_file(link)
}

#[cfg(windows)]
fn remove_link(link: &Path) -> io::Result<()> {
    fs::remove_dir(link)
}

#[cfg(unix)]
fn make_link(target: &str, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_link(target: &str, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// A TOML basic string holding `text`.
fn toml_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            c if c.is_control() => quoted.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// The text of a clip's `manifest.toml`.
pub fn clip_manifest(
    id: &str,
    ticks: u64,
    duration_ms: u32,
    zone: &str,
    captured_unix_ns: u128,
) -> String {
    format!(
        "id = {id}\n\
         ticks = {ticks}\n\
         duration_ms = {duration_ms}\n\
         zone = {zone}\n\
         captured_unix_ns = {captured_unix_ns}\n\
         demo = {demo}\n\
         dump = {dump}\n",
        id = toml_string(id),
        zone = toml_string(zone),
        demo = toml_string(CLIP_DEMO_FILE),
        dump = toml_string(CLIP_DUMP_FILE),
    )
}

/// The demo of the saved clip `requested` names (an id in any case, with or
/// without `.bo2mpdemo`, or `LATEST`), if there is one.
pub fn existing_clip_demo(artifacts_root: &Path, requested: &str) -> Option<PathBuf> {
    let id = sanitize_demo_name(demo_stem(requested))?.to_ascii_uppercase();
    let demo = clip_demo_path(artifacts_root, &id);
    demo.is_file().then_some(demo)
}

/// The file to play for what a player asked for: a demo of that name, else a
/// saved clip of that name. When neither exists, the demo path, so the error
/// that follows names the file that was expected.
pub fn resolve_playback_path(artifacts_root: &Path, requested: &str) -> PathBuf {
    let stem = demo_stem(requested);
    let name = sanitize_demo_name(stem).unwrap_or_else(|| stem.to_owned());
    let demo = demo_path(artifacts_root, &name);
    if demo.exists() {
        return demo;
    }
    existing_clip_demo(artifacts_root, &name).unwrap_or(demo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::{input, snapshot};
    use sim::Tick;

    fn push_ticks(ring: &mut ClipRing, ticks: impl IntoIterator<Item = u32>) {
        for tick in ticks {
            ring.push(input(tick), snapshot(tick));
        }
    }

    fn held_ticks(ring: &ClipRing) -> Vec<u32> {
        ring.iter().map(|(_, snapshot)| snapshot.tick.0).collect()
    }

    /// What a tick without an action costs (tick 1 carries a join request).
    fn one_tick_bytes() -> usize {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, [2]);
        ring.bytes()
    }

    #[test]
    fn a_clip_is_45_seconds_of_900_ticks_of_50_ms() {
        assert_eq!(CLIP_MS, 45_000);
        assert_eq!(CLIP_TICK_MS, 50);
        assert_eq!(CLIP_TICKS, 900);
    }

    #[test]
    fn a_new_ring_is_empty() {
        let ring = ClipRing::default();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.bytes(), 0);
        assert_eq!(ring.duration_ms(), 0);
        assert_eq!(ring.iter().count(), 0);
    }

    #[test]
    fn ticks_are_kept_oldest_first() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, 1..=5);
        assert_eq!(held_ticks(&ring), [1, 2, 3, 4, 5]);
        assert_eq!(ring.len(), 5);
        assert_eq!(ring.duration_ms(), 250);
    }

    #[test]
    fn the_ring_keeps_the_last_900_ticks_which_is_45_seconds() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, 1..=1000);
        assert_eq!(ring.len(), 900);
        assert_eq!(held_ticks(&ring).first(), Some(&101));
        assert_eq!(held_ticks(&ring).last(), Some(&1000));
        assert_eq!(ring.duration_ms(), CLIP_MS);
    }

    #[test]
    fn a_tick_that_is_not_newer_is_ignored() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, [5, 5, 4, 6]);
        assert_eq!(held_ticks(&ring), [5, 6]);
    }

    #[test]
    fn missing_ticks_still_count_as_time() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, [1, 20]);
        assert_eq!(ring.duration_ms(), 1000);
    }

    #[test]
    fn a_jump_longer_than_a_clip_drops_what_came_before() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, [1, 2, 3, 2000]);
        assert_eq!(held_ticks(&ring), [2000]);
        assert_eq!(ring.duration_ms(), CLIP_TICK_MS);
    }

    #[test]
    fn the_tick_limit_applies_whatever_the_time() {
        let mut ring = ClipRing::with_limits(3, usize::MAX);
        push_ticks(&mut ring, 1..=10);
        assert_eq!(held_ticks(&ring), [8, 9, 10]);
    }

    #[test]
    fn a_ring_always_has_room_for_one_tick() {
        let mut ring = ClipRing::with_limits(0, usize::MAX);
        push_ticks(&mut ring, 1..=4);
        assert_eq!(held_ticks(&ring), [4]);
    }

    #[test]
    fn bytes_add_up_and_come_off_with_the_oldest_tick() {
        let each = one_tick_bytes();
        assert!(each > 0);
        let mut ring = ClipRing::with_limits(2, usize::MAX);
        push_ticks(&mut ring, 2..=3);
        assert_eq!(ring.bytes(), 2 * each);
        push_ticks(&mut ring, [4]);
        assert_eq!(ring.bytes(), 2 * each);
        assert_eq!(held_ticks(&ring), [3, 4]);
    }

    #[test]
    fn the_byte_budget_drops_old_ticks_before_it_is_passed() {
        let each = one_tick_bytes();
        let mut ring = ClipRing::with_limits(1000, 2 * each + each / 2);
        push_ticks(&mut ring, 2..=7);
        assert_eq!(held_ticks(&ring), [6, 7]);
        assert!(ring.bytes() <= 2 * each + each / 2);
    }

    #[test]
    fn one_tick_over_the_budget_is_still_kept() {
        let mut ring = ClipRing::with_limits(10, 1);
        push_ticks(&mut ring, 1..=3);
        assert_eq!(held_ticks(&ring), [3]);
        assert!(ring.bytes() > 1);
    }

    #[test]
    fn clearing_empties_the_ring_and_lets_old_tick_numbers_in_again() {
        let mut ring = ClipRing::default();
        push_ticks(&mut ring, 100..=110);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.bytes(), 0);
        push_ticks(&mut ring, [1]);
        assert_eq!(held_ticks(&ring), [1]);
    }

    #[test]
    fn what_goes_in_comes_out() {
        let mut ring = ClipRing::default();
        ring.push(input(1), snapshot(1));
        let (held_input, held_snapshot) = ring.iter().next().unwrap();
        assert_eq!(held_input, &input(1));
        assert_eq!(held_snapshot.tick, Tick(1));
    }

    #[test]
    fn clip_folders_sit_under_clips() {
        let root = Path::new("art");
        assert_eq!(clips_dir(root), root.join("clips"));
        assert_eq!(clip_dir(root, "ID"), root.join("clips").join("ID"));
        assert_eq!(
            clip_demo_path(root, "ID"),
            root.join("clips").join("ID").join("clip.bo2mpdemo")
        );
    }

    #[test]
    fn making_a_clip_folder_twice_is_an_error_the_second_time() {
        let dir = TestDir::new("clipdir");
        let made = create_clip_dir(dir.path(), "01ABC").unwrap();
        assert!(made.is_dir());
        assert_eq!(made, clip_dir(dir.path(), "01ABC"));
        let again = create_clip_dir(dir.path(), "01ABC").unwrap_err();
        assert!(matches!(again, ReplayError::Io(e) if e.kind() == io::ErrorKind::AlreadyExists));
        assert!(create_clip_dir(dir.path(), "01ABD").is_ok());
    }

    #[test]
    fn the_manifest_lists_what_the_clip_holds() {
        let text = clip_manifest(
            "01ABC",
            900,
            45_000,
            "zm_transit",
            1_700_000_000_123_456_789,
        );
        assert_eq!(
            text,
            "id = \"01ABC\"\n\
             ticks = 900\n\
             duration_ms = 45000\n\
             zone = \"zm_transit\"\n\
             captured_unix_ns = 1700000000123456789\n\
             demo = \"clip.bo2mpdemo\"\n\
             dump = \"dump.txt\"\n"
        );
    }

    #[test]
    fn the_manifest_quotes_text_that_would_break_a_string() {
        let text = clip_manifest("id", 1, 50, "a\"b\\c\nd", 0);
        assert!(text.contains("zone = \"a\\\"b\\\\c\\u000Ad\"\n"), "{text}");
    }

    /// Where the OS will not let this account make links, the link tests
    /// have nothing to check.
    fn cannot_link(error: &ReplayError) -> bool {
        matches!(
            error,
            ReplayError::Io(e)
                if e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(1314)
        )
    }

    #[test]
    fn latest_follows_the_newest_clip() {
        let dir = TestDir::new("latest");
        let first = create_clip_dir(dir.path(), "AAA").unwrap();
        let second = create_clip_dir(dir.path(), "BBB").unwrap();
        fs::write(first.join(CLIP_DEMO_FILE), b"a").unwrap();
        fs::write(second.join(CLIP_DEMO_FILE), b"b").unwrap();
        let latest = clips_dir(dir.path()).join(CLIP_LATEST);

        if let Err(error) = rewrite_latest_symlink(dir.path(), "AAA") {
            assert!(cannot_link(&error), "{error}");
            return;
        }
        assert_eq!(fs::read_link(&latest).unwrap(), Path::new("AAA"));
        assert_eq!(fs::read(latest.join(CLIP_DEMO_FILE)).unwrap(), b"a");

        rewrite_latest_symlink(dir.path(), "BBB").unwrap();
        assert_eq!(fs::read_link(&latest).unwrap(), Path::new("BBB"));
        assert_eq!(fs::read(latest.join(CLIP_DEMO_FILE)).unwrap(), b"b");
        assert_eq!(
            existing_clip_demo(dir.path(), "latest"),
            Some(clip_demo_path(dir.path(), CLIP_LATEST))
        );
    }

    fn saved_clip(root: &Path, id: &str) -> PathBuf {
        let dir = create_clip_dir(root, id).unwrap();
        let demo = dir.join(CLIP_DEMO_FILE);
        fs::write(&demo, b"clip").unwrap();
        demo
    }

    #[test]
    fn a_saved_clip_is_found_by_its_id_in_any_case_with_or_without_the_extension() {
        let dir = TestDir::new("find");
        let demo = saved_clip(dir.path(), "01ARYZ6S41");
        for asked in [
            "01ARYZ6S41",
            "01aryz6s41",
            "01aryz6s41.bo2mpdemo",
            "clips/01aryz6s41",
        ] {
            assert_eq!(
                existing_clip_demo(dir.path(), asked),
                Some(demo.clone()),
                "{asked}"
            );
        }
        assert_eq!(existing_clip_demo(dir.path(), "01ARYZ6S42"), None);
    }

    #[test]
    fn a_clip_folder_without_a_demo_in_it_is_not_a_clip() {
        let dir = TestDir::new("hollow");
        create_clip_dir(dir.path(), "HOLLOW").unwrap();
        assert_eq!(existing_clip_demo(dir.path(), "hollow"), None);
    }

    #[test]
    fn a_request_that_is_not_a_name_finds_no_clip() {
        let dir = TestDir::new("traverse");
        saved_clip(dir.path(), "REAL");
        fs::write(dir.path().join(CLIP_DEMO_FILE), b"outside").unwrap();
        for asked in ["", "..", "../..", "...", "!!!", "./", ".\\.."] {
            assert_eq!(existing_clip_demo(dir.path(), asked), None, "{asked:?}");
        }
    }

    #[test]
    fn a_demo_by_that_name_is_played_before_a_clip() {
        let dir = TestDir::new("prefer");
        saved_clip(dir.path(), "SHARED");
        let demos = dir.path().join("demos");
        fs::create_dir_all(&demos).unwrap();
        fs::write(demos.join("SHARED.bo2mpdemo"), b"demo").unwrap();
        assert_eq!(
            resolve_playback_path(dir.path(), "SHARED"),
            demo_path(dir.path(), "SHARED")
        );
    }

    #[test]
    fn with_no_demo_by_that_name_a_clip_is_played() {
        let dir = TestDir::new("fallback");
        let demo = saved_clip(dir.path(), "ONLYCLIP");
        assert_eq!(resolve_playback_path(dir.path(), "onlyclip"), demo);
        assert_eq!(
            resolve_playback_path(dir.path(), "ONLYCLIP.bo2mpdemo"),
            demo
        );
    }

    #[test]
    fn with_neither_the_demo_path_is_returned_for_the_error_to_name() {
        let dir = TestDir::new("neither");
        assert_eq!(
            resolve_playback_path(dir.path(), "nothing here"),
            demo_path(dir.path(), "nothinghere")
        );
    }

    #[test]
    fn a_request_with_no_usable_name_cannot_leave_the_demos_folder() {
        let dir = TestDir::new("escape");
        let path = resolve_playback_path(dir.path(), "../..");
        assert_eq!(path.parent(), Some(dir.path().join("demos").as_path()));
    }

    #[test]
    fn folders_in_a_request_are_dropped() {
        let dir = TestDir::new("folders");
        assert_eq!(
            resolve_playback_path(dir.path(), "C:\\somewhere\\else\\mine.bo2mpdemo"),
            demo_path(dir.path(), "mine")
        );
    }
}
