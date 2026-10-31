//! bo2zm: two of Black Ops II's Advanced settings rows: Draw FPS (the frame
//! rate in the top right corner, as `cg_drawFPS "Simple"` shows it) and Max
//! Frames Per Second (`com_maxfps`: a frame cap, only without sync every
//! frame, as BO2 greys it out with sync on).

use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::{MainWindow, absolute_node, every_ui_frame};
use crate::tiers::{OverlayTier, OverlayStackShown};

#[derive(Component)]
struct FpsRoot;

/// The shown rate, its smoothing, and when it was last redrawn.
#[derive(Default)]
struct FpsState {
    shown: Option<u32>,
    smooth: f32,
    drawn_at: f32,
}

fn fps_counter(
    mut commands: Commands,
    settings: Option<Res<frame::PlayerPrefsBundle>>,
    bo2: Option<Res<crate::bo2_font::Bo2Fonts>>,
    window: MainWindow,
    time: Res<Time>,
    holders: Query<Entity, With<FpsRoot>>,
    mut state: Local<FpsState>,
) {
    let on = settings.as_deref().is_some_and(|s| s.draw_fps);
    let Some(bo2) = bo2.as_deref().filter(|_| on) else {
        for e in &holders {
            commands.entity(e).despawn();
        }
        state.shown = None;
        return;
    };
    let tick = time.delta_secs();
    if tick > 0.0 {
        let fps = 1.0 / tick;
        state.smooth = if state.smooth == 0.0 {
            fps
        } else {
            state.smooth + (fps - state.smooth) * 0.1
        };
    }
    let now = time.elapsed_secs();
    let value = state.smooth.round() as u32;
    if state.shown == Some(value) || (state.shown.is_some() && now - state.drawn_at < 0.25) {
        return;
    }
    state.shown = Some(value);
    state.drawn_at = now;
    for e in &holders {
        commands.entity(e).despawn();
    }
    let sh = window.shown().map_or(1080.0, |w| w.height());
    let scale = (sh / 1080.0).max(0.4);
    // Green at a good rate, yellow below 60, red below 30.
    let color = match value {
        60.. => Color::srgb(0.6, 1.0, 0.6),
        30..60 => Color::srgb(1.0, 0.9, 0.4),
        _ => Color::srgb(1.0, 0.4, 0.35),
    };
    commands
        .spawn((
            FpsRoot,
            OverlayTier::Hud,
            OverlayStackShown,
            ZIndex(50),
            Node {
                right: Val::Px(12.0 * scale),
                top: Val::Px(8.0 * scale),
                ..absolute_node()
            },
        ))
        .with_children(|c| {
            bo2.spawn_line(
                c,
                "Default",
                &[(value.to_string(), color)],
                26.0 * scale,
                2.0 * scale,
            );
        });
}

/// The frame cap: each frame waits out the rest of its share of a second
/// (a sleep, then a short spin for the last millisecond).
fn hold_to_max_fps(settings: Option<Res<frame::PlayerPrefsBundle>>, mut last: Local<Option<Instant>>) {
    sim::client_frame_heartbeat();
    let cap = settings
        .as_deref()
        .filter(|s| !s.sync_frames && s.max_fps > 0)
        .map(|s| s.max_fps);
    if let (Some(cap), Some(prev)) = (cap, *last) {
        let share = Duration::from_secs_f64(1.0 / f64::from(cap));
        let spent = prev.elapsed();
        if spent < share {
            let wait = share - spent;
            if wait > Duration::from_millis(2) {
                std::thread::sleep(wait - Duration::from_millis(1));
            }
            while prev.elapsed() < share {
                std::hint::spin_loop();
            }
        }
    }
    *last = Some(Instant::now());
}

pub(crate) fn register_frame_rate_systems(host: &mut App) {
    every_ui_frame(host, fps_counter);
    host.add_systems(Last, hold_to_max_fps);
}
