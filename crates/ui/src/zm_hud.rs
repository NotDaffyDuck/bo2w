//! bo2zm M3: the Black Ops II Zombies HUD, drawn over the game while a BO2
//! map runs (its server sends `bo2zm_round`): the round bottom-left in
//! red, the player's points and ammo bottom-right, and in the middle the
//! hint of the use trigger he faces ("Hold [F] for M14 [Cost: 500]", the
//! game's own English text with its colour codes).

use bevy::prelude::*;
use frame::{ClientStep, OverlayControlSnapshot};
use net::{HostViewClient, ShownWorldFrame};

use crate::{ActiveView, MainWindow, SceneCameras, absolute_node, covering_node, pinned_node};
use crate::bo2_font::Bo2Fonts;
use crate::tiers::{MenuTypeface, OverlayTier, OverlayStackShown, match_typeface};

#[derive(Component)]
struct ZmHudRoot;

#[derive(Component)]
struct ZmRound;

#[derive(Component)]
struct ZmPoints;

#[derive(Component)]
struct ZmAmmo;

#[derive(Component)]
struct ZmHint;

/// bo2zm: a line drawn in Black Ops II's own font (0 round, 1 points,
/// 2 ammo, 3 hint), beside the stand-in text it replaces.
#[derive(Component)]
struct ZmLine(u8);

/// The values on screen.
#[derive(Clone, Debug, Default, PartialEq)]
struct ZmHudValues {
    hint: String,
    round: String,
    score: i32,
    clip: i32,
    stock: i32,
    /// A dual-wield weapon's left clip.
    clip_left: Option<i32>,
    /// His grenades and mines (`name:count` by `;`).
    offhand: String,
}

/// What `zm_hud` remembers between frames: the values last drawn, the scale
/// they were drawn at and whether it was in BO2's own font.
#[derive(Default)]
struct HudMemo {
    values: Option<ZmHudValues>,
    scale: f32,
    own_font: bool,
}

/// One of the three plain-text stand-ins (round, points, ammo).
type StandIn<'w, 's, M> = Query<'w, 's, (&'static mut Text, &'static mut TextFont), With<M>>;

/// A clip from the player's clip table (rows of 12 bytes: the weapon, the
/// right hand's count, the left hand's).
fn clip_of(table: &[u8], weapon: i32, hand: usize) -> i32 {
    let word = |row: &[u8], at: usize| i32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]]);
    match table.chunks_exact(12).find(|row| word(row, 0) == weapon) {
        Some(row) => word(row, 4 + hand.min(1) * 4),
        None => 0,
    }
}

const ROUND_RED: Color = Color::srgb(0.62, 0.04, 0.02);
const POINTS_WHITE: Color = Color::srgb(0.95, 0.93, 0.85);

fn shadow() -> TextShadow {
    // Black at 80%, two pixels down and right.
    TextShadow { offset: Vec2::splat(2.0), color: Color::BLACK.with_alpha(0.8), ..default() }
}

// ---- small builders shared by every system below ----

/// Both present-state resources, when both exist.
fn both<'a>(
    snapshot: &'a Option<Res<ShownWorldFrame>>,
    me: &'a Option<Res<HostViewClient>>,
) -> Option<(&'a ShownWorldFrame, &'a HostViewClient)> {
    snapshot.as_deref().zip(me.as_deref())
}

/// A script dvar of the local client's snapshot, as an owned string.
fn dvar_of(shot: &ShownWorldFrame, client: &HostViewClient, dvar: &str) -> Option<String> {
    let snap = shot.snapshot()?;
    let found = snap.meta.script_dvars(client.0).string(dvar).map(str::to_owned);
    found
}

fn dvar_text(
    snapshot: &Option<Res<ShownWorldFrame>>,
    me: &Option<Res<HostViewClient>>,
    key: &str,
) -> Option<String> {
    both(snapshot, me).and_then(|(shot, client)| dvar_of(shot, client, key))
}

/// A picture stretched over its box and tinted.
fn stretched(h: Handle<Image>, tint: Color) -> ImageNode {
    let mode = bevy::ui::widget::NodeImageMode::Stretch;
    ImageNode::new(h).with_mode(mode).with_color(tint)
}

/// The two components every HUD layer entity carries.
fn hud_tag() -> (OverlayTier, OverlayStackShown) {
    (OverlayTier::Hud, OverlayStackShown)
}

/// The bundle every HUD root spawns with.
fn hud_layer<M: Component>(marker: M) -> impl Bundle {
    (hud_tag(), covering_node(), marker)
}

/// An absolute strip at a pixel position with a width and no set height.
fn px_strip(left: f32, top: f32, wide: f32) -> Node {
    Node { width: Val::Px(wide), ..pinned_node(left, top) }
}

/// An absolute box at a pixel position and size.
fn px_rect(left: f32, top: f32, wide: f32, high: f32) -> Node {
    let mut rect = px_strip(left, top, wide);
    rect.height = Val::Px(high);
    rect
}

/// A node as wide and high as `side` pixels.
fn square(side: f32) -> Node {
    Node { width: Val::Px(side), height: Val::Px(side), ..default() }
}

/// Empty text that is filled in later: colour and shadow.
fn label(hue: Color) -> (Text, TextColor, TextShadow) {
    (Text::new(""), TextColor(hue), shadow())
}

/// Percent anchoring to the left or right edge and the bottom.
fn corner(left: Option<f32>, right: Option<f32>, bottom: f32) -> Node {
    let edge = |pct: Option<f32>| pct.map_or(Val::Auto, Val::Percent);
    Node {
        bottom: Val::Percent(bottom),
        left: edge(left),
        right: edge(right),
        ..absolute_node()
    }
}

/// The wide centred box the hint sits in.
fn hint_box() -> Node {
    let mut hint = absolute_node();
    hint.top = Val::Percent(62.0);
    hint.justify_content = JustifyContent::Center;
    hint.width = Val::Percent(100.0);
    hint
}

fn window_size(window: &MainWindow) -> (f32, f32) {
    window.shown().map_or((1920.0, 1080.0), |shown| (shown.width(), shown.height()))
}

fn hud_scale(window: &MainWindow) -> f32 {
    window
        .shown()
        .map_or(1.0, |w| (w.height() / 1080.0).max(0.4))
}

fn clear_roots<M: Component>(commands: &mut Commands, roots: &Query<Entity, With<M>>) {
    roots.iter().for_each(|e| commands.entity(e).despawn());
}

fn keys_or_default(keys: &Option<Res<OverlayControlSnapshot>>) -> OverlayControlSnapshot {
    keys.as_deref().cloned().unwrap_or_default()
}

fn lookup(pictures: &[(String, Handle<Image>)], name: &str) -> Option<Handle<Image>> {
    pictures
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, h)| h.clone())
}

/// One picture of the game's icon set as an image asset, made once.
#[derive(bevy::ecs::system::SystemParam)]
struct OneIcon<'w, 's> {
    icons: Option<Res<'w, assets::T6HudIcons>>,
    pics: ResMut<'w, Assets<Image>>,
    made: Local<'s, Option<Handle<Image>>>,
}

impl OneIcon<'_, '_> {
    /// The icon named `name`, added to the images the first time it is asked for.
    fn get(&mut self, name: &str) -> Option<Handle<Image>> {
        if self.made.is_none() {
            let found = self.icons.as_deref()?.0.iter().find(|(k, _)| k == name).map(|(_, i)| i.clone())?;
            *self.made = Some(self.pics.add((*found).clone()));
        }
        self.made.clone()
    }
}

/// The score popups' layer, their pictures and the popups themselves.
#[derive(bevy::ecs::system::SystemParam)]
struct PopupParts<'w, 's> {
    roots: Query<'w, 's, Entity, With<ZmPopupRoot>>,
    kids: Query<'w, 's, &'static Children>,
    popups: Query<'w, 's, (Entity, &'static ZmPopup, &'static mut Node)>,
    glyphs: Query<'w, 's, &'static mut ImageNode>,
}

/// A BO2 colour code (`^1`..`^9`, `^0`).
fn code_color(c: char) -> Option<Color> {
    let (r, g, b) = match c {
        '0' => (0.0, 0.0, 0.0),
        '1' => (1.0, 0.27, 0.27),
        '2' => (0.4, 1.0, 0.4),
        '3' => (1.0, 0.9, 0.2),
        '4' => (0.35, 0.5, 1.0),
        '5' => (0.3, 0.9, 1.0),
        '6' => (1.0, 0.45, 0.9),
        '7' => (1.0, 1.0, 1.0),
        '8' | '9' => (0.7, 0.7, 0.7),
        _ => return None,
    };
    Some(Color::srgb(r, g, b))
}

