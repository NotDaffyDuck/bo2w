//! Writing a match down tick by tick, and reading one back the same way.
//!
//! Both directions go through the frames a server sends its clients, so a
//! replay shows exactly what a client would have been shown.

use std::path::{Path, PathBuf};

use crate::clip::{ClipLoopBuffer, find_demo_to_play};
use crate::error::RecordingFault;
use crate::file::{DemoFileReader, DemoFileWriter};
use crate::identity::RecordedGameLabel;
use crate::names::{first_unused_demo_name, recording_file, tidy_clip_name};

/// A demo being written. Every tick handed to [`DemoRecorder::append_tick`]
/// goes straight to the (buffered) file, so a crash costs only the newest
/// moments.
#[derive(Debug)]
pub struct DemoRecorder {
    writer: DemoFileWriter,
    differ: net::FrameDiffWriter,
    shown_as: String,
    written: u64,
}

impl DemoRecorder {
    /// Starts `<artifacts>/demos/<wanted>.bo2mpdemo` with a blank match label.
    pub fn start(artifacts: &Path, wanted: &str) -> Result<Self, RecordingFault> {
        Self::start_labelled(artifacts, wanted, RecordedGameLabel::default())
    }

    /// Like [`DemoRecorder::start`], with `label` in the header. `wanted` is
    /// cut down to a safe file name; if nothing of it survives, the next free
    /// `demoNNNN` is taken instead.
    pub fn start_labelled(artifacts: &Path, wanted: &str, label: RecordedGameLabel) -> Result<Self, RecordingFault> {
        let stem = tidy_clip_name(wanted)
            .unwrap_or_else(|| first_unused_demo_name(artifacts));
        Self::open_new(recording_file(artifacts, &stem), stem, label)
    }

    /// Starts a demo at exactly `path`, titled after the folder holding it (a
    /// clip's folder is named for its id), or `clip` when there is none.
    pub fn begin_at_location(path: PathBuf, label: RecordedGameLabel) -> Result<Self, RecordingFault> {
        let shown_as = path
            .parent()
            .and_then(Path::file_name)
            .map_or_else(|| "clip".to_owned(), |folder| folder.to_string_lossy().into_owned());
        Self::open_new(path, shown_as, label)
    }

    fn open_new(target: PathBuf, shown_as: String, label: RecordedGameLabel) -> Result<Self, RecordingFault> {
        let writer = DemoFileWriter::build_with_signature(&target, label)?;
        let differ = net::FrameDiffWriter::new();
        Ok(Self { writer, differ, shown_as, written: 0 })
    }

    /// Appends one tick: what the server was fed and the snapshot it produced.
    /// Ticks have to arrive in order.
    pub fn append_tick(&mut self, fed: &sim::StepInputs, state: &sim::Snapshot) -> Result<(), RecordingFault> {
        let encoded = net::frame_for_tick(&mut self.differ, fed, state);
        self.writer.append(&encoded)?;
        self.written += 1;
        Ok(())
    }

    /// Appends everything the ring holds, oldest first, and says how many
    /// ticks that came to.
    pub fn append_ring(&mut self, ring: &ClipLoopBuffer) -> Result<u64, RecordingFault> {
        ring.oldest_first()
            .try_fold(0_u64, |count, (fed, state)| self.append_tick(fed, state).map(|()| count + 1))
    }

    /// How many ticks have gone into the file so far.
    pub fn ticks_written(&self) -> u64 {
        self.written
    }

    pub fn name(&self) -> &str {
        &self.shown_as
    }

    pub fn path(&self) -> &Path {
        self.writer.path()
    }

    /// Closes the demo, giving back how many ticks it holds and where it is.
    pub fn stop(self) -> Result<(u64, PathBuf), RecordingFault> {
        self.writer.close()
    }
}

/// One tick read back from a demo, as a client would have received it.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedTick {
    pub commands: Vec<(sim::ClientId, playerstate_t6::game::InputCommand)>,
    pub intents: Vec<(sim::ClientId, sim::PlayerIntent)>,
    pub state: sim::Snapshot,
    /// The frame it was decoded from, for whatever a snapshot leaves out.
    pub source_frame: net::Frame,
}

