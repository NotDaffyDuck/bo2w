//! Name tables the client cannot look up on its own: which sound alias, effect or material sits in
//! which configstring slot, and the text of the HUD strings.

use super::codec::{put_text8, put_text16, refuse, take_text8, take_text16};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub(super) fn put_slot_names(wr: &mut ByteSinkWriter, pairs: &[(u8, String)]) {
    debug_assert!(pairs.len() <= usize::from(u8::MAX));
    wr.put_u8(pairs.len() as u8);
    for (slot, name) in pairs {
        wr.put_u8(*slot);
        put_text8(wr, name);
    }
}

pub(super) fn take_slot_names(rd: &mut ByteCursorReader<'_>) -> Result<Vec<(u8, String)>, PacketDecodeFault> {
    let count = usize::from(rd.get_u8()?);
    let mut pairs = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = rd.get_u8()?;
        pairs.push((slot, take_text8(rd, "sound alias CS name is not utf-8")?));
    }
    Ok(pairs)
}

pub(super) fn put_hud_strings(wr: &mut ByteSinkWriter, pairs: &[(u16, String)]) {
    debug_assert!(pairs.len() <= usize::from(u16::MAX));
    wr.put_u16(pairs.len() as u16);
    for (slot, text) in pairs {
        wr.put_u16(*slot);
        put_text16(wr, text);
    }
}

pub(super) fn take_hud_strings(rd: &mut ByteCursorReader<'_>) -> Result<Vec<(u16, String)>, PacketDecodeFault> {
    let count = usize::from(rd.get_u16()?);
    if count >= sim::CS_LOCALIZED_STRINGS_SLOTS {
        return refuse("hud string count exceeds its configstrings");
    }
    let mut pairs = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = rd.get_u16()?;
        pairs.push((slot, take_text16(rd, "hud string is not utf-8")?));
    }
    Ok(pairs)
}
