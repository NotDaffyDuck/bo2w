//! bo2mp: Black Ops II's "stuck" warning. An enemy's semtex or explosive
//! crossbow bolt stuck to him beats on his screen with each of its blinks
//! (`clientscripts/mp/_sticky_grenade.csc` and `_explosive_bolt.csc`,
//! `fullscreen_fx`): the blinks start when it is thrown, 0.3 s apart, each
//! wait the last divided by 1.2 and never under 0.08 s; on each one while it
//! rides him the script pulses `sticky_grenade_overlay`'s "overlay" item and,
//! outside hardcore, the "stuck" menu's "sticky_grenade" or "explosive_bolt"
//! item. Never a friend's, and never in a killcam.
//!
//! The "stuck" menu (`stuck0`, ui_mp) is one 64x64 picture centred 100
//! above the screen's middle (`hud_icon_stuck_semtex` or
//! `hud_icon_stuck_arrow`); a pulse snaps it to full, holds it 100 ms
//! ("pulse" then "on") and fades it out over 400 ms ("Default", clear).
//! `sticky_grenade_overlay0`'s "overlay" is the whole screen in
//! `overlay_low_health`, snapped to 0.6 and gone again over 100 ms.

use std::collections::HashSet;

use bevy::prelude::*;
use frame::ClientStep;
use net::{HostViewClient, ShownProjectile, ShownWorldFrame};

use crate::{MainWindow, ScreenBox};
use crate::lui_hud::PictureCache;
use crate::tiers::{OverlayTier, OverlayStackShown};

#[derive(Component)]
struct StuckPulseNode;

/// The menu's rect on the 640x480 screen: its size and how far above the
/// middle its top is.
const ICON_SIZE: f32 = 64.0;
const ICON_TOP: f32 = -100.0;
/// A pulse: full this long, then gone over this long.
const HOLD_MS: f32 = 100.0;
const FADE_MS: f32 = 400.0;
/// The red overlay's pulse: this bright, then gone over this long.
const OVERLAY_ALPHA: f32 = 0.6;
const OVERLAY_FADE_MS: f32 = 100.0;
const OVERLAY: &str = "overlay_low_health";

/// BO2's waits between blinks: the first, the shortest, and the divisor each takes.
const FIRST_MS: f32 = 300.0;
const SHORTEST_MS: f32 = 80.0;
const SPEEDUP: f32 = 1.2;

/// Which item of the "stuck" menu a weapon pulses, by its script name.
pub(crate) fn stuck_item(gsc_name: &str) -> Option<&'static str> {
    if gsc_name == "sticky_grenade_mp" || gsc_name.starts_with("sticky_grenade_mp+") {
        Some("sticky_grenade")
    } else if gsc_name == "explosive_bolt_mp" {
        Some("explosive_bolt")
    } else {
        None
    }
}

/// The "stuck" item's picture.
pub(crate) fn stuck_picture(item: &str) -> &'static str {
    if item == "explosive_bolt" { "hud_icon_stuck_arrow" } else { "hud_icon_stuck_semtex" }
}

/// How long ago (ms) the last blink was, `since_thrown_ms` after the throw;
/// the schedule is the script's (and the sim's light): blink, wait, blink.
pub(crate) fn since_last_blink(since_thrown_ms: i64) -> Option<i64> {
    if since_thrown_ms < 0 {
        return None;
    }
    let (mut at, mut wait) = (0_i64, FIRST_MS);
    loop {
        let next = at + wait.round() as i64;
        if next > since_thrown_ms {
            return Some(since_thrown_ms - at);
        }
        at = next;
        wait = (wait / SPEEDUP).clamp(SHORTEST_MS, FIRST_MS);
    }
}

/// How bright a pulse is `since_ms` after it.
pub(crate) fn pulse_alpha(since_ms: i64) -> f32 {
    let t = since_ms as f32;
    if t <= HOLD_MS { 1.0 } else { (1.0 - (t - HOLD_MS) / FADE_MS).max(0.0) }
}

/// How bright the red overlay is `since_ms` after a pulse.
pub(crate) fn overlay_alpha(since_ms: i64) -> f32 {
    (OVERLAY_ALPHA * (1.0 - since_ms as f32 / OVERLAY_FADE_MS)).max(0.0)
}

#[derive(Default)]
struct StuckState {
    pictures: PictureCache,
    missing: HashSet<String>,
}

