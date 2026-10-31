//! The list of commands the console knows: each one's name and other names,
//! its help line, and the helpers that offer values for its arguments while
//! the player types them.

use std::sync::Arc;

use bevy::ecs::resource::Resource;

/// Something that can offer values for one argument of a command.
pub trait ParamSuggester: Send + Sync {
    /// Values that suit what the player has typed of the argument so far.
    fn suggest(&self, typed: &str) -> Vec<String>;
}

/// Offers from a list that never changes: each value whose start matches
/// the typing, ASCII case aside, in the order the list holds them.
#[derive(Clone)]
pub struct FixedListSuggester {
    choices: Vec<String>,
}

impl FixedListSuggester {
    pub fn new(choices: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let choices = choices.into_iter().map(|one| one.into()).collect::<Vec<String>>();
        FixedListSuggester { choices }
    }
}

/// Whether the first `typed.len()` bytes of `value` spell `typed` in either
/// case. Folding ASCII case never changes a byte count, so a byte slice of
/// that length is the part to compare.
fn opens_with(value: &str, typed: &str) -> bool {
    matches!(value.get(..typed.len()), Some(head) if head.eq_ignore_ascii_case(typed))
}

impl ParamSuggester for FixedListSuggester {
    fn suggest(&self, typed: &str) -> Vec<String> {
        let mut offered = Vec::new();
        for value in &self.choices {
            if opens_with(value, typed) {
                offered.push(value.clone());
            }
        }
        offered
    }
}

/// A plain function or closure from the typing to its values works as well.
impl<F: Fn(&str) -> Vec<String> + Send + Sync> ParamSuggester for F {
    fn suggest(&self, typed: &str) -> Vec<String> {
        (self)(typed)
    }
}

/// One command as it is put in the catalog. Start with
/// [`OrderBlueprint::new`], then chain `alias`, `usage` and `arg`.
#[derive(Clone)]
pub struct OrderBlueprint {
    pub name: String,
    /// More words that run this command.
    pub also_known_as: Vec<String>,
    /// The help line, or empty for none.
    pub help_line: String,
    /// Value helpers for the arguments, first argument first.
    pub arg_suggesters: Vec<Arc<dyn ParamSuggester>>,
}

impl OrderBlueprint {
    pub fn new(called: impl Into<String>) -> Self {
        OrderBlueprint {
            name: called.into(),
            help_line: String::default(),
            also_known_as: vec![],
            arg_suggesters: vec![],
        }
    }

    pub fn also_known(mut self, also: impl Into<String>) -> Self {
        self.also_known_as.push(also.into());
        self
    }

    pub fn help_line(mut self, line: impl Into<String>) -> Self {
        self.help_line = line.into();
        self
    }

    /// Gives the next argument along its value helper.
    pub fn arg(self, helper: impl ParamSuggester + 'static) -> Self {
        let mut arg_suggesters = self.arg_suggesters;
        arg_suggesters.push(Arc::new(helper));
        OrderBlueprint { arg_suggesters, ..self }
    }

    /// The name and each alias, the name first.
    fn every_name(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.name).chain(self.also_known_as.iter())
    }

    /// The help row: just the name when there is no usage line, the usage
    /// line by itself when it opens with the name (any case), and otherwise
    /// the name, a dash and the usage line.
    fn help_row(&self) -> String {
        let usage = self.help_line.as_str();
        let opens_with_name = usage.to_lowercase().starts_with(&self.name.to_lowercase());
        match (usage.is_empty(), opens_with_name) {
            (true, _) => self.name.clone(),
            (false, true) => usage.to_owned(),
            (false, false) => format!("{} — {}", self.name, usage),
        }
    }
}

/// Every command the console knows, in the order they were put in.
#[derive(Resource, Clone, Default)]
pub struct OrderCatalog {
    known: Vec<OrderBlueprint>,
}

/// Which part of the line the caret is in.
enum CaretSpot<'a> {
    /// Nothing but blanks so far.
    Blank,
    /// Still on the first word, the command.
    CommandWord(&'a str),
    /// On argument number `position` (from 0) of `command`, with `typed` of it
    /// entered so far; that is empty straight after a blank.
    Argument { command: &'a str, position: usize, typed: &'a str },
}

/// Reads the text left of the caret into a [`CaretSpot`].
fn caret_spot(left_of_caret: &str) -> CaretSpot<'_> {
    let words: Vec<&str> = left_of_caret.split_whitespace().collect();
    let after_blank = left_of_caret.ends_with(char::is_whitespace);
    match (words.as_slice(), after_blank) {
        ([], _) => CaretSpot::Blank,
        (&[command, ref args @ ..], true) => CaretSpot::Argument { command, position: args.len(), typed: "" },
        (&[command], false) => CaretSpot::CommandWord(command),
        (&[command, ref args @ .., last], false) => CaretSpot::Argument { command, position: args.len(), typed: last },
    }
}

