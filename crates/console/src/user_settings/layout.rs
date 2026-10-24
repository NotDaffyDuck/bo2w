//! The layout number a settings file's binds were saved under, and the move
//! of an older layout to the current one.

use crate::binds::DEFAULT_CONTROLS;

/// Layout 4 was the retail PC keys (`default_controls.cfg`). Layout 5 has one
/// streak key on the keyboard: the keys for streaks 1 and 2 are unbound and
/// key 3 brings out every streak.
pub(super) const CURRENT: u32 = 5;

/// A `bind KEY command words...` line as its key and the words after it.
fn bind_line(line: &str) -> Option<(&str, Vec<&str>)> {
    let mut words = line.split_whitespace();
    if words.next()? != "bind" {
        return None;
    }
    let key = words.next()?;
    Some((key, words.collect()))
}

fn is_pad_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.starts_with("BUTTON_") || key.starts_with("DPAD_")
}

/// A keyboard or mouse bind of streak slot 1 or 2, which layout 5 leaves free.
fn is_keyboard_streak_slot(line: &str) -> bool {
    bind_line(line).is_some_and(|(key, words)| {
        !is_pad_key(key) && matches!(words.as_slice(), ["+actionslot", "1" | "2"])
    })
}

/// `script` (bind lines saved under layout `from`) moved to the current
/// layout. From layout 4 only the streak keys go. From anything older every
/// keyboard and mouse key goes back to the retail defaults, since the old
/// defaults were our own and not the player's picks; saved pad buttons stay.
pub(super) fn migrate(script: &str, from: u32) -> String {
    if from >= CURRENT {
        return script.to_owned();
    }
    if from == 4 {
        return script
            .lines()
            .filter(|line| !is_keyboard_streak_slot(line))
            .map(|line| format!("{line}\n"))
            .collect();
    }
    let defaults = DEFAULT_CONTROLS
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("bind "));
    let kept_pad = script
        .lines()
        .map(str::trim)
        .filter(|line| bind_line(line).is_some_and(|(key, _)| is_pad_key(key)));
    std::iter::once("unbindall")
        .chain(defaults)
        .chain(kept_pad)
        .map(|line| format!("{line}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn an_old_layout_gets_the_retail_keys_and_keeps_its_pad_buttons() {
        let saved = [
            "unbindall",
            "bind BUTTON_A +gostand",
            "bind DPAD_UP +actionslot 1",
            "bind MOUSE2 +speed_throw",
            "bind SHIFT +sprint",
            "bind e +activate",
            "bind f +frag",
            "bind c +movedown",
        ]
        .join("\n");
        let moved = migrate(&saved, 2);
        let moved = lines(&moved);
        for want in [
            "bind F +activate",
            "bind G +frag",
            "bind C +stance",
            "bind CTRL toggleprone",
            "bind SHIFT +breath_sprint",
            "bind MOUSE2 +speed_throw",
            "bind BUTTON_A +gostand",
            "bind DPAD_UP +actionslot 1",
        ] {
            assert!(moved.contains(&want), "missing {want:?} in {moved:?}");
        }
        for gone in ["bind e +activate", "bind f +frag", "bind c +movedown"] {
            assert!(!moved.contains(&gone), "kept {gone:?} in {moved:?}");
        }
        assert_eq!(moved[0], "unbindall");
    }

    #[test]
    fn the_oldest_layout_is_the_defaults_plus_the_pad_lines() {
        let defaults = DEFAULT_CONTROLS
            .lines()
            .filter(|line| line.trim().starts_with("bind "))
            .count();
        let moved = migrate("bind button_b +melee\nbind x +reload", 1);
        assert_eq!(moved.lines().count(), 1 + defaults + 1);
        assert!(moved.lines().any(|line| line == "bind button_b +melee"));
    }

    #[test]
    fn the_current_layout_is_left_alone() {
        let saved = "unbindall\nbind e +activate";
        assert_eq!(migrate(saved, CURRENT), saved);
        assert_eq!(migrate(saved, CURRENT + 3), saved);
    }

    #[test]
    fn layout_four_loses_only_the_keyboard_keys_of_streaks_one_and_two() {
        let saved = [
            "unbindall",
            "bind 1 +actionslot 1",
            "bind MOUSE4 +actionslot 2",
            "bind button_x +actionslot 2",
            "bind DPAD_UP +actionslot 1",
            "bind 3 +actionslot 4",
            "bind F +activate",
        ]
        .join("\n");
        let moved = migrate(&saved, 4);
        assert_eq!(
            lines(&moved),
            [
                "unbindall",
                "bind button_x +actionslot 2",
                "bind DPAD_UP +actionslot 1",
                "bind 3 +actionslot 4",
                "bind F +activate",
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_a_bind_is_not_a_pad_bind() {
        assert!(bind_line("unbindall").is_none());
        assert!(bind_line("bind").is_none());
        assert_eq!(
            bind_line("bind  e   +activate"),
            Some(("e", vec!["+activate"]))
        );
        assert!(is_pad_key("dpad_left") && is_pad_key("BUTTON_A"));
        assert!(!is_pad_key("e") && !is_pad_key("MOUSE1"));
    }
}
