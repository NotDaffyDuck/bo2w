//! How a recording sits on disk.
//!
//! A 64-byte header comes first, then the frames:
//!
//! ```text
//! at   size  field
//!  0      8  FORMAT_TAG
//!  8      4  wire revision of the writing build
//! 12      4  random stream layout
//! 16      8  seed the world grew from
//! 24      8  content fingerprint
//! 32     32  map name, padded with zeros
//! 64      -  frames: a 4-byte byte count, then the frame's wire bytes
//! ```
//!
//! Every number is stored little-endian. Each frame holds the same bytes the
//! host would have sent for that tick, so a file cut off mid-write still plays
//! up to the last frame that made it whole.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use net::{
    Frame, WIRE_REVISION_NUMBER, Transport, LinkIoFailure, PacketDecodeFault, ByteCursorReader,
    WorldPropStreamDecoder,
};
use sim::RANDOM_STREAM_LAYOUT_VERSION;

use crate::error::RecordingFault;
use crate::identity::{RecordedGameLabel, REGION_LABEL_CAPACITY};

pub const FORMAT_TAG: [u8; 8] = *b"BO2MPDEM";

const HEADER_LEN: usize = 8 + 4 + 4 + 8 + 8 + REGION_LABEL_CAPACITY;

/// Any byte count above this is read as a corrupt length rather than a frame.
const FRAME_SIZE_LIMIT: u32 = 16 << 20;

fn read_array<const N: usize>(src: &mut impl Read) -> io::Result<[u8; N]> {
    let mut filled = [0u8; N];
    src.read_exact(&mut filled).map(|()| filled)
}

/// Turns running out of file into `Ok(None)`; any other failure stays an error.
fn none_at_eof<T>(attempt: io::Result<T>) -> Result<Option<T>, RecordingFault> {
    match attempt {
        Err(failure) if failure.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        other => Ok(Some(other?)),
    }
}

fn encode_header(label: &RecordedGameLabel) -> [u8; HEADER_LEN] {
    let layout = if label.random_stream_layout == 0 {
        RANDOM_STREAM_LAYOUT_VERSION
    } else {
        label.random_stream_layout
    };
    let mut packed = Vec::with_capacity(HEADER_LEN);
    packed.extend_from_slice(&FORMAT_TAG);
    packed.extend_from_slice(&WIRE_REVISION_NUMBER.to_le_bytes());
    packed.extend_from_slice(&layout.to_le_bytes());
    packed.extend_from_slice(&label.world_seed.to_le_bytes());
    packed.extend_from_slice(&label.content_fingerprint.to_le_bytes());
    packed.extend_from_slice(&label.map_bytes);
    packed.try_into().expect("the header fields add up to HEADER_LEN")
}

/// Reads a header field by field. The tag and the wire revision are checked
/// the moment they arrive, so a file from some other build is reported as
/// such rather than as a short header.
fn decode_header(src: &mut impl Read) -> Result<RecordedGameLabel, RecordingFault> {
    let tag: [u8; 8] = read_array(src)?;
    if tag != FORMAT_TAG {
        return Err(RecordingFault::Magic { found: tag });
    }
    let found = u32::from_le_bytes(read_array(src)?);
    if found != WIRE_REVISION_NUMBER {
        return Err(RecordingFault::Version { found, wanted: WIRE_REVISION_NUMBER });
    }
    let mut label = RecordedGameLabel::default();
    label.random_stream_layout = u32::from_le_bytes(read_array(src)?);
    label.world_seed = u64::from_le_bytes(read_array(src)?);
    label.content_fingerprint = u64::from_le_bytes(read_array(src)?);
    label.map_bytes = read_array(src)?;
    Ok(label)
}

/// Writes one recording, frame after frame.
#[derive(Debug)]
pub struct DemoFileWriter {
    sink: BufWriter<File>,
    location: PathBuf,
    frame_count: u64,
}

impl DemoFileWriter {
    pub fn blank(at: &Path) -> Result<Self, RecordingFault> {
        Self::build_with_signature(at, RecordedGameLabel::default())
    }

    /// Opens a new recording at `at` with `header` written first, creating
    /// missing folders on the way and overwriting whatever file was there.
    pub fn build_with_signature(
        at: &Path,
        header: RecordedGameLabel,
    ) -> Result<Self, RecordingFault> {
        if let Some(folder) = at.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut sink = BufWriter::new(File::create(at)?);
        sink.write_all(&encode_header(&header))?;
        Ok(Self { sink, location: at.to_owned(), frame_count: 0u64 })
    }

