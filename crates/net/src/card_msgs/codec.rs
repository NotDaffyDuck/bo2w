//! The bytes of the player-card messages. Card slots and menus are a count byte and fixed-size
//! entries; splashes are a 16-bit count and length-prefixed keys.

use super::{HostCardSlotMsg, HostScreenFlash, HostMenuOpenMsg};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Splash slots run 0 to 4.
const SPLASH_SLOTS: std::ops::Range<i32> = 0..5;

fn put_count(out: &mut ByteSinkWriter, total: usize) {
    debug_assert!(total <= usize::from(u8::MAX));
    out.put_u8(total as u8);
}

pub fn pack_host_nameplates(out: &mut ByteSinkWriter, entries: &[HostCardSlotMsg]) {
    put_count(out, entries.len());
    for entry in entries {
        out.put_u32(entry.client);
        out.poke_int32(entry.card_slot);
    }
}

pub fn decode_svc_card_slots(input: &mut ByteCursorReader<'_>) -> Result<Vec<HostCardSlotMsg>, PacketDecodeFault> {
    let count = usize::from(input.get_u8()?);
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let client = input.read_word32()?;
        let shown_slot = input.get_i32()?;
        entries.push(HostCardSlotMsg { client, card_slot: shown_slot });
    }
    Ok(entries)
}

pub fn pack_host_menu_openings(out: &mut ByteSinkWriter, entries: &[HostMenuOpenMsg]) {
    put_count(out, entries.len());
    for entry in entries {
        out.poke_int32(entry.config_slot_number);
    }
}

pub fn decode_svc_open_menus(input: &mut ByteCursorReader<'_>) -> Result<Vec<HostMenuOpenMsg>, PacketDecodeFault> {
    let count = usize::from(input.get_u8()?);
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(HostMenuOpenMsg {
            config_slot_number: input.get_i32()?,
        });
    }
    Ok(entries)
}

pub fn pack_host_overlay_flashes(out: &mut ByteSinkWriter, entries: &[HostScreenFlash]) {
    out.put_u16(u16::try_from(entries.len()).expect("HUD splash count"));
    for entry in entries {
        out.put_u16(u16::try_from(entry.key.len()).expect("HUD splash key length"));
        out.append_raw_slice(entry.key.as_bytes());
        out.poke_int32(entry.card_slot);
        out.poke_int32(entry.optional);
    }
}

pub fn decode_svc_hud_splashes(input: &mut ByteCursorReader<'_>) -> Result<Vec<HostScreenFlash>, PacketDecodeFault> {
    let total = input.get_u16()?;
    let mut entries = Vec::new();
    for _ in 0..total {
        let mut key_bytes = vec![0; usize::from(input.get_u16()?)];
        input.copy_out(&mut key_bytes)?;
        let key = String::from_utf8(key_bytes).map_err(|_| PacketDecodeFault::GarbledRequest("HUD splash key UTF-8"))?;
        let splash_slot = input.get_i32()?;
        if !SPLASH_SLOTS.contains(&splash_slot) {
            return Err(PacketDecodeFault::GarbledRequest("HUD splash slot"));
        }
        let optional = input.get_i32()?;
        entries.push(HostScreenFlash {
            key,
            card_slot: splash_slot,
            optional,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splash(key: &str, slot: i32) -> HostScreenFlash {
        HostScreenFlash {
            key: key.into(),
            card_slot: slot,
            optional: -3,
        }
    }

    #[test]
    fn card_slots_are_a_count_then_client_and_slot() {
        let mut out = ByteSinkWriter::new();
        pack_host_nameplates(&mut out, &[HostCardSlotMsg { client: 0x0102_0304, card_slot: -2 }]);
        let bytes = out.into_bytes();
        assert_eq!(bytes.len(), 1 + 4 + 4);
        assert_eq!(bytes[0], 1);
        let mut input = ByteCursorReader::new(&bytes);
        assert_eq!(
            decode_svc_card_slots(&mut input),
            Ok(vec![HostCardSlotMsg { client: 0x0102_0304, card_slot: -2 }])
        );
        assert!(input.is_empty());
    }

    #[test]
    fn menus_round_trip_and_none_is_one_byte() {
        let mut out = ByteSinkWriter::new();
        pack_host_menu_openings(&mut out, &[]);
        assert_eq!(out.as_slice(), [0]);
        let menus = [HostMenuOpenMsg { config_slot_number: 5 }, HostMenuOpenMsg { config_slot_number: -1 }];
        let mut out = ByteSinkWriter::new();
        pack_host_menu_openings(&mut out, &menus);
        let bytes = out.into_bytes();
        assert_eq!(decode_svc_open_menus(&mut ByteCursorReader::new(&bytes)), Ok(menus.to_vec()));
    }

    #[test]
    fn a_truncated_list_is_an_error_not_a_short_list() {
        let mut out = ByteSinkWriter::new();
        pack_host_nameplates(&mut out, &[HostCardSlotMsg { client: 1, card_slot: 1 }; 2]);
        let bytes = out.into_bytes();
        let cut = &bytes[..bytes.len() - 1];
        assert!(decode_svc_card_slots(&mut ByteCursorReader::new(cut)).is_err());
    }

    #[test]
    fn splashes_round_trip() {
        let splashes = [splash("FIRST_BLOOD", 0), splash("", 4)];
        let mut out = ByteSinkWriter::new();
        pack_host_overlay_flashes(&mut out, &splashes);
        let bytes = out.into_bytes();
        let mut input = ByteCursorReader::new(&bytes);
        assert_eq!(decode_svc_hud_splashes(&mut input), Ok(splashes.to_vec()));
        assert!(input.is_empty());
    }

    #[test]
    fn a_splash_slot_past_four_is_malformed() {
        for slot in [-1, 5] {
            let mut out = ByteSinkWriter::new();
            pack_host_overlay_flashes(&mut out, &[splash("X", slot)]);
            let bytes = out.into_bytes();
            assert_eq!(
                decode_svc_hud_splashes(&mut ByteCursorReader::new(&bytes)),
                Err(PacketDecodeFault::GarbledRequest("HUD splash slot"))
            );
        }
    }

    #[test]
    fn a_splash_key_that_is_not_utf8_is_malformed() {
        let mut out = ByteSinkWriter::new();
        out.put_u16(1);
        out.put_u16(2);
        out.append_raw_slice(&[0xff, 0xfe]);
        out.poke_int32(0);
        out.poke_int32(0);
        let bytes = out.into_bytes();
        assert_eq!(
            decode_svc_hud_splashes(&mut ByteCursorReader::new(&bytes)),
            Err(PacketDecodeFault::GarbledRequest("HUD splash key UTF-8"))
        );
    }
}
