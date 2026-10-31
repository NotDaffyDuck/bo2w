//! Fog from the console: `setexpfog`, the art sliders and the read-only
//! values the art script leaves behind. Every edit is made on a copy of the
//! map's fog and kept only when all of its values were accepted. The two
//! engine variables, `r_fog` and `r_zfar`, are in `dvars`.

use std::fmt::{Display, Error, Formatter};
use std::ops::RangeInclusive;

use asset_world::SunlitMist;
use bevy::{ecs::message::MessageReader, ecs::system::Res, ecs::system::ResMut};
use net::FrameClock;
use render_frontend::assemble::draw_items::fog::MapFrameHaze;
use render_frontend::prepare::scene::eye_parms::StagedStageEye;

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

/// One fog command, with the clock and the eye it is judged against.
struct FogAsk<'a> {
    order: &'a TypedOrder,
    time: i32,
    view: &'a StagedStageEye,
}

/// Why a fog command was turned down; its words follow "refused:" in the reply.
enum FogRefusal {
    NoFog,
    NotFinite,
    Outside,
    NotAFlag,
    NotATriple,
    ReadOnly,
    NoConsumer,
    NoView,
    SetexpfogArity,
    UnknownControl,
}

impl FogRefusal {
    fn words(&self) -> &'static str {
        match self {
            FogRefusal::NoFog => "fog unavailable: map has no authored fog",
            FogRefusal::NotFinite => "expected finite numeric values",
            FogRefusal::Outside => "value outside domain or wrong arity",
            FogRefusal::NotAFlag => "expected 0 or 1",
            FogRefusal::NotATriple => "expected three values in domain",
            FogRefusal::ReadOnly => "read-only; use setexpfog or scr_art_tweak sliders",
            FogRefusal::NoConsumer => {
                "scr_fog_fraction is initialized to 1 by the art script but has no fog consumer"
            }
            FogRefusal::NoView => "player view unavailable",
            FogRefusal::SetexpfogArity => "setexpfog expects 7 values or 16 with flattened sun direction",
            FogRefusal::UnknownControl => "not a fog control",
        }
    }
}

impl Display for FogRefusal {
    fn fmt(&self, out: &mut Formatter<'_>) -> Result<(), Error> {
        out.write_str(self.words())
    }
}

const SETEXPFOG_USAGE: &str = "setexpfog start half R G B maxOpacity seconds [sunR sunG sunB dirX dirY dirZ beginAngle endAngle scale] — local script bridge";

const FRACTION_USAGE: &str = "scr_fog_fraction [value(s)] — fog control";

const ANY: RangeInclusive<f32> = -f32::MAX..=f32::MAX;
const FROM_ZERO: RangeInclusive<f32> = 0.0..=f32::MAX;
const UNIT: RangeInclusive<f32> = 0.0..=1.0;

/// A value the art script wrote last, shown but never set from here.
struct Kept {
    /// The variable as typed.
    command: &'static str,
    show: fn(&MapFrameHaze) -> String,
}

/// A slider of the fog art tweak: how it reads, and what it does to a copy
/// of the map's fog when it is given values.
struct Slider {
    command: &'static str,
    usage: &'static str,
    show: fn(&MapFrameHaze) -> String,
    edit: fn(&mut MapFrameHaze, &[f32], &StagedStageEye) -> Result<(), FogRefusal>,
}

