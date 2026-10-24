//! The text of the settings file: a `key=value` line for each setting, then
//! the binds as console `bind` lines.

use std::str::FromStr;

use frame::{DisplayResolution, GameSettings};

use super::layout;
use crate::KeyBinds;

/// One setting: the key it is saved under and how it is written and read.
struct Field {
    key: &'static str,
    write: fn(&GameSettings) -> String,
    read: fn(&mut GameSettings, &str),
}

/// Sets `slot` from `text`, or leaves it when `text` is not a value.
fn assign<T: FromStr>(slot: &mut T, text: &str) {
    if let Ok(value) = text.parse() {
        *slot = value;
    }
}

/// A setting saved as its own text, or with `places` decimals.
macro_rules! field {
    ($key:literal, $slot:ident) => {
        Field {
            key: $key,
            write: |settings| settings.$slot.to_string(),
            read: |settings, text| assign(&mut settings.$slot, text),
        }
    };
    ($key:literal, $slot:ident, $places:literal) => {
        Field {
            key: $key,
            write: |settings| format!("{:.*}", $places, settings.$slot),
            read: |settings, text| assign(&mut settings.$slot, text),
        }
    };
}

/// In the order they are written. `binds_layout` follows the last of them.
static FIELDS: &[Field] = &[
    Field {
        key: "resolution",
        write: |settings| settings.resolution.to_string(),
        read: |settings, text| {
            if let Some((width, height)) = text.split_once('x')
                && let (Ok(width), Ok(height)) = (width.parse(), height.parse())
            {
                settings.resolution = DisplayResolution::new(width, height);
            }
        },
    },
    field!("fullscreen", fullscreen),
    field!("vsync", vsync),
    field!("master_volume", master_volume, 3),
    field!("voice_volume", voice_volume, 3),
    field!("music_volume", music_volume, 3),
    field!("sfx_volume", sfx_volume, 3),
    field!("cinematic_volume", cinematic_volume, 3),
    field!("audio_preset", audio_preset),
    field!("brightness", brightness, 3),
    field!("fov", fov, 0),
    field!("shadows", shadows),
    field!("depth_of_field", depth_of_field),
    field!("bloom", bloom),
    field!("sensitivity", sensitivity, 3),
    field!("invert_mouse", invert_mouse),
    Field {
        key: "player_name",
        // A name is one line, and an `=` in it would move the key.
        write: |settings| settings.player_name.replace(['\n', '\r', '='], " "),
        read: |settings, text| settings.player_name = text.to_owned(),
    },
    field!("pad_layout", pad_layout),
    field!("pad_stick_layout", pad_stick_layout),
    field!("pad_sensitivity", pad_sensitivity, 2),
    field!("pad_ads_sensitivity", pad_ads_sensitivity, 2),
    field!("pad_invert", pad_invert),
    field!("pad_curve", pad_curve),
    field!("pad_aim_assist", pad_aim_assist),
    field!("pad_prompts", pad_prompts),
    field!("pad_vibration", pad_vibration),
    field!("pad_deadzone_left", pad_deadzone_left, 2),
    field!("pad_deadzone_right", pad_deadzone_right, 2),
    Field {
        key: "bo2_quality",
        write: |settings| (if settings.bo2_fast { "fast" } else { "full" }).to_owned(),
        read: |settings, text| settings.bo2_fast = text.eq_ignore_ascii_case("fast"),
    },
    field!("omni_movement", omni_movement),
];

/// What reading a file had to say, for the caller to log.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Notes {
    /// Lines and binds that were not understood.
    pub(super) warnings: Vec<String>,
    /// The layout the binds were saved under, when they were moved to the
    /// current one.
    pub(super) moved_from: Option<u32>,
}

/// The file's text for these settings and binds.
pub(super) fn render(settings: &GameSettings, binds: &KeyBinds) -> String {
    let mut lines = vec!["// BO2MP user settings v2".to_owned()];
    lines.extend(
        FIELDS
            .iter()
            .map(|field| format!("{}={}", field.key, (field.write)(settings))),
    );
    lines.push(format!("binds_layout={}", layout::CURRENT));
    lines.push("unbindall".to_owned());
    lines.extend(binds.list_lines());
    lines.push(String::new());
    lines.join("\n")
}

