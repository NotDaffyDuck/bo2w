//! MD5 (RFC 1321), only to check decoded FLAC audio against the MD5 its
//! stream header carries.

pub struct Md5 {
    state: [u32; 4],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn k(i: usize) -> u32 {
    // floor(abs(sin(i + 1)) * 2^32)
    ((((i + 1) as f64).sin().abs()) * 4_294_967_296.0) as u32
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    pub fn new() -> Self {
        Self {
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            buffer: [0; 64],
            buffered: 0,
            length: 0,
        }
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k(i)).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }

    pub fn feed_bytes(&mut self, input: &[u8]) {
        self.length = self.length.wrapping_add(input.len() as u64);
        let mut rest = input;
        if self.buffered > 0 {
            // Top up the partial block first; it runs once it holds 64 bytes.
            let (head, tail) = rest.split_at((64 - self.buffered).min(rest.len()));
            self.buffer[self.buffered..][..head.len()].copy_from_slice(head);
            self.buffered += head.len();
            rest = tail;
            if self.buffered == 64 {
                let full = self.buffer;
                self.block(&full);
                self.buffered = 0;
            }
        }
        let mut whole = rest.chunks_exact(64);
        for chunk in whole.by_ref() {
            let mut full = [0u8; 64];
            full.copy_from_slice(chunk);
            self.block(&full);
        }
        let left = whole.remainder();
        self.buffer[..left.len()].copy_from_slice(left);
        if !left.is_empty() {
            self.buffered = left.len();
        }
    }

    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.length.wrapping_mul(8);
        self.feed_bytes(&[0x80]);
        while self.buffered != 56 {
            self.feed_bytes(&[0]);
        }
        self.feed_bytes(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_vectors() {
        let md5 = |s: &[u8]| {
            let mut m = Md5::new();
            m.feed_bytes(s);
            m.finish().iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }
}
