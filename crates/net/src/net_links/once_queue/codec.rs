//! The reliable part of a packet as bytes: the confirm_receipt, then the rows (each with its sequence number
//! and a tag naming its kind), then how many rows were refused.

use super::queue::{GUARANTEED_BACKLOG_LIMIT, GuaranteedEnvelope};
use super::row::{ActionRulingNotice, GuaranteedEntry};
use crate::net_links::snapshot_wire::{read_occurrence, pack_occurrence};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

const TAG_EVENT: u8 = 0;
const TAG_OUTCOME: u8 = 1;
const TAG_SOUND: u8 = 2;
const TAG_CARD: u8 = 3;
const TAG_MENU: u8 = 4;
const TAG_SPLASH: u8 = 5;
const TAG_NOTIFY: u8 = 6;
const TAG_SCORES: u8 = 7;
const TAG_FAILURE: u8 = 8;

fn encode_row(sink: &mut ByteSinkWriter, entry: &GuaranteedEntry) {
    match entry {
        GuaranteedEntry::Event(event) => {
            sink.put_u8(TAG_EVENT);
            pack_occurrence(sink, event);
        }
        GuaranteedEntry::CommandResolution {
            request_id,
            verdict,
        } => {
            sink.put_u8(TAG_OUTCOME);
            sink.put_u32(*request_id);
            sink.put_u8(verdict.tag());
        }
        GuaranteedEntry::Sound(value) => {
            sink.put_u8(TAG_SOUND);
            crate::local_sound_msgs::pack_host_sound_triggers(sink, std::slice::from_ref(value));
        }
        GuaranteedEntry::Card(value) => {
            sink.put_u8(TAG_CARD);
            crate::card_msgs::pack_host_nameplates(sink, std::slice::from_ref(value));
        }
        GuaranteedEntry::Menu(value) => {
            sink.put_u8(TAG_MENU);
            crate::card_msgs::pack_host_menu_openings(sink, std::slice::from_ref(value));
        }
        GuaranteedEntry::Splash(value) => {
            sink.put_u8(TAG_SPLASH);
            crate::card_msgs::pack_host_overlay_flashes(sink, std::slice::from_ref(value));
        }
        GuaranteedEntry::Notify(value) => {
            sink.put_u8(TAG_NOTIFY);
            crate::notice_msgs::pack_host_game_alerts(sink, std::slice::from_ref(value));
        }
        GuaranteedEntry::Standings(text) => {
            sink.put_u8(TAG_SCORES);
            crate::board_msgs::encode_svc_scores(sink, Some(text));
        }
        GuaranteedEntry::RefusalText(text) => {
            sink.put_u8(TAG_FAILURE);
            crate::board_msgs::encode_svc_scores(sink, Some(text));
        }
    }
}

/// Write the reliable part of a packet.
pub fn pack_guaranteed_body(
    sink: &mut ByteSinkWriter,
    confirmed_up_to: u16,
    owed: &[(u16, GuaranteedEntry)],
    evicted_stale_count: u32,
) {
    debug_assert!(owed.len() <= usize::from(u16::MAX));
    sink.put_u16(confirmed_up_to);
    sink.put_u16(owed.len() as u16);
    for (seq_no, entry) in owed {
        sink.put_u16(*seq_no);
        encode_row(sink, entry);
    }
    sink.put_u32(evicted_stale_count);
}

/// The one value a control row carries; the control codecs read lists, and a row holds exactly one.
fn only<T>(mut values: Vec<T>) -> Result<T, PacketDecodeFault> {
    if values.len() == 1 {
        Ok(values.remove(0))
    } else {
        Err(PacketDecodeFault::GarbledRequest("control row count"))
    }
}

fn decode_row(input: &mut ByteCursorReader<'_>) -> Result<GuaranteedEntry, PacketDecodeFault> {
    Ok(match input.get_u8()? {
        TAG_EVENT => GuaranteedEntry::Event(read_occurrence(input)?),
        TAG_OUTCOME => {
            let request_id = input.read_word32()?;
            let verdict = ActionRulingNotice::from_tag(input.get_u8()?)
                .ok_or(PacketDecodeFault::GarbledRequest("unknown ActionVerdict tag"))?;
            GuaranteedEntry::CommandResolution {
                request_id,
                verdict,
            }
        }
        TAG_SOUND => GuaranteedEntry::Sound(only(crate::local_sound_msgs::decode_svc_sounds(input)?)?),
        TAG_CARD => GuaranteedEntry::Card(only(crate::card_msgs::decode_svc_card_slots(input)?)?),
        TAG_MENU => GuaranteedEntry::Menu(only(crate::card_msgs::decode_svc_open_menus(input)?)?),
        TAG_SPLASH => {
            GuaranteedEntry::Splash(only(crate::card_msgs::decode_svc_hud_splashes(input)?)?)
        }
        TAG_NOTIFY => {
            GuaranteedEntry::Notify(only(crate::notice_msgs::parse_host_game_alerts(input)?)?)
        }
        TAG_SCORES => GuaranteedEntry::Standings(
            crate::board_msgs::parse_scoreboard_message(input)?
                .ok_or(PacketDecodeFault::GarbledRequest("missing scores"))?,
        ),
        TAG_FAILURE => GuaranteedEntry::RefusalText(
            crate::board_msgs::parse_scoreboard_message(input)?
                .ok_or(PacketDecodeFault::GarbledRequest("missing failure"))?,
        ),
        _ => return Err(PacketDecodeFault::GarbledRequest("unknown ReliableRow tag")),
    })
}

