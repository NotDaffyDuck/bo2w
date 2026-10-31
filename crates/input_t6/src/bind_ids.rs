//! The bindable commands, by id. A key's binding is an id into [`INPUT_COMMAND_NAMES`].

/// Spells out the command table: id 0 is "no command", then each hold command as a pair
/// (`+name` going down, `-name` coming up), then the commands that act once.
macro_rules! command_table {
    (holds: [$($hold:literal),* $(,)?], once: [$($once:literal),* $(,)?] $(,)?) => {
        /// The hold commands, in pair order; pair `p` is ids `2p + 1` (down) and `2p + 2` (up).
        pub(crate) const HOLD_COMMANDS: &[&str] = &[$($hold),*];

        pub const BIND_COMMAND_TABLE: [&str; 81] =
            ["", $(concat!("+", $hold), concat!("-", $hold),)* $($once),*];
    };
}

command_table! {
    holds: [
        "attack", "melee", "frag", "smoke", "breath_sprint", "usereload", "speed_throw",
        "actionslot 1", "actionslot 2", "actionslot 3", "actionslot 4",
        "stance", "gostand", "forward", "back", "moveleft", "moveright", "movedown",
        "left", "right", "lookup", "lookdown", "strafe", "holdbreath", "activate", "reload",
        "prone", "mlook", "toggleads_throw", "sprint", "scores", "talk",
    ],
    once: [
        "togglemenu", "weapnext", "pause", "chatmodepublic", "chatmodeteam", "weapprev",
        "centerview", "togglecrouch", "toggleprone", "goprone", "gocrouch", "toggleads",
        "leaveads",
        // The switch-weapon key Black Ops II binds by default (mouse wheel up, pad Y): a tap
        // is the next weapon when it comes up, a hold the inventory weapon. Its up half is
        // the one pair outside the hold range.
        "+weapnext_inventory", "-weapnext_inventory",
        "inventory",
    ],
}

/// Ids below this are hold pairs: odd goes down, the even one after it comes up.
pub const HOLD_ID_END: u32 = 1 + 2 * HOLD_COMMANDS.len() as u32;

/// `+weapnext_inventory`; `-weapnext_inventory` is the id after it.
pub const WEAPNEXT_INVENTORY_DOWN: u32 = 78;

/// `inventory`: take the inventory weapon now.
pub const INVENTORY_COMMAND: u32 = 80;

/// The key number script presses use; it is not a key on the board.
pub const SCRIPTED_KEY_SLOT: i32 = 0x400;

/// Longest command name a bind can give; longer text is cut to this.
const NAME_MAX: usize = 32;

/// The id of the command whose name is `prefix` then `word` (with `word` lowercased).
fn find_command(prefix: &[u8], word: &[u8]) -> Option<u32> {
    let id = BIND_COMMAND_TABLE.iter().skip(1).position(|name| {
        let name = name.as_bytes();
        name.len() == prefix.len() + word.len()
            && name.starts_with(prefix)
            && name[prefix.len()..]
                .iter()
                .zip(word)
                .all(|(n, w)| *n == w.to_ascii_lowercase())
    })?;
    Some(id as u32 + 1)
}

/// The id a bind's command text names, in any case. A hold command can be named without its
/// `+` (`attack` binds `+attack`).
pub fn action_number_for(text: &str) -> Option<u32> {
    let word = text.trim().as_bytes();
    let word = &word[..word.len().min(NAME_MAX)];
    if word.is_empty() {
        return None;
    }
    if let Some(ident) = find_command(b"", word) {
        return Some(ident);
    }
    if matches!(word[0], b'+' | b'-') || word.len() == NAME_MAX {
        return None;
    }
    find_command(b"+", word)
}

pub fn action_label(id: u32) -> Option<&'static str> {
    BIND_COMMAND_TABLE.get(id as usize).copied().filter(|label| !label.is_empty())
}

/// The command a key bound to `binding` runs when it comes up: the up half of a hold pair.
pub fn release_action_number(binding: u32) -> Option<u32> {
    let pair_down = binding < HOLD_ID_END && binding % 2 == 1;
    (pair_down || binding == WEAPNEXT_INVENTORY_DOWN).then_some(binding + 1)
}

pub fn action_labels() -> impl Iterator<Item = &'static str> {
    BIND_COMMAND_TABLE.into_iter().skip(1)
}
