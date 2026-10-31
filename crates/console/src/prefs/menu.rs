//! The options menu's side of the settings: `set ui_*` commands change a
//! setting, and every setting is shown back to the menu as a dvar.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::ScreenPixelSize;
use frame::PlayerPrefsBundle;
use frame::TestControllerRumble;
use frame::UiMenuDvars;
use render_frontend::assemble::draw_items::dof::BokehTunables;
use render_frontend::assemble::draw_items::dof::BloomTunables;
use render_frontend::prepare::scene::eye_parms::SmEnableDvar;

use crate::TypedOrder;
use std::str::FromStr;

/// One option of the menu: the dvar it is set and shown through, how a value
/// from the menu changes the setting, and how the setting reads back.
struct Control {
    dvar: &'static str,
    set: fn(&mut PlayerPrefsBundle, &str),
    show: fn(&PlayerPrefsBundle) -> String,
}

/// A number the menu may send. Floats must be finite; anything else is a typo.
trait Reading: FromStr {
    fn usable(&self) -> bool {
        true
    }
}

impl Reading for u8 {}

impl Reading for f32 {
    fn usable(&self) -> bool {
        self.is_finite()
    }
}

fn reading<T: Reading>(value: &str) -> Option<T> {
    value.parse().ok().filter(T::usable)
}

fn flag(on: bool) -> String {
    String::from(if on { "1" } else { "0" })
}

macro_rules! flag {
    ($dvar:literal, $member:ident) => {
        Control {
            dvar: $dvar,
            set: |settings, value| settings.$member = value == "1",
            show: |settings| flag(settings.$member),
        }
    };
}

macro_rules! number {
    ($dvar:literal, $member:ident) => {
        Control {
            dvar: $dvar,
            set: |settings, value| {
                if let Some(number) = reading(value) {
                    settings.$member = number;
                }
            },
            show: |settings| settings.$member.to_string(),
        }
    };
}

static CONTROLS: &[Control] = &[
    Control {
        dvar: "ui_r_mode",
        set: |settings, value| {
            if let Some((wide, high)) = value.split_once('x')
                && let (Ok(w), Ok(h)) = (wide.parse(), high.parse())
            {
                settings.resolution = ScreenPixelSize::new(w, h);
            }
        },
        show: |settings| settings.resolution.to_string(),
    },
    flag!("ui_r_displayMode", full_screen),
    flag!("ui_r_vsync", sync_frames),
    number!("ui_volume", main_level),
    Control {
        dvar: "ui_player_name",
        set: |settings, value| settings.shown_name = value.to_owned(),
        show: |settings| settings.shown_name.clone(),
    },
    number!("ui_fov", view_angle),
    number!("ui_brightness", brightness),
    flag!("ui_shadows", shadows),
    flag!("ui_dof", depth_of_field),
    flag!("ui_bloom", bloom),
    number!("ui_pad_layout", pad_layout),
    number!("ui_pad_stick_layout", pad_stick_layout),
    number!("ui_pad_sensitivity", pad_sensitivity),
    number!("ui_pad_ads_sensitivity", pad_ads_sensitivity),
    flag!("ui_pad_invert", pad_invert),
    number!("ui_pad_curve", pad_curve),
    number!("ui_pad_aim_assist", pad_aim_assist),
    number!("ui_pad_prompts", pad_prompts),
    flag!("ui_pad_vibration", pad_vibration),
    number!("ui_pad_deadzone_left", pad_deadzone_left),
    number!("ui_pad_deadzone_right", pad_deadzone_right),
];

/// Pressing "test" in the menu is a request, not a setting.
const RUMBLE_TEST: &str = "ui_pad_test_rumble";

/// The renderer switches that follow the shadow, depth of field and bloom options.
#[derive(SystemParam)]
pub(crate) struct RenderToggles<'w> {
    shadows: ResMut<'w, SmEnableDvar>,
    depth_of_field: ResMut<'w, BokehTunables>,
    glow: ResMut<'w, BloomTunables>,
}