/// The key a `[{command}]` hint token stands for.
fn key_for(bind: &str, keys: &OverlayControlSnapshot) -> String {
    if let Some(bound) = keys.binding_keys.get(bind) {
        return bound.clone();
    }
    if matches!(bind, "+activate" | "+usereload")
        && let Some(use_key) = keys.act_key.clone()
    {
        return use_key;
    }
    match bind {
        "+activate" => "F".to_owned(),
        other => other.trim_start_matches('+').to_uppercase(),
    }
}

/// Split a hint into coloured runs; `[{+activate}]`-style key names become
/// the player's own key ("[F]").
pub(crate) fn hint_runs(text: &str, keys: &OverlayControlSnapshot) -> Vec<(String, Color)> {
    hint_runs_with(text, keys, true)
}

/// `hint_runs`; `brackets` false drops the brackets round the key ("Press F
/// to respawn": the PC game's multiplayer prompts show only the letter,
/// coloured).
pub(crate) fn hint_runs_with(text: &str, keys: &OverlayControlSnapshot, brackets: bool) -> Vec<(String, Color)> {
    let mut text = text.to_owned();
    while let Some(start) = text.find("[{") {
        let Some(len) = text[start..].find("}]") else {
            break;
        };
        let key = key_for(&text[start + 2..start + len], keys);
        // bo2zm: a pad button shows as its icon, without brackets (as BO2 shows its pad pictures).
        let icon = key.chars().count() == 1 && key.chars().all(crate::bo2_font::is_button_icon);
        let shown = if brackets && !icon { format!("[{key}]") } else { key };
        text.replace_range(start..start + len + 2, &shown);
    }
    let mut runs: Vec<(String, Color)> = Vec::new();
    let mut hue = Color::WHITE;
    let mut run = String::new();
    let mut glyphs = text.chars().peekable();
    while let Some(glyph) = glyphs.next() {
        // `^` and a colour digit start a new run; any other `^` is drawn.
        let switch = if glyph == '^' { glyphs.peek().copied().and_then(code_color) } else { None };
        let Some(next_color) = switch else {
            run.push(glyph);
            continue;
        };
        glyphs.next();
        if !run.is_empty() {
            runs.push((std::mem::take(&mut run), hue));
        }
        hue = next_color;
    }
    if !run.is_empty() {
        runs.push((run, hue));
    }
    runs
}

fn read_values(shot: &ShownWorldFrame, client: &HostViewClient) -> Option<ZmHudValues> {
    let snap = shot.snapshot()?;
    let dvars = snap.meta.script_dvars(client.0);
    // The scripts hid his HUD (the game-over shot).
    if dvars.string("bo2zm_hud_hidden") == Some("1") {
        return None;
    }
    let text_of = |key: &str| dvars.string(key).unwrap_or_default().to_owned();
    let round = dvars.string("bo2zm_round")?.to_owned();
    let meta = snap.meta.for_client(client.0)?;
    // Both guns held (Black Ops II dual wield): the left one's clip.
    let clip_left = shot
        .player(client.0)
        .filter(|ps| ps.previous_gun_hand == 1)
        .map(|ps| clip_of(&ps.magazine_rows, ps.weapon as i32, 1));
    Some(ZmHudValues {
        hint: text_of("bo2zm_hint"),
        offhand: text_of("bo2zm_offhand"),
        score: meta.score,
        clip: meta.mag_rounds,
        stock: meta.ammo_stock,
        clip_left,
        round,
    })
}

/// The HUD icon of a grenade or mine he carries.
fn offhand_icon(weapon: &str) -> Option<&'static str> {
    let icon = match weapon {
        "frag_grenade_zm" => "hud_us_grenade",
        "sticky_grenade_zm" => "hud_icon_sticky_grenade",
        "cymbal_monkey_zm" => "hud_cymbal_monkey",
        "claymore_zm" => "hud_icon_claymore",
        _ => return None,
    };
    Some(icon)
}

const OFFHAND_WEAPONS: [&str; 4] = ["frag_grenade_zm", "sticky_grenade_zm", "cymbal_monkey_zm", "claymore_zm"];

/// (icon, count) for each grenade or mine in `bo2zm_offhand`.
fn offhand_rows(value: &str) -> Vec<(&'static str, u32)> {
    let mut picks = Vec::new();
    for entry in value.split(';') {
        let Some((weapon, count)) = entry.split_once(':') else {
            continue;
        };
        let count: u32 = count.parse().unwrap_or(0);
        if let Some(icon) = offhand_icon(weapon)
            && count > 0
        {
            picks.push((icon, count));
        }
    }
    picks
}

fn ammo_text(v: &ZmHudValues) -> String {
    let main = format!("{} / {}", v.clip, v.stock);
    v.clip_left.map_or(main.clone(), |left| format!("{left} | {main}"))
}

/// The round's chalk marks (rounds 1 to 5), made once.
fn load_chalk(chalk: &mut Vec<Handle<Image>>, icons: Option<&assets::T6HudIcons>, pics: &mut Assets<Image>) {
    let Some(icons) = icons else { return };
    if !chalk.is_empty() {
        return;
    }
    let marks: Vec<Handle<Image>> = (1..=5)
        .filter_map(|n| {
            let wanted = format!("hud_chalk_{n}");
            let (_, img) = icons.0.iter().find(|(k, _)| *k == wanted)?;
            Some(pics.add((**img).clone()))
        })
        .collect();
    if marks.len() == 5 {
        *chalk = marks;
    }
}

/// The grenade and mine icons, made once.
fn load_offhand(
    loaded: &mut Vec<(String, Handle<Image>)>,
    icons: Option<&assets::T6HudIcons>,
    pics: &mut Assets<Image>,
) {
    let Some(icons) = icons else { return };
    if !loaded.is_empty() {
        return;
    }
    let found = OFFHAND_WEAPONS
        .into_iter()
        .filter_map(offhand_icon)
        .filter_map(|name| {
            let (_, img) = icons.0.iter().find(|(k, _)| *k == name)?;
            Some((name.to_owned(), pics.add((**img).clone())))
        });
    loaded.extend(found);
}

fn set_stand_in<M: Component>(stand_in: &mut StandIn<M>, text: &str, font: Option<TextFont>) {
    stand_in.iter_mut().for_each(|(mut t, mut tf)| {
        *t = Text::new(text);
        if let Some(f) = &font {
            *tf = f.clone();
        }
    });
}

/// The plain-text round, points, ammo and hint, and the four slots for
/// BO2's own font beside them.
fn spawn_stand_ins(commands: &mut Commands, ui_font: &MenuTypeface, ui_scale: f32) {
    let sized = |px: f32| match_typeface(&ui_font.0, px * ui_scale);
    let slot = |n: u8, left: Option<f32>, right: Option<f32>, bottom: f32| {
        // Lines sit on the corner's bottom edge.
        let mut at = corner(left, right, bottom);
        at.align_items = AlignItems::FlexEnd;
        (ZmLine(n), at)
    };
    commands.spawn(hud_layer(ZmHudRoot)).with_children(|p| {
        p.spawn((ZmRound, label(ROUND_RED), sized(84.0), corner(Some(3.5), None, 4.0)));
        p.spawn((ZmPoints, label(POINTS_WHITE), sized(34.0), corner(None, Some(3.5), 14.0)));
        p.spawn((ZmAmmo, label(Color::WHITE), sized(30.0), corner(None, Some(3.5), 6.0)));
        p.spawn((ZmHint, label(Color::WHITE), sized(24.0), TextLayout::justify(Justify::Center), hint_box()));
        // The same four in BO2's own font (filled when it loaded).
        p.spawn(slot(0, Some(3.5), None, 4.0));
        p.spawn(slot(1, None, Some(3.5), 14.0));
        p.spawn(slot(2, None, Some(3.5), 6.0));
        p.spawn((ZmLine(3), hint_box()));
    });
}

/// What one BO2-font line needs to be filled.
struct LineCtx<'a> {
    fonts: &'a Bo2Fonts,
    values: &'a ZmHudValues,
    chalk: &'a [Handle<Image>],
    offhand_imgs: &'a [(String, Handle<Image>)],
    keys: &'a OverlayControlSnapshot,
    scale: f32,
    lui: bool,
}

fn put_line(
    commands: &mut Commands,
    e: Entity,
    cx: &LineCtx,
    face: &str,
    runs: &[(String, Color)],
    (size, edge): (f32, f32),
) {
    commands.entity(e).with_children(|c| {
        cx.fonts.spawn_line(c, face, runs, size * cx.scale, edge * cx.scale);
    });
}