impl OrderCatalog {
    /// Puts a command in. If its name is taken already, the older one keeps
    /// winning lookups.
    pub fn enlist(&mut self, command: OrderBlueprint) {
        self.known.push(command);
    }

    /// The names and aliases of every command, sorted with no repeats.
    pub fn all_names(&self) -> Vec<String> {
        let mut every_name = Vec::new();
        for command in &self.known {
            every_name.extend(command.every_name().cloned());
        }
        every_name.sort();
        every_name.dedup();
        every_name
    }

    /// The help rows (see `help_row`), ordered by lowercase name.
    pub fn usage_listing(&self) -> Vec<String> {
        let mut rows = Vec::with_capacity(self.known.len());
        for command in &self.known {
            rows.push((command.name.to_lowercase(), command.help_row()));
        }
        rows.sort();
        rows.into_iter().map(|pair| pair.1).collect::<Vec<String>>()
    }

    /// The first command whose name or an alias is exactly `word`.
    pub fn look_up(&self, word: &str) -> Option<&OrderBlueprint> {
        self.known.iter().find(|command| command.every_name().any(|known| known == word))
    }

    /// What to offer for the text before character `insert_column`: no hints
    /// on a blank line, command names for the first word, and after that
    /// whatever the helper for the argument at the caret offers.
    pub fn hint_list(&self, line: &str, insert_column: usize) -> Vec<String> {
        let cut = match line.char_indices().nth(insert_column) {
            Some((byte, _)) => byte,
            None => line.len(),
        };
        let (command, position, typed) = match caret_spot(&line[..cut]) {
            CaretSpot::Blank => return Vec::new(),
            CaretSpot::CommandWord(typed) => return rank(&self.all_names(), typed),
            CaretSpot::Argument { command, position, typed } => (command, position, typed),
        };
        let helper = self.look_up(command).and_then(|found| found.arg_suggesters.get(position));
        match helper {
            Some(helper) => helper.suggest(typed),
            None => vec![],
        }
    }
}

/// How close `name` comes to `typed`, the smaller the closer: 0 they are the
/// same word, 1 the name starts with it, 2 one piece of the name starts with
/// it, 3 the name holds it somewhere, 4 the name has its letters in the same
/// order (asked only once three or more letters are typed). `None` for none
/// of those. Case is ignored throughout.
fn fit(name: &str, typed: &str) -> Option<u8> {
    let (name, typed) = (name.to_lowercase(), typed.to_lowercase());
    let typed = typed.as_str();
    if name == typed {
        Some(0)
    } else if name.starts_with(typed) {
        Some(1)
    } else if name_parts(&name).any(|piece| piece.starts_with(typed)) {
        Some(2)
    } else if name.contains(typed) {
        Some(3)
    } else if typed.chars().count() >= 3 && letters_in_order(typed, &name) {
        Some(4)
    } else {
        None
    }
}

/// True when each letter of `typed` can be found in `word` past the one found
/// for the letter before it.
fn letters_in_order(typed: &str, word: &str) -> bool {
    let mut left = word.chars();
    typed.chars().all(|letter| left.by_ref().any(|other| other == letter))
}

/// Pieces of a lowercase name split at `_`, `.` and `-`, empty ones dropped.
fn name_parts(name: &str) -> impl Iterator<Item = &str> {
    name.split(['_', '.', '-']).filter(|piece| !piece.is_empty())
}

/// The names that come close to `typed`: closest first, the shorter of two
/// equally close names first, and A to Z after that.
fn rank(names: &[String], typed: &str) -> Vec<String> {
    let mut close = Vec::new();
    for entry in names {
        if let Some(tier) = fit(entry, typed) {
            close.push((tier, entry.chars().count(), entry));
        }
    }
    close.sort();
    close.into_iter().map(|(_, _, name)| name.to_owned()).collect::<Vec<String>>()
}

#[cfg(test)]
mod tests {
    use crate::order_book::*;

    fn book() -> OrderCatalog {
        let mut book = OrderCatalog::default();
        let map = OrderBlueprint::new("map")
            .help_line("map <name>")
            .arg(FixedListSuggester::new(["mp_nuketown", "mp_raid", "zm_town"]));
        book.enlist(map);
        book.enlist(OrderBlueprint::new("quit").also_known("exit"));
        book.enlist(OrderBlueprint::new("quote").help_line("say it"));
        book.enlist(OrderBlueprint::new("r_lodScaleRigid").help_line("r_lodScaleRigid [f]"));
        book.enlist(OrderBlueprint::new("fx_dump"));
        book
    }

