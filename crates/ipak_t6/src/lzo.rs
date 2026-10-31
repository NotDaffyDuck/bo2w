//! LZO1X decompression, written from the LZO1X stream format as the Linux
//! kernel documents it (Documentation/staging/lzo.rst). Safe: every read and
//! back-reference is bounds-checked.
//!
//! The stream is a sequence of instructions. After each one, `state` is the
//! number of literal bytes it copied (0..=3, or 4 for "4 or more"); an
//! instruction byte below 16 means different things in states 0, 1..=3
//! and 4. Matches copy from `distance` bytes back in the output, byte by
//! byte (overlap repeats), then copy the 0..=3 literals their low bits ask
//! for. `0x11 0x00 0x00` ends the stream.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LzoError {
    InputOverrun,
    OutputOverrun,
    LookBehindOverrun {
        distance: usize,
        got: usize,
    },
    /// The stream ended without the end-of-stream marker.
    NoEndMarker,
}

impl core::fmt::Display for LzoError {
    fn fmt(&self, w: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LzoError::InputOverrun => write!(w, "LZO input ends inside an instruction"),
            LzoError::OutputOverrun => write!(w, "LZO output exceeds its limit"),
            LzoError::LookBehindOverrun { distance, got } => {
                write!(
                    w,
                    "LZO match reaches {distance} bytes back with {got} written"
                )
            }
            LzoError::NoEndMarker => write!(w, "LZO stream has no end marker"),
        }
    }
}

struct In<'a> {
    input: &'a [u8],
    at: usize,
}

impl In<'_> {
    fn next_byte(&mut self) -> Result<u8, LzoError> {
        let b = *self.input.get(self.at).ok_or(LzoError::InputOverrun)?;
        self.at += 1;
        Ok(b)
    }

    fn le16(&mut self) -> Result<usize, LzoError> {
        let lo = self.next_byte()? as usize;
        let hi = self.next_byte()? as usize;
        Ok(lo | hi << 8)
    }

    /// Length extension: each zero byte adds 255, the first non-zero byte
    /// adds itself and ends the run.
    fn extension(&mut self) -> Result<usize, LzoError> {
        let mut n = 0usize;
        loop {
            let b = self.next_byte()?;
            if b != 0 {
                return Ok(n + b as usize);
            }
            n += 255;
        }
    }
}

fn literals(inp: &mut In<'_>, dst: &mut Vec<u8>, n: usize, limit: usize) -> Result<(), LzoError> {
    let end = inp.at.checked_add(n).ok_or(LzoError::InputOverrun)?;
    let bytes = inp.input.get(inp.at..end).ok_or(LzoError::InputOverrun)?;
    if dst.len() + n > limit {
        return Err(LzoError::OutputOverrun);
    }
    dst.extend_from_slice(bytes);
    inp.at = end;
    Ok(())
}

fn copy_match(
    written: &mut Vec<u8>,
    distance: usize,
    len: usize,
    cap: usize,
) -> Result<(), LzoError> {
    if distance == 0 || distance > written.len() {
        return Err(LzoError::LookBehindOverrun {
            distance,
            got: written.len(),
        });
    }
    if written.len() + len > cap {
        return Err(LzoError::OutputOverrun);
    }
    let start = written.len() - distance;
    if distance >= len {
        written.extend_from_within(start..start + len);
    } else {
        // Overlapping: each copied byte may be one this copy just wrote.
        for from in start..start + len {
            let byte = written[from];
            written.push(byte);
        }
    }
    Ok(())
}

