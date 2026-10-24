//! The scoped-sight picture: the overlay quads drawn when a gun with an
//! overlay is zoomed in, and the four strips of flat colour that fill the
//! rest of the screen around it.

use assets::PreparedWeapons;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::{Display, FocusPolicy, ZIndex};
use hud_t6::iris::{
    AdsOverlayLayout, AdsOverlayQuad, WeaponOverlayDef, ads_overlay_material, draw_ads_overlay_layout,
    iris_overlay_configured,
};
use net::{LocalPresentClient, PresentedSnapshot, ViewweaponAim};

use crate::gaps::{GapCause, HudGap, HudPresentationGaps};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};
use crate::presentation_scale::{PresentationScale, ScaleClass};
use crate::reticle::ReticleAdsLatch;
use crate::surface::Hud2dSurface;
use crate::ui_write::adopt_display;

/// A sample this opaque or more counts as picture rather than see-through.
const OPAQUE_ALPHA: u8 = 250;
/// A texel with every channel this low or lower is the black the picture
/// fades to at its edge.
const NEAR_BLACK: u8 = 16;

/// The colour sampled from the current overlay picture, and the 1x1 image
/// made from it for the strips.
#[derive(Resource, Default)]
pub(crate) struct IrisLetterboxFill {
    source: Option<String>,
    handle: Option<Handle<Image>>,
    rgba: Option<[u8; 4]>,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct AdsIrisOverlay(u8);

#[derive(Component, Clone, Copy)]
pub(crate) struct AdsIrisLetterbox(u8);

type OverlayQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Node, &'static mut ImageNode, &'static AdsIrisOverlay),
    (With<AdsIrisOverlay>, Without<AdsIrisLetterbox>),
>;

type LetterboxQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Node,
        &'static mut ImageNode,
        &'static mut BackgroundColor,
        &'static AdsIrisLetterbox,
    ),
    Without<AdsIrisOverlay>,
>;

/// An absolutely placed, initially undisplayed image node.
fn blank_image() -> (Node, ImageNode, FocusPolicy) {
    let node = Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        overflow: Overflow::visible(),
        ..default()
    };
    (node, ImageNode::default(), FocusPolicy::Pass)
}

pub(crate) fn spawn_iris(root: &mut ChildSpawnerCommands) {
    // The strips sit under the picture quads.
    for strip in 0..4u8 {
        let (node, image, focus) = blank_image();
        root.spawn((
            AdsIrisLetterbox(strip),
            node,
            image,
            focus,
            BackgroundColor(Color::NONE),
            ZIndex(0),
        ));
    }
    for quad in 0..4u8 {
        let (node, image, focus) = blank_image();
        root.spawn((AdsIrisOverlay(quad), node, image, focus, ZIndex(1)));
    }
}

fn hide_quad(node: &mut Node, image: &mut ImageNode) {
    adopt_display(node, Display::None);
    image.flip_x = false;
    image.flip_y = false;
}

fn hide_overlay(overlay: &mut OverlayQuery) {
    for (mut node, mut image, _) in overlay.iter_mut() {
        hide_quad(&mut node, &mut image);
    }
}

fn hide_strip(node: &mut Node, background: &mut BackgroundColor) {
    adopt_display(node, Display::None);
    *background = BackgroundColor(Color::NONE);
}

fn hide_letterbox(letterbox: &mut LetterboxQuery) {
    for (mut node, _, mut background, _) in letterbox.iter_mut() {
        hide_strip(&mut node, &mut background);
    }
}

fn hide_all(overlay: &mut OverlayQuery, letterbox: &mut LetterboxQuery) {
    hide_overlay(overlay);
    hide_letterbox(letterbox);
}

/// The colour for the strips: read along the middle row of the picture, the
/// first near-black opaque texel, else the first opaque one.
pub(crate) fn overlay_frame_texel(width: u32, height: u32, rgba: &[u8]) -> Option<[u8; 4]> {
    if width == 0 || height == 0 {
        return None;
    }
    let row_start = (height / 2) as usize * width as usize * 4;
    let row = rgba.get(row_start..)?;
    let mut first_opaque = None;
    for texel in row.chunks_exact(4).take(width as usize) {
        let texel = [texel[0], texel[1], texel[2], texel[3]];
        if texel[3] < OPAQUE_ALPHA {
            continue;
        }
        if texel[..3].iter().all(|&channel| channel <= NEAR_BLACK) {
            return Some(texel);
        }
        first_opaque.get_or_insert(texel);
    }
    first_opaque
}

