//! The shared vocabulary of the snapshot wire: one trait for values that know their own bytes,
//! plus the list, flag and tag helpers every section is built from.

use crate::transport::wire::{WireError, WireReader, WireWriter};

pub(super) type Decoded<T> = Result<T, WireError>;

pub(super) fn refuse<T>(what: &'static str) -> Decoded<T> {
    Err(WireError::Malformed(what))
}

/// A value that can write itself to the wire and read itself back, byte for byte.
pub(super) trait Wire: Sized {
    fn put(&self, out: &mut WireWriter);
    fn take(input: &mut WireReader<'_>) -> Decoded<Self>;

    /// Several values in a row; bytes override this to move in one piece.
    fn put_run(run: &[Self], out: &mut WireWriter) {
        for item in run {
            item.put(out);
        }
    }

    fn take_run<const N: usize>(input: &mut WireReader<'_>) -> Decoded<[Self; N]>
    where
        Self: Copy + Default,
    {
        let mut run = [Self::default(); N];
        for slot in &mut run {
            *slot = Self::take(input)?;
        }
        Ok(run)
    }
}

impl Wire for u8 {
    fn put(&self, out: &mut WireWriter) {
        out.put_u8(*self);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        input.get_u8()
    }

    fn put_run(run: &[Self], out: &mut WireWriter) {
        out.put_bytes(run);
    }

    fn take_run<const N: usize>(input: &mut WireReader<'_>) -> Decoded<[Self; N]> {
        let mut run = [0u8; N];
        input.get_bytes(&mut run)?;
        Ok(run)
    }
}

macro_rules! plain_numbers {
    ($($ty:ty: $write:ident / $read:ident),+ $(,)?) => {$(
        impl Wire for $ty {
            fn put(&self, out: &mut WireWriter) {
                out.$write(*self);
            }

            fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
                input.$read()
            }
        }
    )+};
}

plain_numbers! {
    u16: put_u16 / get_u16,
    u32: put_u32 / get_u32,
    u64: put_u64 / get_u64,
    i32: put_i32 / get_i32,
    f32: put_f32 / get_f32,
}

impl Wire for bool {
    fn put(&self, out: &mut WireWriter) {
        out.put_u8(u8::from(*self));
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        Ok(input.get_u8()? != 0)
    }
}

impl<T: Wire + Copy + Default, const N: usize> Wire for [T; N] {
    fn put(&self, out: &mut WireWriter) {
        T::put_run(self, out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        T::take_run(input)
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn put(&self, out: &mut WireWriter) {
        self.0.put(out);
        self.1.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let first = A::take(input)?;
        Ok((first, B::take(input)?))
    }
}

impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn put(&self, out: &mut WireWriter) {
        self.0.put(out);
        self.1.put(out);
        self.2.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let first = A::take(input)?;
        let second = B::take(input)?;
        Ok((first, second, C::take(input)?))
    }
}

/// Free-form text always travels behind a two-byte length.
impl Wire for String {
    fn put(&self, out: &mut WireWriter) {
        put_text16(out, self);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        take_text16(input, "DObj text is not UTF-8")
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
pub(super) fn put_short_list<T: Wire>(out: &mut WireWriter, items: &[T]) {
    debug_assert!(items.len() <= usize::from(u8::MAX));
    out.put_u8(items.len() as u8);
    T::put_run(items, out);
}

/// A list behind a two-byte count.
pub(super) fn put_list<T: Wire>(out: &mut WireWriter, items: &[T]) {
    debug_assert!(items.len() <= usize::from(u16::MAX));
    out.put_u16(items.len() as u16);
    T::put_run(items, out);
}

pub(super) fn take_short_list<T: Wire>(input: &mut WireReader<'_>, reserve: usize) -> Decoded<Vec<T>> {
    let count = usize::from(input.get_u8()?);
    take_counted(input, count, reserve)
}

pub(super) fn take_list<T: Wire>(input: &mut WireReader<'_>, reserve: usize) -> Decoded<Vec<T>> {
    let count = usize::from(input.get_u16()?);
    take_counted(input, count, reserve)
}

/// `count` values in a row. `reserve` bounds the up-front allocation so a hostile count cannot
/// ask for more memory than the bytes behind it can fill.
pub(super) fn take_counted<T: Wire>(
    input: &mut WireReader<'_>,
    count: usize,
    reserve: usize,
) -> Decoded<Vec<T>> {
    let mut items = Vec::with_capacity(count.min(reserve));
    for _ in 0..count {
        items.push(T::take(input)?);
    }
    Ok(items)
}

/// `Some` as a 1 followed by the value, `None` as a lone 0.
pub(super) fn put_flagged<T: Wire>(out: &mut WireWriter, value: &Option<T>) {
    match value {
        Some(inner) => {
            out.put_u8(1);
            inner.put(out);
        }
        None => out.put_u8(0),
    }
}

pub(super) fn take_flagged<T: Wire>(
    input: &mut WireReader<'_>,
    what: &'static str,
) -> Decoded<Option<T>> {
    match input.get_u8()? {
        0 => Ok(None),
        1 => T::take(input).map(Some),
        _ => refuse(what),
    }
}

/// Text out of bytes that must be UTF-8; `what` names the refusal otherwise.
pub(super) fn text_from(bytes: Vec<u8>, what: &'static str) -> Decoded<String> {
    String::from_utf8(bytes).or(refuse(what))
}

/// Reads `len` bytes into a fresh buffer.
pub(super) fn take_blob(input: &mut WireReader<'_>, len: usize) -> Decoded<Vec<u8>> {
    let mut blob = vec![0u8; len];
    input.get_bytes(&mut blob)?;
    Ok(blob)
}

/// Text behind a one-byte length; anything past 255 bytes is cut off.
pub(super) fn put_text8(out: &mut WireWriter, text: &str) {
    debug_assert!(text.len() <= usize::from(u8::MAX));
    let kept = &text.as_bytes()[..text.len().min(usize::from(u8::MAX))];
    out.put_u8(kept.len() as u8);
    out.put_bytes(kept);
}

pub(super) fn take_text8(input: &mut WireReader<'_>, what: &'static str) -> Decoded<String> {
    let len = usize::from(input.get_u8()?);
    text_from(take_blob(input, len)?, what)
}

/// Text behind a two-byte length.
pub(super) fn put_text16(out: &mut WireWriter, text: &str) {
    debug_assert!(text.len() <= usize::from(u16::MAX));
    out.put_u16(text.len() as u16);
    out.put_bytes(text.as_bytes());
}

pub(super) fn take_text16(input: &mut WireReader<'_>, what: &'static str) -> Decoded<String> {
    let len = usize::from(input.get_u16()?);
    text_from(take_blob(input, len)?, what)
}

/// A list behind a two-byte count that must not exceed `limit`; `what` names the refusal.
pub(super) fn take_list_within<T: Wire>(
    input: &mut WireReader<'_>,
    limit: usize,
    what: &'static str,
) -> Decoded<Vec<T>> {
    let count = usize::from(input.get_u16()?);
    if count > limit {
        return refuse(what);
    }
    take_counted(input, count, limit)
}

/// Gives a type with one public number inside its own wire form: the number itself.
macro_rules! wire_newtype {
    ($($ty:ty => $inner:ty),+ $(,)?) => {$(
        impl $crate::transport::snapshot_wire::codec::Wire for $ty {
            fn put(&self, out: &mut $crate::transport::wire::WireWriter) {
                $crate::transport::snapshot_wire::codec::Wire::put(&self.0, out);
            }

            fn take(
                input: &mut $crate::transport::wire::WireReader<'_>,
            ) -> $crate::transport::snapshot_wire::codec::Decoded<Self> {
                Ok(Self(<$inner as $crate::transport::snapshot_wire::codec::Wire>::take(input)?))
            }
        }
    )+};
}

/// A record written as its fields in the order listed, read back in that same order. A field
/// written `name: flag "what"` is an `Option` sent behind a 0/1 byte and refused with `what` on
/// any other byte.
macro_rules! wire_fields {
    (@put $out:ident, $value:expr) => {
        $crate::transport::snapshot_wire::codec::Wire::put($value, $out)
    };
    (@put $out:ident, $value:expr, $flag:literal) => {
        $crate::transport::snapshot_wire::codec::put_flagged($out, $value)
    };
    (@take $input:ident) => {
        $crate::transport::snapshot_wire::codec::Wire::take($input)?
    };
    (@take $input:ident, $flag:literal) => {
        $crate::transport::snapshot_wire::codec::take_flagged($input, $flag)?
    };
    ($ty:ty { $($field:ident $(: flag $flag:literal)?),+ $(,)? }) => {
        impl $crate::transport::snapshot_wire::codec::Wire for $ty {
            fn put(&self, out: &mut $crate::transport::wire::WireWriter) {
                $( wire_fields!(@put out, &self.$field $(, $flag)?); )+
            }

            fn take(
                input: &mut $crate::transport::wire::WireReader<'_>,
            ) -> $crate::transport::snapshot_wire::codec::Decoded<Self> {
                Ok(Self {
                    $( $field: wire_fields!(@take input $(, $flag)?), )+
                })
            }
        }
    };
}

/// An enum without payloads written as one tag byte; anything off the table is refused with `what`.
macro_rules! wire_tags {
    ($ty:ty, $what:literal, { $($variant:path => $tag:literal),+ $(,)? }) => {
        impl $crate::transport::snapshot_wire::codec::Wire for $ty {
            fn put(&self, out: &mut $crate::transport::wire::WireWriter) {
                out.put_u8(match self {
                    $( $variant => $tag, )+
                });
            }

            fn take(
                input: &mut $crate::transport::wire::WireReader<'_>,
            ) -> $crate::transport::snapshot_wire::codec::Decoded<Self> {
                match input.get_u8()? {
                    $( $tag => Ok($variant), )+
                    _ => $crate::transport::snapshot_wire::codec::refuse($what),
                }
            }
        }
    };
}

/// An enum whose variants carry fields: one tag byte per variant, then its fields in the order
/// listed. A field written `name: flag "what"` is an `Option` sent behind a 0/1 byte and refused
/// with `what` on any other byte. The table is the whole format: a new variant is one more row.
macro_rules! wire_variants {
    (@put $out:ident, $field:ident) => {
        $crate::transport::snapshot_wire::codec::Wire::put($field, $out)
    };
    (@put $out:ident, $field:ident, $flag:literal) => {
        $crate::transport::snapshot_wire::codec::put_flagged($out, $field)
    };
    (@take $input:ident) => {
        $crate::transport::snapshot_wire::codec::Wire::take($input)?
    };
    (@take $input:ident, $flag:literal) => {
        $crate::transport::snapshot_wire::codec::take_flagged($input, $flag)?
    };
    ($ty:ident, $what:literal, {
        $($tag:literal => $kind:ident $({ $($field:ident $(: flag $flag:literal)?),* $(,)? })?),+ $(,)?
    }) => {
        impl $crate::transport::snapshot_wire::codec::Wire for $ty {
            fn put(&self, out: &mut $crate::transport::wire::WireWriter) {
                match self {
                    $($ty::$kind { $($($field),*)? } => {
                        out.put_u8($tag);
                        $($(wire_variants!(@put out, $field $(, $flag)?);)*)?
                    })+
                }
            }

            fn take(
                input: &mut $crate::transport::wire::WireReader<'_>,
            ) -> $crate::transport::snapshot_wire::codec::Decoded<Self> {
                match input.get_u8()? {
                    $($tag => {
                        $($(let $field = wire_variants!(@take input $(, $flag)?);)*)?
                        Ok($ty::$kind { $($($field),*)? })
                    })+
                    _ => $crate::transport::snapshot_wire::codec::refuse($what),
                }
            }
        }
    };
}
