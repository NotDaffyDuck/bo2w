//! The inflated T6 zone image: XFile header, eight blocks, pointer fixups.
//!
//! bo2zm: the stream is fastfile_t5's `ZoneByteReader` cut down to the parts the
//! T6 walk uses so far, with T6's block table. Per-asset geometry captures get
//! added beside the readers that fill them.
//!
//! Pointers keep T5's encoding: 3 block bits over a 29-bit offset, biased by
//! one, with -1 = data follows in the stream and -2 = data follows and the
//! pointer is aliased through a slot in the insert block (VIRTUAL).

pub const MAX_ZONE_FILE_TOTAL: usize = 8;

pub const ARCHIVE_SEGMENT_SCRATCH: usize = 0;
pub const XFILE_BLOCK_RUNTIME_VIRTUAL: usize = 1;
pub const XFILE_BLOCK_RUNTIME_PHYSICAL: usize = 2;
pub const XFILE_BLOCK_DELAY_VIRTUAL: usize = 3;
pub const XFILE_BLOCK_DELAY_PHYSICAL: usize = 4;
pub const ARCHIVE_SEGMENT_HEAPED: usize = 5;
pub const ARCHIVE_SEGMENT_HARDWARE: usize = 6;
pub const XFILE_BLOCK_STREAMER_RESERVE: usize = 7;

pub const XFILE_BLOCK_NAMES: [&str; MAX_ZONE_FILE_TOTAL] = [
    "temp",
    "runtime_virtual",
    "runtime_physical",
    "delay_virtual",
    "delay_physical",
    "virtual",
    "physical",
    "streamer_reserve",
];

pub const SLOT_BYTES: usize = 4;

/// A stored zone pointer packs `block << OFFSET_BITS | offset`, biased by one
/// so that zero stays free for "no pointer".
const BLOCK_BITS: u32 = 3;
const OFFSET_BITS: u32 = u32::BITS - BLOCK_BITS;
const OFFSET_MASK: u32 = (1 << OFFSET_BITS) - 1;

/// The two reserved values at the top of the range.
const MARK_FOLLOWS: u32 = u32::MAX;
const MARK_INSERT: u32 = u32::MAX - 1;

pub const ZONE_HEAD_BYTES: usize = 8 + 4 * MAX_ZONE_FILE_TOTAL;

pub const SEGMENT_DEPTH_LIMIT: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentKind {
    /// Read from the stream; the offset rewinds when the block is popped.
    Temp,
    /// Read from the stream and kept.
    Normal,
    /// Reserved and zeroed; nothing is read from the stream.
    Runtime,
    /// Declared by the game but never filled during a load.
    Delay,
}

pub const BLOCK_TYPES: [SegmentKind; MAX_ZONE_FILE_TOTAL] = [
    SegmentKind::Temp,
    SegmentKind::Runtime,
    SegmentKind::Runtime,
    SegmentKind::Delay,
    SegmentKind::Delay,
    SegmentKind::Normal,
    SegmentKind::Normal,
    // The streamer reserve is reserved, not read: the Zombies Declassified
    // zones' `shaderOverflowBuffer` memory block (835,488 bytes of
    // streamMem) is followed at once by the next asset.
    SegmentKind::Runtime,
];

pub fn segment_allows_sharing(block: u8) -> bool {
    BLOCK_TYPES.get(usize::from(block)) == Some(&SegmentKind::Normal)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoneReadFault {
    ImageEndsEarly {
        from: usize,
        want: usize,
        image_len: usize,
    },
    SegmentOverrun {
        block: usize,
        end: usize,
        size: usize,
        request: usize,
    },
    CorruptSegment(usize),
    OffsetPastBlock {
        block: usize,
        offset: usize,
        size: usize,
    },
    DelayBlockLoad(usize),
    UnrecognizedResourceKind(u32),
    NoWalkForType(crate::xasset_kind::XAssetKind),
    TextWithoutEnd {
        block: usize,
    },
    NotUtf8,
    DepthExceeded,
    PopFromEmptyStack,
    NothingPushedYet,
    InsertionMapUndersized {
        needed: usize,
        got: usize,
    },
    /// A list pointer that the format says must be "follows" was something else.
    NotFollowing {
        what: &'static str,
        raw: u32,
    },
    /// A load did not start where the block's write position is: the walk
    /// and the zone disagree about a size somewhere before this point.
    NonContiguous {
        block: usize,
        at: usize,
        expected: usize,
    },
    /// A pointer value the format does not allow at this place.
    BadPointer {
        what: &'static str,
        raw: u32,
    },
    /// A count expression came out negative or absurd.
    BadCount(i64),
    DivideByZero,
    WalkTooDeep,
}

impl ZoneReadFault {
    /// The wording of the errors that carry nothing to splice in.
    fn plain_text(&self) -> Option<&'static str> {
        Some(match self {
            Self::NotUtf8 => "string bytes are not UTF-8",
            Self::DepthExceeded => "block stack is full",
            Self::PopFromEmptyStack => "block stack popped while empty",
            Self::NothingPushedYet => "nothing on the block stack",
            Self::DivideByZero => "count expression divided by zero",
            Self::WalkTooDeep => "zone walk recursed too far",
            _ => return None,
        })
    }
}

