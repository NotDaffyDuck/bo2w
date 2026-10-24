//! The on-disk format of a recording.
//!
//! ```text
//! offset  bytes  what
//!      0      8  MAGIC
//!      8      4  network protocol version of the build that wrote it
//!     12      4  random-number scheme
//!     16      8  root seed
//!     24      8  content digest
//!     32     32  zone name, zero padded
//!     64    ...  frames, each a 4-byte length and then that many bytes
//! ```
//!
//! Numbers are little-endian. A frame is exactly what the server would put on
//! the wire for that tick. A recording cut short by a crash is still good up
//! to its last whole frame.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use net::{
    Frame, PROTOCOL_VERSION, Transport, TransportError, WireError, WireReader,
    WorldObjectSyncDecoder,
};
use sim::RNG_DOMAIN_SCHEME;

use crate::error::ReplayError;
use crate::identity::{MatchRecordIdentity, ZONE_FIELD_LEN};

pub const MAGIC: [u8; 8] = *b"BO2MPDEM";

const HEADER_LEN: usize = 8 + 4 + 4 + 8 + 8 + ZONE_FIELD_LEN;

/// A frame longer than this is taken to be a damaged length, not a frame.
const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

fn take<const N: usize>(from: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    from.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// `None` where a read hit the end of the file, the error for any other
/// failure.
fn or_end<T>(read: io::Result<T>) -> Result<Option<T>, ReplayError> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn header_bytes(identity: &MatchRecordIdentity) -> [u8; HEADER_LEN] {
    let scheme = match identity.rng_scheme {
        0 => RNG_DOMAIN_SCHEME,
        chosen => chosen,
    };
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(&MAGIC);
    header[8..12].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&scheme.to_le_bytes());
    header[16..24].copy_from_slice(&identity.root_seed.to_le_bytes());
    header[24..32].copy_from_slice(&identity.content_digest.to_le_bytes());
    header[32..].copy_from_slice(&identity.zone);
    header
}

/// Reads and checks a header. The magic and the protocol are checked as soon
/// as they are read, so a file from another build is reported as that and not
/// as a short header.
fn read_header(from: &mut impl Read) -> Result<MatchRecordIdentity, ReplayError> {
    let magic = take::<8>(from)?;
    if magic != MAGIC {
        return Err(ReplayError::Magic { found: magic });
    }
    let version = u32::from_le_bytes(take(from)?);
    if version != PROTOCOL_VERSION {
        return Err(ReplayError::Version {
            found: version,
            expected: PROTOCOL_VERSION,
        });
    }
    Ok(MatchRecordIdentity {
        rng_scheme: u32::from_le_bytes(take(from)?),
        root_seed: u64::from_le_bytes(take(from)?),
        content_digest: u64::from_le_bytes(take(from)?),
        zone: take(from)?,
    })
}

#[derive(Debug)]
pub struct RecordWriter {
    out: BufWriter<File>,
    path: PathBuf,
    frames: u64,
}

impl RecordWriter {
    pub fn create(path: &Path) -> Result<Self, ReplayError> {
        Self::create_with_identity(path, MatchRecordIdentity::default())
    }

    /// Starts a recording at `path`, making its folders if they are missing
    /// and replacing any file already there.
    pub fn create_with_identity(
        path: &Path,
        identity: MatchRecordIdentity,
    ) -> Result<Self, ReplayError> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&header_bytes(&identity))?;
        Ok(Self {
            out,
            path: path.to_path_buf(),
            frames: 0,
        })
    }

    pub fn write_frame(&mut self, frame: &Frame) -> Result<(), ReplayError> {
        let bytes = frame.to_bytes();
        let length = u32::try_from(bytes.len())
            .ok()
            .filter(|&length| length <= MAX_FRAME_LEN)
            .ok_or(WireError::Malformed(
                "frame is too large for a recording to hold",
            ))?;
        self.out.write_all(&length.to_le_bytes())?;
        self.out.write_all(&bytes)?;
        self.frames += 1;
        Ok(())
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flushes to disk; the frame count and the file's path.
    pub fn finish(mut self) -> Result<(u64, PathBuf), ReplayError> {
        self.out.flush()?;
        Ok((self.frames, self.path))
    }
}

#[derive(Debug)]
pub struct RecordReader {
    input: BufReader<File>,
    path: PathBuf,
    identity: MatchRecordIdentity,
    world_objects: WorldObjectSyncDecoder,
}

