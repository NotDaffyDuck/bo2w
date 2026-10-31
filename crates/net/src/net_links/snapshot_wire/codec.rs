//! The shared vocabulary of the snapshot wire: one trait for values that know their own bytes,
//! plus the list, flag and tag helpers every section is built from.

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub(super) fn refuse<T>(what: &'static str) -> Result<T, PacketDecodeFault> {
    Err(PacketDecodeFault::GarbledRequest(what))
}

/// A value that can write itself to the wire and read itself back, byte for byte.
pub(super) trait Wire: Sized {
    fn put(&self, wr: &mut ByteSinkWriter);
    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault>;

    /// Several values in a row; bytes override this to move in one piece.
    fn put_run(run: &[Self], wr: &mut ByteSinkWriter) {
        for item in run {
            item.put(wr);
        }
    }

    fn take_run<const N: usize>(rd: &mut ByteCursorReader<'_>) -> Result<[Self; N], PacketDecodeFault>
    where
        Self: Copy + Default,
    {
        let mut run = [Self::default(); N];
        for slot in &mut run {
            *slot = Self::take(rd)?;
        }
        Ok(run)
    }
}

impl Wire for u8 {
    fn put(&self, wr: &mut ByteSinkWriter) {
        wr.put_u8(*self);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        rd.get_u8()
    }

    fn put_run(run: &[Self], wr: &mut ByteSinkWriter) {
        wr.append_raw_slice(run);
    }

    fn take_run<const N: usize>(rd: &mut ByteCursorReader<'_>) -> Result<[Self; N], PacketDecodeFault> {
        let mut run = [0u8; N];
        rd.copy_out(&mut run)?;
        Ok(run)
    }
}

macro_rules! plain_numbers {
    ($($ty:ty: $write:ident / $read:ident),+ $(,)?) => {$(
        impl Wire for $ty {
            fn put(&self, wr: &mut ByteSinkWriter) {
                wr.$write(*self);
            }

            fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
                rd.$read()
            }
        }
    )+};
}

plain_numbers! {
    u16: put_u16 / get_u16,
    u32: put_u32 / read_word32,
    u64: put_u64 / get_u64,
    i32: poke_int32 / get_i32,
    f32: write_real32 / read_f32,
}

impl Wire for bool {
    fn put(&self, wr: &mut ByteSinkWriter) {
        wr.put_u8(u8::from(*self));
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        Ok(rd.get_u8()? != 0)
    }
}

impl<T: Wire + Copy + Default, const N: usize> Wire for [T; N] {
    fn put(&self, wr: &mut ByteSinkWriter) {
        T::put_run(self, wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        T::take_run(rd)
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.0.put(wr);
        self.1.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let first = A::take(rd)?;
        Ok((first, B::take(rd)?))
    }
}

impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.0.put(wr);
        self.1.put(wr);
        self.2.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let first = A::take(rd)?;
        let second = B::take(rd)?;
        Ok((first, second, C::take(rd)?))
    }
}

/// Free-form text always travels behind a two-byte length.
impl Wire for String {
    fn put(&self, wr: &mut ByteSinkWriter) {
        put_text16(wr, self);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        take_text16(rd, "DObj text is not UTF-8")
    }
}

/// Switches packed into one byte, the first switch in the lowest bit.
pub(super) fn pack_bits<const N: usize>(switches: [bool; N]) -> u8 {
    switches
        .iter()
        .enumerate()
        .fold(0, |byte, (place, &on)| byte | (u8::from(on) << place))
}

pub(super) fn unpack_bits<const N: usize>(byte: u8) -> [bool; N] {
    std::array::from_fn(|place| byte & (1 << place) != 0)
}

/// A list behind a one-byte count.
pub(super) fn put_short_list<T: Wire>(wr: &mut ByteSinkWriter, items: &[T]) {
    debug_assert!(items.len() <= usize::from(u8::MAX));
    wr.put_u8(items.len() as u8);
    T::put_run(items, wr);
}

/// A list behind a two-byte count.
pub(super) fn put_list<T: Wire>(wr: &mut ByteSinkWriter, items: &[T]) {
    debug_assert!(items.len() <= usize::from(u16::MAX));
    wr.put_u16(items.len() as u16);
    T::put_run(items, wr);
}

pub(super) fn take_short_list<T: Wire>(rd: &mut ByteCursorReader<'_>, reserve: usize) -> Result<Vec<T>, PacketDecodeFault> {
    let count = usize::from(rd.get_u8()?);
    take_counted(rd, count, reserve)
}

