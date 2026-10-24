//! Recording a match tick by tick, and playing a recording back tick by tick.
//!
//! Both sides use the frames the server sends its clients, so what plays back
//! is exactly what a client would have been shown.

use std::path::{Path, PathBuf};

use net::{Frame, SnapshotDecoder, SnapshotEncoder, frame_from_tick};
use playerstate_t6::game::UserCmd;
use sim::{ClientAction, ClientId, Snapshot, TickInput};

use crate::clip::{ClipRing, resolve_playback_path};
use crate::error::ReplayError;
use crate::file::{RecordReader, RecordWriter};
use crate::identity::MatchRecordIdentity;
use crate::names::{demo_path, next_free_demo_name, sanitize_demo_name};

/// A recording being written. Each tick given to [`Recording::record`] goes
/// to the file at once (buffered), so a crash loses only the last moments.
#[derive(Debug)]
pub struct Recording {
    writer: RecordWriter,
    encoder: SnapshotEncoder,
    name: String,
    ticks: u64,
}

impl Recording {
    /// Starts `<artifacts>/demos/<requested>.bo2mpdemo`, with no match
    /// identity in its header.
    pub fn start(artifacts_root: &Path, requested: &str) -> Result<Self, ReplayError> {
        Self::start_with_identity(artifacts_root, requested, MatchRecordIdentity::default())
    }

    /// As [`Recording::start`]. `requested` is cut down to a safe file name;
    /// when nothing is left of it, the next free `demoNNNN` is used.
    pub fn start_with_identity(
        artifacts_root: &Path,
        requested: &str,
        identity: MatchRecordIdentity,
    ) -> Result<Self, ReplayError> {
        let name =
            sanitize_demo_name(requested).unwrap_or_else(|| next_free_demo_name(artifacts_root));
        let path = demo_path(artifacts_root, &name);
        Self::begin(path, name, identity)
    }

    /// Starts a recording at exactly `path`. It is named after the folder it
    /// is in (a clip's folder is its id), or `clip` where there is none.
    pub fn start_at_path(
        path: PathBuf,
        identity: MatchRecordIdentity,
    ) -> Result<Self, ReplayError> {
        let name = path.parent().and_then(Path::file_name).map_or_else(
            || "clip".to_owned(),
            |folder| folder.to_string_lossy().into_owned(),
        );
        Self::begin(path, name, identity)
    }

    fn begin(
        path: PathBuf,
        name: String,
        identity: MatchRecordIdentity,
    ) -> Result<Self, ReplayError> {
        Ok(Self {
            writer: RecordWriter::create_with_identity(&path, identity)?,
            encoder: SnapshotEncoder::new(),
            name,
            ticks: 0,
        })
    }

    /// Adds one tick: the input the server ran on and the snapshot it made.
    /// Ticks must be given in order.
    pub fn record(&mut self, input: &TickInput, snapshot: &Snapshot) -> Result<(), ReplayError> {
        let frame = frame_from_tick(&mut self.encoder, input, snapshot);
        self.writer.write_frame(&frame)?;
        self.ticks += 1;
        Ok(())
    }

    /// Adds every tick the ring holds, oldest first; how many that was.
    pub fn record_clip_ring(&mut self, ring: &ClipRing) -> Result<u64, ReplayError> {
        let mut added = 0;
        for (input, snapshot) in ring.iter() {
            self.record(input, snapshot)?;
            added += 1;
        }
        Ok(added)
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        self.writer.path()
    }

    /// Ends the recording: the ticks it holds and the file they are in.
    pub fn stop(self) -> Result<(u64, PathBuf), ReplayError> {
        self.writer.finish()
    }
}

/// One tick of a recording, as the client would have received it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayedTick {
    pub cmds: Vec<(ClientId, UserCmd)>,
    pub actions: Vec<(ClientId, ClientAction)>,
    pub snapshot: Snapshot,
    /// The frame it came from, for everything a snapshot does not carry.
    pub frame: Frame,
}

/// A recording being read back.
#[derive(Debug)]
pub struct Playback {
    reader: RecordReader,
    decoder: SnapshotDecoder,
    ticks: u64,
}

impl Playback {
    /// Opens what `requested` names: a demo of that name, else a saved clip.
    pub fn open(artifacts_root: &Path, requested: &str) -> Result<Self, ReplayError> {
        Self::open_path(&resolve_playback_path(artifacts_root, requested))
    }

    pub fn open_path(path: &Path) -> Result<Self, ReplayError> {
        Ok(Self {
            reader: RecordReader::open(path)?,
            decoder: SnapshotDecoder::new(),
            ticks: 0,
        })
    }

    /// The next tick, or `None` once the recording has ended.
    pub fn next_tick(&mut self) -> Result<Option<PlayedTick>, ReplayError> {
        let Some(frame) = self.reader.read_frame()? else {
            return Ok(None);
        };
        let mut snapshot = self.decoder.decode(&frame.snapshot_delta)?;
        snapshot.meta = frame.snapshot_meta.clone();
        self.ticks += 1;
        Ok(Some(PlayedTick {
            cmds: frame.cmds.clone(),
            actions: frame.actions.clone(),
            snapshot,
            frame,
        }))
    }

