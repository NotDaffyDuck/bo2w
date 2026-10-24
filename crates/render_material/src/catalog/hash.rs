//! 64-bit FNV-1a, used to fingerprint small byte runs.

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_more(OFFSET_BASIS, bytes)
}

/// Continues a hash over further bytes.
pub(crate) fn fnv1a64_more(seed: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(seed, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(PRIME))
}
