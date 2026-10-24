/// Builds a byte stream; every number goes out little-endian.
#[derive(Debug, Default)]
pub struct WireWriter {
    bytes: Vec<u8>,
}

/// One `put_*` method per number type, each appending the type's little-endian bytes.
macro_rules! put_numbers {
    ($($name:ident: $ty:ty),* $(,)?) => {
        $(
            pub fn $name(&mut self, value: $ty) {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
        )*
    };
}

impl WireWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { bytes: Vec::with_capacity(capacity) }
    }

    put_numbers! {
        put_u8: u8,
        put_i8: i8,
        put_u16: u16,
        put_u32: u32,
        put_u64: u64,
        put_i32: i32,
    }

    /// Floats travel as their bit pattern, so NaNs and signed zeros survive.
    pub fn put_f32(&mut self, value: f32) {
        self.put_u32(value.to_bits());
    }

    pub fn put_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_go_out_little_endian() {
        let mut out = WireWriter::new();
        out.put_u8(0xAB);
        out.put_u16(0x0102);
        out.put_u32(0x0304_0506);
        out.put_u64(0x0708_090A_0B0C_0D0E);
        assert_eq!(
            out.finish(),
            [
                0xAB, 0x02, 0x01, 0x06, 0x05, 0x04, 0x03, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09,
                0x08, 0x07
            ]
        );
    }

    #[test]
    fn signed_numbers_use_twos_complement() {
        let mut out = WireWriter::new();
        out.put_i8(-1);
        out.put_i32(-2);
        assert_eq!(out.as_slice(), [0xFF, 0xFE, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn floats_keep_their_bit_pattern() {
        let mut out = WireWriter::new();
        out.put_f32(1.0);
        out.put_f32(-0.0);
        assert_eq!(out.as_slice(), [0, 0, 0x80, 0x3F, 0, 0, 0, 0x80]);
    }

    #[test]
    fn raw_bytes_are_copied_as_they_are() {
        let mut out = WireWriter::with_capacity(8);
        assert!(out.is_empty());
        out.put_bytes(&[9, 8, 7]);
        assert_eq!(out.len(), 3);
        assert!(!out.is_empty());
        assert_eq!(out.as_slice(), [9, 8, 7]);
    }
}