/// One group of grenade icons, each fainter than the one before.
fn offhand_group(c: &mut ChildSpawnerCommands, icon: &Handle<Image>, n: u32, ui_scale: f32) {
    let group = Node {
        column_gap: Val::Px(2.0 * ui_scale),
        align_self: AlignSelf::Center,
        margin: UiRect::right(Val::Px(16.0 * ui_scale)),
        ..default()
    };
    c.spawn(group).with_children(|g| {
        for i in 0..n {
            let fade = 0.75f32.powi(i as i32);
            let tint = Color::srgba(1.0, 1.0, 1.0, fade);
            g.spawn((square(34.0 * ui_scale), ImageNode::new(icon.clone()).with_color(tint)));
        }
    });
}

/// Fill one of the four BO2-font slots.
fn fill_bo2_line(commands: &mut Commands, e: Entity, kind: u8, cx: &LineCtx) {
    let scale = cx.scale;
    let round: u32 = cx.values.round.parse().unwrap_or(0);
    commands.entity(e).despawn_related::<Children>();
    match kind {
        0 | 1 if cx.lui => {}
        0 if (1..=5).contains(&round) && cx.chalk.len() == 5 => {
            let mark = ImageNode {
                color: ROUND_RED,
                image: cx.chalk[round as usize - 1].clone(),
                ..default()
            };
            commands.entity(e).with_child((square(150.0 * scale), mark));
        }
        0 => {
            let runs = [(cx.values.round.clone(), ROUND_RED)];
            put_line(commands, e, cx, "Morris", &runs, (110.0, 3.0));
        }
        1 => {
            let runs = [(cx.values.score.to_string(), POINTS_WHITE)];
            put_line(commands, e, cx, "Default", &runs, (46.0, 2.0));
        }
        2 => {
            // BO2's offhand icons left of the ammo: one per grenade,
            // each one fainter (offhandicons.lua), a group per kind.
            let wanted = if cx.lui { "" } else { cx.values.offhand.as_str() };
            let groups: Vec<(Handle<Image>, u32)> = offhand_rows(wanted)
                .into_iter()
                .filter_map(|(icon, n)| {
                    let (_, h) = cx.offhand_imgs.iter().find(|(k, _)| k == icon)?;
                    Some((h.clone(), n.min(4)))
                })
                .collect();
            let runs = [(ammo_text(cx.values), Color::WHITE)];
            commands.entity(e).with_children(|c| {
                groups.iter().for_each(|(h, n)| offhand_group(c, h, *n, scale));
                if !cx.lui {
                    cx.fonts.spawn_line(c, "Big", &runs, 44.0 * scale, 2.0 * scale);
                }
            });
        }
        _ => {
            let runs = hint_runs(&cx.values.hint, cx.keys);
            put_line(commands, e, cx, "Default", &runs, (40.0, 2.0));
        }
    }
}

fn zm_hud(
    mut commands: Commands,
    window: MainWindow,
    (roots, hint, lines): (
        Query<Entity, With<ZmHudRoot>>,
        Query<Entity, With<ZmHint>>,
        Query<(Entity, &ZmLine)>,
    ),
    (snapshot, me, keys, ui_font): (
        Option<Res<ShownWorldFrame>>,
        Option<Res<HostViewClient>>,
        Option<Res<OverlayControlSnapshot>>,
        Option<Res<MenuTypeface>>,
    ),
    mut stand_ins: ParamSet<(StandIn<ZmRound>, StandIn<ZmPoints>, StandIn<ZmAmmo>)>,
    mut memo: Local<HudMemo>,
    (bo2, icons, mut pics, mut chalk, mut offhand_imgs): (
        Option<Res<Bo2Fonts>>,
        Option<Res<assets::T6HudIcons>>,
        ResMut<Assets<Image>>,
        Local<Vec<Handle<Image>>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
) {
    let values = both(&snapshot, &me).and_then(|(shot, client)| read_values(shot, client));
    load_chalk(&mut chalk, icons.as_deref(), &mut pics);
    load_offhand(&mut offhand_imgs, icons.as_deref(), &mut pics);
    let Some(values) = values else {
        clear_roots(&mut commands, &roots);
        memo.values = None;
        return;
    };
    let Some(ui_font) = ui_font else { return };
    let scale = hud_scale(&window);
    if roots.is_empty() {
        spawn_stand_ins(&mut commands, &ui_font, scale);
        memo.values = None;
        return;
    }
    let own_font = bo2.is_some();
    let rescale = (memo.scale - scale).abs() >= 1e-3 || memo.own_font != own_font;
    if !rescale && memo.values.as_ref() == Some(&values) {
        return;
    }
    memo.scale = scale;
    memo.own_font = own_font;
    let keys = keys_or_default(&keys);
    // bo2zm: Black Ops II's own fonts (`ui/t6/codbase.lua`'s names): the
    // round in Morris (rounds 1 to 5 as the chalk marks), the points in
    // Default, the ammo in Big, the hint in Default.
    if let Some(fonts) = bo2.as_deref() {
        set_stand_in(&mut stand_ins.p0(), "", None);
        set_stand_in(&mut stand_ins.p1(), "", None);
        set_stand_in(&mut stand_ins.p2(), "", None);
        for e in &hint {
            commands.entity(e).despawn_related::<Children>();
            let blank = (TextSpan::new(""), match_typeface(&ui_font.0, 24.0 * scale));
            commands.entity(e).with_child(blank);
        }
        // bo2zm M4: BO2's own HUD (unless BO2ZM_LUI=0) draws the round, the points
        // and the ammo.
        let cx = LineCtx {
            fonts,
            values: &values,
            chalk: &chalk,
            offhand_imgs: &offhand_imgs,
            keys: &keys,
            scale,
            lui: crate::lui_hud::lui_enabled(),
        };
        lines.iter().for_each(|(e, line)| fill_bo2_line(&mut commands, e, line.0, &cx));
        memo.values = Some(values);
        return;
    }
    lines.iter().for_each(|(e, _)| {
        commands.entity(e).despawn_related::<Children>();
    });
    let refont = |px: f32| rescale.then(|| match_typeface(&ui_font.0, px * scale));
    set_stand_in(&mut stand_ins.p0(), &values.round, refont(84.0));
    set_stand_in(&mut stand_ins.p1(), &values.score.to_string(), refont(34.0));
    set_stand_in(&mut stand_ins.p2(), &ammo_text(&values), refont(30.0));
    let hint_changed = rescale || memo.values.as_ref().is_none_or(|s| s.hint != values.hint);
    if hint_changed {
        if std::env::var_os("BO2MP_T6_HINTLOG").is_some() {
            diag::info!(World, "zombies hud: hint on screen {:?}", values.hint);
        }
        let mut runs = hint_runs(&values.hint, &keys);
        // An empty hint still gets one (empty) span: with every span
        // removed and none added the text is not laid out again and the
        // old hint stays on screen (seen: the box's hint after it left).
        if runs.is_empty() {
            runs.push((String::new(), Color::WHITE));
        }
        let span_font = match_typeface(&ui_font.0, 24.0 * scale);
        for e in &hint {
            commands.entity(e).despawn_related::<Children>();
            for (run, color) in &runs {
                commands
                    .entity(e)
                    .with_child((TextColor(*color), span_font.clone(), TextSpan::new(run.clone())));
            }
        }
    }
    memo.values = Some(values);
}

#[derive(Component)]
struct ZmScriptHudRoot;

/// One script HUD element as the server sends it (`bo2zm_hud`).
#[derive(Clone, Debug, PartialEq)]
struct ScriptElem {
    x: f32,
    y: f32,
    alignx: String,
    aligny: String,
    horzalign: String,
    vertalign: String,
    fontscale: f32,
    rgba: [f32; 4],
    sort: i32,
    body: String,
    /// bo2mp: hidden while his menus are open (`hidewheninmenu`).
    hide_in_menu: bool,
    /// bo2mp: the element's font name (`default`, `extrabig`, ...).
    font_key: String,
}

/// One row of `bo2zm_hud`: fields split by the unit separator.
fn parse_script_elem(row: &str) -> Option<ScriptElem> {
    let f: Vec<&str> = row.split('\u{1f}').collect();
    if f.len() < 13 {
        return None;
    }
    let num = |i: usize| f[i].parse::<f32>().unwrap_or(0.0);
    let owned = |i: usize| f[i].to_owned();
    Some(ScriptElem {
        rgba: [num(7), num(8), num(9), num(10)],
        body: owned(12),
        sort: f[11].parse().unwrap_or(0),
        x: num(0),
        y: num(1),
        alignx: owned(2),
        aligny: owned(3),
        horzalign: owned(4),
        vertalign: owned(5),
        fontscale: num(6),
        hide_in_menu: matches!(f.get(13), Some(&"m")),
        font_key: f.get(14).copied().unwrap_or("default").to_owned(),
    })
}

fn parse_script_hud(s: &str) -> Vec<ScriptElem> {
    let mut out: Vec<ScriptElem> = s.split('\u{1e}').filter_map(parse_script_elem).collect();
    out.sort_by_key(|e| e.sort);
    out
}

/// What `zm_script_hud` drew last, so an unchanged screen is left alone.
#[derive(Default, PartialEq)]
struct ScriptMemo {
    raw: String,
    size: (f32, f32),
    /// BO2's font loaded, menus open, dimmed out, on the world camera.
    flags: [bool; 4],
}

/// Pictures the scripts put up whose names start like this.
const PICTURE_PREFIXES: [&str; 6] = ["overlay_", "progress_bar_", "damage_feedback", "faction_", "additive:faction_", "perk_"];

/// The screen as one script element sees it.
struct ScriptView<'a> {
    sw: f32,
    sh: f32,
    kx: f32,
    ky: f32,
    /// The outcome screen's safe-area margins in pixels.
    margin: (f32, f32),
    outcome: bool,
    in_mp: bool,
    encoded: bool,
    fonts: Option<&'a Bo2Fonts>,
    keys: &'a OverlayControlSnapshot,
    ui_font: &'a MenuTypeface,
}

