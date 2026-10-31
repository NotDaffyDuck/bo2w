//! The hash every content digest is built from: 64-bit FNV-1a over a fixed little-endian
//! encoding of the values fed to it.
//!
//! Digests are stored in replay files and compared between the two ends of a connection, so
//! this encoding is a wire format: it only changes together with a bump of
//! [`super::CONTENT_PRINT_REVISION`].

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// A running hash of the values `put` into it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Fnv1a(u64);

/// A value with a fixed byte encoding in the digest.
pub(super) trait Encode {
    fn stir_into(self, into: &mut Fnv1a);
}

macro_rules! encode_as_le_bytes {
    ($($ty:ty),+) => {$(
        impl Encode for $ty {
            fn stir_into(self, into: &mut Fnv1a) {
                into.merge_in(&self.to_le_bytes());
            }
        }
    )+};
}

encode_as_le_bytes!(u8, u16, u32, u64, i32);

impl Encode for bool {
    fn stir_into(self, into: &mut Fnv1a) {
        into.merge_in(&[u8::from(self)]);
    }
}

impl Encode for f32 {
    /// By bit pattern, so `-0.0` and `0.0` (and every NaN payload) stay distinct.
    fn stir_into(self, into: &mut Fnv1a) {
        self.to_bits().stir_into(into);
    }
}

impl Fnv1a {
    pub(super) const fn new() -> Self {
        Self(OFFSET_BASIS)
    }

    /// Mixes raw bytes in, one at a time and with no length marker.
    pub(super) fn merge_in(&mut self, raw: &[u8]) {
        self.0 = raw.iter().fold(self.0, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(PRIME));
    }

    pub(super) fn put(&mut self, value: impl Encode) {
        value.stir_into(self);
    }

    /// Each value in turn, with no count in front.
    pub(super) fn put_all<T: Encode>(&mut self, values: impl IntoIterator<Item = T>) {
        values.into_iter().for_each(|item| item.stir_into(self));
    }

    /// The length of a list that follows, as eight bytes.
    pub(super) fn put_len(&mut self, span: usize) {
        self.put(span as u64);
    }

    /// A string as its length and then its UTF-8 bytes.
    pub(super) fn put_text(&mut self, words: &str) {
        self.put_len(words.len());
        self.merge_in(words.as_bytes());
    }

    pub(super) const fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Fnv1a;

    fn of_bytes(bytes: &[u8]) -> u64 {
        let mut hash = Fnv1a::new();
        hash.merge_in(bytes);
        hash.finish()
    }

    #[test]
    fn matches_the_published_fnv1a_vectors() {
        assert_eq!(of_bytes(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(of_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(of_bytes(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn integers_go_in_little_endian() {
        let mut put = Fnv1a::new();
        put.put(0x0102_0304_u32);
        put.put(0x0506_u16);
        put.put(7_u8);
        put.put(0x1122_3344_5566_7788_u64);
        assert_eq!(
            put.finish(),
            of_bytes(&[4, 3, 2, 1, 6, 5, 7, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11])
        );
    }

    #[test]
    fn a_signed_value_is_its_twos_complement() {
        let mut put = Fnv1a::new();
        put.put(-2_i32);
        assert_eq!(put.finish(), of_bytes(&[0xfe, 0xff, 0xff, 0xff]));
    }

    #[test]
    fn a_bool_is_one_byte() {
        let mut put = Fnv1a::new();
        put.put(true);
        put.put(false);
        assert_eq!(put.finish(), of_bytes(&[1, 0]));
    }

    #[test]
    fn a_float_goes_in_by_its_bits() {
        let mut put = Fnv1a::new();
        put.put(1.0_f32);
        assert_eq!(put.finish(), of_bytes(&1.0_f32.to_bits().to_le_bytes()));

        let (mut zero, mut negative_zero) = (Fnv1a::new(), Fnv1a::new());
        zero.put(0.0_f32);
        negative_zero.put(-0.0_f32);
        assert_ne!(zero.finish(), negative_zero.finish());
    }

    #[test]
    fn text_carries_its_length() {
        let mut put = Fnv1a::new();
        put.put_text("ab");
        assert_eq!(put.finish(), of_bytes(&[2, 0, 0, 0, 0, 0, 0, 0, b'a', b'b']));
    }

    #[test]
    fn a_list_length_is_eight_bytes() {
        let mut put = Fnv1a::new();
        put.put_len(3);
        assert_eq!(put.finish(), of_bytes(&[3, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn put_all_equals_putting_one_at_a_time() {
        let (mut together, mut apart) = (Fnv1a::new(), Fnv1a::new());
        together.put_all([1_i32, -5, 9]);
        for value in [1_i32, -5, 9] {
            apart.put(value);
        }
        assert_eq!(together.finish(), apart.finish());
    }

    #[test]
    fn order_matters() {
        let (mut forward, mut backward) = (Fnv1a::new(), Fnv1a::new());
        forward.put_all([1_u32, 2]);
        backward.put_all([2_u32, 1]);
        assert_ne!(forward.finish(), backward.finish());
    }
}
