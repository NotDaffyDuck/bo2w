//! Loading screen. While a map is coming up the player sees a title whose
//! letters light up one after another, plus a small live table of the load
//! stages that are currently running or have just ended.

use std::time::{Duration, Instant};

use asset_transport::StageId;
use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use crate::load_table::{
    LoadRow, LoadTable, Metric, RowState, format_elapsed, project, total_elapsed,
};
use crate::tiers::{MenuTypeface, OverlayTier, OverlayStackShown, match_typeface};

pub use asset_transport::LoadCompletion;
pub use assets::{LoadBackdropOrigin, LoadBackdrop};

/// Top node that everything on the screen hangs from.
#[derive(Component)]
pub(crate) struct SplashSceneAnchor;

/// Title text the current root was built with; a different title means a rebuild.
#[derive(Component)]
pub(crate) struct SplashHeadline(String);

/// Opaque camera drawing the backdrop below the root.
#[derive(Component)]
pub(crate) struct SplashLens;

/// See-through camera so UI stays drawn once the game itself is showing.
#[derive(Component)]
pub(crate) struct OverlayLens;

/// A single character of the animated title.
#[derive(Component)]
pub(crate) struct SplashGlyph(usize);

/// Container whose children are the table rows.
#[derive(Component)]
pub(crate) struct SplashProgressLines;

/// A text cell in one table row.
#[derive(Component)]
pub(crate) struct LoadingStatusCell {
    kind: RowKind,
    col: CellColumn,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Header,
    Stage(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellColumn {
    Name,
    Number,
    Unit,
}

/// Pixels of a finished preview: width, height, RGBA bytes.

/// Handle to a preview decode that is in flight.
#[derive(Resource)]
pub(crate) struct SplashThumbJob {
    which: PreviewRequest,
    task: bevy::tasks::Task<Option<(u32, u32, Vec<u8>)>>,
}

#[derive(Clone, PartialEq, Eq)]
struct PreviewRequest {
    id: u64,
    map: String,
}

impl PreviewRequest {
    fn is_for(&self, source: &LoadBackdropOrigin) -> bool {
        self.id == source.ticket && self.map == source.level_name
    }
}

/// What the table currently shows: header, spacer and a fixed set of stage_slots,
/// each slot remembering the stage it was handed.
#[derive(Default)]
pub(crate) struct StatusTable {
    /// Table contents belong to one load request; another request starts over.
    for_request: Option<u64>,
    header: Option<Entity>,
    gap: Option<Entity>,
    stage_slots: Vec<(Option<StageId>, Entity)>,
}

impl StatusTable {
    /// Hands back every row entity and leaves the table empty.
    fn drain_entities(&mut self) -> Vec<Entity> {
        let fixed = [self.header.take(), self.gap.take()];
        let slot_rows = std::mem::take(&mut self.stage_slots);
        fixed
            .into_iter()
            .flatten()
            .chain(slot_rows.into_iter().map(|(_, entity)| entity))
            .collect()
    }

    fn slot_showing(&self, id: StageId) -> Option<usize> {
        self.stage_slots.iter().position(|slot| slot.0 == Some(id))
    }
}

/// One step of the title animation lasts this long.
const LETTER_STEP: Duration = Duration::from_micros(140_000);
const TITLE_SIZE_MAX: f32 = 56.0;
const TITLE_SIZE_MIN: f32 = 28.0;
const SUBTITLE_SIZE: f32 = 22.0;
const CELL_SIZE: f32 = 14.0;
/// Draw order of the load screen root, above every other UI root.
const SPLASH_Z: i32 = 10_000;
const NUMBER_COL_WIDTH: f32 = 64.0;
const UNIT_COL_WIDTH: f32 = 124.0;
const NAME_GUTTER: f32 = 28.0;
/// Every row has this height, used or not, so the layout stays put.
const ROW_HEIGHT: f32 = 20.0;
const VISIBLE_ROWS: usize = 6;
/// Table contents are recomputed at this pace; the letters move every frame.
const TABLE_REFRESH: Duration = Duration::from_micros(100_000);

const SCREEN_BG: Color = Color::srgb(0.08, 0.09, 0.12);
const INK: Color = Color::srgb(0.92, 0.93, 0.95);
const INK_ERROR: Color = Color::srgb(1.0, 0.35, 0.25);
const UNLIT: Color = Color::srgb(0.42, 0.44, 0.47);
const UNLIT_READY: Color = Color::srgb(0.96, 0.96, 0.97);
const LIT: Color = Color::srgb(1.0, 0.62, 0.12);

/// Applies `edit` to a node and returns it, so layouts read as a short list of
/// assignments on top of a base.
fn tweak(mut node: Node, edit: impl FnOnce(&mut Node)) -> Node {
    edit(&mut node);
    node
}

fn drop_shadow() -> TextShadow {
    let mut shadow = TextShadow::default();
    shadow.offset = Vec2::splat(1.5);
    shadow.color = Color::linear_rgba(0.0, 0.0, 0.0, 0.75);
    shadow
}

/// Spawns a 2D UI camera with the given draw order and clear behaviour.
fn ui_camera<'a>(
    commands: &'a mut Commands,
    order: isize,
    clear: ClearColorConfig,
) -> EntityCommands<'a> {
    let camera = Camera {
        order,
        clear_color: clear,
        ..default()
    };
    commands.spawn((crate::UiCamera, Camera2d, camera))
}