fn solid_image(rgba: [u8; 4], images: &mut Assets<Image>) -> Handle<Image> {
    let size = Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    images.add(Image::new(
        size,
        TextureDimension::D2,
        rgba.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}

/// The gun's overlay definition. Whether it has an overlay material at all is
/// all that matters here (the material itself is looked up by name in
/// `update_iris`), so that is stored as 1 or 0.
pub(crate) fn overlay_def(hud_iris: bool, facts: &asset_game::WeaponBodyFacts) -> WeaponOverlayDef {
    let reticle = if hud_iris {
        u32::try_from(facts.overlay_reticle)
            .ok()
            .filter(|&reticle| reticle != 0)
            .unwrap_or(1)
    } else {
        0
    };
    WeaponOverlayDef {
        overlay_material: u32::from(hud_iris),
        overlay_reticle: reticle,
        overlay_width: facts.ads_overlay_width,
        overlay_height: facts.ads_overlay_height,
        overlay_alpha_scale: facts.overlay_alpha_scale,
        infrared: facts.infrared,
        ..WeaponOverlayDef::default()
    }
}

/// How far into its zoom the overlay is. `None` unless the gun has an
/// overlay and is zoomed in; moving toward ADS uses the zoom-in window,
/// otherwise the zoom-out one.
pub(crate) fn weap_reticle_zoom(
    def: &WeaponOverlayDef,
    facts: &asset_game::WeaponBodyFacts,
    pos_frac: f32,
    to_ads: bool,
) -> Option<f32> {
    if !iris_overlay_configured(def) {
        return None;
    }
    let window = if to_ads {
        facts.ads_zoom_in_frac
    } else {
        facts.ads_zoom_out_frac
    };
    weapon_t6::aim::reticle_zoom(pos_frac, window)
}

/// Why nothing is drawn this frame.
enum Skip {
    /// Nothing to draw and nothing wrong; any standing gap is cleared.
    Quiet,
    /// Something needed is missing; the cause is logged.
    Gap(GapCause),
}

/// What is needed to draw the overlay.
struct Scope {
    facts: asset_game::WeaponBodyFacts,
    zoom: f32,
    image_name: String,
    picture: Handle<Image>,
}

#[allow(clippy::too_many_arguments)]
fn find_scope(
    surface: &Hud2dSurface,
    presented: &PresentedSnapshot,
    local: &LocalPresentClient,
    weapons: Option<&PreparedWeapons>,
    ads_latch: &ReticleAdsLatch,
    hud_images: &mut HudImages,
    images: &mut Assets<Image>,
    t6_icons: Option<&assets::T6HudIcons>,
    t6_cache: &mut Vec<(String, Handle<Image>)>,
) -> Result<Scope, Skip> {
    let player = presented.player(local.0).ok_or(Skip::Quiet)?;
    let viewmodel = crate::reticle::viewmodel_weapon_index(player);
    let catalog = &weapons.ok_or(Skip::Quiet)?.0;
    let facts = catalog.facts_of(viewmodel).ok_or(Skip::Quiet)?;
    let def = overlay_def(catalog.overlay_is_hud_iris(viewmodel), &facts);
    if !surface.is_ready() {
        return Err(Skip::Quiet);
    }
    let zoom = weap_reticle_zoom(&def, &facts, player.f_weapon_pos_frac, ads_latch.position_to_ads)
        .ok_or(Skip::Quiet)?;
    // Not yet fed in: an EMP-jammed state. While jammed, an infrared overlay
    // switches to the shared EMP scope material.
    if ads_overlay_material(&def, surface.height() as u32, false, false, false, 0) == 0 {
        return Err(Skip::Quiet);
    }
    if facts.ads_overlay_width <= 0.0 || facts.ads_overlay_height <= 0.0 {
        return Err(Skip::Gap(GapCause::AdsOverlayNoSize));
    }

    let namespace = catalog
        .namespace_of(viewmodel)
        .unwrap_or(HUD_CHROME_NAMESPACE);
    let (image_name, picture) = if namespace == asset_core::AssetNamespace::T6 {
        // A Black Ops II gun's scope picture is its overlay material's colour
        // map, which the asset lane decodes among the HUD icons under
        // "overlay:<material>".
        let material = catalog.overlay_material_of(viewmodel).unwrap_or("");
        let name = format!("overlay:{material}");
        match t6_overlay_handle(&name, material.is_empty(), t6_icons, t6_cache, images) {
            Some(handle) => (name, handle),
            None => {
                return Err(Skip::Gap(GapCause::AdsOverlayImageMissing {
                    name,
                    miss: hud_images.miss_reason(),
                }));
            }
        }
    } else {
        let Some(name) = catalog.overlay_image_of(viewmodel) else {
            return Err(Skip::Gap(GapCause::AdsOverlayNamesNoImage {
                material: catalog.overlay_material_of(viewmodel).map(str::to_owned),
            }));
        };
        match hud_images.get(namespace, name, images) {
            Some(handle) => (name.to_owned(), handle),
            None => {
                return Err(Skip::Gap(GapCause::AdsOverlayImageMissing {
                    name: name.to_owned(),
                    miss: hud_images.miss_reason(),
                }));
            }
        }
    };
    Ok(Scope {
        facts,
        zoom,
        image_name,
        picture,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_iris(
    surface: Res<Hud2dSurface>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    ads_latch: Res<ReticleAdsLatch>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut fill: ResMut<IrisLetterboxFill>,
    aim: Res<ViewweaponAim>,
    (t6_icons, mut t6_cache): (
        Option<Res<assets::T6HudIcons>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
    mut overlay: OverlayQuery,
    mut letterbox: LetterboxQuery,
) {
    let found = find_scope(
        &surface,
        &presented,
        &local,
        weapons.as_deref(),
        &ads_latch,
        &mut hud_images,
        &mut images,
        t6_icons.as_deref(),
        &mut t6_cache,
    );
    let scope = match found {
        Ok(scope) => scope,
        Err(skip) => {
            match skip {
                Skip::Quiet => gaps.clear(HudGap::AdsOverlay),
                Skip::Gap(cause) => gaps.raise(cause),
            }
            hide_all(&mut overlay, &mut letterbox);
            return;
        }
    };
    gaps.clear(HudGap::AdsOverlay);
    let Scope {
        facts,
        zoom,
        image_name,
        picture,
    } = scope;

    let window = PresentationScale::from_window(surface.width(), surface.height());
    let factor = window.factor(ScaleClass::ProjectionBound);
    let (win_w, win_h) = (window.width(), window.height());
    // The picture is centred on the crosshair, which follows the gun.
    let (origin_x, origin_y) = if aim.live {
        (win_w * 0.5 + aim.xhair_x * factor, win_h * 0.5 + aim.xhair_y * factor)
    } else {
        (win_w * 0.5, win_h * 0.5)
    };
    let alpha = weapon_t6::aim::overlay_alphas(zoom, facts.overlay_alpha_scale).0;
    // Laid out around (0, 0) in authored units and moved to the origin
    // below; there is no dual-play split screen.
    let layout = draw_ads_overlay_layout(
        0.0,
        0.0,
        facts.ads_overlay_width,
        facts.ads_overlay_height,
        false,
        alpha,
    );
    let quads: &[AdsOverlayQuad] = match &layout {
        AdsOverlayLayout::Four(four) => four,
        AdsOverlayLayout::One { quad, .. } => std::slice::from_ref(quad),
    };
    // A rect in authored units to whole window pixels: (left, top, right, bottom).
    let to_pixels = |x: f32, y: f32, w: f32, h: f32| {
        (
            (origin_x + x * factor).round(),
            (origin_y + y * factor).round(),
            (origin_x + (x + w) * factor).round(),
            (origin_y + (y + h) * factor).round(),
        )
    };

    for (mut node, mut image, slot) in overlay.iter_mut() {
        let Some(quad) = quads.get(usize::from(slot.0)) else {
            hide_quad(&mut node, &mut image);
            continue;
        };
        let (left, top, right, bottom) = to_pixels(quad.x, quad.y, quad.w, quad.h);
        adopt_display(&mut node, Display::Flex);
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        node.width = Val::Px(right - left);
        node.height = Val::Px(bottom - top);
        image.image = picture.clone();
        image.color = Color::srgba(1.0, 1.0, 1.0, alpha);
        image.rect = None;
        image.flip_x = quad.s0 > quad.s1;
        image.flip_y = quad.t0 > quad.t1;
    }

    let Some(sampled) = sample_fill(&mut fill, &image_name, &picture, &mut images) else {
        hide_letterbox(&mut letterbox);
        return;
    };
    let Some(picture_image) = images.get(&picture) else {
        hide_letterbox(&mut letterbox);
        return;
    };

    // The whole layout's rectangle, pulled in so the strips also sit under
    // the picture's outermost texel and its linear-filter blur.
    let (inner_x, inner_y, inner_w, inner_h) = layout.rect();
    let (inner_left, inner_top, inner_right, inner_bottom) =
        to_pixels(inner_x, inner_y, inner_w, inner_h);
    let first = quads[0];
    let inset_x = (first.w * factor * 1.5 / picture_image.width() as f32).ceil();
    let inset_y = (first.h * factor * 1.5 / picture_image.height() as f32).ceil();
    let fill_left = (inner_left + inset_x).clamp(0.0, win_w);
    let fill_right = (inner_right - inset_x).clamp(fill_left, win_w);
    let fill_top = (inner_top + inset_y).clamp(0.0, win_h);
    let fill_bottom = (inner_bottom - inset_y).clamp(fill_top, win_h);
    let middle_width = fill_right - fill_left;
    let (screen_w, screen_h) = (win_w.round(), win_h.round());
    let strip_color = Color::srgba(
        f32::from(sampled.rgba[0]) / 255.0,
        f32::from(sampled.rgba[1]) / 255.0,
        f32::from(sampled.rgba[2]) / 255.0,
        alpha,
    );

    for (mut node, mut image, mut background, strip) in letterbox.iter_mut() {
        // Left and right take the full height; top and bottom fill the
        // width between them.
        let rect = match strip.0 {
            0 if fill_left > 0.0 => Some((0.0, 0.0, fill_left, screen_h)),
            1 if screen_w > fill_right => Some((fill_right, 0.0, screen_w - fill_right, screen_h)),
            2 if fill_top > 0.0 => Some((fill_left, 0.0, middle_width, fill_top)),
            3 if screen_h > fill_bottom => {
                Some((fill_left, fill_bottom, middle_width, screen_h - fill_bottom))
            }
            _ => None,
        };
        let Some((left, top, width, height)) = rect.filter(|r| r.2 > 0.0 && r.3 > 0.0) else {
            hide_strip(&mut node, &mut background);
            continue;
        };
        adopt_display(&mut node, Display::Flex);
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        node.width = Val::Px(width);
        node.height = Val::Px(height);
        image.image = sampled.handle.clone();
        image.color = Color::srgba(1.0, 1.0, 1.0, alpha);
        image.rect = None;
        *background = BackgroundColor(strip_color);
    }
}

struct SampledFill {
    handle: Handle<Image>,
    rgba: [u8; 4],
}

/// The strip colour for the picture called `image_name`, sampled once per
/// picture and kept until another one is shown.
fn sample_fill(
    fill: &mut IrisLetterboxFill,
    image_name: &str,
    picture: &Handle<Image>,
    images: &mut Assets<Image>,
) -> Option<SampledFill> {
    if fill.source.as_deref() == Some(image_name) {
        return Some(SampledFill {
            handle: fill.handle.clone()?,
            rgba: fill.rgba?,
        });
    }
    fill.source = Some(image_name.to_owned());
    fill.handle = None;
    fill.rgba = None;
    let source = images.get(picture)?;
    let rgba = overlay_frame_texel(source.width(), source.height(), source.data.as_ref()?)?;
    let handle = solid_image(rgba, images);
    fill.handle = Some(handle.clone());
    fill.rgba = Some(rgba);
    Some(SampledFill { handle, rgba })
}

/// A Black Ops II scope picture as a HUD image. The copy keeps its pixels on
/// the CPU too, since the strip colour is read from them.
fn t6_overlay_handle(
    key: &str,
    unnamed: bool,
    icons: Option<&assets::T6HudIcons>,
    cache: &mut Vec<(String, Handle<Image>)>,
    images: &mut Assets<Image>,
) -> Option<Handle<Image>> {
    if unnamed {
        return None;
    }
    if let Some((_, cached)) = cache.iter().find(|(name, _)| name == key) {
        return Some(cached.clone());
    }
    let (_, source) = icons?.0.iter().find(|(name, _)| name == key)?;
    let mut copy = (**source).clone();
    copy.asset_usage = RenderAssetUsages::default();
    let handle = images.add(copy);
    cache.push((key.to_owned(), handle.clone()));
    Some(handle)
}