impl ScriptView<'_> {
    /// bo2mp: on the encoded world camera the numbers go in as the
    /// script wrote them (the same as lui_hud.rs's enc_ui colours).
    fn tone(&self, c: Color) -> Color {
        if !self.encoded {
            return c;
        }
        let s = c.to_srgba();
        Color::linear_rgba(s.red, s.green, s.blue, s.alpha)
    }
}

/// Where an element goes: its point, its text size and font face.
struct Placement {
    x: f32,
    y: f32,
    px: f32,
    face: &'static str,
    key_prompt: bool,
}

fn horz_base(align: &str) -> f32 {
    match align {
        "center" | "user_center" | "center_safearea" => 320.0,
        "right" | "user_right" | "right_adjustable" => 640.0,
        _ => 0.0,
    }
}

fn base_vertex_at(align: &str) -> f32 {
    match align {
        "middle" | "user_middle" | "center" | "center_safearea" => 240.0,
        "bottom" | "user_bottom" | "bottom_adjustable" => 480.0,
        _ => 0.0,
    }
}

fn place(e: &ScriptElem, v: &ScriptView) -> Placement {
    // bo2mp: a key prompt in a match ("Press F to respawn") is
    // three quarters of the plain size, without brackets
    // (measured against real/55: 60 px against 80 for "Press").
    let key_prompt = v.in_mp && e.body.contains("[{+");
    let shrink = if key_prompt { 0.75 } else { 1.0 };
    let mut at = Placement {
        px: 18.0 * e.fontscale.max(0.1) * v.ky * shrink,
        x: (horz_base(&e.horzalign) + e.x) * v.kx,
        y: (base_vertex_at(&e.vertalign) + e.y) * v.ky,
        face: "Default",
        key_prompt,
    };
    if !v.outcome {
        return at;
    }
    // bo2mp: the outcome screen: the engine's placement, font and
    // size (`hudelem_font`); px is then the font's line height.
    at.x = match e.horzalign.as_str() {
        "center" | "user_center" | "center_safearea" => v.sw * 0.5 + e.x * v.ky,
        "right" | "user_right" | "right_adjustable" => v.sw - v.margin.0 + e.x * v.ky,
        "left" | "user_left" | "left_adjustable" => v.margin.0 + e.x * v.ky,
        _ => e.x * v.sw / 640.0,
    };
    at.y = match e.vertalign.as_str() {
        "middle" | "user_middle" | "center" | "center_safearea" => v.sh * 0.5 + e.y * v.ky,
        "bottom" | "user_bottom" | "bottom_adjustable" => v.sh - v.margin.1 + e.y * v.ky,
        "top" | "user_top" | "top_adjustable" => v.margin.1 + e.y * v.ky,
        _ => e.y * v.sh / 480.0,
    };
    (at.face, at.px) = hudelem_font(&e.font_key, e.fontscale, v.ky);
    at
}

/// A wide box the text is justified in, so alignx anchors it.
fn text_left(alignx: &str, x: f32, width: f32) -> (f32, Justify) {
    match alignx {
        "center" => (x - width * 0.5, Justify::Center),
        "right" => (x - width, Justify::Right),
        _ => (x, Justify::Left),
    }
}

fn text_top(outcome: bool, aligny: &str, y: f32, px: f32) -> f32 {
    // On the outcome screen the engine's box is the font's line height tall,
    // its top whole pixels.
    if outcome {
        return match aligny {
            "middle" => (y - px * 0.5).trunc(),
            "bottom" => (y - px).trunc(),
            _ => y.trunc(),
        };
    }
    match aligny {
        "middle" => y - px * 0.6,
        "bottom" => y - px * 1.2,
        _ => y,
    }
}

fn box_justify(j: Justify) -> JustifyContent {
    match j {
        Justify::Center => JustifyContent::Center,
        Justify::Right => JustifyContent::FlexEnd,
        _ => JustifyContent::FlexStart,
    }
}

/// A picture element: BO2's plain black/white fills (the game-over fade),
/// the game's own pictures, or a team icon added onto the world.
fn draw_shader(
    p: &mut ChildSpawnerCommands,
    e: &ScriptElem,
    spec: &str,
    at: &Placement,
    hue: Color,
    v: &ScriptView,
    pictures: &[(String, Handle<Image>)],
    additive: &mut Assets<crate::lui_additive::AdditiveUi>,
) {
    let mut parts = spec.split(':');
    let name = parts.next().unwrap_or_default();
    let mut dim = || {
        parts
            .next()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0)
    };
    let (pic_w, pic_h) = (dim(), dim());
    // bo2zm M3 fix list 2: or one of the game's own pictures:
    // the red overlay the health script fades in when he is
    // hit ("my screen should get red"); it was skipped.
    let picture = lookup(pictures, name);
    // bo2zm (Buried 10-09): the build progress bar (_hud_util createbar: progress_bar_bg behind,
    // progress_bar_fill growing); a plain fill in its colour if the picture did not load.
    let fill = match name {
        "black" => Some(Color::srgba(0.0, 0.0, 0.0, e.rgba[3])),
        "white" => Some(hue),
        _ if name.starts_with("progress_bar_") && picture.is_none() => Some(hue),
        _ => None,
    };
    if fill.is_none() && picture.is_none() {
        return;
    }
    let node = if e.horzalign == "fullscreen" && e.vertalign == "fullscreen" {
        px_rect(0.0, 0.0, v.sw, v.sh)
    } else {
        // bo2mp: anchored by alignx/aligny as BO2 does (the
        // outcome screen's team icons are centred on their
        // point).
        let (w, h) = (pic_w * v.kx, pic_h * v.ky);
        let share_x = match e.alignx.as_str() {
            "center" => 0.5,
            "right" => 1.0,
            _ => 0.0,
        };
        let share_y = match e.aligny.as_str() {
            "middle" => 0.5,
            "bottom" => 1.0,
            _ => 0.0,
        };
        px_rect(at.x - w * share_x, at.y - h * share_y, w, h)
    };
    let layer = ZIndex(e.sort - 1000);
    // bo2mp: the team icons are ONE/ONE pictures in BO2 (their
    // glow is in the colours, on black): added onto the world,
    // not alpha-blended (a bright world still takes the glow).
    if fill.is_none()
        && let Some(pic) = lookup(pictures, &format!("additive:{name}"))
    {
        let material = crate::lui_additive::AdditiveUi {
            picture: pic,
            tint: hue.to_linear(),
            uv: Vec4::new(0.0, 0.0, 1.0, 1.0),
            color_add: 0.0,
            encoded: f32::from(u8::from(v.encoded)),
        };
        p.spawn((node, layer, MaterialNode(additive.add(material))));
        return;
    }
    match (fill, picture) {
        (Some(fill), _) => {
            p.spawn((node, layer, BackgroundColor(fill)));
        }
        (None, Some(h)) => {
            p.spawn((node, layer, stretched(h, hue)));
        }
        (None, None) => {}
    }
}

