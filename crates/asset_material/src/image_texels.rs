//! Reading the pixels an image entry holds into bytes a GPU upload takes.
//!
//! Block-compressed entries (DXT1, DXT3, DXT5) are kept as they are. The
//! uncompressed ones are widened to RGBA8.

/// How the bytes of a decoded image are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Rgba8,
    Bc1,
    Bc2,
    Bc3,
}

/// The one mip level an entry decodes to.
#[derive(Debug)]
pub(crate) struct Texels {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub bytes: Vec<u8>,
}

/// What an entry stores, by its D3D format code.
#[derive(Clone, Copy)]
enum Stored {
    Bgra8,
    Bgrx8,
    L8,
    A8,
    La8,
    Block(Layout),
}

impl Stored {
    fn of(d3d_format: u32) -> Result<Self, String> {
        let fourcc = |tag: &[u8; 4]| u32::from_le_bytes(*tag);
        Ok(match d3d_format {
            f if f == fourcc(b"DXT1") => Self::Block(Layout::Bc1),
            f if f == fourcc(b"DXT3") => Self::Block(Layout::Bc2),
            f if f == fourcc(b"DXT5") => Self::Block(Layout::Bc3),
            21 => Self::Bgra8,
            22 => Self::Bgrx8,
            28 => Self::A8,
            50 => Self::L8,
            51 => Self::La8,
            other => return Err(format!("unsupported D3D format {other}")),
        })
    }
}

/// The first `len` bytes of `data`, or the reason there are not that many.
fn leading<'a>(data: &'a [u8], len: usize, what: &str) -> Result<&'a [u8], String> {
    data.get(..len).ok_or_else(|| format!("truncated {what}"))
}

impl Layout {
    /// Bytes a block-compressed level of this size occupies in this layout.
    fn level_bytes(self, width: u32, height: u32) -> usize {
        let block = if self == Self::Bc1 { 8 } else { 16 };
        width.div_ceil(4) as usize * height.div_ceil(4) as usize * block
    }
}

/// Widens each `step`-byte source pixel to four bytes with `expand`.
fn widen(source: &[u8], pixel_bytes: usize, expand: impl Fn(&[u8]) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len() / pixel_bytes * 4);
    for pixel in source.chunks_exact(pixel_bytes) {
        out.extend_from_slice(&expand(pixel));
    }
    out
}

pub(crate) fn decode(
    data: &[u8],
    width: u32,
    height: u32,
    d3d_format: u32,
) -> Result<Texels, String> {
    let stored = Stored::of(d3d_format)?;
    if let Stored::Block(layout) = stored {
        let bytes = leading(data, layout.level_bytes(width, height), "BC image")?.to_vec();
        return Ok(Texels {
            width,
            height,
            layout,
            bytes,
        });
    }
    if width == 0 || height == 0 {
        return Err("zero-sized image".into());
    }
    let count = width as usize * height as usize;
    let bytes = match stored {
        Stored::Bgra8 => widen(leading(data, count * 4, "RGBA8")?, 4, |p| {
            [p[2], p[1], p[0], p[3]]
        }),
        Stored::Bgrx8 => widen(leading(data, count * 4, "RGBA8")?, 4, |p| {
            [p[2], p[1], p[0], 255]
        }),
        Stored::L8 => widen(leading(data, count, "L8/A8")?, 1, |p| [p[0], p[0], p[0], 255]),
        Stored::A8 => widen(leading(data, count, "L8/A8")?, 1, |p| [255, 255, 255, p[0]]),
        Stored::La8 => widen(leading(data, count * 2, "LA8")?, 2, |p| [p[0], p[0], p[0], p[1]]),
        Stored::Block(_) => unreachable!("block formats return above"),
    };
    Ok(Texels {
        width,
        height,
        layout: Layout::Rgba8,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BGRA: u32 = 21;
    const BGRX: u32 = 22;
    const ALPHA: u32 = 28;
    const LUMA: u32 = 50;
    const LUMA_ALPHA: u32 = 51;

    fn dxt(tag: &[u8; 4]) -> u32 {
        u32::from_le_bytes(*tag)
    }

    #[test]
    fn bgra_is_swapped_to_rgba() {
        let texels = decode(&[1, 2, 3, 4], 1, 1, BGRA).unwrap();
        assert_eq!(texels.bytes, [3, 2, 1, 4]);
        assert_eq!(texels.layout, Layout::Rgba8);
    }

    #[test]
    fn bgrx_forces_opaque() {
        let texels = decode(&[1, 2, 3, 0], 1, 1, BGRX).unwrap();
        assert_eq!(texels.bytes, [3, 2, 1, 255]);
    }

    #[test]
    fn single_channel_formats_widen() {
        assert_eq!(decode(&[9], 1, 1, LUMA).unwrap().bytes, [9, 9, 9, 255]);
        assert_eq!(decode(&[9], 1, 1, ALPHA).unwrap().bytes, [255, 255, 255, 9]);
        assert_eq!(decode(&[5, 7], 1, 1, LUMA_ALPHA).unwrap().bytes, [5, 5, 5, 7]);
    }

    #[test]
    fn block_data_is_kept_whole() {
        let texels = decode(&[0; 16], 5, 4, dxt(b"DXT1")).unwrap();
        assert_eq!(texels.layout, Layout::Bc1);
        assert_eq!(texels.bytes.len(), 16);
        assert_eq!(decode(&[0; 32], 4, 8, dxt(b"DXT5")).unwrap().bytes.len(), 32);
    }

    #[test]
    fn short_or_unknown_data_is_refused() {
        assert_eq!(decode(&[0; 3], 1, 1, BGRA).unwrap_err(), "truncated RGBA8");
        assert_eq!(decode(&[0; 7], 4, 4, dxt(b"DXT1")).unwrap_err(), "truncated BC image");
        assert_eq!(decode(&[], 0, 4, LUMA).unwrap_err(), "zero-sized image");
        assert_eq!(decode(&[0], 1, 1, 99).unwrap_err(), "unsupported D3D format 99");
    }
}