impl RecordReader {
    pub fn open(path: &Path) -> Result<Self, ReplayError> {
        let mut input = BufReader::new(File::open(path)?);
        let identity = read_header(&mut input)?;
        Ok(Self {
            input,
            path: path.to_path_buf(),
            identity,
            world_objects: WorldObjectSyncDecoder::default(),
        })
    }

    pub fn identity(&self) -> MatchRecordIdentity {
        self.identity
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The next frame, or `None` at the end of the file (a frame cut off by
    /// the end counts as the end).
    pub fn read_frame(&mut self) -> Result<Option<Frame>, ReplayError> {
        let Some(prefix) = or_end(take::<4>(&mut self.input))? else {
            return Ok(None);
        };
        let length = u32::from_le_bytes(prefix);
        if length > MAX_FRAME_LEN {
            return Err(
                WireError::Malformed("frame length is implausible; the file is damaged").into(),
            );
        }
        let mut bytes = vec![0u8; length as usize];
        if or_end(self.input.read_exact(&mut bytes))?.is_none() {
            return Ok(None);
        }
        let frame = Frame::decode(&mut WireReader::new(&bytes), &mut self.world_objects)?;
        Ok(Some(frame))
    }
}

#[derive(Debug)]
enum Mode {
    /// Writing; `None` once finished.
    Recording(Option<RecordWriter>),
    Playing(RecordReader),
}

/// A recording seen as a [`Transport`]: sending writes a frame, receiving
/// reads one.
#[derive(Debug)]
pub struct FileTransport {
    mode: Mode,
}

impl FileTransport {
    pub fn recording(path: &Path) -> Result<Self, ReplayError> {
        Self::recording_with_identity(path, MatchRecordIdentity::default())
    }

    pub fn recording_with_identity(
        path: &Path,
        identity: MatchRecordIdentity,
    ) -> Result<Self, ReplayError> {
        let writer = RecordWriter::create_with_identity(path, identity)?;
        Ok(Self {
            mode: Mode::Recording(Some(writer)),
        })
    }

    pub fn playback(path: &Path) -> Result<Self, ReplayError> {
        Ok(Self {
            mode: Mode::Playing(RecordReader::open(path)?),
        })
    }

    /// The header of the recording being played; `None` when recording.
    pub fn identity(&self) -> Option<MatchRecordIdentity> {
        match &self.mode {
            Mode::Playing(reader) => Some(reader.identity()),
            Mode::Recording(_) => None,
        }
    }

    /// Ends a recording: its frame count and path. `None` when there is
    /// nothing to end (playing, or already finished).
    pub fn finish(&mut self) -> Result<Option<(u64, PathBuf)>, ReplayError> {
        match &mut self.mode {
            Mode::Recording(writer) => writer.take().map(RecordWriter::finish).transpose(),
            Mode::Playing(_) => Ok(None),
        }
    }
}

impl Transport for FileTransport {
    fn send(&mut self, frame: &Frame) -> Result<(), TransportError> {
        match &mut self.mode {
            Mode::Recording(Some(writer)) => Ok(writer.write_frame(frame)?),
            Mode::Recording(None) => Err(TransportError::Io(io::Error::other(
                "this recording is finished",
            ))),
            Mode::Playing(_) => Err(TransportError::Io(io::Error::other(
                "this FileTransport was opened for playback, not recording",
            ))),
        }
    }

    fn recv(&mut self) -> Result<Option<Frame>, TransportError> {
        match &mut self.mode {
            Mode::Playing(reader) => reader.read_frame()?.map(Some).ok_or(TransportError::Ended),
            Mode::Recording(_) => Err(TransportError::Ended),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::frames;

    fn identity() -> MatchRecordIdentity {
        let mut identity = MatchRecordIdentity {
            rng_scheme: 3,
            root_seed: 0x0123_4567_89AB_CDEF,
            content_digest: 0xFEDC_BA98_7654_3210,
            ..MatchRecordIdentity::default()
        };
        identity.set_zone("zm_transit");
        identity
    }

    fn write_all(path: &Path, frames: &[Frame]) {
        let mut writer = RecordWriter::create_with_identity(path, identity()).unwrap();
        for frame in frames {
            writer.write_frame(frame).unwrap();
        }
        writer.finish().unwrap();
    }

    fn read_all(path: &Path) -> Vec<Frame> {
        let mut reader = RecordReader::open(path).unwrap();
        std::iter::from_fn(|| reader.read_frame().unwrap()).collect()
    }

    #[test]
    fn the_header_is_64_bytes_that_start_with_the_magic_and_the_protocol() {
        let dir = TestDir::new("header");
        let path = dir.path().join("h.bo2mpdemo");
        RecordWriter::create_with_identity(&path, identity())
            .unwrap()
            .finish()
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[..8], b"BO2MPDEM");
        assert_eq!(bytes[8..12], PROTOCOL_VERSION.to_le_bytes());
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
        RecordWriter::create(&path).unwrap().finish().unwrap();
        let read = RecordReader::open(&path).unwrap().identity();
        assert_eq!(read.rng_scheme, RNG_DOMAIN_SCHEME);
        assert_eq!(read.zone_name(), None);
    }

    #[test]
    fn the_identity_comes_back_as_it_was_written() {
        let dir = TestDir::new("identity");
        let path = dir.path().join("i.bo2mpdemo");
        write_all(&path, &[]);
        assert_eq!(RecordReader::open(&path).unwrap().identity(), identity());
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
        let mut writer = RecordWriter::create(&path).unwrap();
        for frame in frames(4) {
            writer.write_frame(&frame).unwrap();
        }
        assert_eq!(writer.frames(), 4);
        assert_eq!(writer.path(), path);
        assert_eq!(writer.finish().unwrap(), (4, path));
    }

    #[test]
    fn a_recording_makes_the_folders_it_sits_in() {
        let dir = TestDir::new("folders");
        let path = dir.path().join("a").join("b").join("n.bo2mpdemo");
        RecordWriter::create(&path).unwrap().finish().unwrap();
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
        let error = RecordReader::open(&path).unwrap_err();
        assert!(matches!(error, ReplayError::Magic { found } if &found == b"RIFFxxxx"));
    }

    #[test]
    fn another_protocol_is_refused_before_the_rest_of_the_header_is_read() {
        let dir = TestDir::new("version");
        let path = dir.path().join("v.bo2mpdemo");
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let error = RecordReader::open(&path).unwrap_err();
        assert!(matches!(
            error,
            ReplayError::Version { found, expected }
                if found == PROTOCOL_VERSION + 1 && expected == PROTOCOL_VERSION
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
            let error = RecordReader::open(&path).unwrap_err();
            assert!(
                matches!(&error, ReplayError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof),
                "{tag}: {error}"
            );
        }
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = TestDir::new("missing");
        let error = RecordReader::open(&dir.path().join("nope.bo2mpdemo")).unwrap_err();
        assert!(matches!(error, ReplayError::Io(e) if e.kind() == io::ErrorKind::NotFound));
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
        let mut reader = RecordReader::open(&path).unwrap();
        assert!(reader.read_frame().unwrap().is_some());
        let error = reader.read_frame().unwrap_err();
        assert!(matches!(error, ReplayError::Wire(WireError::Malformed(_))));
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
        let error = RecordReader::open(&path).unwrap().read_frame().unwrap_err();
        assert!(matches!(error, ReplayError::Wire(_)), "{error}");
    }

    #[test]
    fn a_transport_records_and_then_plays_the_same_frames() {
        let dir = TestDir::new("transport");
        let path = dir.path().join("t.bo2mpdemo");
        let sent = frames(4);
        let mut recording = FileTransport::recording_with_identity(&path, identity()).unwrap();
        assert_eq!(recording.identity(), None);
        for frame in &sent {
            recording.send(frame).unwrap();
        }
        assert_eq!(recording.finish().unwrap(), Some((4, path.clone())));

        let mut playing = FileTransport::playback(&path).unwrap();
        assert_eq!(playing.identity(), Some(identity()));
        for frame in &sent {
            assert_eq!(playing.recv().unwrap().as_ref(), Some(frame));
        }
        assert!(matches!(playing.recv(), Err(TransportError::Ended)));
        assert!(matches!(playing.recv(), Err(TransportError::Ended)));
    }

    #[test]
    fn a_transport_only_goes_one_way() {
        let dir = TestDir::new("oneway");
        let path = dir.path().join("o.bo2mpdemo");
        let frame = frames(1).remove(0);
        let mut recording = FileTransport::recording(&path).unwrap();
        assert!(matches!(recording.recv(), Err(TransportError::Ended)));
        recording.send(&frame).unwrap();
        recording.finish().unwrap();
        assert!(matches!(recording.send(&frame), Err(TransportError::Io(_))));
        assert_eq!(recording.finish().unwrap(), None);

        let mut playing = FileTransport::playback(&path).unwrap();
        assert!(matches!(playing.send(&frame), Err(TransportError::Io(_))));
        assert_eq!(playing.finish().unwrap(), None);
    }
}