/// bo2mp: a perk list (`setperks`: the spawn list, the killer
/// card): rows of `icon|name`, each a name right-aligned
/// beside its picture, the rows one picture-height apart,
/// stacked from the element's point.
fn draw_perks(
    p: &mut ChildSpawnerCommands,
    e: &ScriptElem,
    rows: &str,
    at: &Placement,
    hue: Color,
    v: &ScriptView,
    pictures: &[(String, Handle<Image>)],
) {
    // Measured against the real spawn list (50b, 1280x720):
    // the picture is 32 high (48 px), its right edge 2 px
    // further right and its middle 6 px above the name's.
    let (size, tsize, step) = (32.0 * v.ky, 30.0 * v.ky, 32.0 * v.ky);
    let rows = rows.split(';').filter(|r| !r.is_empty());
    for (i, row) in rows.enumerate() {
        let (icon, name) = row.split_once('|').unwrap_or(("", row));
        let top = at.y - 123.0 * v.ky + i as f32 * step;
        if let Some(h) = lookup(pictures, icon) {
            let tint = v.tone(Color::srgba(1.0, 1.0, 1.0, e.rgba[3]));
            let image = stretched(h, tint);
            let at_left = at.x + 12.0 * v.kx + 2.0 - size;
            p.spawn((ZIndex(e.sort), image, px_rect(at_left, top - 7.5 * v.ky / 1.5, size, size)));
        }
        let Some(fonts) = v.fonts else { continue };
        let runs = vec![(name.to_owned(), hue)];
        let name_box = Node {
            justify_content: JustifyContent::FlexEnd,
            ..px_strip(
                at.x + 12.0 * v.kx + 1.0 - tsize - 12.0 - v.sw,
                top + tsize * 0.5 - at.px * 0.5 + 3.0 * v.ky / 1.5,
                v.sw,
            )
        };
        p.spawn((ZIndex(e.sort), name_box)).with_children(|c| {
            fonts.spawn_line(c, "Default", &runs, at.px * 0.72, at.px * 0.05);
        });
    }
}

/// A text element, in BO2's own font when it loaded.
fn draw_text(p: &mut ChildSpawnerCommands, e: &ScriptElem, at: &Placement, hue: Color, v: &ScriptView) {
    let (left, justify) = text_left(&e.alignx, at.x, v.sw);
    let top = text_top(v.outcome, &e.aligny, at.y, at.px);
    let spot = px_strip(left, top, v.sw);
    // bo2zm: in BO2's own font when it loaded.
    if let Some(fonts) = v.fonts {
        let runs: Vec<(String, Color)> = hint_runs_with(&e.body, v.keys, !at.key_prompt)
            .into_iter()
            .map(|(run, c)| {
                let shown = if c == Color::WHITE { hue } else { v.tone(c).with_alpha(e.rgba[3]) };
                (run, shown)
            })
            .collect();
        let holder = Node { justify_content: box_justify(justify), ..spot };
        p.spawn((ZIndex(e.sort), holder)).with_children(|c| {
            if v.outcome {
                // The engine: the em is the line height, the
                // drop shadow one font pixel down and right in
                // black at the text's own alpha.
                let edge = at.px / fonts.raster_rows(at.face).max(1.0);
                fonts.spawn_line_shadowed(c, at.face, &runs, at.px, edge, 1.0);
            } else {
                // His "a little bit too big": three quarters of
                // the size this drew at before.
                fonts.spawn_line(c, "Default", &runs, at.px * 0.94, at.px * 0.06);
            }
        });
        return;
    }
    let spans = hint_runs_with(&e.body, v.keys, !at.key_prompt);
    p.spawn((label(hue), match_typeface(&v.ui_font.0, at.px), TextLayout::justify(justify), spot))
        .with_children(|t| {
            spans.into_iter().for_each(|(run, c)| {
                let c = if c == Color::WHITE { hue } else { c.with_alpha(e.rgba[3]) };
                t.spawn((TextColor(c), TextSpan::new(run), match_typeface(&v.ui_font.0, at.px)));
            });
        });
}

fn draw_script_elem(
    p: &mut ChildSpawnerCommands,
    e: &ScriptElem,
    v: &ScriptView,
    pictures: &[(String, Handle<Image>)],
    additive: &mut Assets<crate::lui_additive::AdditiveUi>,
) {
    let at = place(e, v);
    let color = v.tone(Color::srgba(e.rgba[0], e.rgba[1], e.rgba[2], e.rgba[3]));
    if let Some(spec) = e.body.strip_prefix("#shader:") {
        draw_shader(p, e, spec, &at, color, v, pictures, additive);
    } else if let Some(rows) = e.body.strip_prefix("#perks:") {
        draw_perks(p, e, rows, &at, color, v, pictures);
    } else {
        draw_text(p, e, &at, color, v);
    }
}

/// The scripts' own HUD elements (Game Over, "you survived N rounds", Max
/// Ammo...) where BO2 puts them: a 640x480 screen, x/y from the element's
/// horizontal/vertical alignment base, the text anchored by alignx/aligny.
fn zm_script_hud(
    mut commands: Commands,
    window: MainWindow,
    holders: Query<Entity, With<ZmScriptHudRoot>>,
    (snapshot, me): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    (keys, ui_font, bo2): (Option<Res<OverlayControlSnapshot>>, Option<Res<MenuTypeface>>, Option<Res<Bo2Fonts>>),
    // bo2mp: BO2's menus open (its `hidewheninmenu` elements hide).
    (menus, menu_dims): (Option<Res<frame::LuiMenus>>, Option<Res<frame::MenuDims>>),
    mut memo: Local<ScriptMemo>,
    (icons, mut images, mut pictures, mut additive, world_cam): (
        Option<Res<assets::T6HudIcons>>,
        ResMut<Assets<Image>>,
        Local<Vec<(String, Handle<Image>)>>,
        ResMut<Assets<crate::lui_additive::AdditiveUi>>,
        crate::CompositedWorldCameras,
    ),
) {
    // The game's own pictures the scripts put up (the red low-health
    // overlay, the build progress bar), again whenever the loaded pictures change: the front end
    // loads its own first (Create-a-Class's perk icons), and a match from
    // the menus has to swap in the match's (its hit marker).
    if let Some(icons) = icons.as_ref()
        && icons.is_changed()
    {
        // (bo2mp: and the hit marker, `damage_feedback`; the teams' icons
        // on the outcome screen, `faction_*`.)
        let wanted = icons
            .0
            .iter()
            .filter(|(n, _)| PICTURE_PREFIXES.iter().any(|pre| n.starts_with(*pre)));
        let fresh: Vec<(String, Handle<Image>)> = wanted
            .map(|(stem, pic)| (stem.clone(), images.add((**pic).clone())))
            .collect();
        *pictures = fresh;
    }
    let Some(value) = dvar_text(&snapshot, &me, "bo2zm_hud") else {
        clear_roots(&mut commands, &holders);
        *memo = ScriptMemo::default();
        return;
    };
    let Some(ui_font) = ui_font else { return };
    let menus_open = menus.as_deref().is_some_and(|l| l.open);
    // bo2mp: two menu dims stacked (a popup over the pause menu) dim the whole
    // HUD out, as in `sync_hud_visibility`: what the script HUD draws above the
    // menus (the respawn prompt) goes with it.
    let dimmed_out = menu_dims.as_deref().is_some_and(|d| d.0 >= 2);
    // bo2mp: a multiplayer match (its key prompts are the PC game's).
    let in_mp = dvar_text(&snapshot, &me, "bo2mp_gametype").is_some();
    // bo2mp: BO2 blends the match's script HUD (the outcome screen's team
    // pictures are added) onto the frame in the frame's own (encoded) space:
    // on the match world's camera when there is one, as the LUI match HUD and
    // the sight reticle are drawn.
    let lens = world_cam.first().filter(|_| in_mp);
    let (sw, sh) = window_size(&window);
    let now = ScriptMemo {
        raw: value.clone(),
        size: (sw, sh),
        flags: [bo2.is_some(), menus_open, dimmed_out, lens.is_some()],
    };
    if !holders.is_empty() && *memo == now {
        return;
    }
    *memo = now;
    clear_roots(&mut commands, &holders);
    let keys = keys_or_default(&keys);
    let shown = |e: &ScriptElem| !dimmed_out && !(menus_open && e.hide_in_menu);
    let elems: Vec<ScriptElem> = parse_script_hud(&value).into_iter().filter(shown).collect();
    // bo2mp: the match-end outcome screen (title, text, the teams' pictures
    // and scores; the pictures name it) is placed as BO2's engine places a
    // hudelem: 480 virtual units high, scaled the same both ways, its
    // top/bottom/left/right aligns inside the safe area (see `OUTCOME_SAFE`).
    let outcome = elems.iter().any(|e| e.body.starts_with("#shader:faction_"));
    let ky = sh / 480.0;
    // The safe area's margins in pixels: floor(share * size + 0.5).
    let margin_of = |extent: f32| (OUTCOME_SAFE * extent + 0.5).floor();
    let view = ScriptView {
        sw,
        sh,
        ky,
        kx: if outcome { ky } else { sw / 640.0 },
        margin: (margin_of(sw), margin_of(sh)),
        outcome,
        in_mp,
        encoded: lens.is_some(),
        fonts: bo2.as_deref(),
        keys: &keys,
        ui_font: &ui_font,
    };
    let mut root = commands.spawn(hud_layer(ZmScriptHudRoot));
    if let Some(cam) = lens {
        root.insert(bevy::ui::UiTargetCamera(cam));
    }
    root.with_children(|p| {
        elems
            .iter()
            .for_each(|e| draw_script_elem(p, e, &view, &pictures, &mut additive));
    });
}

