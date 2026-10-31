//! bo2zm: the solo cheat menu, for recording a map's features. Num 1 opens
//! it and pauses the game (as BO2's pause menu stops a solo game, without
//! showing that menu); Num 1 or Esc closes it. Up / Down (arrows, W / S, the
//! wheel) move the highlight, Enter or a left click picks the line and the
//! menu stays open, so a pick can be repeated. The lines are `sim::Cheat`:
//! the game applies them at its next tick. Special Weapons opens a tab of
//! every special weapon this map has, by name: the one he picks goes in his
//! hands and the menu closes; Esc or Backspace goes back. Only in a local
//! game of one player (`sim::cheats_allowed`).

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use frame::ClientStep;
use sim::Cheat;

use crate::{MainWindow, pinned_node};
use crate::tiers::{OverlayTier, OverlayStackShown};

/// Whether the menu is up, and its highlighted line. `ate_keys`: it used
/// this frame's keys (the Esc that closed it must not open BO2's pause
/// menu).
#[derive(Resource, Default)]
pub(crate) struct CheatMenu {
    pub is_up: bool,
    pub ate_keys: bool,
    highlight: usize,
    /// On the Special Weapons tab, and its highlighted line.
    special: bool,
    special_highlight: usize,
}

/// Lines the Special Weapons tab shows at once; it scrolls past them.
const SPECIAL_ROWS: usize = 14;

#[derive(Component)]
struct CheatRoot;

/// The line `by` lines on from `at`, round the ends.
fn step(at: usize, by: isize, lines: usize) -> usize {
    let n = lines.max(1) as isize;
    (at as isize + by).rem_euclid(n) as usize
}

/// One line as drawn: its label and, for a toggle, ON / OFF.
fn line_text(cheat: Cheat) -> (String, Option<&'static str>) {
    let state = cheat
        .is_toggle()
        .then(|| if sim::cheat_on(cheat) { "ON" } else { "OFF" });
    (cheat.label().to_owned(), state)
}

#[allow(clippy::too_many_arguments)]
fn cheat_menu(
    mut commands: Commands,
    mut menu: ResMut<CheatMenu>,
    (keys, mouse, scroll): (
        Res<ButtonInput<KeyCode>>,
        Res<ButtonInput<MouseButton>>,
        Res<AccumulatedMouseScroll>,
    ),
    (screen, role, view, lui): (
        Res<frame::ShellScreen>,
        Option<Res<frame::ProcessDuty>>,
        Option<Res<frame::OverlayControlSnapshot>>,
        Option<Res<frame::LuiMenus>>,
    ),
    mut paused: ResMut<frame::GamePaused>,
    bo2: Option<Res<crate::bo2_font::Bo2Fonts>>,
    window: MainWindow,
    holders: Query<Entity, With<CheatRoot>>,
    mut drawn: Local<Option<String>>,
) {
    menu.ate_keys = false;
    let allowed = *screen == frame::ShellScreen::InGame
        && role.as_deref() == Some(&frame::ProcessDuty::Listen)
        && sim::cheats_allowed();
    let console = view.as_deref().is_some_and(|v| v.console_open);
    let specials = if menu.special {
        sim::special_weapons()
    } else {
        Vec::new()
    };
    let lines = if menu.special {
        specials.len()
    } else {
        Cheat::ALL.len()
    };
    if menu.is_up && !allowed {
        menu.is_up = false;
        paused.0 = false;
        diag::info!(Ui, "bo2zm cheats: closed, the game ended");
    } else if allowed && !console {
        if !menu.is_up {
            if keys.just_pressed(KeyCode::Numpad1) && !lui.as_deref().is_some_and(|l| l.open) {
                menu.is_up = true;
                menu.special = false;
                menu.ate_keys = true;
                paused.0 = true;
                diag::info!(Ui, "bo2zm cheats: opened, game paused");
            }
        } else {
            menu.ate_keys = true;
            if keys.just_pressed(KeyCode::Numpad1)
                || (keys.just_pressed(KeyCode::Escape) && !menu.special)
            {
                menu.is_up = false;
                menu.special = false;
                paused.0 = false;
                diag::info!(Ui, "bo2zm cheats: closed, game running again");
            } else if menu.special
                && (keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Backspace))
            {
                menu.special = false;
            } else {
                let up = keys.just_pressed(KeyCode::ArrowUp)
                    || keys.just_pressed(KeyCode::KeyW)
                    || scroll.delta.y > 0.0;
                let down = keys.just_pressed(KeyCode::ArrowDown)
                    || keys.just_pressed(KeyCode::KeyS)
                    || scroll.delta.y < 0.0;
                let pick = keys.just_pressed(KeyCode::Enter)
                    || keys.just_pressed(KeyCode::NumpadEnter)
                    || mouse.just_pressed(MouseButton::Left);
                if menu.special {
                    if up {
                        menu.special_highlight = step(menu.special_highlight, -1, lines);
                    }
                    if down {
                        menu.special_highlight = step(menu.special_highlight, 1, lines);
                    }
                    if let Some((pick_name, label)) =
                        specials.get(menu.special_highlight).filter(|_| pick)
                    {
                        sim::special_pick(pick_name);
                        menu.is_up = false;
                        menu.special = false;
                        paused.0 = false;
                        diag::info!(
                            Ui,
                            "bo2zm cheats: gave special weapon {label} ({pick_name}), closed"
                        );
                    }
                } else if up {
                    menu.highlight = step(menu.highlight, -1, lines);
                } else if down {
                    menu.highlight = step(menu.highlight, 1, lines);
                } else if pick && Cheat::ALL[menu.highlight.min(lines - 1)] == Cheat::SpecialWeapon
                {
                    menu.special = true;
                    menu.special_highlight = 0;
                    diag::info!(
                        Ui,
                        "bo2zm cheats: special weapons list ({} on this map)",
                        sim::special_weapons().len()
                    );
                } else if pick {
                    let cheat = Cheat::ALL[menu.highlight.min(lines - 1)];
                    let on = sim::cheat_pick(cheat);
                    if cheat.is_toggle() {
                        let state = if on { "ON" } else { "OFF" };
                        diag::info!(Ui, "bo2zm cheats: chose {} -> {state}", cheat.label());
                    } else {
                        diag::info!(Ui, "bo2zm cheats: chose {}", cheat.label());
                    }
                }
            }
        }
    }
    draw(
        &mut commands,
        &menu,
        &specials,
        bo2.as_deref(),
        &window,
        &holders,
        &mut drawn,
    );
}

