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

use asset_core::ZoneGame;

#[derive(Debug)]
pub enum ZoneOpenError {
    Io(std::io::Error),
    Header(fastfile_t6::FileHeaderError),
    Chunk(fastfile_t6::XChunkError),
    /// An encrypted zone with nothing to derive its key from.
    NoZoneName,
    Inflate {
        chunk: usize,
        message: String,
    },
    UnsupportedVersion {
        got: u32,
    },
    Truncated,
    AtPath {
        path: PathBuf,
        source: Box<ZoneOpenError>,
    },
}

impl fmt::Display for ZoneOpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "reading zone: {e}"),
            Self::Header(e) => write!(f, "bad zone file header: {e}"),
            Self::Chunk(e) => write!(f, "bad zone chunk: {e}"),
            Self::NoZoneName => write!(f, "encrypted zone has no name to key its chunks"),
            Self::Inflate { chunk, message } => {
                write!(f, "zone chunk {chunk} does not inflate: {message}")
            }
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported zone version {got:#x}")
            }
            Self::Truncated => write!(f, "inflated zone is shorter than its header"),
            Self::AtPath { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ZoneOpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::AtPath { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A whole zone, inflated.
pub struct ZoneImage {
    pub game: ZoneGame,
    pub version: u32,
    pub bytes: Vec<u8>,
}

impl ZoneImage {
    /// The block sizes and counts that open the image.
    pub fn t6_header(&self) -> Result<fastfile_t6::ZoneHeader, ZoneOpenError> {
        fastfile_t6::parse_zone_header(&self.bytes).map_err(|_| ZoneOpenError::Truncated)
    }
}

/// What identifies a file's contents without reading it.
type SourceKey = (PathBuf, u64, Option<SystemTime>);

fn source_key(path: &Path) -> Option<SourceKey> {
    let meta = std::fs::metadata(path).ok()?;
    Some((path.to_owned(), meta.len(), meta.modified().ok()))
}

#[derive(Default)]
struct Shared {
    images: HashMap<SourceKey, Weak<ZoneImage>>,
    /// One lock per file being opened, so concurrent callers wait for the
    /// first instead of each inflating their own copy.
    in_flight: HashMap<SourceKey, Arc<Mutex<()>>>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

fn live_image(key: &SourceKey) -> Option<Arc<ZoneImage>> {
    let mut shared = shared().lock().unwrap_or_else(PoisonError::into_inner);
    let image = shared.images.get(key).and_then(Weak::upgrade);
    if image.is_none() {
        shared.images.remove(key);
    }
    image
}

/// Like [`open_zone`], but every caller holding the same file at the same
/// moment gets the same image. Once the last holder lets go the next call
/// reads the file again.
pub fn open_zone_shared(path: impl AsRef<Path>) -> Result<Arc<ZoneImage>, ZoneOpenError> {
    let path = path.as_ref();
    let Some(key) = source_key(path) else {
        return open_zone(path).map(Arc::new);
    };
    if let Some(image) = live_image(&key) {
        return Ok(image);
    }
    let flight = Arc::clone(
        shared()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .in_flight
            .entry(key.clone())
            .or_default(),
    );
    let _turn = flight.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(image) = live_image(&key) {
        return Ok(image);
    }
    let image = Arc::new(open_zone(path)?);
    shared()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .images
        .insert(key, Arc::downgrade(&image));
    Ok(image)
}

/// Read and inflate the zone at `path`.
pub fn open_zone(path: impl AsRef<Path>) -> Result<ZoneImage, ZoneOpenError> {
    let path = path.as_ref();
    let stem = path.file_stem().and_then(|stem| stem.to_str());
    std::fs::read(path)
        .map_err(ZoneOpenError::Io)
        .and_then(|bytes| inflate_image(&bytes, stem))
        .map_err(|source| ZoneOpenError::AtPath {
            path: path.to_owned(),
            source: Box::new(source),
        })
}

/// `stem` names the zone for a file whose header does not carry its own name.
fn inflate_image(bytes: &[u8], stem: Option<&str>) -> Result<ZoneImage, ZoneOpenError> {
    let version = bytes
        .get(8..12)
        .and_then(|field| <[u8; 4]>::try_from(field).ok())
        .map(u32::from_le_bytes)
        .ok_or(ZoneOpenError::Truncated)?;
    if version != fastfile_t6::ZONE_VERSION_PC || !fastfile_t6::is_t6_magic(bytes) {
        return Err(ZoneOpenError::UnsupportedVersion { got: version });
    }

    let header = fastfile_t6::parse_file_header(bytes).map_err(ZoneOpenError::Header)?;
    let mut cipher = if header.signing.is_encrypted() {
        let name = header
            .zone_name()
            .or(stem)
            .ok_or(ZoneOpenError::NoZoneName)?;
        let cipher = fastfile_t6::ChunkCipher::new(name.as_bytes(), &fastfile_t6::SALSA20_KEY_PC)
            .ok_or(ZoneOpenError::NoZoneName)?;
        Some(Box::new(cipher))
    } else {
        None
    };

    let mut image = Vec::new();
    let mut record = vec![0u8; fastfile_t6::XCHUNK_SIZE];
    let mut inflated = vec![0u8; fastfile_t6::XCHUNK_SIZE];
    let mut inflater = flate2::Decompress::new(false);
    for chunk in fastfile_t6::XChunks::new(bytes, header.body_offset) {
        let chunk = chunk.map_err(ZoneOpenError::Chunk)?;
        let data = &mut record[..chunk.data.len()];
        data.copy_from_slice(chunk.data);
        if let Some(cipher) = cipher.as_mut() {
            cipher.decrypt(chunk.stream, data);
        }
        let produced = inflate_record(&mut inflater, data, &mut inflated, chunk.index)?;
        image.extend_from_slice(&inflated[..produced]);
        if chunk.index == 0
            && let Ok(xfile) = fastfile_t6::parse_zone_header(&image)
        {
            image.reserve(xfile.size as usize + fastfile_t6::XFILE_HEADER_LEN);
        }
    }
    if image.len() < fastfile_t6::XFILE_HEADER_LEN {
        return Err(ZoneOpenError::Truncated);
    }
    Ok(ZoneImage {
        game: ZoneGame::T6,
        version: header.version,
        bytes: image,
    })
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
) -> Result<usize, ZoneOpenError> {
    inflater.reset(false);
    let status = inflater
        .decompress(data, out, flate2::FlushDecompress::Finish)
        .map_err(|e| ZoneOpenError::Inflate {
            chunk,
            message: e.to_string(),
        })?;
    let produced = inflater.total_out() as usize;
    if status != flate2::Status::StreamEnd {
        return Err(ZoneOpenError::Inflate {
            chunk,
            message: format!("stream did not end within its chunk ({produced} bytes out, {status:?})"),
        });
    }
    Ok(produced)
}

/// The memory a zone walk fills: one buffer per block, plus the map of which
/// pointer insertions have already been made.
pub struct T6ZoneMemory {
    blocks: [Vec<u8>; fastfile_t6::MAX_XFILE_COUNT],
    insert_map: Vec<u8>,
}

impl T6ZoneMemory {
    pub fn for_header(header: &fastfile_t6::ZoneHeader) -> Self {
        Self {
            blocks: std::array::from_fn(|i| vec![0u8; header.block_size[i] as usize]),
            insert_map: vec![0u8; fastfile_t6::ZoneStream::insert_map_len(header)],
        }
    }

    pub fn total_bytes(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }

    pub fn stream<'a>(
        &'a mut self,
        image: &'a [u8],
    ) -> Result<fastfile_t6::ZoneStream<'a>, fastfile_t6::ZoneError> {
        let [b0, b1, b2, b3, b4, b5, b6, b7] = &mut self.blocks;
        let blocks: [&mut [u8]; fastfile_t6::MAX_XFILE_COUNT] = [
            b0.as_mut_slice(),
            b1.as_mut_slice(),
            b2.as_mut_slice(),
            b3.as_mut_slice(),
            b4.as_mut_slice(),
            b5.as_mut_slice(),
            b6.as_mut_slice(),
            b7.as_mut_slice(),
        ];
        fastfile_t6::ZoneStream::new(image, blocks, &mut self.insert_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_foreign_files_are_refused() {
        assert!(matches!(
            inflate_image(&[0; 4], None),
            Err(ZoneOpenError::Truncated)
        ));
        let mut foreign = [0u8; 16];
        foreign[8..12].copy_from_slice(&0x1234u32.to_le_bytes());
        assert!(matches!(
            inflate_image(&foreign, None),
            Err(ZoneOpenError::UnsupportedVersion { got: 0x1234 })
        ));
    }

    #[test]
    fn a_missing_file_names_its_path() {
        let err = open_zone("no/such/zone.ff").err().expect("must fail");
        assert!(err.to_string().starts_with("no/such/zone.ff: "), "{err}");
        assert!(matches!(
            open_zone_shared("no/such/zone.ff"),
            Err(ZoneOpenError::AtPath { .. })
        ));
    }

    #[test]
    fn a_record_that_never_ends_is_an_error() {
        // A stored (uncompressed) deflate block that is not the final one.
        let open_stream = [0x00, 0x01, 0x00, 0xfe, 0xff, 0xaa];
        let mut out = vec![0u8; 64];
        let mut inflater = flate2::Decompress::new(false);
        assert!(matches!(
            inflate_record(&mut inflater, &open_stream, &mut out, 3),
            Err(ZoneOpenError::Inflate { chunk: 3, .. })
        ));
    }
}