/// Read the reliable part of a packet.
pub fn unpack_guaranteed_body(input: &mut ByteCursorReader<'_>) -> Result<GuaranteedEnvelope, PacketDecodeFault> {
    let confirmed_up_to = input.get_u16()?;
    let count = usize::from(input.get_u16()?);
    // The count comes off the wire, so it only sizes the vector as far as a queue could hold.
    let mut owed = Vec::with_capacity(count.min(GUARANTEED_BACKLOG_LIMIT));
    for _ in 0..count {
        let number = input.get_u16()?;
        owed.push((number, decode_row(input)?));
    }
    let evicted_stale_count = input.read_word32()?;
    Ok(GuaranteedEnvelope {
        confirmed_up_to,
        entries: owed,
        evicted_stale_count,
    })
}

#[cfg(test)]
mod tests {
    use sim::{ClassId, MatchOccurrence};

    use super::*;
    use crate::{HostCardSlotMsg, HostGameAnnouncement, HostScreenFlash, HostMenuOpenMsg, HostSoundCue};

    fn wire_of(through_seq: u16, owed: &[(u16, GuaranteedEntry)], dropped: u32) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        pack_guaranteed_body(&mut sink, through_seq, owed, dropped);
        sink.into_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<GuaranteedEnvelope, PacketDecodeFault> {
        unpack_guaranteed_body(&mut ByteCursorReader::new(bytes))
    }

    fn one_of_each() -> Vec<(u16, GuaranteedEntry)> {
        vec![
            (
                1,
                GuaranteedEntry::Event(MatchOccurrence::LoadoutTaken {
                    request_id: 7,
                    class_id: ClassId(2),
                    revision: 3,
                }),
            ),
            (
                2,
                GuaranteedEntry::CommandResolution {
                    request_id: 9,
                    verdict: ActionRulingNotice::DuplicateIdDifferentBody,
                },
            ),
            (3, GuaranteedEntry::Sound(HostSoundCue::start(12))),
            (4, GuaranteedEntry::Sound(HostSoundCue::silence(13))),
            (5, GuaranteedEntry::Card(HostCardSlotMsg { client: 4, card_slot: -1 })),
            (6, GuaranteedEntry::Menu(HostMenuOpenMsg { config_slot_number: 321 })),
            (
                7,
                GuaranteedEntry::Splash(HostScreenFlash {
                    key: "MP_SPLASH".to_owned(),
                    card_slot: 2,
                    optional: -5,
                }),
            ),
            (
                8,
                GuaranteedEntry::Notify(HostGameAnnouncement {
                    id: 99,
                    tag: b'f',
                    name: "kill".to_owned(),
                    key: "streak".to_owned(),
                }),
            ),
            (9, GuaranteedEntry::Standings("a\tb\t1".to_owned())),
            (10, GuaranteedEntry::RefusalText("no room".to_owned())),
        ]
    }

    #[test]
    fn every_kind_of_row_survives_the_wire() {
        let owed = one_of_each();
        let decoded = decode(&wire_of(77, &owed, 5)).expect("decodes");
        assert_eq!(decoded.confirmed_up_to, 77);
        assert_eq!(decoded.evicted_stale_count, 5);
        assert_eq!(decoded.entries, owed);
    }

    #[test]
    fn a_packet_with_no_rows_is_the_ack_the_count_and_the_drop_count() {
        let mut expected = ByteSinkWriter::new();
        expected.put_u16(3);
        expected.put_u16(0);
        expected.put_u32(0);
        assert_eq!(wire_of(3, &[], 0), expected.into_bytes());
        assert_eq!(decode(&wire_of(3, &[], 0)).unwrap(), GuaranteedEnvelope {
            confirmed_up_to: 3,
            entries: Vec::new(),
            evicted_stale_count: 0,
        });
    }