const KEPT: &[Kept] = &[
    Kept {
        command: "g_fogColorReadOnly",
        show: |map| format!("{:?}", map.fog.tint_rgb),
    },
    Kept {
        command: "g_fogStartDistReadOnly",
        show: |map| map.fog.near_edge.to_string(),
    },
    Kept {
        command: "g_fogHalfDistReadOnly",
        show: |map| map.fog.half_density_at.to_string(),
    },
    Kept {
        command: "g_fogMaxOpacityReadOnly",
        show: |map| map.fog.opacity_cap.to_string(),
    },
    Kept {
        command: "g_sunFogEnabledReadOnly",
        show: |map| u8::from(map.fog.sun.is_some()).to_string(),
    },
    Kept {
        command: "g_sunFogColorReadOnly",
        show: |map| format!("{:?}", map.readonly_daylight.tint_rgb),
    },
    Kept {
        command: "g_sunFogDirReadOnly",
        show: |map| format!("{:?}", map.readonly_daylight.toward_sun),
    },
    Kept {
        command: "g_sunFogBeginFadeAngleReadOnly",
        show: |map| map.readonly_daylight.inner_cone_deg.to_string(),
    },
    Kept {
        command: "g_sunFogEndFadeAngleReadOnly",
        show: |map| map.readonly_daylight.outer_cone_deg.to_string(),
    },
    Kept {
        command: "g_sunFogScaleReadOnly",
        show: |map| map.readonly_daylight.scale.to_string(),
    },
];

