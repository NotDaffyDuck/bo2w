//! Name tables the client cannot look up on its own: which sound alias, effect or material sits in
//! which configstring slot, and the text of the HUD strings.

use super::codec::{Decoded, put_text8, put_text16, refuse, take_text8, take_text16};
use crate::transport::wire::{WireReader, WireWriter};

pub(super) fn put_slot_names(out: &mut WireWriter, rows: &[(u8, String)]) {
    debug_assert!(rows.len() <= usize::from(u8::MAX));
    out.put_u8(rows.len() as u8);
    for (slot, name) in rows {
        out.put_u8(*slot);
        put_text8(out, name);
    }
}

pub(super) fn take_slot_names(input: &mut WireReader<'_>) -> Decoded<Vec<(u8, String)>> {
    let count = usize::from(input.get_u8()?);
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = input.get_u8()?;
        rows.push((slot, take_text8(input, "sound alias CS name is not utf-8")?));
    }
    Ok(rows)
}

pub(super) fn put_hud_strings(out: &mut WireWriter, rows: &[(u16, String)]) {
    debug_assert!(rows.len() <= usize::from(u16::MAX));
    out.put_u16(rows.len() as u16);
    for (slot, text) in rows {
        out.put_u16(*slot);
        put_text16(out, text);
    }
}

pub(super) fn take_hud_strings(input: &mut WireReader<'_>) -> Decoded<Vec<(u16, String)>> {
    let count = usize::from(input.get_u16()?);
    if count >= sim::CS_LOCALIZED_STRINGS_SLOTS {
        return refuse("hud string count exceeds its configstrings");
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = input.get_u16()?;
        rows.push((slot, take_text16(input, "hud string is not utf-8")?));
    }
    Ok(rows)
}
