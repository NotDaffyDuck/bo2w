//! Lenient number reading for text commands: bad text is 0, never an error.

/// The token at `at`, or an empty string past the end.
pub(super) fn token<'a>(tokens: &[&'a str], at: usize) -> &'a str {
    tokens.get(at).copied().unwrap_or("")
}

/// Read a leading integer the way a text console does: skip leading whitespace, take an
/// optional sign, then digits up to the first other character. No digits is 0. Values that
/// do not fit stop at the largest magnitude.
pub(super) fn leading_int(text: &str) -> i32 {
    let text = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let magnitude = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i32, |sum, digit| {
            sum.saturating_mul(10).saturating_add(i32::from(digit - b'0'))
        });
    if negative { magnitude.saturating_neg() } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_numbers_read_as_themselves() {
        assert_eq!(leading_int("0"), 0);
        assert_eq!(leading_int("42"), 42);
        assert_eq!(leading_int("-7"), -7);
        assert_eq!(leading_int("+9"), 9);
    }

    #[test]
    fn reading_stops_at_the_first_other_character() {
        assert_eq!(leading_int("12abc"), 12);
        assert_eq!(leading_int("3.9"), 3);
        assert_eq!(leading_int("-5x7"), -5);
    }

    #[test]
    fn leading_whitespace_is_skipped_but_not_a_gap_after_the_sign() {
        assert_eq!(leading_int("  \t\n15"), 15);
        assert_eq!(leading_int("- 5"), 0);
    }

    #[test]
    fn no_digits_is_zero() {
        for text in ["", " ", "abc", "-", "+", "--5", "+-5", "x5"] {
            assert_eq!(leading_int(text), 0, "{text:?}");
        }
    }

    #[test]
    fn huge_values_stop_at_the_limit() {
        assert_eq!(leading_int("2147483647"), i32::MAX);
        assert_eq!(leading_int("99999999999999"), i32::MAX);
        assert_eq!(leading_int("-99999999999999"), -i32::MAX);
    }

    #[test]
    fn a_missing_token_is_the_empty_string() {
        let tokens = ["b", "2"];
        assert_eq!(token(&tokens, 1), "2");
        assert_eq!(token(&tokens, 2), "");
        assert_eq!(token(&[], 0), "");
    }
}