/// Takes the loading screen down. When `keep_overlay_camera` is set the
/// see-through UI camera survives, or is made if none was around, so menus
/// shown afterwards still have a camera to render with.
pub(crate) fn close_splash_overlay(
    commands: &mut Commands,
    chrome_roots: impl IntoIterator<Item = Entity>,
    camera_ids: impl IntoIterator<Item = Entity>,
    keep_overlay_camera: bool,
) {
    for root in chrome_roots {
        commands.entity(root).try_despawn();
    }

    let cameras: Vec<Entity> = camera_ids.into_iter().collect();
    match (keep_overlay_camera, cameras.is_empty()) {
        (true, true) => {
            ui_camera(commands, 200, ClearColorConfig::Custom(Color::NONE)).insert(OverlayLens);
        }
        (true, false) => {}
        (false, _) => {
            for camera in cameras {
                commands.entity(camera).try_despawn();
            }
        }
    }

    retire_preview(commands);
    commands.remove_resource::<LoadBackdrop>();
}

/// Forgets which preview was asked for and any decode still running.
fn retire_preview(commands: &mut Commands) {
    commands.remove_resource::<LoadBackdropOrigin>();
    commands.remove_resource::<SplashThumbJob>();
}

/// Letters shrink as the title grows, but never leave the allowed range.
fn letter_px(title: &str) -> f32 {
    let count = title.chars().count().max(1) as f32;
    let budget = TITLE_SIZE_MAX * 10.0;
    (budget / count).clamp(TITLE_SIZE_MIN, TITLE_SIZE_MAX)
}

/// Which letter is lit at animation `step`: the light travels to the last
/// letter and then back to the first, over and over.
fn lit_index(step: usize, letters: usize) -> usize {
    let Some(last) = letters.checked_sub(1).filter(|last| *last > 0) else {
        return 0;
    };
    let period = last * 2;
    let into_sweep = step % period;
    into_sweep.min(period - into_sweep)
}

/// How many animation steps `elapsed` covers.
fn animation_step(elapsed: Duration) -> usize {
    (elapsed.as_millis() / LETTER_STEP.as_millis()) as usize
}

/// A vertical flex box of the given width in percent.
fn column_box(width_percent: f32, align: AlignItems, gap: f32) -> Node {
    tweak(Node::default(), |node| {
        node.flex_direction = FlexDirection::Column;
        node.width = Val::Percent(width_percent);
        node.align_items = align;
        node.row_gap = Val::Px(gap);
    })
}

/// A fixed-width table column that never shrinks.
fn fixed_column(px: f32, justify: JustifyContent) -> Node {
    tweak(Node::default(), |node| {
        node.width = Val::Px(px);
        node.min_width = Val::Px(px);
        node.flex_shrink = 0.0;
        node.align_items = AlignItems::Center;
        node.justify_content = justify;
    })
}

fn row_box() -> Node {
    tweak(Node::default(), |node| {
        node.flex_direction = FlexDirection::Row;
        node.width = Val::Percent(100.0);
        node.height = Val::Px(ROW_HEIGHT);
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::SpaceBetween;
        node.column_gap = Val::Px(8.0);
    })
}

/// No wrapping in the table: a cell that wrapped would grow past its fixed
/// row height and its first line would ride higher than the other cells'.
fn single_line() -> TextLayout {
    TextLayout::new(Justify::Left, bevy::text::LineBreak::NoWrap)
}

