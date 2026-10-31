//! Black Ops II image packs on disk: open their indexes, find a GfxImage's
//! entry, read and decode it. Packs are only read.

use std::fs::File;
use std::io::{Seek as _, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ipak_t6::{IndexEntry, decode_entry, parse_header, parse_index};

use crate::zone_grab::ImageRef;
use std::io::SeekFrom;

pub struct Pack {
    pub name: String,
    pub path: PathBuf,
    data_offset: u64,
    index: Vec<IndexEntry>,
    file: Mutex<File>,
}

impl Pack {
    pub fn open(path: &Path) -> Result<Pack, String> {
        let mut file = File::open(path).map_err(|e| format!("{shown}: {e}", shown = path.display()))?;
        let head = read_at(&mut file, 0, 256)?;
        let header = parse_header(&head).map_err(|e| format!("{shown}: {e}", shown = path.display()))?;
        let bytes = read_at(
            &mut file,
            header.index.starts_at as u64,
            header.index.item_count as usize * ipak_t6::INDEX_ENTRY_LEN,
        )?;
        let index = parse_index(&bytes, header.index.item_count as usize)
            .map_err(|e| format!("{shown}: {e}", shown = path.display()))?;
        Ok(Pack {
            name: stem_of(path),
            path: path.to_owned(),
            data_offset: header.data.starts_at as u64,
            index,
            file: Mutex::new(file),
        })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn entry_for(&self, label_digest: u32, data_hash: u32) -> Option<IndexEntry> {
        let key = IndexEntry::key_of(label_digest, data_hash);
        self.index
            .binary_search_by_key(&key, |e| e.lookup_key)
            .ok()
            .map(|i| self.index[i])
    }

    /// Every entry stored under an image name's hash (any data hash).
    pub fn find_name(&self, label_digest: u32) -> Vec<IndexEntry> {
        let lo = IndexEntry::key_of(label_digest, 0);
        let start = self.index.partition_point(|e| e.lookup_key < lo);
        self.index[start..].iter().take_while(|e| e.label_digest() == label_digest).copied().collect()
    }

    /// The decoded entry: an IWI v27 file.
    pub fn read(&self, entry: IndexEntry) -> Result<Vec<u8>, String> {
        let at = self.data_offset + entry.data_at as u64;
        let stored = {
            let mut file = self
                .file
                .lock()
                .map_err(|_| "pack file lock poisoned".to_owned())?;
            read_at(&mut file, at, entry.stored_len as usize)?
        };
        let mut out = Vec::new();
        decode_entry(&stored, at, &mut out).map_err(|e| format!("{}: {e}", self.name))?;
        Ok(out)
    }
}

fn read_at(file: &mut File, at: u64, len: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    let mut buf = vec![0_u8; len];
    let mut got = 0;
    while got < len {
        match file.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) => return Err(e.to_string()),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// Every pack of an install's zone folder, searched in priority order:
/// patches and map packs before the shared ones.
pub struct PackSet {
    pub packs: Vec<Pack>,
}

/// Where an image's pixels are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// In a pack: which pack (index into `PackSet::packs`) and entry.
    Pack(usize, IndexEntry),
    /// In the zone itself.
    Embedded,
    /// Not streamed and not embedded: nothing to draw.
    Empty,
}

impl PackSet {
    pub fn open_dir(dir: &Path) -> Result<PackSet, String> {
        let listing = std::fs::read_dir(dir).map_err(|e| format!("{shown}: {e}", shown = dir.display()))?;
        let mut paths: Vec<PathBuf> = listing
            .flatten()
            .map(|entry| entry.path())
            .filter(|p| has_ipak_extension(p))
            .collect();
        paths.sort_by_cached_key(|p| priority(p));
        // bo2zm: then the extra folders' packs (`BO2MP_T6_EXTRA`), after the install's.
        let mut extra = asset_transport::t6_extra::extra_files("ipak");
        extra.sort_by_cached_key(|p| priority(p));
        paths.extend(extra);
        let packs = paths
            .iter()
            .map(|p| Pack::open(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PackSet { packs })
    }

    /// An image by its name alone (a zone's `,name` reference to an image
    /// another zone defines): the first pack holding that name, in
    /// priority order, and its largest entry (the full mip chain).
    pub fn locate_name(&self, name: &str) -> Option<(usize, IndexEntry)> {
        let h = ipak_t6::hash_name(name.trim_start_matches(','));
        self.packs
            .iter()
            .enumerate()
            .find_map(|(i, p)| p.find_name(h).into_iter().max_by_key(|e| e.stored_len).map(|e| (i, e)))
    }

    pub fn locate(&self, image: &ImageRef) -> Option<ImageSource> {
        if image.embedded.is_some() {
            return Some(ImageSource::Embedded);
        }
        if image.streamed_parts == 0 {
            return Some(ImageSource::Empty);
        }
        self.packs.iter().enumerate().find_map(|(i, p)| {
            p.entry_for(image.label_digest, image.part_hash)
                .map(|e| ImageSource::Pack(i, e))
        })
    }
}

/// The file name without its extension (empty when there is none).
pub(crate) fn stem_of(path: &Path) -> String {
    path.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

fn has_ipak_extension(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ipak"))
}

/// Lower sorts first: zone patches, then map packs, then shared packs.
fn priority(path: &Path) -> (u8, String) {
    let stem = stem_of(path).to_ascii_lowercase();
    // First rule that fits wins; a pack none of them names ranks 4.
    let rules: [(fn(&str) -> bool, u8); 5] = [
        (|n| n.ends_with("_patch"), 0),
        (|n| ["zm_", "mp_"].iter().any(|p| n.starts_with(p)), 1),
        (|n| n.starts_with("patch"), 2),
        (|n| n.starts_with("dlc"), 3),
        (|n| n == "base", 5),
    ];
    let rank = rules.iter().find(|(fits, _)| fits(&stem)).map_or(4, |&(_, rank)| rank);
    (rank, stem)
}

#[cfg(test)]
mod priority_tests {
    use super::*;

    fn rank(name: &str) -> (u8, String) {
        priority(Path::new(name))
    }

    #[test]
    fn pack_priority_orders_patches_then_maps_then_shared() {
        assert_eq!(rank("a/Zm_Transit_Patch.ipak").0, 0);
        assert_eq!(rank("zm_transit.ipak").0, 1);
        assert_eq!(rank("mp_nuked.ipak").0, 1);
        assert_eq!(rank("patch_mp.ipak").0, 2);
        assert_eq!(rank("dlc1_zm.ipak").0, 3);
        assert_eq!(rank("common.ipak").0, 4);
        assert_eq!(rank("base.ipak"), (5, "base".to_owned()));
        assert_eq!(rank("mp_x_patch.ipak").0, 0);
        assert_eq!(rank("").0, 4);
    }
}