    #[test]
    fn an_outcome_row_is_sequence_tag_request_and_verdict() {
        let entry = GuaranteedEntry::CommandResolution {
            request_id: 0xAABB,
            verdict: ActionRulingNotice::TimedOut,
        };
        let mut expected = ByteSinkWriter::new();
        expected.put_u16(0);
        expected.put_u16(1);
        expected.put_u16(40);
        expected.put_u8(1);
        expected.put_u32(0xAABB);
        expected.put_u8(3);
        expected.put_u32(0);
        assert_eq!(wire_of(0, &[(40, entry)], 0), expected.into_bytes());
    }

    #[test]
    fn the_tag_on_the_wire_names_the_kind_of_row() {
        let tags: Vec<(u8, GuaranteedEntry)> = vec![
            (TAG_SOUND, GuaranteedEntry::Sound(HostSoundCue::start(1))),
            (TAG_CARD, GuaranteedEntry::Card(HostCardSlotMsg { client: 0, card_slot: 0 })),
            (TAG_MENU, GuaranteedEntry::Menu(HostMenuOpenMsg { config_slot_number: 0 })),
            (TAG_SCORES, GuaranteedEntry::Standings(String::new())),
            (TAG_FAILURE, GuaranteedEntry::RefusalText(String::new())),
        ];
        assert_eq!(
            [TAG_EVENT, TAG_OUTCOME, TAG_SOUND, TAG_CARD, TAG_MENU, TAG_SPLASH, TAG_NOTIFY, TAG_SCORES, TAG_FAILURE],
            [0, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        for (tag, entry) in tags {
            let bytes = wire_of(0, &[(1, entry)], 0);
            // confirm_receipt (2) + count (2) + sequence (2), then the tag.
            assert_eq!(bytes[6], tag);
        }
    }

    #[test]
    fn scores_and_failure_carry_the_same_text_under_different_tags() {
        let scores = wire_of(0, &[(1, GuaranteedEntry::Standings("x".to_owned()))], 0);
        let failure = wire_of(0, &[(1, GuaranteedEntry::RefusalText("x".to_owned()))], 0);
        assert_eq!(scores[6], TAG_SCORES);
        assert_eq!(failure[6], TAG_FAILURE);
        assert_eq!(scores[7..], failure[7..]);
    }

    #[test]
    fn an_unknown_row_tag_is_malformed() {
        let mut sink = ByteSinkWriter::new();
        sink.put_u16(0);
        sink.put_u16(1);
        sink.put_u16(1);
        sink.put_u8(200);
        sink.put_u32(0);
        let coded_bytes = sink.into_bytes();
        assert!(matches!(
            decode(&coded_bytes),
            Err(PacketDecodeFault::GarbledRequest("unknown ReliableRow tag"))
        ));
    }

    #[test]
    fn an_unknown_verdict_is_malformed() {
        let mut sink = ByteSinkWriter::new();
        sink.put_u16(0);
        sink.put_u16(1);
        sink.put_u16(1);
        sink.put_u8(TAG_OUTCOME);
        sink.put_u32(5);
        sink.put_u8(9);
        sink.put_u32(0);
        let coded_bytes = sink.into_bytes();
        assert!(matches!(
            decode(&coded_bytes),
            Err(PacketDecodeFault::GarbledRequest("unknown ActionVerdict tag"))
        ));
    }

    #[test]
    fn a_control_row_holding_two_values_is_malformed() {
        let mut sink = ByteSinkWriter::new();
        sink.put_u16(0);
        sink.put_u16(1);
        sink.put_u16(1);
        sink.put_u8(TAG_SOUND);
        crate::local_sound_msgs::pack_host_sound_triggers(&mut sink, &[HostSoundCue::start(1), HostSoundCue::start(2)]);
        sink.put_u32(0);
        let coded_bytes = sink.into_bytes();
        assert!(matches!(
            decode(&coded_bytes),
            Err(PacketDecodeFault::GarbledRequest("control row count"))
        ));
    }

    #[test]
    fn every_cut_short_packet_is_an_error_and_never_a_panic() {
        let bytes = wire_of(77, &one_of_each(), 5);
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len]).is_err(), "{len} bytes decoded");
        }
    }

    #[test]
    fn a_huge_row_count_on_a_short_packet_fails_without_a_huge_allocation() {
        let mut sink = ByteSinkWriter::new();
        sink.put_u16(0);
        sink.put_u16(u16::MAX);
        let bytes = sink.into_bytes();
        assert!(decode(&bytes).is_err());
    }
}