    /// How many ticks have been played so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn path(&self) -> &Path {
        self.reader.path()
    }

    /// What the recording's header says about the match it came from.
    pub fn identity(&self) -> MatchRecordIdentity {
        self.reader.identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{CLIP_DEMO_FILE, clip_demo_path, create_clip_dir};
    use crate::testdir::TestDir;
    use crate::testing::{PLAYER, input, snapshot};
    use sim::Tick;
    use std::fs;
    use std::io::{self, Write};

    fn record_ticks(recording: &mut Recording, ticks: impl IntoIterator<Item = u32>) {
        for tick in ticks {
            recording.record(&input(tick), &snapshot(tick)).unwrap();
        }
    }

    fn play_all(playback: &mut Playback) -> Vec<PlayedTick> {
        std::iter::from_fn(|| playback.next_tick().unwrap()).collect()
    }

    fn record_demo(root: &Path, name: &str, ticks: u32) -> PathBuf {
        let mut recording = Recording::start(root, name).unwrap();
        record_ticks(&mut recording, 1..=ticks);
        recording.stop().unwrap().1
    }

    #[test]
    fn what_is_recorded_plays_back_tick_for_tick() {
        let dir = TestDir::new("roundtrip");
        record_demo(dir.path(), "round", 5);

        let mut playback = Playback::open(dir.path(), "round").unwrap();
        let played = play_all(&mut playback);

        assert_eq!(played.len(), 5);
        for (index, tick) in played.iter().enumerate() {
            let number = index as u32 + 1;
            assert_eq!(tick.snapshot.tick, Tick(number));
            assert_eq!(tick.cmds, input(number).cmds);
            assert_eq!(tick.actions, input(number).actions);
            let (id, state) = &tick.snapshot.players[0];
            assert_eq!(*id, PLAYER);
            assert_eq!(state.origin, [number as f32, 2.0, 3.0]);
            assert_eq!(state.command_time, number as i32 * 50);
        }
        assert_eq!(playback.ticks(), 5);
    }

    #[test]
    fn the_join_request_comes_back_on_the_tick_it_was_made() {
        let dir = TestDir::new("actions");
        record_demo(dir.path(), "actions", 3);
        let played = play_all(&mut Playback::open(dir.path(), "actions").unwrap());
        assert_eq!(played[0].actions.len(), 1);
        assert!(played[1].actions.is_empty());
        assert!(played[2].actions.is_empty());
    }

    #[test]
    fn the_frame_a_tick_came_from_is_kept_whole() {
        let dir = TestDir::new("frame");
        record_demo(dir.path(), "frame", 2);
        let played = play_all(&mut Playback::open(dir.path(), "frame").unwrap());
        assert_eq!(played[1].frame.tick, Tick(2));
        assert_eq!(played[1].frame.cmds, played[1].cmds);
        assert_eq!(played[1].frame.snapshot_meta, played[1].snapshot.meta);
    }

    #[test]
    fn a_recording_that_ended_keeps_ending() {
        let dir = TestDir::new("ended");
        record_demo(dir.path(), "ended", 1);
        let mut playback = Playback::open(dir.path(), "ended").unwrap();
        assert!(playback.next_tick().unwrap().is_some());
        for _ in 0..3 {
            assert!(playback.next_tick().unwrap().is_none());
        }
        assert_eq!(playback.ticks(), 1);
    }

    #[test]
    fn an_empty_recording_has_no_ticks() {
        let dir = TestDir::new("empty");
        let path = record_demo(dir.path(), "empty", 0);
        let mut playback = Playback::open_path(&path).unwrap();
        assert!(playback.next_tick().unwrap().is_none());
        assert_eq!(playback.ticks(), 0);
    }

    #[test]
    fn a_recording_stops_with_its_tick_count_and_its_file() {
        let dir = TestDir::new("stop");
        let mut recording = Recording::start(dir.path(), "stopped").unwrap();
        assert_eq!(recording.ticks(), 0);
        record_ticks(&mut recording, 1..=4);
        assert_eq!(recording.ticks(), 4);
        let path = recording.path().to_path_buf();
        let (ticks, written) = recording.stop().unwrap();
        assert_eq!(ticks, 4);
        assert_eq!(written, path);
        assert_eq!(written, demo_path(dir.path(), "stopped"));
        assert!(written.is_file());
    }

    #[test]
    fn a_requested_name_is_cut_down_to_a_safe_one() {
        let dir = TestDir::new("sanitize");
        let recording = Recording::start(dir.path(), "../my demo!").unwrap();
        assert_eq!(recording.name(), "mydemo");
        assert_eq!(recording.path(), demo_path(dir.path(), "mydemo"));
    }

    #[test]
    fn an_unnamed_recording_takes_the_next_free_automatic_name() {
        let dir = TestDir::new("auto");
        let first = Recording::start(dir.path(), "").unwrap();
        assert_eq!(first.name(), "demo0000");
        let second = Recording::start(dir.path(), "!!!").unwrap();
        assert_eq!(second.name(), "demo0001");
        assert_eq!(second.path(), demo_path(dir.path(), "demo0001"));
    }

    #[test]
    fn the_match_identity_is_in_the_header() {
        let dir = TestDir::new("identity");
        let mut identity = MatchRecordIdentity {
            rng_scheme: 1,
            root_seed: 99,
            content_digest: 1234,
            ..MatchRecordIdentity::default()
        };
        identity.set_zone("zm_transit");
        Recording::start_with_identity(dir.path(), "ident", identity)
            .unwrap()
            .stop()
            .unwrap();

        let playback = Playback::open(dir.path(), "ident").unwrap();
        assert_eq!(playback.identity(), identity);
        assert_eq!(playback.identity().zone_name(), Some("zm_transit"));
    }

    #[test]
    fn a_recording_started_at_a_path_is_named_for_its_folder() {
        let dir = TestDir::new("atpath");
        let folder = create_clip_dir(dir.path(), "01CLIP").unwrap();
        let recording =
            Recording::start_at_path(folder.join(CLIP_DEMO_FILE), MatchRecordIdentity::default())
                .unwrap();
        assert_eq!(recording.name(), "01CLIP");
        assert_eq!(recording.path(), folder.join(CLIP_DEMO_FILE));
    }

    #[test]
    fn a_recording_started_with_no_folder_is_called_clip() {
        let recording = Recording::start_at_path(
            PathBuf::from("bo2mp-replay-nofolder-test.bo2mpdemo"),
            MatchRecordIdentity::default(),
        )
        .unwrap();
        let path = recording.path().to_path_buf();
        assert_eq!(recording.name(), "clip");
        recording.stop().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_started_recording_makes_the_folders_it_needs() {
        let dir = TestDir::new("folders");
        let path = dir.path().join("deep").join("er").join("x.bo2mpdemo");
        Recording::start_at_path(path.clone(), MatchRecordIdentity::default())
            .unwrap()
            .stop()
            .unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn a_clip_ring_is_recorded_oldest_first() {
        let dir = TestDir::new("ring");
        let mut ring = ClipRing::default();
        for tick in 10..=14 {
            ring.push(input(tick), snapshot(tick));
        }
        let mut recording = Recording::start(dir.path(), "fromring").unwrap();
        assert_eq!(recording.record_clip_ring(&ring).unwrap(), 5);
        assert_eq!(recording.ticks(), 5);
        let path = recording.stop().unwrap().1;

        let played = play_all(&mut Playback::open_path(&path).unwrap());
        let ticks: Vec<u32> = played.iter().map(|tick| tick.snapshot.tick.0).collect();
        assert_eq!(ticks, [10, 11, 12, 13, 14]);
    }

    #[test]
    fn an_empty_clip_ring_records_nothing() {
        let dir = TestDir::new("emptyring");
        let mut recording = Recording::start(dir.path(), "none").unwrap();
        assert_eq!(recording.record_clip_ring(&ClipRing::default()).unwrap(), 0);
        assert_eq!(recording.ticks(), 0);
    }

    #[test]
    fn a_saved_clip_opens_by_its_id() {
        let dir = TestDir::new("openclip");
        let folder = create_clip_dir(dir.path(), "01ABC").unwrap();
        let mut recording =
            Recording::start_at_path(folder.join(CLIP_DEMO_FILE), MatchRecordIdentity::default())
                .unwrap();
        record_ticks(&mut recording, 1..=3);
        recording.stop().unwrap();

        let mut playback = Playback::open(dir.path(), "01abc").unwrap();
        assert_eq!(playback.path(), clip_demo_path(dir.path(), "01ABC"));
        assert_eq!(play_all(&mut playback).len(), 3);
    }

    #[test]
    fn a_missing_recording_is_not_found() {
        let dir = TestDir::new("missing");
        let error = Playback::open(dir.path(), "nothing").unwrap_err();
        assert!(matches!(error, ReplayError::Io(e) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn a_file_that_is_not_a_recording_is_refused() {
        let dir = TestDir::new("notademo");
        let path = dir.path().join("junk.bo2mpdemo");
        fs::write(&path, vec![b'x'; 200]).unwrap();
        assert!(matches!(
            Playback::open_path(&path),
            Err(ReplayError::Magic { .. })
        ));
    }

    #[test]
    fn a_damaged_frame_is_an_error_not_the_end() {
        let dir = TestDir::new("damaged");
        let path = record_demo(dir.path(), "damaged", 2);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&4u32.to_le_bytes()).unwrap();
        file.write_all(&[0xFF; 4]).unwrap();
        drop(file);

        let mut playback = Playback::open_path(&path).unwrap();
        assert!(playback.next_tick().unwrap().is_some());
        assert!(playback.next_tick().unwrap().is_some());
        assert!(matches!(playback.next_tick(), Err(ReplayError::Wire(_))));
    }
}
