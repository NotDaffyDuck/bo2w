//! Picks the interface tiers that go with each app screen and keeps the tier set in step.

use assets::LoadBackdrop;
use bevy::prelude::*;
use frame::ShellScreen;

use crate::tiers::{MenuPaintPass, OverlayTier, OverlayTierPass, OverlayTierSet};

const LOAD_TIERS: &[OverlayTier] = &[OverlayTier::Loading, OverlayTier::Overlay];
const SHELL_TIERS: &[OverlayTier] = &[OverlayTier::Shell, OverlayTier::Overlay];
const PICKER_TIERS: &[OverlayTier] = &[OverlayTier::Hud, OverlayTier::Overlay];
const MATCH_TIERS: &[OverlayTier] = &[OverlayTier::Hud, OverlayTier::Debug, OverlayTier::Overlay];
const BARE_TIERS: &[OverlayTier] = &[OverlayTier::Overlay];

/// Tiers for `screen`. An active load backdrop beats the screen itself, and with drawing
/// switched off (`ui_draw` false) the in-match tiers drop down to the overlay alone.
pub fn tiers_for_screen(on_screen: ShellScreen, drawing: bool, backdrop_up: bool) -> &'static [OverlayTier] {
    if backdrop_up {
        return LOAD_TIERS;
    }
    match on_screen {
        ShellScreen::MainMenu => SHELL_TIERS,
        ShellScreen::Loading => LOAD_TIERS,
        ShellScreen::KitPicker | ShellScreen::InGame if !drawing => BARE_TIERS,
        ShellScreen::KitPicker => PICKER_TIERS,
        ShellScreen::InGame => MATCH_TIERS,
    }
}

pub fn refresh_overlay_tiers(
    current: Res<ShellScreen>,
    draw: Res<MenuPaintPass>,
    backdrop: Option<Res<LoadBackdrop>>,
    mut tier_set: ResMut<OverlayTierSet>,
) {
    let picked = tiers_for_screen(*current, draw.0, backdrop.is_some());
    tier_set.raise_exactly(picked.iter().copied());
}

/// Seeds the screen resource and refreshes the tiers at start-up and each frame.
pub(crate) fn wire_screen_tiers(host: &mut App) {
    host.init_resource::<ShellScreen>();
    host.add_systems(Startup, refresh_overlay_tiers);
    host.add_systems(PostUpdate, refresh_overlay_tiers.before(OverlayTierPass));
}
