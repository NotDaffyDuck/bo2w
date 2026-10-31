//! Reading a zone file off disk into one flat image.
//!
//! A zone file is a header followed by independently compressed records. They
//! are decrypted (when the header says so) and inflated in file order into a
//! single buffer that the zone walker then reads.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::SystemTime;

use asset_core::ZoneTitleKind;

#[derive(Debug)]
pub enum ZoneUnpackFault {
    Io(std::io::Error),
    Header(fastfile_t6::LeadingBytesFault),
    Chunk(fastfile_t6::XChunkError),
    /// An encrypted zone with nothing to derive its key from.
    NoZoneName,
    Inflate {
        chunk: usize,
        inflate_error: String,
    },
    UnknownVersion {
        got: u32,
    },
    ShortBody,
    WithPath {
        path: PathBuf,
        source: Box<ZoneUnpackFault>,
    },
}

impl fmt::Display for ZoneUnpackFault {
    fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(sink, "reading zone: {e}"),
            Self::Header(e) => write!(sink, "bad zone file header: {e}"),
            Self::Chunk(e) => write!(sink, "bad zone chunk: {e}"),
            Self::NoZoneName => write!(sink, "encrypted zone has no name to key its chunks"),
            Self::Inflate { chunk, inflate_error } => {
                write!(sink, "zone chunk {chunk} does not inflate: {inflate_error}")
            }
            Self::UnknownVersion { got } => {
                write!(sink, "unsupported zone version {got:#x}")
            }
            Self::ShortBody => write!(sink, "inflated zone is shorter than its header"),
            Self::WithPath { path: at, source: cause } => write!(sink, "{}: {cause}", at.to_string_lossy()),
        }
    }
}

impl std::error::Error for ZoneUnpackFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WithPath { source: inner, .. } => Some(inner),
            Self::Io(cause) => Some(cause),
            _ => None,
        }
    }
}

/// A whole zone, inflated.
pub struct ZoneTextureData {
    pub game: ZoneTitleKind,
    pub zone_version: u32,
    pub bytes: Vec<u8>,
}

impl ZoneTextureData {
    /// The block sizes and counts that open the image.
    pub fn t6_header(&self) -> Result<fastfile_t6::ZonePreamble, ZoneUnpackFault> {
        fastfile_t6::read_zone_head(&self.bytes).map_err(|_| ZoneUnpackFault::ShortBody)
    }
}

/// What identifies a file's contents without reading it.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SourceKey {
    path: PathBuf,
    byte_len: u64,
    modified: Option<SystemTime>,
}

impl SourceKey {
    /// The key of `file` as it is on disk now; `None` when it cannot be looked at.
    fn of(file: &Path) -> Option<Self> {
        let stat = std::fs::metadata(file).ok()?;
        let modified = stat.modified().ok();
        Some(Self { path: file.to_path_buf(), byte_len: stat.len(), modified })
    }
}

#[derive(Default)]
struct Shared {
    images: HashMap<SourceKey, Weak<ZoneTextureData>>,
    /// One lock per file being opened, so concurrent callers wait for the
    /// first instead of each inflating their own copy.
    in_flight: HashMap<SourceKey, Arc<Mutex<()>>>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

/// The registry, usable even after a panic elsewhere poisoned its lock.
fn registry() -> std::sync::MutexGuard<'static, Shared> {
    shared().lock().unwrap_or_else(PoisonError::into_inner)
}

fn live_image(key: &SourceKey) -> Option<Arc<ZoneTextureData>> {
    let mut books = registry();
    let alive = books.images.get(key).and_then(Weak::upgrade);
    if alive.is_none() {
        books.images.remove(key);
    }
    alive
}

/// Like [`mount_zone`], but every caller holding the same file at the same
/// moment gets the same image. Once the last holder lets go the next call
/// reads the file again.
pub fn open_shared_zone(path: impl AsRef<Path>) -> Result<Arc<ZoneTextureData>, ZoneUnpackFault> {
    let file = path.as_ref();
    match SourceKey::of(file) {
        None => mount_zone(file).map(Arc::new),
        Some(key) => live_image(&key).map_or_else(|| open_behind_gate(file, key), Ok),
    }
}

