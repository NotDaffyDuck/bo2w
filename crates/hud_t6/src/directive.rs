//! Key directives in hint strings: "[{+command}]" becomes the key bound to the command, or the
//! unbound string with the command when nothing is bound.
//!
//! TODO(fact): BO2's directive rule, unbound string and default binds are not in the facts yet
//! (QUESTIONS.md 10-21 Clean1 hud directive); until answered these are MW2's.

extern crate alloc;

use alloc::string::String;

/// The localized string shown for a command with no key.
pub const KEY_UNBOUND: &str = "KEY_UNBOUND";

/// Replaces each non-empty "[{directive}]" with `resolve(directive)`; an empty one stays as it is,
/// and an unclosed one ends the replacing.
pub fn replace_directive(src: &str, mut resolve: impl FnMut(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(start) = rest.find("[{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}]") else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let directive = &after_open[..end];
        if directive.is_empty() {
            out.push_str("[{}]");
        } else {
            out.push_str(&resolve(directive));
        }
        rest = &after_open[end + 2..];
    }
    out.push_str(rest);
    out
}

/// An unbound command: the unbound string, then the command in brackets.
pub fn unbound_directive(unbound_localized: &str, directive: &str) -> String {
    alloc::format!("{unbound_localized}({directive})")
}

/// The default PC multiplayer key for a command, when the config binds none.
pub fn default_mp_key_binding(command: &str) -> Option<&'static str> {
    match command {
        "+activate" => Some("F"),
        "+gostand" => Some("SPACE"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::ToOwned;

    #[test]
    fn a_directive_becomes_its_key() {
        let out = replace_directive("Press [{+activate}] to use", |cmd| {
            default_mp_key_binding(cmd).unwrap_or("?").to_owned()
        });
        assert_eq!(out, "Press F to use");
    }

    #[test]
    fn empty_and_unclosed_directives_stay() {
        assert_eq!(replace_directive("a [{}] b", |_| "x".to_owned()), "a [{}] b");
        assert_eq!(replace_directive("a [{+gostand", |_| "x".to_owned()), "a [{+gostand");
    }

    #[test]
    fn an_unbound_command_shows_in_brackets() {
        assert_eq!(unbound_directive("UNBOUND", "+gostand"), "UNBOUND(+gostand)");
    }
}
