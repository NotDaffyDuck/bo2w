//! Console variables that are one number in a resource. A table per resource
//! names each variable, how it reads, how it sets and what values it takes;
//! one router answers them all: no argument asks, one argument sets.

use std::{fmt, ops::DerefMut};

use bevy::{ecs::component::Mutable, ecs::schedule::ScheduleConfigs, ecs::system::ScheduleSystem};
use bevy::{ecs::message::MessageReader, ecs::resource::Resource, ecs::system::ResMut};
use bevy::ecs::schedule::IntoScheduleConfigs;

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

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
            _other => String::new(),
        }
    }
}

/// One console variable of a resource `R`.
pub(crate) struct Dvar<R: 'static> {
    dvar_name: &'static str,
    about: &'static str,
    kind: Kind,
    get: fn(&R) -> Reading,
    set: fn(&mut R, f32),
    note: fn(&R) -> String,
}

fn no_note<R>(_: &R) -> String {
    String::default()
}

impl<R> Dvar<R> {
    const fn new(
        dvar_name: &'static str,
        about: &'static str,
        kind: Kind,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self {
            dvar_name,
            about,
            kind,
            get,
            set,
            note: no_note::<R>,
        }
    }

    pub(crate) const fn toggle(
        dvar_name: &'static str,
        about: &'static str,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(dvar_name, about, Kind::Toggle, get, set)
    }

    pub(crate) const fn range(
        dvar_name: &'static str,
        about: &'static str,
        (low, high): (f32, f32),
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(dvar_name, about, Kind::Range(low, high), get, set)
    }

    pub(crate) const fn at_least(
        dvar_name: &'static str,
        about: &'static str,
        low: f32,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(dvar_name, about, Kind::AtLeast(low), get, set)
    }

    pub(crate) const fn free(
        dvar_name: &'static str,
        about: &'static str,
        unit: &'static str,
        get: fn(&R) -> Reading,
        set: fn(&mut R, f32),
    ) -> Self {
        Self::new(dvar_name, about, Kind::Free(unit), get, set)
    }

    /// Adds a line of explanation to every answer.
    pub(crate) const fn noting(mut self, note: fn(&R) -> String) -> Self {
        self.note = note;
        self
    }

    fn usage(&self) -> String {
        format!("{} [{}] — {}", self.dvar_name, self.kind.argument(), self.about)
    }

    fn refusal(&self) -> String {
        format!("usage: {} [{}]", self.dvar_name, self.kind.argument())
    }