fn text_cell(face: &Handle<Font>, kind: RowKind, col: CellColumn) -> impl Bundle {
    (
        Text::new(String::new()),
        TextColor(INK),
        match_typeface(face, CELL_SIZE),
        LoadingStatusCell { kind, col },
        single_line(),
        drop_shadow(),
    )
}

fn spawn_gap_row(commands: &mut Commands) -> Entity {
    commands.spawn(row_box()).id()
}

/// The box holding one column of a row; the name column takes whatever width
/// the number and unit columns leave and clips at its left edge.
fn column_holder(col: CellColumn) -> Node {
    match col {
        CellColumn::Name => tweak(Node::default(), |node| {
            node.flex_grow = 1.0;
            node.flex_shrink = 1.0;
            node.overflow = Overflow::clip_x();
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::FlexEnd;
            node.padding = UiRect::right(Val::Px(NAME_GUTTER));
        }),
        CellColumn::Number => fixed_column(NUMBER_COL_WIDTH, JustifyContent::FlexEnd),
        CellColumn::Unit => tweak(
            fixed_column(UNIT_COL_WIDTH, JustifyContent::FlexStart),
            |node| {
                node.padding = UiRect::left(Val::Px(6.0));
            },
        ),
    }
}

fn spawn_row(commands: &mut Commands, face: &Handle<Font>, kind: RowKind) -> Entity {
    let row = commands.spawn(row_box()).id();
    for col in [CellColumn::Name, CellColumn::Number, CellColumn::Unit] {
        let holder = commands
            .spawn(column_holder(col))
            .with_child(text_cell(face, kind, col))
            .id();
        commands.entity(row).add_child(holder);
    }
    row
}

fn spawn_title_block(
    into: &mut ChildSpawnerCommands,
    face: &Handle<Font>,
    screen: &LoadBackdrop,
) {
    let heading = screen.title();
    let size = letter_px(heading);
    into
        .spawn(column_box(48.0, AlignItems::FlexStart, 8.0))
        .with_children(|column| {
            column
                .spawn((
                    drop_shadow(),
                    TextLayout::default().with_justify(Justify::Left),
                    Text(String::new()),
                ))
                .with_children(|line| {
                    for (index, ch) in heading.chars().enumerate() {
                        line.spawn((
                            SplashGlyph(index),
                            TextSpan::new(String::from(ch)),
                            TextColor(UNLIT),
                            match_typeface(face, size),
                        ));
                    }
                });
            column.spawn((
                Text::new(String::from(screen.mode_caption())),
                TextColor(INK),
                match_typeface(face, SUBTITLE_SIZE),
                drop_shadow(),
                TextLayout::default().with_justify(Justify::Left),
            ));
        });
}

fn spawn_table_block(parent: &mut ChildSpawnerCommands) {
    let outer = tweak(column_box(48.0, AlignItems::Stretch, 2.0), |node| {
        node.justify_content = JustifyContent::FlexEnd;
    });
    parent.spawn(outer).with_child((
        column_box(100.0, AlignItems::Stretch, 2.0),
        SplashProgressLines,
    ));
}

/// Title on the left, table on the right, along the lower part of the screen.
fn spawn_content_band(
    root: &mut ChildSpawnerCommands,
    face: &Handle<Font>,
    screen: &LoadBackdrop,
) {
    let band = tweak(Node::default(), |node| {
        node.flex_direction = FlexDirection::Row;
        node.width = Val::Percent(78.0);
        node.margin = UiRect::bottom(Val::Percent(10.0));
        node.align_items = AlignItems::FlexEnd;
        node.justify_content = JustifyContent::SpaceBetween;
        node.column_gap = Val::Px(48.0);
    });
    root.spawn(band).with_children(|band| {
        spawn_title_block(band, face, screen);
        spawn_table_block(band);
    });
}

