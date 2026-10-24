//! The commands the console knows: their usage lines, and the completions
//! offered while one is typed.

use std::sync::Arc;

use bevy::prelude::Resource;

/// Offers the values an argument can take.
pub trait ArgCompleter: Send + Sync {
    /// The values that fit what has been typed of the argument so far.
    fn complete(&self, prefix: &str) -> Vec<String>;
}

/// A fixed list of values, offered by the start they share with the typing in
/// either case, in the order given.
#[derive(Clone)]
pub struct StaticCompleter {
    values: Vec<String>,
}

impl StaticCompleter {
    pub fn new(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            values: values.into_iter().map(Into::into).collect(),
        }
    }
}

impl ArgCompleter for StaticCompleter {
    fn complete(&self, prefix: &str) -> Vec<String> {
        let cut = |value: &str| {
            value
                .char_indices()
                .nth(prefix.chars().count())
                .map_or(value.len(), |(at, _)| at)
        };
        self.values
            .iter()
            .filter(|value| value[..cut(value)].eq_ignore_ascii_case(prefix))
            .cloned()
            .collect()
    }
}

impl<F> ArgCompleter for F
where
    F: Fn(&str) -> Vec<String> + Send + Sync,
{
    fn complete(&self, prefix: &str) -> Vec<String> {
        self(prefix)
    }
}

/// One command: what it is called, how it is used, what its arguments offer.
#[derive(Clone)]
pub struct CommandSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub usage: String,
    /// One completer per argument, first argument first.
    pub args: Vec<Arc<dyn ArgCompleter>>,
}

impl CommandSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            usage: String::new(),
            args: Vec::new(),
        }
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = usage.into();
        self
    }

    /// Completes the next argument that has no completer yet.
    pub fn arg(mut self, completer: impl ArgCompleter + 'static) -> Self {
        self.args.push(Arc::new(completer));
        self
    }

    fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }
}

#[derive(Resource, Default, Clone)]
pub struct ConsoleRegistry {
    commands: Vec<CommandSpec>,
}

/// What the caret is on, in the typed line.
enum Slot<'a> {
    Nothing,
    /// The name of the command, partly typed.
    Name(&'a str),
    /// Argument `index` of `command`, partly typed (or not started: empty).
    Arg {
        command: &'a str,
        index: usize,
        prefix: &'a str,
    },
}

fn slot(head: &str) -> Slot<'_> {
    let words: Vec<&str> = head.split_whitespace().collect();
    let started_new = head.ends_with(char::is_whitespace);
    match (words.as_slice(), started_new) {
        ([], _) => Slot::Nothing,
        ([name], false) => Slot::Name(name),
        ([command, rest @ ..], true) => Slot::Arg {
            command,
            index: rest.len(),
            prefix: "",
        },
        ([command, rest @ .., last], false) => Slot::Arg {
            command,
            index: rest.len(),
            prefix: last,
        },
    }
}

impl ConsoleRegistry {
    pub fn register(&mut self, spec: CommandSpec) {
        self.commands.push(spec);
    }

    /// Every name and alias, sorted, each once.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .iter()
            .flat_map(|spec| std::iter::once(&spec.name).chain(&spec.aliases))
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// One line per command, by name: its usage, which starts with its name,
    /// or the bare name when it has none.
    pub fn help_lines(&self) -> Vec<String> {
        let mut lines: Vec<(String, String)> = self
            .commands
            .iter()
            .map(|spec| {
                let line = match spec.usage.as_str() {
                    "" => spec.name.clone(),
                    usage if usage.to_lowercase().starts_with(&spec.name.to_lowercase()) => {
                        usage.to_owned()
                    }
                    usage => format!("{} — {usage}", spec.name),
                };
                (spec.name.to_lowercase(), line)
            })
            .collect();
        lines.sort();
        lines.into_iter().map(|(_, line)| line).collect()
    }

    /// The command called `name`, by its name or one of its aliases.
    pub fn resolve(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|spec| spec.answers_to(name))
    }

    /// What can be typed at the caret: command names while the first word is
    /// being typed (none for an empty line), then what the argument under the
    /// caret takes.
    pub fn suggestions(&self, line: &str, caret: usize) -> Vec<String> {
        let at = line
            .char_indices()
            .nth(caret)
            .map_or(line.len(), |(at, _)| at);
        match slot(&line[..at]) {
            Slot::Nothing => Vec::new(),
            Slot::Name(typed) => rank(&self.names(), typed),
            Slot::Arg {
                command,
                index,
                prefix,
            } => self
                .resolve(command)
                .and_then(|spec| spec.args.get(index))
                .map_or_else(Vec::new, |completer| completer.complete(prefix)),
        }
    }
}

/// How well `name` answers `typed`, best (0) first; `None` for no answer.
fn fit(name: &str, typed: &str) -> Option<u8> {
    let name = name.to_lowercase();
    let typed = typed.to_lowercase();
    if name == typed {
        return Some(0);
    }
    if name.starts_with(&typed) {
        return Some(1);
    }
    if words_of(&name).any(|word| word.starts_with(&typed)) {
        return Some(2);
    }
    if name.contains(&typed) {
        return Some(3);
    }
    let mut rest = name.chars();
    let spread = typed.chars().count() >= 3 && typed.chars().all(|c| rest.any(|n| n == c));
    spread.then_some(4)
}

/// The words a lowercase command name is made of, split at `_`, `.`, `-`.
fn words_of(name: &str) -> impl Iterator<Item = &str> {
    name.split(['_', '.', '-']).filter(|word| !word.is_empty())
}

