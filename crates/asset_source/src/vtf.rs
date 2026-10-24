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

/// Decodes a VTF file.
pub fn decode(data: &[u8]) -> Option<Rgba> {
    if data.get(0..4)? != b"VTF\0" {
        return None;
    }
    let minor = u32_at(data, 8)?;
    let header = u32_at(data, 12)? as usize;
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
    let mut pixels = vec![0u8; w * h * 4];
    match format {
        13 | 14 | 15 | 20 => {
            let mut tile = [0u8; 64];
            let block = if matches!(format, 13 | 20) { 8 } else { 16 };
            for by in 0..h.div_ceil(4) {
                for bx in 0..w.div_ceil(4) {
                    let o = (by * w.div_ceil(4) + bx) * block;
                    let b = &src[o..o + block];
                    match format {
                        14 => bcdec_rs::bc2(b, &mut tile, 16),
                        15 => bcdec_rs::bc3(b, &mut tile, 16),
                        _ => bcdec_rs::bc1(b, &mut tile, 16),
                    }
                    for y in 0..4 {
                        for x in 0..4 {
                            let (px, py) = (bx * 4 + x, by * 4 + y);
                            if px < w && py < h {
                                let d = (py * w + px) * 4;
                                let s = (y * 4 + x) * 4;
                                pixels[d..d + 4].copy_from_slice(&tile[s..s + 4]);
                            }
                        }
                    }
                }
            }
        }
        _ => {
            for i in 0..w * h {
                let p = &mut pixels[i * 4..i * 4 + 4];
                let rgba: [u8; 4] = match format {
                    0 => src[i * 4..i * 4 + 4].try_into().ok()?,
                    1 => [src[i * 4 + 3], src[i * 4 + 2], src[i * 4 + 1], src[i * 4]],
                    2 | 9 => [src[i * 3], src[i * 3 + 1], src[i * 3 + 2], 255],
                    3 | 10 => [src[i * 3 + 2], src[i * 3 + 1], src[i * 3], 255],
                    11 => [src[i * 4 + 1], src[i * 4 + 2], src[i * 4 + 3], src[i * 4]],
                    12 => [src[i * 4 + 2], src[i * 4 + 1], src[i * 4], src[i * 4 + 3]],
                    16 => [src[i * 4 + 2], src[i * 4 + 1], src[i * 4], 255],
                    5 => [src[i], src[i], src[i], 255],
                    8 => [255, 255, 255, src[i]],
                    6 => [src[i * 2], src[i * 2], src[i * 2], src[i * 2 + 1]],
                    _ => return None,
                };
                p.copy_from_slice(&rgba);
            }
        }
    }
    Some(Rgba { width, height, pixels })
}
