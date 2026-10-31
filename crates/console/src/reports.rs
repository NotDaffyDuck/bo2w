//! Commands that only report: `fx_dump` asks the effects system for a log of
//! the sprites near the player, `distortion_dump` says what the distortion
//! pass found last frame, and a question about `fx_mark_profile` while it is
//! on adds the effects system's mark census to the variable's own answer.

use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use render_frontend::assemble::draw_items::WarpSettings;
use render_frontend::assemble::draw_items::FrameOutputKind;
use render_frontend::assemble::draw_items::MaterialEpoch;
use render_frontend::assemble::draw_items::FrameRenderOutputs;
use render_fx::EffectDumpRequest;
use render_fx::EffectStainTunables;
use render_fx::EffectsSystemHandle;
use render_fx::MarkSwitch;

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

/// The report commands and their usage lines.
const REPORTS: [(&str, &str); 2] = [
    ("fx_dump", "fx_dump [radius] — log nearby CodeMesh sprites (needs a live world)"),
    ("distortion_dump", "distortion_dump — current resolve request and detected materials"),
];

pub(crate) fn enlist(book: &mut OrderCatalog) {
    let missing: Vec<_> = REPORTS.iter().filter(|(name, _)| book.look_up(name).is_none()).collect();
    for (name, usage) in missing {
        book.enlist(OrderBlueprint::new(*name).help_line(*usage));
    }
}

/// The report systems, in the order they must run: the mark census follows
/// the variable answer it is added to.
pub(crate) fn routers() -> ScheduleConfigs<ScheduleSystem> {
    (route_fx_dump, route_distortion_dump, route_mark_profile).chain()
}

/// The radius `fx_dump` was given: None for the usual one, or the reply
/// refusing what was typed.
fn dump_radius(args: &[String]) -> Result<Option<f32>, String> {
    if args.len() > 1 {
        return Err("usage: fx_dump [radius]".into());
    }
    let Some(typed) = args.first() else {
        return Ok(None);
    };
    let usable = typed.parse::<f32>().ok().filter(|value| value.is_finite() && *value > 0.0);
    usable.map(Some).ok_or_else(|| format!("usage: fx_dump [radius] (got `{typed}`)"))
}

fn fx_dump(args: &[String], dump: Option<&mut EffectDumpRequest>) -> String {
    let Some(request) = dump else {
        return "fx_dump: render FX dump request missing (no RenderPlugin)".into();
    };
    dump_radius(args).map_or_else(
        |refusal| refusal,
        |asked| {
            let radius = request.queue(asked);
            format!("fx_dump: queued radius={radius:.0} (next Effects tick)")
        },
    )
}

/// Only the commands called `name`.
fn named<'a>(
    commands: &'a mut MessageReader<TypedOrder>,
    name: &'static str,
) -> impl Iterator<Item = &'a TypedOrder> {
    commands.read().filter(move |asked| asked.name == name)
}

fn route_fx_dump(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut dump: Option<ResMut<EffectDumpRequest>>,
) {
    for asked in named(&mut commands, "fx_dump") {
        let reply = fx_dump(&asked.words, dump.as_deref_mut());
        echo.write(reply);
    }
}

/// How many distortion draws use each material, by name.
fn draws_by_material<'m>(
    product: &render_frontend::assemble::draw_items::FrameOutputItem,
    materials: &'m MaterialEpoch,
) -> std::collections::BTreeMap<&'m str, usize> {
    let mut tally = std::collections::BTreeMap::new();
    let names = product.draw_sequence.iter().map(|draw| {
        let found = materials.catalog.by_sort_position(draw.material_order_no);
        found.map_or("<missing material>", |material| material.name.as_str())
    });
    for material in names {
        tally.entry(material).and_modify(|count| *count += 1).or_insert(1);
    }
    tally
}

fn route_distortion_dump(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    distortion: Res<WarpSettings>,
    frame_out: Res<FrameRenderOutputs>,
    mat_epoch: Res<MaterialEpoch>,
) {
    for asked in named(&mut commands, "distortion_dump") {
        if asked.words.is_empty() {
            let product = frame_out.product(FrameOutputKind::RefractionPass);
            let tally = draws_by_material(product, &mat_epoch);
            let enabled = u8::from(distortion.enabled);
            let (frame, status, detected) = (frame_out.frame_seq_no, &product.state, product.draw_sequence.len());
            echo.write(format!(
                "distortion: enabled={enabled} frame={frame} request={status:?} detected={detected} materials={tally:?}"
            ));
        } else {
            echo.write("usage: distortion_dump");
        }
    }
}

fn route_mark_profile(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    dvars: Res<EffectStainTunables>,
    host: Option<Res<EffectsSystemHandle>>,
) {
    // Only a bare question about the variable, and only while profiling is on.
    let Some(effects) = host.filter(|_| dvars.is_on(MarkSwitch::Profile)) else {
        return;
    };
    let questions = named(&mut commands, "fx_mark_profile").filter(|asked| asked.words.is_empty()).count();
    for _ in 0..questions {
        echo.write(effects.0.tag_timing_row());
    }
}
#[cfg(test)]
mod tests {
    use crate::reports::*;
    use crate::dvars::route_dvars;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use bevy::app::Update;

