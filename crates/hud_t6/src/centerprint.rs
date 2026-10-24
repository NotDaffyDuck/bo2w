//! Centre-printed text: its fade and its "&&1" name.
//!
//! TODO(fact): BO2's centerprint fade and replace are not in the facts yet (QUESTIONS.md 10-21
//! Clean1 hud centerprint); until answered these are MW2's.

extern crate alloc;

use alloc::string::String;

/// The alpha of a message started at `start` that lasts `duration` ms: full until its last
/// `fade_tail` ms, then down to 0. None once it is over, or when it never started.
#[must_use]
pub fn fade_color(now: i32, start: i32, duration: i32, fade_tail: i32) -> Option<f32> {
    if start == 0 {
        return None;
    }
    let elapsed = now - start;
    if elapsed >= duration {
        return None;
    }
    let remaining = duration - elapsed;
    if fade_tail < 1 || remaining >= fade_tail {
        Some(1.0)
    } else {
        Some(remaining as f32 / fade_tail as f32)
    }
}

/// A template with "&&1" gets the name in its place; any other template is kept.
#[must_use]
pub fn centerprint_replace_name(template: &str, name: &str) -> String {
    if !template.contains("&&") {
        return String::from(template);
    }
    template.replace("&&1", name)
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
        assert_eq!(centerprint_replace_name("&&1 joined", "Ann"), "Ann joined");
        assert_eq!(centerprint_replace_name("joined", "Ann"), "joined");
    }
}
