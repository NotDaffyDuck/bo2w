//! A BO1 zone image bound to the memory it would be unpacked into.

/// Blocks a zone unpacks into.
pub const MAX_XFILE_COUNT: usize = 7;

/// The block for data the engine builds at load time.
pub const XFILE_BLOCK_RUNTIME: usize = 1;

/// The block whose pointer slots the insert map tracks.
const XFILE_BLOCK_VIRTUAL: usize = 4;

/// One pointer slot in the virtual block.
const POINTER_BYTES: usize = 4;

/// Two sizes, then the size of each block.
const HEADER_LEN: usize = 8 + 4 * MAX_XFILE_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneHeader {
    pub block_size: [u32; MAX_XFILE_COUNT],
}

#[derive(Debug, PartialEq, Eq)]
pub enum ZoneError {
    Truncated { needed: usize, len: usize },
    BlockTooSmall { block: usize, want: usize, have: usize },
    InsertMapTooSmall { needed: usize, have: usize },
}

impl ZoneHeader {
    pub fn parse(image: &[u8]) -> Result<ZoneHeader, ZoneError> {
        let sizes = image.get(8..HEADER_LEN).ok_or(ZoneError::Truncated {
            needed: HEADER_LEN,
            len: image.len(),
        })?;
        let mut block_size = [0u32; MAX_XFILE_COUNT];
        for (size, bytes) in block_size.iter_mut().zip(sizes.chunks_exact(4)) {
            *size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        Ok(ZoneHeader { block_size })
    }
}

/// A zone's bytes past its header, with the blocks they unpack into.
pub struct ZoneStream<'a> {
    pub header: ZoneHeader,
    pub body: &'a [u8],
    pub blocks: [&'a mut [u8]; MAX_XFILE_COUNT],
    pub insert_map: &'a mut [u8],
}

impl<'a> ZoneStream<'a> {
    /// Bytes of insert map a zone with this header needs: one bit per pointer
    /// slot of the virtual block, and a spare byte.
    pub fn insert_map_len(header: &ZoneHeader) -> usize {
        let slots = header.block_size[XFILE_BLOCK_VIRTUAL] as usize / POINTER_BYTES;
        slots.div_ceil(8) + 1
    }

    pub fn new(
        image: &'a [u8],
        blocks: [&'a mut [u8]; MAX_XFILE_COUNT],
        insert_map: &'a mut [u8],
    ) -> Result<ZoneStream<'a>, ZoneError> {
        let header = ZoneHeader::parse(image)?;
        for (block, (memory, &size)) in blocks.iter().zip(&header.block_size).enumerate() {
            if memory.len() < size as usize {
                return Err(ZoneError::BlockTooSmall {
                    block,
                    want: size as usize,
                    have: memory.len(),
                });
            }
        }
        let needed = Self::insert_map_len(&header);
        if insert_map.len() < needed {
            return Err(ZoneError::InsertMapTooSmall {
                needed,
                have: insert_map.len(),
            });
        }
        insert_map.fill(0);
        Ok(ZoneStream {
            header,
            body: &image[HEADER_LEN..],
            blocks,
            insert_map,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(sizes: [u32; MAX_XFILE_COUNT]) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        for (i, size) in sizes.iter().enumerate() {
            bytes[8 + i * 4..12 + i * 4].copy_from_slice(&size.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn header_reads_block_sizes() {
        let header = ZoneHeader::parse(&image([1, 2, 3, 4, 5, 6, 7])).unwrap();
        assert_eq!(header.block_size, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn short_image_is_truncated() {
        assert_eq!(
            ZoneHeader::parse(&[0u8; 10]),
            Err(ZoneError::Truncated { needed: HEADER_LEN, len: 10 })
        );
    }

    #[test]
    fn insert_map_covers_virtual_slots() {
        let header = ZoneHeader { block_size: [0, 0, 0, 0, 64, 0, 0] };
        assert_eq!(ZoneStream::insert_map_len(&header), 3);
    }

    #[test]
    fn stream_refuses_small_block_and_clears_map() {
        let bytes = image([0, 8, 0, 0, 0, 0, 0]);
        let mut spare = [[0u8; 4]; MAX_XFILE_COUNT];
        let mut map = [0xffu8; 4];
        let [a, b, c, d, e, f, g] = &mut spare;
        let blocks = [&mut a[..], &mut b[..], &mut c[..], &mut d[..], &mut e[..], &mut f[..], &mut g[..]];
        let err = ZoneStream::new(&bytes, blocks, &mut map).err();
        assert_eq!(err, Some(ZoneError::BlockTooSmall { block: 1, want: 8, have: 4 }));

        let mut big = [[0u8; 8]; MAX_XFILE_COUNT];
        let [a, b, c, d, e, f, g] = &mut big;
        let blocks = [&mut a[..], &mut b[..], &mut c[..], &mut d[..], &mut e[..], &mut f[..], &mut g[..]];
        let stream = ZoneStream::new(&bytes, blocks, &mut map).unwrap();
        assert!(stream.insert_map.iter().all(|&b| b == 0));
        assert!(stream.body.is_empty());
    }
}
