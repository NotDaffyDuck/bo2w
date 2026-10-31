//! Lobby gamertags for the bots: 1000 made-up names in the style of a PS3-era lobby, some with a clan tag (a
//! pick over BO2's own `[3arc]` names). A session deals them from a random starting point with a fixed prime stride,
//! so no name is dealt twice until the whole list has gone round.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Prime and larger than the list, so stepping by it touches every name once per lap.
const STRIDE: usize = 7919;

fn list() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| include_str!("bot_names.txt").lines().filter(|name| !name.is_empty()).collect())
}

/// The session's starting point, from the clock the first time a name is dealt.
fn origin() -> usize {
    static ORIGIN: OnceLock<usize> = OnceLock::new();
    *ORIGIN.get_or_init(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        now.map_or(0, |since| since.subsec_nanos() as usize ^ since.as_secs() as usize)
    })
}

/// Index into a list of `len` names for the `turn`-th deal from `origin`.
fn slot(len: usize, origin: usize, turn: usize) -> usize {
    origin.wrapping_add(turn.wrapping_mul(STRIDE)) % len
}

/// The next gamertag.
pub(crate) fn deal() -> String {
    static TURN: AtomicUsize = AtomicUsize::new(0);
    let names = list();
    names[slot(names.len(), origin(), TURN.fetch_add(1, Ordering::Relaxed))].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_a_thousand_real_names() {
        assert_eq!(list().len(), 1000);
        assert!(list().iter().all(|name| !name.trim().is_empty()));
    }

    #[test]
    fn one_lap_deals_every_name_once() {
        let len = list().len();
        for origin in [0, 1, 999, 123_456_789] {
            let mut seen = vec![false; len];
            for turn in 0..len {
                let at = slot(len, origin, turn);
                assert!(!seen[at], "origin {origin} turn {turn} dealt {at} again");
                seen[at] = true;
            }
        }
    }

    #[test]
    fn consecutive_deals_differ() {
        assert_ne!(deal(), deal());
    }
}