/// bo2mp: the safe area's margin on each side as a share of the screen:
/// (1 - 0.85) / 2, the engine family's `safeArea` default of 0.85.
/// Measured: the real 1280x720 DEFEAT screen (ref row 60) puts the
/// title's box at 99 px = (54 + 30 * 1.5), 54 = floor(0.075 * 720 + 0.5).
const OUTCOME_SAFE: f32 = (1.0 - 0.85) / 2.0;

/// bo2mp: a text hudelem's font and line height in pixels as BO2's engine
/// picks them (the cg_hudelem rules): the real scale is `fontscale` times
/// the font's base (a quarter; bigfixed a half, smallfixed a third) times
/// the screen's height over 480, and the line is 48 times that. `extrabig`
/// is the extra-big font (Morris), `big` the big one, `small` and
/// `smallfixed` the condensed one, `objective` the normal one; `default`
/// takes the small font up to a scale of 0.25 (ui_smallFont), the big one
/// from 0.4 (ui_bigFont), the normal one between.
fn hudelem_font(font: &str, fontscale: f32, ky: f32) -> (&'static str, f32) {
    let base = match font {
        "bigfixed" => 0.5,
        "smallfixed" => 1.0 / 3.0,
        _ => 0.25,
    };
    let scale = fontscale.max(0.0) * base * ky;
    let by_size = |a: f32| match a {
        a if a <= 0.25 => "Condensed",
        a if a >= 0.4 => "Big",
        _ => "Default",
    };
    let face = match font {
        "extrabig" => "Morris",
        "big" | "bigfixed" => "Big",
        "small" | "smallfixed" => "Condensed",
        "objective" => "Default",
        _ => by_size(scale * ky),
    };
    (face, (48.0 * scale + 0.5).floor())
}

#[derive(Component)]
struct ZmIconsRoot;

/// The game's icon for a player client field (`setclientfieldtoplayer`):
/// the perks he has, the power-ups running.
fn icon_material(field: &str) -> Option<&'static str> {
    let pic_name = match field {
        "perk_juggernaut" => "specialty_juggernaut_zombies",
        "perk_quick_revive" => "specialty_quickrevive_zombies",
        "perk_sleight_of_hand" => "specialty_fastreload_zombies",
        "perk_double_tap" => "specialty_doubletap_zombies",
        "powerup_instant_kill" => "specialty_instakill_zombies",
        "powerup_double_points" => "specialty_doublepoints_zombies",
        "powerup_fire_sale" => "specialty_firesale_zombies",
        _ => return None,
    };
    Some(pic_name)
}

/// Perk icons in a row above the round, in the order he got them (a paused
/// perk dimmed); running power-ups centred at the bottom (a power-up about
/// to end blinks: the server's 2 is its dark beat).
fn zm_icons(
    mut commands: Commands,
    window: MainWindow,
    holders: Query<Entity, With<ZmIconsRoot>>,
    (snapshot, me): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    mut memo: Local<(String, f32)>,
    mut handles: Local<Vec<(String, Handle<Image>)>>,
    (icons, mut images): (Option<Res<assets::T6HudIcons>>, ResMut<Assets<Image>>),
) {
    // BO2's own HUD (unless BO2ZM_LUI=0) draws the perks and power-ups.
    let value = both(&snapshot, &me)
        .filter(|_| !crate::lui_hud::lui_enabled())
        .and_then(|(shot, client)| {
            if dvar_of(shot, client, "bo2zm_hud_hidden").as_deref() == Some("1") {
                return None;
            }
            // bo2zm: only the perks and power-ups (Mob's Afterlife mana, also on the list, steps
            // every 0.05 s).
            let icons = dvar_of(shot, client, "bo2zm_icons")?;
            let kept: Vec<&str> = icons
                .split(',')
                .filter(|e| e.starts_with("perk_") || e.starts_with("powerup_"))
                .collect();
            Some(kept.join(","))
        });
    let Some(value) = value else {
        clear_roots(&mut commands, &holders);
        *memo = (String::new(), 0.0);
        return;
    };
    if let Some(icons) = icons.as_deref()
        && handles.is_empty()
    {
        let made = icons
            .0
            .iter()
            .map(|(stem, pic)| (stem.clone(), images.add((**pic).clone())))
            .collect::<Vec<_>>();
        handles.extend(made);
    }
    let scale = hud_scale(&window);
    if !holders.is_empty() && memo.0 == value && (memo.1 - scale).abs() < 1e-3 {
        return;
    }
    *memo = (value.clone(), scale);
    clear_roots(&mut commands, &holders);
    let handle = |m: &str| lookup(&handles, m);
    let found: Vec<_> = value
        .split(',')
        .filter_map(|entry| {
            let (field, raw) = entry.split_once(':')?;
            let h = icon_material(field).and_then(|m| handle(m))?;
            Some((field.starts_with("perk_"), raw.parse::<i32>().unwrap_or(0), h))
        })
        .collect();
    // A perk at state 2 is dimmed; a powerup at state 2 is not shown.
    let perks: Vec<_> = found.iter().filter(|(perk, ..)| *perk).map(|(_, v, h)| (h.clone(), if *v == 2 { 0.4 } else { 1.0 })).collect();
    let powerups: Vec<_> = found.iter().filter(|(perk, v, _)| !perk && *v != 2).map(|(.., h)| h.clone()).collect();
    let strip = |left_pct: Option<f32>, bottom_pct: f32| {
        let (left, width, justify_content) = match left_pct {
            Some(pct) => (Val::Percent(pct), Val::Auto, JustifyContent::Start),
            None => (Val::Auto, Val::Percent(100.0), JustifyContent::Center),
        };
        Node {
            left,
            width,
            justify_content,
            bottom: Val::Percent(bottom_pct),
            column_gap: Val::Px(8.0 * scale),
            ..absolute_node()
        }
    };
    commands.spawn(hud_layer(ZmIconsRoot)).with_children(|p| {
        p.spawn(strip(Some(3.5), 15.0)).with_children(|r| {
            perks.into_iter().for_each(|(h, alpha)| {
                let tint = Color::srgba(1.0, 1.0, 1.0, alpha);
                r.spawn((square(52.0 * scale), ImageNode::new(h).with_color(tint)));
            });
        });
        p.spawn(strip(None, 7.0)).with_children(|r| {
            powerups.into_iter().for_each(|h| {
                r.spawn((square(72.0 * scale), ImageNode::new(h)));
            });
        });
    });
}

#[derive(Component)]
struct ZmPopupRoot;

/// A points popup flying off the score: when it started (s), how long it
/// flies, how far left and up it goes, and where it started (from the right
/// and bottom).
#[derive(Component)]
struct ZmPopup {
    start: f32,
    life: f32,
    drift_x: f32,
    drift_y: f32,
    from_right: f32,
    from_bottom: f32,
}

/// What `zm_popups` keeps: the points last seen and when it started looking.
#[derive(Default)]
struct PopupMemo {
    last: Option<i32>,
    since: Option<f32>,
}

const LOSE_RED: Color = Color::srgb(0.85, 0.08, 0.05);

/// Set the alpha of every letter picture under `at`.
fn fade_glyphs(at: Entity, alpha: f32, kids: &Query<&Children>, glyphs: &mut Query<&mut ImageNode>) {
    if let Ok(mut img) = glyphs.get_mut(at) {
        // The dark copy under each letter keeps its own share.
        let is_shadow = img.color.to_srgba().red == 0.0;
        img.color.set_alpha(if is_shadow { alpha * 0.75 } else { alpha });
    }
    if let Ok(below) = kids.get(at) {
        for c in below.to_vec() {
            fade_glyphs(c, alpha, kids, glyphs);
        }
    }
}

