//! Settings from `.env` at the workspace root. A value written there beats the
//! process environment, and an empty value means "not set".

use std::collections::BTreeMap;
use std::path::Path;

use crate::shell::Res;

pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    /// No `.env` at all is fine: every key then comes from the process.
    pub fn load(root: &Path) -> Res<Self> {
        let path = root.join(".env");
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::parse("")),
            Err(error) => Err(format!("reading {}: {error}", path.display())),
        }
    }

    /// `KEY=value` lines, optionally behind `export `. `'single'` quotes keep
    /// the text as written; `"double"` quotes and bare values expand `$NAME`
    /// and `${NAME}` from keys above them, then from the process.
    pub fn parse(text: &str) -> Self {
        let mut vars = BTreeMap::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let value = match quoted(value, '\'') {
                Some(literal) => literal.to_string(),
                None => expand(quoted(value, '"').unwrap_or(value), &vars),
            };
            vars.insert(key.trim().to_string(), value);
        }
        Self { vars }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let written = self.vars.get(key).filter(|value| !value.is_empty());
        match written {
            Some(value) => Some(value.clone()),
            None => std::env::var(key).ok().filter(|value| !value.is_empty()),
        }
    }

    pub fn require(&self, key: &str) -> Res<String> {
        self.get(key)
            .ok_or_else(|| format!("set {key} in .env (see .env.example)"))
    }
}

fn quoted(value: &str, quote: char) -> Option<&str> {
    value.strip_prefix(quote)?.strip_suffix(quote)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// An unset name expands to nothing, as in a shell.
fn expand(text: &str, known: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];
        let (name, consumed) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(close) => (&braced[..close], close + 2),
                None => ("", 0),
            },
            None => {
                let end = after.find(|c| !is_name_char(c)).unwrap_or(after.len());
                (&after[..end], end)
            }
        };
        if name.is_empty() {
            out.push('$');
            rest = after;
            continue;
        }
        let value = known
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            // An unset name expands to nothing, as in a shell.
            .unwrap_or_else(String::new);
        out.push_str(&value);
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_blanks_and_export_are_skipped() {
        let env = Env::parse("# host\n\n  export BO2MP_A = one \nnot a pair\nBO2MP_B=two\n");
        assert_eq!(env.get("BO2MP_A").as_deref(), Some("one"));
        assert_eq!(env.get("BO2MP_B").as_deref(), Some("two"));
        assert_eq!(env.vars.len(), 2);
    }

    #[test]
    fn single_quotes_are_literal_and_double_quotes_expand() {
        let env = Env::parse(
            "XT_ROOT=/srv/x\nXT_LIT='$XT_ROOT/bin'\nXT_DQ=\"$XT_ROOT/bin\"\nXT_BRACE=${XT_ROOT}s\n",
        );
        assert_eq!(env.get("XT_LIT").as_deref(), Some("$XT_ROOT/bin"));
        assert_eq!(env.get("XT_DQ").as_deref(), Some("/srv/x/bin"));
        assert_eq!(env.get("XT_BRACE").as_deref(), Some("/srv/xs"));
    }

    #[test]
    fn unknown_names_expand_to_nothing_and_a_lone_dollar_stays() {
        let env = Env::parse("XT_A=a$XT_NOT_SET_ANYWHERE_42-b\nXT_B=cost $ 5\nXT_C=${}\n");
        assert_eq!(env.get("XT_A").as_deref(), Some("a-b"));
        assert_eq!(env.get("XT_B").as_deref(), Some("cost $ 5"));
        assert_eq!(env.get("XT_C").as_deref(), Some("${}"));
    }

    #[test]
    fn empty_counts_as_unset_and_require_says_where_to_set_it() {
        let env = Env::parse("XT_EMPTY=\nXT_QUOTED_EMPTY=''\n");
        assert_eq!(env.get("XT_EMPTY"), None);
        assert_eq!(env.get("XT_QUOTED_EMPTY"), None);
        assert_eq!(
            env.require("XT_EMPTY").unwrap_err(),
            "set XT_EMPTY in .env (see .env.example)"
        );
    }

    #[test]
    fn a_missing_file_is_an_empty_env() {
        let dir = std::env::temp_dir().join(format!("xtask-dotenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env = Env::load(&dir).unwrap();
        assert!(env.vars.is_empty());
        std::fs::write(dir.join(".env"), "XT_FROM_FILE=yes\n").unwrap();
        assert_eq!(
            Env::load(&dir).unwrap().get("XT_FROM_FILE").as_deref(),
            Some("yes")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
