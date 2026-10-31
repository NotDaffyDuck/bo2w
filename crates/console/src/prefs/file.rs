//! The text of the settings file: a `key=value` line for each setting, then
//! the binds as console `bind` lines.

use std::str::FromStr;

use frame::ScreenPixelSize;
use frame::PlayerPrefsBundle;

use super::layout;
use crate::BindingTable;

/// One setting: the key it is saved under and how it is written and read.
struct Field {
    key: &'static str,
    write: fn(&PlayerPrefsBundle) -> String,
    read: fn(&mut PlayerPrefsBundle, &str),
}

/// Sets `slot` from `raw`, or leaves it when `raw` does not parse.
fn assign<T: FromStr>(slot: &mut T, raw: &str) {
    if let Ok(parsed) = raw.parse() {
        *slot = parsed;
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
            if let Some((wide, high)) = text.split_once('x')
                && let (Ok(w), Ok(h)) = (wide.parse(), high.parse())
            {
                settings.resolution = ScreenPixelSize::new(w, h);
            }
        },
    },
    field!("fullscreen", full_screen),
    field!("vsync", sync_frames),
    field!("master_volume", main_level, 3),
    field!("voice_volume", voice_volume, 3),
    field!("music_volume", music_volume, 3),
    field!("sfx_volume", sfx_volume, 3),
    field!("cinematic_volume", cinematic_volume, 3),
    field!("audio_preset", audio_preset),
    field!("brightness", brightness, 3),
    field!("fov", view_angle, 0),
    field!("shadows", shadows),
    field!("depth_of_field", depth_of_field),
    field!("bloom", bloom),
    field!("sensitivity", sensitivity, 3),
    field!("invert_mouse", flip_look),
    Field {
        key: "player_name",
        // A name is one line, and an `=` in it would move the key.
        write: |settings| settings.shown_name.replace(['\n', '\r', '='], " "),
        read: |settings, text| settings.shown_name = text.to_owned(),
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
    // bo2zm: more of Black Ops II's Settings rows.
    field!("mature", mature),
    field!("corpses", corpses),
    field!("draw_fps", draw_fps),
    field!("max_fps", max_fps),
    field!("fxaa", fxaa),
    field!("aa_samples", aa_samples),
    field!("tex_filter", tex_filter),
    field!("tex_quality", tex_quality),
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
pub(super) fn render(settings: &PlayerPrefsBundle, binds: &BindingTable) -> String {
    let mut lines = vec!["// BO2MP user settings v2".to_owned()];
    lines.extend(
        FIELDS
            .iter()
            .map(|field| format!("{}={}", field.key, (field.write)(settings))),
    );
    lines.push(format!("binds_layout={}", layout::CURRENT));
    lines.push("unbindall".to_owned());
    lines.extend(binds.control_listing());
    let text = lines.join("\n");
    text + "\n"
}

/// Applies a file's text to `settings` and `keymap`. A value that is not one
/// leaves its setting as it was. A file with no pad binds gets the pad
/// layout it names.
pub(super) fn read(source: &str, settings: &mut PlayerPrefsBundle, binds: &mut BindingTable) -> Notes {
    let mut notes = Notes::default();
    let mut bind_lines = String::new();
    let mut layout = 1;
    for entry in source.lines().map(str::trim) {
        if entry.is_empty() || entry.starts_with("//") {
            continue;
        }
        if entry.starts_with("bind ") || entry == "unbindall" {
            bind_lines.push_str(entry);
            bind_lines.push('\n');
            continue;
        }
        let Some((name, value)) = entry.split_once('=') else {
            notes
                .warnings
                .push(format!("ignored malformed setting line: {entry}"));
            continue;
        };
        let (key, value) = (name.trim(), value.trim());
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
    if !bind_lines.is_empty() {
        if layout < layout::CURRENT {
            bind_lines = layout::migrate(&bind_lines, layout);
            notes.moved_from = Some(layout);
        }
        notes.warnings.extend(
            binds
                .run_prefs_script(&bind_lines)
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
    use bevy::prelude::*;

    use crate::prefs::file::*;
    use crate::ActionBinding;
    use input_t6::action_number_for;

    fn fresh() -> (PlayerPrefsBundle, BindingTable) {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        (PlayerPrefsBundle::default(), binds)
    }

    fn read_into(source: &str) -> (PlayerPrefsBundle, BindingTable, Notes) {
        let (mut prefs, mut table) = fresh();
        let notes = read(source, &mut prefs, &mut table);
        (prefs, table, notes)
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
        settings.resolution = ScreenPixelSize::new(1920, 1080);
        settings.full_screen = !settings.full_screen;
        settings.view_angle = 100.0;
        settings.main_level = 0.5;
        settings.audio_preset = 3;
        settings.shown_name = "Sam".into();
        settings.pad_layout = 2;
        settings.pad_deadzone_left = 0.25;
        settings.bo2_fast = true;
        settings.omni_movement = true;
        settings.mature = false;
        settings.corpses = 5;
        settings.draw_fps = true;
        settings.max_fps = 144;
        settings.fxaa = true;
        settings.aa_samples = 4;
        settings.tex_filter = 1;
        settings.tex_quality = 2;
        binds.assign(ActionBinding::Board(KeyCode::KeyJ), action_number_for("+frag").unwrap());
        let text = render(&settings, &binds);

        let (read_settings, read_binds, notes) = read_into(&text);
        assert_eq!(render(&read_settings, &read_binds), text);
        assert_eq!(read_settings.resolution, ScreenPixelSize::new(1920, 1080));
        assert_eq!(read_settings.view_angle, 100.0);
        assert_eq!(read_settings.main_level, 0.5);
        assert_eq!(read_settings.audio_preset, 3);
        assert_eq!(read_settings.shown_name, "Sam");
        assert_eq!(read_settings.pad_layout, 2);
        assert_eq!(read_settings.pad_deadzone_left, 0.25);
        assert!(read_settings.bo2_fast && read_settings.omni_movement);
        assert!(!read_settings.mature && read_settings.draw_fps && read_settings.fxaa);
        let want = (5, 144, 4, 1, 2);
        assert_eq!((
            read_settings.corpses,
            read_settings.max_fps,
            read_settings.aa_samples,
            read_settings.tex_filter,
            read_settings.tex_quality
        ), want);
        assert_eq!(read_binds.bound_to(ActionBinding::Board(KeyCode::KeyJ)), action_number_for("+frag"));
        assert_eq!(notes, Notes::default());
    }

    #[test]
    fn a_name_cannot_break_its_line() {
        let (mut settings, binds) = fresh();
        settings.shown_name = "a=b\nc\rd".into();
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
        assert_eq!(settings.view_angle, 90.0);
        let want = [
            "ignored unknown setting `colour`",
            "ignored malformed setting line: no equals sign"
        ];
        assert_eq!(notes.warnings, want);
    }

    #[test]
    fn comments_blank_lines_and_spaces_around_a_value_are_fine() {
        let (settings, _, notes) = read_into("// note\n\n  fov = 90 \r\nplayer_name = Sam\n");
        assert_eq!(settings.view_angle, 90.0);
        assert_eq!(settings.shown_name, "Sam");
        assert!(notes.warnings.is_empty());
    }

    #[test]
    fn quality_reads_fast_in_any_case() {
        assert!(read_into("bo2_quality=FAST").0.bo2_fast);
        let (mut prefs, mut table) = fresh();
        prefs.bo2_fast = true;
        read("bo2_quality=full", &mut prefs, &mut table);
        assert!(!prefs.bo2_fast);
    }

    #[test]
    fn saved_binds_replace_the_defaults() {
        let (_, binds, notes) = read_into("binds_layout=5\nunbindall\nbind J +activate\n");
        assert_eq!(binds.bound_to(ActionBinding::Board(KeyCode::KeyJ)), action_number_for("+activate"));
        assert_eq!(binds.bound_to(ActionBinding::Board(KeyCode::KeyF)), None);
        assert_eq!(notes.moved_from, None);
        assert!(notes.warnings.is_empty());
    }

    #[test]
    fn binds_saved_under_an_older_layout_are_moved() {
        let (_, binds, notes) = read_into("unbindall\nbind e +activate\nbind BUTTON_A +gostand\n");
        assert_eq!(notes.moved_from, Some(1));
        assert_eq!(binds.bound_to(ActionBinding::Board(KeyCode::KeyE)), None);
        assert_eq!(binds.bound_to(ActionBinding::Board(KeyCode::KeyF)), action_number_for("+activate"));
        assert!(
            binds
                .control_listing()
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
        let mut expected = BindingTable::default();
        expected.apply_pad_layout(1);
        for line in expected.control_listing() {
            assert!(binds.control_listing().contains(&line), "missing {line}");
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
