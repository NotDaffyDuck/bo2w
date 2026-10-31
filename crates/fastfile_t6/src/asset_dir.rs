//! The XAssetList at the start of a T6 zone's content.
//!
//! Layout (24 bytes, read off the stream into no block): script-string count
//! and pointer, dependency count and pointer, asset count and pointer. With
//! the VIRTUAL block pushed, the three arrays follow in that order: the
//! script-string pointer array then each string that follows inline, the
//! dependency-name pointer array then its strings, then the XAsset array
//! (u32 pool id + u32 header pointer per entry). The asset bodies come after
//! that, in array order.

use crate::xasset_kind::XAssetKind;
use crate::zone_core::{RawRef, LoadResult, ARCHIVE_SEGMENT_HEAPED, ZoneReadFault, StreamRef, ZoneByteReader};

pub const ASSET_TABLE_HEAD_BYTES: usize = 24;
pub const RESOURCE_RECORD_SIZE: usize = 8;

/// An array of string pointers loaded into the VIRTUAL block, with every
/// entry rewritten to an offset (or 0 for a null string).
#[derive(Clone, Copy, Debug, Default)]
pub struct StringList {
    ptr_run: Option<RawRef>,
    len_items: usize,
}

impl StringList {
    pub fn count(&self) -> usize {
        self.len_items
    }

    pub fn ptr_list(&self) -> Option<RawRef> {
        self.ptr_run
    }

