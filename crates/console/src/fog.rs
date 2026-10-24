//! Fog from the console: `setexpfog`, the art sliders and the read-only
//! values the art script leaves behind. Every edit is made on a copy of the
//! map's fog and kept only when all of its values were accepted. The two
//! engine variables, `r_fog` and `r_zfar`, are in `dvars`.

use std::ops::RangeInclusive;

use asset_world::SunFog;
use bevy::prelude::*;
use net::FrameClock;
use render_frontend::{
    assemble::drawsurf::fog::MapFrameFog, prepare::scene::view_parms::PreparedSceneView,
};

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

type Refusal = &'static str;

const NO_FOG: Refusal = "fog unavailable: map has no authored fog";
const NOT_FINITE: Refusal = "expected finite numeric values";
const OUTSIDE: Refusal = "value outside domain or wrong arity";
const NOT_A_FLAG: Refusal = "expected 0 or 1";
const NOT_A_TRIPLE: Refusal = "expected three values in domain";
const READ_ONLY: Refusal = "read-only; use setexpfog or scr_art_tweak sliders";
const NO_CONSUMER: Refusal =
    "scr_fog_fraction is initialized to 1 by the art script but has no fog consumer";
const NO_VIEW: Refusal = "player view unavailable";
const SETEXPFOG_ARITY: Refusal = "setexpfog expects 7 values or 16 with flattened sun direction";

const SETEXPFOG_USAGE: &str = "setexpfog start half R G B maxOpacity seconds [sunR sunG sunB dirX dirY dirZ beginAngle endAngle scale] — local script bridge";

const ANY: RangeInclusive<f32> = -f32::MAX..=f32::MAX;
const FROM_ZERO: RangeInclusive<f32> = 0.0..=f32::MAX;
const UNIT: RangeInclusive<f32> = 0.0..=1.0;

/// A value the art script wrote last, shown but never set from here.
struct Kept {
    name: &'static str,
    show: fn(&MapFrameFog) -> String,
}

/// A slider of the fog art tweak: how it reads, and what it does to a copy
/// of the map's fog when it is given values.
struct Slider {
    name: &'static str,
    usage: &'static str,
    show: fn(&MapFrameFog) -> String,
    edit: fn(&mut MapFrameFog, &[f32], &PreparedSceneView) -> Result<(), Refusal>,
}

const KEPT: &[Kept] = &[
    Kept {
        name: "g_fogColorReadOnly",
        show: |map| format!("{:?}", map.fog.color_rgb),
    },
    Kept {
        name: "g_fogStartDistReadOnly",
        show: |map| map.fog.start_dist.to_string(),
    },
    Kept {
        name: "g_fogHalfDistReadOnly",
        show: |map| map.fog.halfway_dist.to_string(),
    },
    Kept {
        name: "g_fogMaxOpacityReadOnly",
        show: |map| map.fog.max_opacity.to_string(),
    },
    Kept {
        name: "g_sunFogEnabledReadOnly",
        show: |map| u8::from(map.fog.sun.is_some()).to_string(),
    },
    Kept {
        name: "g_sunFogColorReadOnly",
        show: |map| format!("{:?}", map.readonly_sun.color_rgb),
    },
    Kept {
        name: "g_sunFogDirReadOnly",
        show: |map| format!("{:?}", map.readonly_sun.sun_dir),
    },
    Kept {
        name: "g_sunFogBeginFadeAngleReadOnly",
        show: |map| map.readonly_sun.begin_angle_deg.to_string(),
    },
    Kept {
        name: "g_sunFogEndFadeAngleReadOnly",
        show: |map| map.readonly_sun.end_angle_deg.to_string(),
    },
    Kept {
        name: "g_sunFogScaleReadOnly",
        show: |map| map.readonly_sun.scale.to_string(),
    },
];