/// BO2's floating score (hudcompetitivescoreboardzombie.lua): each time his
/// points change, the change flies off the left of the score and fades:
/// "+50" as he earns, "-950" in red as he spends (its FlyingDuration /
/// FlyingLeftOffSet / FlyingTopOffSet ranges, measured here as numbers of
/// our own).
fn zm_popups(
    mut commands: Commands,
    time: Res<Time>,
    window: MainWindow,
    (snapshot, me, bo2): (
        Option<Res<ShownWorldFrame>>,
        Option<Res<HostViewClient>>,
        Option<Res<Bo2Fonts>>,
    ),
    parts: PopupParts,
    (mut memo, mut seed): (Local<PopupMemo>, Local<u32>),
) {
    let PopupParts { roots, kids, mut popups, mut glyphs } = parts;
    // BO2's own HUD (unless BO2ZM_LUI=0) flies its own points.
    let values = both(&snapshot, &me)
        .filter(|_| !crate::lui_hud::lui_enabled())
        .and_then(|(shot, client)| read_values(shot, client));
    let Some(values) = values else {
        clear_roots(&mut commands, &roots);
        *memo = PopupMemo::default();
        return;
    };
    let now = time.elapsed_secs();
    let watched = now - *memo.since.get_or_insert(now);
    // Fly and fade.
    popups.iter_mut().for_each(|(e, pop, mut node)| {
        let t = (now - pop.start) / pop.life;
        if t >= 1.0 {
            commands.entity(e).despawn();
            return;
        }
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        node.bottom = Val::Px(pop.from_bottom + pop.drift_y * ease);
        node.right = Val::Px(pop.from_right + pop.drift_x * ease);
        fade_glyphs(e, (1.0 - (t - 0.35) / 0.65).min(1.0), &kids, &mut glyphs);
    });
    let Some(fonts) = bo2.as_deref() else {
        memo.last = Some(values.score);
        return;
    };
    let root = roots
        .iter()
        .next()
        .unwrap_or_else(|| commands.spawn(hud_layer(ZmPopupRoot)).id());
    let Some(before) = memo.last.replace(values.score) else {
        return;
    };
    let change = values.score - before;
    // The game's starting points arrive in its first moments: no popup.
    if change == 0 || watched < 3.0 {
        return;
    }
    let (sw, sh) = window_size(&window);
    let scale = (sh / 1080.0).max(0.4);
    // The score's left edge (it is drawn right-aligned 3.5% from the right,
    // 14% up, 46 px Default).
    let score_w = fonts.line_width("Default", &values.score.to_string(), 46.0 * scale);
    let from_right = sw * 0.035 + score_w + 6.0 * scale;
    let from_bottom = sh * 0.14 + 4.0 * scale;
    let mut rand = || {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) ^ (now.to_bits());
        (*seed >> 8) as f32 / 16_777_216.0
    };
    let (text, color) = match change {
        c if c > 0 => (format!("+{c}"), POINTS_WHITE),
        c => (format!("-{}", -c), LOSE_RED),
    };
    if std::env::var_os("BO2MP_T6_HUDLOG").is_some() {
        diag::info!(World, "zombies hud: points popup {text} (now {})", values.score);
    }
    let life = 0.6 + 0.4 * rand();
    let drift_x = (50.0 + 90.0 * rand()) * scale;
    let drift_y = (-30.0 + 70.0 * rand()) * scale;
    let popup = ZmPopup { start: now, life, drift_x, drift_y, from_right, from_bottom };
    let anchor = Node {
        right: Val::Px(from_right),
        bottom: Val::Px(from_bottom),
        ..absolute_node()
    };
    commands
        .spawn((popup, anchor, ChildOf(root)))
        .with_children(|c| {
            fonts.spawn_line(c, "Default", &[(text, color)], 34.0 * scale, 2.0 * scale);
        });
}

#[derive(Component)]
struct ZmHurtRoot;

/// bo2zm M3 fix list 2: the red screen edges when he is hurt, BO2's own
/// `overlay_low_health` over the whole screen: as strong as the health he
/// has lost the moment he is hit, fading as it comes back (the legacy blood
/// overlay curve, 0.3 a second: BO2's engine keeps its own in the exe, so
/// the fade speed is OUR CHOICE). The health script's red flashing at a
/// fifth of his health still pulses on top. His "when I get hit, I have no
/// effect like I got hit. My screen should get red."
fn zm_hurt_overlay(
    mut commands: Commands,
    time: Res<Time>,
    mut overlays: Query<(Entity, &mut ImageNode), With<ZmHurtRoot>>,
    (snapshot, me): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    mut edge: OneIcon,
    mut intensity: Local<f32>,
) {
    // Below the HUD's text; the dead and spectators get none.
    const RATE: f32 = 0.3;
    let loaded = edge.get("overlay_low_health");
    let lost = both(&snapshot, &me).and_then(|(shot, client)| {
        // (bo2mp: a multiplayer match has its own, `bo2mp_damage`.)
        if dvar_of(shot, client, "bo2mp_gametype").is_some() {
            return None;
        }
        let ps = shot.player(client.0)?;
        let alive = ps.health > 0 && ps.hit_point_ceiling > 0 && ps.movement_mode < 2;
        alive.then(|| 1.0 - (ps.health as f32 / ps.hit_point_ceiling as f32).clamp(0.0, 1.0))
    });
    let Some(lost) = lost else {
        *intensity = 0.0;
        overlays.iter().for_each(|(e, _)| commands.entity(e).despawn());
        return;
    };
    let faded = (*intensity - time.delta_secs() * RATE).max(lost);
    *intensity = if lost > *intensity { lost } else { faded };
    let Some(handle) = loaded else { return };
    let color = Color::srgba(1.0, 1.0, 1.0, *intensity);
    if let Ok((_root, mut node)) = overlays.single_mut() {
        node.color = color;
        return;
    }
    if *intensity <= 0.0 {
        return;
    }
    // Stretched over the screen, as BO2 draws it (not kept square).
    commands.spawn((
        ZmHurtRoot,
        stretched(handle, color),
        ZIndex(-2000),
        hud_tag(),
        Node { left: Val::Px(0.0), top: Val::Px(0.0), ..covering_node() },
    ));
}

#[derive(Component)]
struct ScriptWaypoint;

/// A world point put on the screen: the picture's name, its centre, size, alpha.
struct Spot<'a> {
    name: &'a str,
    centre: Vec2,
    size: Vec2,
    alpha: f32,
}

/// One `bo2mp_waypoints` row as the camera sees it, or `None` when it is
/// behind the camera, off screen or too small.
fn waypoint_spot<'a>(row: &'a str, camera: &Camera, view: &GlobalTransform, (sw, sh): (f32, f32)) -> Option<Spot<'a>> {
    let f: Vec<&str> = row.split(',').collect();
    if f.len() < 8 {
        return None;
    }
    let num = |i: usize| f[i].trim().parse::<f32>().unwrap_or(0.0);
    let at = Vec3::new(num(1), num(2), num(3));
    let (w, h) = (num(4), num(5));
    if (at - view.translation()).dot(*view.forward()) <= 0.0 {
        return None;
    }
    let pos = camera.world_to_viewport(view, at).ok()?;
    if !(0.0..=sw).contains(&pos.x) || !(0.0..=sh).contains(&pos.y) {
        return None;
    }
    let size = if f[7] == "1" {
        Vec2::new(w * (sw / 640.0), h * (sh / 480.0))
    } else {
        let edge = camera.world_to_viewport(view, at + *view.right() * w).ok()?;
        let pw = (edge - pos).length();
        Vec2::new(pw, pw * h / w.max(0.01))
    };
    (size.x >= 1.0).then_some(Spot { name: f[0], centre: pos, size, alpha: num(6) })
}

