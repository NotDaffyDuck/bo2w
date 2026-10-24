//! Finding the names a script or entity file mentions.

use std::collections::BTreeSet;

/// Adds every double-quoted string in `text` to `names`, skipping the ones
/// inside `//` and `/* */` comments. A backslash inside a string keeps the
/// next byte from ending it. Strings that are not UTF-8 are passed over.
pub(crate) fn collect_quoted(text: &[u8], names: &mut BTreeSet<String>) {
    let mut at = 0;
    while at < text.len() {
        let next = text.get(at + 1).copied();
        match (text[at], next) {
            (b'/', Some(b'/')) => {
                at = text[at..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(text.len(), |len| at + len);
            }
            (b'/', Some(b'*')) => {
                let body = &text[at + 2..];
                at = body
                    .windows(2)
                    .position(|pair| pair == b"*/")
                    .map_or(text.len(), |len| at + 2 + len + 2);
            }
            (b'"', _) => {
                let start = at + 1;
                let mut end = start;
                while end < text.len() && text[end] != b'"' {
                    end += if text[end] == b'\\' { 2 } else { 1 };
                }
                let end = end.min(text.len());
                if let Ok(name) = std::str::from_utf8(&text[start..end]) {
                    names.insert(name.to_owned());
                }
                at = end + 1;
            }
            _ => at += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_in(text: &str) -> Vec<String> {
        let mut names = BTreeSet::new();
        collect_quoted(text.as_bytes(), &mut names);
        names.into_iter().collect()
    }

    #[test]
    fn quoted_strings_are_collected() {
        assert_eq!(
            names_in(r#"precache("fx/muzzle"); load("mp_nuked");"#),
            ["fx/muzzle", "mp_nuked"]
        );
    }

    #[test]
    fn comments_hide_their_strings() {
        let text = "a(\"kept\"); // \"line\"\n/* \"block\"\n \"more\" */ b(\"also\");";
        assert_eq!(names_in(text), ["also", "kept"]);
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        assert_eq!(names_in(r#"x("a\"b")"#), [r#"a\"b"#]);
    }

    #[test]
    fn an_unterminated_string_or_comment_ends_the_scan() {
        assert_eq!(names_in("\"open"), ["open"]);
        assert!(names_in("/* \"never closed\"").is_empty());
    }

    #[test]
    fn an_empty_string_is_a_name() {
        assert_eq!(names_in("\"\""), [""]);
    }
}
