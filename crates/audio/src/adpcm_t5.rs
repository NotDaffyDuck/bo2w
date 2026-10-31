//! Streamed-sound containers holding MS-ADPCM: the container header is
//! stripped and the blocks are wrapped in a RIFF/WAVE header so the ordinary
//! audio decoder can read them.

use crate::sample_feed::{ClipPcm, decode_clip_bytes};

/// One MS-ADPCM block carries this many sample frames.
const FRAMES_PER_BLOCK: u64 = 512;
/// Block size in bytes for one channel; a stereo block is twice this.
const BLOCK_BYTES_PER_CHANNEL: u32 = 262;
/// Container format tag for MS-ADPCM streams.
const CONTAINER_ADPCM: u32 = 6;
/// Smallest container header that still holds every field read below.
const MIN_HEADER: usize = 56;

/// The fixed predictor coefficient pairs every MS-ADPCM stream declares.
const COEFFICIENTS: [[i16; 2]; 7] = [
    [256, 0],
    [512, -256],
    [0, 0],
    [192, 64],
    [240, 0],
    [460, -208],
    [392, -232],
];

/// Reads a streamed-sound container and decodes its ADPCM payload.
pub(crate) fn decode(bytes: &[u8]) -> Option<ClipPcm> {
    let word = |at: usize| -> Option<u32> {
        let raw = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes(raw.try_into().ok()?))
    };

    if word(0)? != 1 || word(28)? != CONTAINER_ADPCM {
        return None;
    }
    let frames = word(4)?;
    let rate = word(8)?;
    let channels = word(12)?;
    let data_start = word(16)? as usize;
    let data_len = word(48)? as usize;
    if data_start < MIN_HEADER {
        return None;
    }
    let payload = bytes.get(data_start..data_start.checked_add(data_len)?)?;
    decode_xbadpcm(payload, frames, rate, channels)
}

/// Decodes raw MS-ADPCM blocks. `frames` is the true length of the sound;
/// the last block is padded, so the decoded samples are cut back to it.
pub(crate) fn decode_xbadpcm(data: &[u8], frame_ct: u32, rate_hz: u32, channels: u32) -> Option<ClipPcm> {
    if !(1..=2).contains(&channels) || rate_hz == 0 || data.is_empty() {
        return None;
    }
    let block_bytes = BLOCK_BYTES_PER_CHANNEL * channels;
    let blocks = data.len() / block_bytes as usize;
    if data.len() % block_bytes as usize != 0 || frame_ct == 0 {
        return None;
    }
    if u64::from(frame_ct) > blocks as u64 * FRAMES_PER_BLOCK {
        return None;
    }

    let wav = wrap_in_wave(data, frame_ct, rate_hz, channels as u16, block_bytes)?;
    let mut pcm = decode_clip_bytes(&wav)?;

    let wanted = (frame_ct as usize).checked_mul(channels as usize)?;
    if pcm.data.len() < wanted {
        return None;
    }
    pcm.data = pcm.data[..wanted].into();
    Some(pcm)
}

fn wrap_in_wave(
    data: &[u8],
    n_frames: u32,
    rate: u32,
    chan_count: u16,
    block_bytes: u32,
) -> Option<Vec<u8>> {
    const FMT_CHUNK_LEN: u32 = 50;
    // "WAVE" + fmt chunk (8 + 50) + fact chunk (8 + 4) + data chunk header (8)
    const RIFF_OVERHEAD: u32 = 4 + 8 + FMT_CHUNK_LEN + 12 + 8;

    let data_len = u32::try_from(data.len()).ok()?;
    let riff_len = RIFF_OVERHEAD.checked_add(data_len)?;
    let bytes_per_second = rate.checked_mul(block_bytes)? / FRAMES_PER_BLOCK as u32;

    // The fmt chunk body: ADPCM header fields, then the coefficient table.
    let mut fmt = Vec::with_capacity(FMT_CHUNK_LEN as usize);
    let halves = [2_u16 /* WAVE_FORMAT_ADPCM */, chan_count];
    fmt.extend(halves.iter().flat_map(|h| h.to_le_bytes()));
    fmt.extend([rate, bytes_per_second].iter().flat_map(|w| w.to_le_bytes()));
    // Block align, bits per sample, extra bytes, samples per block, coefficient count.
    let tail = [block_bytes as u16, 4, 32, FRAMES_PER_BLOCK as u16, COEFFICIENTS.len() as u16];
    fmt.extend(tail.iter().flat_map(|h| h.to_le_bytes()));
    fmt.extend(COEFFICIENTS.iter().flatten().flat_map(|c| c.to_le_bytes()));

    let mut wav = Vec::with_capacity(data.len() + 90);
    let mut chunk = |tag: &[u8; 4], len: u32, body: &[u8]| {
        wav.extend_from_slice(tag);
        wav.extend_from_slice(&len.to_le_bytes());
        wav.extend_from_slice(body);
    };
    chunk(b"RIFF", riff_len, b"WAVE");
    chunk(b"fmt ", FMT_CHUNK_LEN, &fmt);
    chunk(b"fact", 4, &n_frames.to_le_bytes());
    chunk(b"data", data_len, data);
    Some(wav)
}
