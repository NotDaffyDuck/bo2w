//! ULIDs: the 26-character ids clip folders are named by.
//!
//! An id is 48 bits of milliseconds since the Unix epoch followed by 80 random
//! bits, written in Crockford base32 (digits and capitals without I, L, O and
//! U). That makes ids sort by creation time as plain text. Within one process
//! they also strictly increase: an id made in the same millisecond as the
//! last one (or after the clock stepped back) is the last one plus one.

use std::sync::Mutex;

/// Crockford's base32 digits: 0-9, then A-Z less I, L, O and U.
const CROCKFORD: [u8; 32] = crockford_digits();
const ID_LEN: usize = 26;
const RANDOM_BITS: u32 = 80;
const MAX_MILLIS: u64 = (1 << 48) - 1;
const MAX_RANDOM: u128 = (1 << RANDOM_BITS) - 1;

const fn crockford_digits() -> [u8; 32] {
    let mut table = [0u8; 32];
    let mut filled = 0;
    let mut code = b'0';
    while filled < table.len() {
        let gap = code > b'9' && code < b'A';
        let banned = matches!(code, b'I' | b'L' | b'O' | b'U');
        if !gap && !banned {
            table[filled] = code;
            filled += 1;
        }
        code += 1;
    }
    table
}

#[derive(Debug)]
pub enum SortableIdFault {
    ClockPrecedesOrigin,
    Entropy(String),
}

impl core::fmt::Display for SortableIdFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SortableIdFault::ClockPrecedesOrigin => f.write_str("system clock is before UNIX epoch"),
            SortableIdFault::Entropy(why) => write!(f, "CSPRNG failed: {why}"),
        }
    }
}

impl std::error::Error for SortableIdFault {}

/// The two halves of an id before they are written out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Parts {
    millis: u64,
    random: u128,
}

impl Parts {
    /// The smallest id above this one. When the random half is used up the
    /// time moves on by a millisecond instead (stopping at the largest time
    /// there is, which is thousands of years away).
    fn successor(self) -> Self {
        if self.random < MAX_RANDOM {
            Self {
                random: self.random + 1,
                ..self
            }
        } else {
            Self {
                millis: (self.millis + 1).min(MAX_MILLIS),
                random: 0,
            }
        }
    }

    fn write(self) -> String {
        let mut bits = (u128::from(self.millis) << RANDOM_BITS) | self.random;
        let mut text = [b'0'; ID_LEN];
        for slot in text.iter_mut().rev() {
            *slot = CROCKFORD[(bits & 0x1f) as usize];
            bits >>= 5;
        }
        text.iter().map(|&byte| char::from(byte)).collect()
    }
}

/// The id after `previous` at clock reading `now`. Randomness is asked for
/// only when the clock has moved past `previous`.
fn next_parts(
    previous: Option<Parts>,
    now: u64,
    fresh_random: impl FnOnce() -> Result<u128, SortableIdFault>,
) -> Result<Parts, SortableIdFault> {
    if let Some(last) = previous.filter(|last| now <= last.millis) {
        return Ok(last.successor());
    }
    let random = fresh_random()?;
    Ok(Parts { millis: now.min(MAX_MILLIS), random })
}

fn unix_millis() -> Result<u64, SortableIdFault> {
    let since_epoch = std::time::UNIX_EPOCH.elapsed().map_err(|_| SortableIdFault::ClockPrecedesOrigin)?;
    Ok(u64::try_from(since_epoch.as_millis()).unwrap_or(MAX_MILLIS))
}

fn random_80_bits() -> Result<u128, SortableIdFault> {
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes).map_err(|why| SortableIdFault::Entropy(why.to_string()))?;
    Ok(bytes
        .iter()
        .fold(0u128, |bits, &byte| (bits << 8) | u128::from(byte)))
}

/// The newest id this process has handed out.
static NEWEST: Mutex<Option<Parts>> = Mutex::new(None);

