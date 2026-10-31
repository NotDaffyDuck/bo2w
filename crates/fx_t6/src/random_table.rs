//! The effect random table: 508 floats in 0..1 that every effect channel reads.

// BO2 unknown: the game's own values were never taken; this is our own
// xorshift32 stream, built at compile time, with the same size and range.
pub(crate) const FX_RANDOM_TABLE: [f32; 508] = build();

const fn build() -> [f32; 508] {
    let mut out = [0.0f32; 508];
    let mut x: u32 = 0x2545_f491;
    let mut i = 0;
    while i < out.len() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out[i] = (x >> 8) as f32 / 16_777_216.0;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_in_0_to_1_and_spread() {
        assert!(FX_RANDOM_TABLE.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = FX_RANDOM_TABLE.iter().sum::<f32>() / FX_RANDOM_TABLE.len() as f32;
        assert!((0.4..0.6).contains(&mean));
        let low16: std::collections::HashSet<u16> =
            FX_RANDOM_TABLE.iter().map(|v| v.to_bits() as u16).collect();
        assert!(low16.len() > 450);
    }
}