pub(crate) fn build_splash_scene(
    built_roots: Query<(Entity, Option<&SplashHeadline>), With<SplashSceneAnchor>>,
    backdrop_cams: Query<Entity, With<SplashLens>>,
    top_lenses: Query<Entity, With<OverlayLens>>,
    match_setup: Option<Res<session::Bo2mpMatchSetup>>,
    running_preview: Option<Res<SplashThumbJob>>,
    preview_source: Option<Res<LoadBackdropOrigin>>,
    font: Option<Res<MenuTypeface>>,
    screen: Option<Res<LoadBackdrop>>,
    mut commands: Commands,
    (front, mut pictures): (Option<Res<assets::T6HudIcons>>, ResMut<Assets<Image>>),
) {
    let screen = match screen {
        Some(screen) => screen,
        None => return,
    };
    let name = screen.title();

    // Nothing to do when the root already shows this title and no overlay
    // camera is left over.
    let current = built_roots
        .iter()
        .any(|(_, built_for)| built_for.is_some_and(|built| built.0 == name));
    if current && top_lenses.is_empty() {
        return;
    }
    let font = match font {
        Some(font) => font.0.clone(),
        None => {
            diag::warn!(Ui, "loading: no game UI font, loading screen not shown");
            return;
        }
    };

    built_roots
        .iter()
        .map(|pair| pair.0)
        .chain(backdrop_cams.iter())
        .chain(top_lenses.iter())
        .for_each(|gone| {
            commands.entity(gone).try_despawn();
        });

    ui_camera(&mut commands, 100, ClearColorConfig::Custom(SCREEN_BG)).insert(SplashLens);

    // bo2zm: a Black Ops II map shows its own loading picture, named as BO2's loading script
    // builds it: map, game type, start (`loadscreen_zm_prison_zgrief_cellblock`), read with the
    // front end. Then the map's picture for that game type, then any of the map's.
    let bo2_picture = preview_source.as_deref().and_then(|source| {
        let map = &source.level_name;
        let icons = &front.as_deref()?.0;
        let mut wanted = Vec::new();
        if let Some((location, mode)) = frame::bo2_zm_start(map) {
            wanted.push(format!("loadscreen_{map}_{mode}_{location}"));
            wanted.push(format!("loadscreen_{map}_{mode}"));
        }
        wanted.push(format!("loadscreen_{map}"));
        let (picture, image) = wanted.iter().find_map(|want| {
            let exact = icons.iter().find(|(n, _)| n == want);
            exact.or_else(|| icons.iter().find(|(n, _)| n.starts_with(want.as_str())))
        })?;
        diag::info!(Ui, "loading: using bo2 picture `{picture}`");
        Some(pictures.add((**image).clone()))
    });

    let root_node = tweak(Node::default(), |node| {
        node.flex_direction = FlexDirection::Column;
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::FlexEnd;
    });
    let mut backdrop = commands.spawn((
        SplashSceneAnchor,
        SplashHeadline(String::from(name)),
        root_node,
        BackgroundColor(SCREEN_BG),
        GlobalZIndex(SPLASH_Z),
        OverlayTier::Loading,
        OverlayStackShown,
        Visibility::Inherited,
    ));
    // A bo2mp match brings its own Loading menu (`lui_hud`) which is drawn on
    // top of this plain backdrop, so the title and table are left out there.
    if let Some(picture) = &bo2_picture {
        let stretched = ImageNode::new(picture.clone()).with_mode(bevy::ui::widget::NodeImageMode::Stretch);
        backdrop.insert(stretched);
    }
    if match_setup.is_none() && bo2_picture.is_none() {
        backdrop.with_children(|root| spawn_content_band(root, &font, &screen));
    }
    let mode = screen.mode_caption();
    diag::info!(Ui, "loading: screen ready for `{name}` ({mode})");

    let wanted = preview_source.filter(|source| {
        bo2_picture.is_none()
            && !running_preview
            .as_deref()
            .is_some_and(|task| task.which.is_for(source))
    });
    if let Some(source) = wanted {
        let progress = screen.progress.clone();
        start_preview_decode(&mut commands, &source, progress);
    }
}

fn start_preview_decode(
    commands: &mut Commands,
    source: &LoadBackdropOrigin,
    progress: LoadCompletion,
) {
    // No preview picture exists for the map yet, so there is nothing to
    // decode; the stage is marked skipped, which does not count as a failure.
    let job = async move {
        progress.begin(StageId::Preview, None).skip();
        None::<(u32, u32, Vec<u8>)>
    };
    commands.insert_resource(SplashThumbJob {
        task: bevy::tasks::AsyncComputeTaskPool::get().spawn(job),
        which: PreviewRequest {
            map: source.level_name.clone(),
            id: source.ticket,
        },
    });
}

/// Wraps `w`x`h` RGBA8 sRGB bytes in an image sampled with linear filtering.
fn preview_image(w: u32, h: u32, rgba: Vec<u8>) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource;

    let size = render_resource::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let flat = Image::new(
        size,
        render_resource::TextureDimension::D2,
        rgba,
        render_resource::TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    Image {
        sampler: bevy::image::ImageSampler::linear(),
        ..flat
    }
}