pub(super) fn take_list<T: Wire>(rd: &mut ByteCursorReader<'_>, reserve: usize) -> Result<Vec<T>, PacketDecodeFault> {
    let count = usize::from(rd.get_u16()?);
    take_counted(rd, count, reserve)
}

/// `count` values in a row. `reserve` bounds the up-front allocation so a hostile count cannot
/// ask for more memory than the bytes behind it can fill.
pub(super) fn take_counted<T: Wire>(
    rd: &mut ByteCursorReader<'_>,
    count: usize,
    reserve: usize,
) -> Result<Vec<T>, PacketDecodeFault> {
    let mut items = Vec::with_capacity(count.min(reserve));
    for _ in 0..count {
        items.push(T::take(rd)?);
    }
    Ok(items)
}

/// `Some` as a 1 followed by the value, `None` as a lone 0.
pub(super) fn put_flagged<T: Wire>(wr: &mut ByteSinkWriter, maybe: &Option<T>) {
    match maybe {
        Some(inner) => {
            wr.put_u8(1);
            inner.put(wr);
        }
        None => wr.put_u8(0),
    }
}

pub(super) fn take_flagged<T: Wire>(
    rd: &mut ByteCursorReader<'_>,
    what: &'static str,
) -> Result<Option<T>, PacketDecodeFault> {
    match rd.get_u8()? {
        0 => Ok(None),
        1 => T::take(rd).map(Some),
        _ => refuse(what),
    }
}

/// Text made of bytes that must be UTF-8; `what` names the refusal otherwise.
pub(super) fn text_from(bytes: Vec<u8>, what: &'static str) -> Result<String, PacketDecodeFault> {
    String::from_utf8(bytes).or(refuse(what))
}

/// Reads `len` bytes into a fresh buffer.
pub(super) fn take_blob(rd: &mut ByteCursorReader<'_>, len: usize) -> Result<Vec<u8>, PacketDecodeFault> {
    let mut blob = vec![0u8; len];
    rd.copy_out(&mut blob)?;
    Ok(blob)
}

/// Text behind a one-byte length; anything past 255 bytes is cut off.
pub(super) fn put_text8(wr: &mut ByteSinkWriter, body: &str) {
    debug_assert!(body.len() <= usize::from(u8::MAX));
    let kept = &body.as_bytes()[..body.len().min(usize::from(u8::MAX))];
    wr.put_u8(kept.len() as u8);
    wr.append_raw_slice(kept);
}

pub(super) fn take_text8(rd: &mut ByteCursorReader<'_>, what: &'static str) -> Result<String, PacketDecodeFault> {
    let len = usize::from(rd.get_u8()?);
    text_from(take_blob(rd, len)?, what)
}

/// Text behind a two-byte length.
pub(super) fn put_text16(wr: &mut ByteSinkWriter, body: &str) {
    debug_assert!(body.len() <= usize::from(u16::MAX));
    wr.put_u16(body.len() as u16);
    wr.append_raw_slice(body.as_bytes());
}

pub(super) fn take_text16(rd: &mut ByteCursorReader<'_>, what: &'static str) -> Result<String, PacketDecodeFault> {
    let len = usize::from(rd.get_u16()?);
    text_from(take_blob(rd, len)?, what)
}

/// A list behind a two-byte count that must not exceed `limit`; `what` names the refusal.
pub(super) fn take_list_within<T: Wire>(
    rd: &mut ByteCursorReader<'_>,
    cap: usize,
    what: &'static str,
) -> Result<Vec<T>, PacketDecodeFault> {
    let count = usize::from(rd.get_u16()?);
    if count > cap {
        return refuse(what);
    }
    take_counted(rd, count, cap)
}

/// Gives a type with one public number inside its own wire form: the number itself.
macro_rules! wire_newtype {
    ($($ty:ty => $inner:ty),+ $(,)?) => {$(
        impl $crate::net_links::snapshot_wire::codec::Wire for $ty {
            fn put(&self, wr: &mut $crate::net_links::byte_codec::ByteSinkWriter) {
                $crate::net_links::snapshot_wire::codec::Wire::put(&self.0, wr);
            }

            fn take(
                rd: &mut $crate::net_links::byte_codec::ByteCursorReader<'_>,
            ) -> Result<Self, $crate::net_links::byte_codec::PacketDecodeFault> {
                Ok(Self(<$inner as $crate::net_links::snapshot_wire::codec::Wire>::take(rd)?))
            }
        }
    )+};
}