impl core::fmt::Display for ZoneReadFault {
    fn fmt(&self, sink: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(text) = self.plain_text() {
            return sink.write_str(text);
        }
        match self {
            Self::ImageEndsEarly { from, want, image_len } => {
                write!(sink, "zone image ends early: {want} bytes wanted from offset {from}, image is {image_len} long")
            }
            Self::SegmentOverrun { block, end, size, request } => {
                write!(sink, "block {block} is full: write reaches {end} but it holds {size} (asked for {request})")
            }
            Self::CorruptSegment(n) => write!(sink, "zone pointer names block {n}, which does not exist"),
            Self::OffsetPastBlock { block, offset, size } => {
                write!(sink, "block {block} has {size} bytes, offset {offset} is past them")
            }
            Self::DelayBlockLoad(n) => write!(sink, "delay block {n} was given data, but loads never fill it"),
            Self::UnrecognizedResourceKind(id) => write!(sink, "asset pool id {id:#x} is not a T6 asset type"),
            Self::NoWalkForType(kind) => write!(sink, "asset type `{}` has no T6 zone walk yet", kind.name()),
            Self::TextWithoutEnd { block } => write!(sink, "string in block {block} never ends"),
            Self::InsertionMapUndersized { needed, got } => {
                write!(sink, "insert-slot map holds {got} bytes, {needed} are required")
            }
            Self::NotFollowing { what, raw } => {
                write!(sink, "{what} pointer {raw:#x} should have been the data-follows marker")
            }
            Self::NonContiguous { block, at, expected } => {
                write!(sink, "load starts at {at:#x} in block {block}, but its write position is {expected:#x}")
            }
            Self::BadPointer { what, raw } => write!(sink, "{what} cannot hold pointer {raw:#x}"),
            Self::BadCount(n) => write!(sink, "count expression evaluated to {n}"),
            _ => unreachable!("plain_text covers the rest"),
        }
    }
}

pub type LoadResult<T> = core::result::Result<T, ZoneReadFault>;
pub type Result<T> = LoadResult<T>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct RawRef {
    pub seg: u8,
    pub offset: u32,
}

impl RawRef {
    pub fn shifted_by(self, delta: usize) -> RawRef {
        let moved = self.offset.wrapping_add(delta as u32);
        RawRef { seg: self.seg, offset: moved }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamRef {
    Null,
    Following,
    Insert,
    At(RawRef),
}

const NULL_WORD: u32 = 0;

impl StreamRef {
    pub fn from_word(word: u32) -> StreamRef {
        match word {
            NULL_WORD => StreamRef::Null,
            MARK_FOLLOWS => StreamRef::Following,
            MARK_INSERT => StreamRef::Insert,
            _ => {
                let unbiased = word - 1;
                let (block, offset) = (unbiased >> OFFSET_BITS, unbiased & OFFSET_MASK);
                StreamRef::At(RawRef { seg: block as u8, offset })
            }
        }
    }

    pub fn pack_displacement(p: RawRef) -> u32 {
        let block_part = u32::from(p.seg) << OFFSET_BITS;
        (p.offset & OFFSET_MASK).wrapping_add(block_part).wrapping_add(1)
    }

    pub fn is_trailing(self) -> bool {
        self == StreamRef::Following || self == StreamRef::Insert
    }

    pub fn is_null_word(self) -> bool {
        self == StreamRef::Null
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZonePreamble {
    pub size: u32,
    pub outside_extent: u32,
    pub block_size: [u32; MAX_ZONE_FILE_TOTAL],
}

pub fn read_zone_head(image: &[u8]) -> LoadResult<ZonePreamble> {
    // Header words are little-endian: size, external size, then one per block.
    const BIT_CHUNK_COUNT: usize = 2 + MAX_ZONE_FILE_TOTAL;
    let Some(head) = image.get(..ZONE_HEAD_BYTES) else {
        return Err(ZoneReadFault::ImageEndsEarly { from: 0, want: ZONE_HEAD_BYTES, image_len: image.len() });
    };
    let words: [u32; BIT_CHUNK_COUNT] = core::array::from_fn(|n| {
        let at = n * 4;
        u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]])
    });
    let [size, outside_extent, block_size @ ..] = words;
    Ok(ZonePreamble { size, outside_extent, block_size })
}

/// Round `v` up to a multiple of the power-of-two `align`, in 32-bit
/// arithmetic like the game's own allocator.
fn align_up(value: usize, align: usize) -> usize {
    debug_assert_eq!(align.count_ones(), 1);
    let slack = (align as u32).wrapping_sub(1);
    (value as u32).wrapping_add(slack) as usize & !slack as usize
}

/// One entry of the block stack: the block, and for a temp block the write
/// position to rewind to when the entry is popped.
#[derive(Clone, Copy)]
struct StackEntry {
    block: usize,
    rewind_to: Option<usize>,
}

impl StackEntry {
    const BLANK: StackEntry = StackEntry {
        block: 0,
        rewind_to: None,
    };
}

/// The inflated image and how far reading has got into it.
struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// Alias flags of the virtual block's pointer slots, and the slot reserved
/// for the body that is about to arrive, if any.
struct Inserts<'a> {
    flags: &'a mut [u8],
    waiting: Option<RawRef>,
}

