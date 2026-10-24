//! bo2mp: the dirt and dust the RC-XD's wheels throw on its driver's screen.
//! Its client script (`_rcbomb.csc`) picks one of four of a menu's items
//! every 250-500 ms while he drives fast on dirt (`fullscreen_dirt`) or on
//! anything else (`fullscreen_dust`); the server's `bo2mp_rc_screen` script
//! dvar carries "<count> <menu> <item>". Each item plays its menu's own
//! states (common_mp's fullscreen_dirt / fullscreen_dust, a 640x480
//! fullscreen menu, tinted 0.56 0.53 0.4): a clump flying out from the
//! bottom and the blur it leaves for two seconds, a splash down the right
//! or left side, or a blur at a random place.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::ClientSet;
use net::{LocalPresentClient, PresentedSnapshot};
use std::collections::HashMap;

use crate::layers::{UiLayer, UiLayerVisibility};

#[derive(Component)]
struct RcxdScreenNode;

/// A rect in the menu's 640x480 screen and its alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    rect: [f32; 4],
    alpha: f32,
}

const FULL: [f32; 4] = [-192.0, -144.0, 1024.0, 768.0];
const MID: [f32; 4] = [-64.0, 27.0, 768.0, 576.0];
const SCREEN: [f32; 4] = [0.0, 0.0, 640.0, 480.0];
/// The items' picture tint (their `forecolor`).
const TINT: [f32; 3] = [0.56, 0.53, 0.4];

/// An item's part: a dirt clump, the blur it leaves, a side splash, a blur
/// at a random place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Part {
    Clump,
    Blur,
    Right,
    Left,
    Random,
}

impl Part {
    fn of(item: &str) -> Option<Self> {
        Some(match item {
            "dirt" | "dust" => Self::Clump,
            "dirt_right_splash" | "dust_right_splash" => Self::Right,
            "dirt_left_splash" | "dust_left_splash" => Self::Left,
            "blurred_dirt_random" | "blurred_dust_random" => Self::Random,
            _ => return None,
        })
    }
    fn picture(self) -> &'static str {
        match self {
            Self::Clump => "fullscreen_dirt_bottom",
            Self::Blur | Self::Random => "fullscreen_dirt_bottom_b",
            Self::Right => "fullscreen_dirt_right",
            Self::Left => "fullscreen_dirt_left",
        }
    }
    /// Where it rests (its "Default" state).
    fn rest(self) -> Look {
        let rect = match self {
            Self::Clump | Self::Random => FULL,
            Self::Blur => MID,
            Self::Right | Self::Left => SCREEN,
        };
        Look { rect, alpha: 0.0 }
    }
}

/// One item playing: from where it was, through its states (each reached
/// over its time in ms).
#[derive(Clone, Debug)]
struct Track {
    from: Look,
    start: f64,
    steps: Vec<(Look, f64)>,
}

impl Track {
    fn at(&self, now: f64) -> Look {
        let mut from = self.from;
        let mut t = self.start;
        for &(to, ms) in &self.steps {
            if now < t + ms {
                let k = ((now - t) / ms.max(1.0)) as f32;
                let mut rect = from.rect;
                for (r, (a, b)) in rect.iter_mut().zip(from.rect.iter().zip(to.rect)) {
                    *r = a + (b - a) * k;
                }
                return Look { rect, alpha: from.alpha + (to.alpha - from.alpha) * k };
            }
            from = to;
            t += ms;
        }
        from
    }
    fn end(&self) -> f64 {
        self.start + self.steps.iter().map(|s| s.1).sum::<f64>()
    }
}

#[derive(Default)]
struct State {
    seen: Option<String>,
    /// (menu, part) playing.
    tracks: HashMap<(bool, Part), Track>,
    /// The blur a clump starts when it reaches the middle (its in_2
    /// state's `ui_animate ... blurred_dirt in 65`): when, and its peak.
    blur_due: Vec<(bool, f64, f32)>,
    /// The random blur's place (`blurred_dirt_random_X/_Y`, rolled again as
    /// it fades out), per menu, and a dice.
    random_at: HashMap<bool, (f32, f32)>,
    dice: u32,
    pictures: HashMap<&'static str, Handle<Image>>,
}

