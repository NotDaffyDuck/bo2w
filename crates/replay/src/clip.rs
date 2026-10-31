//! Clips: the last 45 seconds of a match, kept in memory so `clip` can save
//! them after the fact, and the folders they are saved to.
//!
//! A saved clip is `<artifacts>/clips/<id>/` holding a demo of those ticks
//! ([`CLIP_RECORDING_FILE`]), a text dump of the state ([`CAPTURE_LOG_PATH`]) and a
//! small description ([`CLIP_CONTENTS_FILENAME`]). `<artifacts>/clips/LATEST`
//! points at the newest one.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use net::HOST_STEP_MILLIS;
use playerstate_t6::game::{AvatarState, InputCommand};

use crate::error::RecordingFault;
use crate::names::{recording_file, recording_stem, tidy_clip_name};

/// How much of the match a clip holds.
pub const CLIP_SPAN_MILLIS: u32 = 45_000;
/// The length of one tick; the ring counts in these.
pub const CLIP_STEP_MILLIS: u32 = 50;
pub const CLIP_TICKS: usize = (CLIP_SPAN_MILLIS / CLIP_STEP_MILLIS) as usize;

/// The name of the link to the newest clip.
pub const CLIP_NEWEST_MARKER: &str = "LATEST";

pub const CLIP_RECORDING_FILE: &str = "clip.bo2mpdemo";
/// Older spelling console still reads; the rename sweep moves it over.
pub const CAPTURE_LOG_PATH: &str = "dump.txt";
pub const CLIP_CONTENTS_FILENAME: &str = "manifest.toml";

/// What the ring may hold, by the estimate in [`ClipLoopBuffer::bytes`], however
/// short the ticks are of detail.
pub const CLIP_STORAGE_CEILING: usize = 32 * 1024 * 1024;

const _: () = assert!(CLIP_TICKS * CLIP_STEP_MILLIS as usize == CLIP_SPAN_MILLIS as usize);
const _: () = assert!(CLIP_STEP_MILLIS as i32 == HOST_STEP_MILLIS);

#[derive(Clone, Debug)]
struct HeldTick {
    fed: sim::StepInputs,
    state: sim::Snapshot,
    cost: usize,
}

/// A rough memory cost of keeping one tick: enough to keep a crowded match
/// inside the budget, not an exact count.
fn estimate_bytes(fed: &sim::StepInputs, state: &sim::Snapshot) -> usize {
    const SNAPSHOT_OVERHEAD: usize = 64;
    const PROJECTILE: usize = 64;
    const ENTITY: usize = 256;
    const ACTION: usize = 32;
    let players = state.players.len() * size_of::<AvatarState>();
    let projectiles = state.projectiles.len() * PROJECTILE;
    let entities = state.meta.entities.len() * ENTITY;
    let cmds = fed.cmds.len() * size_of::<InputCommand>();
    let actions = fed.actions.len() * ACTION;
    SNAPSHOT_OVERHEAD + players + projectiles + entities + cmds + actions
}

/// The most recent ticks of the match, oldest dropped first.
///
/// It keeps at most `max_ticks` ticks, at most `byte_budget` estimated bytes
/// and at most [`CLIP_SPAN_MILLIS`] of match time, but never fewer than one tick.
#[derive(Resource, Debug)]
pub struct ClipLoopBuffer {
    held: VecDeque<HeldTick>,
    total_bytes: usize,
    tick_cap: usize,
    byte_cap: usize,
}

impl Default for ClipLoopBuffer {
    fn default() -> Self {
        Self::bounded(CLIP_TICKS, CLIP_STORAGE_CEILING)
    }
}

impl ClipLoopBuffer {
    pub fn bounded(tick_cap: usize, byte_cap: usize) -> Self {
        Self {
            held: VecDeque::new(),
            total_bytes: 0,
            tick_cap: tick_cap.max(1),
            byte_cap,
        }
    }

    /// Adds a tick. One that is not later than the newest held is ignored, so
    /// a tick offered twice (or after a rewind) is kept once.
    pub fn record_tick(&mut self, fed: sim::StepInputs, state: sim::Snapshot) {
        if let Some(newest) = self.held.back()
            && state.tick <= newest.state.tick
        {
            return;
        }
        let cost = estimate_bytes(&fed, &state);
        self.total_bytes = self.total_bytes.saturating_add(cost);
        self.held.push_back(HeldTick { fed, state, cost });
        while self.held.len() > 1 && self.over_limits() {
            let Some(dropped) = self.held.pop_front() else {
                break;
            };
            self.total_bytes = self.total_bytes.saturating_sub(dropped.cost);
        }
    }

