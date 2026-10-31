//! `showpos`: where the local player stands, as one line or as an overlay
//! that stays up and follows them.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{HostTimeKeeper, PlayerControlSnapshot, HostViewClient, ShownWorldFrame};
use ui::OverlayTier;

use crate::tool_cmds::{PromptReplyWriter, CoordinateReadout};
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::PromptOutputRow;
use crate::OrderCatalog;
use sim::ClientId;

const NAME: &str = "showpos";
const ALIAS: &str = "debug_pos";
const HELP_BANNER: &str = "usage: showpos [on|off]";

pub(crate) fn enlist(book: &mut OrderCatalog) {
    if book.look_up(NAME).is_some() {
        return;
    }
    let spec = OrderBlueprint::new(NAME).also_known(ALIAS);
    book.enlist(spec.help_line("showpos [on|off] — origin overlay (bare also prints once)"));
}

/// The overlay's text box, top left, over everything else.
#[derive(Component)]
pub(crate) struct CoordinateReadoutHud;

pub(crate) fn spawn_hud(commands: &mut Commands, typeface: Handle<Font>) {
    let mut place = Node::default();
    place.position_type = PositionType::Absolute;
    place.left = px(12);
    place.top = px(12);
    place.padding = UiRect::all(px(6));
    let mut face = TextFont::default();
    face.font = typeface.into();
    face.font_size = FontSize::Px(14.0);
    let backdrop = BackgroundColor(Color::srgba(0.02, 0.03, 0.04, 0.72));
    let ink = TextColor(Color::srgb(0.95, 0.85, 0.40));
    // Just under the console panel.
    let stacking = GlobalZIndex(19_000);
    let hidden = Visibility::Hidden;
    let text = Text(String::new());
    commands.spawn((CoordinateReadoutHud, OverlayTier::Overlay, hidden, place, backdrop, stacking, text, face, ink));
}

/// The one-line answer: feet, eyes and where the player looks.
fn once(presented: &ShownWorldFrame, client: ClientId) -> String {
    match presented.living_participant(client) {
        None => "showpos: not Alive".to_owned(),
        Some(player) => {
            let [x, y, z] = player.origin;
            let eye_z = z + player.eye_height_now;
            let [pitch, yaw, _] = player.look_angles;
            format!("showpos origin={x:.1} {y:.1} {z:.1}  eye={x:.1} {y:.1} {eye_z:.1}  yaw={yaw:.0} pitch={pitch:.0}")
        }
    }
}

/// `—` for nothing, else the text.
fn or_dash(text: Option<String>) -> String {
    text.unwrap_or_else(|| "—".to_owned())
}

/// The overlay's text: the one-line answer's facts plus what the frame is
/// doing — the tick, the life state, speed up, the ground, held buttons, and
/// how fast frames come.
fn live(
    presented: &ShownWorldFrame,
    who: sim::ClientId,
    tick: u32,
    frame_secs: f32,
    held: &[&str],
) -> String {
    let meta = presented.snapshot().and_then(|snapshot| snapshot.meta.for_client(who));
    let life = or_dash(meta.map(|found| format!("{:?}", found.lifecycle)));
    let player = presented.living_participant(who);
    let origin = or_dash(player.map(|p| format!("{:.0} {:.0} {:.0}", p.origin[0], p.origin[1], p.origin[2])));
    let eye = or_dash(player.map(|p| {
        let [x, y, z] = p.origin;
        format!("{x:.0} {y:.0} {:.0}", z + p.eye_height_now)
    }));
    let yaw = player.map_or(0.0, |p| p.look_angles[1]);
    let pitch = player.map_or(0.0, |p| p.look_angles[0]);
    let vz = player.map_or(0.0, |p| p.velocity[2]);
    let ground = player.map_or(-1, |p| p.standing_on);
    let fps = (frame_secs > 0.0).then(|| frame_secs.recip()).unwrap_or(0.0);
    let held = or_dash((!held.is_empty()).then(|| held.join(",")));
    let turn = format!("yaw={yaw:.0} pitch={pitch:.0}");
    format!("showpos tick={tick} life={life} origin={origin} eye={eye} {turn} vz={vz:.0} ground={ground} held=[{held}] fps={fps:.0}")
}

/// What a `showpos` with these words does to the overlay, and what it says.
fn showpos_reply(args: &[String], overlay: &mut bool, answer_now: impl FnOnce() -> String) -> String {
    if args.len() > 1 {
        return HELP_BANNER.into();
    }
    let Some(word) = args.first().map(String::as_str) else {
        *overlay = true;
        return answer_now();
    };
    let wanted = match word {
        "on" | "1" => true,
        "off" | "0" => false,
        _ => return format!("{HELP_BANNER} (got `{word}`)"),
    };
    *overlay = wanted;
    let state = if wanted { "on" } else { "off" };
    format!("showpos {state}")
}

pub(crate) fn route_showpos(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut overlay: ResMut<CoordinateReadout>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
) {
    let ours = |name: &str| name == NAME || name == ALIAS;
    for asked in commands.read().filter(|asked| ours(&asked.name)) {
        let reply = showpos_reply(&asked.words, &mut overlay.0, || once(&presented, local.0));
        echo.write(reply);
    }
}

