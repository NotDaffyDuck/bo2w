//! The loading screen: a title whose letters chase each other, and a table
//! that follows the load's progress while a map comes up.

use std::time::{Duration, Instant};

use asset_transport::StageId;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures_lite::future};

use crate::load_table::{RowState, format_elapsed, project, total_elapsed};
use crate::{GameUiFont, UiLayer, UiLayerVisibility, game_text_font};

pub use asset_transport::LoadProgress;
pub use assets::{LoadingPreviewSource, LoadingScreen};

/// Root node of the screen.
#[derive(Component)]
pub(crate) struct LoadingRoot;

/// Remembers which title the root was built for.
#[derive(Component)]
pub(crate) struct LoadingOverlayTitle(String);

/// The opaque camera under the loading root.
#[derive(Component)]
pub(crate) struct LoadingCamera;

/// A transparent camera that keeps the UI visible over the finished game.
#[derive(Component)]
pub(crate) struct OverlayUiCamera;

/// One letter of the title.
#[derive(Component)]
pub(crate) struct LoadingLetter(usize);

/// The column the table rows are parented to.
#[derive(Component)]
pub(crate) struct LoadingStatusList;

/// One text cell of the table.
#[derive(Component)]
pub(crate) struct LoadingStatusCell {
    line: StatusLine,
    column: StatusColumn,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusLine {
    Head,
    Slot(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusColumn {
    Name,
    Number,
    Unit,
}

/// A map preview decode that has been started.
#[derive(Resource)]
pub(crate) struct LoadingPreviewTask {
    which: PreviewRequest,
    task: Task<Option<(u32, u32, Vec<u8>)>>,
}

#[derive(Clone, PartialEq, Eq)]
struct PreviewRequest {
    request_id: u64,
    map_name: String,
}

impl PreviewRequest {
    fn is_for(&self, source: &LoadingPreviewSource) -> bool {
        self.request_id == source.request_id && self.map_name == source.map_name
    }
}

/// The rows currently on screen and which stage each one shows.
#[derive(Default)]
pub(crate) struct StatusTable {
    /// A new request is a new load and gets a fresh table.
    request: Option<u64>,
    head: Option<Entity>,
    spacer: Option<Entity>,
    slots: Vec<(Option<StageId>, Entity)>,
}

impl StatusTable {
    fn take_entities(&mut self) -> Vec<Entity> {
        let mut all: Vec<Entity> = self.head.take().into_iter().chain(self.spacer.take()).collect();
        all.extend(self.slots.drain(..).map(|(_, row)| row));
        all
    }

    fn slot_of(&self, id: StageId) -> Option<usize> {
        self.slots.iter().position(|(shown, _)| *shown == Some(id))
    }
}

const LETTER_STEP: Duration = Duration::from_millis(140);
const LETTER_FONT_PX: f32 = 56.0;
const LETTER_FONT_MIN: f32 = 28.0;
const MODE_FONT_PX: f32 = 22.0;
const CELL_FONT_PX: f32 = 14.0;
const NUMBER_COLUMN_PX: f32 = 64.0;
const UNIT_COLUMN_PX: f32 = 124.0;
const NAME_PADDING_PX: f32 = 28.0;
/// All rows are this tall, filled or not, so the table does not jump.
const ROW_PX: f32 = 20.0;
const TABLE_SLOTS: usize = 6;
/// The table refreshes about ten times a second; letters animate every frame.
const TABLE_REFRESH: Duration = Duration::from_millis(100);

const BACKDROP: Color = Color::srgb(0.08, 0.09, 0.12);
const TEXT_LIGHT: Color = Color::srgb(0.92, 0.93, 0.95);
const TEXT_ERROR: Color = Color::srgb(1.0, 0.35, 0.25);
const LETTER_IDLE: Color = Color::srgb(0.42, 0.44, 0.47);
const LETTER_IDLE_READY: Color = Color::srgb(0.96, 0.96, 0.97);
const LETTER_ACTIVE: Color = Color::srgb(1.0, 0.62, 0.12);

fn text_shadow() -> TextShadow {
    TextShadow {
        offset: Vec2::splat(1.5),
        color: Color::linear_rgba(0., 0., 0., 0.75),
        ..default()
    }
}

fn spawn_overlay_camera(commands: &mut Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 200,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        crate::UiCamera,
        OverlayUiCamera,
    ));
}

/// Tears the loading screen down. With `keep_overlay_camera` the transparent
/// UI camera stays (or is created) so later UI still has something to draw on.
pub(crate) fn dismiss_loading_overlay(
    commands: &mut Commands,
    chrome: impl IntoIterator<Item = Entity>,
    overlay_cams: impl IntoIterator<Item = Entity>,
    keep_overlay_camera: bool,
) {
    for entity in chrome {
        commands.entity(entity).try_despawn();
    }
    let mut seen = 0usize;
    for cam in overlay_cams {
        seen += 1;
        if !keep_overlay_camera {
            commands.entity(cam).try_despawn();
        }
    }
    commands.remove_resource::<LoadingScreen>();
    commands.remove_resource::<LoadingPreviewTask>();
    commands.remove_resource::<LoadingPreviewSource>();
    if keep_overlay_camera && seen == 0 {
        spawn_overlay_camera(commands);
    }
}

fn letter_size(title: &str) -> f32 {
    let letters = title.chars().count().max(1) as f32;
    (LETTER_FONT_PX * 10.0 / letters).clamp(LETTER_FONT_MIN, LETTER_FONT_PX)
}

/// Position of the lit letter: it runs to the end of the title and back.
fn bounce(step: usize, letters: usize) -> usize {
    if letters <= 1 {
        return 0;
    }
    let cycle = 2 * (letters - 1);
    let at = step % cycle;
    if at < letters { at } else { cycle - at }
}

fn column_node(width: f32, justify: JustifyContent) -> Node {
    Node {
        width: Val::Px(width),
        min_width: Val::Px(width),
        flex_shrink: 0.0,
        justify_content: justify,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn row_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        height: Val::Px(ROW_PX),
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    }
}

/// Table text never wraps: a wrapped cell becomes taller than the fixed-height
/// row and its first line then sits higher than its neighbours'.
fn cell_layout() -> TextLayout {
    TextLayout::new(Justify::Left, bevy::text::LineBreak::NoWrap)
}

fn cell_text(font: &Handle<Font>, line: StatusLine, column: StatusColumn) -> impl Bundle {
    (
        LoadingStatusCell { line, column },
        Text::new(""),
        game_text_font(font, CELL_FONT_PX),
        TextColor(TEXT_LIGHT),
        cell_layout(),
        text_shadow(),
    )
}

fn spawn_spacer(commands: &mut Commands) -> Entity {
    commands.spawn(row_node()).id()
}

fn spawn_table_row(commands: &mut Commands, font: &Handle<Font>, line: StatusLine) -> Entity {
    let row = commands.spawn(row_node()).id();
    commands.entity(row).with_children(|row| {
        row.spawn(Node {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            overflow: Overflow::clip_x(),
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            padding: UiRect::right(Val::Px(NAME_PADDING_PX)),
            ..default()
        })
        .with_child(cell_text(font, line, StatusColumn::Name));
        row.spawn(column_node(NUMBER_COLUMN_PX, JustifyContent::FlexEnd))
            .with_child(cell_text(font, line, StatusColumn::Number));
        row.spawn(Node {
            padding: UiRect::left(Val::Px(6.0)),
            ..column_node(UNIT_COLUMN_PX, JustifyContent::FlexStart)
        })
        .with_child(cell_text(font, line, StatusColumn::Unit));
    });
    row
}

fn spawn_title_block(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, screen: &LoadingScreen) {
    parent
        .spawn(Node {
            width: Val::Percent(48.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|column| {
            let size = letter_size(screen.title());
            column
                .spawn((
                    Text::default(),
                    TextLayout::justify(Justify::Left),
                    text_shadow(),
                ))
                .with_children(|title| {
                    for (index, letter) in screen.title().chars().enumerate() {
                        title.spawn((
                            TextSpan::new(letter.to_string()),
                            game_text_font(font, size),
                            TextColor(LETTER_IDLE),
                            LoadingLetter(index),
                        ));
                    }
                });
            column.spawn((
                Text::new(screen.mode_label().to_owned()),
                game_text_font(font, MODE_FONT_PX),
                TextColor(TEXT_LIGHT),
                TextLayout::justify(Justify::Left),
                text_shadow(),
            ));
        });
}

fn spawn_table_block(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            width: Val::Percent(48.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Stretch,
            justify_content: JustifyContent::FlexEnd,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_child((
            LoadingStatusList,
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(2.0),
                ..default()
            },
        ));
}

pub(crate) fn spawn_loading_screen(
    mut commands: Commands,
    screen: Option<Res<LoadingScreen>>,
    font: Option<Res<GameUiFont>>,
    preview_source: Option<Res<LoadingPreviewSource>>,
    existing: Query<(Entity, Option<&LoadingOverlayTitle>), With<LoadingRoot>>,
    cameras: Query<Entity, With<LoadingCamera>>,
    leftover_overlay: Query<Entity, With<OverlayUiCamera>>,
    running_preview: Option<Res<LoadingPreviewTask>>,
    bo2mp: Option<Res<session::Bo2mpMatchSetup>>,
) {
    let Some(screen) = screen else {
        return;
    };
    let title = screen.title();
    let built_for_title = existing
        .iter()
        .any(|(_, built)| built.is_some_and(|built| built.0 == title));
    if built_for_title && leftover_overlay.is_empty() {
        return;
    }
    let Some(font) = font else {
        diag::warn!(Ui, "loading: GameUiFont missing; loading screen skipped");
        return;
    };
    let font = font.0.clone();

    let stale: Vec<Entity> = existing
        .iter()
        .map(|(entity, _)| entity)
        .chain(cameras.iter())
        .chain(leftover_overlay.iter())
        .collect();
    for entity in stale {
        commands.entity(entity).try_despawn();
    }

    commands.spawn((
        Camera2d,
        Camera {
            order: 100,
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        crate::UiCamera,
        LoadingCamera,
    ));

    // In a bo2mp match the game's own Loading menu (`lui_hud`) draws over this
    // plain backdrop, so no title, mode line or table is added here.
    let mut root = commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            ..default()
        },
        BackgroundColor(BACKDROP),
        GlobalZIndex(10_000),
        UiLayer::Loading,
        UiLayerVisibility,
        Visibility::Inherited,
        LoadingRoot,
        LoadingOverlayTitle(title.to_owned()),
    ));
    if bo2mp.is_none() {
        root.with_children(|root| {
            root.spawn(Node {
                width: Val::Percent(78.0),
                margin: UiRect::bottom(Val::Percent(10.0)),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(48.0),
                ..default()
            })
            .with_children(|band| {
                spawn_title_block(band, &font, &screen);
                spawn_table_block(band);
            });
        });
    }

    diag::info!(
        Ui,
        "loading: screen spawned (title=`{}`, mode=`{}`)",
        screen.title(),
        screen.mode_label()
    );
    if let Some(source) = preview_source {
        let already = running_preview
            .as_deref()
            .is_some_and(|running| running.which.is_for(&source));
        if !already {
            start_preview_decode(
                &mut commands,
                source.map_name.clone(),
                source.request_id,
                screen.progress.clone(),
            );
        }
    }
}

fn start_preview_decode(
    commands: &mut Commands,
    map_name: String,
    request_id: u64,
    progress: LoadProgress,
) {
    let which = PreviewRequest {
        request_id,
        map_name,
    };
    // The map preview image of a BO2 map has no source here yet, so there is
    // nothing to decode: the stage is reported as skipped, which is not a failure.
    let task = AsyncComputeTaskPool::get().spawn(async move {
        progress.begin(StageId::Preview, None).skip();
        None
    });
    commands.insert_resource(LoadingPreviewTask { which, task });
}

pub(crate) fn poll_loading_preview(
    mut commands: Commands,
    mut pending: Option<ResMut<LoadingPreviewTask>>,
    mut loading: Option<ResMut<LoadingScreen>>,
    source: Option<Res<LoadingPreviewSource>>,
    mut images: ResMut<Assets<Image>>,
    root: Query<Entity, With<LoadingRoot>>,
) {
    let Some(pending) = pending.as_deref_mut() else {
        return;
    };
    let Some(decoded) = future::block_on(future::poll_once(&mut pending.task)) else {
        return;
    };
    let which = pending.which.clone();
    commands.remove_resource::<LoadingPreviewTask>();
    let (Some(source), Some(loading)) = (source.as_deref(), loading.as_deref_mut()) else {
        return;
    };
    if !which.is_for(source) {
        return;
    }
    let Some((width, height, pixels)) = decoded else {
        diag::info!(Ui, "loading: no map preview (keeping solid color)");
        return;
    };
    let Ok(root) = root.single() else {
        return;
    };
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    let handle = images.add(image);
    commands
        .entity(root)
        .insert(ImageNode::new(handle).with_mode(bevy::ui::widget::NodeImageMode::Stretch));
    loading.mark_preview_ready();
    diag::info!(Ui, "loading: map preview ready ({width}x{height})");
}

/// Assigns snapshot rows to the on-screen slots. A row keeps its slot once it
/// has one; a new row takes an empty slot, or else replaces a finished one.
fn assign_slots(shown: &mut StatusTable, table: &crate::load_table::LoadTable) {
    for row in table.running.iter().chain(&table.ended) {
        if shown.slot_of(row.id).is_some() {
            continue;
        }
        let empty = shown.slots.iter().position(|(held, _)| held.is_none());
        let finished = || {
            shown.slots.iter().position(|(held, _)| {
                held.and_then(|id| table.row(id))
                    .is_some_and(|held| matches!(held.state, RowState::Done | RowState::Reused))
            })
        };
        if let Some(slot) = empty.or_else(finished) {
            shown.slots[slot].0 = Some(row.id);
        }
    }
}

fn set_text(text: &mut Text, value: &str) {
    // Writing identical text would still trigger a re-layout.
    if text.0 != value {
        value.clone_into(&mut text.0);
    }
}

pub(crate) fn update_loading_screen(
    mut commands: Commands,
    time: Res<Time>,
    font: Option<Res<GameUiFont>>,
    mut screen: Option<ResMut<LoadingScreen>>,
    load: Option<Res<assets::MapLoadProcess>>,
    mut next_refresh: Local<Option<Instant>>,
    mut shown: Local<StatusTable>,
    mut letters: Query<(&LoadingLetter, &mut TextColor), Without<LoadingStatusCell>>,
    list: Query<(Entity, Option<&Children>), With<LoadingStatusList>>,
    mut cells: Query<(&LoadingStatusCell, &mut Text, &mut TextColor), Without<LoadingLetter>>,
    roots: Query<Entity, Or<(With<LoadingRoot>, With<LoadingCamera>)>>,
) {
    let Some(screen) = screen.as_deref_mut() else {
        return;
    };
    screen.tick_elapsed(time.delta());

    let lit = bounce(
        (screen.elapsed().as_millis() / LETTER_STEP.as_millis()) as usize,
        letters.iter().count().max(1),
    );
    let idle = if screen.preview_ready() {
        LETTER_IDLE_READY
    } else {
        LETTER_IDLE
    };
    for (letter, mut color) in &mut letters {
        color.0 = if letter.0 == lit { LETTER_ACTIVE } else { idle };
    }

    if screen.is_complete() {
        dismiss_loading_overlay(&mut commands, roots.iter(), std::iter::empty(), true);
        diag::info!(Ui, "loading: screen dismissed");
        return;
    }

    let now = Instant::now();
    if next_refresh.is_some_and(|due| now < due) {
        return;
    }
    *next_refresh = Some(now + TABLE_REFRESH);

    // The load process is the real thing and the screen only watches it; when
    // both exist the process is the one a newer request replaces.
    let progress = load
        .as_deref()
        .map_or(&screen.progress, |process| &process.progress);
    let snapshot = progress.snapshot();
    let table = project(&snapshot);

    let (Ok((list_entity, children)), Some(font)) = (list.single(), font.as_ref()) else {
        return;
    };
    let empty_list = children.is_none_or(|children| children.is_empty());
    if empty_list || shown.request != Some(snapshot.request_id) {
        for old in shown.take_entities() {
            commands.entity(old).try_despawn();
        }
        shown.request = Some(snapshot.request_id);
        // Rows replaced via `replace_children` are only detached, and a
        // detached row would stay on screen as a root of its own, hence the
        // explicit despawn above.
        let head = spawn_table_row(&mut commands, &font.0, StatusLine::Head);
        let spacer = spawn_spacer(&mut commands);
        shown.head = Some(head);
        shown.spacer = Some(spacer);
        let mut order = vec![head, spacer];
        for slot in 0..TABLE_SLOTS {
            let row = spawn_table_row(&mut commands, &font.0, StatusLine::Slot(slot));
            shown.slots.push((None, row));
            order.push(row);
        }
        commands.entity(list_entity).replace_children(&order);
    }

    assign_slots(&mut shown, &table);

    let total = format_elapsed(total_elapsed(&snapshot));
    let (head_label, head_color) = match screen.failure() {
        Some(reason) => (format!("Map load failed — {reason}"), TEXT_ERROR),
        None => ("Loading".to_owned(), TEXT_LIGHT),
    };

    for (cell, mut text, mut color) in &mut cells {
        let (label, metric, paint) = match cell.line {
            StatusLine::Head => (head_label.as_str(), Some(&total), head_color),
            StatusLine::Slot(slot) => {
                let row = shown
                    .slots
                    .get(slot)
                    .and_then(|(id, _)| *id)
                    .and_then(|id| table.row(id));
                match row {
                    Some(row) => (row.name.as_str(), Some(&row.value), row.state.color()),
                    None => ("", None, TEXT_LIGHT),
                }
            }
        };
        set_text(
            &mut text,
            match cell.column {
                StatusColumn::Name => label,
                StatusColumn::Number => metric.map_or("", |m| m.number.as_str()),
                StatusColumn::Unit => metric.map_or("", |m| m.unit.as_str()),
            },
        );
        color.0 = paint;
    }
}