/// The names that answer `typed`, best fit first; the shorter name, then the
/// alphabetically earlier, among equals.
fn rank(names: &[String], typed: &str) -> Vec<String> {
    let mut hits: Vec<(u8, usize, &String)> = names
        .iter()
        .filter_map(|name| Some((fit(name, typed)?, name.chars().count(), name)))
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, _, name)| name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ConsoleRegistry {
        let mut registry = ConsoleRegistry::default();
        registry.register(
            CommandSpec::new("map")
                .usage("map <name>")
                .arg(StaticCompleter::new(["mp_nuketown", "mp_raid", "zm_town"])),
        );
        registry.register(CommandSpec::new("quit").alias("exit"));
        registry.register(CommandSpec::new("quote").usage("say it"));
        registry.register(CommandSpec::new("r_lodScaleRigid").usage("r_lodScaleRigid [f]"));
        registry.register(CommandSpec::new("fx_dump"));
        registry
    }

    #[test]
    fn a_static_completer_offers_the_values_that_start_with_the_typing() {
        let completer = StaticCompleter::new(["+forward", "+back", "Fire"]);
        assert_eq!(completer.complete("+"), ["+forward", "+back"]);
        assert_eq!(completer.complete("fi"), ["Fire"], "either case");
        assert_eq!(completer.complete(""), ["+forward", "+back", "Fire"]);
        assert!(completer.complete("x").is_empty());
        assert!(completer.complete("+forwardx").is_empty());
    }

    #[test]
    fn a_closure_is_a_completer() {
        let completer = |prefix: &str| vec![format!("{prefix}!")];
        assert_eq!(completer.complete("a"), ["a!"]);
    }

    #[test]
    fn names_and_aliases_are_sorted_and_not_repeated() {
        let mut registry = registry();
        registry.register(CommandSpec::new("map"));
        assert_eq!(
            registry.names(),
            ["exit", "fx_dump", "map", "quit", "quote", "r_lodScaleRigid"]
        );
    }

    #[test]
    fn a_command_is_found_by_name_or_alias_in_the_case_it_was_given() {
        let registry = registry();
        assert_eq!(
            registry.resolve("exit").map(|s| s.name.as_str()),
            Some("quit")
        );
        assert!(registry.resolve("quit").is_some());
        assert!(registry.resolve("Quit").is_none());
        assert!(registry.resolve("nope").is_none());
    }

    #[test]
    fn the_first_command_of_a_name_is_the_one_found() {
        let mut registry = registry();
        registry.register(CommandSpec::new("map").usage("second"));
        assert_eq!(
            registry.resolve("map").map(|s| s.usage.as_str()),
            Some("map <name>")
        );
    }

    #[test]
    fn help_has_a_line_per_command_in_name_order() {
        assert_eq!(
            registry().help_lines(),
            [
                "fx_dump",
                "map <name>",
                "quit",
                "quote — say it",
                "r_lodScaleRigid [f]"
            ]
        );
    }

    #[test]
    fn an_empty_line_suggests_nothing() {
        assert!(registry().suggestions("", 0).is_empty());
        assert!(registry().suggestions("   ", 3).is_empty());
    }

    #[test]
    fn a_name_is_completed_best_fit_first() {
        let registry = registry();
        assert_eq!(registry.suggestions("qu", 2), ["quit", "quote"]);
        assert_eq!(registry.suggestions("QUIT", 4), ["quit"]);
        assert_eq!(registry.suggestions("ex", 2), ["exit"]);
    }

    #[test]
    fn a_name_is_also_found_by_a_word_in_it_or_part_of_it() {
        let registry = registry();
        assert_eq!(registry.suggestions("dump", 4), ["fx_dump"]);
        assert_eq!(registry.suggestions("lodscale", 8), ["r_lodScaleRigid"]);
        assert_eq!(
            registry.suggestions("fxd", 3),
            ["fx_dump"],
            "letters in order"
        );
        assert!(
            registry.suggestions("xf", 2).is_empty(),
            "too short to spread"
        );
    }

    #[test]
    fn only_what_is_before_the_caret_is_looked_at() {
        let registry = registry();
        assert_eq!(registry.suggestions("quit now", 2), ["quit", "quote"]);
        assert_eq!(registry.suggestions("é€", 99), Vec::<String>::new());
    }

    #[test]
    fn an_argument_is_completed_by_its_own_completer() {
        let registry = registry();
        assert_eq!(
            registry.suggestions("map mp_", 7),
            ["mp_nuketown", "mp_raid"]
        );
        assert_eq!(
            registry.suggestions("map ", 4).len(),
            3,
            "a new argument offers all"
        );
        assert_eq!(registry.suggestions("map zm", 6), ["zm_town"]);
    }

    #[test]
    fn an_alias_completes_arguments_too() {
        let mut registry = registry();
        registry.register(
            CommandSpec::new("go")
                .alias("travel")
                .arg(StaticCompleter::new(["a"])),
        );
        assert_eq!(registry.suggestions("travel ", 7), ["a"]);
    }

    #[test]
    fn nothing_is_offered_where_there_is_no_completer() {
        let registry = registry();
        assert!(
            registry.suggestions("map mp_raid ", 12).is_empty(),
            "second argument"
        );
        assert!(registry.suggestions("quit ", 5).is_empty());
        assert!(registry.suggestions("nope ", 5).is_empty());
    }

    #[test]
    fn ranking_prefers_exact_then_prefix_then_the_shorter_name() {
        let names: Vec<String> = ["mapa", "map", "xmap", "map_long"]
            .iter()
            .map(|n| (*n).to_owned())
            .collect();
        assert_eq!(rank(&names, "map"), ["map", "mapa", "map_long", "xmap"]);
    }
}
