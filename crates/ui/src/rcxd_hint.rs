//! bo2mp: Black Ops II's RC-XD key hints, in the game's own font. Driving
//! the RC-XD the game shows its `rcbombhints` menu at the bottom left of the
//! screen: "4 Boost" (`PLATFORM_RC_BOMB_BOOST`, his key for the car's boost
//! button, +smoke), "Detonate" (`PLATFORM_RC_BOMB_DETONATE`, +attack) and
//! "Handbrake" (`PLATFORM_RC_BOMB_HANDBRAKE`, +speed_throw), three lines 20
//! apart from 5, -60 of its 640x480 screen, scale 0.315 of its normal font,
//! shadowed; the key in yellow, a mouse button as its picture
//! (`button_left_mouse`, BO2's `^BBUTTON_MOUSE_LEFT^`).

use bevy::prelude::*;
use frame::{ClientStep, OverlayControlSnapshot};
use net::{HostViewClient, ShownWorldFrame};
use std::collections::HashMap;

use crate::{MainWindow, covering_node, pinned_node};
use crate::tiers::{OverlayTier, OverlayStackShown};

/// The menu's lines' centres in its 480-high screen (the real game's shot:
/// 391, 410 and 429 of 455), and their left edge.
const FIRST_Y: f32 = 412.5;
const LINE_STEP: f32 = 20.0;
const LEFT_X: f32 = 5.0;
/// Its text: scale 0.315 of the 48-high normal font, in the 480-high screen.
const FONT_PX: f32 = 0.315 * 48.0;

/// The lines: (localization key, commands whose first bound key fills
/// `&&1`).
const LINES: [(&str, &[&str]); 3] = [
    ("PLATFORM_RC_BOMB_BOOST", &["+smoke"]),
    ("PLATFORM_RC_BOMB_DETONATE", &["+attack"]),
    ("PLATFORM_RC_BOMB_HANDBRAKE", &["+speed_throw", "+toggleads_throw", "+frag"]),
];

#[derive(Component)]
struct RcxdHintRoot {
    shows: String,
}

