//! ULIDs: the 26-character ids clip folders are named by.
//!
//! An id is 48 bits of milliseconds since the Unix epoch followed by 80 random
//! bits, written in Crockford base32 (digits and capitals without I, L, O and
//! U). That makes ids sort by creation time as plain text. Within one process
//! they also strictly increase: an id made in the same millisecond as the
//! last one (or after the clock stepped back) is the last one plus one.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ID_LEN: usize = 26;
const RANDOM_BITS: u32 = 80;
const MAX_MILLIS: u64 = (1 << 48) - 1;
const MAX_RANDOM: u128 = (1 << RANDOM_BITS) - 1;

#[derive(Debug)]
pub enum UlidError {
    ClockBeforeEpoch,
    Random(String),
}

impl core::fmt::Display for UlidError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UlidError::ClockBeforeEpoch => f.write_str("system clock is before UNIX epoch"),
            UlidError::Random(why) => write!(f, "CSPRNG failed: {why}"),
        }
    }
}

impl std::error::Error for UlidError {}

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
            *slot = ALPHABET[(bits & 0x1f) as usize];
            bits >>= 5;
        }
        text.iter().map(|&byte| char::from(byte)).collect()
    }
}

/// The id after `newest` at clock reading `now`. Randomness is asked for only
/// when the clock has moved past `newest`.
fn next_parts(
    newest: Option<Parts>,
    now: u64,
    fresh_random: impl FnOnce() -> Result<u128, UlidError>,
) -> Result<Parts, UlidError> {
    match newest {
        Some(last) if now <= last.millis => Ok(last.successor()),
        _ => Ok(Parts {
            millis: now.min(MAX_MILLIS),
            random: fresh_random()?,
        }),
    }
}

fn unix_millis() -> Result<u64, UlidError> {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| UlidError::ClockBeforeEpoch)?;
    Ok(u64::try_from(since_epoch.as_millis()).unwrap_or(MAX_MILLIS))
}

fn random_80_bits() -> Result<u128, UlidError> {
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes).map_err(|why| UlidError::Random(why.to_string()))?;
    Ok(bytes
        .iter()
        .fold(0u128, |bits, &byte| (bits << 8) | u128::from(byte)))
}

/// The newest id this process has handed out.
static NEWEST: Mutex<Option<Parts>> = Mutex::new(None);

/// A new id, later than every id this process made before.
pub fn new_ulid() -> Result<String, UlidError> {
    let now = unix_millis()?;
    let mut newest = NEWEST.lock().unwrap_or_else(PoisonError::into_inner);
    let next = next_parts(*newest, now, random_80_bits)?;
    *newest = Some(next);
    Ok(next.write())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refuse_randomness() -> Result<u128, UlidError> {
        Err(UlidError::Random("not asked for".to_owned()))
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
        let mut seen = std::collections::BTreeSet::new();
        seen.extend(ALPHABET.iter().copied());
        assert_eq!(seen.len(), 32);
        for banned in *b"ILOU" {
            assert!(!seen.contains(&banned));
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
            UlidError::ClockBeforeEpoch.to_string(),
            "system clock is before UNIX epoch"
        );
    }

    #[test]
    fn real_ids_are_26_crockford_characters_and_strictly_increase() {
        let mut previous = new_ulid().unwrap();
        for _ in 0..2_000 {
            let id = new_ulid().unwrap();
            assert_eq!(id.len(), 26);
            assert!(id.bytes().all(|byte| ALPHABET.contains(&byte)), "{id}");
            assert!(id > previous, "{id} should follow {previous}");
            previous = id;
        }
    }
}