/// The menu at the left of the screen in BO2's menu fonts: the title, then
/// a line each, the highlighted one on a bar.
fn draw(
    commands: &mut Commands,
    menu: &CheatMenu,
    specials: &[(String, String)],
    bo2: Option<&crate::bo2_font::Bo2Fonts>,
    window: &MainWindow,
    holders: &Query<Entity, With<CheatRoot>>,
    drawn: &mut Option<String>,
) {
    let sh = window.shown().map_or(1080.0, |w| w.height());
    // The tab: a window of its lines round the highlight, or a line saying
    // this map has none.
    let (title, entries, lit) = if menu.special {
        let first = window_start(menu.special_highlight, specials.len(), SPECIAL_ROWS);
        let mut entries: Vec<(String, Option<&'static str>)> = specials
            .iter()
            .skip(first)
            .take(SPECIAL_ROWS)
            .map(|(_, label)| (label.clone(), None))
            .collect();
        if entries.is_empty() {
            entries.push(("None on this map".to_owned(), None));
        }
        let lit = (!specials.is_empty()).then(|| menu.special_highlight - first);
        ("SPECIAL WEAPONS", entries, lit)
    } else {
        let entries: Vec<_> = Cheat::ALL.iter().map(|c| line_text(*c)).collect();
        ("CHEATS", entries, Some(menu.highlight))
    };
    let sig = format!("{} {title} {lit:?} {sh} {entries:?}", menu.is_up);
    let Some(bo2) = bo2.filter(|_| menu.is_up) else {
        if drawn.is_some() {
            for e in holders {
                commands.entity(e).despawn();
            }
            *drawn = None;
        }
        return;
    };
    if drawn.as_deref() == Some(sig.as_str()) {
        return;
    }
    *drawn = Some(sig);
    for e in holders {
        commands.entity(e).despawn();
    }
    let scale = (sh / 1080.0).max(0.4);
    let (white, grey) = (Color::srgb(1.0, 1.0, 1.0), Color::srgb(0.62, 0.62, 0.62));
    let bar = Color::srgba(0.86, 0.5, 0.12, 0.85);
    commands
        .spawn((
            CheatRoot,
            OverlayTier::Hud,
            OverlayStackShown,
            ZIndex(60),
            Node {
                width: Val::Px(440.0 * scale),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(12.0 * scale)),
                row_gap: Val::Px(2.0 * scale),
                ..pinned_node(64.0 * scale, 240.0 * scale)
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        ))
        .with_children(|c| {
            c.spawn(Node {
                margin: UiRect::bottom(Val::Px(10.0 * scale)),
                ..default()
            })
            .with_children(|t| {
                bo2.spawn_line(
                    t,
                    "Big",
                    &[(title.to_owned(), white)],
                    40.0 * scale,
                    2.0 * scale,
                );
            });
            for (i, (label, shown)) in entries.iter().enumerate() {
                let lit = lit == Some(i);
                let color = if lit { white } else { grey };
                c.spawn((
                    Node {
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(10.0 * scale), Val::Px(3.0 * scale)),
                        ..default()
                    },
                    BackgroundColor(if lit { bar } else { Color::NONE }),
                ))
                .with_children(|cell| {
                    bo2.spawn_line(
                        cell,
                        "Default",
                        &[(label.clone(), color)],
                        26.0 * scale,
                        1.5 * scale,
                    );
                    if let Some(shown) = shown {
                        bo2.spawn_line(
                            cell,
                            "Default",
                            &[((*shown).to_owned(), color)],
                            26.0 * scale,
                            1.5 * scale,
                        );
                    }
                });
            }
        });
}

