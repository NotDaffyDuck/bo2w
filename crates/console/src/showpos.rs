//! `showpos`: where the local player stands, as one line or as an overlay
//! that stays up and follows them.

use bevy::{ecs::system::SystemParam, prelude::*};
use net::{AuthorityClock, ClientActionInput, LocalPresentClient, PresentedSnapshot};
use sim::ClientId;
use ui::UiLayer;

use crate::feature_dispatch::{ConsoleEcho, DebugPosOverlay};
use crate::{CommandSpec, ConsoleCommand, ConsoleLine, ConsoleRegistry};

const NAME: &str = "showpos";
const ALIAS: &str = "debug_pos";
const USAGE: &str = "usage: showpos [on|off]";

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    if registry.resolve(NAME).is_none() {
        registry.register(
            CommandSpec::new(NAME)
                .alias(ALIAS)
                .usage("showpos [on|off] — origin overlay (bare also prints once)"),
        );
    }
}

/// The overlay's text box, top left, over everything else.
#[derive(Component)]
pub(crate) struct ShowposHud;

pub(crate) fn spawn_hud(commands: &mut Commands, font: Handle<Font>) {
    commands.spawn((
        ShowposHud,
        UiLayer::Overlay,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            left: px(12),
            top: px(12),
            padding: UiRect::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.04, 0.72)),
        GlobalZIndex(19_000),
        Text::new(""),
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.85, 0.40)),
    ));
}

/// The one-line answer: feet, eyes and where the player looks.
fn once(presented: &PresentedSnapshot, client: ClientId) -> String {
    let Some(player) = presented.alive_player(client) else {
        return "showpos: not Alive".into();
    };
    let [x, y, z] = player.origin;
    let eye = z + player.view_height_current;
    format!(
        "showpos origin={x:.1} {y:.1} {z:.1}  eye={x:.1} {y:.1} {eye:.1}  yaw={:.0} pitch={:.0}",
        player.viewangles[1], player.viewangles[0],
    )
}

/// The overlay's text: the one-line answer's facts plus what the frame is
/// doing — the tick, the life state, speed up, the ground, held buttons, and
/// how fast frames come.
fn live(
    presented: &PresentedSnapshot,
    client: ClientId,
    tick: u32,
    frame_secs: f32,
    held: &[&str],
) -> String {
    let life = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(client))
        .map_or_else(|| "—".to_owned(), |meta| format!("{:?}", meta.lifecycle));
    let (origin, eye, yaw, pitch, vz, ground) = match presented.alive_player(client) {
        Some(player) => {
            let [x, y, z] = player.origin;
            (
                format!("{x:.0} {y:.0} {z:.0}"),
                format!("{x:.0} {y:.0} {:.0}", z + player.view_height_current),
                player.viewangles[1],
                player.viewangles[0],
                player.velocity[2],
                player.ground_entity_num,
            )
        }
        None => ("—".to_owned(), "—".to_owned(), 0.0, 0.0, 0.0, -1),
    };
    let fps = if frame_secs > 0.0 {
        1.0 / frame_secs
    } else {
        0.0
    };
    let held = if held.is_empty() {
        "—".to_owned()
    } else {
        held.join(",")
    };
    format!(
        "showpos tick={tick} life={life} origin={origin} eye={eye} yaw={yaw:.0} pitch={pitch:.0} vz={vz:.0} ground={ground} held=[{held}] fps={fps:.0}"
    )
}

pub(crate) fn route_showpos(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut overlay: ResMut<DebugPosOverlay>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
) {
    for command in commands
        .read()
        .filter(|command| matches!(command.name.as_str(), NAME | ALIAS))
    {
        let said = match command.args.as_slice() {
            [] => {
                overlay.0 = true;
                once(&presented, local.0)
            }
            [word] if matches!(word.as_str(), "on" | "1") => {
                overlay.0 = true;
                "showpos on".into()
            }
            [word] if matches!(word.as_str(), "off" | "0") => {
                overlay.0 = false;
                "showpos off".into()
            }
            [word] => format!("{USAGE} (got `{word}`)"),
            _ => USAGE.into(),
        };
        echo.write(said);
    }
}

/// What the overlay reads each frame.
#[derive(SystemParam)]
pub(crate) struct Readings<'w> {
    presented: Res<'w, PresentedSnapshot>,
    local: Res<'w, LocalPresentClient>,
    clock: Res<'w, AuthorityClock>,
    time: Res<'w, Time>,
    input: Res<'w, ClientActionInput>,
}

