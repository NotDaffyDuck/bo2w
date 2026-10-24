use super::error::WireError;

/// Reads numbers back out of a byte stream written by `WireWriter`.
#[derive(Debug)]
pub struct WireReader<'a> {
    rest: &'a [u8],
}

/// One `get_*` method per number type, each taking the type's width from the stream.
macro_rules! get_numbers {
    ($($name:ident: $ty:ty),* $(,)?) => {
        $(
            pub fn $name(&mut self) -> Result<$ty, WireError> {
                self.take_array().map(<$ty>::from_le_bytes)
            }
        )*
    };
}

impl<'a> WireReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    pub fn remaining(&self) -> usize {
        self.rest.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    /// Splits `count` bytes off the front. Nothing is consumed when there are not enough.
    fn take(&mut self, count: usize) -> Result<&'a [u8], WireError> {
        let Some((head, tail)) = self.rest.split_at_checked(count) else {
            return Err(WireError::Truncated {
                needed: count,
                available: self.rest.len(),
            });
        };
        self.rest = tail;
        Ok(head)
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let head = self.take(N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(head);
        Ok(array)
    }

    get_numbers! {
        get_u8: u8,
        get_i8: i8,
        get_u16: u16,
        get_u32: u32,
        get_u64: u64,
        get_i32: i32,
    }

    pub fn get_f32(&mut self) -> Result<f32, WireError> {
        self.get_u32().map(f32::from_bits)
    }

    /// Fills `out` from the stream.
    pub fn get_bytes(&mut self, out: &mut [u8]) -> Result<(), WireError> {
        out.copy_from_slice(self.take(out.len())?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::WireWriter;
    use super::*;

    #[test]
    fn everything_written_reads_back() {
        let mut out = WireWriter::new();
        out.put_u8(200);
        out.put_i8(-5);
        out.put_u16(65_000);
        out.put_u32(4_000_000_000);
        out.put_u64(u64::MAX - 3);
        out.put_i32(i32::MIN);
        out.put_f32(-12.5);
        out.put_bytes(&[1, 2, 3]);
        let bytes = out.finish();

        let mut input = WireReader::new(&bytes);
        assert_eq!(input.get_u8(), Ok(200));
        assert_eq!(input.get_i8(), Ok(-5));
        assert_eq!(input.get_u16(), Ok(65_000));
        assert_eq!(input.get_u32(), Ok(4_000_000_000));
        assert_eq!(input.get_u64(), Ok(u64::MAX - 3));
        assert_eq!(input.get_i32(), Ok(i32::MIN));
        assert_eq!(input.get_f32(), Ok(-12.5));
        let mut tail = [0u8; 3];
        assert_eq!(input.get_bytes(&mut tail), Ok(()));
        assert_eq!(tail, [1, 2, 3]);
        assert!(input.is_empty());
    }

    #[test]
    fn a_float_keeps_its_nan_bits() {
        let nan = f32::from_bits(0x7FC0_1234);
        let mut out = WireWriter::new();
        out.put_f32(nan);
        let bytes = out.finish();
        let back = WireReader::new(&bytes).get_f32().unwrap();
        assert_eq!(back.to_bits(), nan.to_bits());
    }

    #[test]
    fn running_out_says_how_much_was_needed_and_left() {
        let mut input = WireReader::new(&[1, 2, 3]);
        assert_eq!(
            input.get_u32(),
            Err(WireError::Truncated { needed: 4, available: 3 })
        );
        let mut input = WireReader::new(&[]);
        assert_eq!(
            input.get_u8(),
            Err(WireError::Truncated { needed: 1, available: 0 })
        );
    }

    #[test]
    fn a_failed_read_consumes_nothing() {
        let mut input = WireReader::new(&[7, 8, 9]);
        assert!(input.get_u32().is_err());
        assert_eq!(input.remaining(), 3);
        assert_eq!(input.get_u16(), Ok(0x0807));
        assert_eq!(input.remaining(), 1);
    }

    #[test]
    fn a_short_byte_copy_leaves_the_target_alone() {
        let mut input = WireReader::new(&[1, 2]);
        let mut target = [0xEE; 4];
        assert_eq!(
            input.get_bytes(&mut target),
            Err(WireError::Truncated { needed: 4, available: 2 })
        );
        assert_eq!(target, [0xEE; 4]);
        assert_eq!(input.remaining(), 2);
    }

    #[test]
    fn an_empty_copy_always_works() {
        let mut input = WireReader::new(&[]);
        assert_eq!(input.get_bytes(&mut []), Ok(()));
    }
}