/// bo2mp: the scripts' pictures in the world (`setwaypoint` elements: the
/// equipment head icons over a planted claymore, seen by its team only),
/// from `bo2mp_waypoints` ("material,x,y,z,width,height,alpha,constant" by
/// `;`), each frame where the camera puts them. A constant-size one is
/// its width x height on BO2's 640x480 screen; else its width is in world
/// units, so it shrinks with distance. Off screen: not drawn.
fn script_waypoints(
    mut commands: Commands,
    old: Query<Entity, With<ScriptWaypoint>>,
    window: MainWindow,
    views: SceneCameras,
    (snapshot, me): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    (world_dim, menu_dims, mut logged): (Option<Res<frame::WorldDim>>, Option<Res<frame::MenuDims>>, Local<(String, bool)>),
    (icons, mut images, mut pictures): (
        Option<Res<assets::T6HudIcons>>,
        ResMut<Assets<Image>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
) {
    old.iter().for_each(|e| commands.entity(e).despawn());
    let Some(value) = dvar_text(&snapshot, &me, "bo2mp_waypoints").filter(|v| !v.is_empty()) else {
        return;
    };
    let dims = world_dim
        .as_deref()
        .map_or(0.0, |d| d.0)
        .max(menu_dims.as_deref().map_or(0.0, |d| d.0 as f32));
    let hidden = dims >= 2.0;
    if std::env::var_os("BO2MP_WAYPOINT_LOG").is_some() && *logged != (value.clone(), hidden) {
        diag::info!(Ui, "match hud: waypoint markers (dims {dims}, hidden {hidden}): {value}");
        *logged = (value.clone(), hidden);
    }
    // bo2mp: these head icons are drawn as UI above the popup panel, outside
    // the HUD root; under two stacked menu dims (the pause menu, then a popup)
    // the HUD is dimmed out entirely, so the icon (an enemy's skull over the
    // End Game? panel) goes with it. With one dim it stays and dims like the
    // world.
    if hidden {
        return;
    }
    let Some(ActiveView { camera, at: view, .. }) = views.active() else {
        return;
    };
    let screen = window_size(&window);
    for spot in value.split(';').filter_map(|row| waypoint_spot(row, camera, view, screen)) {
        let handle = match lookup(&pictures, spot.name) {
            Some(h) => h,
            None => {
                let Some(img) = icons
                    .as_deref()
                    .and_then(|i| i.0.iter().find(|(k, _)| k == spot.name))
                    .map(|(_, i)| i.clone())
                else {
                    continue;
                };
                let h = images.add((*img).clone());
                pictures.push((spot.name.to_owned(), h.clone()));
                h
            }
        };
        let corner = spot.centre - spot.size * 0.5;
        commands.spawn((
            ScriptWaypoint,
            ImageNode::new(handle).with_color(Color::srgba(1.0, 1.0, 1.0, spot.alpha)),
            px_rect(corner.x, corner.y, spot.size.x, spot.size.y),
            ZIndex(-900),
            hud_tag(),
        ));
    }
}

/// The killcam's orange YOU pin: the engine hangs it over the player who
/// was killed, seen in the replay (`headiconyouinkillcam`).
#[derive(Component)]
struct KillcamYou;

fn killcam_you(
    mut commands: Commands,
    old: Query<Entity, With<KillcamYou>>,
    window: MainWindow,
    views: SceneCameras,
    entities: Query<(&net::ClientEntityRecord, &net::ClientEntityPoseState)>,
    (snapshot, viewer): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    (menu_dims, world_dim): (Option<Res<frame::MenuDims>>, Option<Res<frame::WorldDim>>),
    mut you: OneIcon,
) {
    old.iter().for_each(|e| commands.entity(e).despawn());
    // bo2mp: gone under two stacked menu dims, like the HUD root.
    if menu_dims.is_some_and(|d| d.0 >= 2) || world_dim.is_some_and(|d| d.0 >= 2.0) {
        return;
    }
    let Some((_frame, client)) = both(&snapshot, &viewer) else {
        return;
    };
    let on = dvar_text(&snapshot, &viewer, "bo2mp_killcam")
        .is_some_and(|v| v.starts_with("1|") || v.starts_with("2|") || v.starts_with("4|"));
    if !on {
        return;
    }
    let Some(ActiveView { camera, at: view, .. }) = views.active() else {
        return;
    };
    let me_runtime = entities
        .iter()
        .find(|(e, r)| e.client() == Some(client.0) && r.present_in_newer_frame());
    let Some((_record, runtime)) = me_runtime else {
        return;
    };
    let head = Vec3::from_array(runtime.origin) + Vec3::Z * 98.0;
    if (head - view.translation()).dot(*view.forward()) <= 0.0 {
        return;
    }
    let Ok(pos) = camera.world_to_viewport(view, head) else {
        return;
    };
    let (sw, sh) = window_size(&window);
    if !(0.0..=sw).contains(&pos.x) || !(0.0..=sh).contains(&pos.y) {
        return;
    }
    let Some(you_pic) = you.get("headiconyouinkillcam") else {
        return;
    };
    // The real pin is its 128-pixel square at 720p (the pin fills the middle
    // of it, its point at 89% of the way down), over his head.
    let side = 128.0 * (sh / 720.0);
    commands.spawn((
        KillcamYou,
        ImageNode::new(you_pic),
        px_rect(pos.x - side * 0.5, pos.y - side * 0.89, side, side),
        ZIndex(-900),
        hud_tag(),
    ));
}

pub(crate) fn register_zm_hud_systems(app: &mut App) {
    crate::every_ui_frame(app, (killcam_you, script_waypoints));
    app.add_systems(
        Update,
        (
            crate::bo2_font::load_bo2_fonts,
            (zm_hud, zm_script_hud, zm_icons, zm_popups, zm_hurt_overlay),
        )
            .chain()
            .in_set(ClientStep::Ui),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_runs_fill_the_key_and_split_colours() {
        let keys = OverlayControlSnapshot {
            act_key: Some("F".into()),
            ..Default::default()
        };
        let runs = hint_runs("Hold ^3[{+activate}]^7 for M14 [Cost: 500]", &keys);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].0, "Hold ");
        assert_eq!(runs[1].0, "[F]");
        assert_eq!(runs[2].0, " for M14 [Cost: 500]");
    }

    #[test]
    fn hint_runs_without_brackets_and_with_unknown_keys() {
        let keys = OverlayControlSnapshot::default();
        let runs = hint_runs_with("Press ^2[{+actionslot 3}]^7 now ^", &keys, false);
        let text: String = runs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(text, "Press +ACTIONSLOT 3 now ^".replace('+', ""));
        assert_eq!(key_for("+activate", &keys), "F");
    }

    #[test]
    fn clip_table_rows_are_read_by_weapon_and_hand() {
        let mut table = Vec::new();
        for (gun_id, right, left) in [(7_i32, 30_i32, 0_i32), (9, 12, 8)] {
            for word in [gun_id, right, left] {
                table.extend_from_slice(&word.to_le_bytes());
            }
        }
        assert_eq!(clip_of(&table, 9, 0), 12);
        assert_eq!(clip_of(&table, 9, 1), 8);
        assert_eq!(clip_of(&table, 5, 0), 0);
    }

    #[test]
    fn offhand_rows_keep_known_nonzero_entries() {
        let rows = offhand_rows("frag_grenade_zm:2;claymore_zm:0;mystery:3;cymbal_monkey_zm:x;sticky_grenade_zm:1");
        assert_eq!(rows, vec![("hud_us_grenade", 2), ("hud_icon_sticky_grenade", 1)]);
    }

    #[test]
    fn ammo_text_shows_the_left_clip_when_dual_wielding() {
        let mut v = ZmHudValues::default();
        v.clip = 8;
        v.stock = 64;
        assert_eq!(ammo_text(&v), "8 / 64");
        v.clip_left = Some(5);
        assert_eq!(ammo_text(&v), "5 | 8 / 64");
    }

    #[test]
    fn element_tops_follow_the_outcome_rules() {
        assert_eq!(text_top(true, "middle", 100.7, 20.0), 90.0);
        assert_eq!(text_top(true, "bottom", 100.7, 20.0), 80.0);
        assert_eq!(text_top(false, "middle", 100.0, 10.0), 94.0);
        assert_eq!(text_top(false, "top", 100.0, 10.0), 100.0);
        assert_eq!(text_left("right", 50.0, 20.0).0, 30.0);
        assert_eq!(horz_base("user_right"), 640.0);
        assert_eq!(base_vertex_at("middle"), 240.0);
    }

    #[test]
    fn hudelem_font_picks_a_face_by_size() {
        assert_eq!(hudelem_font("extrabig", 1.0, 1.0).0, "Morris");
        assert_eq!(hudelem_font("default", 1.0, 1.0).0, "Condensed");
        assert_eq!(hudelem_font("default", 2.0, 1.0).0, "Big");
        assert_eq!(hudelem_font("default", 1.4, 1.0).0, "Default");
    }

    #[test]
    fn script_rows_parse_with_the_optional_tail() {
        let row = ["1", "2", "center", "top", "left", "top", "1.5", "1", "0", "0", "1", "7", "hi", "m", "big"].join("\u{1f}");
        let elems = parse_script_hud(&row);
        assert_eq!(elems.len(), 1);
        assert!(elems[0].hide_in_menu);
        assert_eq!(elems[0].font_key, "big");
        assert!(parse_script_hud("too\u{1f}short").is_empty());
    }

    #[test]
    fn pixel_boxes_carry_their_numbers() {
        let n = px_rect(1.0, 2.0, 3.0, 4.0);
        assert_eq!((n.left, n.top, n.width, n.height), (Val::Px(1.0), Val::Px(2.0), Val::Px(3.0), Val::Px(4.0)));
        assert_eq!(px_strip(1.0, 2.0, 3.0).height, Val::Auto);
        assert_eq!(corner(None, Some(3.5), 6.0).right, Val::Percent(3.5));
    }
}