/// A demo being read back.
#[derive(Debug)]
pub struct DemoReplay {
    reader: DemoFileReader,
    undiffer: net::FrameDiffReader,
    read_so_far: u64,
}

impl DemoReplay {
    /// Opens whatever `asked` names: a demo by that name, failing that a
    /// saved clip.
    pub fn open(artifacts: &Path, asked: &str) -> Result<Self, RecordingFault> {
        Self::open_file(&find_demo_to_play(artifacts, asked))
    }

    pub fn open_file(file: &Path) -> Result<Self, RecordingFault> {
        let reader = DemoFileReader::open_at(file)?;
        let undiffer = net::FrameDiffReader::new();
        Ok(Self { reader, undiffer, read_so_far: 0 })
    }

    /// The next tick, or `None` once the demo has run out.
    pub fn next_tick(&mut self) -> Result<Option<DecodedTick>, RecordingFault> {
        let Some(source_frame) = self.reader.next_frame()? else {
            return Ok(None);
        };
        let mut state = self.undiffer.decode(&source_frame.frame_diff_section)?;
        state.meta = source_frame.meta_rows.clone();
        self.read_so_far += 1;
        Ok(Some(DecodedTick {
            commands: source_frame.inputs.clone(),
            intents: source_frame.actions.clone(),
            state,
            source_frame,
        }))
    }

    /// How many ticks have been read so far.
    pub fn ticks_played(&self) -> u64 {
        self.read_so_far
    }

    pub fn path(&self) -> &Path {
        self.reader.path()
    }