impl RenderToggles<'_> {
    fn follow(&mut self, settings: &PlayerPrefsBundle) {
        self.shadows.enabled = Some(settings.shadows);
        self.depth_of_field.enable = settings.depth_of_field;
        self.glow.enable = settings.bloom;
    }
}

pub(crate) fn native_menu_settings(
    mut commands: MessageReader<TypedOrder>,
    mut prefs: ResMut<PlayerPrefsBundle>,
    mut dvars: ResMut<UiMenuDvars>,
    mut render: RenderToggles,
    mut rumble: MessageWriter<TestControllerRumble>,
) {
    for order in commands.read() {
        if !matches!(order.name.as_str(), "set" | "seta") {
            continue;
        }
        let [name, value, ..] = order.words.as_slice() else {
            continue;
        };
        if name.eq_ignore_ascii_case(RUMBLE_TEST) {
            if value == "1" {
                rumble.write(TestControllerRumble);
            }
            continue;
        }
        let Some(control) = CONTROLS
            .iter()
            .find(|control| name.eq_ignore_ascii_case(control.dvar))
        else {
            continue;
        };
        (control.set)(&mut prefs, value);
        prefs.pull_into_limits();
        prefs.touch();
    }
    if prefs.is_changed() {
        render.follow(&prefs);
    }
    for control in CONTROLS {
        dvars.set(control.dvar, (control.show)(&prefs));
    }
}

#[cfg(test)]
mod tests {
    use crate::prefs::menu::*;
    use bevy::app::Update;

    fn menu_app() -> App {
        let mut app = App::default();
        app.init_resource::<PlayerPrefsBundle>()
            .init_resource::<UiMenuDvars>()
            .insert_resource(SmEnableDvar::default())
            .init_resource::<BokehTunables>()
            .init_resource::<BloomTunables>()
            .add_message::<TypedOrder>()
            .add_message::<TestControllerRumble>()
            .add_systems(Update, native_menu_settings);
        app
    }

    fn run(app: &mut App, typed: &str) {
        let command = TypedOrder::parse(typed).expect("a command line");
        app.world_mut().write_message(command);
        app.update();
    }

    fn settings(app: &App) -> &PlayerPrefsBundle {
        app.world().resource::<PlayerPrefsBundle>()
    }

    fn dvar(app: &App, name: &str) -> Option<String> {
        app.world()
            .resource::<UiMenuDvars>()
            .get(name)
            .map(str::to_owned)
    }

    fn rumbles(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<TestControllerRumble>>()
            .drain()
            .count()
    }

    #[test]
    fn a_resolution_is_width_x_height() {
        let mut app = menu_app();
        run(&mut app, "set ui_r_mode 1920x1080");
        let expected = ScreenPixelSize::new(1920, 1080);
        assert_eq!(settings(&app).resolution, expected);
        assert_eq!(dvar(&app, "ui_r_mode").as_deref(), Some("1920x1080"));
        for bad in ["big", "1920", "x", "1920x", "axb"] {
            run(&mut app, &format!("set ui_r_mode {bad}"));
            let expected = ScreenPixelSize::new(1920, 1080);
            assert_eq!(settings(&app).resolution, expected);
        }
    }

    #[test]
    fn flags_are_set_by_one_and_cleared_by_anything_else() {
        let mut app = menu_app();
        run(&mut app, "set ui_r_displayMode 1");
        run(&mut app, "set ui_r_vsync 1");
        assert!(settings(&app).full_screen && settings(&app).sync_frames);
        assert_eq!(dvar(&app, "ui_r_displayMode").as_deref(), Some("1"));
        run(&mut app, "seta ui_r_displayMode 0");
        run(&mut app, "set ui_r_vsync yes");
        assert!(!settings(&app).full_screen && !settings(&app).sync_frames);
        assert_eq!(dvar(&app, "ui_r_vsync").as_deref(), Some("0"));
    }

    #[test]
    fn the_name_of_a_dvar_is_not_case_sensitive() {
        let mut app = menu_app();
        run(&mut app, "set UI_R_DISPLAYMODE 1");
        assert!(settings(&app).full_screen);
    }