/// Applies a file's text to `settings` and `binds`. A value that is not one
/// leaves its setting as it was. A file with no pad binds gets the pad
/// layout it names.
pub(super) fn read(source: &str, settings: &mut GameSettings, binds: &mut KeyBinds) -> Notes {
    let mut notes = Notes::default();
    let mut script = String::new();
    let mut layout = 1;
    for line in source.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with("bind ") || line == "unbindall" {
            script.push_str(line);
            script.push('\n');
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            notes
                .warnings
                .push(format!("ignored malformed setting line: {line}"));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "binds_layout" {
            layout = value.parse().unwrap_or(1);
        } else if let Some(field) = FIELDS.iter().find(|field| field.key == key) {
            (field.read)(settings, value);
        } else {
            notes
                .warnings
                .push(format!("ignored unknown setting `{key}`"));
        }
    }
    if !script.is_empty() {
        if layout < layout::CURRENT {
            script = layout::migrate(&script, layout);
            notes.moved_from = Some(layout);
        }
        notes.warnings.extend(
            binds
                .apply_config_script(&script)
                .into_iter()
                .map(|warning| format!("settings bind: {warning}")),
        );
    }
    if !binds.has_pad_binds() {
        binds.apply_pad_layout(usize::from(settings.pad_layout.min(4)));
    }
    notes
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;

    use super::*;
    use crate::BindButton;

    fn fresh() -> (GameSettings, KeyBinds) {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        (GameSettings::default(), binds)
    }

    fn read_into(source: &str) -> (GameSettings, KeyBinds, Notes) {
        let (mut settings, mut binds) = fresh();
        let notes = read(source, &mut settings, &mut binds);
        (settings, binds, notes)
    }

    fn id(command: &str) -> Option<u32> {
        input_t6::command_id_lookup(command)
    }

    #[test]
    fn the_file_has_a_header_then_the_settings_then_the_binds() {
        let (settings, binds) = fresh();
        let text = render(&settings, &binds);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "// BO2MP user settings v2");
        assert!(lines.contains(&"fov=65"));
        assert!(lines.contains(&"master_volume=1.000"));
        let layout = lines
            .iter()
            .position(|line| *line == "binds_layout=5")
            .expect("layout line");
        assert_eq!(lines[layout + 1], "unbindall");
        assert!(lines[layout + 2..].contains(&"bind f +activate"));
        assert!(text.ends_with('\n'), "ends in a newline");
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let (mut settings, mut binds) = fresh();
        settings.resolution = DisplayResolution::new(1920, 1080);
        settings.fullscreen = !settings.fullscreen;
        settings.fov = 100.0;
        settings.master_volume = 0.5;
        settings.audio_preset = 3;
        settings.player_name = "Sam".into();
        settings.pad_layout = 2;
        settings.pad_deadzone_left = 0.25;
        settings.bo2_fast = true;
        settings.omni_movement = true;
        binds.set(BindButton::Key(KeyCode::KeyJ), id("+frag").unwrap());
        let text = render(&settings, &binds);

        let (read_settings, read_binds, notes) = read_into(&text);
        assert_eq!(render(&read_settings, &read_binds), text);
        assert_eq!(read_settings.resolution, DisplayResolution::new(1920, 1080));
        assert_eq!(read_settings.fov, 100.0);
        assert_eq!(read_settings.master_volume, 0.5);
        assert_eq!(read_settings.audio_preset, 3);
        assert_eq!(read_settings.player_name, "Sam");
        assert_eq!(read_settings.pad_layout, 2);
        assert_eq!(read_settings.pad_deadzone_left, 0.25);
        assert!(read_settings.bo2_fast && read_settings.omni_movement);
        assert_eq!(read_binds.get(BindButton::Key(KeyCode::KeyJ)), id("+frag"));
        assert_eq!(notes, Notes::default());
    }

    #[test]
    fn a_name_cannot_break_its_line() {
        let (mut settings, binds) = fresh();
        settings.player_name = "a=b\nc\rd".into();
        let text = render(&settings, &binds);
        assert!(text.lines().any(|line| line == "player_name=a b c d"));
    }

    #[test]
    fn a_value_that_is_not_one_leaves_the_setting() {
        let (settings, ..) = fresh();
        let (after, ..) = read_into(
            "fov=wide\nvsync=maybe\nresolution=1280x\nresolution=axb\npad_layout=-1\nmaster_volume=",
        );
        assert_eq!(after, settings);
    }

    #[test]
    fn lines_nobody_understands_are_reported_and_the_rest_still_apply() {
        let (settings, _, notes) = read_into("colour=blue\nno equals sign\nfov=90\n");
        assert_eq!(settings.fov, 90.0);
        assert_eq!(
            notes.warnings,
            [
                "ignored unknown setting `colour`",
                "ignored malformed setting line: no equals sign"
            ]
        );
    }

    #[test]
    fn comments_blank_lines_and_spaces_around_a_value_are_fine() {
        let (settings, _, notes) = read_into("// note\n\n  fov = 90 \r\nplayer_name = Sam\n");
        assert_eq!(settings.fov, 90.0);
        assert_eq!(settings.player_name, "Sam");
        assert!(notes.warnings.is_empty());
    }

    #[test]
    fn quality_reads_fast_in_any_case() {
        assert!(read_into("bo2_quality=FAST").0.bo2_fast);
        let (mut settings, mut binds) = fresh();
        settings.bo2_fast = true;
        read("bo2_quality=full", &mut settings, &mut binds);
        assert!(!settings.bo2_fast);
    }

    #[test]
    fn saved_binds_replace_the_defaults() {
        let (_, binds, notes) = read_into("binds_layout=5\nunbindall\nbind J +activate\n");
        assert_eq!(binds.get(BindButton::Key(KeyCode::KeyJ)), id("+activate"));
        assert_eq!(binds.get(BindButton::Key(KeyCode::KeyF)), None);
        assert_eq!(notes.moved_from, None);
        assert!(notes.warnings.is_empty());
    }

    #[test]
    fn binds_saved_under_an_older_layout_are_moved() {
        let (_, binds, notes) = read_into("unbindall\nbind e +activate\nbind BUTTON_A +gostand\n");
        assert_eq!(notes.moved_from, Some(1));
        assert_eq!(binds.get(BindButton::Key(KeyCode::KeyE)), None);
        assert_eq!(binds.get(BindButton::Key(KeyCode::KeyF)), id("+activate"));
        assert!(
            binds
                .list_lines()
                .contains(&"bind BUTTON_A +gostand".to_owned())
        );
    }

    #[test]
    fn a_bind_that_does_not_parse_is_reported() {
        let (_, _, notes) = read_into("binds_layout=5\nunbindall\nbind J +nosuchcommand\n");
        assert_eq!(notes.warnings.len(), 1, "{:?}", notes.warnings);
        assert!(notes.warnings[0].starts_with("settings bind: "));
    }

    #[test]
    fn a_file_with_no_pad_binds_gets_the_pad_layout_it_names() {
        let (_, binds, _) =
            read_into("pad_layout=1\nbinds_layout=5\nunbindall\nbind J +activate\n");
        assert!(binds.has_pad_binds());
        let mut expected = KeyBinds::default();
        expected.apply_pad_layout(1);
        for line in expected.list_lines() {
            assert!(binds.list_lines().contains(&line), "missing {line}");
        }
    }

    #[test]
    fn every_field_has_its_own_key_and_writes_one_line() {
        let (settings, _) = fresh();
        for (at, field) in FIELDS.iter().enumerate() {
            assert!(!field.key.is_empty() && !field.key.contains('='));
            assert!(
                FIELDS[at + 1..].iter().all(|other| other.key != field.key),
                "{} twice",
                field.key
            );
            assert!(!(field.write)(&settings).contains('\n'));
        }
        assert!(FIELDS.iter().all(|field| field.key != "binds_layout"));
    }
}