/// Offsets that pointed past what had been written so far.
struct Unsettled {
    count: usize,
    first: Option<(RawRef, usize, usize)>,
}

pub struct ZoneByteReader<'a> {
    input: Input<'a>,
    seg_bufs: [&'a mut [u8]; MAX_ZONE_FILE_TOTAL],
    cursors: [usize; MAX_ZONE_FILE_TOTAL],
    stack: [StackEntry; SEGMENT_DEPTH_LIMIT],
    stack_len: usize,
    inserts: Inserts<'a>,
    unsettled: Unsettled,
    declared: ZonePreamble,
}

/// Byte and bit mask of `slot`'s alias flag in the insert map; only the
/// virtual block has flags.
fn alias_flag(bay: RawRef) -> Option<(usize, u8)> {
    let on_virtual = usize::from(bay.seg) == ARCHIVE_SEGMENT_HEAPED;
    on_virtual.then(|| {
        let n = bay.offset as usize / SLOT_BYTES;
        (n >> 3, 1u8 << (n & 7))
    })
}

impl<'a> ZoneByteReader<'a> {
    pub fn patch_byte_map_size(header: &ZonePreamble) -> usize {
        let pointer_slots = header.block_size[ARCHIVE_SEGMENT_HEAPED] as usize / SLOT_BYTES;
        1 + pointer_slots.div_ceil(8)
    }

    /// `seg_bufs[i]` must hold at least `header.block_size[i]` bytes. Delay
    /// blocks are never written and may be empty.
    pub fn new(
        image: &'a [u8],
        seg_bufs: [&'a mut [u8]; MAX_ZONE_FILE_TOTAL],
        patch_byte_map: &'a mut [u8],
    ) -> LoadResult<ZoneByteReader<'a>> {
        let sizes: [usize; MAX_ZONE_FILE_TOTAL] = core::array::from_fn(|i| seg_bufs[i].len());
        let header = Self::vetted_header(image, &sizes, patch_byte_map.len())?;
        for cell in patch_byte_map.iter_mut() {
            *cell = 0;
        }
        Ok(Self {
            declared: header,
            input: Input { bytes: image, at: ZONE_HEAD_BYTES },
            seg_bufs,
            inserts: Inserts { flags: patch_byte_map, waiting: None },
            cursors: [0; MAX_ZONE_FILE_TOTAL],
            stack: [StackEntry::BLANK; SEGMENT_DEPTH_LIMIT],
            stack_len: 0,
            unsettled: Unsettled { count: 0, first: None },
        })
    }

    /// The header of `image`, if the supplied block buffers and insert map
    /// are big enough for everything it declares.
    fn vetted_header(image: &[u8], sizes: &[usize; MAX_ZONE_FILE_TOTAL], map_len: usize) -> LoadResult<ZonePreamble> {
        read_zone_head(image).and_then(|header| {
            let short = (0..MAX_ZONE_FILE_TOTAL).find(|&i| BLOCK_TYPES[i] != SegmentKind::Delay && sizes[i] < header.block_size[i] as usize);
            if let Some(block) = short {
                let need_bytes = header.block_size[block] as usize;
                return Err(ZoneReadFault::SegmentOverrun { block, end: need_bytes, size: sizes[block], request: need_bytes });
            }
            match Self::patch_byte_map_size(&header) {
                needed if needed > map_len => Err(ZoneReadFault::InsertionMapUndersized { needed, got: map_len }),
                _ => Ok(header),
            }
        })
    }

