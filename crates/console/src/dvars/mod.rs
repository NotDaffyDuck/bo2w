//! Console variables that are one number in a resource. A table per resource
//! names each variable, how it reads, how it sets and what values it takes;
//! one router answers them all: no argument asks, one argument sets.

use std::{fmt, ops::DerefMut};

use bevy::{
    ecs::{component::Mutable, schedule::ScheduleConfigs, system::ScheduleSystem},
    prelude::*,
};

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

mod sets;

/// What a variable holds right now. Some are not read until the renderer
/// first asks for them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Reading {
    Unread,
    Value(f32),
}

impl From<f32> for Reading {
    fn from(value: f32) -> Self {
        Self::Value(value)
    }
}

impl From<bool> for Reading {
    fn from(on: bool) -> Self {
        Self::Value(f32::from(u8::from(on)))
    }
}

impl From<Option<f32>> for Reading {
    fn from(value: Option<f32>) -> Self {
        value.map_or(Self::Unread, Self::Value)
    }
}

impl From<Option<bool>> for Reading {
    fn from(on: Option<bool>) -> Self {
        on.map_or(Self::Unread, Self::from)
    }
}

impl fmt::Display for Reading {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unread => out.write_str("unread"),
            Self::Value(value) => write!(out, "{value}"),
        }
    }
}

/// The values a variable takes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    /// On or off: `0`, `1`, `on`, `off`, `true` or `false`.
    Toggle,
    /// A finite number from the first to the second, both included.
    Range(f32, f32),
    /// A finite number no smaller than this.
    AtLeast(f32),
    /// Any finite number, in this unit.
    Free(&'static str),
}

impl Kind {
    fn parse(self, text: &str) -> Option<f32> {
        if let Self::Toggle = self {
            return match text {
                "1" | "on" | "true" => Some(1.0),
                "0" | "off" | "false" => Some(0.0),
                _ => None,
            };
        }
        let value = text.parse::<f32>().ok().filter(|value| value.is_finite())?;
        let (low, high) = match self {
            Self::Range(low, high) => (low, high),
            Self::AtLeast(low) => (low, f32::MAX),
            Self::Toggle | Self::Free(_) => (f32::MIN, f32::MAX),
        };
        (low..=high).contains(&value).then_some(value)
    }

    /// What the usage line shows between the brackets.
    fn argument(self) -> String {
        match self {
            Self::Toggle => "0|1".to_owned(),
            Self::Range(low, high) => format!("{low}..{high}"),
            Self::AtLeast(low) => format!(">= {low}"),
            Self::Free(unit) => unit.to_owned(),
        }
    }

    /// What a query adds after the value.
    fn domain(self) -> String {
        match self {
            Self::Range(low, high) => format!(" (domain {low}..{high})"),
            _ => String::new(),
        }
    }
}

/// One console variable of a resource `R`.
pub(crate) struct Dvar<R: 'static> {
    name: &'static str,
    about: &'static str,
    kind: Kind,
    get: fn(&R) -> Reading,
    set: fn(&mut R, f32),
    note: fn(&R) -> String,
}

fn no_note<R>(_: &R) -> String {
    String::new()
}

impl<R> Dvar<R> {
    const fn new(
        name: &'static str,
        about: &'static str,
        kind: Kind,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self {
            name,
            about,
            kind,
            get,
            set,
            note: no_note::<R>,
        }
    }

    pub(crate) const fn toggle(
        name: &'static str,
        about: &'static str,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(name, about, Kind::Toggle, get, set)
    }

    pub(crate) const fn range(
        name: &'static str,
        about: &'static str,
        (low, high): (f32, f32),
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(name, about, Kind::Range(low, high), get, set)
    }

    pub(crate) const fn at_least(
        name: &'static str,
        about: &'static str,
        low: f32,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(name, about, Kind::AtLeast(low), get, set)
    }

    pub(crate) const fn free(
        name: &'static str,
        about: &'static str,
        unit: &'static str,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(name, about, Kind::Free(unit), get, set)
    }

    /// Adds a line of explanation to every answer.
    pub(crate) const fn noting(mut self, note: fn(&R) -> String) -> Self {
        self.note = note;
        self
    }

    fn usage(&self) -> String {
        format!("{} [{}] — {}", self.name, self.kind.argument(), self.about)
    }

    fn refusal(&self) -> String {
        format!("usage: {} [{}]", self.name, self.kind.argument())
    }

    fn show(&self, target: &R, domain: bool) -> String {
        let mut line = format!("{} = {}", self.name, (self.get)(target));
        if domain {
            line.push_str(&self.kind.domain());
        }
        let note = (self.note)(target);
        if !note.is_empty() {
            line.push(' ');
            line.push_str(&note);
        }
        line
    }