#[allow(clippy::too_many_arguments)]
fn stuck_pulse(
    mut commands: Commands,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    weapons: Option<Res<assets::StagedWeapons>>,
    cg_clock: Option<Res<net::FrameClock>>,
    mut pics: ResMut<Assets<Image>>,
    window: MainWindow,
    nodes: Query<Entity, With<StuckPulseNode>>,
    view: Option<Res<frame::EyeSubject>>,
    mut st: Local<StuckState>,
) {
    for e in &nodes {
        commands.entity(e).despawn();
    }
    let (Some(p), Some(l), Some(weapons)) = (presented.as_deref(), local.as_deref(), weapons.as_deref()) else {
        return;
    };
    let Some(snapshot) = p.snapshot() else { return };
    let dvars = snapshot.meta.script_dvars(l.0);
    if dvars.string("bo2mp_gametype").is_none() || view.as_deref().is_some_and(|v| v.replaying_final_death()) {
        return;
    }
    let hardcore = dvars.int("g_hardcore").or_else(|| dvars.int("scr_hardcore")).unwrap_or(0) != 0;
    let my_team = snapshot.meta.for_client(l.0).map_or(0, |m| m.client_state_team);
    let now = cg_clock
        .as_deref()
        .filter(|c| c.started())
        .map(|c| c.time())
        .unwrap_or_else(|| p.tick().map(sim::level_time_ms).unwrap_or(0));
    let reg = &weapons.0;
    // The newest blink of any enemy semtex or bolt riding him.
    let mut shown: Option<(i64, &'static str)> = None;
    for row in p.presented_projectiles() {
        let ShownProjectile::HostDecided(s) = row else { continue };
        if !s.in_flight || !s.rider.is_some_and(|r| r.client == l.0) {
            continue;
        }
        let Some(item) = stuck_item(&reg.gsc_name_for(s.gun)) else { continue };
        // A friend's never pulses (the script's friendly check: his team in
        // a team game, else only his own).
        if s.owner == l.0
            || (my_team != 0 && snapshot.meta.for_client(s.owner).map_or(0, |m| m.client_state_team) == my_team)
        {
            continue;
        }
        let Some(since) = since_last_blink(i64::from(now) - i64::from(s.spawn_time_ms)) else { continue };
        if shown.is_none_or(|(d, _)| since < d) {
            shown = Some((since, item));
        }
    }
    let Some((since, item)) = shown else { return };
    let (sw, sh) = window.shown().map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    let k = sh / 480.0;
    let s = ICON_SIZE * k;
    // The overlay on every blink; the picture outside hardcore.
    let mut draw = vec![(OVERLAY, overlay_alpha(since), ScreenBox { left_px: 0.0, top_px: 0.0, width_px: sw, height_px: sh })];
    if !hardcore {
        let icon = ScreenBox { left_px: sw * 0.5 - s * 0.5, top_px: sh * 0.5 + ICON_TOP * k, width_px: s, height_px: s };
        draw.push((stuck_picture(item), pulse_alpha(since), icon));
    }
    draw.retain(|(_, alpha, _)| *alpha > 0.0);
    let Some(icons) = icons.as_deref() else { return };
    for (name, _, _) in &draw {
        if st.pictures.holds(name) || st.missing.contains(*name) {
            continue;
        }
        match icons.0.iter().find(|(n, _)| n == name) {
            Some((_, image)) => st.pictures.keep((*name).to_owned(), &pics.add((**image).clone())),
            None => {
                st.missing.insert((*name).to_owned());
            }
        }
    }
    for (name, alpha, at) in draw {
        let Some(art) = st.pictures.kept(name) else { continue };
        commands.spawn((
            StuckPulseNode,
            OverlayTier::Hud,
            OverlayStackShown,
            ImageNode::new(art)
                .with_color(Color::srgba(1.0, 1.0, 1.0, alpha))
                .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
            at.node(),
        ));
    }
}

pub(crate) fn register(host: &mut App) {
    host.add_systems(Update, stuck_pulse.in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blinks_speed_up_from_three_tenths_to_the_shortest() {
        // 0, 300, 550 (250), 758 (208.3), 932 (173.6) ...
        assert_eq!(since_last_blink(0), Some(0));
        assert_eq!(since_last_blink(299), Some(299));
        assert_eq!(since_last_blink(300), Some(0));
        assert_eq!(since_last_blink(560), Some(10));
        assert_eq!(since_last_blink(758), Some(0));
        assert_eq!(since_last_blink(-1), None);
        // Late on, never more than the shortest wait apart.
        assert!(since_last_blink(10_000).is_some_and(|d| d < 80));
    }

    #[test]
    fn only_the_semtex_and_the_explosive_bolt_pulse() {
        assert_eq!(stuck_item("sticky_grenade_mp"), Some("sticky_grenade"));
        assert_eq!(stuck_item("explosive_bolt_mp"), Some("explosive_bolt"));
        assert_eq!(stuck_item("crossbow_mp"), None);
        assert_eq!(stuck_item("frag_grenade_mp"), None);
        assert_eq!(stuck_picture("sticky_grenade"), "hud_icon_stuck_semtex");
        assert_eq!(stuck_picture("explosive_bolt"), "hud_icon_stuck_arrow");
    }

    #[test]
    fn a_pulse_holds_a_tenth_then_fades_and_the_overlay_goes_in_a_tenth() {
        assert_eq!(pulse_alpha(0), 1.0);
        assert_eq!(pulse_alpha(100), 1.0);
        assert!((pulse_alpha(300) - 0.5).abs() < 1e-6);
        assert_eq!(pulse_alpha(500), 0.0);
        assert!((overlay_alpha(0) - 0.6).abs() < 1e-6);
        assert!((overlay_alpha(50) - 0.3).abs() < 1e-6);
        assert_eq!(overlay_alpha(100), 0.0);
    }
}