/// What the overlay reads each frame.
#[derive(SystemParam)]
pub(crate) struct Readings<'w> {
    frame: Res<'w, ShownWorldFrame>,
    viewer: Res<'w, HostViewClient>,
    clock: Res<'w, HostTimeKeeper>,
    time: Res<'w, Time>,
    input: Res<'w, PlayerControlSnapshot>,
}

impl Readings<'_> {
    fn text(&self) -> String {
        let mut held = Vec::new();
        self.input.client.pressed_key_latch.each_held_command(|name| held.push(name));
        live(&self.frame, self.viewer.0, self.clock.tick, self.time.delta_secs(), &held)
    }
}

/// Keeps the overlay's box and the console's status line current while the
/// overlay is on, and the box hidden while it is off.
pub(crate) fn update_overlay(
    overlay: Res<CoordinateReadout>,
    readings: Readings,
    mut line: ResMut<PromptOutputRow>,
    mut huds: Query<(&'static mut Text, &'static mut Visibility), With<CoordinateReadoutHud>>,
) {
    let text = overlay.0.then(|| readings.text());
    for (mut hud, mut shown) in &mut huds {
        match &text {
            Some(current) => {
                *shown = Visibility::Visible;
                hud.0.clone_from(current);
            }
            None => *shown = Visibility::Hidden,
        }
    }
    if let Some(current) = text {
        line.0 = current;
    }
}
#[cfg(test)]
mod tests {
    use crate::showpos::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use crate::host_link::testing::{host_app, present_player};
    use bevy::app::Update;

    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<CoordinateReadout>()
            .add_systems(Update, route_showpos);
        present_player(&mut app, [10.0, 20.5, 30.0], [-10.0, 90.0, 0.0], true, "");
        app
    }

    fn overlay(app: &App) -> bool {
        app.world().resource::<CoordinateReadout>().0
    }

    #[test]
    fn bare_turns_the_overlay_on_and_prints_feet_eyes_and_where_it_looks() {
        let mut app = app();
        feed(&mut app, "showpos");
        let expected = ["showpos origin=10.0 20.5 30.0  eye=10.0 20.5 90.0  yaw=90 pitch=-10"];
        assert_eq!(said(&app), expected);
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
        let expected = [
            "usage: showpos [on|off] (got `maybe`)",
            "usage: showpos [on|off]"
        ];
        assert_eq!(said(&app), expected);
        assert!(!overlay(&app));
    }

    #[test]
    fn registered_once_with_the_old_name_as_an_alias() {
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        assert!(book.look_up("showpos").is_some());
        assert!(book.look_up("debug_pos").is_some());
        let expected = 1;
        assert_eq!(book
            .usage_listing()
            .iter()
            .filter(|line| line.contains("origin overlay"))
            .count(), expected);
    }

    #[test]
    fn the_live_text_gives_the_frame_the_player_and_the_held_buttons() {
        let mut app = app();
        let presented = app.world().resource::<ShownWorldFrame>();
        let client = app.world().resource::<HostViewClient>().0;
        let expected = "showpos tick=7 life=Alive origin=10 20 30 eye=10 20 90 yaw=90 pitch=-10 vz=0 ground=5 held=[+attack,+melee] fps=50";
        assert_eq!(live(presented, client, 7, 0.02, &["+attack", "+melee"]), expected);
        present_player(&mut app, [0.0; 3], [0.0; 3], false, "");
        let presented = app.world().resource::<ShownWorldFrame>();
        let expected = "showpos tick=0 life=Connecting origin=— eye=— yaw=0 pitch=0 vz=0 ground=-1 held=[—] fps=0";
        assert_eq!(live(presented, client, 0, 0.0, &[]), expected);
        let nobody = ShownWorldFrame::default();
        assert!(live(&nobody, client, 0, 0.0, &[]).contains("life=— origin=—"));
    }

    fn overlay_app(on: bool) -> (App, Entity) {
        let mut app = console_app();
        app.init_resource::<HostViewClient>()
            .init_resource::<HostTimeKeeper>()
            .init_resource::<PlayerControlSnapshot>()
            .init_resource::<Time>()
            .insert_resource(CoordinateReadout(on))
            .add_systems(Update, update_overlay);
        present_player(&mut app, [1.0, 2.0, 3.0], [0.0; 3], true, "");
        let hud = app
            .world_mut()
            .spawn((CoordinateReadoutHud, Text::new("old"), Visibility::Visible))
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
        let expected = Some(&Visibility::Visible);
        assert_eq!(app.world().get::<Visibility>(hud), expected);
        assert_eq!(app.world().resource::<PromptOutputRow>().0, shown);

        app.world_mut().resource_mut::<CoordinateReadout>().0 = false;
        app.update();
        let expected = Some(&Visibility::Hidden);
        assert_eq!(app.world().get::<Visibility>(hud), expected);
    }

    #[test]
    fn an_overlay_that_is_off_leaves_the_status_line_and_the_box_text_alone() {
        let (mut app, hud) = overlay_app(false);
        app.world_mut().resource_mut::<PromptOutputRow>().0 = "kept".into();
        app.update();
        assert_eq!(app.world().resource::<PromptOutputRow>().0, "kept");
        assert_eq!(app.world().get::<Text>(hud).unwrap().0, "old");
        let expected = Some(&Visibility::Hidden);
        assert_eq!(app.world().get::<Visibility>(hud), expected);
    }
}