    /// What the demo's header says about the match it was taken from.
    pub fn identity(&self) -> RecordedGameLabel {
        self.reader.identity()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{CLIP_RECORDING_FILE, clip_recording_file, make_clip_folder};
    use crate::error::RecordingFault;
    use crate::testdir::TestDir;
    use crate::testing::{PLAYER, input, snapshot};
    use std::fs;
    use std::io::{self, Write};

    fn append_ticks(demo: &mut DemoRecorder, ticks: impl IntoIterator<Item = u32>) {
        for number in ticks {
            demo.append_tick(&input(number), &snapshot(number)).unwrap();
        }
    }

    fn read_all(replay: &mut DemoReplay) -> Vec<DecodedTick> {
        std::iter::from_fn(|| replay.next_tick().unwrap()).collect()
    }

    fn write_demo(artifacts: &Path, stem: &str, ticks: u32) -> PathBuf {
        let mut demo = DemoRecorder::start(artifacts, stem).unwrap();
        append_ticks(&mut demo, 1..=ticks);
        demo.stop().unwrap().1
    }

    fn clip_folder(artifacts: &Path, clip_id: &str) -> PathBuf {
        make_clip_folder(artifacts, clip_id).unwrap()
    }

    #[test]
    fn what_is_recorded_plays_back_tick_for_tick() {
        let dir = TestDir::new("roundtrip");
        write_demo(dir.path(), "round", 5);

        let mut replay = DemoReplay::open(dir.path(), "round").unwrap();
        let played = read_all(&mut replay);

        assert_eq!(played.len(), 5);
        for (index, tick) in played.iter().enumerate() {
            let number = index as u32 + 1;
            assert_eq!(tick.state.tick, sim::Tick(number));
            assert_eq!(tick.commands, input(number).cmds);
            assert_eq!(tick.intents, input(number).actions);
            let (id, player) = &tick.state.players[0];
            assert_eq!(*id, PLAYER);
            assert_eq!(player.origin, [number as f32, 2.0, 3.0]);
            assert_eq!(player.last_cmd_ms, number as i32 * 50);
        }
        assert_eq!(replay.ticks_played(), 5);
    }

    #[test]
    fn the_join_request_comes_back_on_the_tick_it_was_made() {
        let dir = TestDir::new("actions");
        write_demo(dir.path(), "actions", 3);
        let played = read_all(&mut DemoReplay::open(dir.path(), "actions").unwrap());
        assert_eq!(played[0].intents.len(), 1);
        assert!(played[1].intents.is_empty());
        assert!(played[2].intents.is_empty());
    }

    #[test]
    fn the_frame_a_tick_came_from_is_kept_whole() {
        let dir = TestDir::new("frame");
        write_demo(dir.path(), "frame", 2);
        let played = read_all(&mut DemoReplay::open(dir.path(), "frame").unwrap());
        assert_eq!(played[1].source_frame.tick, sim::Tick(2));
        assert_eq!(played[1].source_frame.inputs, played[1].commands);
        assert_eq!(played[1].source_frame.meta_rows, played[1].state.meta);
    }

    #[test]
    fn a_recording_that_ended_keeps_ending() {
        let dir = TestDir::new("ended");
        write_demo(dir.path(), "ended", 1);
        let mut replay = DemoReplay::open(dir.path(), "ended").unwrap();
        assert!(replay.next_tick().unwrap().is_some());
        for _ in 0..3 {
            assert!(replay.next_tick().unwrap().is_none());
        }
        assert_eq!(replay.ticks_played(), 1);
    }

    #[test]
    fn an_empty_recording_has_no_ticks() {
        let dir = TestDir::new("empty");
        let file = write_demo(dir.path(), "empty", 0);
        let mut replay = DemoReplay::open_file(&file).unwrap();
        assert!(replay.next_tick().unwrap().is_none());
        assert_eq!(replay.ticks_played(), 0);
    }

    #[test]
    fn a_recording_stops_with_its_tick_count_and_its_file() {
        let dir = TestDir::new("stop");
        let mut demo = DemoRecorder::start(dir.path(), "stopped").unwrap();
        assert_eq!(demo.ticks_written(), 0);
        append_ticks(&mut demo, 1..=4);
        assert_eq!(demo.ticks_written(), 4);
        let where_it_was = demo.path().to_path_buf();
        let (ticks, written) = demo.stop().unwrap();
        assert_eq!(ticks, 4);
        assert_eq!(written, where_it_was);
        assert_eq!(written, recording_file(dir.path(), "stopped"));
        assert!(written.is_file());
    }

    #[test]
    fn a_requested_name_is_cut_down_to_a_safe_one() {
        let dir = TestDir::new("sanitize");
        let demo = DemoRecorder::start(dir.path(), "../my demo!").unwrap();
        assert_eq!(demo.name(), "mydemo");
        assert_eq!(demo.path(), recording_file(dir.path(), "mydemo"));
    }

    #[test]
    fn an_unnamed_recording_takes_the_next_free_automatic_name() {
        let dir = TestDir::new("auto");
        let first = DemoRecorder::start(dir.path(), "").unwrap();
        assert_eq!(first.name(), "demo0000");
        let second = DemoRecorder::start(dir.path(), "!!!").unwrap();
        assert_eq!(second.name(), "demo0001");
        assert_eq!(second.path(), recording_file(dir.path(), "demo0001"));
    }

    #[test]
    fn the_match_identity_is_in_the_header() {
        let dir = TestDir::new("identity");
        let mut label = RecordedGameLabel {
            random_stream_layout: 1,
            world_seed: 99,
            content_fingerprint: 1234,
            ..RecordedGameLabel::default()
        };
        label.assign_region_label("zm_transit");
        DemoRecorder::start_labelled(dir.path(), "ident", label)
            .unwrap()
            .stop()
            .unwrap();

        let replay = DemoReplay::open(dir.path(), "ident").unwrap();
        assert_eq!(replay.identity(), label);
        assert_eq!(replay.identity().map_name(), Some("zm_transit"));
    }

    #[test]
    fn a_recording_started_at_a_path_is_named_for_its_folder() {
        let dir = TestDir::new("atpath");
        let folder = clip_folder(dir.path(), "01CLIP");
        let target = folder.join(CLIP_RECORDING_FILE);
        let demo = DemoRecorder::begin_at_location(target.clone(), RecordedGameLabel::default()).unwrap();
        assert_eq!(demo.name(), "01CLIP");
        assert_eq!(demo.path(), target);
    }

    #[test]
    fn a_recording_started_with_no_folder_is_called_clip() {
        let demo = DemoRecorder::begin_at_location(
            PathBuf::from("bo2mp-replay-nofolder-test.bo2mpdemo"),
            RecordedGameLabel::default(),
        )
        .unwrap();
        let left_behind = demo.path().to_path_buf();
        assert_eq!(demo.name(), "clip");
        demo.stop().unwrap();
        fs::remove_file(left_behind).unwrap();
    }

    #[test]
    fn a_started_recording_makes_the_folders_it_needs() {
        let dir = TestDir::new("folders");
        let nested = dir.path().join("deep").join("er").join("x.bo2mpdemo");
        DemoRecorder::begin_at_location(nested.clone(), RecordedGameLabel::default())
            .unwrap()
            .stop()
            .unwrap();
        assert!(nested.is_file());
    }

    #[test]
    fn a_clip_ring_is_recorded_oldest_first() {
        let dir = TestDir::new("ring");
        let mut ring = ClipLoopBuffer::default();
        for tick in 10..=14 {
            ring.record_tick(input(tick), snapshot(tick));
        }
        let mut demo = DemoRecorder::start(dir.path(), "fromring").unwrap();
        assert_eq!(demo.append_ring(&ring).unwrap(), 5);
        assert_eq!(demo.ticks_written(), 5);
        let file = demo.stop().unwrap().1;

        let played = read_all(&mut DemoReplay::open_file(&file).unwrap());
        let order: Vec<u32> = played.iter().map(|tick| tick.state.tick.0).collect();
        assert_eq!(order, [10, 11, 12, 13, 14]);
    }

    #[test]
    fn an_empty_clip_ring_records_nothing() {
        let dir = TestDir::new("emptyring");
        let mut demo = DemoRecorder::start(dir.path(), "none").unwrap();
        assert_eq!(demo.append_ring(&ClipLoopBuffer::default()).unwrap(), 0);
        assert_eq!(demo.ticks_written(), 0);
    }

    #[test]
    fn a_saved_clip_opens_by_its_id() {
        let dir = TestDir::new("openclip");
        let folder = clip_folder(dir.path(), "01ABC");
        let mut demo = DemoRecorder::begin_at_location(
            folder.join(CLIP_RECORDING_FILE),
            RecordedGameLabel::default(),
        )
        .unwrap();
        append_ticks(&mut demo, 1..=3);
        demo.stop().unwrap();

        let mut replay = DemoReplay::open(dir.path(), "01abc").unwrap();
        assert_eq!(replay.path(), clip_recording_file(dir.path(), "01ABC"));
        assert_eq!(read_all(&mut replay).len(), 3);
    }

    #[test]
    fn a_missing_recording_is_not_found() {
        let dir = TestDir::new("missing");
        let error = DemoReplay::open(dir.path(), "nothing").unwrap_err();
        assert!(matches!(error, RecordingFault::Io(e) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn a_file_that_is_not_a_recording_is_refused() {
        let dir = TestDir::new("notademo");
        let junk = dir.path().join("junk.bo2mpdemo");
        fs::write(&junk, vec![b'x'; 200]).unwrap();
        assert!(matches!(
            DemoReplay::open_file(&junk),
            Err(RecordingFault::Magic { .. })
        ));
    }

    #[test]
    fn a_damaged_frame_is_an_error_not_the_end() {
        let dir = TestDir::new("damaged");
        let file = write_demo(dir.path(), "damaged", 2);
        let mut tail = fs::OpenOptions::new().append(true).open(&file).unwrap();
        tail.write_all(&4u32.to_le_bytes()).unwrap();
        tail.write_all(&[0xFF; 4]).unwrap();
        drop(tail);

        let mut replay = DemoReplay::open_file(&file).unwrap();
        assert!(replay.next_tick().unwrap().is_some());
        assert!(replay.next_tick().unwrap().is_some());
        assert!(matches!(replay.next_tick(), Err(RecordingFault::Wire(_))));
    }
}