/// Reads the file once for every caller racing on `key`: whoever gets the
/// gate second finds the first one's image already registered.
fn open_behind_gate(file: &Path, key: SourceKey) -> Result<Arc<ZoneTextureData>, ZoneUnpackFault> {
    let gate = Arc::clone(registry().in_flight.entry(key.clone()).or_default());
    let _held = gate.lock().unwrap_or_else(PoisonError::into_inner);
    match live_image(&key) {
        Some(cached) => Ok(cached),
        None => {
            let fresh = Arc::new(mount_zone(file)?);
            registry().images.insert(key, Arc::downgrade(&fresh));
            Ok(fresh)
        }
    }
}

/// Read and inflate the zone at `path`.
pub fn mount_zone(path: impl AsRef<Path>) -> Result<ZoneTextureData, ZoneUnpackFault> {
    let file = path.as_ref();
    let stem = file.file_stem().and_then(|name| name.to_str());
    let inflated = match std::fs::read(file) {
        Ok(raw) => inflate_image(&raw, stem),
        Err(io) => Err(ZoneUnpackFault::Io(io)),
    };
    inflated.map_err(|cause| ZoneUnpackFault::WithPath { path: file.into(), source: Box::new(cause) })
}

/// `stem` names the zone for a file whose header does not carry its own name.
fn inflate_image(bytes: &[u8], stem: Option<&str>) -> Result<ZoneTextureData, ZoneUnpackFault> {
    let Some(version) = bytes
        .get(8..12)
        .and_then(|field| <[u8; 4]>::try_from(field).ok())
        .map(u32::from_le_bytes)
    else {
        return Err(ZoneUnpackFault::ShortBody)
    };
    if !(version == fastfile_t6::T6_PC_ZONE_REV && fastfile_t6::is_t6_magic(bytes)) {
        return Err(ZoneUnpackFault::UnknownVersion { got: version });
    }

    let header = fastfile_t6::read_ff_preamble(bytes).map_err(ZoneUnpackFault::Header)?;
    let mut cipher = if header.signing.is_encrypted() {
        let name = header.signed_name().or(stem).ok_or(ZoneUnpackFault::NoZoneName)?;
        let made = fastfile_t6::ChunkCipher::new(name.as_bytes(), &fastfile_t6::SALSA20_KEY_PC);
        Some(made.ok_or(ZoneUnpackFault::NoZoneName)?)
    } else {
        None
    };

    let mut scratch = ChunkScratch::new();
    let mut flat = Vec::new();
    for item in fastfile_t6::XChunks::new(bytes, header.body_offset) {
        let piece = item.map_err(ZoneUnpackFault::Chunk)?;
        let plain = scratch.open_piece(&piece, cipher.as_mut())?;
        flat.extend_from_slice(plain);
        if piece.index == 0 {
            if let Ok(top) = fastfile_t6::read_zone_head(&flat) {
                flat.reserve(top.size as usize + fastfile_t6::ZONE_HEAD_BYTES);
            }
        }
    }
    let image = ZoneTextureData { game: ZoneTitleKind::T6, zone_version: header.version, bytes: flat };
    (image.bytes.len() >= fastfile_t6::ZONE_HEAD_BYTES)
        .then_some(image)
        .ok_or(ZoneUnpackFault::ShortBody)
}

/// The two chunk-sized buffers one zone read reuses for every record.
struct ChunkScratch {
    encrypted: Vec<u8>,
    inflated: Vec<u8>,
    inflater: flate2::Decompress,
}

impl ChunkScratch {
    fn new() -> Self {
        Self {
            encrypted: vec![0; fastfile_t6::XCHUNK_SIZE],
            inflated: vec![0; fastfile_t6::XCHUNK_SIZE],
            inflater: flate2::Decompress::new(false),
        }
    }

    /// Decrypts the record when there is a cipher, inflates it and returns
    /// the bytes that came out.
    fn open_piece(
        &mut self,
        piece: &fastfile_t6::XChunk<'_>,
        cipher: Option<&mut fastfile_t6::ChunkCipher>,
    ) -> Result<&[u8], ZoneUnpackFault> {
        let window = &mut self.encrypted[..piece.data.len()];
        window.copy_from_slice(piece.data);
        if let Some(cipher) = cipher {
            cipher.decrypt(piece.stream, window);
        }
        let len = inflate_record(&mut self.inflater, window, &mut self.inflated, piece.index)?;
        Ok(&self.inflated[..len])
    }
}