    /// Answers `args`. Only a value that is accepted touches the resource, so
    /// a question leaves its change flag alone.
    fn answer<T: DerefMut<Target = R>>(&self, args: &[String], target: &mut T) -> String {
        match args {
            [] => self.show(&**target, true),
            [text] => match self.kind.parse(text) {
                Some(value) => {
                    (self.set)(&mut **target, value);
                    self.show(&**target, false)
                }
                None => self.refusal(),
            },
            _ => self.refusal(),
        }
    }
}

/// A resource that holds console variables.
pub(crate) trait DvarSet: Resource<Mutability = Mutable> + Sized {
    const DVARS: &'static [Dvar<Self>];
}

/// Adds the usage line of every variable of `R` that nothing else named.
fn register_dvars<R: DvarSet>(registry: &mut ConsoleRegistry) {
    for dvar in R::DVARS {
        if registry.resolve(dvar.name).is_none() {
            registry.register(CommandSpec::new(dvar.name).usage(dvar.usage()));
        }
    }
}

pub(crate) fn route_dvars<R: DvarSet>(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut target: ResMut<R>,
) {
    for command in commands.read() {
        if let Some(dvar) = R::DVARS.iter().find(|dvar| dvar.name == command.name) {
            echo.write(dvar.answer(&command.args, &mut target));
        }
    }
}

macro_rules! every_set {
    ($($set:ty),+ $(,)?) => {
        /// Registers the variables of every set.
        pub(crate) fn register_all(registry: &mut ConsoleRegistry) {
            $(register_dvars::<$set>(registry);)+
        }

        /// The routers of every set, run one after the other.
        pub(crate) fn routers() -> ScheduleConfigs<ScheduleSystem> {
            ($(route_dvars::<$set>,)+).chain()
        }

        #[cfg(test)]
        fn every_name() -> Vec<&'static str> {
            let mut names = Vec::new();
            $(names.extend(<$set as DvarSet>::DVARS.iter().map(|dvar| dvar.name));)+
            names
        }
    };
}