    pub fn header(&self) -> &ZonePreamble {
        &self.declared
    }

    pub fn cursor(&self) -> usize {
        self.input.at
    }

    pub fn len(&self) -> usize {
        <[u8]>::len(self.input.bytes)
    }

    pub fn is_empty(&self) -> bool {
        self.input.bytes.first().is_none()
    }

    pub fn remaining(&self) -> usize {
        self.len().saturating_sub(self.input.at)
    }

    pub fn look_ahead(&self, max_len: usize) -> &[u8] {
        let tail = self.input.bytes.get(self.input.at..).unwrap_or_default();
        &tail[..max_len.min(tail.len())]
    }

    pub fn open_fixup_count(&self) -> usize {
        self.unsettled.count
    }

    pub fn finish(&self) -> LoadResult<()> {
        if self.remaining() > 0 {
            let total = self.input.bytes.len();
            return Err(ZoneReadFault::ImageEndsEarly { from: self.input.at, want: total, image_len: total });
        }
        Ok(())
    }

    pub fn put(&mut self, block: usize) -> LoadResult<()> {
        let kind = *BLOCK_TYPES.get(block).ok_or(ZoneReadFault::CorruptSegment(block))?;
        if self.stack_len == SEGMENT_DEPTH_LIMIT {
            return Err(ZoneReadFault::DepthExceeded);
        }
        let rewind_to = match kind {
            SegmentKind::Temp => Some(self.cursors[block]),
            _ => None,
        };
        self.stack[self.stack_len] = StackEntry { block, rewind_to };
        self.stack_len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> LoadResult<()> {
        if self.stack_len == 0 {
            return Err(ZoneReadFault::PopFromEmptyStack);
        }
        self.stack_len -= 1;
        let StackEntry { block, rewind_to } = self.stack[self.stack_len];
        if let Some(saved) = rewind_to {
            self.cursors[block] = saved;
        }
        Ok(())
    }

    /// The block allocations currently go to.
    pub fn top_block(&self) -> LoadResult<usize> {
        match self.stack[..self.stack_len].last() {
            Some(entry) => Ok(entry.block),
            None => Err(ZoneReadFault::NothingPushedYet),
        }
    }

    /// The block allocations go to, refusing blocks a load never fills.
    fn loadable_top(&self) -> LoadResult<usize> {
        let b = self.top_block()?;
        match BLOCK_TYPES[b] {
            SegmentKind::Delay => Err(ZoneReadFault::DelayBlockLoad(b)),
            _ => Ok(b),
        }
    }

    fn overflow(&self, b: usize, end: usize, request: usize) -> ZoneReadFault {
        let size = self.seg_bufs[b].len();
        ZoneReadFault::SegmentOverrun { block: b, end, size, request }
    }

    fn truncated(&self, needed: usize) -> ZoneReadFault {
        ZoneReadFault::ImageEndsEarly { from: self.input.at, want: needed, image_len: self.input.bytes.len() }
    }

    /// Take the next `size` bytes off the stream and move past them.
    pub(crate) fn pull(&mut self, size: usize) -> LoadResult<&'a [u8]> {
        let data: &'a [u8] = self.input.bytes;
        let from = self.input.at;
        let chunk = from
            .checked_add(size)
            .and_then(|to| data.get(from..to))
            .ok_or_else(|| self.truncated(size))?;
        self.input.at = from + size;
        Ok(chunk)
    }

    /// Move block `b`'s write position to the next `align` boundary and
    /// claim `size` bytes there; the claimed span starts at the return value.
    fn bump(&mut self, b: usize, align: usize, size: usize) -> LoadResult<usize> {
        let start = align_up(self.cursors[b], align);
        let end = start + size;
        if end > self.seg_bufs[b].len() {
            return Err(self.overflow(b, end, size));
        }
        self.cursors[b] = end;
        Ok(start)
    }

    /// Align the current block's write position and return it, reserving
    /// nothing: the bytes arrive through `load_bytes`, possibly in pieces.
    pub fn align_alloc(&mut self, align: usize) -> LoadResult<RawRef> {
        self.snap_position(align)?;
        let blk = self.top_block()?;
        let offset = self.cursors[blk] as u32;
        Ok(RawRef { seg: blk as u8, offset })
    }

    /// Fill `size` bytes at the current block's write position: from the
    /// stream for temp and normal blocks, zeros for runtime blocks.
    pub fn load_bytes(&mut self, size: usize) -> LoadResult<RawRef> {
        self.place(1, size)
    }

