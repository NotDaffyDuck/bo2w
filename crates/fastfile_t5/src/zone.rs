//! A BO1 zone image bound to the memory it would be unpacked into.

/// Blocks a zone unpacks into.
pub const ZONE_BLOCK_COUNT: usize = 7;

/// The block for data the engine builds at load time.
pub const ARCHIVE_SEGMENT_LIVE: usize = 1;

/// The block whose pointer slots the insert map tracks.
const ARCHIVE_SEGMENT_HEAPED: usize = 4;

/// One pointer slot in the virtual block.
const POINTER_BYTES: usize = 4;

/// Two sizes, then the size of each block.
const HEADER_LEN: usize = 8 + 4 * ZONE_BLOCK_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZonePreamble {
    pub block_lens: [u32; ZONE_BLOCK_COUNT],
}

#[derive(Debug, PartialEq, Eq)]
pub enum ZoneLayoutError {
    Truncated { image_needs: usize, len: usize },
    BlockTooSmall { which: usize, want: usize, holds: usize },
    InsertionMapUndersized { map_needs: usize, map_has: usize },
}

/// Reads the little-endian word at `at` of `bytes`.
fn word_at(image: &[u8], at: usize) -> u32 {
    let chunk: [u8; 4] = image[at..at + 4].try_into().expect("a header holds every block size");
    u32::from_le_bytes(chunk)
}

impl ZonePreamble {
    pub fn parse(image: &[u8]) -> Result<ZonePreamble, ZoneLayoutError> {
        let len = image.len();
        if len < HEADER_LEN {
            return Err(ZoneLayoutError::Truncated { image_needs: HEADER_LEN, len });
        }
        // The block sizes follow the two leading sizes, one word each.
        let block_lens = core::array::from_fn(|block| word_at(image, 8 + 4 * block));
        Ok(ZonePreamble { block_lens })
    }

    /// The first block whose memory is shorter than the header asks for.
    fn short_block(&self, blocks: &[&mut [u8]]) -> Option<ZoneLayoutError> {
        let wants = self.block_lens.iter().map(|&size| size as usize);
        let mut sized = wants.zip(blocks.iter().map(|memory| memory.len())).enumerate();
        sized
            .find(|&(_, (want, have))| have < want)
            .map(|(which, (want, holds))| ZoneLayoutError::BlockTooSmall { which, want, holds })
    }
}

/// A zone's bytes past its header, with the blocks they unpack into.
pub struct ZoneByteReader<'a> {
    pub header: ZonePreamble,
    pub body: &'a [u8],
    pub blocks: [&'a mut [u8]; ZONE_BLOCK_COUNT],
    pub patch_byte_map: &'a mut [u8],
}

impl<'a> ZoneByteReader<'a> {
    /// Bytes of insert map a zone with this header needs: one bit per pointer
    /// slot of the virtual block, and a spare byte.
    pub fn patch_byte_map_size(header: &ZonePreamble) -> usize {
        let pointer_slots = header.block_lens[ARCHIVE_SEGMENT_HEAPED] as usize / POINTER_BYTES;
        1 + pointer_slots.div_ceil(8)
    }

    pub fn new(
        image: &'a [u8],
        buffers: [&'a mut [u8]; ZONE_BLOCK_COUNT],
        patch_byte_map: &'a mut [u8],
    ) -> Result<ZoneByteReader<'a>, ZoneLayoutError> {
        let header = ZonePreamble::parse(image)?;
        if let Some(short) = header.short_block(&buffers) {
            return Err(short);
        }
        let map_bytes = Self::patch_byte_map_size(&header);
        let map_has = patch_byte_map.len();
        if map_has < map_bytes {
            return Err(ZoneLayoutError::InsertionMapUndersized { map_needs: map_bytes, map_has });
        }
        // Every pointer slot starts out as not yet inserted.
        patch_byte_map.iter_mut().for_each(|bit_byte| *bit_byte = 0);
        let body = &image[HEADER_LEN..];
        Ok(ZoneByteReader { header, body, blocks: buffers, patch_byte_map })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(sizes: [u32; ZONE_BLOCK_COUNT]) -> [u8; HEADER_LEN] {
        let mut raw = [0u8; HEADER_LEN];
        for (i, size) in sizes.iter().enumerate() {
            raw[8 + i * 4..12 + i * 4].copy_from_slice(&size.to_le_bytes());
        }
        raw
    }

    #[test]
    fn header_reads_block_sizes() {
        let header = ZonePreamble::parse(&image([1, 2, 3, 4, 5, 6, 7])).unwrap();
        assert_eq!(header.block_lens, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn short_image_is_truncated() {
        assert_eq!(
            ZonePreamble::parse(&[0u8; 10]),
            Err(ZoneLayoutError::Truncated { image_needs: HEADER_LEN, len: 10 })
        );
    }

    #[test]
    fn insert_map_covers_virtual_slots() {
        let header = ZonePreamble { block_lens: [0, 0, 0, 0, 64, 0, 0] };
        assert_eq!(ZoneByteReader::patch_byte_map_size(&header), 3);
    }

    #[test]
    fn stream_refuses_small_block_and_clears_map() {
        let bytes = image([0, 8, 0, 0, 0, 0, 0]);
        let mut spare = [[0u8; 4]; ZONE_BLOCK_COUNT];
        let mut map = [0xffu8; 4];
        let [a, b, c, d, e, f, g] = &mut spare;
        let blocks = [&mut a[..], &mut b[..], &mut c[..], &mut d[..], &mut e[..], &mut f[..], &mut g[..]];
        let err = ZoneByteReader::new(&bytes, blocks, &mut map).err();
        assert_eq!(err, Some(ZoneLayoutError::BlockTooSmall { which: 1, want: 8, holds: 4 }));

        let mut big = [[0u8; 8]; ZONE_BLOCK_COUNT];
        let [a, b, c, d, e, f, g] = &mut big;
        let blocks = [&mut a[..], &mut b[..], &mut c[..], &mut d[..], &mut e[..], &mut f[..], &mut g[..]];
        let stream = ZoneByteReader::new(&bytes, blocks, &mut map).unwrap();
        assert!(stream.patch_byte_map.iter().all(|&b| b == 0));
        assert!(stream.body.is_empty());
    }
}