every_set!(
    render_frontend::assemble::drawsurf::dof::GlowDvars,
    render_frontend::assemble::drawsurf::dof::DofDvars,
    render_frontend::assemble::drawsurf::fog::FogDvars,
    render_frontend::assemble::drawsurf::DistortionSettings,
    render_frontend::assemble::drawsurf::ColourDrawMethod,
    render_frontend::adapters::anim::view_kick::GunOffset,
    render_frontend::prepare::scene::view_parms::SmEnableDvar,
    render_frontend::prepare::scene::view_parms::SmSunEnableDvar,
    render_frontend::prepare::scene::view_parms::RZnearDvar,
    render_frontend::prepare::scene::view_parms::RZnearDepthhackDvar,
    render_frontend::prepare::scene::smodel_geom_cache::SmcEnableDvar,
    render_frontend::prepare::scene::smodel_geom_cache::LodRampDvar,
    render_frontend::prepare::scene::smodel_geom_cache::LodRampSkinnedDvar,
    render_fx::FxMarkDvars,
    net::ClientActionInput,
);

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    #[derive(Resource, Default)]
    struct Fake {
        on: bool,
        level: f32,
        later: Option<f32>,
        free: f32,
        floor: f32,
    }

    impl DvarSet for Fake {
        const DVARS: &'static [Dvar<Self>] = &[
            Dvar::toggle(
                "fk_on",
                "a switch",
                |fake| fake.on.into(),
                |fake, value| fake.on = value != 0.0,
            ),
            Dvar::range(
                "fk_level",
                "a level",
                (1.0, 5.0),
                |fake: &Fake| fake.level.into(),
                |fake: &mut Fake, value| fake.level = value,
            )
            .noting(|fake: &Fake| format!("(floor {})", fake.floor)),
            Dvar::free(
                "fk_free",
                "any number",
                "units",
                |fake| fake.free.into(),
                |fake, value| fake.free = value,
            ),
            Dvar::at_least(
                "fk_floor",
                "no less than",
                0.5,
                |fake| fake.floor.into(),
                |fake, value| fake.floor = value,
            ),
            Dvar::free(
                "fk_later",
                "read late",
                "f",
                |fake| fake.later.into(),
                |fake, value| fake.later = Some(value),
            ),
        ];
    }

    #[derive(Resource, Default)]
    struct Flags(Vec<bool>);

    fn note_change(fake: Res<Fake>, mut flags: ResMut<Flags>) {
        flags.0.push(fake.is_changed());
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .init_resource::<Flags>()
            .insert_resource(Fake {
                level: 2.0,
                floor: 1.0,
                ..Fake::default()
            })
            .add_systems(Update, (route_dvars::<Fake>, note_change).chain());
        app
    }

    fn feed(app: &mut App, script: &str) {
        for command in ConsoleCommand::parse_script(script) {
            app.world_mut().write_message(command);
        }
        app.update();
    }

    fn lines(app: &App) -> Vec<String> {
        app.world().resource::<ConsoleState>().log.to_vec()
    }

    #[test]
    fn a_question_shows_the_value_and_what_it_may_be() {
        let mut app = app();
        feed(&mut app, "fk_level; fk_on; fk_later; fk_free");
        assert_eq!(
            lines(&app),
            [
                "fk_level = 2 (domain 1..5) (floor 1)",
                "fk_on = 0",
                "fk_later = unread",
                "fk_free = 0",
            ]
        );
    }

    #[test]
    fn a_value_in_range_is_set_and_read_back() {
        let mut app = app();
        feed(
            &mut app,
            "fk_level 4.5; fk_free -12.25; fk_later 3; fk_floor 9",
        );
        assert_eq!(
            lines(&app),
            [
                "fk_level = 4.5 (floor 1)",
                "fk_free = -12.25",
                "fk_later = 3",
                "fk_floor = 9",
            ]
        );
        let fake = app.world().resource::<Fake>();
        assert_eq!(
            (fake.level, fake.free, fake.later),
            (4.5, -12.25, Some(3.0))
        );
    }

    #[test]
    fn a_value_outside_what_it_takes_is_refused_and_changes_nothing() {
        let mut app = app();
        feed(
            &mut app,
            "fk_level 0.5; fk_level 5.5; fk_level high; fk_level 1 2; fk_floor 0.25; fk_free nan; fk_free inf; fk_on 2",
        );
        assert_eq!(
            lines(&app),
            [
                "usage: fk_level [1..5]",
                "usage: fk_level [1..5]",
                "usage: fk_level [1..5]",
                "usage: fk_level [1..5]",
                "usage: fk_floor [>= 0.5]",
                "usage: fk_free [units]",
                "usage: fk_free [units]",
                "usage: fk_on [0|1]",
            ]
        );
        let fake = app.world().resource::<Fake>();
        assert_eq!((fake.level, fake.floor, fake.free), (2.0, 1.0, 0.0));
        assert!(!fake.on);
    }

    #[test]
    fn the_ends_of_a_range_are_in_it() {
        let mut app = app();
        feed(&mut app, "fk_level 1; fk_level 5");
        assert_eq!(app.world().resource::<Fake>().level, 5.0);
        assert_eq!(lines(&app).len(), 2);
    }

    #[test]
    fn a_switch_takes_numbers_and_words() {
        let mut app = app();
        feed(
            &mut app,
            "fk_on 1; fk_on off; fk_on true; fk_on 0; fk_on on",
        );
        assert_eq!(
            lines(&app),
            [
                "fk_on = 1",
                "fk_on = 0",
                "fk_on = 1",
                "fk_on = 0",
                "fk_on = 1"
            ]
        );
        assert!(app.world().resource::<Fake>().on);
    }

    #[test]
    fn only_a_set_marks_the_resource_changed() {
        let mut app = app();
        app.update();
        feed(&mut app, "fk_level");
        feed(&mut app, "fk_level 9");
        feed(&mut app, "fk_level 3");
        assert_eq!(app.world().resource::<Fake>().level, 3.0);
        let flags = &app.world().resource::<Flags>().0;
        assert_eq!(flags[1..], [false, false, true]);
    }

    #[test]
    fn other_commands_are_left_alone() {
        let mut app = app();
        feed(&mut app, "bot add; fk_unknown 1");
        assert!(lines(&app).is_empty());
    }

    #[test]
    fn usage_names_what_the_variable_takes() {
        let mut registry = ConsoleRegistry::default();
        register_dvars::<Fake>(&mut registry);
        let help = registry.help_lines();
        for want in [
            "fk_on [0|1] — a switch",
            "fk_level [1..5] — a level",
            "fk_free [units] — any number",
            "fk_floor [>= 0.5] — no less than",
        ] {
            assert!(
                help.contains(&want.to_owned()),
                "{want} missing in {help:?}"
            );
        }
    }

    #[test]
    fn a_name_that_was_there_first_is_left_alone() {
        let mut registry = ConsoleRegistry::default();
        registry.register(CommandSpec::new("fk_on").usage("fk_on — mine"));
        register_dvars::<Fake>(&mut registry);
        assert!(registry.help_lines().contains(&"fk_on — mine".to_owned()));
        assert_eq!(registry.names().iter().filter(|n| *n == "fk_on").count(), 1);
    }

    #[test]
    fn no_two_sets_use_the_same_name() {
        let names = every_name();
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "a name is used twice");
        assert!(names.len() >= 39, "{} names", names.len());
    }

    #[test]
    fn every_set_registers_a_usage_that_starts_with_its_name() {
        let mut registry = ConsoleRegistry::default();
        register_all(&mut registry);
        let help = registry.help_lines();
        for name in every_name() {
            assert!(
                help.iter()
                    .any(|line| line.starts_with(&format!("{name} "))),
                "{name} has no usage line"
            );
        }
    }
}