/// A record written as its fields in the order listed, read back in that same order. A field
/// written `name: flag "what"` is an `Option` sent behind a 0/1 byte and refused with `what` on
/// any other byte.
macro_rules! wire_fields {
    (@put $wr:ident, $value:expr) => {
        $crate::net_links::snapshot_wire::codec::Wire::put($value, $wr)
    };
    (@put $wr:ident, $value:expr, $flag:literal) => {
        $crate::net_links::snapshot_wire::codec::put_flagged($wr, $value)
    };
    (@take $rd:ident) => {
        $crate::net_links::snapshot_wire::codec::Wire::take($rd)?
    };
    (@take $rd:ident, $flag:literal) => {
        $crate::net_links::snapshot_wire::codec::take_flagged($rd, $flag)?
    };
    ($ty:ty { $($item:ident $(: flag $flag:literal)?),+ $(,)? }) => {
        impl $crate::net_links::snapshot_wire::codec::Wire for $ty {
            fn put(&self, wr: &mut $crate::net_links::byte_codec::ByteSinkWriter) {
                $( wire_fields!(@put wr, &self.$item $(, $flag)?); )+
            }

            fn take(
                rd: &mut $crate::net_links::byte_codec::ByteCursorReader<'_>,
            ) -> Result<Self, $crate::net_links::byte_codec::PacketDecodeFault> {
                Ok(Self {
                    $( $item: wire_fields!(@take rd $(, $flag)?), )+
                })
            }
        }
    };
}

/// An enum without payloads written as one tag byte; anything off the table is refused with `what`.
macro_rules! wire_tags {
    ($ty:ty, $what:literal, { $($variant:path => $tag:literal),+ $(,)? }) => {
        impl $crate::net_links::snapshot_wire::codec::Wire for $ty {
            fn put(&self, wr: &mut $crate::net_links::byte_codec::ByteSinkWriter) {
                wr.put_u8(match self {
                    $( $variant => $tag, )+
                });
            }

            fn take(
                rd: &mut $crate::net_links::byte_codec::ByteCursorReader<'_>,
            ) -> Result<Self, $crate::net_links::byte_codec::PacketDecodeFault> {
                match rd.get_u8()? {
                    $( $tag => Ok($variant), )+
                    _ => $crate::net_links::snapshot_wire::codec::refuse($what),
                }
            }
        }
    };
}

/// An enum whose variants carry fields: one tag byte per variant, then its fields in the order
/// listed. A field written `name: flag "what"` is an `Option` sent behind a 0/1 byte and refused
/// with `what` on any other byte. The table is the whole format: a new variant is one more row.
macro_rules! wire_variants {
    (@put $wr:ident, $item:ident) => {
        $crate::net_links::snapshot_wire::codec::Wire::put($item, $wr)
    };
    (@put $wr:ident, $item:ident, $flag:literal) => {
        $crate::net_links::snapshot_wire::codec::put_flagged($wr, $item)
    };
    (@take $rd:ident) => {
        $crate::net_links::snapshot_wire::codec::Wire::take($rd)?
    };
    (@take $rd:ident, $flag:literal) => {
        $crate::net_links::snapshot_wire::codec::take_flagged($rd, $flag)?
    };
    ($ty:ident, $what:literal, {
        $($tag:literal => $kind:ident $({ $($item:ident $(: flag $flag:literal)?),* $(,)? })?),+ $(,)?
    }) => {
        impl $crate::net_links::snapshot_wire::codec::Wire for $ty {
            fn put(&self, wr: &mut $crate::net_links::byte_codec::ByteSinkWriter) {
                match self {
                    $($ty::$kind { $($($item),*)? } => {
                        wr.put_u8($tag);
                        $($(wire_variants!(@put wr, $item $(, $flag)?);)*)?
                    })+
                }
            }

            fn take(
                rd: &mut $crate::net_links::byte_codec::ByteCursorReader<'_>,
            ) -> Result<Self, $crate::net_links::byte_codec::PacketDecodeFault> {
                match rd.get_u8()? {
                    $($tag => {
                        $($(let $item = wire_variants!(@take rd $(, $flag)?);)*)?
                        Ok($ty::$kind { $($($item),*)? })
                    })+
                    _ => $crate::net_links::snapshot_wire::codec::refuse($what),
                }
            }
        }
    };
}