/// The item's states from "in" (menus: `true` = fullscreen_dust, whose
/// peaks are 0.25 where the dirt's clump and blurs are 1).
fn steps(part: Part, dust: bool, random: (f32, f32)) -> Vec<(Look, f64)> {
    let peak = if dust { 0.25 } else { 1.0 };
    let look = |rect, alpha| Look { rect, alpha };
    match part {
        Part::Clump => vec![
            (look([320.0, 540.0, 1.0, 1.0], peak), 0.0),
            (look(MID, 0.25), 195.0),
            (look(FULL, 0.0), 65.0),
        ],
        Part::Blur => vec![(look(FULL, peak), 65.0), (look(FULL, peak), 2000.0), (look(FULL, 0.0), 1000.0)],
        Part::Right | Part::Left => {
            vec![(look(SCREEN, peak), 0.0), (look(SCREEN, peak), 500.0), (look(SCREEN, 0.0), 1000.0)]
        }
        Part::Random => {
            let r = [random.0 - 292.0, random.1 - 244.0, 1024.0, 768.0];
            vec![(look(r, peak), 0.0), (look(r, peak), 500.0), (look(r, 0.0), 250.0)]
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    nodes: Query<Entity, With<RcxdScreenNode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut st: Local<State>,
) {
    for e in &nodes {
        commands.entity(e).try_despawn();
    }
    let now = time.elapsed_secs_f64() * 1000.0;
    let said = (|| {
        let (p, l) = (presented.as_deref()?, local.as_deref()?);
        p.snapshot()?.meta.script_dvars(l.0).string("bo2mp_rc_screen").map(str::to_owned)
    })();
    if said.is_some() && said != st.seen {
        st.seen = said.clone();
        let said = said.unwrap_or_default();
        let mut words = said.split(' ').skip(1);
        if let (Some(menu), Some(part)) = (words.next(), words.next().and_then(Part::of)) {
            let dust = menu == "fullscreen_dust";
            let random = st.random_at.get(&dust).copied().unwrap_or((0.0, 0.0));
            let from = st.tracks.get(&(dust, part)).map_or(part.rest(), |t| t.at(now));
            st.tracks.insert((dust, part), Track { from, start: now, steps: steps(part, dust, random) });
            if part == Part::Clump {
                st.blur_due.push((dust, now + 195.0, if dust { 0.25 } else { 1.0 }));
            }
            if std::env::var_os("BO2MP_RC_SCREEN_LOG").is_some() {
                eprintln!("bo2mp rc screen: {said}");
            }
        }
    }
    // A clump at the middle starts its blur.
    let due: Vec<_> = st.blur_due.iter().copied().filter(|d| d.1 <= now).collect();
    st.blur_due.retain(|d| d.1 > now);
    for (dust, at, _) in due {
        let from = st.tracks.get(&(dust, Part::Blur)).map_or(Part::Blur.rest(), |t| t.at(at));
        st.tracks.insert((dust, Part::Blur), Track { from, start: at, steps: steps(Part::Blur, dust, (0.0, 0.0)) });
    }
    // Done: gone (the random blur rolls its next place as it fades out).
    let ended: Vec<_> = st.tracks.iter().filter(|(_, t)| t.end() <= now).map(|(k, _)| *k).collect();
    for key in ended {
        st.tracks.remove(&key);
        if key.1 == Part::Random {
            st.dice = st.dice.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let x = ((st.dice >> 8) % 200) as f32;
            st.dice = st.dice.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let y = ((st.dice >> 8) % 200) as f32;
            st.random_at.insert(key.0, (x, y));
        }
    }
    let Ok(win) = window.single() else { return };
    if st.tracks.is_empty() {
        return;
    }
    if st.pictures.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for name in ["fullscreen_dirt_bottom", "fullscreen_dirt_bottom_b", "fullscreen_dirt_right", "fullscreen_dirt_left"] {
            if let Some((_, image)) = icons.0.iter().find(|(n, _)| n == name) {
                st.pictures.insert(name, images.add((**image).clone()));
            }
        }
    }
    let (kx, ky) = (win.width() / 640.0, win.height() / 480.0);
    // Menu order: the clump, the splashes, the blurs.
    let order = [Part::Clump, Part::Right, Part::Left, Part::Blur, Part::Random];
    for dust in [false, true] {
        for (i, part) in order.iter().enumerate() {
            let Some(track) = st.tracks.get(&(dust, *part)) else { continue };
            let look = track.at(now);
            let Some(handle) = st.pictures.get(part.picture()) else { continue };
            if look.alpha <= 0.0 {
                continue;
            }
            let [x, y, w, h] = look.rect;
            commands.spawn((
                RcxdScreenNode,
                UiLayer::Hud,
                UiLayerVisibility,
                ImageNode::new(handle.clone())
                    .with_color(Color::srgba(TINT[0], TINT[1], TINT[2], look.alpha))
                    .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
                ZIndex(-2200 + i as i32 + if dust { 10 } else { 0 }),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x * kx),
                    top: Val::Px(y * ky),
                    width: Val::Px(w * kx),
                    height: Val::Px(h * ky),
                    ..default()
                },
            ));
        }
    }
}

pub(crate) fn register(app: &mut App) {
    app.add_systems(Update, update.in_set(ClientSet::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clump_flies_out_and_fades() {
        let t = Track { from: Part::Clump.rest(), start: 0.0, steps: steps(Part::Clump, false, (0.0, 0.0)) };
        assert_eq!(t.at(0.0).alpha, 1.0);
        assert_eq!(t.at(0.0).rect, [320.0, 540.0, 1.0, 1.0]);
        assert_eq!(t.at(195.0).rect, MID);
        assert_eq!(t.at(260.0).alpha, 0.0);
        assert_eq!(t.end(), 260.0);
    }

    #[test]
    fn dust_is_a_quarter() {
        let t = Track { from: Part::Right.rest(), start: 0.0, steps: steps(Part::Right, true, (0.0, 0.0)) };
        assert_eq!(t.at(100.0).alpha, 0.25);
        assert!((t.at(1000.0).alpha - 0.125).abs() < 1e-4);
    }
}
