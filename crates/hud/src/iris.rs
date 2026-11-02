//! The scope picture. While a gun that carries a scope overlay is aimed down
//! the sights, four picture quads are laid out around the crosshair, and four
//! plain-colour strips cover whatever part of the screen the picture leaves
//! bare.

use asset_game::{WeaponChassisSummary, WeaponLedger};
use assets::StagedWeapons;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::ui::BackgroundColor;
use bevy::ui::Display;
use bevy::ui::FocusPolicy;
use bevy::ui::Overflow;
use bevy::ui::PositionType;
use bevy::ui::ZIndex;
use bevy::ui::widget::ImageNode;
use hud_t6::iris::{
    AdsOverlayLayout, ScopeVignetteRect, WeaponOverlayDef, ads_overlay_material,
    draw_ads_overlay_layout, iris_overlay_configured,
};
use net::{HostViewClient, ShownWorldFrame, ViewweaponAim};

use crate::gaps::GapReason;
use crate::gaps::HudGapBook;
use crate::gaps::HudShortage;
use crate::images::{CHROME_NAMESPACE, HudPictures};
use crate::screen_fit::UnitClass;
use crate::screen_fit::WindowScale;
use crate::aim_mark::CrosshairScopeHold;
use crate::surface::FlatOverlayCanvas;
use crate::ui_emit::take_over_monitor;

/// Alpha at or above this reads as solid picture.
const SOLID_ALPHA: u8 = 250;
/// Colour channels at or below this make a texel count as black.
const BLACK_CHANNEL: u8 = 16;
/// The strips start this many texels inside the picture's outer edge, so
/// they hide the filtered fringe.
const FRINGE_TEXELS: f32 = 1.5;

/// What the strips were last coloured from: the picture's name, and what
/// sampling it gave (`None` inside means the sample failed and is not retried).
#[derive(Resource, Default)]
pub(crate) struct ScopeStripFill {
    last: Option<(String, Option<Fill>)>,
}

/// A strip colour together with the one-texel image that carries it.
#[derive(Clone)]
struct Fill {
    solid: Handle<Image>,
    texel: [u8; 4],
}

/// One of the four picture quads.
#[derive(Component, Clone, Copy)]
pub(crate) struct ScopeQuad(u8);

/// One of the four colour strips (left, right, top, bottom).
#[derive(Component, Clone, Copy)]
pub(crate) struct ScopeStrip(u8);

type ScopeQuadQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Node,
        &'static mut ImageNode,
        &'static ScopeQuad,
    ),
    (With<ScopeQuad>, Without<ScopeStrip>),
>;

type ScopeStripQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Node,
        &'static mut ImageNode,
        &'static mut BackgroundColor,
        &'static ScopeStrip,
    ),
    Without<ScopeQuad>,
>;

/// A rectangle in whole window pixels, stored as its four edges.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Edges {
    left_x: f32,
    top_y: f32,
    right_x: f32,
    bottom_y: f32,
}

impl Edges {
    fn width(self) -> f32 {
        self.right_x - self.left_x
    }

    fn height(self) -> f32 {
        self.bottom_y - self.top_y
    }

    fn is_visible(self) -> bool {
        self.width() > 0.0 && self.height() > 0.0
    }
}

/// Where authored scope units land on the window: centre point and scale.
#[derive(Clone, Copy, Debug)]
struct ScopeFrame {
    origin_x: f32,
    origin_y: f32,
    factor: f32,
}

impl ScopeFrame {
    /// The window rectangle covered by an authored rect, rounded to pixels.
    fn pixels(self, x: f32, y: f32, w: f32, h: f32) -> Edges {
        let at = |offset: f32, origin: f32| (origin + offset * self.factor).round();
        Edges {
            left_x: at(x, self.origin_x),
            top_y: at(y, self.origin_y),
            right_x: at(x + w, self.origin_x),
            bottom_y: at(y + h, self.origin_y),
        }
    }

    /// The frame centred on the window, shifted by the crosshair offset (in
    /// authored units) when the gun is driving one.
    fn around_crosshair(window: &WindowScale, offset: Option<(f32, f32)>) -> Self {
        let factor = window.unit_px(UnitClass::Projection);
        let (step_along_x, dy) = offset.unwrap_or_default();
        Self {
            origin_x: window.width() * 0.5 + step_along_x * factor,
            origin_y: window.height() * 0.5 + dy * factor,
            factor,
        }
    }
}