pub(crate) fn check_splash_thumb(
    root: Query<Entity, With<SplashSceneAnchor>>,
    source: Option<Res<LoadBackdropOrigin>>,
    mut screen_res: Option<ResMut<LoadBackdrop>>,
    mut pending: Option<ResMut<SplashThumbJob>>,
    mut pictures: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    use bevy::tasks::futures_lite::future;
    let finished = pending.as_deref_mut().and_then(|job| {
        let decoded = future::block_on(future::poll_once(&mut job.task))?;
        Some((job.which.clone(), decoded))
    });
    let Some((which, decoded)) = finished else {
        return;
    };
    commands.remove_resource::<SplashThumbJob>();

    // A result for a request that has since been replaced is dropped.
    let still_wanted = source.as_deref().is_some_and(|source| which.is_for(source));
    let Some(screen) = screen_res.as_deref_mut().filter(|_| still_wanted) else {
        return;
    };
    let Some((w, h, rgba)) = decoded else {
        diag::info!(Ui, "loading: map has no preview, plain color stays");
        return;
    };
    let Ok(root) = root.single() else {
        return;
    };

    let handle = pictures.add(preview_image(w, h, rgba));
    let picture = ImageNode::new(handle).with_mode(bevy::ui::widget::NodeImageMode::Stretch);
    commands.entity(root).insert(picture);
    screen.flag_showcase_done();
    diag::info!(Ui, "loading: map preview up, {w}x{h}");
}

/// A shown stage that no longer needs to stay visible.
fn has_settled(table: &LoadTable, id: StageId) -> bool {
    table
        .row(id)
        .is_some_and(|row| matches!(row.state, RowState::Done | RowState::Recycled))
}

/// The slot a newcomer may take: the first empty one, otherwise the first one
/// whose stage has finished.
fn free_slot(shown: &StatusTable, table: &LoadTable) -> Option<usize> {
    let vacant = shown.stage_slots.iter().position(|slot| slot.0.is_none());
    vacant.or_else(|| {
        shown
            .stage_slots
            .iter()
            .position(|slot| slot.0.is_some_and(|id| has_settled(table, id)))
    })
}

/// Hands out the on-screen stage_slots. A stage keeps its slot as long as it is
/// shown; a stage without one takes a free slot if there is one.
fn assign_slots(shown: &mut StatusTable, table: &LoadTable) {
    let wanted: Vec<StageId> = table
        .running
        .iter()
        .chain(table.ended.iter())
        .map(|row| row.id)
        .collect();
    for id in wanted {
        if shown.slot_showing(id).is_some() {
            continue;
        }
        if let Some(slot) = free_slot(shown, table) {
            shown.stage_slots[slot].0 = Some(id);
        }
    }
}

fn write_text(text: &mut Text, fresh: &str) {
    // Skip identical text: any write, even an equal one, re-runs text layout.
    if text.0 != fresh {
        text.0.clear();
        text.0.push_str(fresh);
    }
}

/// True (and the next deadline armed) once the table is due for new contents.
fn refresh_due(next_refresh: &mut Option<Instant>, now: Instant) -> bool {
    if matches!(*next_refresh, Some(due) if now < due) {
        return false;
    }
    *next_refresh = Some(now + TABLE_REFRESH);
    true
}

/// Replaces the table's rows with a fresh header, spacer and empty stage_slots.
fn rebuild_table(
    commands: &mut Commands,
    shown: &mut StatusTable,
    face: &Handle<Font>,
    list: Entity,
    ticket: u64,
) {
    // Swapping a parent's children only detaches the old rows, and a detached
    // row would linger as a root node, so they are despawned here.
    for old in shown.drain_entities() {
        commands.entity(old).try_despawn();
    }
    shown.for_request = Some(ticket);

    let head = spawn_row(commands, face, RowKind::Header);
    let spacer = spawn_gap_row(commands);
    let slot_rows: Vec<Entity> = (0..VISIBLE_ROWS)
        .map(|index| spawn_row(commands, face, RowKind::Stage(index)))
        .collect();

    shown.header = Some(head);
    shown.gap = Some(spacer);
    shown
        .stage_slots
        .extend(slot_rows.iter().map(|row| (None, *row)));

    let children: Vec<Entity> = [head, spacer].into_iter().chain(slot_rows).collect();
    commands.entity(list).replace_children(&children);
}

/// What a cell prints for the given column.
fn column_text<'a>(col: CellColumn, label: &'a str, metric: Option<&'a Metric>) -> &'a str {
    match (col, metric) {
        (CellColumn::Name, _) => label,
        (CellColumn::Number, Some(metric)) => metric.number.as_str(),
        (CellColumn::Unit, Some(metric)) => metric.unit.as_str(),
        (_, None) => "",
    }
}

