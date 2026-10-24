//! Commands that only report: `fx_dump` asks the effects system for a log of
//! the sprites near the player, `distortion_dump` says what the distortion
//! pass found last frame, and a question about `fx_mark_profile` while it is
//! on adds the effects system's mark census to the variable's own answer.

use bevy::{
    ecs::{schedule::ScheduleConfigs, system::ScheduleSystem},
    prelude::*,
};
use render_frontend::assemble::drawsurf::{
    DistortionSettings, FrameProductKind, MaterialGeneration, RenderFrameProducts,
};
use render_fx::{FxDumpRequest, FxMarkDvars, HostFxSystem};

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    for (name, usage) in [
        (
            "fx_dump",
            "fx_dump [radius] — log nearby CodeMesh sprites (needs a live world)",
        ),
        (
            "distortion_dump",
            "distortion_dump — current resolve request and detected materials",
        ),
    ] {
        if registry.resolve(name).is_none() {
            registry.register(CommandSpec::new(name).usage(usage));
        }
    }
}

/// The report systems, in the order they must run: the mark census follows
/// the variable answer it is added to.
pub(crate) fn routers() -> ScheduleConfigs<ScheduleSystem> {
    (route_fx_dump, route_distortion_dump, route_mark_profile).chain()
}

fn fx_dump(args: &[String], dump: Option<&mut FxDumpRequest>) -> String {
    let Some(dump) = dump else {
        return "fx_dump: render FX dump request missing (no RenderPlugin)".into();
    };
    match args {
        [] => {}
        [radius] => match radius.parse::<f32>() {
            Ok(radius) if radius.is_finite() && radius > 0.0 => dump.radius = radius,
            _ => return format!("usage: fx_dump [radius] (got `{radius}`)"),
        },
        _ => return "usage: fx_dump [radius]".into(),
    }
    dump.pending = true;
    format!(
        "fx_dump: queued radius={:.0} (next Effects tick)",
        dump.radius
    )
}

fn route_fx_dump(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut dump: Option<ResMut<FxDumpRequest>>,
) {
    for command in commands.read().filter(|command| command.name == "fx_dump") {
        echo.write(fx_dump(&command.args, dump.as_deref_mut()));
    }
}

fn route_distortion_dump(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    distortion: Res<DistortionSettings>,
    products: Res<RenderFrameProducts>,
    materials: Res<MaterialGeneration>,
) {
    for command in commands
        .read()
        .filter(|command| command.name == "distortion_dump")
    {
        if !command.args.is_empty() {
            echo.write("usage: distortion_dump");
            continue;
        }
        let product = products.product(FrameProductKind::Distortion);
        let mut by_material = std::collections::BTreeMap::new();
        for draw in &product.ordered_draws {
            let name = materials
                .catalog
                .material_for_sorted_ordinal(draw.material_rank)
                .map_or("<missing material>", |material| material.name.as_str());
            *by_material.entry(name).or_insert(0usize) += 1;
        }
        echo.write(format!(
            "distortion: enabled={} frame={} request={:?} detected={} materials={by_material:?}",
            u8::from(distortion.enabled),
            products.frame_id,
            product.status,
            product.ordered_draws.len(),
        ));
    }
}

fn route_mark_profile(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    dvars: Res<FxMarkDvars>,
    host: Option<Res<HostFxSystem>>,
) {
    let (true, Some(host)) = (dvars.fx_mark_profile, host) else {
        return;
    };
    for _ in commands
        .read()
        .filter(|command| command.name == "fx_mark_profile" && command.args.is_empty())
    {
        echo.write(host.0.mark_profile_line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvars::route_dvars;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn app_with_dump() -> App {
        let mut app = console_app();
        app.init_resource::<FxDumpRequest>()
            .add_systems(Update, route_fx_dump);
        app
    }

    fn dump(app: &App) -> &FxDumpRequest {
        app.world().resource::<FxDumpRequest>()
    }

    #[test]
    fn fx_dump_queues_a_dump_at_the_usual_radius_or_the_one_given() {
        let mut app = app_with_dump();
        let usual = dump(&app).radius;
        feed(&mut app, "fx_dump");
        assert_eq!(
            said(&app),
            [format!(
                "fx_dump: queued radius={usual:.0} (next Effects tick)"
            )]
        );
        assert!(dump(&app).pending);

        let mut app = app_with_dump();
        feed(&mut app, "fx_dump 128.4");
        assert_eq!(
            said(&app),
            ["fx_dump: queued radius=128 (next Effects tick)"]
        );
        assert_eq!((dump(&app).pending, dump(&app).radius), (true, 128.4));
    }

    #[test]
    fn fx_dump_with_a_bad_radius_queues_nothing_and_keeps_the_radius() {
        let mut app = app_with_dump();
        let usual = dump(&app).radius;
        feed(&mut app, "fx_dump near; fx_dump 0; fx_dump -5; fx_dump inf");
        assert_eq!(
            said(&app),
            [
                "usage: fx_dump [radius] (got `near`)",
                "usage: fx_dump [radius] (got `0`)",
                "usage: fx_dump [radius] (got `-5`)",
                "usage: fx_dump [radius] (got `inf`)",
            ]
        );
        feed(&mut app, "fx_dump 1 2");
        assert_eq!(said(&app).last().unwrap(), "usage: fx_dump [radius]");
        assert!(!dump(&app).pending);
        assert_eq!(dump(&app).radius, usual);
    }

    #[test]
    fn fx_dump_without_the_render_plugin_says_so() {
        let mut app = console_app();
        app.add_systems(Update, route_fx_dump);
        feed(&mut app, "fx_dump");
        assert_eq!(
            said(&app),
            ["fx_dump: render FX dump request missing (no RenderPlugin)"]
        );
    }

    fn app_with_distortion() -> App {
        let mut app = console_app();
        app.init_resource::<DistortionSettings>()
            .init_resource::<RenderFrameProducts>()
            .init_resource::<MaterialGeneration>()
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

        app.world_mut().resource_mut::<DistortionSettings>().enabled = false;
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
        app.insert_resource(FxMarkDvars {
            fx_mark_profile: profile_on,
            ..FxMarkDvars::default()
        })
        .add_systems(
            Update,
            (route_dvars::<FxMarkDvars>, route_mark_profile).chain(),
        );
        if host {
            app.init_resource::<HostFxSystem>();
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
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        register(&mut registry);
        let lines = registry.help_lines();
        for name in ["fx_dump", "distortion_dump"] {
            assert!(registry.resolve(name).is_some(), "{name}");
            assert_eq!(
                lines.iter().filter(|line| line.starts_with(name)).count(),
                1,
                "{name}"
            );
        }
    }
}
