//! The glass part of a Black Ops II snapshot and gamestate. Only each
//! pane's state travels, plus the hit position and direction of a
//! shattered pane; every client cuts its own random pieces.
//!
//! Layout (bits, low bit first): "has glass data", then "full list", then
//! a 16-bit count and the entries, then the 16-bit end marker. A full list
//! has one entry per pane in order; a delta entry starts with its 10-bit
//! pane index. Each entry is a 2-bit state, and a shattered pane adds six
//! 32-bit floats (hit position, then hit direction).

use crate::server::{PaneState, ServerPane};

/// Every glass message ends with this.
pub const END_MARKER: u16 = 0x75ca;
/// A delta entry's pane index has 10 bits.
pub const MAX_PANES: usize = 1 << 10;

/// One pane in a glass message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneUpdate {
    pub pane: u16,
    pub state: PaneState,
    /// Hit position and direction; only a shattered pane carries them.
    pub hit: Option<([f32; 3], [f32; 3])>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GlassMsg {
    /// Nothing changed since the client's last acknowledged snapshot.
    NoData,
    /// Every pane, in order (the gamestate, or when cheaper than a delta).
    Full(Vec<PaneUpdate>),
    /// Only the panes that changed.
    Delta(Vec<PaneUpdate>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    Truncated,
    /// A full list whose count is not this map's pane count.
    CountMismatch,
    /// A delta entry past the last pane.
    BadIndex,
    BadMarker,
}

fn update_of(i: usize, p: &ServerPane) -> PaneUpdate {
    PaneUpdate {
        pane: i as u16,
        state: p.state,
        hit: (p.state == PaneState::Shattered).then_some((p.impact_point, p.hit_dir)),
    }
}

/// What the server sends a client whose last acknowledged snapshot is
/// `since_ms` (0 or less: none, so the full list). The panes changed at or
/// after it go as a delta only when that is cheaper than the full list:
/// `(changed + 16 x changed shattered) x 12 < (96 x all shattered + panes) x 2`.
pub fn pick(panes: &[ServerPane], since_ms: i32) -> GlassMsg {
    let full = || GlassMsg::Full(panes.iter().enumerate().map(|(i, p)| update_of(i, p)).collect());
    if since_ms <= 0 {
        return full();
    }
    let changed: Vec<PaneUpdate> = panes
        .iter()
        .enumerate()
        .filter(|pair| pair.1.changed_ms >= since_ms)
        .map(|(i, p)| update_of(i, p))
        .collect();
    if changed.is_empty() {
        return GlassMsg::NoData;
    }
    let changed_shattered = changed.iter().filter(|u| u.hit.is_some()).count();
    let shattered = panes.iter().filter(|p| p.state == PaneState::Shattered).count();
    if (changed.len() + 16 * changed_shattered) * 12 < (96 * shattered + panes.len()) * 2 {
        GlassMsg::Delta(changed)
    } else {
        full()
    }
}

/// Every pane's update, in order (what a full list carries).
pub fn full_list(panes: &[ServerPane]) -> Vec<PaneUpdate> {
    panes.iter().enumerate().map(|(i, p)| update_of(i, p)).collect()
}

/// The message that takes a client holding `old` to `new` (both full
/// lists), with BO2's choice between a delta and the full list. A pane
/// count change (a new map) always sends the full list.
pub fn diff(old: &[PaneUpdate], new: &[PaneUpdate]) -> GlassMsg {
    if old.len() != new.len() {
        return GlassMsg::Full(new.to_vec());
    }
    let changed: Vec<PaneUpdate> = old.iter().zip(new).filter(|(a, b)| a != b).map(|(_, b)| *b).collect();
    if changed.is_empty() {
        return GlassMsg::NoData;
    }
    let changed_shattered = changed.iter().filter(|u| u.hit.is_some()).count();
    let shattered = new.iter().filter(|u| u.state == PaneState::Shattered).count();
    if (changed.len() + 16 * changed_shattered) * 12 < (96 * shattered + new.len()) * 2 {
        GlassMsg::Delta(changed)
    } else {
        GlassMsg::Full(new.to_vec())
    }
}

/// Applies a decoded message to a client's full list.
pub fn apply(list: &mut Vec<PaneUpdate>, msg: GlassMsg) {
    match msg {
        GlassMsg::NoData => {}
        GlassMsg::Full(l) => *list = l,
        GlassMsg::Delta(l) => {
            for u in l {
                if let Some(slot) = list.get_mut(u.pane as usize) {
                    *slot = u;
                }
            }
        }
    }
}

#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    at: usize,
}

impl Bits {
    fn put(&mut self, value: u32, bits: u32) {
        for b in 0..bits {
            if self.at.is_multiple_of(8) {
                self.out.push(0);
            }
            if value >> b & 1 != 0 {
                *self.out.last_mut().unwrap() |= 1 << (self.at % 8);
            }
            self.at += 1;
        }
    }
}

struct BitCursor<'a> {
    src_bytes: &'a [u8],
    at: usize,
}

impl BitCursor<'_> {
    fn get(&mut self, bits: u32) -> Result<u32, NetError> {
        let mut v = 0u32;
        for b in 0..bits {
            let byte = *self.src_bytes.get(self.at / 8).ok_or(NetError::Truncated)?;
            v |= u32::from(byte >> (self.at % 8) & 1) << b;
            self.at += 1;
        }
        Ok(v)
    }
}