    #[test]
    fn a_static_completer_offers_the_values_that_start_with_the_typing() {
        let completer = FixedListSuggester::new(["+forward", "+back", "Fire"]);
        assert_eq!(completer.suggest("+"), ["+forward", "+back"]);
        assert_eq!(completer.suggest("fi"), ["Fire"], "either case");
        assert_eq!(completer.suggest(""), ["+forward", "+back", "Fire"]);
        assert!(completer.suggest("x").is_empty());
        assert!(completer.suggest("+forwardx").is_empty());
    }

    #[test]
    fn a_closure_is_a_completer() {
        let completer = |prefix: &str| vec![format!("{prefix}!")];
        assert_eq!(completer.suggest("a"), ["a!"]);
    }

    #[test]
    fn names_and_aliases_are_sorted_and_not_repeated() {
        let mut book = book();
        book.enlist(OrderBlueprint::new("map"));
        let expected = ["exit", "fx_dump", "map", "quit", "quote", "r_lodScaleRigid"];
        assert_eq!(book.all_names(), expected);
    }

    #[test]
    fn a_command_is_found_by_name_or_alias_in_the_case_it_was_given() {
        let book = book();
        let expected = Some("quit");
        assert_eq!(book.look_up("exit").map(|s| s.name.as_str()), expected);
        assert!(book.look_up("quit").is_some());
        assert!(book.look_up("Quit").is_none());
        assert!(book.look_up("nope").is_none());
    }

    #[test]
    fn the_first_command_of_a_name_is_the_one_found() {
        let mut book = book();
        book.enlist(OrderBlueprint::new("map").help_line("second"));
        let expected = Some("map <name>");
        assert_eq!(book.look_up("map").map(|s| s.help_line.as_str()), expected);
    }

    #[test]
    fn help_has_a_line_per_command_in_name_order() {
        let expected = [
            "fx_dump",
            "map <name>",
            "quit",
            "quote — say it",
            "r_lodScaleRigid [f]"
        ];
        assert_eq!(book().usage_listing(), expected);
    }

    #[test]
    fn an_empty_line_suggests_nothing() {
        assert!(book().hint_list("", 0).is_empty());
        assert!(book().hint_list("   ", 3).is_empty());
    }

    #[test]
    fn a_name_is_completed_best_fit_first() {
        let book = book();
        assert_eq!(book.hint_list("qu", 2), ["quit", "quote"]);
        assert_eq!(book.hint_list("QUIT", 4), ["quit"]);
        assert_eq!(book.hint_list("ex", 2), ["exit"]);
    }

    #[test]
    fn a_name_is_also_found_by_a_word_in_it_or_part_of_it() {
        let book = book();
        assert_eq!(book.hint_list("dump", 4), ["fx_dump"]);
        assert_eq!(book.hint_list("lodscale", 8), ["r_lodScaleRigid"]);
        let expected = ["fx_dump"];
        assert_eq!(book.hint_list("fxd", 3), expected, "letters in order");
        assert!(
            book.hint_list("xf", 2).is_empty(),
            "too short to spread"
        );
    }

    #[test]
    fn only_what_is_before_the_caret_is_looked_at() {
        let book = book();
        assert_eq!(book.hint_list("quit now", 2), ["quit", "quote"]);
        assert_eq!(book.hint_list("é€", 99), Vec::<String>::new());
    }

    #[test]
    fn an_argument_is_completed_by_its_own_completer() {
        let book = book();
        let expected = ["mp_nuketown", "mp_raid"];
        assert_eq!(book.hint_list("map mp_", 7), expected);
        let expected = 3;
        assert_eq!(book.hint_list("map ", 4).len(), expected, "a new argument offers all");
        assert_eq!(book.hint_list("map zm", 6), ["zm_town"]);
    }

    #[test]
    fn an_alias_completes_arguments_too() {
        let mut book = book();
        let go = OrderBlueprint::new("go")
            .also_known("travel")
            .arg(FixedListSuggester::new(["a"]));
        book.enlist(go);
        assert_eq!(book.hint_list("travel ", 7), ["a"]);
    }

    #[test]
    fn nothing_is_offered_where_there_is_no_completer() {
        let book = book();
        assert!(
            book.hint_list("map mp_raid ", 12).is_empty(),
            "second argument"
        );
        assert!(book.hint_list("quit ", 5).is_empty());
        assert!(book.hint_list("nope ", 5).is_empty());
    }

    #[test]
    fn ranking_prefers_exact_then_prefix_then_the_shorter_name() {
        let names: Vec<String> = ["mapa", "map", "xmap", "map_long"]
            .iter()
            .map(|n| (*n).to_owned())
            .collect::<Vec<String>>();
        assert_eq!(rank(&names, "map"), ["map", "mapa", "map_long", "xmap"]);
    }
}