    fn show(&self, target: &R, domain: bool) -> String {
        let mut line = format!("{} = {}", self.dvar_name, (self.get)(target));
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

    /// Answers `given`. Only a value that is accepted touches the resource, so
    /// a question leaves its change flag alone.
    fn answer<T: DerefMut<Target = R>>(&self, given: &[String], target: &mut T) -> String {
        match given {
            [] => self.show(&**target, true),
            [text] => match self.kind.parse(text) {
                Some(parsed) => {
                    (self.set)(&mut **target, parsed);
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
fn register_dvars<R: DvarSet>(book: &mut OrderCatalog) {
    for dvar in R::DVARS {
        if book.look_up(dvar.dvar_name).is_none() {
            book.enlist(OrderBlueprint::new(dvar.dvar_name).help_line(dvar.usage()));
        }
    }
}

pub(crate) fn route_dvars<R: DvarSet>(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut target: ResMut<R>,
) {
    for order in commands.read() {
        if let Some(dvar) = R::DVARS.iter().find(|dvar| dvar.dvar_name == order.name) {
            echo.write(dvar.answer(&order.words, &mut target));
        }
    }
}

macro_rules! every_set {
    ($($set:ty),+ $(,)?) => {
        /// Registers the variables of every set.
        pub(crate) fn register_all(book: &mut OrderCatalog) {
            $(register_dvars::<$set>(book);)+
        }

        /// The routers of every set, run one after the other.
        pub(crate) fn routers() -> ScheduleConfigs<ScheduleSystem> {
            ($(route_dvars::<$set>,)+).chain()
        }

        #[cfg(test)]
        fn every_name() -> Vec<&'static str> {
            let mut names: Vec<&'static str> = vec![];
            $(names.extend(<$set as DvarSet>::DVARS.iter().map(|dvar| dvar.dvar_name));)+
            names
        }
    };
}

every_set!(
    render_frontend::assemble::draw_items::dof::BloomTunables,
    render_frontend::assemble::draw_items::dof::BokehTunables,
    render_frontend::assemble::draw_items::fog::HazeTunables,
    render_frontend::assemble::draw_items::WarpSettings,
    render_frontend::assemble::draw_items::ColourPaintMethod,
    render_frontend::adapters::motion::eye_kick::GunOffset,
    render_frontend::prepare::scene::eye_parms::SmEnableDvar,
    render_frontend::prepare::scene::eye_parms::SmSunEnableDvar,
    render_frontend::prepare::scene::eye_parms::NearClipSetting,
    render_frontend::prepare::scene::eye_parms::GunNearClipSetting,
    render_frontend::prepare::scene::prop_geom_memo::PropCacheEnableTunable,
    render_frontend::prepare::scene::prop_geom_memo::DetailRampTunable,
    render_frontend::prepare::scene::prop_geom_memo::DetailRampSkinnedTunable,
    render_fx::EffectStainTunables,
    net::PlayerControlSnapshot,
);

#[cfg(test)]
mod tests {
    use bevy::{app::App, ecs::change_detection::DetectChanges, ecs::system::Res};
    use std::collections::HashSet;

    use crate::dvars::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
    use bevy::app::Update;

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

    #[derive(Default, Resource)]
    struct Flags(Vec<bool>);

    fn note_change(fake: Res<Fake>, mut flags: ResMut<Flags>) {
        flags.0.push(fake.is_changed());
    }

    fn app() -> App {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .init_resource::<Flags>()
            .insert_resource(Fake {
                level: 2.0,
                floor: 1.0,
                ..Fake::default()
            })
            .add_systems(Update, (route_dvars::<Fake>, note_change).chain());
        app
    }

    fn feed(app: &mut App, typed: &str) {
        for command in TypedOrder::parse_script(typed) {
            app.world_mut().write_message(command);
        }
        app.update();
    }

    fn lines(app: &App) -> Vec<String> {
        app.world().resource::<PromptPanelStatus>().lines.to_vec()
    }

    #[test]
    fn a_question_shows_the_value_and_what_it_may_be() {
        let mut app = app();
        feed(&mut app, "fk_level; fk_on; fk_later; fk_free");
        let expected = [
            "fk_level = 2 (domain 1..5) (floor 1)",
            "fk_on = 0",
            "fk_later = unread",
            "fk_free = 0",
        ];
        assert_eq!(lines(&app), expected);
    }

    #[test]
    fn a_value_in_range_is_set_and_read_back() {
        let mut app = app();
        feed(
            &mut app,
            "fk_level 4.5; fk_free -12.25; fk_later 3; fk_floor 9",
        );
        let expected = [
            "fk_level = 4.5 (floor 1)",
            "fk_free = -12.25",
            "fk_later = 3",
            "fk_floor = 9",
        ];
        assert_eq!(lines(&app), expected);
        let fake = app.world().resource::<Fake>();
        let expected = (4.5, -12.25, Some(3.0));
        assert_eq!((fake.level, fake.free, fake.later), expected);
    }

    #[test]
    fn a_value_outside_what_it_takes_is_refused_and_changes_nothing() {
        let mut app = app();
        feed(
            &mut app,
            "fk_level 0.5; fk_level 5.5; fk_level high; fk_level 1 2; fk_floor 0.25; fk_free nan; fk_free inf; fk_on 2",
        );
        let expected = [
            "usage: fk_level [1..5]",
            "usage: fk_level [1..5]",
            "usage: fk_level [1..5]",
            "usage: fk_level [1..5]",
            "usage: fk_floor [>= 0.5]",
            "usage: fk_free [units]",
            "usage: fk_free [units]",
            "usage: fk_on [0|1]",
        ];
        assert_eq!(lines(&app), expected);
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
        let expected = [
            "fk_on = 1",
            "fk_on = 0",
            "fk_on = 1",
            "fk_on = 0",
            "fk_on = 1"
        ];
        assert_eq!(lines(&app), expected);
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
        let mut book = OrderCatalog::default();
        register_dvars::<Fake>(&mut book);
        let help = book.usage_listing();
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
        let mut book = OrderCatalog::default();
        book.enlist(OrderBlueprint::new("fk_on").help_line("fk_on — mine"));
        register_dvars::<Fake>(&mut book);
        assert!(book.usage_listing().contains(&"fk_on — mine".to_owned()));
        assert_eq!(book.all_names().iter().filter(|n| *n == "fk_on").count(), 1);
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
        let mut book = OrderCatalog::default();
        register_all(&mut book);
        let help = book.usage_listing();
        for name in every_name() {
            assert!(
                help.iter()
                    .any(|line| line.starts_with(&format!("{name} "))),
                "{name} has no usage line"
            );
        }
    }
}