    /// Align, then fill `size` bytes there (see `load_bytes`), and advance
    /// the block's write position past them.
    fn place(&mut self, align: usize, size: usize) -> LoadResult<RawRef> {
        let blk_no = self.loadable_top()?;
        let from_at = align_up(self.cursors[blk_no], align);
        let end = from_at + size;
        if end > self.seg_bufs[blk_no].len() {
            return Err(self.overflow(blk_no, end, size));
        }
        match BLOCK_TYPES[blk_no] {
            SegmentKind::Runtime => self.seg_bufs[blk_no][from_at..end].fill(0),
            _ => {
                let src = self.pull(size)?;
                self.seg_bufs[blk_no][from_at..end].copy_from_slice(src);
            }
        }
        self.cursors[blk_no] = end;
        Ok(RawRef { seg: blk_no as u8, offset: from_at as u32 })
    }

    /// Point a reserved insert slot at `body` and mark it as an alias.
    pub fn bind_insert(&mut self, bay: RawRef, body: RawRef) -> LoadResult<()> {
        self.inserts.waiting = Some(bay);
        self.settle_pending(body)
    }

    pub fn snap_position(&mut self, align: usize) -> LoadResult<()> {
        let b = self.top_block()?;
        self.bump(b, align, 0).map(drop)
    }

    pub fn reserve_for_read(&mut self, align: usize, size: usize) -> LoadResult<RawRef> {
        let body = self.place(align, size)?;
        self.settle_pending(body)?;
        Ok(body)
    }

    pub fn read_text_value(&mut self) -> LoadResult<RawRef> {
        let blk_no = self.loadable_top()?;
        let from_at = self.write_pos(blk_no as u8);
        let room = self.seg_bufs[blk_no].len().saturating_sub(from_at);
        let tail = self.input.bytes.get(self.input.at..).unwrap_or_default();
        let reach = tail.len().min(room);
        let n = match tail[..reach].iter().position(|&c| c == 0) {
            Some(nul) => nul + 1,
            // No terminator where the block still has space: either the
            // block filled up first, or the stream ran dry.
            None if tail.len() >= room => return Err(self.overflow(blk_no, from_at + room, 0)),
            None => return Err(ZoneReadFault::TextWithoutEnd { block: blk_no }),
        };
        self.seg_bufs[blk_no][from_at..from_at + n].copy_from_slice(&tail[..n]);
        self.input.at += n;
        self.cursors[blk_no] = from_at + n;
        let stored = RawRef { seg: blk_no as u8, offset: from_at as u32 };
        self.settle_pending(stored)?;
        Ok(stored)
    }

    pub fn patch_reference_cell(&mut self) -> LoadResult<RawRef> {
        let at = self.bump(ARCHIVE_SEGMENT_HEAPED, SLOT_BYTES, SLOT_BYTES)?;
        Ok(RawRef { seg: ARCHIVE_SEGMENT_HEAPED as u8, offset: at as u32 })
    }

    /// Settle the insert slot waiting for a body, if there is one: it takes
    /// the body's address and is flagged as an alias.
    fn settle_pending(&mut self, body: RawRef) -> LoadResult<()> {
        let Some(slot) = self.inserts.waiting.take() else {
            return Ok(());
        };
        self.store_u32_at(slot, 0, StreamRef::pack_displacement(body))?;
        if let Some((byte, mask)) = alias_flag(slot) {
            if let Some(cell) = self.inserts.flags.get_mut(byte) {
                *cell |= mask;
            }
        }
        Ok(())
    }

    fn is_alias_slot(&self, spot: RawRef) -> bool {
        let flag = alias_flag(spot);
        flag.and_then(|(byte, mask)| self.inserts.flags.get(byte).map(|cell| cell & mask != 0)).unwrap_or(false)
    }

    pub fn follow_entry_link(&self, spot: RawRef) -> RawRef {
        if !self.is_alias_slot(spot) {
            return spot;
        }
        match self.address_for(spot, 0) {
            Ok(StreamRef::At(target)) => target,
            _ => spot,
        }
    }

    pub fn open_payload_patched(&mut self, bay: RawRef) -> LoadResult<(bool, Option<RawRef>)> {
        match self.address_for(bay, 0)? {
            StreamRef::At(placed) => {
                self.record_displacement(placed);
                Ok((false, None))
            }
            StreamRef::Insert => {
                let held = self.patch_reference_cell()?;
                let _ = self.inserts.waiting.replace(held);
                Ok((true, Some(held)))
            }
            other => Ok((other == StreamRef::Following, None)),
        }
    }

    pub fn open_payload(&mut self, bay: RawRef) -> LoadResult<bool> {
        self.open_payload_patched(bay).map(|(follows, _)| follows)
    }