    #[test]
    fn numbers_are_taken_and_typos_are_not() {
        let mut app = menu_app();
        run(&mut app, "set ui_fov 90");
        assert_eq!(settings(&app).view_angle, 90.0);
        for bad in ["abc", "nan", "inf", "-inf", ""] {
            run(&mut app, &format!("set ui_fov {bad}"));
            assert_eq!(settings(&app).view_angle, 90.0, "{bad:?} changed the fov");
        }
        run(&mut app, "set ui_pad_curve 2");
        run(&mut app, "set ui_pad_curve 300");
        run(&mut app, "set ui_pad_curve -1");
        assert_eq!(settings(&app).pad_curve, 2);
    }

    #[test]
    fn a_value_is_cleaned_up_the_moment_it_is_set() {
        let mut app = menu_app();
        run(&mut app, "set ui_fov 9000");
        assert_eq!(settings(&app).view_angle, PlayerPrefsBundle::FOV_MAX);
        let expected = Some(PlayerPrefsBundle::FOV_MAX.to_string());
        assert_eq!(dvar(&app, "ui_fov"), expected);
    }

    #[test]
    fn the_player_name_is_taken_as_typed_then_cut_down() {
        let mut app = menu_app();
        run(&mut app, "set ui_player_name Sam");
        assert_eq!(settings(&app).shown_name, "Sam");
        assert_eq!(dvar(&app, "ui_player_name").as_deref(), Some("Sam"));
    }

    #[test]
    fn a_set_marks_the_settings_changed_so_they_are_saved() {
        let mut app = menu_app();
        app.update();
        let before = settings(&app).revision;
        run(&mut app, "set ui_volume 0.5");
        assert_ne!(settings(&app).revision, before);
        assert_eq!(settings(&app).main_level, 0.5);
    }

    #[test]
    fn other_commands_and_unknown_dvars_change_nothing() {
        let mut app = menu_app();
        app.update();
        let before = settings(&app).clone();
        run(&mut app, "echo hello");
        run(&mut app, "set ui_not_a_dvar 1");
        run(&mut app, "set ui_fov");
        run(&mut app, "get ui_fov 100");
        assert_eq!(settings(&app), &before);
    }

    #[test]
    fn the_rumble_test_is_a_request_and_changes_no_setting() {
        let mut app = menu_app();
        app.update();
        let before = settings(&app).clone();
        run(&mut app, "set ui_pad_test_rumble 1");
        assert_eq!(rumbles(&mut app), 1);
        run(&mut app, "set ui_pad_test_rumble 0");
        assert_eq!(rumbles(&mut app), 0, "none was sent the second time");
        assert_eq!(settings(&app), &before);
    }

    #[test]
    fn the_render_switches_follow_the_options() {
        let mut app = menu_app();
        run(&mut app, "set ui_shadows 1");
        run(&mut app, "set ui_dof 1");
        run(&mut app, "set ui_bloom 1");
        assert_eq!(app.world().resource::<SmEnableDvar>().enabled, Some(true));
        assert!(app.world().resource::<BokehTunables>().enable);
        assert!(app.world().resource::<BloomTunables>().enable);
        run(&mut app, "set ui_shadows 0");
        run(&mut app, "set ui_dof 0");
        run(&mut app, "set ui_bloom 0");
        let shadows = app.world().resource::<SmEnableDvar>().enabled;
        assert!(shadows == Some(false));
        assert!(!app.world().resource::<BokehTunables>().enable);
        assert!(!app.world().resource::<BloomTunables>().enable);
    }

    #[test]
    fn every_option_is_shown_from_the_first_frame() {
        let mut app = menu_app();
        app.update();
        for control in CONTROLS {
            let actual = dvar(&app, control.dvar);
            assert_eq!(actual, Some((control.show)(settings(&app))), "{} is not shown", control.dvar);
        }
    }

    #[test]
    fn no_two_options_share_a_dvar() {
        let mut seen: std::collections::HashSet<String> = Default::default();
        for control in CONTROLS {
            assert!(
                seen.insert(control.dvar.to_ascii_lowercase()),
                "{} twice",
                control.dvar
            );
        }
        assert!(!seen.contains(RUMBLE_TEST));
    }
}