const SLIDERS: &[Slider] = &[
    Slider {
        name: "scr_art_tweak",
        usage: "scr_art_tweak [0|1] — enable local fog art sliders; other art families are not hosted",
        show: |map| u8::from(map.art_tweak).to_string(),
        edit: |map, values, _| {
            let on = flag(values)?;
            if on && !map.art_tweak {
                map.sliders = map.fog;
                map.slider_sun = map.readonly_sun;
            }
            map.art_tweak = on;
            Ok(())
        },
    },
    Slider {
        name: "scr_fog_disable",
        usage: "scr_fog_disable [value(s)] — fog control",
        show: |map| u8::from(map.script_disabled).to_string(),
        edit: |map, values, _| {
            map.script_disabled = flag(values)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_fog_nearplane",
        usage: "scr_fog_nearplane [value(s)] — fog control",
        show: |map| map.sliders.start_dist.to_string(),
        edit: |map, values, _| {
            map.sliders.start_dist = only(values, FROM_ZERO)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_fog_exp_halfplane",
        usage: "scr_fog_exp_halfplane [value(s)] — fog control",
        show: |map| map.sliders.halfway_dist.to_string(),
        edit: |map, values, _| {
            map.sliders.halfway_dist = only(values, f32::MIN_POSITIVE..=f32::MAX)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_fog_color",
        usage: "scr_fog_color [value(s)] — fog control",
        show: |map| format!("{:?}", map.sliders.color_rgb),
        edit: |map, values, _| {
            map.sliders.color_rgb = triple(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_fog_max_opacity",
        usage: "scr_fog_max_opacity [value(s)] — fog control",
        show: |map| map.sliders.max_opacity.to_string(),
        edit: |map, values, _| {
            map.sliders.max_opacity = only(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogEnabled",
        usage: "scr_sunFogEnabled [value(s)] — fog control",
        show: |map| u8::from(map.sliders.sun.is_some()).to_string(),
        edit: |map, values, _| {
            map.sliders.sun = flag(values)?.then_some(map.slider_sun);
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogColor",
        usage: "scr_sunFogColor [value(s)] — fog control",
        show: |map| format!("{:?}", map.slider_sun.color_rgb),
        edit: |map, values, _| {
            map.slider_sun.color_rgb = triple(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogDir",
        usage: "scr_sunFogDir [value(s)] — fog control",
        show: |map| format!("{:?}", map.slider_sun.sun_dir),
        edit: |map, values, _| {
            map.slider_sun.sun_dir = triple(values, ANY)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogBeginFadeAngle",
        usage: "scr_sunFogBeginFadeAngle [value(s)] — fog control",
        show: |map| map.slider_sun.begin_angle_deg.to_string(),
        edit: |map, values, _| {
            map.slider_sun.begin_angle_deg = only(values, 0.0..=map.slider_sun.end_angle_deg)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogEndFadeAngle",
        usage: "scr_sunFogEndFadeAngle [value(s)] — fog control",
        show: |map| map.slider_sun.end_angle_deg.to_string(),
        edit: |map, values, _| {
            map.slider_sun.end_angle_deg = only(values, map.slider_sun.begin_angle_deg..=180.0)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_sunFogScale",
        usage: "scr_sunFogScale [value(s)] — fog control",
        show: |map| map.slider_sun.scale.to_string(),
        edit: |map, values, _| {
            map.slider_sun.scale = only(values, FROM_ZERO)?;
            Ok(())
        },
    },
    Slider {
        name: "scr_cmd_plr_sun",
        usage: "scr_cmd_plr_sun [value(s)] — fog control",
        show: |_| "0".to_owned(),
        edit: |map, values, view| {
            if flag(values)? {
                if !view.ready {
                    return Err(NO_VIEW);
                }
                let back = view.view_from_world.inverse().z_axis.truncate();
                map.slider_sun.sun_dir = (-back).to_array();
            }
            Ok(())
        },
    },
];

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    let kept = KEPT.iter().map(|kept| {
        (
            kept.name,
            format!("{} — read-only last setexpfog input", kept.name),
        )
    });
    let sliders = SLIDERS
        .iter()
        .map(|slider| (slider.name, slider.usage.to_owned()));
    let others = [
        ("setexpfog", SETEXPFOG_USAGE.to_owned()),
        (
            "scr_fog_fraction",
            "scr_fog_fraction [value(s)] — fog control".to_owned(),
        ),
    ];
    for (name, usage) in kept.chain(sliders).chain(others) {
        registry.register(CommandSpec::new(name).usage(usage));
    }
}

fn is_fog_command(name: &str) -> bool {
    matches!(name, "setexpfog" | "scr_fog_fraction")
        || SLIDERS.iter().any(|slider| slider.name == name)
        || KEPT.iter().any(|kept| kept.name == name)
}

fn numbers(args: &[String]) -> Result<Vec<f32>, Refusal> {
    args.iter()
        .map(|arg| {
            arg.parse::<f32>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or(NOT_FINITE)
        })
        .collect()
}

fn within(value: f32, range: RangeInclusive<f32>) -> Result<f32, Refusal> {
    range.contains(&value).then_some(value).ok_or(OUTSIDE)
}

fn only(values: &[f32], range: RangeInclusive<f32>) -> Result<f32, Refusal> {
    match values {
        [value] => within(*value, range),
        _ => Err(OUTSIDE),
    }
}

fn flag(values: &[f32]) -> Result<bool, Refusal> {
    match values {
        [value] if *value == 0.0 => Ok(false),
        [value] if *value == 1.0 => Ok(true),
        _ => Err(NOT_A_FLAG),
    }
}

fn triple(values: &[f32], range: RangeInclusive<f32>) -> Result<[f32; 3], Refusal> {
    match values {
        &[x, y, z] if values.iter().all(|value| range.contains(value)) => Ok([x, y, z]),
        _ => Err(NOT_A_TRIPLE),
    }
}

/// `setexpfog`: the target fog, with a sun when there are sixteen values.
fn set_expfog(map: &mut MapFrameFog, values: &[f32], time: i32) -> Result<String, Refusal> {
    if !matches!(values.len(), 7 | 16) {
        return Err(SETEXPFOG_ARITY);
    }
    let (head, sun) = values.split_at(7);
    let &[start, half, red, green, blue, opacity, seconds] = head else {
        return Err(SETEXPFOG_ARITY);
    };
    let mut fog = map.fog;
    fog.start_dist = within(start, FROM_ZERO)?;
    fog.halfway_dist = within(half, f32::MIN_POSITIVE..=f32::MAX)?;
    fog.color_rgb = triple(&[red, green, blue], UNIT)?;
    fog.max_opacity = within(opacity, UNIT)?;
    fog.transition_time = within(seconds, 0.0..=i32::MAX as f32 / 1000.0)?;
    fog.volumetric = None;
    fog.sun = match *sun {
        [] => None,
        [red, green, blue, x, y, z, begin, end, scale] => Some(SunFog {
            color_rgb: triple(&[red, green, blue], UNIT)?,
            sun_dir: triple(&[x, y, z], ANY)?,
            begin_angle_deg: within(begin, 0.0..=180.0)?,
            end_angle_deg: within(end, begin..=180.0)?,
            scale: within(scale, FROM_ZERO)?,
        }),
        _ => return Err(SETEXPFOG_ARITY),
    };
    map.set(fog, time);
    Ok(format!(
        "setexpfog: local target start={} half={} opacity={} seconds={} sun={}",
        fog.start_dist,
        fog.halfway_dist,
        fog.max_opacity,
        fog.transition_time,
        fog.sun.is_some()
    ))
}

fn answer(
    command: &ConsoleCommand,
    map: Option<&mut MapFrameFog>,
    time: i32,
    view: &PreparedSceneView,
) -> Result<String, Refusal> {
    let name = command.name.as_str();
    let map = map.ok_or(NO_FOG)?;
    if let Some(kept) = KEPT.iter().find(|kept| kept.name == name) {
        if !command.args.is_empty() {
            return Err(READ_ONLY);
        }
        return Ok(format!("{name} = {} (read-only)", (kept.show)(map)));
    }
    if name == "scr_fog_fraction" {
        return Err(NO_CONSUMER);
    }
    let values = numbers(&command.args)?;
    if name == "setexpfog" {
        return set_expfog(map, &values, time);
    }
    let Some(slider) = SLIDERS.iter().find(|slider| slider.name == name) else {
        return Err("not a fog control");
    };
    if values.is_empty() {
        return Ok(format!("{name} = {}", (slider.show)(map)));
    }
    let mut next = *map;
    (slider.edit)(&mut next, &values, view)?;
    if next.art_tweak {
        next.apply_sliders(time);
    }
    *map = next;
    Ok(format!(
        "{name} = {values:?} (fog art tweak {})",
        map.art_tweak
    ))
}

pub(crate) fn route_fog_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut map: Option<ResMut<MapFrameFog>>,
    clock: Option<Res<FrameClock>>,
    view: Res<PreparedSceneView>,
) {
    let time = clock.map_or(0, |clock| clock.time());
    for command in commands
        .read()
        .filter(|command| is_fog_command(&command.name))
    {
        let said = answer(command, map.as_deref_mut(), time, &view)
            .unwrap_or_else(|reason| format!("{}: refused: {reason}", command.name));
        echo.write(said);
    }
}

#[cfg(test)]
mod tests {
    use asset_world::ExpFog;
    use bevy::math::Mat4;

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn map_fog() -> MapFrameFog {
        MapFrameFog::new(ExpFog {
            start_dist: 10.0,
            halfway_dist: 100.0,
            color_rgb: [0.5, 0.5, 0.5],
            max_opacity: 0.8,
            transition_time: 0.0,
            sun: None,
            volumetric: None,
        })
    }

    fn app_with(map: Option<MapFrameFog>) -> App {
        let mut app = console_app();
        app.init_resource::<PreparedSceneView>()
            .add_systems(Update, route_fog_commands);
        if let Some(map) = map {
            app.insert_resource(map);
        }
        app
    }

    fn map(app: &App) -> &MapFrameFog {
        app.world().resource::<MapFrameFog>()
    }

    #[test]
    fn without_authored_fog_every_control_says_so() {
        let mut app = app_with(None);
        feed(&mut app, "scr_fog_nearplane; setexpfog 1 2 0 0 0 1 0");
        assert_eq!(
            said(&app),
            [
                "scr_fog_nearplane: refused: fog unavailable: map has no authored fog",
                "setexpfog: refused: fog unavailable: map has no authored fog",
            ]
        );
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "r_fog 0; bot add; hitvol");
        assert_eq!(said(&app), Vec::<String>::new());
    }

    #[test]
    fn the_kept_values_read_but_do_not_set() {
        let mut app = app_with(Some(map_fog()));
        feed(
            &mut app,
            "g_fogStartDistReadOnly; g_sunFogEnabledReadOnly; g_fogColorReadOnly; g_fogStartDistReadOnly 5",
        );
        assert_eq!(
            said(&app),
            [
                "g_fogStartDistReadOnly = 10 (read-only)",
                "g_sunFogEnabledReadOnly = 0 (read-only)",
                "g_fogColorReadOnly = [0.5, 0.5, 0.5] (read-only)",
                "g_fogStartDistReadOnly: refused: read-only; use setexpfog or scr_art_tweak sliders",
            ]
        );
        assert_eq!(map(&app).fog.start_dist, 10.0);
    }

    #[test]
    fn setexpfog_takes_seven_values_or_sixteen() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "setexpfog 20 200 0.1 0.2 0.3 0.5 2");
        assert_eq!(
            said(&app).last().unwrap(),
            "setexpfog: local target start=20 half=200 opacity=0.5 seconds=2 sun=false"
        );
        assert_eq!(map(&app).fog.color_rgb, [0.1, 0.2, 0.3]);

        feed(&mut app, "setexpfog 20 200 0 0 0 1 0 1 1 1 0 0 1 10 90 2");
        assert!(said(&app).last().unwrap().ends_with("sun=true"));
        let sun = map(&app).fog.sun.unwrap();
        assert_eq!(
            (sun.sun_dir, sun.begin_angle_deg, sun.end_angle_deg),
            ([0.0, 0.0, 1.0], 10.0, 90.0)
        );
        assert_eq!(sun.scale, 2.0);

        feed(&mut app, "setexpfog 1 2 3");
        assert_eq!(
            said(&app).last().unwrap(),
            "setexpfog: refused: setexpfog expects 7 values or 16 with flattened sun direction"
        );
    }

    #[test]
    fn setexpfog_with_a_bad_value_changes_nothing() {
        let mut app = app_with(Some(map_fog()));
        for (line, why) in [
            ("setexpfog 20 0 0 0 0 1 0", OUTSIDE),
            ("setexpfog 20 200 2 0 0 1 0", NOT_A_TRIPLE),
            ("setexpfog 20 200 0 0 0 1.5 0", OUTSIDE),
            ("setexpfog 20 200 0 0 0 1 -1", OUTSIDE),
            ("setexpfog 20 200 0 0 0 1 0 1 1 1 0 0 1 90 10 1", OUTSIDE),
            ("setexpfog 20 abc 0 0 0 1 0", NOT_FINITE),
            ("setexpfog 20 inf 0 0 0 1 0", NOT_FINITE),
        ] {
            feed(&mut app, line);
            assert_eq!(
                said(&app).last().unwrap(),
                &format!("setexpfog: refused: {why}"),
                "{line}"
            );
        }
        assert_eq!(map(&app).fog.start_dist, 10.0);
        assert_eq!(map(&app).fog.halfway_dist, 100.0);
    }

    #[test]
    fn the_sliders_only_reach_the_fog_while_the_art_tweak_is_on() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_fog_nearplane 50");
        assert_eq!(
            said(&app).last().unwrap(),
            "scr_fog_nearplane = [50.0] (fog art tweak false)"
        );
        assert_eq!(map(&app).sliders.start_dist, 50.0);
        assert_eq!(map(&app).fog.start_dist, 10.0);

        feed(&mut app, "scr_art_tweak 1");
        assert!(map(&app).art_tweak);
        assert_eq!(
            map(&app).sliders.start_dist,
            10.0,
            "turning it on starts from the fog the map has"
        );

        feed(&mut app, "scr_fog_nearplane 60; scr_fog_max_opacity 0.25");
        assert_eq!(map(&app).fog.start_dist, 60.0);
        assert_eq!(map(&app).fog.max_opacity, 0.25);
    }

    #[test]
    fn a_slider_asked_without_values_reads() {
        let mut app = app_with(Some(map_fog()));
        feed(
            &mut app,
            "scr_art_tweak; scr_fog_color; scr_sunFogEnabled; scr_cmd_plr_sun; scr_sunFogEndFadeAngle",
        );
        assert_eq!(
            said(&app),
            [
                "scr_art_tweak = 0",
                "scr_fog_color = [0.5, 0.5, 0.5]",
                "scr_sunFogEnabled = 0",
                "scr_cmd_plr_sun = 0",
                "scr_sunFogEndFadeAngle = 180",
            ]
        );
    }

    #[test]
    fn a_refused_slider_value_leaves_the_fog_as_it_was() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_art_tweak 1");
        for (line, why) in [
            ("scr_fog_color 2 0 0", NOT_A_TRIPLE),
            ("scr_fog_color 0 0", NOT_A_TRIPLE),
            ("scr_fog_nearplane -1", OUTSIDE),
            ("scr_fog_nearplane 1 2", OUTSIDE),
            ("scr_fog_max_opacity 1.1", OUTSIDE),
            ("scr_fog_disable 2", NOT_A_FLAG),
            ("scr_art_tweak 0.5", NOT_A_FLAG),
        ] {
            feed(&mut app, line);
            let name = line.split(' ').next().unwrap();
            assert_eq!(
                said(&app).last().unwrap(),
                &format!("{name}: refused: {why}"),
                "{line}"
            );
        }
        assert_eq!(map(&app).sliders.color_rgb, [0.5, 0.5, 0.5]);
        assert_eq!(map(&app).sliders.start_dist, 10.0);
        assert!(map(&app).art_tweak);
        assert!(!map(&app).script_disabled);
    }

    #[test]
    fn the_sun_angles_keep_their_order() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_art_tweak 1; scr_sunFogEndFadeAngle 100");
        feed(&mut app, "scr_sunFogBeginFadeAngle 120");
        assert_eq!(
            said(&app).last().unwrap(),
            "scr_sunFogBeginFadeAngle: refused: value outside domain or wrong arity"
        );
        feed(&mut app, "scr_sunFogBeginFadeAngle 40");
        assert_eq!(map(&app).slider_sun.begin_angle_deg, 40.0);
        feed(&mut app, "scr_sunFogEndFadeAngle 30");
        assert_eq!(
            said(&app).last().unwrap(),
            "scr_sunFogEndFadeAngle: refused: value outside domain or wrong arity"
        );
        assert_eq!(map(&app).slider_sun.end_angle_deg, 100.0);
    }

    #[test]
    fn the_sun_switch_takes_the_slider_sun_with_it() {
        let mut app = app_with(Some(map_fog()));
        feed(
            &mut app,
            "scr_art_tweak 1; scr_sunFogScale 3; scr_sunFogEnabled 1",
        );
        assert_eq!(map(&app).sliders.sun.unwrap().scale, 3.0);
        assert_eq!(map(&app).fog.sun.unwrap().scale, 3.0);
        feed(&mut app, "scr_sunFogEnabled 0");
        assert!(map(&app).fog.sun.is_none());
    }

    #[test]
    fn the_sun_can_point_where_the_player_looks() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_cmd_plr_sun 1");
        assert_eq!(
            said(&app).last().unwrap(),
            "scr_cmd_plr_sun: refused: player view unavailable"
        );

        let mut view = app.world_mut().resource_mut::<PreparedSceneView>();
        view.ready = true;
        view.view_from_world = Mat4::IDENTITY;
        feed(&mut app, "scr_cmd_plr_sun 1");
        assert_eq!(map(&app).slider_sun.sun_dir, [0.0, 0.0, -1.0]);

        feed(&mut app, "scr_cmd_plr_sun 0");
        assert_eq!(
            map(&app).slider_sun.sun_dir,
            [0.0, 0.0, -1.0],
            "0 leaves it"
        );
    }

    #[test]
    fn the_fog_fraction_has_nothing_to_drive() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_fog_fraction; scr_fog_fraction 0.5");
        let said = said(&app);
        assert_eq!(said.len(), 2);
        for line in said {
            assert!(
                line.starts_with("scr_fog_fraction: refused: scr_fog_fraction is initialized to 1"),
                "{line}"
            );
        }
    }

    #[test]
    fn every_control_is_registered_with_a_usage_line() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        let lines = registry.help_lines();
        for name in SLIDERS
            .iter()
            .map(|slider| slider.name)
            .chain(KEPT.iter().map(|kept| kept.name))
            .chain(["setexpfog", "scr_fog_fraction"])
        {
            assert!(registry.resolve(name).is_some(), "{name}");
            assert!(
                lines.iter().any(|line| line.starts_with(name)),
                "{name} has no usage line"
            );
            assert!(is_fog_command(name), "{name}");
        }
    }
}