const SLIDERS: &[Slider] = &[
    Slider {
        command: "scr_art_tweak",
        usage: "scr_art_tweak [0|1] — enable local fog art sliders; other art families are not hosted",
        show: |map| u8::from(map.art_trim).to_string(),
        edit: |map, values, _| {
            let on = flag(values)?;
            if on && !map.art_trim {
                map.sliders = map.fog;
                map.slider_daylight = map.readonly_daylight;
            }
            map.art_trim = on;
            Ok(())
        },
    },
    Slider {
        command: "scr_fog_disable",
        usage: "scr_fog_disable [value(s)] — fog control",
        show: |map| u8::from(map.scripts_off).to_string(),
        edit: |map, values, _| {
            map.scripts_off = flag(values)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_fog_nearplane",
        usage: "scr_fog_nearplane [value(s)] — fog control",
        show: |map| map.sliders.near_edge.to_string(),
        edit: |map, values, _| {
            map.sliders.near_edge = only(values, FROM_ZERO)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_fog_exp_halfplane",
        usage: "scr_fog_exp_halfplane [value(s)] — fog control",
        show: |map| map.sliders.half_density_at.to_string(),
        edit: |map, values, _| {
            map.sliders.half_density_at = only(values, f32::MIN_POSITIVE..=f32::MAX)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_fog_color",
        usage: "scr_fog_color [value(s)] — fog control",
        show: |map| format!("{:?}", map.sliders.tint_rgb),
        edit: |map, values, _| {
            map.sliders.tint_rgb = triple(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_fog_max_opacity",
        usage: "scr_fog_max_opacity [value(s)] — fog control",
        show: |map| map.sliders.opacity_cap.to_string(),
        edit: |map, values, _| {
            map.sliders.opacity_cap = only(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogEnabled",
        usage: "scr_sunFogEnabled [value(s)] — fog control",
        show: |map| u8::from(map.sliders.sun.is_some()).to_string(),
        edit: |map, values, _| {
            map.sliders.sun = flag(values)?.then_some(map.slider_daylight);
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogColor",
        usage: "scr_sunFogColor [value(s)] — fog control",
        show: |map| format!("{:?}", map.slider_daylight.tint_rgb),
        edit: |map, values, _| {
            map.slider_daylight.tint_rgb = triple(values, UNIT)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogDir",
        usage: "scr_sunFogDir [value(s)] — fog control",
        show: |map| format!("{:?}", map.slider_daylight.toward_sun),
        edit: |map, values, _| {
            map.slider_daylight.toward_sun = triple(values, ANY)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogBeginFadeAngle",
        usage: "scr_sunFogBeginFadeAngle [value(s)] — fog control",
        show: |map| map.slider_daylight.inner_cone_deg.to_string(),
        edit: |map, values, _| {
            map.slider_daylight.inner_cone_deg = only(values, 0.0..=map.slider_daylight.outer_cone_deg)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogEndFadeAngle",
        usage: "scr_sunFogEndFadeAngle [value(s)] — fog control",
        show: |map| map.slider_daylight.outer_cone_deg.to_string(),
        edit: |map, values, _| {
            map.slider_daylight.outer_cone_deg = only(values, map.slider_daylight.inner_cone_deg..=180.0)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_sunFogScale",
        usage: "scr_sunFogScale [value(s)] — fog control",
        show: |map| map.slider_daylight.scale.to_string(),
        edit: |map, values, _| {
            map.slider_daylight.scale = only(values, FROM_ZERO)?;
            Ok(())
        },
    },
    Slider {
        command: "scr_cmd_plr_sun",
        usage: "scr_cmd_plr_sun [value(s)] — fog control",
        show: |_| "0".to_owned(),
        edit: |map, values, view| {
            if flag(values)? {
                if !view.primed {
                    return Err(FogRefusal::NoView);
                }
                let back = view.eye_from_map.inverse().z_axis.truncate();
                map.slider_daylight.toward_sun = (-back).to_array();
            }
            Ok(())
        },
    },
];

pub(crate) fn enlist(book: &mut OrderCatalog) {
    let kept = KEPT.iter().map(|kept| {
        (
            kept.command,
            format!("{} — read-only last setexpfog input", kept.command),
        )
    });
    let sliders = SLIDERS
        .iter()
        .map(|slider| (slider.command, slider.usage.to_owned()));
    let others = [("setexpfog", SETEXPFOG_USAGE), ("scr_fog_fraction", FRACTION_USAGE)]
        .map(|(command, usage)| (command, usage.to_owned()));
    for (name, usage) in kept.chain(sliders).chain(others) {
        book.enlist(OrderBlueprint::new(name).help_line(usage));
    }
}

fn is_fog_command(name: &str) -> bool {
    matches!(name, "setexpfog" | "scr_fog_fraction")
        || SLIDERS.iter().any(|slider| slider.command == name)
        || KEPT.iter().any(|kept| kept.command == name)
}

fn numbers(words: &[String]) -> Result<Vec<f32>, FogRefusal> {
    words.iter()
        .map(|arg| {
            arg.parse::<f32>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or(FogRefusal::NotFinite)
        })
        .collect()
}

fn within(value: f32, range: RangeInclusive<f32>) -> Result<f32, FogRefusal> {
    range.contains(&value).then_some(value).ok_or(FogRefusal::Outside)
}

fn only(values: &[f32], range: RangeInclusive<f32>) -> Result<f32, FogRefusal> {
    match values {
        [value] => within(*value, range),
        _ => Err(FogRefusal::Outside),
    }
}

fn flag(values: &[f32]) -> Result<bool, FogRefusal> {
    match values {
        [value] if *value == 0.0 => Ok(false),
        [value] if *value == 1.0 => Ok(true),
        _ => Err(FogRefusal::NotAFlag),
    }
}

fn triple(values: &[f32], range: RangeInclusive<f32>) -> Result<[f32; 3], FogRefusal> {
    match values {
        &[x, y, z] if values.iter().all(|value| range.contains(value)) => Ok([x, y, z]),
        _ => Err(FogRefusal::NotATriple),
    }
}

/// `setexpfog`: the target fog, with a sun when there are sixteen values.
fn set_expfog(map: &mut MapFrameHaze, values: &[f32], time: i32) -> Result<String, FogRefusal> {
    if !matches!(values.len(), 7 | 16) {
        return Err(FogRefusal::SetexpfogArity);
    }
    let (head, sun) = values.split_at(7);
    let &[start, half, red, green, blue, opacity, seconds] = head else {
        return Err(FogRefusal::SetexpfogArity);
    };
    let mut fog = map.fog;
    fog.near_edge = within(start, FROM_ZERO)?;
    fog.half_density_at = within(half, f32::MIN_POSITIVE..=f32::MAX)?;
    fog.tint_rgb = triple(&[red, green, blue], UNIT)?;
    fog.opacity_cap = within(opacity, UNIT)?;
    fog.fade_duration = within(seconds, 0.0..=i32::MAX as f32 / 1000.0)?;
    fog.volumetric.take();
    fog.sun = match *sun {
        [] => None,
        [red, green, blue, x, y, z, begin, end, scale] => Some(SunlitMist {
            tint_rgb: triple(&[red, green, blue], UNIT)?,
            toward_sun: triple(&[x, y, z], ANY)?,
            inner_cone_deg: within(begin, 0.0..=180.0)?,
            outer_cone_deg: within(end, begin..=180.0)?,
            scale: within(scale, FROM_ZERO)?,
        }),
        _ => return Err(FogRefusal::SetexpfogArity),
    };
    map.set(fog, time);
    let has_sun = fog.sun.is_some();
    let (near, half_at) = (fog.near_edge, fog.half_density_at);
    let (opaque, fade) = (fog.opacity_cap, fog.fade_duration);
    Ok(format!("setexpfog: local target start={near} half={half_at} opacity={opaque} seconds={fade} sun={has_sun}"))
}

/// What a fog command prints, or why it refused.
fn answer(ask: &FogAsk, map: Option<&mut MapFrameHaze>) -> Result<String, FogRefusal> {
    let name = ask.order.name.as_str();
    let map = map.ok_or(FogRefusal::NoFog)?;
    if let Some(kept) = KEPT.iter().find(|kept| kept.command == name) {
        if !ask.order.words.is_empty() {
            return Err(FogRefusal::ReadOnly);
        }
        return Ok(format!("{name} = {} (read-only)", (kept.show)(map)));
    }
    match name {
        "scr_fog_fraction" => Err(FogRefusal::NoConsumer),
        "setexpfog" => set_expfog(map, &numbers(&ask.order.words)?, ask.time),
        _ => move_slider(map, &numbers(&ask.order.words)?, ask),
    }
}

/// Shows a slider, or edits a copy of the fog with it and keeps the copy
/// only if the edit was accepted. While the art tweak is on, the sliders
/// drive the live fog straight away.
fn move_slider(map: &mut MapFrameHaze, values: &[f32], ask: &FogAsk) -> Result<String, FogRefusal> {
    let name = ask.order.name.as_str();
    let slider = SLIDERS
        .iter()
        .find(|slider| slider.command == name)
        .ok_or(FogRefusal::UnknownControl)?;
    if values.is_empty() {
        return Ok(format!("{name} = {}", (slider.show)(map)));
    }
    let mut draft = *map;
    (slider.edit)(&mut draft, values, ask.view)?;
    let art_tweak = draft.art_trim;
    if art_tweak {
        draft.pull_tunables(ask.time);
    }
    *map = draft;
    Ok(format!("{name} = {values:?} (fog art tweak {art_tweak})"))
}

pub(crate) fn route_fog_commands(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut map: Option<ResMut<MapFrameHaze>>,
    clock: Option<Res<FrameClock>>,
    view: Res<StagedStageEye>,
) {
    let time = clock.map_or(0, |clock| clock.time());
    for command in commands
        .read()
        .filter(|command| is_fog_command(&command.name))
    {
        let ask = FogAsk { order: command, time, view: &view };
        let said = answer(&ask, Option::as_deref_mut(&mut map))
            .unwrap_or_else(|why| format!("{}: refused: {why}", command.name));
        echo.write(said);
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use asset_world::ExpHaze;
    use bevy::prelude::*;

    use crate::fog::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use bevy::app::Update;

    const GREY: [f32; 3] = [0.5; 3];

    fn map_fog() -> MapFrameHaze {
        MapFrameHaze::new(ExpHaze {
            near_edge: 10.0,
            half_density_at: 100.0,
            tint_rgb: GREY,
            opacity_cap: 0.8,
            fade_duration: 0.0,
            sun: None,
            volumetric: None,
        })
    }

    fn app_with(map: Option<MapFrameHaze>) -> App {
        let mut app = console_app();
        app.init_resource::<StagedStageEye>()
            .add_systems(Update, route_fog_commands);
        if let Some(map) = map {
            app.insert_resource(map);
        }
        app
    }

    fn map(app: &App) -> &MapFrameHaze {
        app.world().resource::<MapFrameHaze>()
    }

    #[test]
    fn without_authored_fog_every_control_says_so() {
        let mut app = app_with(None);
        feed(&mut app, "scr_fog_nearplane; setexpfog 1 2 0 0 0 1 0");
        let want = [
            "scr_fog_nearplane: refused: fog unavailable: map has no authored fog",
            "setexpfog: refused: fog unavailable: map has no authored fog",
        ];
        assert_eq!(said(&app), want);
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
        let want = [
            "g_fogStartDistReadOnly = 10 (read-only)",
            "g_sunFogEnabledReadOnly = 0 (read-only)",
            "g_fogColorReadOnly = [0.5, 0.5, 0.5] (read-only)",
            "g_fogStartDistReadOnly: refused: read-only; use setexpfog or scr_art_tweak sliders",
        ];
        assert_eq!(said(&app), want);
        assert_eq!(map(&app).fog.near_edge, 10.0);
    }

    #[test]
    fn setexpfog_takes_seven_values_or_sixteen() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "setexpfog 20 200 0.1 0.2 0.3 0.5 2");
        let want = "setexpfog: local target start=20 half=200 opacity=0.5 seconds=2 sun=false";
        assert_eq!(said(&app).last().unwrap(), want);
        assert_eq!(map(&app).fog.tint_rgb, [0.1, 0.2, 0.3]);

        feed(&mut app, "setexpfog 20 200 0 0 0 1 0 1 1 1 0 0 1 10 90 2");
        assert!(said(&app).last().unwrap().ends_with("sun=true"));
        let mist = map(&app).fog.sun.unwrap();
        let want = ([0.0, 0.0, 1.0], 10.0, 90.0);
        assert_eq!((mist.toward_sun, mist.inner_cone_deg, mist.outer_cone_deg), want);
        assert_eq!(mist.scale, 2.0);

        feed(&mut app, "setexpfog 1 2 3");
        let want = "setexpfog: refused: setexpfog expects 7 values or 16 with flattened sun direction";
        assert_eq!(said(&app).last().unwrap(), want);
    }

    #[test]
    fn setexpfog_with_a_bad_value_changes_nothing() {
        let mut app = app_with(Some(map_fog()));
        for (line, why) in [
            ("setexpfog 20 0 0 0 0 1 0", FogRefusal::Outside),
            ("setexpfog 20 200 2 0 0 1 0", FogRefusal::NotATriple),
            ("setexpfog 20 200 0 0 0 1.5 0", FogRefusal::Outside),
            ("setexpfog 20 200 0 0 0 1 -1", FogRefusal::Outside),
            ("setexpfog 20 200 0 0 0 1 0 1 1 1 0 0 1 90 10 1", FogRefusal::Outside),
            ("setexpfog 20 abc 0 0 0 1 0", FogRefusal::NotFinite),
            ("setexpfog 20 inf 0 0 0 1 0", FogRefusal::NotFinite),
        ] {
            feed(&mut app, line);
            let want = &format!("setexpfog: refused: {why}");
            assert_eq!(said(&app).last().unwrap(), want, "{line}");
        }
        assert_eq!(map(&app).fog.near_edge, 10.0);
        assert_eq!(map(&app).fog.half_density_at, 100.0);
    }

    #[test]
    fn the_sliders_only_reach_the_fog_while_the_art_tweak_is_on() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_fog_nearplane 50");
        let want = "scr_fog_nearplane = [50.0] (fog art tweak false)";
        assert_eq!(said(&app).last().unwrap(), want);
        assert_eq!(map(&app).sliders.near_edge, 50.0);
        assert_eq!(map(&app).fog.near_edge, 10.0);

        feed(&mut app, "scr_art_tweak 1");
        assert!(map(&app).art_trim);
        let want = 10.0;
        assert_eq!(map(&app).sliders.near_edge, want, "turning it on starts from the fog the map has");

        feed(&mut app, "scr_fog_nearplane 60; scr_fog_max_opacity 0.25");
        assert_eq!(map(&app).fog.near_edge, 60.0);
        assert_eq!(map(&app).fog.opacity_cap, 0.25);
    }

    #[test]
    fn a_slider_asked_without_values_reads() {
        let mut app = app_with(Some(map_fog()));
        feed(
            &mut app,
            "scr_art_tweak; scr_fog_color; scr_sunFogEnabled; scr_cmd_plr_sun; scr_sunFogEndFadeAngle",
        );
        let want = [
            "scr_art_tweak = 0",
            "scr_fog_color = [0.5, 0.5, 0.5]",
            "scr_sunFogEnabled = 0",
            "scr_cmd_plr_sun = 0",
            "scr_sunFogEndFadeAngle = 180",
        ];
        assert_eq!(said(&app), want);
    }

    #[test]
    fn a_refused_slider_value_leaves_the_fog_as_it_was() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_art_tweak 1");
        for (line, why) in [
            ("scr_fog_color 2 0 0", FogRefusal::NotATriple),
            ("scr_fog_color 0 0", FogRefusal::NotATriple),
            ("scr_fog_nearplane -1", FogRefusal::Outside),
            ("scr_fog_nearplane 1 2", FogRefusal::Outside),
            ("scr_fog_max_opacity 1.1", FogRefusal::Outside),
            ("scr_fog_disable 2", FogRefusal::NotAFlag),
            ("scr_art_tweak 0.5", FogRefusal::NotAFlag),
        ] {
            feed(&mut app, line);
            let name = line.split(' ').next().unwrap();
            let want = &format!("{name}: refused: {why}");
            assert_eq!(said(&app).last().unwrap(), want, "{line}");
        }
        assert_eq!(map(&app).sliders.tint_rgb, [0.5, 0.5, 0.5]);
        assert_eq!(map(&app).sliders.near_edge, 10.0);
        assert!(map(&app).art_trim);
        assert!(!map(&app).scripts_off);
    }

    #[test]
    fn the_sun_angles_keep_their_order() {
        let mut app = app_with(Some(map_fog()));
        feed(&mut app, "scr_art_tweak 1; scr_sunFogEndFadeAngle 100");
        feed(&mut app, "scr_sunFogBeginFadeAngle 120");
        let want = "scr_sunFogBeginFadeAngle: refused: value outside domain or wrong arity";
        assert_eq!(said(&app).last().unwrap(), want);
        feed(&mut app, "scr_sunFogBeginFadeAngle 40");
        assert_eq!(map(&app).slider_daylight.inner_cone_deg, 40.0);
        feed(&mut app, "scr_sunFogEndFadeAngle 30");
        let want = "scr_sunFogEndFadeAngle: refused: value outside domain or wrong arity";
        assert_eq!(said(&app).last().unwrap(), want);
        assert_eq!(map(&app).slider_daylight.outer_cone_deg, 100.0);
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
        let want = "scr_cmd_plr_sun: refused: player view unavailable";
        assert_eq!(said(&app).last().unwrap(), want);

        let mut view = app.world_mut().resource_mut::<StagedStageEye>();
        view.primed |= true;
        view.eye_from_map = Mat4::IDENTITY;
        feed(&mut app, "scr_cmd_plr_sun 1");
        assert_eq!(map(&app).slider_daylight.toward_sun, [0.0, 0.0, -1.0]);

        feed(&mut app, "scr_cmd_plr_sun 0");
        let want = [0.0, 0.0, -1.0];
        assert_eq!(map(&app).slider_daylight.toward_sun, want, "0 leaves it");
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
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        let lines = book.usage_listing();
        for name in SLIDERS
            .iter()
            .map(|slider| slider.command)
            .chain(KEPT.iter().map(|kept| kept.command))
            .chain(["setexpfog", "scr_fog_fraction"])
        {
            assert!(book.look_up(name).is_some(), "{name}");
            assert!(
                lines.iter().any(|line| line.starts_with(name)),
                "{name} has no usage line"
            );
            assert!(is_fog_command(name), "{name}");
        }
    }
}