    pub fn append(&mut self, frame: &Frame) -> Result<(), RecordingFault> {
        let encoded = frame.wire_bytes();
        let Some(length) = u32::try_from(encoded.len()).ok().filter(|&n| n <= FRAME_SIZE_LIMIT) else {
            return Err(PacketDecodeFault::GarbledRequest("this frame is bigger than a recording allows").into());
        };
        self.sink.write_all(&length.to_le_bytes())?;
        self.sink.write_all(&encoded)?;
        self.frame_count += 1;
        Ok(())
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn path(&self) -> &Path {
        &self.location
    }

    /// Pushes everything to disk and hands back how many frames went in and
    /// where the file is.
    pub fn close(mut self) -> Result<(u64, PathBuf), RecordingFault> {
        self.sink.flush()?;
        let Self { location, frame_count, .. } = self;
        Ok((frame_count, location))
    }
}

/// Reads one recording back, frame after frame.
#[derive(Debug)]
pub struct DemoFileReader {
    source: BufReader<File>,
    location: PathBuf,
    label: RecordedGameLabel,
    object_sync: WorldPropStreamDecoder,
}

impl DemoFileReader {
    pub fn open_at(at: &Path) -> Result<Self, RecordingFault> {
        let mut source = BufReader::new(File::open(at)?);
        let label = decode_header(&mut source)?;
        Ok(Self {
            source,
            location: at.to_owned(),
            label,
            object_sync: WorldPropStreamDecoder::default(),
        })
    }

    pub fn identity(&self) -> RecordedGameLabel {
        self.label
    }

    pub fn path(&self) -> &Path {
        &self.location
    }

    /// The following frame; `None` once the file runs out, including when it
    /// runs out partway through a frame.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, RecordingFault> {
        let Some(length) = none_at_eof(read_array::<4>(&mut self.source))?.map(u32::from_le_bytes) else {
            return Ok(None);
        };
        if length > FRAME_SIZE_LIMIT {
            return Err(PacketDecodeFault::GarbledRequest("frame length is implausible; the file is damaged").into());
        }
        let body_len = length as usize;
        let mut body = vec![0u8; body_len];
        if none_at_eof(self.source.read_exact(&mut body))?.is_none() {
            return Ok(None);
        }
        Ok(Some(Frame::decode(&mut ByteCursorReader::new(&body), &mut self.object_sync)?))
    }
}

#[derive(Debug)]
enum Direction {
    /// Taking frames in; empty after it is closed.
    Writing(Option<DemoFileWriter>),
    Reading(DemoFileReader),
}

/// A demo file wearing the [`Transport`] shape: send appends a frame and
/// receive reads the next one.
#[derive(Debug)]
pub struct DemoFileLink {
    direction: Direction,
}

impl DemoFileLink {
    pub fn for_recording(at: &Path) -> Result<Self, RecordingFault> {
        Self::capture_with_signature(at, RecordedGameLabel::default())
    }

    pub fn capture_with_signature(
        at: &Path,
        header: RecordedGameLabel,
    ) -> Result<Self, RecordingFault> {
        let writer = DemoFileWriter::build_with_signature(at, header)?;
        Ok(Self { direction: Direction::Writing(Some(writer)) })
    }

    pub fn for_playback(at: &Path) -> Result<Self, RecordingFault> {
        let reader = DemoFileReader::open_at(at)?;
        Ok(Self { direction: Direction::Reading(reader) })
    }

    /// The header of a demo being read back; `None` while writing.
    pub fn identity(&self) -> Option<RecordedGameLabel> {
        if let Direction::Reading(reader) = &self.direction {
            Some(reader.identity())
        } else {
            None
        }
    }

    /// Closes a recording and gives its frame count and path. `None` when
    /// there is nothing open to close (reading, or closed already).
    pub fn close(&mut self) -> Result<Option<(u64, PathBuf)>, RecordingFault> {
        let Direction::Writing(slot) = &mut self.direction else {
            return Ok(None);
        };
        slot.take().map(DemoFileWriter::close).transpose()
    }
}

impl Transport for DemoFileLink {
    #[allow(clippy::needless_arbitrary_self_type)]
    fn send(self: &mut Self, outgoing: &Frame) -> Result<(), LinkIoFailure> {
        let refusal = match &mut self.direction {
            Direction::Writing(Some(writer)) => return writer.append(outgoing).map_err(Into::into),
            Direction::Writing(None) => "this recording is finished",
            Direction::Reading(_) => "a demo opened for playback takes no frames",
        };
        Err(LinkIoFailure::Io(io::Error::other(refusal)))
    }

