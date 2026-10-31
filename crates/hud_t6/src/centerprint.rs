//! Centre-printed text: its fade and its "&&1" name.
//!
//! TODO(fact): BO2's centerprint fade and replace are not in the facts yet (QUESTIONS.md 10-21
//! Clean1 hud centerprint); until answered these are MW2's.

extern crate alloc;

use alloc::string::String;

/// Where the player's name goes in a template.
const NAME_SLOT: &str = "&&1";

/// The alpha of a message started at `start` that lasts `duration` ms: full until its last
/// `dim_trailing` ms, then down to 0. None once it is over, or when it never started.
#[must_use]
pub fn fade_color(at_ms: i32, began_ms: i32, lasts_ms: i32, dim_trailing: i32) -> Option<f32> {
    if began_ms == 0 {
        return None;
    }
    let gone = at_ms - began_ms;
    if gone >= lasts_ms {
        return None;
    }
    let left = lasts_ms - gone;
    if dim_trailing < 1 || left >= dim_trailing {
        Some(1.0)
    } else {
        Some(left as f32 / dim_trailing as f32)
    }
}

/// A template with "&&1" gets the name in its place; any other template comes back as it was
/// (replacing finds nothing to swap in those).
#[must_use]
pub fn banner_swap_player_tag(template: &str, who: &str) -> String {
    template.replace(NAME_SLOT, who)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_full_then_fades() {
        assert_eq!(fade_color(1000, 0, 500, 100), None);
        assert_eq!(fade_color(1100, 1000, 500, 100), Some(1.0));
        assert_eq!(fade_color(1450, 1000, 500, 100), Some(0.5));
        assert_eq!(fade_color(1500, 1000, 500, 100), None);
    }

    #[test]
    fn the_name_goes_in_place_of_and_and_one() {
        assert_eq!(banner_swap_player_tag("&&1 joined", "Ann"), "Ann joined");
        assert_eq!(banner_swap_player_tag("joined", "Ann"), "joined");
    }
}
