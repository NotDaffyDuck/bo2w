//! Streamed-sound containers holding MS-ADPCM: the container header is
//! stripped and the blocks are wrapped in a RIFF/WAVE header so the ordinary
//! audio decoder can read them.

use crate::pcm::{PcmAudio, decode_audio_bytes};

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
pub(crate) fn decode(bytes: &[u8]) -> Option<PcmAudio> {
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
    decode_adpcm(payload, frames, rate, channels)
}

/// Decodes raw MS-ADPCM blocks. `frames` is the true length of the sound;
/// the last block is padded, so the decoded samples are cut back to it.
pub(crate) fn decode_adpcm(data: &[u8], frames: u32, rate: u32, channels: u32) -> Option<PcmAudio> {
    if !(1..=2).contains(&channels) || rate == 0 || data.is_empty() {
        return None;
    }
    let block_bytes = BLOCK_BYTES_PER_CHANNEL * channels;
    let blocks = data.len() / block_bytes as usize;
    if data.len() % block_bytes as usize != 0 || frames == 0 {
        return None;
    }
    if u64::from(frames) > blocks as u64 * FRAMES_PER_BLOCK {
        return None;
    }

    let wav = wrap_in_wave(data, frames, rate, channels as u16, block_bytes)?;
    let mut pcm = decode_audio_bytes(&wav)?;

    let wanted = (frames as usize).checked_mul(channels as usize)?;
    if pcm.samples.len() < wanted {
        return None;
    }
    pcm.samples = pcm.samples[..wanted].into();
    Some(pcm)
}

fn wrap_in_wave(
    data: &[u8],
    frames: u32,
    rate: u32,
    channels: u16,
    block_bytes: u32,
) -> Option<Vec<u8>> {
    const FMT_CHUNK_LEN: u32 = 50;
    // "WAVE" + fmt chunk (8 + 50) + fact chunk (8 + 4) + data chunk header (8)
    const RIFF_OVERHEAD: u32 = 4 + 8 + FMT_CHUNK_LEN + 12 + 8;

    let data_len = u32::try_from(data.len()).ok()?;
    let mut wav = Vec::with_capacity(data.len() + 90);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&RIFF_OVERHEAD.checked_add(data_len)?.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&FMT_CHUNK_LEN.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes()); // WAVE_FORMAT_ADPCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    let bytes_per_second = rate.checked_mul(block_bytes)? / FRAMES_PER_BLOCK as u32;
    wav.extend_from_slice(&bytes_per_second.to_le_bytes());
    for field in [
        block_bytes as u16,
        4,                          // bits per sample
        32,                         // extra bytes that follow
        FRAMES_PER_BLOCK as u16,    // samples per block
        COEFFICIENTS.len() as u16,
    ] {
        wav.extend_from_slice(&field.to_le_bytes());
    }
    for pair in COEFFICIENTS {
        for coefficient in pair {
            wav.extend_from_slice(&coefficient.to_le_bytes());
        }
    }

    wav.extend_from_slice(b"fact");
    wav.extend_from_slice(&4_u32.to_le_bytes());
    wav.extend_from_slice(&frames.to_le_bytes());

    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(data);
    Some(wav)
}
