//! Sounds the server tells one client to play or stop locally, and how they ride a frame.

mod pending;

use bevy::prelude::Message;

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub use pending::StagedHostSounds;

/// Wire tag for "play this sound only for this client" (ASCII `p`).
pub const HOST_SOUND_START_TAG: u8 = 0x70;

/// Wire tag for "stop this sound for this client" (ASCII `h`).
pub const HOST_SOUND_HALT_TAG: u8 = 0x68;

/// One play or stop of a sound, by its index in the sound-alias table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostSoundCue {
    pub halt: bool,
    pub slot: u8,
}

impl HostSoundCue {
    pub fn start(slot: u8) -> Self {
        Self { halt: false, slot }
    }

    pub fn silence(slot: u8) -> Self {
        Self { halt: true, slot }
    }

    pub fn tag_byte(self) -> u8 {
        if self.halt { HOST_SOUND_HALT_TAG } else { HOST_SOUND_START_TAG }
    }

    pub fn from_wire_tag(letter: u8, slot: u8) -> Result<Self, PacketDecodeFault> {
        match letter {
            HOST_SOUND_START_TAG => Ok(Self::start(slot)),
            HOST_SOUND_HALT_TAG => Ok(Self::silence(slot)),
            _ => Err(PacketDecodeFault::GarbledRequest("svc sound tag is not 'p' or 'h'")),
        }
    }
}

/// A received sound command, handed to the client's audio side.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostLocalSoundMsg {
    pub stop: bool,
    pub index: u8,
}

/// Write a count byte, then a tag byte and an index byte per command.
pub fn pack_host_sound_triggers(sink: &mut ByteSinkWriter, commands: &[HostSoundCue]) {
    let cue_total = commands.len();
    debug_assert!(cue_total <= usize::from(u8::MAX));
    sink.put_u8(cue_total as u8);
    for cue in commands {
        sink.put_u8(cue.tag_byte());
        sink.put_u8(cue.slot);
    }
}

pub fn decode_svc_sounds(input: &mut ByteCursorReader<'_>) -> Result<Vec<HostSoundCue>, PacketDecodeFault> {
    let count = usize::from(input.get_u8()?);
    (0..count)
        .map(|_| {
            let (kind, slot) = (input.get_u8()?, input.get_u8()?);
            HostSoundCue::from_wire_tag(kind, slot)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of(commands: &[HostSoundCue]) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        pack_host_sound_triggers(&mut sink, commands);
        sink.into_bytes()
    }

    #[test]
    fn the_tags_are_the_letters_p_and_h() {
        assert_eq!(HOST_SOUND_START_TAG, b'p');
        assert_eq!(HOST_SOUND_HALT_TAG, b'h');
        assert_eq!(HostSoundCue::start(1).tag_byte(), b'p');
        assert_eq!(HostSoundCue::silence(1).tag_byte(), b'h');
    }

    #[test]
    fn a_tag_and_index_rebuild_the_command() {
        assert_eq!(HostSoundCue::from_wire_tag(b'p', 9), Ok(HostSoundCue::start(9)));
        assert_eq!(HostSoundCue::from_wire_tag(b'h', 9), Ok(HostSoundCue::silence(9)));
    }

    #[test]
    fn any_other_tag_is_malformed() {
        assert_eq!(
            HostSoundCue::from_wire_tag(b'x', 9),
            Err(PacketDecodeFault::GarbledRequest("svc sound tag is not 'p' or 'h'"))
        );
    }

    #[test]
    fn the_wire_layout_is_count_then_tag_index_pairs() {
        let bytes = bytes_of(&[HostSoundCue::start(3), HostSoundCue::silence(200)]);
        assert_eq!(bytes, [2, b'p', 3, b'h', 200]);
    }

    #[test]
    fn an_empty_list_is_one_zero_byte() {
        assert_eq!(bytes_of(&[]), [0]);
        let mut reader = ByteCursorReader::new(&[0]);
        assert_eq!(decode_svc_sounds(&mut reader), Ok(Vec::new()));
        assert!(reader.is_empty());
    }

    #[test]
    fn commands_survive_a_round_trip_in_order() {
        let commands = [HostSoundCue::silence(7), HostSoundCue::start(7), HostSoundCue::start(255)];
        let bytes = bytes_of(&commands);
        let mut reader = ByteCursorReader::new(&bytes);
        assert_eq!(decode_svc_sounds(&mut reader), Ok(commands.to_vec()));
        assert!(reader.is_empty());
    }

    #[test]
    fn a_bad_tag_inside_a_list_fails_the_whole_decode() {
        let mut reader = ByteCursorReader::new(&[2, b'p', 1, b'z', 2]);
        assert_eq!(
            decode_svc_sounds(&mut reader),
            Err(PacketDecodeFault::GarbledRequest("svc sound tag is not 'p' or 'h'"))
        );
    }

    #[test]
    fn a_list_cut_short_is_truncated() {
        let mut reader = ByteCursorReader::new(&[2, b'p', 1, b'h']);
        assert!(matches!(
            decode_svc_sounds(&mut reader),
            Err(PacketDecodeFault::ShortRead { .. })
        ));
    }
}
