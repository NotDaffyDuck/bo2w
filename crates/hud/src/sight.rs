//! bo2mp: a sight attachment's reticle (the reflex's dot, the EOTech's
//! ring, the ACOG's chevron, the Target Finder's box).
//!
//! Black Ops II draws it on the sight's glass with `sw4_3d_reticle_dynamic`
//! (fxc disassembly): the picture's texture coordinate is
//! `0.5 + Color_Map_Scale * (tangent . d, -binormal . d)`, `d` the eye ray,
//! so the picture hangs at infinity along the sight and spans
//! `1 / Color_Map_Scale` in tangent of the view angle. Aiming down the
//! sights puts the sight on the line of sight, so the HUD draws it there at
//! that size (the asset lane bakes its colours and glow into
//! `reticle:<weapon>` and keeps the glass surface from drawing it flat).
//! The material adds its colour onto the frame (blend ONE / ONE, or ONE /
//! 1 - alpha with the shader's zero alpha), so the HUD draws it with the
//! match HUD's additive material, on the match world's camera.

use assets::StagedWeapons;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::ui::Display;
use bevy::ui::FocusPolicy;
use bevy::ui::PositionType;
use bevy::ui::UiTargetCamera;
use bevy::ui::ZIndex;
use net::{HostViewClient, ShownWorldFrame, ViewweaponAim};

use crate::screen_fit::WindowScale;
use crate::screen_fit::UnitClass;
use crate::ui_emit::take_over_monitor;

#[derive(Component, Clone, Copy)]
pub(crate) struct SightReticle;

/// How far up the sights must be before the reticle shows (it fades in to
/// full at full ADS, as the glass comes in front of the eye).
const SHOW_FROM_FRAC: f32 = 0.75;

/// The reticle's opacity at this ADS fraction.
pub(crate) fn sight_alpha(frac: f32) -> f32 {
    ((frac - SHOW_FROM_FRAC) / (1.0 - SHOW_FROM_FRAC)).clamp(0.0, 1.0)
}

