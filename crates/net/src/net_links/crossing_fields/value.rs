//! How one player-state field compares, measures and travels on the wire. The field table calls
//! these through [`FieldValue`], so a new kind of field is one more impl.

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub(crate) trait FieldValue {
    /// Whether the two values differ in any bit.
    fn changed_from(&self, other: &Self) -> bool;
    /// The size of the difference. Kinds with no notion of size report infinity when unequal.
    fn distance(&self, other: &Self) -> f32;
    fn put(&self, out: &mut ByteSinkWriter);
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault>;
}

fn unequal_is_infinite(equal: bool) -> f32 {
    if equal { 0.0 } else { f32::INFINITY }
}

/// Short integers travel as 32-bit ones.
impl FieldValue for i16 {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        (f32::from(*self) - f32::from(*other)).abs()
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        out.poke_int32(i32::from(*self));
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        *self = input.get_i32()? as i16;
        Ok(())
    }
}

// 32-bit integers do not convert losslessly to f32, so they cast.
impl FieldValue for i32 {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        (*self as f32 - *other as f32).abs()
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        out.poke_int32(*self);
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        *self = input.get_i32()?;
        Ok(())
    }
}

impl FieldValue for u32 {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        (*self as f32 - *other as f32).abs()
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        out.put_u32(*self);
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        *self = input.read_word32()?;
        Ok(())
    }
}

impl FieldValue for f32 {
    fn changed_from(&self, other: &Self) -> bool {
        self.to_bits() != other.to_bits()
    }
    fn distance(&self, other: &Self) -> f32 {
        (self - other).abs()
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        out.write_real32(*self);
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        *self = input.read_f32()?;
        Ok(())
    }
}

/// A vector: compared bit for bit, measured by its worst axis, sent axis by axis.
impl<const N: usize> FieldValue for [f32; N] {
    fn changed_from(&self, other: &Self) -> bool {
        self.iter().zip(other).any(|(a, b)| a.changed_from(b))
    }
    fn distance(&self, other: &Self) -> f32 {
        let mut widest = 0.0f32;
        for (a, b) in self.iter().zip(other) {
            let gap = a.distance(b);
            if gap > widest {
                widest = gap;
            }
        }
        widest
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        for component in self {
            out.write_real32(*component);
        }
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        for component in self.iter_mut() {
            *component = input.read_f32()?;
        }
        Ok(())
    }
}

impl<const N: usize> FieldValue for [i32; N] {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        unequal_is_infinite(self == other)
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        for slot in self {
            out.poke_int32(*slot);
        }
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        for slot in self.iter_mut() {
            *slot = input.get_i32()?;
        }
        Ok(())
    }
}

impl<const N: usize> FieldValue for [u32; N] {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        unequal_is_infinite(self == other)
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        for slot in self {
            out.put_u32(*slot);
        }
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        for slot in self.iter_mut() {
            *slot = input.read_word32()?;
        }
        Ok(())
    }
}

/// A block of bytes the wire carries as it is.
impl<const N: usize> FieldValue for [u8; N] {
    fn changed_from(&self, other: &Self) -> bool {
        self != other
    }
    fn distance(&self, other: &Self) -> f32 {
        unequal_is_infinite(self == other)
    }
    fn put(&self, out: &mut ByteSinkWriter) {
        out.append_raw_slice(self);
    }
    fn get(&mut self, input: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        input.copy_out(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: FieldValue + Default + Copy>(value: T) -> T {
        let mut out = ByteSinkWriter::new();
        value.put(&mut out);
        let bytes = out.into_bytes();
        let mut reader = ByteCursorReader::new(&bytes);
        let mut back = T::default();
        back.get(&mut reader).expect("a value just written reads back");
        back
    }

    #[test]
    fn scalars_round_trip() {
        assert_eq!(round_trip(-7i32), -7);
        assert_eq!(round_trip(4_000_000_000u32), 4_000_000_000);
        assert_eq!(round_trip(-123i16), -123);
        assert_eq!(round_trip(1.5f32), 1.5);
    }

    #[test]
    fn arrays_round_trip() {
        assert_eq!(round_trip([1.0f32, -2.0, 3.5]), [1.0, -2.0, 3.5]);
        assert_eq!(round_trip([1i32, -2, 3, 4]), [1, -2, 3, 4]);
        assert_eq!(round_trip([9u32, 8]), [9, 8]);
        assert_eq!(round_trip([1u8, 2, 3, 255]), [1, 2, 3, 255]);
    }

    #[test]
    fn a_short_i16_travels_as_four_bytes() {
        let mut out = ByteSinkWriter::new();
        (-1i16).put(&mut out);
        assert_eq!(out.into_bytes().len(), 4);
    }

    #[test]
    fn floats_differ_by_bit_pattern_not_by_value() {
        assert!(0.0f32.changed_from(&-0.0));
        assert!(!f32::NAN.changed_from(&f32::NAN));
        assert!(![1.0f32, 2.0, 3.0].changed_from(&[1.0, 2.0, 3.0]));
        assert!([1.0f32, 2.0, 3.0].changed_from(&[1.0, 2.0, 3.5]));
    }

    #[test]
    fn distances_measure_what_they_can() {
        assert_eq!(3i32.distance(&10), 7.0);
        assert_eq!(10u32.distance(&3), 7.0);
        assert_eq!((-2i16).distance(&5), 7.0);
        assert_eq!([0.0f32, 1.0, 0.0].distance(&[0.5, 4.0, -1.0]), 3.0);
        assert_eq!([1i32, 2].distance(&[1, 2]), 0.0);
        assert_eq!([1i32, 2].distance(&[1, 3]), f32::INFINITY);
        assert_eq!([1u8, 2].distance(&[1, 3]), f32::INFINITY);
    }

    #[test]
    fn a_short_read_is_an_error_and_leaves_the_rest_unread() {
        let mut slot = 0i32;
        let mut reader = ByteCursorReader::new(&[1, 2]);
        assert!(slot.get(&mut reader).is_err());
    }
}
