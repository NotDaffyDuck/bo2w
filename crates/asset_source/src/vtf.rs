//! VTF textures: the largest mip of the first frame, as RGBA8.

/// A decoded picture, rows top to bottom, 4 bytes a pixel.
#[derive(Clone, Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Bytes one mip of `format` takes, and whether it is block compressed.
fn mip_bytes(format: u32, w: u32, h: u32) -> Option<usize> {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    Some(match format {
        0 | 1 | 11 | 12 | 16 | 23 | 26 => w * h * 4,
        2 | 3 | 9 | 10 => w * h * 3,
        4 | 6 | 17 | 18 | 19 | 21 | 22 => w * h * 2,
        5 | 8 => w * h,
        13 | 20 => blocks * 8,
        14 | 15 => blocks * 16,
        24 | 25 => w * h * 8,
        _ => return None,
    })
}

/// A block-compressed picture (DXT1 / DXT3 / DXT5), cut off at the edges.
fn decode_blocks(format: u32, src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let block_len = if format == 13 || format == 20 { 8 } else { 16 };
    let across = w.div_ceil(4);
    let mut pixels = vec![0u8; w * h * 4];
    let mut cell = [0u8; 64];
    for (n, block) in src.chunks_exact(block_len).take(across * h.div_ceil(4)).enumerate() {
        match format {
            14 => bcdec_rs::bc2(block, &mut cell, 16),
            15 => bcdec_rs::bc3(block, &mut cell, 16),
            _ => bcdec_rs::bc1(block, &mut cell, 16),
        }
        let (x0, y0) = (n % across * 4, n / across * 4);
        let run = 4.min(w - x0) * 4;
        for (ty, row) in cell.chunks_exact(16).enumerate().take_while(|(ty, _)| y0 + ty < h) {
            let d = ((y0 + ty) * w + x0) * 4;
            pixels[d..d + run].copy_from_slice(&row[..run]);
        }
    }
    pixels
}

/// Uncompressed formats: bytes a pixel, then for each of R,G,B,A which
/// source byte it comes from (4 stands for a full 255).
fn packed_layout(format: u32) -> Option<(usize, [usize; 4])> {
    Some(match format {
        0 => (4, [0, 1, 2, 3]),
        1 => (4, [3, 2, 1, 0]),
        2 | 9 => (3, [0, 1, 2, 4]),
        3 | 10 => (3, [2, 1, 0, 4]),
        11 => (4, [1, 2, 3, 0]),
        12 => (4, [2, 1, 0, 3]),
        16 => (4, [2, 1, 0, 4]),
        5 => (1, [0, 0, 0, 4]),
        8 => (1, [4, 4, 4, 0]),
        6 => (2, [0, 0, 0, 1]),
        _ => return None,
    })
}

fn decode_packed(format: u32, src: &[u8]) -> Option<Vec<u8>> {
    let (bytes, taps) = packed_layout(format)?;
    let mut out = Vec::with_capacity(src.len() / bytes * 4);
    for px in src.chunks_exact(bytes) {
        let mut seen = [255u8; 5];
        seen[..bytes].copy_from_slice(px);
        out.extend(taps.map(|t| seen[t]));
    }
    Some(out)
}

/// Decodes a VTF file.
pub fn decode(data: &[u8]) -> Option<Rgba> {
    if data.get(0..4)? != b"VTF\0" {
        return None;
    }
    let minor = u32_at(data, 8)?;
    let header = u32_at(data, 12).map(|n| n as usize)?;
    let width = u32::from(u16_at(data, 16)?);
    let height = u32::from(u16_at(data, 18)?);
    let flags = u32_at(data, 20)?;
    let frames = u32::from(u16_at(data, 24)?.max(1));
    let format = u32_at(data, 52)?;
    let mips = u32::from(*data.get(56)?).max(1);
    let low_format = u32_at(data, 57)?;
    let low_w = u32::from(*data.get(61)?);
    let low_h = u32::from(*data.get(62)?);
    let depth = if minor >= 2 { u32::from(u16_at(data, 63)?.max(1)) } else { 1 };
    let faces = if flags & 0x4000 != 0 { 6 } else { 1 };
    let per_mip = (frames * faces * depth) as usize;
    // Where the big picture's mips start: a resource from 7.3 on, else
    // after the header and the small thumbnail.
    let mut start = None;
    if minor >= 3 {
        let count = u32_at(data, 68)? as usize;
        for i in 0..count {
            let at = 80 + i * 8;
            if data.get(at..at + 3)? == [0x30, 0, 0] {
                start = Some(u32_at(data, at + 4)? as usize);
            }
        }
    }
    let start = match start {
        Some(s) => s,
        None => {
            let thumb = if low_format == u32::MAX || low_w == 0 { 0 } else { mip_bytes(low_format, low_w, low_h)? };
            header + thumb
        }
    };
    // Smallest mip first; the largest is last.
    let mut at = start;
    for m in (1..mips).rev() {
        at += mip_bytes(format, width >> m, height >> m)? * per_mip;
    }
    let size = mip_bytes(format, width, height)?;
    let src = data.get(at..at + size)?;
    let (w, h) = (width as usize, height as usize);
    let rgba = match format {
        13..=15 | 20 => decode_blocks(format, src, w, h),
        _ => decode_packed(format, src)?,
    };
    Some(Rgba { width, height, pixels: rgba })
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    }

    /// A one-mip VTF 7.1 of `w`x`h` in `format` with a repeatable payload.
    fn vtf(format: u32, w: u16, h: u16) -> Vec<u8> {
        let mut d = vec![0u8; 64];
        d[0..4].copy_from_slice(b"VTF\0");
        d[8..12].copy_from_slice(&1u32.to_le_bytes());
        d[12..16].copy_from_slice(&64u32.to_le_bytes());
        d[16..18].copy_from_slice(&w.to_le_bytes());
        d[18..20].copy_from_slice(&h.to_le_bytes());
        d[24..26].copy_from_slice(&1u16.to_le_bytes());
        d[52..56].copy_from_slice(&format.to_le_bytes());
        d[56] = 1;
        d[57..61].copy_from_slice(&u32::MAX.to_le_bytes());
        let size = mip_bytes(format, u32::from(w), u32::from(h)).unwrap();
        d.extend((0..size).map(|i| (i as u32).wrapping_mul(37).wrapping_add(11) as u8));
        d
    }

    #[test]
    fn every_format_decodes_to_the_same_pixels() {
        let got: Vec<(u32, u64)> = [0u32, 1, 2, 3, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 20]
            .into_iter()
            .map(|f| (f, fnv(&decode(&vtf(f, 7, 5)).unwrap().pixels)))
            .collect();
        assert_eq!(got, vec![(0, 2315460490112276049), (1, 16814089722942423345), (2, 6853058693281578637), (3, 16464835995279100945), (5, 15849491509099224258), (6, 7038011954907754164), (8, 8742150901380183774), (9, 6853058693281578637), (10, 16464835995279100945), (11, 13852234630005815853), (12, 8178616560417763481), (13, 9615194535043858083), (14, 16363378864778755043), (15, 6140524213402932635), (16, 16200869752498453346), (20, 9615194535043858083)]);
        assert!(decode(&vtf(4, 7, 5)).is_none());
    }
}