/// Keeps the overlay's box and the console's status line current while the
/// overlay is on, and the box hidden while it is off.
pub(crate) fn update_overlay(
    overlay: Res<DebugPosOverlay>,
    readings: Readings,
    mut line: ResMut<ConsoleLine>,
    mut huds: Query<(&mut Text, &mut Visibility), With<ShowposHud>>,
) {
    if !overlay.0 {
        for (_, mut visibility) in &mut huds {
            *visibility = Visibility::Hidden;
        }
        return;
    }
    let mut held = Vec::new();
    readings
        .input
        .client
        .kb
        .visit_active_names(|name| held.push(name));
    let text = live(
        &readings.presented,
        readings.local.0,
        readings.clock.tick,
        readings.time.delta_secs(),
        &held,
    );
    for (mut hud, mut visibility) in &mut huds {
        *visibility = Visibility::Visible;
        hud.0.clone_from(&text);
    }
    line.0 = text;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};
    use crate::host_link::testing::{host_app, present_player};

    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<DebugPosOverlay>()
            .add_systems(Update, route_showpos);
        present_player(&mut app, [10.0, 20.5, 30.0], [-10.0, 90.0, 0.0], true, "");
        app
    }

    fn overlay(app: &App) -> bool {
        app.world().resource::<DebugPosOverlay>().0
    }

    #[test]
    fn bare_turns_the_overlay_on_and_prints_feet_eyes_and_where_it_looks() {
        let mut app = app();
        feed(&mut app, "showpos");
        assert_eq!(
            said(&app),
            ["showpos origin=10.0 20.5 30.0  eye=10.0 20.5 90.0  yaw=90 pitch=-10"]
        );
        assert!(overlay(&app));
    }

    #[test]
    fn bare_for_a_player_who_is_not_alive_says_so() {
        let mut app = app();
        let origin = [0.0; 3];
        present_player(&mut app, origin, origin, false, "");
        feed(&mut app, "showpos");
        assert_eq!(said(&app), ["showpos: not Alive"]);
        assert!(overlay(&app), "the overlay still comes up, to show why");
    }

    #[test]
    fn on_and_off_by_word_or_digit_and_the_old_name_too() {
        let mut app = app();
        for (line, reply, on) in [
            ("showpos on", "showpos on", true),
            ("showpos off", "showpos off", false),
            ("debug_pos 1", "showpos on", true),
            ("debug_pos 0", "showpos off", false),
        ] {
            feed(&mut app, line);
            assert_eq!(said(&app).last().map(String::as_str), Some(reply), "{line}");
            assert_eq!(overlay(&app), on, "{line}");
        }
    }

    #[test]
    fn anything_else_is_usage_and_leaves_the_overlay_as_it_was() {
        let mut app = app();
        feed(&mut app, "showpos maybe; showpos on off");
        assert_eq!(
            said(&app),
            [
                "usage: showpos [on|off] (got `maybe`)",
                "usage: showpos [on|off]"
            ]
        );
        assert!(!overlay(&app));
    }

    #[test]
    fn registered_once_with_the_old_name_as_an_alias() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        register(&mut registry);
        assert!(registry.resolve("showpos").is_some());
        assert!(registry.resolve("debug_pos").is_some());
        assert_eq!(
            registry
                .help_lines()
                .iter()
                .filter(|line| line.contains("origin overlay"))
                .count(),
            1
        );
    }

    #[test]
    fn the_live_text_gives_the_frame_the_player_and_the_held_buttons() {
        let mut app = app();
        let presented = app.world().resource::<PresentedSnapshot>();
        let client = app.world().resource::<LocalPresentClient>().0;
        assert_eq!(
            live(presented, client, 7, 0.02, &["+attack", "+melee"]),
            "showpos tick=7 life=Alive origin=10 20 30 eye=10 20 90 yaw=90 pitch=-10 vz=0 ground=5 held=[+attack,+melee] fps=50"
        );
        present_player(&mut app, [0.0; 3], [0.0; 3], false, "");
        let presented = app.world().resource::<PresentedSnapshot>();
        assert_eq!(
            live(presented, client, 0, 0.0, &[]),
            "showpos tick=0 life=Connecting origin=— eye=— yaw=0 pitch=0 vz=0 ground=-1 held=[—] fps=0"
        );
        let nobody = PresentedSnapshot::default();
        assert!(live(&nobody, client, 0, 0.0, &[]).contains("life=— origin=—"));
    }

    fn overlay_app(on: bool) -> (App, Entity) {
        let mut app = console_app();
        app.init_resource::<LocalPresentClient>()
            .init_resource::<AuthorityClock>()
            .init_resource::<ClientActionInput>()
            .init_resource::<Time>()
            .insert_resource(DebugPosOverlay(on))
            .add_systems(Update, update_overlay);
        present_player(&mut app, [1.0, 2.0, 3.0], [0.0; 3], true, "");
        let hud = app
            .world_mut()
            .spawn((ShowposHud, Text::new("old"), Visibility::Visible))
            .id();
        (app, hud)
    }

    #[test]
    fn the_overlay_box_shows_the_live_text_while_on_and_hides_while_off() {
        let (mut app, hud) = overlay_app(true);
        app.update();
        let shown = app.world().get::<Text>(hud).unwrap().0.clone();
        assert!(
            shown.starts_with("showpos tick=0 life=Alive origin=1 2 3 "),
            "{shown}"
        );
        assert_eq!(
            app.world().get::<Visibility>(hud),
            Some(&Visibility::Visible)
        );
        assert_eq!(app.world().resource::<ConsoleLine>().0, shown);

        app.world_mut().resource_mut::<DebugPosOverlay>().0 = false;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(hud),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn an_overlay_that_is_off_leaves_the_status_line_and_the_box_text_alone() {
        let (mut app, hud) = overlay_app(false);
        app.world_mut().resource_mut::<ConsoleLine>().0 = "kept".into();
        app.update();
        assert_eq!(app.world().resource::<ConsoleLine>().0, "kept");
        assert_eq!(app.world().get::<Text>(hud).unwrap().0, "old");
        assert_eq!(
            app.world().get::<Visibility>(hud),
            Some(&Visibility::Hidden)
        );
    }
}