    fn app_with_dump() -> App {
        let mut app = console_app();
        app.init_resource::<EffectDumpRequest>()
            .add_systems(Update, route_fx_dump);
        app
    }

    fn dump(app: &App) -> &EffectDumpRequest {
        app.world().resource::<EffectDumpRequest>()
    }

    #[test]
    fn fx_dump_queues_a_dump_at_the_usual_radius_or_the_one_given() {
        let mut app = app_with_dump();
        let usual = dump(&app).reach();
        feed(&mut app, "fx_dump");
        let expected = [format!(
            "fx_dump: queued radius={usual:.0} (next Effects tick)"
        )];
        assert_eq!(said(&app), expected);
        assert!(dump(&app).is_queued());

        let mut app = app_with_dump();
        feed(&mut app, "fx_dump 128.4");
        let expected = ["fx_dump: queued radius=128 (next Effects tick)"];
        assert_eq!(said(&app), expected);
        assert_eq!((dump(&app).is_queued(), dump(&app).reach()), (true, 128.4));
    }

    #[test]
    fn fx_dump_with_a_bad_radius_queues_nothing_and_keeps_the_radius() {
        let mut app = app_with_dump();
        let usual = dump(&app).reach();
        feed(&mut app, "fx_dump near; fx_dump 0; fx_dump -5; fx_dump inf");
        let expected = [
            "usage: fx_dump [radius] (got `near`)",
            "usage: fx_dump [radius] (got `0`)",
            "usage: fx_dump [radius] (got `-5`)",
            "usage: fx_dump [radius] (got `inf`)",
        ];
        assert_eq!(said(&app), expected);
        feed(&mut app, "fx_dump 1 2");
        assert_eq!(said(&app).last().unwrap(), "usage: fx_dump [radius]");
        assert!(!dump(&app).is_queued());
        assert_eq!(dump(&app).reach(), usual);
    }

    #[test]
    fn fx_dump_without_the_render_plugin_says_so() {
        let mut app = console_app();
        app.add_systems(Update, route_fx_dump);
        feed(&mut app, "fx_dump");
        let actual = said(&app);
        assert_eq!(actual, ["fx_dump: render FX dump request missing (no RenderPlugin)"]);
    }

    fn app_with_distortion() -> App {
        let mut app = console_app();
        app.init_resource::<WarpSettings>()
            .init_resource::<FrameRenderOutputs>()
            .init_resource::<MaterialEpoch>()
            .add_systems(Update, route_distortion_dump);
        app
    }

    #[test]
    fn the_distortion_dump_names_the_switch_the_frame_and_what_was_found() {
        let mut app = app_with_distortion();
        feed(&mut app, "distortion_dump");
        let line = said(&app).remove(0);
        assert!(
            line.starts_with("distortion: enabled=1 frame=0 request="),
            "{line}"
        );
        assert!(line.ends_with(" detected=0 materials={}"), "{line}");

        app.world_mut().resource_mut::<WarpSettings>().enabled = false;
        feed(&mut app, "distortion_dump");
        assert!(
            said(&app)[1].starts_with("distortion: enabled=0 "),
            "{:?}",
            said(&app)
        );
    }

    #[test]
    fn the_distortion_dump_takes_no_arguments() {
        let mut app = app_with_distortion();
        feed(&mut app, "distortion_dump now");
        assert_eq!(said(&app), ["usage: distortion_dump"]);
    }

    fn app_with_marks(profile_on: bool, host: bool) -> App {
        let mut app = console_app();
        app.insert_resource(EffectStainTunables::default().with(MarkSwitch::Profile, profile_on))
        .add_systems(
            Update,
            (route_dvars::<EffectStainTunables>, route_mark_profile).chain(),
        );
        if host {
            app.init_resource::<EffectsSystemHandle>();
        }
        app
    }

    #[test]
    fn asking_for_the_mark_profile_while_it_is_on_adds_the_census() {
        let mut app = app_with_marks(true, true);
        feed(&mut app, "fx_mark_profile");
        let said = said(&app);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].starts_with("fx_mark_profile = 1"), "{said:?}");
        assert!(said[1].starts_with("fx_mark_profile live="), "{said:?}");
    }

    #[test]
    fn the_census_is_only_for_a_question_while_it_is_on() {
        for (on, host, line) in [
            (false, true, "fx_mark_profile"),
            (true, false, "fx_mark_profile"),
            (true, true, "fx_mark_profile 0"),
            (true, true, "fx_marks"),
        ] {
            let mut app = app_with_marks(on, host);
            feed(&mut app, line);
            let said = said(&app);
            assert_eq!(said.len(), 1, "{line} on={on} host={host}: {said:?}");
            assert!(!said[0].contains("live="), "{said:?}");
        }
    }

    #[test]
    fn both_dumps_are_registered_with_a_usage_line() {
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        let lines = book.usage_listing();
        for name in ["fx_dump", "distortion_dump"] {
            assert!(book.look_up(name).is_some(), "{name}");
            let expected = 1;
            assert_eq!(lines.iter().filter(|line| line.starts_with(name)).count(), expected, "{name}");
        }
    }
}
