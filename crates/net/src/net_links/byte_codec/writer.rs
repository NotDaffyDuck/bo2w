/// Builds a byte stream; every number goes out little-endian.
#[derive(Debug, Default)]
pub struct ByteSinkWriter {
    staged: Vec<u8>,
}

/// One `put_*` method per number type, each appending the type's little-endian bytes.
macro_rules! put_numbers {
    ($($name:ident: $ty:ty),* $(,)?) => {
        $(
            pub fn $name(&mut self, num: $ty) {
                self.staged.extend_from_slice(&num.to_le_bytes());
            }
        )*
    };
}

impl ByteSinkWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { staged: Vec::with_capacity(capacity) }
    }

    put_numbers! {
        put_u8: u8,
        put_i8: i8,
        put_u16: u16,
        put_u32: u32,
        put_u64: u64,
        poke_int32: i32,
    }

    /// Floats travel as their bit pattern, so NaNs and signed zeros survive.
    pub fn write_real32(&mut self, float: f32) {
        self.put_u32(float.to_bits());
    }

    pub fn append_raw_slice(&mut self, chunk: &[u8]) {
        self.staged.extend_from_slice(chunk);
    }

    pub fn len(&self) -> usize {
        self.staged.len()
    }

    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.staged
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.staged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_go_out_little_endian() {
        let mut sink = ByteSinkWriter::new();
        sink.put_u8(0xAB);
        sink.put_u16(0x0102);
        sink.put_u32(0x0304_0506);
        sink.put_u64(0x0708_090A_0B0C_0D0E);
        assert_eq!(
            sink.into_bytes(),
            [
                0xAB, 0x02, 0x01, 0x06, 0x05, 0x04, 0x03, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09,
                0x08, 0x07
            ]
        );
    }

    #[test]
    fn signed_numbers_use_twos_complement() {
        let mut sink = ByteSinkWriter::new();
        sink.put_i8(-1);
        sink.poke_int32(-2);
        assert_eq!(sink.as_slice(), [0xFF, 0xFE, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn floats_keep_their_bit_pattern() {
        let mut sink = ByteSinkWriter::new();
        sink.write_real32(1.0);
        sink.write_real32(-0.0);
        assert_eq!(sink.as_slice(), [0, 0, 0x80, 0x3F, 0, 0, 0, 0x80]);
    }

    #[test]
    fn raw_bytes_are_copied_as_they_are() {
        let mut sink = ByteSinkWriter::with_capacity(8);
        assert!(sink.is_empty());
        sink.append_raw_slice(&[9, 8, 7]);
        assert_eq!(sink.len(), 3);
        assert!(!sink.is_empty());
        assert_eq!(sink.as_slice(), [9, 8, 7]);
    }
}