/// The first line drawn of `len`, `entries` at once, so `at` is in view (kept
/// a line from the edge while there are more).
fn window_start(at: usize, len: usize, entries: usize) -> usize {
    if len <= entries {
        return 0;
    }
    at.saturating_sub(entries / 2).min(len - entries)
}

/// Test aid, BO2MP_CHEAT_KEYS=<key>@<seconds>,...: the menu's keys pressed
/// that long into the game (numpad1, up, down, enter, esc, backspace), so a
/// hidden test game drives the menu by its own keys.
fn cheat_keys_test(
    mut pressed: ResMut<ButtonInput<KeyCode>>,
    on_screen: Res<frame::ShellScreen>,
    time: Res<Time>,
    mut since: Local<Option<f32>>,
    mut held: Local<Vec<KeyCode>>,
    mut done: Local<usize>,
) {
    static WANT: std::sync::OnceLock<Vec<(KeyCode, f32)>> = std::sync::OnceLock::new();
    let want = WANT.get_or_init(|| {
        std::env::var("BO2MP_CHEAT_KEYS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|item| {
                let (k, t) = item.trim().split_once('@')?;
                let key = match k {
                    "numpad1" => KeyCode::Numpad1,
                    "up" => KeyCode::ArrowUp,
                    "down" => KeyCode::ArrowDown,
                    "enter" => KeyCode::Enter,
                    "esc" => KeyCode::Escape,
                    "backspace" => KeyCode::Backspace,
                    _ => return None,
                };
                Some((key, t.trim().parse().ok()?))
            })
            .collect()
    });
    for k in held.drain(..) {
        pressed.release(k);
    }
    if want.is_empty() || *on_screen != frame::ShellScreen::InGame {
        return;
    }
    let start = *since.get_or_insert(time.elapsed_secs());
    let at = time.elapsed_secs() - start;
    while let Some((key, t)) = want.get(*done).filter(|(_, t)| *t <= at) {
        pressed.press(*key);
        held.push(*key);
        diag::info!(Ui, "bo2zm cheats: scripted key {key:?} at {t}s");
        *done += 1;
    }
}

pub(crate) fn register_cheat_menu_systems(host: &mut App) {
    host.init_resource::<CheatMenu>().add_systems(
        Update,
        (cheat_keys_test, cheat_menu).chain().in_set(ClientStep::Ui),
    );
}

#[cfg(test)]
mod tests {
    use super::{step, window_start};

    #[test]
    fn the_special_tab_scrolls_to_keep_the_highlight_in_view() {
        assert_eq!(window_start(0, 10, 14), 0);
        assert_eq!(window_start(9, 10, 14), 0);
        assert_eq!(window_start(0, 30, 14), 0);
        assert_eq!(window_start(10, 30, 14), 3);
        assert_eq!(window_start(29, 30, 14), 16);
    }

    #[test]
    fn the_highlight_goes_round_the_ends() {
        assert_eq!(step(0, -1, 11), 10);
        assert_eq!(step(10, 1, 11), 0);
        assert_eq!(step(3, 1, 11), 4);
        assert_eq!(step(0, 1, 1), 0);
    }
}