/// A new id, later than every id this process made before.
pub fn fresh_sortable_id() -> Result<String, SortableIdFault> {
    let now = unix_millis()?;
    let mut newest = NEWEST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let next = next_parts(*newest, now, random_80_bits)?;
    *newest = Some(next);
    Ok(next.write())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refuse_randomness() -> Result<u128, SortableIdFault> {
        Err(SortableIdFault::Entropy("not asked for".to_owned()))
    }

    #[test]
    fn zero_writes_as_26_zeros() {
        let zero = Parts {
            millis: 0,
            random: 0,
        };
        assert_eq!(zero.write(), "0".repeat(26));
    }

    #[test]
    fn the_largest_id_starts_with_a_7_because_128_bits_fill_130() {
        let largest = Parts {
            millis: MAX_MILLIS,
            random: MAX_RANDOM,
        };
        assert_eq!(largest.write(), format!("7{}", "Z".repeat(25)));
    }

    #[test]
    fn time_takes_the_first_ten_characters_and_randomness_the_last_sixteen() {
        let one_ms = Parts {
            millis: 1,
            random: 0,
        };
        assert_eq!(one_ms.write(), format!("0000000001{}", "0".repeat(16)));
        let one_random = Parts {
            millis: 0,
            random: 1,
        };
        assert_eq!(one_random.write(), format!("{}1", "0".repeat(25)));
    }

    #[test]
    fn a_known_time_writes_as_its_published_prefix() {
        // The timestamp the ULID specification's own example decodes to.
        let known = Parts {
            millis: 1_469_918_176_385,
            random: 0,
        };
        assert!(known.write().starts_with("01ARYZ6S41"));
    }

    #[test]
    fn the_alphabet_has_32_distinct_characters_and_no_confusable_letters() {
        let mut handed_out = std::collections::BTreeSet::new();
        handed_out.extend(CROCKFORD.iter().copied());
        assert_eq!(handed_out.len(), 32);
        for banned in *b"ILOU" {
            assert!(!handed_out.contains(&banned));
        }
    }

    #[test]
    fn the_same_millisecond_counts_up_without_asking_for_randomness() {
        let last = Parts {
            millis: 40,
            random: 7,
        };
        let next = next_parts(Some(last), 40, refuse_randomness).unwrap();
        assert_eq!(
            next,
            Parts {
                millis: 40,
                random: 8
            }
        );
    }

    #[test]
    fn a_clock_that_stepped_back_still_counts_up() {
        let last = Parts {
            millis: 40,
            random: 7,
        };
        let next = next_parts(Some(last), 12, refuse_randomness).unwrap();
        assert_eq!(
            next,
            Parts {
                millis: 40,
                random: 8
            }
        );
    }

    #[test]
    fn a_later_millisecond_takes_new_randomness() {
        let last = Parts {
            millis: 40,
            random: 7,
        };
        let next = next_parts(Some(last), 41, || Ok(99)).unwrap();
        assert_eq!(
            next,
            Parts {
                millis: 41,
                random: 99
            }
        );
    }

    #[test]
    fn the_first_id_takes_new_randomness() {
        let next = next_parts(None, 5, || Ok(3)).unwrap();
        assert_eq!(
            next,
            Parts {
                millis: 5,
                random: 3
            }
        );
    }

    #[test]
    fn a_used_up_random_half_moves_to_the_next_millisecond() {
        let full = Parts {
            millis: 5,
            random: MAX_RANDOM,
        };
        assert_eq!(
            full.successor(),
            Parts {
                millis: 6,
                random: 0
            }
        );
    }

    #[test]
    fn the_time_never_passes_48_bits() {
        let end = Parts {
            millis: MAX_MILLIS,
            random: MAX_RANDOM,
        };
        assert_eq!(end.successor().millis, MAX_MILLIS);
        let far_future = next_parts(None, u64::MAX, || Ok(0)).unwrap();
        assert_eq!(far_future.millis, MAX_MILLIS);
    }

    #[test]
    fn a_failing_random_source_is_reported_and_not_swallowed() {
        let error = next_parts(None, 5, refuse_randomness).unwrap_err();
        assert_eq!(error.to_string(), "CSPRNG failed: not asked for");
    }

    #[test]
    fn the_clock_error_says_so() {
        assert_eq!(
            SortableIdFault::ClockPrecedesOrigin.to_string(),
            "system clock is before UNIX epoch"
        );
    }

    #[test]
    fn real_ids_are_26_crockford_characters_and_strictly_increase() {
        let mut previous = fresh_sortable_id().unwrap();
        for _ in 0..2_000 {
            let id = fresh_sortable_id().unwrap();
            assert_eq!(id.len(), 26);
            assert!(id.bytes().all(|byte| CROCKFORD.contains(&byte)), "{id}");
            assert!(id > previous, "{id} should follow {previous}");
            previous = id;
        }
    }
}
