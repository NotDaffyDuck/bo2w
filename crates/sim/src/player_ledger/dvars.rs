//! Reading the dvars a script has set: a client's own first, then the server's.

#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptDvars<'a> {
    client: &'a [(String, String)],
    server: &'a [(String, String)],
}

impl<'a> ScriptDvars<'a> {
    pub(super) fn new(client: &'a [(String, String)], server: &'a [(String, String)]) -> Self {
        Self { client, server }
    }

    /// The value of `name` (case does not matter); a client's own shadows the server's.
    pub fn string(&self, name: &str) -> Option<&'a str> {
        let mut rows = self.client.iter().chain(self.server);
        rows.find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    /// The value as a whole number; a decimal one is cut toward zero.
    pub fn int(&self, name: &str) -> Option<i32> {
        let text = self.string(name)?.trim();
        match text.parse::<i32>() {
            Ok(whole) => Some(whole),
            Err(_) => text.parse::<f32>().ok().map(|fraction| fraction as i32),
        }
    }

    /// The value as display text. One starting with `@` names a localised string, filled in with the arguments that
    /// follow it (`&&1`, `&&2`, ..); any other value is returned as it stands.
    pub fn text(&self, name: &str, localize: impl Fn(&str) -> Option<String>) -> Option<String> {
        let mut parts = self.string(name)?.split(crate::HUD_PRINT_ARG_SEPARATOR);
        let head = parts.next().unwrap_or_default();
        let Some(key) = head.strip_prefix('@') else {
            return Some(head.to_owned());
        };
        let mut line = localize(key)?;
        for (n, arg) in parts.enumerate() {
            line = line.replace(&format!("&&{}", n + 1), arg);
        }
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::ScriptDvars;

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_clients_own_value_shadows_the_servers_and_case_is_ignored() {
        let own = rows(&[("Timer", "5")]);
        let server = rows(&[("timer", "9"), ("limit", "30")]);
        let dvars = ScriptDvars::new(&own, &server);
        assert_eq!(dvars.string("TIMER"), Some("5"));
        assert_eq!(dvars.string("limit"), Some("30"));
        assert_eq!(dvars.string("missing"), None);
    }

    #[test]
    fn numbers_read_whole_or_cut_toward_zero() {
        let server = rows(&[("a", " 12 "), ("b", "3.9"), ("c", "-3.9"), ("d", "x")]);
        let dvars = ScriptDvars::new(&[], &server);
        assert_eq!(dvars.int("a"), Some(12));
        assert_eq!(dvars.int("b"), Some(3));
        assert_eq!(dvars.int("c"), Some(-3));
        assert_eq!(dvars.int("d"), None);
    }

    #[test]
    fn text_fills_a_localised_template_with_its_arguments() {
        let sep = crate::HUD_PRINT_ARG_SEPARATOR;
        let server = rows(&[
            ("plain", "hello"),
            ("tmpl", &format!("@KILLS{sep}Ann{sep}7")),
            ("unknown", &format!("@NOPE{sep}x")),
        ]);
        let dvars = ScriptDvars::new(&[], &server);
        let lookup = |key: &str| (key == "KILLS").then(|| "&&1 has &&2 kills".to_string());
        assert_eq!(dvars.text("plain", lookup).as_deref(), Some("hello"));
        assert_eq!(dvars.text("tmpl", lookup).as_deref(), Some("Ann has 7 kills"));
        assert_eq!(dvars.text("unknown", lookup), None);
    }
}