    /// Check `len` bytes at `p + off` lie inside their block; give back the
    /// block and the start of the span within it.
    fn locate(&self, p: RawRef, off: usize, len: usize) -> LoadResult<(usize, usize)> {
        let b = usize::from(p.seg);
        let seg_buf = self.seg_bufs.get(b).ok_or(ZoneReadFault::CorruptSegment(b))?;
        let start = off + p.offset as usize;
        if seg_buf.len() < start + len {
            return Err(ZoneReadFault::OffsetPastBlock { block: b, offset: start, size: seg_buf.len() });
        }
        Ok((b, start))
    }

    fn array_at<const N: usize>(&self, p: RawRef, off: usize) -> LoadResult<[u8; N]> {
        let (b, start) = self.locate(p, off, N)?;
        let bytes = &self.seg_bufs[b][start..];
        Ok(core::array::from_fn(|i| bytes[i]))
    }

    pub fn u8_at(&self, p: RawRef, off: usize) -> LoadResult<u8> {
        self.array_at::<1>(p, off).map(|[v]| v)
    }

    pub fn byte_window_from(&self, p: RawRef, off: usize, len: usize) -> LoadResult<&[u8]> {
        let (b, start) = self.locate(p, off, len)?;
        Ok(&self.seg_bufs[b][start..start + len])
    }

    pub fn ushort_from_bytes(&self, p: RawRef, off: usize) -> LoadResult<u16> {
        self.array_at(p, off).map(u16::from_le_bytes)
    }

    pub fn short_from_bytes(&self, p: RawRef, off: usize) -> LoadResult<i16> {
        self.array_at(p, off).map(i16::from_le_bytes)
    }

    pub fn int_from_bytes(&self, p: RawRef, off: usize) -> LoadResult<i32> {
        self.array_at(p, off).map(i32::from_le_bytes)
    }

    pub fn uint_from_bytes(&self, p: RawRef, off: usize) -> LoadResult<u32> {
        self.array_at(p, off).map(u32::from_le_bytes)
    }

    pub fn float_from_bytes(&self, p: RawRef, off: usize) -> LoadResult<f32> {
        self.array_at(p, off).map(f32::from_le_bytes)
    }

    pub fn address_for(&self, p: RawRef, off: usize) -> LoadResult<StreamRef> {
        self.uint_from_bytes(p, off).map(StreamRef::from_word)
    }

    pub fn earliest_pending(&self) -> Option<(RawRef, usize, usize)> {
        self.unsettled.first
    }

    pub fn repair_cell(&mut self, bay: RawRef, body: RawRef) -> LoadResult<()> {
        let word = StreamRef::pack_displacement(body);
        self.store_u32_at(bay, 0, word)
    }

    /// Resolve the pointer stored at `parent + field`: an already-placed
    /// target is aliased, a "follows" marker has `load` pull the body out of
    /// the stream. Either way the slot ends up holding the body's address.
    fn follow(
        &mut self,
        owner: RawRef,
        field: usize,
        load: impl FnOnce(&mut Self) -> LoadResult<RawRef>,
    ) -> LoadResult<Option<RawRef>> {
        let slot = owner.shifted_by(field);
        let body = match self.address_for(owner, field)? {
            StreamRef::Null => None,
            StreamRef::At(target) => Some(self.noted_alias(target)),
            StreamRef::Following | StreamRef::Insert => {
                self.open_payload(slot)?;
                Some(load(self)?)
            }
        };
        body.map(|placed| self.repair_cell(slot, placed)).transpose()?;
        Ok(body)
    }

    fn noted_alias(&mut self, target: RawRef) -> RawRef {
        self.record_displacement(target);
        self.follow_entry_link(target)
    }

    pub fn walk_list_pointer(
        &mut self,
        owner: RawRef,
        field: usize,
        align: usize,
        elem_size: usize,
        count: usize,
    ) -> LoadResult<Option<RawRef>> {
        self.follow(owner, field, |zone| {
            zone.reserve_for_read(align, elem_size * count)
        })
    }

    pub fn walk_text_pointer(&mut self, owner: RawRef, field: usize) -> LoadResult<Option<RawRef>> {
        self.follow(owner, field, Self::read_text_value)
    }