/// The picture bounds pulled in by `inset` on each side but never outside the
/// window, and never inside-out.
fn pulled_in(bounds: Edges, inset: (f32, f32), window: (f32, f32)) -> Edges {
    let (left, right) = pulled_span(bounds.left_x, bounds.right_x, inset.0, window.0);
    let (top, bottom) = pulled_span(bounds.top_y, bounds.bottom_y, inset.1, window.1);
    Edges {
        left_x: left,
        top_y: top,
        right_x: right,
        bottom_y: bottom,
    }
}

/// One axis of `pulled_in`: both ends move toward each other by `inset`, stay
/// within `0..=limit`, and the far end never passes the near one.
fn pulled_span(near: f32, far: f32, inset: f32, limit: f32) -> (f32, f32) {
    let near = (near + inset).clamp(0.0, limit);
    (near, (far - inset).clamp(near, limit))
}

/// The four strips around `hole`: left and right run the full screen height,
/// top and bottom fill the span between them. `None` where a strip would be
/// empty.
fn strips_around(hole: Edges, screen: (f32, f32)) -> [Option<Edges>; 4] {
    let (screen_w, screen_h) = screen;
    let Edges {
        left_x: left,
        top_y: top,
        right_x: right,
        bottom_y: bottom,
    } = hole;
    [
        Edges {
            left_x: 0.0,
            top_y: 0.0,
            right_x: left,
            bottom_y: screen_h,
        },
        Edges {
            left_x: right,
            top_y: 0.0,
            right_x: screen_w,
            bottom_y: screen_h,
        },
        Edges {
            left_x: left,
            top_y: 0.0,
            right_x: right,
            bottom_y: top,
        },
        Edges {
            left_x: left,
            top_y: bottom,
            right_x: right,
            bottom_y: screen_h,
        },
    ]
    .map(|edges| Some(edges).filter(|e| e.is_visible()))
}

/// How many pixels of a quad's span one picture texel (and half again, for the
/// filter blur) covers, rounded up.
fn texel_margin(span: f32, factor: f32, texels: u32) -> f32 {
    (span * factor * FRINGE_TEXELS / texels as f32).ceil()
}

/// The picture's strips-free middle, as a window rectangle: the whole layout's
/// bounds pulled in past the outermost texel and its linear-filter blur.
fn picture_hole(
    layout: &AdsOverlayLayout,
    first: &ScopeVignetteRect,
    frame: ScopeFrame,
    picture: (u32, u32),
    window: (f32, f32),
) -> Edges {
    let (x, y, w, h) = layout.rect();
    let inset = (
        texel_margin(first.w, frame.factor, picture.0),
        texel_margin(first.h, frame.factor, picture.1),
    );
    pulled_in(frame.pixels(x, y, w, h), inset, window)
}

/// The colour a strip is painted: the fill's colour at `alpha`.
fn strip_tint(texel: [u8; 4], alpha: f32) -> Color {
    let [r, g, b, _] = texel;
    Color::srgb_u8(r, g, b).with_alpha(alpha)
}

/// A node that starts hidden and is positioned by absolute offsets.
fn hidden_absolute_image() -> (Node, ImageNode, FocusPolicy) {
    let node = Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        overflow: Overflow::visible(),
        ..default()
    };
    (node, ImageNode::default(), FocusPolicy::Pass)
}

/// Strips sit behind the quads, hence the lower layer.
fn strip_bundle(index: u8) -> impl Bundle {
    (
        ScopeStrip(index),
        hidden_absolute_image(),
        BackgroundColor(Color::NONE),
        ZIndex(0),
    )
}

fn quad_bundle(index: u8) -> impl Bundle {
    (ScopeQuad(index), hidden_absolute_image(), ZIndex(1))
}

pub(crate) fn spawn_scope(parent: &mut ChildSpawnerCommands) {
    (0..4u8).for_each(|index| {
        parent.spawn(strip_bundle(index));
    });
    (0..4u8).for_each(|index| {
        parent.spawn(quad_bundle(index));
    });
}

fn conceal_quad(node: &mut Node, image: &mut ImageNode) {
    take_over_monitor(node, Display::None);
    (image.flip_x, image.flip_y) = (false, false);
}

fn conceal_strip(node: &mut Node, background: &mut BackgroundColor) {
    take_over_monitor(node, Display::None);
    background.0 = Color::NONE;
}

