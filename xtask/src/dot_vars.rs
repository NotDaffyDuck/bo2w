//! Settings from `.env` at the workspace root. A value written there beats the
//! process environment, and an empty value means "not set".

use std::collections::BTreeMap;
use std::path::Path;

pub struct DotVars {
    pairs: BTreeMap<String, String>,
}

impl DotVars {
    /// No `.env` at all is fine: every key then comes from the process.
    pub fn load(repo_root: &Path) -> Result<Self, String> {
        let file = repo_root.join(".env");
        std::fs::read_to_string(&file)
            .or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(String::new()),
                _ => Err(format!("reading {}: {error}", file.display())),
            })
            .map(|text| Self::parse(&text))
    }

    /// `KEY=value` lines, optionally behind `export `. `'single'` quotes keep
    /// the text as written; `"double"` quotes and bare values expand `$NAME`
    /// and `${NAME}` from keys above them, then from the process.
    pub fn parse(text: &str) -> Self {
        let assignments = text.lines().filter_map(|line| {
            let line = line.trim();
            if matches!(line.chars().next(), None | Some('#')) {
                return None;
            }
            line.strip_prefix("export ").unwrap_or(line).split_once('=')
        });
        // Each value may refer to the keys read before it, so fold in order.
        assignments.fold(Self { pairs: BTreeMap::new() }, |mut vars, (knob, raw)| {
            let val = vars.interpret(raw.trim());
            vars.pairs.insert(knob.trim().to_owned(), val);
            vars
        })
    }

    pub fn value_of(&self, knob: &str) -> Option<String> {
        let written = self.pairs.get(knob).cloned();
        let ambient = std::env::var(knob).ok();
        [written, ambient]
            .into_iter()
            .flatten()
            .find(|val| !val.is_empty())
    }

    pub fn must_get(&self, knob: &str) -> Result<String, String> {
        let Some(val) = self.value_of(knob) else {
            return Err(format!("set {knob} in .env (see .env.example)"));
        };
        Ok(val)
    }
}

/// `val` without the `quote` characters around it, if it has both.
fn quoted(quote: char, val: &str) -> Option<&str> {
    val.strip_prefix(quote).and_then(|inner| inner.strip_suffix(quote))
}


/// The variable named right after a `$`, and how many bytes it used up, or
/// `None` when the `$` is not a reference.
fn reference(after: &str) -> Option<(&str, usize)> {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let (title, used) = match after.strip_prefix('{') {
        Some(braced) => braced.split_once('}').map(|(title, _)| (title, title.len() + 2))?,
        None => {
            let end = after.find(|c: char| !is_name_char(c)).unwrap_or(after.len());
            let title = &after[..end];
            (title, end)
        }
    };
    if title.is_empty() {
        return None;
    }
    Some((title, used))
}

impl DotVars {
    /// The final text of one right-hand side.
    fn interpret(&self, raw: &str) -> String {
        if let Some(literal) = quoted('\'', raw) {
            return literal.to_owned();
        }
        self.expand(quoted('"', raw).unwrap_or(raw))
    }

    fn lookup(&self, title: &str) -> String {
        match self.pairs.get(title) {
            Some(val) => val.clone(),
            None => std::env::var(title).unwrap_or_default(),
        }
    }

    /// An unset name expands to nothing, as in a shell.
    fn expand(&self, text: &str) -> String {
        let mut expanded = String::new();
        let mut rest = text;
        while let Some((head, after)) = rest.split_once('$') {
            expanded += head;
            rest = match reference(after) {
                Some((title, used)) => {
                    expanded += &self.lookup(title);
                    &after[used..]
                }
                None => {
                    expanded += "$";
                    after
                }
            };
        }
        expanded + rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_blanks_and_export_are_skipped() {
        let dot_conf = DotVars::parse("# host\n\n  export BO2MP_A = one \nnot a pair\nBO2MP_B=two\n");
        assert_eq!(dot_conf.value_of("BO2MP_A").as_deref(), Some("one"));
        assert_eq!(dot_conf.value_of("BO2MP_B").as_deref(), Some("two"));
        assert_eq!(dot_conf.pairs.len(), 2);
    }

    #[test]
    fn single_quotes_are_literal_and_double_quotes_expand() {
        let dot_conf = DotVars::parse(
            "XT_ROOT=/srv/x\nXT_LIT='$XT_ROOT/bin'\nXT_DQ=\"$XT_ROOT/bin\"\nXT_BRACE=${XT_ROOT}s\n",
        );
        assert_eq!(dot_conf.value_of("XT_LIT").as_deref(), Some("$XT_ROOT/bin"));
        assert_eq!(dot_conf.value_of("XT_DQ").as_deref(), Some("/srv/x/bin"));
        assert_eq!(dot_conf.value_of("XT_BRACE").as_deref(), Some("/srv/xs"));
    }

    #[test]
    fn unknown_names_expand_to_nothing_and_a_lone_dollar_stays() {
        let dot_conf = DotVars::parse("XT_A=a$XT_NOT_SET_ANYWHERE_42-b\nXT_B=cost $ 5\nXT_C=${}\n");
        assert_eq!(dot_conf.value_of("XT_A").as_deref(), Some("a-b"));
        assert_eq!(dot_conf.value_of("XT_B").as_deref(), Some("cost $ 5"));
        assert_eq!(dot_conf.value_of("XT_C").as_deref(), Some("${}"));
    }

    #[test]
    fn empty_counts_as_unset_and_require_says_where_to_set_it() {
        let dot_conf = DotVars::parse("XT_EMPTY=\nXT_QUOTED_EMPTY=''\n");
        assert_eq!(dot_conf.value_of("XT_EMPTY"), None);
        assert_eq!(dot_conf.value_of("XT_QUOTED_EMPTY"), None);
        assert_eq!(
            dot_conf.must_get("XT_EMPTY").unwrap_err(),
            "set XT_EMPTY in .env (see .env.example)"
        );
    }

    #[test]
    fn an_unreadable_env_is_an_error_naming_the_path() {
        let base_dir = std::env::temp_dir().join(format!("xtask-dotenv-dir-{}", std::process::id()));
        std::fs::create_dir_all(base_dir.join(".env")).unwrap();
        let error = DotVars::load(&base_dir).err().unwrap();
        assert!(error.starts_with("reading "), "{error}");
        assert!(error.contains(".env"), "{error}");
        std::fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn later_keys_see_earlier_ones_but_not_the_reverse() {
        let dot_conf = DotVars::parse("XT_LATE=$XT_EARLY!
XT_EARLY=e
XT_NEXT=${XT_EARLY}${XT_EARLY}
");
        std::assert_eq!(dot_conf.value_of("XT_LATE"), Some(String::from("!")));
        std::assert_eq!(dot_conf.value_of("XT_NEXT"), Some(String::from("ee")));
    }

    #[test]
    fn a_missing_file_is_an_empty_env() {
        let base_dir = std::env::temp_dir().join(format!("xtask-dotenv-{}", std::process::id()));
        std::fs::create_dir_all(&base_dir).unwrap();
        let dot_conf = DotVars::load(&base_dir).unwrap();
        assert!(dot_conf.pairs.is_empty());
        std::fs::write(base_dir.join(".env"), "XT_FROM_FILE=yes\n").unwrap();
        assert_eq!(
            DotVars::load(&base_dir).unwrap().value_of("XT_FROM_FILE").as_deref(),
            Some("yes")
        );
        std::fs::remove_dir_all(&base_dir).unwrap();
    }
}
