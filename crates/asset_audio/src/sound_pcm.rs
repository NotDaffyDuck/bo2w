//! Clips held in memory, and what picking a variant hands the player.
//!
//! Black Ops II streams every clip, so no catalog holds one of these. The
//! types stay because the clip store still has a path that reads them.

use std::sync::Arc;

use asset_core::ZoneOwner;
use asset_transport::ZoneGame;

/// The format code of plain 16-bit PCM.
pub const MSS_PCM: i32 = 1;

const ADPCM: i32 = 6;

/// A clip as it sits in memory, still in its stored encoding.
#[derive(Clone, Debug, Default)]
pub struct LoadedSoundPcm {
    pub name: String,
    pub game: ZoneGame,
    pub zone: ZoneOwner,
    pub rate: u32,
    pub samples: u32,
    pub block_size: u32,
    pub seek_table: Vec<u32>,
    format: i32,
    bits: i32,
    channels: i32,
    data: Arc<[u8]>,
}

impl LoadedSoundPcm {
    /// The stored bytes, which stay as they were read; decoded samples belong
    /// to the clip store.
    pub fn encoded_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn encoded_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }

    pub fn format(&self) -> i32 {
        self.format
    }

    pub fn bits(&self) -> i32 {
        self.bits
    }

    pub fn channels(&self) -> i32 {
        self.channels
    }

    pub fn t5_adpcm_bytes(&self) -> Option<&[u8]> {
        (self.format == ADPCM).then(|| self.encoded_bytes())
    }
}

/// A variant chosen for a sound, with the volume and pitch rolled for it.
#[derive(Clone, Debug)]
pub struct PickedSound<'a> {
    pub sound: &'a LoadedSoundPcm,
    pub variant_index: usize,
    pub volume: f32,
    pub pitch: f32,
    pub layer: Option<String>,
}

/// Which variant a pick landed on, and its in-memory clip when it has one.
#[derive(Clone, Debug)]
pub struct PickLoadedOutcome<'a> {
    pub variant_index: usize,
    pub picked: Option<PickedSound<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_adpcm_exposes_its_bytes_as_adpcm() {
        let clip = LoadedSoundPcm {
            format: ADPCM,
            data: Arc::from([1u8, 2, 3].as_slice()),
            ..LoadedSoundPcm::default()
        };
        assert_eq!(clip.t5_adpcm_bytes(), Some([1u8, 2, 3].as_slice()));
        let pcm = LoadedSoundPcm {
            format: MSS_PCM,
            ..clip
        };
        assert_eq!(pcm.t5_adpcm_bytes(), None);
    }
}
