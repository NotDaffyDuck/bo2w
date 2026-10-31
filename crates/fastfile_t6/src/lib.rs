//! bo2zm: retail T6 (Black Ops II, PC) zone reading. Not part of upstream
//! the old engine; added by the bo2zm fork.
//!
//! The file is a 12-byte preamble, an auth header on signed zones
//! (`ff_header`), then XChunk records that decrypt and inflate into the zone
//! image (`xchunk`; inflating is the transport's job, like the other lanes).
//! The image is an XFile header and the content stream (`zone_core`), which
//! starts with the XAssetList (`asset_dir`).

#![no_std]
#![forbid(unsafe_code)]

mod xasset_kind;
mod asset_dir;
mod ff_header;
mod salsa20;
mod sha1;
mod zone_walk;
#[rustfmt::skip]
mod walk_gen;
/// Generated T6 structure layouts: `layout::GfxImage::name` is a byte offset.
#[rustfmt::skip]
pub mod layout_gen;
pub use layout_gen as layout;
mod xchunk;
mod zone_core;

pub use xasset_kind::{ASSET_TYPE_COUNT, XAssetKind};
pub use asset_dir::{AssetDirectory, StringList, RESOURCE_RECORD_SIZE, ASSET_TABLE_HEAD_BYTES, read_asset_directory};
pub use ff_header::{
    AUTH_HEADER_LEN, AUTH_MAGIC, AUTH_NAME_LEN, AUTH_SIGNATURE_LEN, LEADING_BYTES_SIZE, ArchivePreamble,
    LeadingBytesFault, SIGNED_ZONE_TAG, UNSIGNED_ZONE_TAG, FF_TAG_SERVER, AuthSealState,
    ZONE_NAME_KEY_MAX, T6_PC_ZONE_REV, is_t6_magic, read_ff_preamble,
};
pub use zone_walk::{Loaded, NoSink, WalkSink, Walker};
pub use xchunk::{
    ChunkCipher, HASH_TABLE_LEN, SALSA20_KEY_PC, STREAM_COUNT, VANILLA_BUFFER_SIZE, XCHUNK_SIZE,
    XChunk, XChunkError, XChunks,
};
pub use zone_core::{
    SEGMENT_DEPTH_LIMIT, BLOCK_TYPES, SegmentKind, MAX_ZONE_FILE_TOTAL, SLOT_BYTES, RawRef, Result, LoadResult,
    XFILE_BLOCK_DELAY_PHYSICAL, XFILE_BLOCK_DELAY_VIRTUAL, XFILE_BLOCK_NAMES, ARCHIVE_SEGMENT_HARDWARE,
    XFILE_BLOCK_RUNTIME_PHYSICAL, XFILE_BLOCK_RUNTIME_VIRTUAL, XFILE_BLOCK_STREAMER_RESERVE,
    ARCHIVE_SEGMENT_SCRATCH, ARCHIVE_SEGMENT_HEAPED, ZONE_HEAD_BYTES, ZoneReadFault, ZonePreamble, StreamRef,
    ZoneByteReader, segment_allows_sharing, read_zone_head,
};
