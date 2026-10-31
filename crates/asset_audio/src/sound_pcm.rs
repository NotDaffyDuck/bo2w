//! Clips held in memory, and what picking a variant hands the player.
//!
//! Black Ops II streams every clip, so no catalog holds one of these. The
//! types stay because the clip store still has a path that reads them.

use std::sync::Arc;

use asset_core::ZoneCustodian;
use asset_transport::ZoneTitleKind;

/// The format code of plain 16-bit PCM.
pub const RAW_WAVE_FORMAT: i32 = 1;

const ADPCM: i32 = 6;

/// A clip as it sits in memory, still in its stored encoding.
#[derive(Clone, Debug, Default)]
pub struct ResidentRawAudio {
    pub name: String,
    pub game: ZoneTitleKind,
    pub zone: ZoneCustodian,
    pub sample_rate: u32,
    pub frame_count: u32,
    pub block_bytes: u32,
    pub jump_index: Vec<u32>,
    fmt_code: i32,
    bits: i32,
    channel_count: i32,
    data: Arc<[u8]>,
}

impl ResidentRawAudio {
    /// The stored bytes, which stay as they were read; decoded samples belong
    /// to the clip store.
    pub fn encoded_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn encoded_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }

    pub fn format(&self) -> i32 {
        self.fmt_code
    }

    pub fn bits(&self) -> i32 {
        self.bits
    }

    pub fn channel_total(&self) -> i32 {
        self.channel_count
    }

    pub fn adpcm_payload(&self) -> Option<&[u8]> {
        (self.fmt_code == ADPCM).then(|| self.encoded_bytes())
    }
}

/// A variant chosen for a sound, with the volume and pitch rolled for it.
#[derive(Clone, Debug)]
pub struct ChosenSoundVariant<'a> {
    pub sound: &'a ResidentRawAudio,
    pub row: usize,
    pub gain: f32,
    pub pitch_ratio: f32,
    pub layer_name: Option<String>,
}

/// Which variant a pick landed on, and its in-memory clip when it has one.
#[derive(Clone, Debug)]
pub struct ResidentSelectionResult<'a> {
    pub chosen_row: usize,
    pub in_memory: Option<ChosenSoundVariant<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_adpcm_exposes_its_bytes_as_adpcm() {
        let clip = ResidentRawAudio {
            fmt_code: ADPCM,
            data: Arc::from([1u8, 2, 3].as_slice()),
            ..ResidentRawAudio::default()
        };
        assert_eq!(clip.adpcm_payload(), Some([1u8, 2, 3].as_slice()));
        let pcm = ResidentRawAudio {
            fmt_code: RAW_WAVE_FORMAT,
            ..clip
        };
        assert_eq!(pcm.adpcm_payload(), None);
    }
}