    fn over_limits(&self) -> bool {
        let too_many = self.held.len() > self.tick_cap;
        let too_big = self.total_bytes > self.byte_cap;
        too_many || too_big || self.span_ms() > CLIP_SPAN_MILLIS
    }

    pub fn clear(&mut self) {
        self.held.truncate(0);
        self.total_bytes = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Match time from the first held tick to the end of the last. Ticks
    /// missing in between count, as time passed all the same.
    pub fn span_ms(&self) -> u32 {
        let (Some(first), Some(last)) = (self.held.front(), self.held.back()) else {
            return 0;
        };
        let span = last.state.tick.0.saturating_sub(first.state.tick.0);
        span.saturating_add(1).saturating_mul(CLIP_STEP_MILLIS)
    }

    /// The estimated memory held; what [`CLIP_STORAGE_CEILING`] limits.
    pub fn bytes(&self) -> usize {
        self.total_bytes
    }

    /// The held ticks, oldest first.
    pub fn oldest_first(&self) -> impl Iterator<Item = (&sim::StepInputs, &sim::Snapshot)> {
        self.held.iter().map(|entry| (&entry.fed, &entry.state))
    }
}

pub fn clips_dir(root: &Path) -> PathBuf {
    root.join("clips")
}

pub fn clip_dir(root: &Path, clip_id: &str) -> PathBuf {
    clips_dir(root).join(clip_id)
}

pub fn clip_recording_file(root: &Path, clip_id: &str) -> PathBuf {
    clip_dir(root, clip_id).join(CLIP_RECORDING_FILE)
}

/// Makes the folder for clip `id`. It is an error (`AlreadyExists`) for the
/// folder to be there already, so one clip never overwrites another.
pub fn make_clip_folder(root: &Path, clip_id: &str) -> Result<PathBuf, RecordingFault> {
    fs::create_dir_all(clips_dir(root))?;
    let folder = clip_dir(root, clip_id);
    fs::create_dir(&folder)?;
    Ok(folder)
}

/// Points `clips/LATEST` at clip `id`, replacing where it pointed before.
pub fn rewrite_latest_symlink(root: &Path, clip_id: &str) -> Result<(), RecordingFault> {
    let parent = clips_dir(root);
    fs::create_dir_all(&parent)?;
    let pointer = parent.join(CLIP_NEWEST_MARKER);
    if let Err(error) = remove_link(&pointer)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    Ok(make_link(clip_id, &pointer)?)
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
fn make_link(points_to: &str, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(points_to, at)
}

#[cfg(windows)]
fn make_link(points_to: &str, at: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(points_to, at)
}

/// A TOML basic string holding `text`.
fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for letter in text.chars() {
        match letter {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other if other.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(other))),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The text of a clip's `manifest.toml`.
pub fn clip_contents_list(
    clip_id: &str,
    tick_total: u64,
    span_ms: u32,
    zone: &str,
    taken_at_ns: u128,
) -> String {
    let id = toml_string(clip_id);
    let zone = toml_string(zone);
    let demo = toml_string(CLIP_RECORDING_FILE);
    let dump = toml_string(CAPTURE_LOG_PATH);
    format!(
        "id = {id}\n\
         ticks = {tick_total}\n\
         duration_ms = {span_ms}\n\
         zone = {zone}\n\
         captured_unix_ns = {taken_at_ns}\n\
         demo = {demo}\n\
         dump = {dump}\n"
    )
}

/// The demo of the saved clip `asked` names (an id in any case, with or
/// without `.bo2mpdemo`, or `LATEST`), if there is one.
pub fn existing_clip_demo(artifacts: &Path, asked: &str) -> Option<PathBuf> {
    let wanted = tidy_clip_name(recording_stem(asked))?.to_ascii_uppercase();
    let candidate = clip_recording_file(artifacts, &wanted);
    if candidate.is_file() { Some(candidate) } else { None }
}

/// The file to play for what a player asked for: a demo of that name, else a
/// saved clip of that name. When neither exists, the demo path, so the error
/// that follows names the file that was expected.
pub fn find_demo_to_play(artifacts: &Path, asked: &str) -> PathBuf {
    let stem = recording_stem(asked);
    let cleaned = tidy_clip_name(stem).unwrap_or_else(|| stem.to_owned());
    let plain = recording_file(artifacts, &cleaned);
    if plain.exists() {
        return plain;
    }
    existing_clip_demo(artifacts, &cleaned).unwrap_or(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::{input, snapshot};

    fn push_ticks(ring: &mut ClipLoopBuffer, ticks: impl IntoIterator<Item = u32>) {
        for number in ticks {
            ring.record_tick(input(number), snapshot(number));
        }
    }

    fn held_ticks(ring: &ClipLoopBuffer) -> Vec<u32> {
        ring.oldest_first().map(|(_, snapshot)| snapshot.tick.0).collect()
    }

    /// What a tick without an action costs (tick 1 carries a join request).
    fn one_tick_bytes() -> usize {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, [2]);
        ring.bytes()
    }

    #[test]
    fn a_clip_is_45_seconds_of_900_ticks_of_50_ms() {
        assert_eq!(CLIP_SPAN_MILLIS, 45_000);
        assert_eq!(CLIP_STEP_MILLIS, 50);
        assert_eq!(CLIP_TICKS, 900);
    }

    #[test]
    fn a_new_ring_is_empty() {
        let ring = ClipLoopBuffer::default();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.bytes(), 0);
        assert_eq!(ring.span_ms(), 0);
        assert_eq!(ring.oldest_first().count(), 0);
    }

    #[test]
    fn ticks_are_kept_oldest_first() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, 1..=5);
        assert_eq!(held_ticks(&ring), [1, 2, 3, 4, 5]);
        assert_eq!(ring.len(), 5);
        assert_eq!(ring.span_ms(), 250);
    }

    #[test]
    fn the_ring_keeps_the_last_900_ticks_which_is_45_seconds() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, 1..=1000);
        assert_eq!(ring.len(), 900);
        assert_eq!(held_ticks(&ring).first(), Some(&101));
        assert_eq!(held_ticks(&ring).last(), Some(&1000));
        assert_eq!(ring.span_ms(), CLIP_SPAN_MILLIS);
    }

    #[test]
    fn a_tick_that_is_not_newer_is_ignored() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, [5, 5, 4, 6]);
        assert_eq!(held_ticks(&ring), [5, 6]);
    }

    #[test]
    fn missing_ticks_still_count_as_time() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, [1, 20]);
        assert_eq!(ring.span_ms(), 1000);
    }

    #[test]
    fn a_jump_longer_than_a_clip_drops_what_came_before() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, [1, 2, 3, 2000]);
        assert_eq!(held_ticks(&ring), [2000]);
        assert_eq!(ring.span_ms(), CLIP_STEP_MILLIS);
    }

    #[test]
    fn the_tick_limit_applies_whatever_the_time() {
        let mut ring = ClipLoopBuffer::bounded(3, usize::MAX);
        push_ticks(&mut ring, 1..=10);
        assert_eq!(held_ticks(&ring), [8, 9, 10]);
    }

    #[test]
    fn a_ring_always_has_room_for_one_tick() {
        let mut ring = ClipLoopBuffer::bounded(0, usize::MAX);
        push_ticks(&mut ring, 1..=4);
        assert_eq!(held_ticks(&ring), [4]);
    }

    #[test]
    fn bytes_add_up_and_come_off_with_the_oldest_tick() {
        let each = one_tick_bytes();
        assert!(each > 0);
        let mut ring = ClipLoopBuffer::bounded(2, usize::MAX);
        push_ticks(&mut ring, 2..=3);
        assert_eq!(ring.bytes(), 2 * each);
        push_ticks(&mut ring, [4]);
        assert_eq!(ring.bytes(), 2 * each);
        assert_eq!(held_ticks(&ring), [3, 4]);
    }

    #[test]
    fn the_byte_budget_drops_old_ticks_before_it_is_passed() {
        let each = one_tick_bytes();
        let mut ring = ClipLoopBuffer::bounded(1000, 2 * each + each / 2);
        push_ticks(&mut ring, 2..=7);
        assert_eq!(held_ticks(&ring), [6, 7]);
        assert!(ring.bytes() <= 2 * each + each / 2);
    }

    #[test]
    fn one_tick_over_the_budget_is_still_kept() {
        let mut ring = ClipLoopBuffer::bounded(10, 1);
        push_ticks(&mut ring, 1..=3);
        assert_eq!(held_ticks(&ring), [3]);
        assert!(ring.bytes() > 1);
    }

    #[test]
    fn clearing_empties_the_ring_and_lets_old_tick_numbers_in_again() {
        let mut ring = ClipLoopBuffer::default();
        push_ticks(&mut ring, 100..=110);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.bytes(), 0);
        push_ticks(&mut ring, [1]);
        assert_eq!(held_ticks(&ring), [1]);
    }

    #[test]
    fn what_goes_in_comes_out() {
        let mut ring = ClipLoopBuffer::default();
        ring.record_tick(input(1), snapshot(1));
        let (held_input, retained_world_state) = ring.oldest_first().next().unwrap();
        assert_eq!(held_input, &input(1));
        assert_eq!(retained_world_state.tick, sim::Tick(1));
    }

    #[test]
    fn clip_folders_sit_under_clips() {
        let root = Path::new("art");
        assert_eq!(clips_dir(root), root.join("clips"));
        assert_eq!(clip_dir(root, "ID"), root.join("clips").join("ID"));
        assert_eq!(
            clip_recording_file(root, "ID"),
            root.join("clips").join("ID").join("clip.bo2mpdemo")
        );
    }

    #[test]
    fn making_a_clip_folder_twice_is_an_error_the_second_time() {
        let dir = TestDir::new("clipdir");
        let made = make_clip_folder(dir.path(), "01ABC").unwrap();
        assert!(made.is_dir());
        assert_eq!(made, clip_dir(dir.path(), "01ABC"));
        let again = make_clip_folder(dir.path(), "01ABC").unwrap_err();
        assert!(matches!(again, RecordingFault::Io(e) if e.kind() == io::ErrorKind::AlreadyExists));
        assert!(make_clip_folder(dir.path(), "01ABD").is_ok());
    }

    #[test]
    fn the_manifest_lists_what_the_clip_holds() {
        let text = clip_contents_list(
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
        let text = clip_contents_list("id", 1, 50, "a\"b\\c\nd", 0);
        assert!(text.contains("zone = \"a\\\"b\\\\c\\u000Ad\"\n"), "{text}");
    }

    /// Where the OS will not let this account make links, the link tests
    /// have nothing to check.
    fn cannot_link(error: &RecordingFault) -> bool {
        matches!(
            error,
            RecordingFault::Io(e)
                if e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(1314)
        )
    }

    #[test]
    fn latest_follows_the_newest_clip() {
        let dir = TestDir::new("latest");
        let first = make_clip_folder(dir.path(), "AAA").unwrap();
        let second = make_clip_folder(dir.path(), "BBB").unwrap();
        fs::write(first.join(CLIP_RECORDING_FILE), b"a").unwrap();
        fs::write(second.join(CLIP_RECORDING_FILE), b"b").unwrap();
        let latest = clips_dir(dir.path()).join(CLIP_NEWEST_MARKER);

        if let Err(error) = rewrite_latest_symlink(dir.path(), "AAA") {
            assert!(cannot_link(&error), "{error}");
            return;
        }
        assert_eq!(fs::read_link(&latest).unwrap(), Path::new("AAA"));
        assert_eq!(fs::read(latest.join(CLIP_RECORDING_FILE)).unwrap(), b"a");

        rewrite_latest_symlink(dir.path(), "BBB").unwrap();
        assert_eq!(fs::read_link(&latest).unwrap(), Path::new("BBB"));
        assert_eq!(fs::read(latest.join(CLIP_RECORDING_FILE)).unwrap(), b"b");
        assert_eq!(
            existing_clip_demo(dir.path(), "latest"),
            Some(clip_recording_file(dir.path(), CLIP_NEWEST_MARKER))
        );
    }

    fn saved_clip(under: &Path, named: &str) -> PathBuf {
        let dir = make_clip_folder(under, named).unwrap();
        let demo = dir.join(CLIP_RECORDING_FILE);
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
        make_clip_folder(dir.path(), "HOLLOW").unwrap();
        assert_eq!(existing_clip_demo(dir.path(), "hollow"), None);
    }

    #[test]
    fn a_request_that_is_not_a_name_finds_no_clip() {
        let dir = TestDir::new("traverse");
        saved_clip(dir.path(), "REAL");
        fs::write(dir.path().join(CLIP_RECORDING_FILE), b"outside").unwrap();
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
            find_demo_to_play(dir.path(), "SHARED"),
            recording_file(dir.path(), "SHARED")
        );
    }

    #[test]
    fn with_no_demo_by_that_name_a_clip_is_played() {
        let dir = TestDir::new("fallback");
        let demo = saved_clip(dir.path(), "ONLYCLIP");
        assert_eq!(find_demo_to_play(dir.path(), "onlyclip"), demo);
        assert_eq!(
            find_demo_to_play(dir.path(), "ONLYCLIP.bo2mpdemo"),
            demo
        );
    }

    #[test]
    fn with_neither_the_demo_path_is_returned_for_the_error_to_name() {
        let dir = TestDir::new("neither");
        assert_eq!(
            find_demo_to_play(dir.path(), "nothing here"),
            recording_file(dir.path(), "nothinghere")
        );
    }

    #[test]
    fn a_request_with_no_usable_name_cannot_leave_the_demos_folder() {
        let dir = TestDir::new("escape");
        let path = find_demo_to_play(dir.path(), "../..");
        assert_eq!(path.parent(), Some(dir.path().join("demos").as_path()));
    }

    #[test]
    fn folders_in_a_request_are_dropped() {
        let dir = TestDir::new("folders");
        assert_eq!(
            find_demo_to_play(dir.path(), "C:\\somewhere\\else\\mine.bo2mpdemo"),
            recording_file(dir.path(), "mine")
        );
    }
}