/// Letter glyph in the title and the colour it is painted.
#[derive(QueryData)]
#[query_data(mutable)]
pub(crate) struct GlyphPaint {
    glyph: &'static SplashGlyph,
    tint: &'static mut TextColor,
}

/// A table cell with its text and colour.
#[derive(QueryData)]
#[query_data(mutable)]
pub(crate) struct CellPaint {
    cell: &'static LoadingStatusCell,
    text: &'static mut Text,
    tint: &'static mut TextColor,
}

/// The node holding the table rows, and the rows it has now.
#[derive(QueryData)]
pub(crate) struct ProgressList {
    id: Entity,
    kids: Option<&'static Children>,
}

pub(crate) fn advance_splash_scene(
    mut next_refresh: Local<Option<Instant>>,
    mut shown: Local<StatusTable>,
    clock: Res<Time>,
    mut commands: Commands,
    load: Option<Res<assets::MapLoadProcess>>,
    ui_font: Option<Res<MenuTypeface>>,
    mut screen_slot: Option<ResMut<LoadBackdrop>>,
    splash_roots: Query<Entity, Or<(With<SplashSceneAnchor>, With<SplashLens>)>>,
    progress_list: Query<ProgressList, With<SplashProgressLines>>,
    mut glyphs: Query<GlyphPaint, Without<LoadingStatusCell>>,
    mut cells: Query<CellPaint, Without<SplashGlyph>>,
) {
    let screen = match screen_slot.as_deref_mut() {
        Some(screen) => screen,
        None => return,
    };
    screen.advance_clock(clock.delta());

    // One letter is lit; every other one sits in the resting colour.
    let lit = lit_index(
        animation_step(screen.elapsed()),
        glyphs.iter().count().max(1),
    );
    let resting = match screen.backdrop_available() {
        true => UNLIT_READY,
        false => UNLIT,
    };
    for mut g in glyphs.iter_mut() {
        g.tint.0 = if g.glyph.0 == lit { LIT } else { resting };
    }

    let complete = screen.is_done();
    if complete {
        close_splash_overlay(&mut commands, splash_roots.iter(), [], true);
        diag::info!(Ui, "loading: screen going away");
        return;
    }

    if !refresh_due(&mut next_refresh, Instant::now()) {
        return;
    }

    // The real load process is authoritative; the screen's own progress is
    // only used when no process exists, and a newer request replaces the process.
    let progress = match load.as_deref() {
        Some(process) => &process.progress,
        None => &screen.progress,
    };
    let snapshot = progress.snapshot();
    let table = project(&snapshot);

    let Ok(list) = progress_list.single() else {
        return;
    };
    let Some(font) = ui_font.as_deref() else {
        return;
    };
    let list_is_empty = list.kids.map_or(true, |kids| kids.is_empty());
    if list_is_empty || shown.for_request != Some(snapshot.ask_id) {
        rebuild_table(
            &mut commands,
            &mut shown,
            &font.0,
            list.id,
            snapshot.ask_id,
        );
    }
    assign_slots(&mut shown, &table);

    let total = format_elapsed(total_elapsed(&snapshot));
    let (head_label, head_color) = screen.failure().map_or_else(
        || (String::from("Loading"), INK),
        |reason| (format!("Map load failed — {reason}"), INK_ERROR),
    );
    let slot_rows: Vec<Option<&LoadRow>> = shown
        .stage_slots
        .iter()
        .map(|(stage, _)| stage.and_then(|id| table.row(id)))
        .collect();

    for mut c in cells.iter_mut() {
        let (label, metric, paint) = match c.cell.kind {
            RowKind::Header => (head_label.as_str(), Some(&total), head_color),
            RowKind::Stage(index) => match slot_rows.get(index).copied().flatten() {
                Some(row) => (row.name.as_str(), Some(&row.value), row.state.color()),
                None => ("", None, INK),
            },
        };
        write_text(&mut c.text, column_text(c.cell.col, label, metric));
        c.tint.0 = paint;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_table::{LoadRow, LoadTable, Metric};

    fn entity_pool(n: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }

    fn stage_row(id: StageId, state: RowState) -> LoadRow {
        LoadRow {
            id,
            name: format!("{id:?}"),
            value: Metric {
                number: "1".into(),
                unit: "x".into(),
            },
            state,
        }
    }

    fn preview_source(ticket: u64, map: &str) -> LoadBackdropOrigin {
        LoadBackdropOrigin {
            path: std::path::PathBuf::new(),
            level_name: map.into(),
            ticket,
        }
    }

    fn table_with(running: Vec<LoadRow>, ended: Vec<LoadRow>) -> LoadTable {
        LoadTable { running, ended }
    }

    fn empty_slots(count: usize) -> StatusTable {
        let pool = entity_pool(count);
        StatusTable {
            stage_slots: pool.into_iter().map(|e| (None, e)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn short_titles_get_the_biggest_letters_long_ones_the_smallest() {
        assert_eq!(letter_px(""), 56.0);
        assert_eq!(letter_px("abcde"), 56.0);
        assert_eq!(letter_px("abcdefghij"), 56.0);
        assert_eq!(letter_px(&"a".repeat(20)), 28.0);
        assert_eq!(letter_px(&"a".repeat(40)), 28.0);
        let mid = letter_px(&"a".repeat(14));
        assert!((mid - 40.0).abs() < 1e-4, "{mid}");
    }

    #[test]
    fn letter_size_counts_characters_not_bytes() {
        assert_eq!(letter_px("äöüäöüäöüä"), letter_px("abcdefghij"));
    }

    #[test]
    fn lit_position_walks_out_and_back() {
        let walk: Vec<usize> = (0..10).map(|s| lit_index(s, 4)).collect();
        assert_eq!(walk, [0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
        assert_eq!(lit_index(0, 0), 0);
        assert_eq!(lit_index(77, 1), 0);
        assert_eq!(lit_index(5, 2), 1);
        assert_eq!(lit_index(6, 2), 0);
    }

    #[test]
    fn preview_request_matches_only_same_id_and_map() {
        let req = PreviewRequest {
            id: 7,
            map: "mp_a".into(),
        };
        let mut src = preview_source(7, "mp_a");
        assert!(req.is_for(&src));
        src.ticket = 8;
        assert!(!req.is_for(&src));
        src.ticket = 7;
        src.level_name = "mp_b".into();
        assert!(!req.is_for(&src));
    }

    #[test]
    fn set_text_only_writes_on_change() {
        let mut text = Text::new("one");
        write_text(&mut text, "two");
        assert_eq!(text.0, "two");
        write_text(&mut text, "two");
        assert_eq!(text.0, "two");
        write_text(&mut text, "");
        assert!(text.0.is_empty());
    }

    #[test]
    fn status_table_finds_slots_and_empties_itself() {
        let pool = entity_pool(5);
        let mut shown = StatusTable {
            for_request: Some(3),
            header: Some(pool[0]),
            gap: Some(pool[1]),
            stage_slots: vec![
                (None, pool[2]),
                (Some(StageId::Audio), pool[3]),
                (Some(StageId::Images), pool[4]),
            ],
        };
        assert_eq!(shown.slot_showing(StageId::Images), Some(2));
        assert_eq!(shown.slot_showing(StageId::Audio), Some(1));
        assert_eq!(shown.slot_showing(StageId::Shaders), None);
        let taken = shown.drain_entities();
        assert_eq!(taken, pool);
        assert!(shown.header.is_none() && shown.gap.is_none() && shown.stage_slots.is_empty());
        assert!(shown.drain_entities().is_empty());
    }

    #[test]
    fn new_rows_fill_empty_slots_in_order() {
        let mut shown = empty_slots(3);
        let table = table_with(
            vec![stage_row(StageId::Images, RowState::Running)],
            vec![stage_row(StageId::Audio, RowState::Done)],
        );
        assign_slots(&mut shown, &table);
        let held: Vec<_> = shown.stage_slots.iter().map(|s| s.0).collect();
        assert_eq!(held, [Some(StageId::Images), Some(StageId::Audio), None]);
    }

    #[test]
    fn a_row_keeps_its_slot_between_refreshes() {
        let mut shown = empty_slots(3);
        let first = table_with(
            vec![
                stage_row(StageId::Images, RowState::Running),
                stage_row(StageId::Audio, RowState::Running),
            ],
            vec![],
        );
        assign_slots(&mut shown, &first);
        let later = table_with(
            vec![stage_row(StageId::Audio, RowState::Running)],
            vec![stage_row(StageId::Images, RowState::Done)],
        );
        assign_slots(&mut shown, &later);
        assert_eq!(shown.slot_showing(StageId::Images), Some(0));
        assert_eq!(shown.slot_showing(StageId::Audio), Some(1));
    }

    #[test]
    fn a_full_table_recycles_only_finished_slots() {
        let mut shown = empty_slots(2);
        let table = table_with(
            vec![
                stage_row(StageId::Images, RowState::Done),
                stage_row(StageId::Audio, RowState::Running),
            ],
            vec![],
        );
        assign_slots(&mut shown, &table);
        let more = table_with(
            vec![
                stage_row(StageId::Audio, RowState::Running),
                stage_row(StageId::Shaders, RowState::Running),
            ],
            vec![stage_row(StageId::Images, RowState::Done)],
        );
        assign_slots(&mut shown, &more);
        // Shaders took over the finished Images slot; Audio is untouched.
        assert_eq!(shown.slot_showing(StageId::Shaders), Some(0));
        assert_eq!(shown.slot_showing(StageId::Audio), Some(1));
        assert_eq!(shown.slot_showing(StageId::Images), None);

        // Nothing finished means a newcomer gets no slot at all.
        let busy = table_with(
            vec![
                stage_row(StageId::Shaders, RowState::Running),
                stage_row(StageId::Audio, RowState::Running),
                stage_row(StageId::Programs, RowState::Running),
            ],
            vec![],
        );
        assign_slots(&mut shown, &busy);
        assert_eq!(shown.slot_showing(StageId::Programs), None);
    }

    #[test]
    fn reused_counts_as_finished_but_failed_does_not() {
        let mut shown = empty_slots(2);
        let table = table_with(
            vec![
                stage_row(StageId::Images, RowState::Recycled),
                stage_row(StageId::Audio, RowState::Failed),
            ],
            vec![],
        );
        assign_slots(&mut shown, &table);
        let next = table_with(
            vec![
                stage_row(StageId::Shaders, RowState::Running),
                stage_row(StageId::Programs, RowState::Running),
            ],
            vec![
                stage_row(StageId::Images, RowState::Recycled),
                stage_row(StageId::Audio, RowState::Failed),
            ],
        );
        assign_slots(&mut shown, &next);
        assert_eq!(shown.slot_showing(StageId::Shaders), Some(0));
        assert_eq!(shown.slot_showing(StageId::Audio), Some(1));
        assert_eq!(shown.slot_showing(StageId::Programs), None);
    }

    #[test]
    fn a_slot_whose_row_vanished_is_not_recycled() {
        let mut shown = empty_slots(1);
        assign_slots(
            &mut shown,
            &table_with(vec![stage_row(StageId::Images, RowState::Done)], vec![]),
        );
        assign_slots(
            &mut shown,
            &table_with(vec![stage_row(StageId::Audio, RowState::Running)], vec![]),
        );
        assert_eq!(shown.slot_showing(StageId::Images), Some(0));
        assert_eq!(shown.slot_showing(StageId::Audio), None);
    }

    fn count_with<T: Component>(world: &mut World) -> usize {
        world
            .query_filtered::<Entity, With<T>>()
            .iter(world)
            .count()
    }

    fn run_dismiss(world: &mut World, chrome: Vec<Entity>, cams: Vec<Entity>, keep: bool) {
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, world);
        close_splash_overlay(&mut commands, chrome, cams, keep);
        queue.apply(world);
    }

    #[test]
    fn dismissing_removes_chrome_and_loading_resources() {
        let mut world = World::new();
        let root = world.spawn(SplashSceneAnchor).id();
        let cam = world.spawn(OverlayLens).id();
        world.insert_resource(preview_source(1, "m"));
        run_dismiss(&mut world, vec![root], vec![cam], false);
        assert_eq!(count_with::<SplashSceneAnchor>(&mut world), 0);
        assert_eq!(count_with::<OverlayLens>(&mut world), 0);
        assert!(!world.contains_resource::<LoadBackdropOrigin>());
    }

    #[test]
    fn dismissing_keeps_or_creates_the_overlay_camera_when_asked() {
        let mut world = World::new();
        let cam = world.spawn(OverlayLens).id();
        run_dismiss(&mut world, vec![], vec![cam], true);
        assert_eq!(count_with::<OverlayLens>(&mut world), 1);
        assert!(world.get_entity(cam).is_ok());

        let mut bare = World::new();
        run_dismiss(&mut bare, vec![], vec![], true);
        assert_eq!(count_with::<OverlayLens>(&mut bare), 1);
        run_dismiss(&mut bare, vec![], vec![], false);
        assert_eq!(count_with::<OverlayLens>(&mut bare), 1);
    }
}