    pub fn store_u32_at(&mut self, p: RawRef, off: usize, v: u32) -> LoadResult<()> {
        let (b, start) = self.locate(p, off, 4)?;
        self.seg_bufs[b][start..start + 4].copy_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn zero_ended_bytes(&self, p: RawRef) -> LoadResult<&[u8]> {
        let (b, start) = self.locate(p, 0, 1)?;
        let rest = &self.seg_bufs[b][start..];
        Ok(rest.split(|&c| c == 0).next().unwrap_or(rest))
    }

    pub fn zero_ended_str(&self, p: RawRef) -> LoadResult<&str> {
        self.zero_ended_bytes(p).and_then(|raw| core::str::from_utf8(raw).map_err(|_| ZoneReadFault::NotUtf8))
    }

    pub fn write_pos(&self, block: u8) -> usize {
        self.cursors.get(usize::from(block)).copied().unwrap_or(0)
    }

    pub fn segment_exceeded(&self, blk: u8) -> usize {
        let index = usize::from(blk);
        let (Some(&used), Some(&declared)) = (self.cursors.get(index), self.declared.block_size.get(index)) else {
            panic!("T6 block index {blk} exceeds {MAX_ZONE_FILE_TOTAL} blocks");
        };
        used.saturating_sub(declared as usize)
    }

    pub fn displacement_is_final(&self, spot: RawRef) -> bool {
        self.write_pos(spot.seg) > spot.offset as usize
    }

    pub fn record_displacement(&mut self, spot: RawRef) {
        if self.displacement_is_final(spot) {
            return;
        }
        self.unsettled.count = self.unsettled.count.saturating_add(1);
        let seen = (spot, self.write_pos(spot.seg), self.input.at);
        self.unsettled.first.get_or_insert(seen);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{vec, vec::Vec};

    #[test]
    fn pointers_use_three_block_bits() {
        let p = RawRef {
            seg: ARCHIVE_SEGMENT_HEAPED as u8,
            offset: 0x1234,
        };
        let raw = StreamRef::pack_displacement(p);
        assert_eq!(raw, (5 << 29 | 0x1234) + 1);
        assert_eq!(StreamRef::from_word(raw), StreamRef::At(p));
        assert_eq!(StreamRef::from_word(0), StreamRef::Null);
        assert_eq!(StreamRef::from_word(u32::MAX), StreamRef::Following);
        assert_eq!(StreamRef::from_word(u32::MAX - 1), StreamRef::Insert);
    }

    #[test]
    fn header_is_forty_bytes_of_sizes() {
        let mut image = [0u8; ZONE_HEAD_BYTES];
        image[0] = 9;
        image[8 + 4 * XFILE_BLOCK_STREAMER_RESERVE] = 7;
        let h = read_zone_head(&image).unwrap();
        assert_eq!(ZONE_HEAD_BYTES, 40);
        assert_eq!(h.size, 9);
        assert_eq!(h.block_size[XFILE_BLOCK_STREAMER_RESERVE], 7);
        assert!(read_zone_head(&image[..39]).is_err());
    }

    fn tiny_zone(payload: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; ZONE_HEAD_BYTES];
        image.extend_from_slice(payload);
        image
    }

    fn with_stream<R>(image: &[u8], f: impl FnOnce(&mut ZoneByteReader) -> R) -> R {
        let mut sizes = [0u32; MAX_ZONE_FILE_TOTAL];
        sizes[ARCHIVE_SEGMENT_SCRATCH] = 16;
        sizes[ARCHIVE_SEGMENT_HEAPED] = 16;
        let mut full = image.to_vec();
        for (i, s) in sizes.iter().enumerate() {
            full[8 + 4 * i..12 + 4 * i].copy_from_slice(&s.to_le_bytes());
        }
        let mut storage: Vec<Vec<u8>> = sizes.iter().map(|&s| vec![0u8; s as usize]).collect();
        let mut it = storage.iter_mut();
        let seg_bufs: [&mut [u8]; MAX_ZONE_FILE_TOTAL] =
            core::array::from_fn(|_| it.next().unwrap().as_mut_slice());
        let mut map = vec![0u8; 8];
        let mut zone = ZoneByteReader::new(&full, seg_bufs, &mut map).unwrap();
        f(&mut zone)
    }

    #[test]
    fn popping_a_temp_block_rewinds_its_write_position() {
        let image = tiny_zone(&[1, 2, 3, 4, 5, 6, 7, 8]);
        with_stream(&image, |zone| {
            zone.put(ARCHIVE_SEGMENT_HEAPED).unwrap();
            zone.put(ARCHIVE_SEGMENT_SCRATCH).unwrap();
            zone.load_bytes(4).unwrap();
            assert_eq!(zone.write_pos(ARCHIVE_SEGMENT_SCRATCH as u8), 4);
            zone.pop().unwrap();
            assert_eq!(zone.write_pos(ARCHIVE_SEGMENT_SCRATCH as u8), 0);
            zone.load_bytes(4).unwrap();
            assert_eq!(zone.write_pos(ARCHIVE_SEGMENT_HEAPED as u8), 4);
            zone.pop().unwrap();
            assert_eq!(zone.pop(), Err(ZoneReadFault::PopFromEmptyStack));
        });
    }

    #[test]
    fn the_block_stack_holds_thirty_two_entries() {
        with_stream(&tiny_zone(&[]), |zone| {
            for _ in 0..SEGMENT_DEPTH_LIMIT {
                zone.put(ARCHIVE_SEGMENT_HEAPED).unwrap();
            }
            assert_eq!(zone.put(ARCHIVE_SEGMENT_HEAPED), Err(ZoneReadFault::DepthExceeded));
            assert_eq!(zone.put(MAX_ZONE_FILE_TOTAL), Err(ZoneReadFault::CorruptSegment(MAX_ZONE_FILE_TOTAL)));
        });
    }

    #[test]
    fn strings_stop_at_their_terminator_or_fail_cleanly() {
        let image = tiny_zone(b"ab cd");
        with_stream(&image, |zone| {
            zone.put(ARCHIVE_SEGMENT_HEAPED).unwrap();
            let s = zone.read_text_value().unwrap();
            assert_eq!(zone.zero_ended_str(s).unwrap(), "ab");
            assert_eq!(zone.write_pos(ARCHIVE_SEGMENT_HEAPED as u8), 3);
            assert_eq!(
                zone.read_text_value(),
                Err(ZoneReadFault::TextWithoutEnd { block: ARCHIVE_SEGMENT_HEAPED })
            );
        });
        let full_block = tiny_zone(&[b'x'; 20]);
        with_stream(&full_block, |zone| {
            zone.put(ARCHIVE_SEGMENT_HEAPED).unwrap();
            assert!(matches!(
                zone.read_text_value(),
                Err(ZoneReadFault::SegmentOverrun { block: ARCHIVE_SEGMENT_HEAPED, end: 16, .. })
            ));
        });
    }

    #[test]
    fn follow_array_fills_the_slot_and_aliases_known_bodies() {
        let mut payload = vec![0xFFu8; 4];
        payload.extend_from_slice(&[9, 8, 7, 6]);
        with_stream(&tiny_zone(&payload), |zone| {
            zone.put(ARCHIVE_SEGMENT_HEAPED).unwrap();
            let head = zone.load_bytes(4).unwrap();
            zone.store_u32_at(head, 0, u32::MAX).unwrap();
            let body = zone.walk_list_pointer(head, 0, 1, 1, 4).unwrap().unwrap();
            assert_eq!(zone.byte_window_from(body, 0, 4).unwrap(), &[9, 8, 7, 6]);
            assert_eq!(
                zone.address_for(head, 0).unwrap(),
                StreamRef::At(body)
            );
            assert_eq!(zone.walk_list_pointer(head, 0, 1, 1, 4).unwrap(), Some(body));
        });
    }

    #[test]
    fn marker_values_and_edges_round_trip() {
        assert_eq!(StreamRef::from_word(u32::MAX - 1), StreamRef::Insert);
        let far = RawRef { seg: 7, offset: OFFSET_MASK - 3 };
        assert_eq!(StreamRef::from_word(StreamRef::pack_displacement(far)), StreamRef::At(far));
        let first = RawRef { seg: 0, offset: 0 };
        assert_eq!(StreamRef::pack_displacement(first), 1);
        assert!(StreamRef::Following.is_trailing() && StreamRef::Insert.is_trailing());
    }

    #[test]
    fn alignment_rounds_up_to_the_boundary() {
        let grid = [(0, 4, 0), (1, 4, 4), (4, 4, 4), (5, 8, 8), (17, 16, 32), (9, 1, 9)];
        for (v, align, want) in grid {
            assert_eq!(align_up(v, align), want, "{v} aligned to {align}");
        }
    }

    #[test]
    fn errors_read_the_same_whichever_path_prints_them() {
        extern crate std;
        use std::string::ToString;
        let cases = [
            (ZoneReadFault::DepthExceeded, "block stack is full"),
            (ZoneReadFault::NothingPushedYet, "nothing on the block stack"),
            (ZoneReadFault::CorruptSegment(9), "zone pointer names block 9, which does not exist"),
            (ZoneReadFault::BadCount(-3), "count expression evaluated to -3"),
            (
                ZoneReadFault::OffsetPastBlock { block: 2, offset: 40, size: 16 },
                "block 2 has 16 bytes, offset 40 is past them",
            ),
        ];
        for (err, text) in cases {
            assert_eq!(err.to_string(), text);
        }
    }
}