#[derive(Default)]
struct Pictures(HashMap<&'static str, Handle<Image>>);

/// The picture BO2 draws for a mouse key, and its FontIcon name.
struct MousePicture {
    image: &'static str,
    font_icon: &'static str,
}

fn mouse_picture(key: &str) -> Option<MousePicture> {
    let (image, font_icon) = match key {
        "MOUSE1" => ("button_left_mouse", "BUTTON_MOUSE_LEFT"),
        "MOUSE2" => ("button_right_mouse", "BUTTON_MOUSE_RIGHT"),
        "MOUSE3" => ("button_middle_mouse", "BUTTON_MOUSE_MIDDLE"),
        _ => return None,
    };
    Some(MousePicture { image, font_icon })
}

/// A FontIcon entry's size scales (`bo2mp/fonticon.csv`: name, hash,
/// material, size, xScale, yScale).
fn icon_scale(ui: &assets::T6Ui, name: &str) -> Option<(f32, f32)> {
    let (_, cols, _, cells) = ui.tables.iter().find(|t| t.0 == "bo2mp/fonticon.csv")?;
    let line = cells.chunks((*cols).max(1)).find(|r| r.first().is_some_and(|n| n.eq_ignore_ascii_case(name)))?;
    Some((line.get(4)?.parse().ok()?, line.get(5)?.parse().ok()?))
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    roots: Query<(Entity, &RcxdHintRoot)>,
    window: MainWindow,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    ui: Option<Res<assets::T6Ui>>,
    icons: Option<Res<assets::T6HudIcons>>,
    keys: Option<Res<OverlayControlSnapshot>>,
    fonts: Option<Res<crate::bo2_font::Bo2Fonts>>,
    mut pics: ResMut<Assets<Image>>,
    mut pictures: Local<Pictures>,
) {
    let riding = (|| {
        let (p, l) = (presented.as_deref()?, local.as_deref()?);
        let v = p.snapshot()?.meta.script_dvars(l.0).string("bo2mp_vehicle")?;
        v.split(' ').next().is_some_and(|k| k.to_ascii_lowercase().starts_with("rc_car")).then_some(())
    })()
    .is_some();
    let (true, Some(ui), Some(win), Some(fonts)) = (riding, ui.as_deref(), window.shown(), fonts.as_deref()) else {
        for (e, _) in &roots {
            commands.entity(e).try_despawn();
        }
        return;
    };
    if pictures.0.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for name in ["button_left_mouse", "button_right_mouse", "button_middle_mouse"] {
            if let Some(image) = crate::hud_picture(icons, name) {
                pictures.0.insert(name, pics.add((**image).clone()));
            }
        }
    }
    let keys = keys.as_deref().cloned().unwrap_or_default();
    // Each line: its words before the key, the key, its words after.
    let lines: Vec<(String, String, String)> = LINES
        .iter()
        .filter_map(|(name, commands)| {
            let template = ui.strings.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())?;
            let key = commands
                .iter()
                .find_map(|c| keys.binding_keys.get(*c).cloned())
                .unwrap_or_else(|| "UNBOUND".to_owned());
            let (before, after) = template.split_once("&&1").unwrap_or((template, ""));
            Some((before.to_owned(), key.to_uppercase(), after.to_owned()))
        })
        .collect();
    let scale = win.height() / 480.0;
    let shows = format!(
        "{}|{:.3}|{}|{}",
        lines.iter().map(|(a, k, b)| format!("{a}{k}{b}")).collect::<Vec<_>>().join("/"),
        scale,
        win.width(),
        pictures.0.len()
    );
    if roots.iter().any(|(_, r)| r.shows == shows) {
        return;
    }
    for (e, _) in &roots {
        commands.entity(e).try_despawn();
    }
    let px = FONT_PX * scale;
    let shadow = scale;
    commands
        .spawn((
            RcxdHintRoot { shows },
            OverlayTier::Hud,
            OverlayStackShown,
            covering_node(),
        ))
        .with_children(|outer| {
            for (i, (before, key, after)) in lines.iter().enumerate() {
                let y = (FIRST_Y + LINE_STEP * i as f32) * scale;
                outer.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..pinned_node(LEFT_X * scale, y - px * 0.6)
                })
                .with_children(|line| {
                    let text = |line: &mut ChildSpawnerCommands, t: &str| {
                        let runs = crate::zm_hud::hint_runs_with(t, &keys, false);
                        if !runs.is_empty() {
                            fonts.spawn_line_shadowed(line, "Default", &runs, px, shadow, 1.0);
                        }
                    };
                    match mouse_picture(key).and_then(|p| Some((pictures.0.get(p.image)?.clone(), p.font_icon))) {
                        Some((icon, name)) => {
                            text(line, before);
                            let (xs, ys) = icon_scale(ui, name).unwrap_or((1.0, 1.0));
                            line.spawn((
                                Node {
                                    width: Val::Px(px * xs),
                                    height: Val::Px(px * ys),
                                    ..default()
                                },
                                ImageNode::new(icon),
                            ));
                            text(line, after);
                        }
                        // A key's name keeps the line's colour codes (the
                        // key in yellow).
                        None => text(line, &format!("{before}{key}{after}")),
                    }
                });
            }
        });
}

pub(crate) fn register(host: &mut App) {
    host.add_systems(Update, update.after(crate::bo2_font::load_bo2_fonts).in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_keys_draw_their_pictures() {
        assert_eq!(mouse_picture("MOUSE1").map(|p| p.image), Some("button_left_mouse"));
        assert_eq!(mouse_picture("MOUSE2").map(|p| p.image), Some("button_right_mouse"));
        assert!(mouse_picture("4").is_none());
    }
}