    pub fn get<'zr>(&self, s: &'zr ZoneByteReader<'_>, i: usize) -> Option<&'zr str> {
        let base = self.ptr_run.filter(|_| i < self.len_items)?;
        match s.address_for(base, i * 4) {
            Ok(StreamRef::Null) => Some(""),
            Ok(StreamRef::At(at)) => s.zero_ended_str(s.follow_entry_link(at)).ok(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AssetDirectory {
    pub strings: StringList,
    pub depends: StringList,
    table_at: Option<RawRef>,
    n_records: usize,
}

impl AssetDirectory {
    pub fn count(&self) -> usize {
        self.n_records
    }

    pub fn array(&self) -> Option<RawRef> {
        self.table_at
    }

    pub fn raw_kind(&self, s: &ZoneByteReader<'_>, i: usize) -> LoadResult<u32> {
        let base = self.table_at.ok_or(ZoneReadFault::NothingPushedYet)?;
        (i < self.n_records)
            .then(|| s.uint_from_bytes(base, i * RESOURCE_RECORD_SIZE))
            .unwrap_or(Err(ZoneReadFault::OffsetPastBlock {
                block: ARCHIVE_SEGMENT_HEAPED,
                offset: i,
                size: self.n_records,
            }))
    }

    pub fn asset_type_at(&self, s: &ZoneByteReader<'_>, ix: usize) -> LoadResult<XAssetKind> {
        self.raw_kind(s, ix)
            .and_then(|id| XAssetKind::from_u32(id).ok_or(ZoneReadFault::UnrecognizedResourceKind(id)))
    }

    /// The header pointer slot of asset `asset_no` (where its body pointer lives).
    pub fn bay(&self, asset_no: usize) -> Option<RawRef> {
        self.table_at
            .filter(|_| asset_no < self.n_records)
            .map(|base| base.shifted_by(RESOURCE_RECORD_SIZE * asset_no + 4))
    }
}

/// Whether the head's pointer word says the array follows inline; a count with
/// a null pointer means nothing was written.
fn following_list(raw: u32, what: &'static str) -> LoadResult<bool> {
    let ptr = StreamRef::from_word(raw);
    if ptr.is_null_word() {
        Ok(false)
    } else if ptr.is_trailing() {
        Ok(true)
    } else {
        Err(ZoneReadFault::NotFollowing { what, raw })
    }
}

/// Read the XAssetList head and its three arrays. Leaves the VIRTUAL block
/// pushed: the asset bodies load under it, and the walk pops it at the end.
pub fn read_asset_directory(s: &mut ZoneByteReader<'_>) -> LoadResult<AssetDirectory> {
    let mut hdr_u32s = [0u32; ASSET_TABLE_HEAD_BYTES / 4];
    for (word, chunk) in hdr_u32s.iter_mut().zip(s.pull(ASSET_TABLE_HEAD_BYTES)?.chunks_exact(4)) {
        *word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    let [n_strs, strings_raw, depend_count, depends_raw, n_xassets, assets_raw] = hdr_u32s;

    s.put(ARCHIVE_SEGMENT_HEAPED)?;

    let mut lists = [StringList::default(); 2];
    let sources = [
        (strings_raw, "script string list", n_strs),
        (depends_raw, "dependency list", depend_count),
    ];
    for (slot, (raw, what, count)) in lists.iter_mut().zip(sources) {
        if following_list(raw, what)? {
            *slot = load_string_list(s, count as usize)?;
        }
    }

    let mut table = AssetDirectory {
        strings: lists[0],
        depends: lists[1],
        table_at: None,
        n_records: 0,
    };
    if following_list(assets_raw, "asset list")? {
        let n = n_xassets as usize;
        table.table_at = Some(s.reserve_for_read(4, RESOURCE_RECORD_SIZE * n)?);
        table.n_records = n;
    }
    Ok(table)
}

fn load_string_list(s: &mut ZoneByteReader<'_>, count: usize) -> LoadResult<StringList> {
    let array = s.reserve_for_read(4, 4 * count)?;
    for i in 0..count {
        let slot = array.shifted_by(i * 4);
        let target = match s.address_for(array, i * 4)? {
            StreamRef::Null => 0,
            StreamRef::At(prior) => {
                s.record_displacement(prior);
                StreamRef::pack_displacement(s.follow_entry_link(prior))
            }
            _ => {
                s.open_payload(slot)?;
                StreamRef::pack_displacement(s.read_text_value()?)
            }
        };
        s.store_u32_at(array, i * 4, target)?;
    }
    Ok(StringList {
        ptr_run: Some(array),
        len_items: count,
    })
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::zone_core::{MAX_ZONE_FILE_TOTAL, ZONE_HEAD_BYTES};
    use alloc::vec;
    use alloc::vec::Vec;

    /// A zone image with two script strings (one inline, one null), one
    /// dependency and two assets.
    fn image() -> Vec<u8> {
        let mut out = vec![0u8; ZONE_HEAD_BYTES];
        // VIRTUAL block size.
        out[8 + 4 * ARCHIVE_SEGMENT_HEAPED..12 + 4 * ARCHIVE_SEGMENT_HEAPED]
            .copy_from_slice(&256u32.to_le_bytes());
        let w = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        // XAssetList head.
        w(&mut out, 2);
        w(&mut out, u32::MAX);
        w(&mut out, 1);
        w(&mut out, u32::MAX);
        w(&mut out, 2);
        w(&mut out, u32::MAX);
        // Script strings: pointer array, then the one that follows.
        w(&mut out, u32::MAX);
        w(&mut out, 0);
        out.extend_from_slice(b"zombie\0");
        // Dependencies.
        w(&mut out, u32::MAX);
        out.extend_from_slice(b"common_zm\0");
        // Assets: rawfile, zbarrier.
        w(&mut out, XAssetKind::RawFile as u32);
        w(&mut out, u32::MAX);
        w(&mut out, XAssetKind::ZBarrier as u32);
        w(&mut out, u32::MAX);
        out
    }

    #[test]
    fn asset_list_reads_strings_depends_and_kinds() {
        let image = image();
        let mut storage: [Vec<u8>; MAX_ZONE_FILE_TOTAL] = core::array::from_fn(|_| vec![0u8; 256]);
        let mut patch_byte_map = vec![0u8; 16];
        let blocks = {
            let [a, b, c, d, e, f, g, h] = &mut storage;
            [
                a.as_mut_slice(),
                b.as_mut_slice(),
                c.as_mut_slice(),
                d.as_mut_slice(),
                e.as_mut_slice(),
                f.as_mut_slice(),
                g.as_mut_slice(),
                h.as_mut_slice(),
            ]
        };
        let mut s = ZoneByteReader::new(&image, blocks, &mut patch_byte_map).unwrap();
        let table = read_asset_directory(&mut s).unwrap();
        assert_eq!(table.strings.count(), 2);
        assert_eq!(table.strings.get(&s, 0), Some("zombie"));
        assert_eq!(table.strings.get(&s, 1), Some(""));
        assert_eq!(table.depends.get(&s, 0), Some("common_zm"));
        assert_eq!(table.count(), 2);
        assert_eq!(table.asset_type_at(&s, 0), Ok(XAssetKind::RawFile));
        assert_eq!(table.asset_type_at(&s, 1), Ok(XAssetKind::ZBarrier));
        assert!(table.bay(0).is_some() && table.bay(1).is_some());
        assert!(table.bay(2).is_none());
        assert!(matches!(table.raw_kind(&s, 2), Err(ZoneReadFault::OffsetPastBlock { offset: 2, size: 2, .. })));
        assert!(AssetDirectory::default().bay(0).is_none());
        assert_eq!(s.remaining(), 0);
    }
}