fn conceal_scope(quads: &mut ScopeQuadQuery, strips: &mut ScopeStripQuery) {
    quads
        .iter_mut()
        .for_each(|(mut node, mut image, _)| conceal_quad(&mut node, &mut image));
    conceal_all_strips(strips);
}

fn conceal_all_strips(strips: &mut ScopeStripQuery) {
    strips
        .iter_mut()
        .for_each(|(mut node, _, mut background, _)| conceal_strip(&mut node, &mut background));
}

/// Shows `node` covering `area`.
fn cover(node: &mut Node, area: Edges) {
    take_over_monitor(node, Display::Flex);
    (node.left, node.top) = (Val::Px(area.left_x), Val::Px(area.top_y));
    (node.width, node.height) = (Val::Px(area.width()), Val::Px(area.height()));
}

/// Points `image` at `source`, drawn white at `alpha` and uncropped.
fn paint(image: &mut ImageNode, source: &Handle<Image>, alpha: f32) {
    image.image = source.clone();
    image.color = Color::srgba(1.0_f32, 1.0, 1.0, alpha);
    image.rect = None;
}

/// The strip colour: scanning the picture's middle row, the first solid
/// near-black texel wins; failing that, the first solid texel.
pub(crate) fn edge_texel(w: u32, h: u32, texels: &[u8]) -> Option<[u8; 4]> {
    if w == 0 || h == 0 {
        return None;
    }
    let row_first_cell = h as usize / 2 * w as usize * 4;
    let row = texels.get(row_first_cell..)?;
    let solid_texels = || {
        row.chunks_exact(4)
            .take(w as usize)
            .filter_map(|chunk| <[u8; 4]>::try_from(chunk).ok())
            .filter(|texel| texel[3] >= SOLID_ALPHA)
    };
    solid_texels()
        .find(|texel| texel[..3].iter().all(|&c| c <= BLACK_CHANNEL))
        .or_else(|| solid_texels().next())
}

fn one_texel_image(texel: [u8; 4], image_store: &mut Assets<Image>) -> Handle<Image> {
    let size = Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    image_store.add(Image::new_fill(
        size,
        TextureDimension::D2,
        &texel,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}

/// Which reticle the overlay shows: none without an overlay material, else the
/// authored index, or the first one when it is unset or negative.
fn reticle_slot(has_overlay: bool, authored: i32) -> u32 {
    if !has_overlay {
        return 0;
    }
    match u32::try_from(authored) {
        Ok(index) if index > 0 => index,
        _ => 1,
    }
}

/// The overlay definition for a gun. Only whether an overlay material exists
/// matters here (the picture itself is found by name in `gather_scope`), so
/// that goes in as 1 or 0.
pub(crate) fn scope_def(has_overlay: bool, facts: &WeaponChassisSummary) -> WeaponOverlayDef {
    WeaponOverlayDef {
        hud_layer_surface: has_overlay.into(),
        hud_crosshair_layer: reticle_slot(has_overlay, facts.hud_crosshair_layer),
        overlay_width: facts.scope_pic_w,
        overlay_height: facts.scope_pic_h,
        overlay_alpha_scale: facts.overlay_alpha_scale,
        infrared: facts.infrared,
        ..WeaponOverlayDef::default()
    }
}

/// Progress through the zoom, or `None` if the gun has no overlay or is not
/// zoomed. Heading into the sights reads the zoom-in window, otherwise the
/// zoom-out one.
pub(crate) fn scope_zoom(
    def: &WeaponOverlayDef,
    facts: &WeaponChassisSummary,
    pos_frac: f32,
    into_sights: bool,
) -> Option<f32> {
    iris_overlay_configured(def)
        .then(|| scope_window(facts.aim_in_share, facts.aim_out_share, pos_frac, into_sights))
        .and_then(|window| weapon_t6::aim::reticle_zoom(pos_frac, window))
}

/// Which share of the travel `CG_GetWeapReticleZoom` reads. It takes the
/// zoom-in share only while `bPositionToADS` says the gun is moving into the
/// sights, and a gun resting in them is not: Black Ops II's snipers have a
/// zoom-in share of 0 (`dsr50_mp`, `svu_mp`, the XPR-50; zoom-out 0.05), so
/// read through the zoom-in window they would never show their scope. Fully
/// in, every other gun comes out at 1 through either window.
pub(crate) fn scope_window(in_share: f32, out_share: f32, pos_frac: f32, into_sights: bool) -> f32 {
    if into_sights && pos_frac < 1.0 { in_share } else { out_share }
}

/// Why nothing is drawn this frame.
enum Hold {
    /// Nothing to show and nothing wrong; the gap, if raised, is cleared.
    Quiet,
    /// Something needed is missing; the reason is raised.
    Gap(GapReason),
}

impl Hold {
    /// Records this outcome in the gap book.
    fn settle(self, gaps: &mut HudGapBook) {
        match self {
            Hold::Quiet => gaps.settle(HudShortage::AdsScope),
            Hold::Gap(reason) => gaps.log_gap(reason),
        }
    }
}

/// Everything the scope needs this frame.
struct ScopeInputs {
    facts: WeaponChassisSummary,
    zoom: f32,
    picture_name: String,
    picture: Handle<Image>,
}

/// The gun being held, once it is known to be scoped in and drawing.
struct AimedGun<'a> {
    catalog: &'a WeaponLedger,
    held: u32,
    facts: WeaponChassisSummary,
    zoom: f32,
}

/// Checks that the held gun is zoomed with an overlay material to show.
fn aimed_gun<'a>(
    surface: &FlatOverlayCanvas,
    presented: &ShownWorldFrame,
    local: &HostViewClient,
    weapons: Option<&'a StagedWeapons>,
    latch: &CrosshairScopeHold,
) -> Result<AimedGun<'a>, Hold> {
    let player = presented.player(local.0).ok_or(Hold::Quiet)?;
    let held = crate::aim_mark::viewmodel_weapon_index(player);
    let catalog = &weapons.ok_or(Hold::Quiet)?.0;
    let facts = catalog.summary_for(held).ok_or(Hold::Quiet)?;
    let def = scope_def(catalog.scope_is_iris(held), &facts);
    surface.has_size().then_some(()).ok_or(Hold::Quiet)?;
    let zoom = scope_zoom(
        &def,
        &facts,
        player.weapon_raise_fraction,
        latch.place_for_scope,
    )
    .ok_or(Hold::Quiet)?;
    // The arguments after the height say "not jammed"; EMP state is not fed in
    // yet. When jammed, an infrared scope swaps to the shared EMP material.
    match ads_overlay_material(&def, surface.height() as u32, false, false, false, 0) {
        0 => Err(Hold::Quiet),
        _ => Ok(AimedGun {
            catalog,
            held,
            facts,
            zoom,
        }),
    }
}