    fn take_frame(&mut self) -> Result<Option<Frame>, LinkIoFailure> {
        // A recording gives nothing back; playback stops where the file does.
        let Direction::Reading(reader) = &mut self.direction else {
            return Err(LinkIoFailure::Ended);
        };
        reader.next_frame()?.ok_or(LinkIoFailure::Ended).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::frames;

    fn identity() -> RecordedGameLabel {
        let mut identity = RecordedGameLabel {
            random_stream_layout: 3,
            world_seed: 0x0123_4567_89AB_CDEF,
            content_fingerprint: 0xFEDC_BA98_7654_3210,
            ..RecordedGameLabel::default()
        };
        identity.assign_region_label("zm_transit");
        identity
    }

    fn write_all(at: &Path, sent: &[Frame]) {
        let mut writer = DemoFileWriter::build_with_signature(at, identity()).unwrap();
        for frame in sent {
            writer.append(frame).unwrap();
        }
        writer.close().unwrap();
    }

    fn read_all(at: &Path) -> Vec<Frame> {
        let mut reader = DemoFileReader::open_at(at).unwrap();
        std::iter::from_fn(|| reader.next_frame().unwrap()).collect()
    }

    #[test]
    fn the_header_is_64_bytes_that_start_with_the_magic_and_the_protocol() {
        let dir = TestDir::new("header");
        let path = dir.path().join("h.bo2mpdemo");
        DemoFileWriter::build_with_signature(&path, identity())
            .unwrap()
            .close()
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[..8], b"BO2MPDEM");
        assert_eq!(bytes[8..12], WIRE_REVISION_NUMBER.to_le_bytes());
        assert_eq!(bytes[12..16], 3u32.to_le_bytes());
        assert_eq!(bytes[16..24], 0x0123_4567_89AB_CDEFu64.to_le_bytes());
        assert_eq!(bytes[24..32], 0xFEDC_BA98_7654_3210u64.to_le_bytes());
        assert_eq!(&bytes[32..42], b"zm_transit");
        assert!(bytes[42..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn an_identity_with_no_scheme_is_written_with_the_current_one() {
        let dir = TestDir::new("scheme");
        let path = dir.path().join("s.bo2mpdemo");
        DemoFileWriter::blank(&path).unwrap().close().unwrap();
        let read = DemoFileReader::open_at(&path).unwrap().identity();
        assert_eq!(read.random_stream_layout, RANDOM_STREAM_LAYOUT_VERSION);
        assert_eq!(read.map_name(), None);
    }

    #[test]
    fn the_identity_comes_back_as_it_was_written() {
        let dir = TestDir::new("identity");
        let path = dir.path().join("i.bo2mpdemo");
        write_all(&path, &[]);
        assert_eq!(DemoFileReader::open_at(&path).unwrap().identity(), identity());
    }

    #[test]
    fn frames_come_back_equal_and_in_order() {
        let dir = TestDir::new("frames");
        let path = dir.path().join("f.bo2mpdemo");
        let sent = frames(6);
        write_all(&path, &sent);
        assert_eq!(read_all(&path), sent);
    }

    #[test]
    fn finishing_reports_the_count_and_the_path() {
        let dir = TestDir::new("finish");
        let path = dir.path().join("c.bo2mpdemo");
        let mut writer = DemoFileWriter::blank(&path).unwrap();
        for frame in frames(4) {
            writer.append(&frame).unwrap();
        }
        assert_eq!(writer.frame_count(), 4);
        assert_eq!(writer.path(), path);
        assert_eq!(writer.close().unwrap(), (4, path));
    }

    #[test]
    fn a_recording_makes_the_folders_it_sits_in() {
        let dir = TestDir::new("folders");
        let path = dir.path().join("a").join("b").join("n.bo2mpdemo");
        DemoFileWriter::blank(&path).unwrap().close().unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn a_header_only_file_has_no_frames() {
        let dir = TestDir::new("empty");
        let path = dir.path().join("e.bo2mpdemo");
        write_all(&path, &[]);
        assert!(read_all(&path).is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_recording_is_refused_by_its_magic() {
        let dir = TestDir::new("magic");
        let path = dir.path().join("m.bo2mpdemo");
        fs::write(&path, b"RIFFxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").unwrap();
        let error = DemoFileReader::open_at(&path).unwrap_err();
        assert!(matches!(error, RecordingFault::Magic { found } if &found == b"RIFFxxxx"));
    }

    #[test]
    fn another_protocol_is_refused_before_the_rest_of_the_header_is_read() {
        let dir = TestDir::new("version");
        let path = dir.path().join("v.bo2mpdemo");
        let mut bytes = FORMAT_TAG.to_vec();
        bytes.extend_from_slice(&(WIRE_REVISION_NUMBER + 1).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let error = DemoFileReader::open_at(&path).unwrap_err();
        assert!(matches!(
            error,
            RecordingFault::Version { found: seen, wanted: ours }
                if seen == WIRE_REVISION_NUMBER + 1 && ours == WIRE_REVISION_NUMBER
        ));
    }

    #[test]
    fn a_file_that_ends_inside_its_header_is_an_io_error() {
        let dir = TestDir::new("short");
        for (tag, keep) in [("none", 0usize), ("magic", 5), ("most", 40)] {
            let path = dir.path().join(format!("{tag}.bo2mpdemo"));
            write_all(&path, &[]);
            let full = fs::read(&path).unwrap();
            fs::write(&path, &full[..keep]).unwrap();
            let error = DemoFileReader::open_at(&path).unwrap_err();
            assert!(
                matches!(&error, RecordingFault::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof),
                "{tag}: {error}"
            );
        }
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = TestDir::new("missing");
        let error = DemoFileReader::open_at(&dir.path().join("nope.bo2mpdemo")).unwrap_err();
        assert!(matches!(error, RecordingFault::Io(e) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn a_frame_cut_off_by_the_end_of_the_file_is_the_end_of_the_recording() {
        let dir = TestDir::new("cutbody");
        let path = dir.path().join("t.bo2mpdemo");
        write_all(&path, &frames(5));
        let whole = fs::read(&path).unwrap();
        fs::write(&path, &whole[..whole.len() - 3]).unwrap();
        assert_eq!(read_all(&path), frames(4));
    }

    #[test]
    fn a_length_cut_off_by_the_end_of_the_file_is_the_end_of_the_recording() {
        let dir = TestDir::new("cutlen");
        let path = dir.path().join("l.bo2mpdemo");
        write_all(&path, &frames(3));
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[9, 0]);
        fs::write(&path, bytes).unwrap();
        assert_eq!(read_all(&path), frames(3));
    }

    #[test]
    fn an_absurd_length_is_a_damaged_file_and_not_a_huge_allocation() {
        let dir = TestDir::new("absurd");
        let path = dir.path().join("a.bo2mpdemo");
        write_all(&path, &frames(1));
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let mut reader = DemoFileReader::open_at(&path).unwrap();
        assert!(reader.next_frame().unwrap().is_some());
        let error = reader.next_frame().unwrap_err();
        assert!(matches!(error, RecordingFault::Wire(PacketDecodeFault::GarbledRequest(_))));
    }

    #[test]
    fn a_frame_that_does_not_decode_is_a_wire_error() {
        let dir = TestDir::new("garbage");
        let path = dir.path().join("g.bo2mpdemo");
        write_all(&path, &[]);
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
        fs::write(&path, bytes).unwrap();
        let error = DemoFileReader::open_at(&path).unwrap().next_frame().unwrap_err();
        assert!(matches!(error, RecordingFault::Wire(_)), "{error}");
    }

    #[test]
    fn a_transport_records_and_then_plays_the_same_frames() {
        let dir = TestDir::new("transport");
        let path = dir.path().join("t.bo2mpdemo");
        let sent = frames(4);
        let mut recording = DemoFileLink::capture_with_signature(&path, identity()).unwrap();
        assert_eq!(recording.identity(), None);
        for frame in &sent {
            recording.send(frame).unwrap();
        }
        assert_eq!(recording.close().unwrap(), Some((4, path.clone())));

        let mut playing = DemoFileLink::for_playback(&path).unwrap();
        assert_eq!(playing.identity(), Some(identity()));
        for frame in &sent {
            assert_eq!(playing.take_frame().unwrap().as_ref(), Some(frame));
        }
        assert!(matches!(playing.take_frame(), Err(LinkIoFailure::Ended)));
        assert!(matches!(playing.take_frame(), Err(LinkIoFailure::Ended)));
    }

    #[test]
    fn a_transport_only_goes_one_way() {
        let dir = TestDir::new("oneway");
        let path = dir.path().join("o.bo2mpdemo");
        let frame = frames(1).remove(0);
        let mut recording = DemoFileLink::for_recording(&path).unwrap();
        assert!(matches!(recording.take_frame(), Err(LinkIoFailure::Ended)));
        recording.send(&frame).unwrap();
        recording.close().unwrap();
        assert!(matches!(recording.send(&frame), Err(LinkIoFailure::Io(_))));
        assert_eq!(recording.close().unwrap(), None);

        let mut playing = DemoFileLink::for_playback(&path).unwrap();
        assert!(matches!(playing.send(&frame), Err(LinkIoFailure::Io(_))));
        assert_eq!(playing.close().unwrap(), None);
    }
}