/// Decompress one LZO1X stream, appending at most `limit - dst.len()` bytes
/// to `dst`. Returns how many input bytes the stream used.
pub fn decompress(src: &[u8], dst: &mut Vec<u8>, limit: usize) -> Result<usize, LzoError> {
    let mut inp = In { input: src, at: 0 };
    let mut state;
    let first = *src.first().ok_or(LzoError::InputOverrun)?;
    if first > 17 {
        inp.at = 1;
        let t = (first - 17) as usize;
        literals(&mut inp, dst, t, limit)?;
        state = if t < 4 { t } else { 4 };
    } else {
        state = 0;
    }

    loop {
        let inst = inp.next_byte()?;
        let (distance, len, trailing);
        if inst < 16 {
            if state == 0 {
                // Long literal run.
                let mut n = inst as usize;
                if n == 0 {
                    n = 15 + inp.extension()?;
                }
                literals(&mut inp, dst, n + 3, limit)?;
                state = 4;
                continue;
            }
            let h = inp.next_byte()? as usize;
            let d = ((inst >> 2) & 3) as usize;
            if state < 4 {
                distance = (h << 2) + d + 1;
                len = 2;
            } else {
                distance = (h << 2) + d + 2049;
                len = 3;
            }
            trailing = (inst & 3) as usize;
        } else if inst < 32 {
            let mut n = (inst & 7) as usize;
            if n == 0 {
                n = 7 + inp.extension()?;
            }
            let le = inp.le16()?;
            let d = 16384 + (((inst & 8) as usize) << 11) + (le >> 2);
            if d == 16384 {
                return Ok(inp.at);
            }
            distance = d;
            len = n + 2;
            trailing = le & 3;
        } else if inst < 64 {
            let mut n = (inst & 31) as usize;
            if n == 0 {
                n = 31 + inp.extension()?;
            }
            let le = inp.le16()?;
            distance = (le >> 2) + 1;
            len = n + 2;
            trailing = le & 3;
        } else {
            let h = inp.next_byte()? as usize;
            distance = (h << 3) + ((inst >> 2) & 7) as usize + 1;
            len = if inst < 128 {
                3 + ((inst >> 5) & 1) as usize
            } else {
                5 + ((inst >> 5) & 3) as usize
            };
            trailing = (inst & 3) as usize;
        }
        copy_match(dst, distance, len, limit)?;
        literals(&mut inp, dst, trailing, limit)?;
        state = trailing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn literal_only_stream() {
        // 22 - 17 = 5 literals, then the end marker.
        let src = [22, b'h', b'e', b'l', b'l', b'o', 0x11, 0, 0];
        let mut out = vec![];
        assert_eq!(decompress(&src, &mut out, 64), Ok(src.len()));
        assert_eq!(out, b"hello");
    }

    #[test]
    fn short_match_repeats_overlapping_bytes() {
        // 4 literals "abab" (21 - 17 = 4), then 01LDDDSS with L=1 (len 4),
        // DDD=1, H=0 -> distance 2: copies "abab" again; end.
        let src = [21, b'a', b'b', b'a', b'b', 0b0110_0100, 0, 0x11, 0, 0];
        let mut out = vec![];
        decompress(&src, &mut out, 64).unwrap();
        assert_eq!(out, b"abababab");
    }

    #[test]
    fn long_literal_run_in_state_zero() {
        // First byte < 18 starts in state 0: 0000LLLL with L=2 -> 5 literals.
        let src = [2, 1, 2, 3, 4, 5, 0x11, 0, 0];
        let mut out = vec![];
        decompress(&src, &mut out, 64).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn errors_instead_of_panicking() {
        let mut out = vec![];
        assert_eq!(
            decompress(&[22, 1], &mut out, 64),
            Err(LzoError::InputOverrun)
        );
        let mut out = vec![];
        // A match before any output.
        assert!(matches!(
            decompress(&[0x40, 0, 0x11, 0, 0], &mut out, 64),
            Err(LzoError::InputOverrun) | Err(LzoError::LookBehindOverrun { .. })
        ));
        let mut out = vec![];
        assert_eq!(
            decompress(&[22, 1, 2, 3, 4, 5, 0x11, 0, 0], &mut out, 3),
            Err(LzoError::OutputOverrun)
        );
    }
}