/// Writes a glass message.
pub fn encode(msg: &GlassMsg) -> Vec<u8> {
    let mut w = Bits::default();
    let (full, list) = match msg {
        GlassMsg::NoData => {
            w.put(0, 1);
            w.put(u32::from(END_MARKER), 16);
            return w.out;
        }
        GlassMsg::Full(l) => (true, l),
        GlassMsg::Delta(l) => (false, l),
    };
    w.put(1, 1);
    w.put(u32::from(full), 1);
    w.put(list.len() as u32, 16);
    for u in list {
        if !full {
            w.put(u32::from(u.pane), 10);
        }
        w.put(u.state as u32, 2);
        if u.state == PaneState::Shattered {
            let (pos, dir) = u.hit.unwrap_or(([0.0; 3], [0.0; 3]));
            for f in pos.into_iter().chain(dir) {
                w.put(f.to_bits(), 32);
            }
        }
    }
    w.put(u32::from(END_MARKER), 16);
    w.out
}

/// Reads a glass message for a map with `pane_count` panes. Any error
/// drops the whole message, as BO2 does.
pub fn decode(data: &[u8], pane_count: usize) -> Result<GlassMsg, NetError> {
    let mut r = BitCursor { src_bytes: data, at: 0 };
    let msg = if r.get(1)? == 0 {
        GlassMsg::NoData
    } else {
        let full = r.get(1)? != 0;
        let count = r.get(16)? as usize;
        if full && count != pane_count {
            return Err(NetError::CountMismatch);
        }
        let mut list = Vec::with_capacity(count.min(MAX_PANES));
        for i in 0..count {
            let pane = if full { i } else { r.get(10)? as usize };
            if pane >= pane_count {
                return Err(NetError::BadIndex);
            }
            let state = PaneState::from_bits(r.get(2)? as u8);
            let hit = if state == PaneState::Shattered {
                let mut f = [0.0f32; 6];
                for v in &mut f {
                    *v = f32::from_bits(r.get(32)?);
                }
                Some(([f[0], f[1], f[2]], [f[3], f[4], f[5]]))
            } else {
                None
            };
            list.push(PaneUpdate { pane: pane as u16, state, hit });
        }
        if full { GlassMsg::Full(list) } else { GlassMsg::Delta(list) }
    };
    if r.get(16)? != u32::from(END_MARKER) {
        return Err(NetError::BadMarker);
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{PaneDef, ServerPane};

    fn panes(n: usize) -> Vec<ServerPane> {
        (0..n)
            .map(|_| ServerPane {
                def: PaneDef { hit_point_ceiling: 100.0, ..PaneDef::default() },
                health: 100.0,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn no_ack_sends_everything_and_quiet_sends_nothing() {
        let p = panes(4);
        assert!(matches!(pick(&p, 0), GlassMsg::Full(l) if l.len() == 4));
        assert_eq!(pick(&p, 500), GlassMsg::NoData);
    }

    #[test]
    fn one_change_among_many_goes_as_a_delta() {
        let mut p = panes(200);
        p[7].state = PaneState::Cracked;
        p[7].changed_ms = 900;
        assert_eq!(
            pick(&p, 800),
            GlassMsg::Delta(vec![PaneUpdate { pane: 7, state: PaneState::Cracked, hit: None }])
        );
    }

    #[test]
    fn many_shattered_changes_fall_back_to_the_full_list() {
        // 3 panes, all shattered just now: (3 + 48) x 12 = 612 vs (288 + 3) x 2 = 582.
        let mut p = panes(3);
        for q in &mut p {
            q.state = PaneState::Shattered;
            q.changed_ms = 900;
        }
        assert!(matches!(pick(&p, 800), GlassMsg::Full(_)));
    }

    #[test]
    fn messages_survive_the_wire() {
        let mut p = panes(300);
        p[3].state = PaneState::Cracked;
        p[3].changed_ms = 10;
        p[299].state = PaneState::Shattered;
        p[299].impact_point = [1.5, -2.0, 64.25];
        p[299].hit_dir = [0.0, 1.0, 0.0];
        p[299].changed_ms = 10;
        for msg in [pick(&p, 0), pick(&p, 5), GlassMsg::NoData] {
            assert_eq!(decode(&encode(&msg), 300), Ok(msg));
        }
        let delta = pick(&p, 5);
        assert!(matches!(delta, GlassMsg::Delta(_)));
    }

    #[test]
    fn a_diff_applied_gives_the_new_list() {
        let mut p = panes(40);
        let old = full_list(&p);
        assert_eq!(diff(&old, &old), GlassMsg::NoData);
        p[5].state = PaneState::Cracked;
        p[9].state = PaneState::Shattered;
        p[9].hit_dir = [1.0, 0.0, 0.0];
        let new = full_list(&p);
        let msg = diff(&old, &new);
        assert!(matches!(msg, GlassMsg::Delta(ref l) if l.len() == 2));
        let mut got = old.clone();
        apply(&mut got, decode(&encode(&msg), 40).unwrap());
        assert_eq!(got, new);
        assert!(matches!(diff(&old[..3], &new), GlassMsg::Full(_)));
    }

    #[test]
    fn bad_messages_are_dropped() {
        let p = panes(5);
        let full = encode(&pick(&p, 0));
        assert_eq!(decode(&full, 6), Err(NetError::CountMismatch));
        let mut broken = full.clone();
        // 44 bits: the last byte's low 4 bits are the marker's top.
        *broken.last_mut().unwrap() ^= 0x01;
        assert_eq!(decode(&broken, 5), Err(NetError::BadMarker));
        let d = encode(&GlassMsg::Delta(vec![PaneUpdate {
            pane: 9,
            state: PaneState::Cracked,
            hit: None,
        }]));
        assert_eq!(decode(&d, 5), Err(NetError::BadIndex));
        assert_eq!(decode(&full[..2], 5), Err(NetError::Truncated));
    }
}