/// The reticle picture's width in pixels for a screen `screen_px_tall` pixels tall
/// seen through a vertical field of view whose half-angle tangent is
/// `vertical_cone_slope`.
pub(crate) fn sight_size_px(scale: f32, screen_px_tall: f32, vertical_cone_slope: f32) -> f32 {
    (1.0 / scale) * (screen_px_tall * 0.5) / vertical_cone_slope.max(1e-4)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_sight(
    mut commands: Commands,
    canvas: Res<crate::surface::FlatOverlayCanvas>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    weapons: Option<Res<StagedWeapons>>,
    icons: Option<Res<assets::T6HudIcons>>,
    lenses: Query<&Projection, With<Camera3d>>,
    world_cam: Query<Entity, (With<Camera3d>, With<bevy::camera::CompositingSpace>, Without<frame::UiCamera>)>,
    aim: Res<ViewweaponAim>,
    mut image_store: ResMut<Assets<Image>>,
    mut additive: ResMut<Assets<ui::AdditiveUi>>,
    mut cache: Local<Vec<(String, Handle<Image>)>>,
    mut node: Query<(Entity, &mut Node, &MaterialNode<ui::AdditiveUi>, Option<&UiTargetCamera>), With<SightReticle>>,
) {
    // The reticle is a node of its own, made once its material can be.
    let Ok((sight_ent, mut node, material, target)) = node.single_mut() else {
        commands.spawn((
            SightReticle,
            Node { position_type: PositionType::Absolute, display: Display::None, ..Node::DEFAULT },
            MaterialNode(additive.add(ui::AdditiveUi {
                tint: LinearRgba::NONE,
                picture: Handle::default(),
                uv: Vec4::new(0.0, 0.0, 1.0, 1.0),
                encoded: 0.0,
                color_add: 0.0,
            })),
            FocusPolicy::Pass,
            ZIndex(0),
        ));
        return;
    };
    let hide = |node: &mut Node| take_over_monitor(node, Display::None);
    let (Some(ps), Some(weapons)) = (presented.player(local.0), weapons.as_deref()) else {
        hide(&mut node);
        return;
    };
    let alpha = sight_alpha(ps.weapon_raise_fraction);
    let weapon = crate::aim_mark::viewmodel_weapon_index(ps);
    let scale = weapons
        .0
        .summary_for(weapon)
        .map_or(0.0_f32, |f| f.sight_reticle_scale);
    if !canvas.has_size()
        || alpha <= 0.0
        || scale <= 0.0
        || ps.movement_mode >= playerstate_t6::game::MOVE_KIND_DEAD
    {
        hide(&mut node);
        return;
    }
    let key = format!("reticle:{}", weapons.0.label_for(weapon));
    let handle = match cache.iter().find(|(name, _)| *name == key) {
        Some((_, h)) => h.clone(),
        None => {
            let Some(&(_, ref picture)) = icons.as_deref().and_then(|i| i.0.iter().find(|(name, _)| *name == key))
            else {
                hide(&mut node);
                return;
            };
            let mut picture = (**picture).clone();
            picture.asset_usage = RenderAssetUsages::default();
            let h = image_store.add(picture);
            cache.push((key, h.clone()));
            h
        }
    };
    let Some(vertical_cone_slope) = lenses.iter().find_map(|lens| match lens {
        Projection::Perspective(lens) => Some((lens.fov / 2.0).tan()),
        _ => None,
    }) else {
        hide(&mut node);
        return;
    };
    // BO2 adds the reticle onto the frame, in the frame's own (encoded)
    // space: on the match world's camera when there is one (its target holds
    // encoded values, as the match HUD's additive pictures are drawn),
    // otherwise on the overlay's straight-alpha target.
    let lens = world_cam.iter().next();
    if lens != target.map(|t| t.0) {
        match lens {
            Some(cam) => commands.entity(sight_ent).insert(UiTargetCamera(cam)),
            None => commands.entity(sight_ent).remove::<UiTargetCamera>(),
        };
    }
    let tint = LinearRgba::new(1.0, 1.0, 1.0, alpha);
    let encoded = if lens.is_some() { 1.0 } else { 0.0 };
    let stale = additive.get(&material.0).is_none_or(|m| m.picture != handle || m.tint != tint || m.encoded != encoded);
    if stale && let Some(mut m) = additive.get_mut(&material.0) {
        m.picture = handle;
        m.tint = tint;
        m.encoded = encoded;
    }
    let tall = canvas.height();
    let view = WindowScale::for_window(canvas.width(), tall);
    let factor = view.unit_px(UnitClass::Projection);
    let (step_along_x, dy) = match aim.live {
        true => (factor * aim.crosshair_x, factor * aim.crosshair_y),
        false => (0.0, 0.0),
    };
    let size = sight_size_px(scale, view.height(), vertical_cone_slope);
    let cx = view.width() * 0.5 + step_along_x;
    let cy = view.height() * 0.5 + dy;
    take_over_monitor(&mut node, Display::Flex); // shown again once it has a place
    node.left = Val::Px((cx - size * 0.5).round());
    node.top = Val::Px((cy - size * 0.5).round());
    node.width = Val::Px(size.round().max(1.0));
    node.height = Val::Px(size.round().max(1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflex_dot_size_on_a_1080p_screen() {
        // The reflex (scale 30) at its 50 degree zoom: tan(25 deg) * 0.75
        // vertical, 1080 tall.
        let vertical_cone_slope = (25f32).to_radians().tan() * 0.75;
        let px = sight_size_px(30.0, 1080.0, vertical_cone_slope);
        assert!((px - 51.5).abs() < 1.0, "{px}");
        assert_eq!(sight_alpha(0.5), 0.0);
        assert_eq!(sight_alpha(1.0), 1.0);
    }
}