/// The Black Ops II picture: the overlay material's colour map out of the
/// decoded HUD icons, stored as `overlay:<material>`.
fn t6_scope_picture(
    catalog: &WeaponLedger,
    held: u32,
    pictures: &HudPictures,
    image_store: &mut Assets<Image>,
    t6_icons: Option<&assets::T6HudIcons>,
    t6_cache: &mut Vec<(String, Handle<Image>)>,
) -> Result<(String, Handle<Image>), GapReason> {
    let material = catalog.scope_surface_for(held).unwrap_or_default();
    let key = format!("overlay:{material}");
    let found = t6_overlay_handle(&key, material.is_empty(), t6_icons, t6_cache, image_store);
    found
        .map(|handle| (key.clone(), handle))
        .ok_or_else(|| GapReason::ScopeImageAbsent {
            asset_key: key,
            miss: pictures.lookup_failure(),
        })
}

/// Any other gun names its picture directly.
fn named_scope_picture(
    catalog: &WeaponLedger,
    held: u32,
    namespace: asset_core::AssetCategoryScope,
    pictures: &mut HudPictures,
    image_store: &mut Assets<Image>,
) -> Result<(String, Handle<Image>), GapReason> {
    let name = catalog
        .scope_picture_for(held)
        .ok_or_else(|| GapReason::ScopeUnnamed {
            material: catalog.scope_surface_for(held).map(str::to_owned),
        })?;
    match pictures.get(namespace, name, image_store) {
        Some(handle) => Ok((name.to_owned(), handle)),
        None => Err(GapReason::ScopeImageAbsent {
            asset_key: name.to_owned(),
            miss: pictures.lookup_failure(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn gather_scope(
    surface: &FlatOverlayCanvas,
    presented: &ShownWorldFrame,
    local: &HostViewClient,
    weapons: Option<&StagedWeapons>,
    latch: &CrosshairScopeHold,
    pictures: &mut HudPictures,
    image_store: &mut Assets<Image>,
    t6_icons: Option<&assets::T6HudIcons>,
    t6_cache: &mut Vec<(String, Handle<Image>)>,
) -> Result<ScopeInputs, Hold> {
    let AimedGun {
        catalog,
        held,
        facts,
        zoom,
    } = aimed_gun(surface, presented, local, weapons, latch)?;
    let sized = facts.scope_pic_w > 0.0 && facts.scope_pic_h > 0.0;
    if !sized {
        return Err(Hold::Gap(GapReason::ScopeNoSize));
    }
    let namespace = catalog.scope_for(held).unwrap_or(CHROME_NAMESPACE);
    let picked = if namespace == asset_core::AssetCategoryScope::T6 {
        t6_scope_picture(catalog, held, pictures, image_store, t6_icons, t6_cache)
    } else {
        named_scope_picture(catalog, held, namespace, pictures, image_store)
    };
    let (picture_name, picture) = picked.map_err(Hold::Gap)?;
    Ok(ScopeInputs {
        facts,
        zoom,
        picture_name,
        picture,
    })
}

/// Moves each picture quad to its slot in the laid-out scope.
fn place_quads(
    quads: &mut ScopeQuadQuery,
    layout: &[ScopeVignetteRect],
    frame: ScopeFrame,
    picture: &Handle<Image>,
    alpha: f32,
) {
    for (mut cell, mut image, slot) in quads.iter_mut() {
        match layout.get(usize::from(slot.0)) {
            None => conceal_quad(&mut cell, &mut image),
            Some(&ScopeVignetteRect {
                x,
                y,
                w,
                h,
                s0,
                s1,
                t0,
                t1,
                ..
            }) => {
                cover(&mut cell, frame.pixels(x, y, w, h));
                paint(&mut image, picture, alpha);
                (image.flip_x, image.flip_y) = (s0 > s1, t0 > t1);
            }
        }
    }
}

/// Colours the strips around `hole`.
fn place_strips(
    strips: &mut ScopeStripQuery,
    hole: Edges,
    screen: (f32, f32),
    fill: &Fill,
    alpha: f32,
) {
    let tint = strip_tint(fill.texel, alpha);
    let areas = strips_around(hole, screen);
    for (mut cell, mut image, mut background, which) in strips.iter_mut() {
        let area = areas.get(usize::from(which.0)).copied().flatten();
        let Some(area) = area else {
            conceal_strip(&mut cell, &mut background);
            continue;
        };
        cover(&mut cell, area);
        paint(&mut image, &fill.solid, alpha);
        background.0 = tint;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_scope(
    surface: Res<FlatOverlayCanvas>,
    shown_frame: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    staged: Option<Res<StagedWeapons>>,
    latch: Res<CrosshairScopeHold>,
    mut pictures: ResMut<HudPictures>,
    mut image_store: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudGapBook>,
    mut fill: ResMut<ScopeStripFill>,
    aim: Res<ViewweaponAim>,
    (t6_icons, mut t6_cache): (
        Option<Res<assets::T6HudIcons>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
    mut quads: ScopeQuadQuery,
    mut strips: ScopeStripQuery,
) {
    let gathered = gather_scope(
        &surface,
        &shown_frame,
        &local,
        staged.as_deref(),
        &latch,
        &mut pictures,
        &mut image_store,
        t6_icons.as_deref(),
        &mut t6_cache,
    );
    let ScopeInputs {
        facts,
        zoom,
        picture_name,
        picture,
    } = match gathered {
        Ok(inputs) => inputs,
        Err(hold) => {
            hold.settle(&mut gaps);
            conceal_scope(&mut quads, &mut strips);
            return;
        }
    };
    gaps.settle(HudShortage::AdsScope);

    let tall = surface.height();
    let window = WindowScale::for_window(surface.width(), tall);
    // The picture follows the crosshair, which follows the gun.
    let frame =
        ScopeFrame::around_crosshair(&window, aim.live.then_some((aim.crosshair_x, aim.crosshair_y)));
    let alpha = weapon_t6::aim::overlay_alphas(zoom, facts.overlay_alpha_scale).0;
    // Authored units around (0, 0); `frame` moves them. No split screen.
    let layout = draw_ads_overlay_layout(
        0.0,
        0.0,
        facts.scope_pic_w,
        facts.scope_pic_h,
        false,
        alpha,
    );
    let slots: &[ScopeVignetteRect] = match &layout {
        AdsOverlayLayout::Four(four) => four,
        AdsOverlayLayout::One { quad, .. } => std::slice::from_ref(quad),
    };
    place_quads(&mut quads, slots, frame, &picture, alpha);

    let strip_paint = resolve_fill(&mut fill, &picture_name, &picture, &mut image_store).zip(
        image_store
            .get(&picture)
            .map(|image| (image.width(), image.height())),
    );
    let Some((fill, picture_size)) = strip_paint else {
        conceal_all_strips(&mut strips);
        return;
    };
    let win = (window.width(), window.height());
    let hole = picture_hole(&layout, &slots[0], frame, picture_size, win);
    place_strips(
        &mut strips,
        hole,
        (win.0.round(), win.1.round()),
        &fill,
        alpha,
    );
}

/// The strip colour for the picture called `picture_name`, sampled once and
/// kept until a different picture is shown.
fn resolve_fill(
    memo: &mut ScopeStripFill,
    picture_name: &str,
    picture: &Handle<Image>,
    image_store: &mut Assets<Image>,
) -> Option<Fill> {
    if let Some((seen, outcome)) = &memo.last
        && seen == picture_name
    {
        return outcome.clone();
    }
    let outcome = sample_fill(picture, image_store);
    memo.last = Some((picture_name.to_owned(), outcome.clone()));
    outcome
}

/// Reads the strip colour off `picture` and makes its one-texel image.
fn sample_fill(picture: &Handle<Image>, image_store: &mut Assets<Image>) -> Option<Fill> {
    let source = image_store.get(picture)?;
    let texel = edge_texel(source.width(), source.height(), source.data.as_deref()?)?;
    Some(Fill {
        solid: one_texel_image(texel, image_store),
        texel,
    })
}

/// A Black Ops II scope picture as a HUD image. The copy keeps its pixels on
/// the CPU as well, since the strip colour is read back from them.
fn t6_overlay_handle(
    key: &str,
    unnamed: bool,
    icons: Option<&assets::T6HudIcons>,
    cache: &mut Vec<(String, Handle<Image>)>,
    image_store: &mut Assets<Image>,
) -> Option<Handle<Image>> {
    if unnamed {
        return None;
    }
    let cached = cache
        .iter()
        .find_map(|(name, handle)| (name == key).then(|| handle.clone()));
    cached.or_else(|| {
        let source = icons?
            .0
            .iter()
            .find_map(|(name, image)| (name == key).then_some(image))?;
        let handle = image_store.add(Image {
            asset_usage: RenderAssetUsages::default(),
            ..(**source).clone()
        });
        cache.push((key.to_owned(), handle.clone()));
        Some(handle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texels(row: &[[u8; 4]]) -> Vec<u8> {
        row.iter().flatten().copied().collect()
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Edges {
        Edges {
            left_x: left,
            top_y: top,
            right_x: right,
            bottom_y: bottom,
        }
    }

    #[test]
    fn a_sniper_resting_in_its_sights_shows_the_scope() {
        // The DSR-50: zoom in 0, zoom out 0.05.
        let at_rest = scope_window(0.0, 0.05, 1.0, true);
        assert_eq!(weapon_t6::aim::reticle_zoom(1.0, at_rest), Some(1.0));
        // On the way in it stays off until it arrives.
        assert_eq!(weapon_t6::aim::reticle_zoom(0.9, scope_window(0.0, 0.05, 0.9, true)), None);
        // An ACOG (in 0.5) reads its own window on the way in.
        assert_eq!(scope_window(0.5, 0.4, 0.8, true), 0.5);
        assert_eq!(scope_window(0.5, 0.4, 0.8, false), 0.4);
    }

    #[test]
    fn edge_texel_prefers_the_first_solid_black() {
        let data = texels(&[[200, 10, 10, 255], [4, 4, 4, 255], [0, 0, 0, 255]]);
        assert_eq!(edge_texel(3, 1, &data), Some([4, 4, 4, 255]));
    }

    #[test]
    fn edge_texel_falls_back_to_the_first_solid_texel() {
        let data = texels(&[[9, 9, 9, 100], [200, 10, 10, 255], [90, 90, 90, 255]]);
        assert_eq!(edge_texel(3, 1, &data), Some([200, 10, 10, 255]));
    }

    #[test]
    fn edge_texel_reads_the_middle_row() {
        let top = [255, 255, 255, 255];
        let middle = [1, 2, 3, 255];
        let data = texels(&[top, middle, top]);
        assert_eq!(edge_texel(1, 3, &data), Some(middle));
    }

    #[test]
    fn edge_texel_rejects_empty_and_short_data() {
        assert_eq!(edge_texel(0, 4, &[]), None);
        assert_eq!(edge_texel(4, 0, &[]), None);
        assert_eq!(edge_texel(2, 2, &[0; 4]), None);
        assert_eq!(
            edge_texel(2, 1, &texels(&[[1, 1, 1, 0], [2, 2, 2, 0]])),
            None
        );
    }

    #[test]
    fn frame_rounds_to_whole_pixels() {
        let frame = ScopeFrame {
            origin_x: 100.0,
            origin_y: 50.0,
            factor: 1.5,
        };
        assert_eq!(
            frame.pixels(-10.0, -4.0, 20.0, 8.0),
            rect(85.0, 44.0, 115.0, 56.0)
        );
        assert_eq!(
            frame.pixels(0.3, 0.3, 1.0, 1.0),
            rect(100.0, 50.0, 102.0, 52.0)
        );
    }

    #[test]
    fn pulled_in_stays_inside_the_window_and_right_way_round() {
        let window = (200.0, 100.0);
        assert_eq!(
            pulled_in(rect(20.0, 10.0, 180.0, 90.0), (4.0, 2.0), window),
            rect(24.0, 12.0, 176.0, 88.0)
        );
        let off = pulled_in(rect(-30.0, -30.0, 400.0, 400.0), (5.0, 5.0), window);
        assert_eq!(off, rect(0.0, 0.0, 200.0, 100.0));
        let folded = pulled_in(rect(50.0, 40.0, 52.0, 42.0), (10.0, 10.0), window);
        assert!(folded.right_x >= folded.left_x && folded.bottom_y >= folded.top_y);
    }

    #[test]
    fn four_strips_surround_a_centred_hole() {
        let strips = strips_around(rect(40.0, 20.0, 160.0, 80.0), (200.0, 100.0));
        assert_eq!(strips[0], Some(rect(0.0, 0.0, 40.0, 100.0)));
        assert_eq!(strips[1], Some(rect(160.0, 0.0, 200.0, 100.0)));
        assert_eq!(strips[2], Some(rect(40.0, 0.0, 160.0, 20.0)));
        assert_eq!(strips[3], Some(rect(40.0, 80.0, 160.0, 100.0)));
    }

    #[test]
    fn a_hole_that_fills_the_screen_needs_no_strips() {
        let strips = strips_around(rect(0.0, 0.0, 200.0, 100.0), (200.0, 100.0));
        assert!(strips.iter().all(Option::is_none));
    }

    #[test]
    fn scope_def_marks_overlay_presence() {
        let facts = WeaponChassisSummary::default();
        assert_eq!(scope_def(false, &facts).hud_layer_surface, 0);
        assert_eq!(scope_def(false, &facts).hud_crosshair_layer, 0);
        let on = scope_def(true, &facts);
        assert_eq!(on.hud_layer_surface, 1);
        assert_eq!(on.hud_crosshair_layer, 1);
    }

    #[test]
    fn scope_def_keeps_an_authored_reticle_index() {
        let facts = WeaponChassisSummary {
            hud_crosshair_layer: 3,
            ..WeaponChassisSummary::default()
        };
        assert_eq!(scope_def(true, &facts).hud_crosshair_layer, 3);
        let negative = WeaponChassisSummary {
            hud_crosshair_layer: -2,
            ..WeaponChassisSummary::default()
        };
        assert_eq!(scope_def(true, &negative).hud_crosshair_layer, 1);
    }

    #[test]
    fn covering_a_node_shows_it_at_the_area() {
        let mut node = Node {
            display: Display::None,
            ..default()
        };
        cover(&mut node, rect(10.0, 20.0, 40.0, 60.0));
        assert_eq!(node.display, Display::Flex);
        assert_eq!(node.left, Val::Px(10.0));
        assert_eq!(node.top, Val::Px(20.0));
        assert_eq!(node.width, Val::Px(30.0));
        assert_eq!(node.height, Val::Px(40.0));
    }

    #[test]
    fn pulled_span_never_inverts_and_stays_in_the_window() {
        assert_eq!(pulled_span(10.0, 90.0, 5.0, 100.0), (15.0, 85.0));
        assert_eq!(pulled_span(-20.0, 500.0, 5.0, 100.0), (0.0, 100.0));
        let (near, far) = pulled_span(40.0, 42.0, 9.0, 100.0);
        assert!(far >= near);
    }

    #[test]
    fn texel_margin_rounds_up() {
        assert_eq!(texel_margin(100.0, 1.0, 100), 2.0);
        assert_eq!(texel_margin(100.0, 1.0, 150), 1.0);
        assert_eq!(texel_margin(10.0, 2.0, 256), 1.0);
    }

    #[test]
    fn crosshair_frame_centres_then_shifts() {
        let window = WindowScale::for_window(1280.0, 720.0);
        let still = ScopeFrame::around_crosshair(&window, None);
        assert_eq!((still.origin_x, still.origin_y), (640.0, 360.0));
        let moved = ScopeFrame::around_crosshair(&window, Some((2.0, -3.0)));
        assert_eq!(moved.factor, still.factor);
        assert_eq!(moved.origin_x, 640.0 + 2.0 * still.factor);
        assert_eq!(moved.origin_y, 360.0 - 3.0 * still.factor);
    }

    #[test]
    fn strip_tint_keeps_the_colour_and_takes_the_alpha() {
        let tint = strip_tint([255, 0, 51, 7], 0.5).to_srgba();
        assert_eq!(
            (tint.red, tint.green, tint.blue, tint.alpha),
            (1.0, 0.0, 0.2, 0.5)
        );
    }

    #[test]
    fn reticle_slot_defaults_to_the_first() {
        assert_eq!(reticle_slot(false, 5), 0);
        assert_eq!(reticle_slot(true, 0), 1);
        assert_eq!(reticle_slot(true, -1), 1);
        assert_eq!(reticle_slot(true, 4), 4);
    }

    #[test]
    fn zoom_window_follows_the_direction_of_travel() {
        let facts = WeaponChassisSummary::default();
        let unconfigured = scope_def(false, &facts);
        assert_eq!(scope_zoom(&unconfigured, &facts, 0.5, true), None);
        let def = scope_def(true, &facts);
        let expect = |window| weapon_t6::aim::reticle_zoom(0.5, window);
        assert_eq!(
            scope_zoom(&def, &facts, 0.5, true),
            expect(facts.aim_in_share)
        );
        assert_eq!(
            scope_zoom(&def, &facts, 0.5, false),
            expect(facts.aim_out_share)
        );
    }

    fn flat_image(texel: [u8; 4], usage: RenderAssetUsages) -> Image {
        Image::new_fill(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &texel,
            TextureFormat::Rgba8UnormSrgb,
            usage,
        )
    }

    #[test]
    fn fill_is_sampled_once_per_picture_name() {
        let mut image_store = Assets::<Image>::default();
        let mut memo = ScopeStripFill::default();
        let red = image_store.add(flat_image([200, 10, 10, 255], RenderAssetUsages::default()));
        let first = resolve_fill(&mut memo, "scope", &red, &mut image_store).expect("sampled");
        assert_eq!(first.texel, [200, 10, 10, 255]);
        let count = image_store.len();
        // Same name: served from the memo, nothing new is made.
        let again = resolve_fill(&mut memo, "scope", &red, &mut image_store).expect("memo");
        assert_eq!(again.solid, first.solid);
        assert_eq!(image_store.len(), count);
        // A different name samples again.
        let other = resolve_fill(&mut memo, "other", &red, &mut image_store).expect("resampled");
        assert_ne!(other.solid, first.solid);
    }

    #[test]
    fn a_failed_sample_is_remembered_by_name() {
        let mut image_store = Assets::<Image>::default();
        let mut memo = ScopeStripFill::default();
        let missing = Handle::<Image>::default();
        assert!(resolve_fill(&mut memo, "gone", &missing, &mut image_store).is_none());
        assert!(matches!(&memo.last, Some((seen, None)) if seen == "gone"));
    }

    #[test]
    fn t6_handles_come_from_the_cache_after_the_first_use() {
        let mut image_store = Assets::<Image>::default();
        let source = flat_image([1, 2, 3, 255], RenderAssetUsages::RENDER_WORLD);
        let icons = assets::T6HudIcons(vec![("overlay:m".to_owned(), std::sync::Arc::new(source))]);
        let mut cache = Vec::new();
        let mut handle_for = |key: &str, unnamed: bool, icons: Option<&assets::T6HudIcons>| {
            t6_overlay_handle(key, unnamed, icons, &mut cache, &mut image_store)
        };
        assert!(handle_for("overlay:m", true, Some(&icons)).is_none());
        let first = handle_for("overlay:m", false, Some(&icons)).expect("found in icons");
        let second = handle_for("overlay:m", false, None).expect("cached");
        assert_eq!(first, second);
        assert!(handle_for("overlay:x", false, Some(&icons)).is_none());
    }
}