/// Inflate one record into `out`, which is one chunk long, and return how
/// many bytes came out. The record's deflate stream has to end inside that one
/// chunk. `out` is a fixed scratch buffer on purpose: growing a `Vec` through
/// `decompress_vec` zero-fills its spare capacity on every call, which on a
/// reserved 370 MB image cost minutes.
fn inflate_record(
    inflater: &mut flate2::Decompress,
    data: &[u8],
    out: &mut [u8],
    chunk: usize,
) -> Result<usize, ZoneUnpackFault> {
    inflater.reset(false);
    let outcome = inflater.decompress(data, out, flate2::FlushDecompress::Finish);
    let status = match outcome {
        Ok(done) => done,
        Err(err) => {
            return Err(ZoneUnpackFault::Inflate {
                chunk,
                inflate_error: err.to_string(),
            });
        }
    };
    let produced = inflater.total_out() as usize;
    if !matches!(status, flate2::Status::StreamEnd) {
        let message =
            format!("stream did not end within its chunk ({produced} bytes out, {status:?})");
        return Err(ZoneUnpackFault::Inflate { chunk, inflate_error: message });
    }
    Ok(produced)
}

/// The memory a zone walk fills: one buffer per block, plus the map of which
/// pointer insertions have already been made.
pub struct T6ZoneMemory {
    regions: [Vec<u8>; fastfile_t6::MAX_ZONE_FILE_TOTAL],
    inserted: Vec<u8>,
}

impl T6ZoneMemory {
    pub fn for_header(header: &fastfile_t6::ZonePreamble) -> Self {
        let regions = std::array::from_fn(|slot| vec![0u8; header.block_size[slot] as usize]);
        let inserted = vec![0u8; fastfile_t6::ZoneByteReader::patch_byte_map_size(header)];
        Self { regions, inserted }
    }

    pub fn byte_total(&self) -> usize {
        self.regions.iter().map(|region| region.len()).sum()
    }

    pub fn stream<'a>(
        &'a mut self,
        image: &'a [u8],
    ) -> Result<fastfile_t6::ZoneByteReader<'a>, fastfile_t6::ZoneReadFault> {
        let views: [&mut [u8]; fastfile_t6::MAX_ZONE_FILE_TOTAL] =
            self.regions.each_mut().map(|region| region.as_mut_slice());
        fastfile_t6::ZoneByteReader::new(image, views, &mut self.inserted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_names_its_path_in_the_error() {
        let gone = std::env::temp_dir().join("bo2mp-no-such-zone.ff");
        for attempt in [mount_zone(&gone).map(Arc::new), open_shared_zone(&gone)] {
            match attempt {
                Err(ZoneUnpackFault::WithPath { path: zone_path, source: cause }) => {
                    assert_eq!(zone_path, gone);
                    assert!(matches!(*cause, ZoneUnpackFault::Io(_)));
                }
                other => panic!("expected an AtPath error, got ok={}", other.is_ok()),
            }
        }
    }

    #[test]
    fn short_and_foreign_files_are_refused() {
        let tiny = inflate_image(&[0; 4], None);
        assert!(matches!(tiny, Err(ZoneUnpackFault::ShortBody)));
        let mut alien = [0u8; 16];
        alien[8..12].copy_from_slice(&0x1234u32.to_le_bytes());
        let verdict = inflate_image(&alien, None);
        assert!(matches!(
            verdict,
            Err(ZoneUnpackFault::UnknownVersion { got: 0x1234 })
        ));
    }

    #[test]
    fn a_missing_file_names_its_path() {
        let failure = mount_zone("no/such/zone.ff").err().expect("must fail");
        assert!(
            failure.to_string().starts_with("no/such/zone.ff: "),
            "{failure}"
        );
        let shared_attempt = open_shared_zone("no/such/zone.ff");
        assert!(matches!(shared_attempt, Err(ZoneUnpackFault::WithPath { .. })));
    }

    #[test]
    fn a_record_that_never_ends_is_an_error() {
        // A stored (uncompressed) deflate block that is not the final one.
        let unfinished = [0x00, 0x01, 0x00, 0xfe, 0xff, 0xaa];
        let mut sink = vec![0u8; 64];
        let mut engine = flate2::Decompress::new(false);
        let result = inflate_record(&mut engine, &unfinished, &mut sink, 3);
        assert!(matches!(
            result,
            Err(ZoneUnpackFault::Inflate { chunk: 3, .. })
        ));
    }
}
