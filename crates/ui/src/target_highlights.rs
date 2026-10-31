//! bo2mp: the red diamonds on the enemies a Dragonfire's pilot can see.
//! BO2 draws them from its `target_highlights` menu (common_mp): a full
//! 640x480 screen whose line-of-sight item paints
//! `compassping_enemysatellite_diamond` in red over every live enemy player
//! while he rides a remote vehicle, each one only while nothing blocks the
//! view to it. The diamond sits on the body (46.67 up standing, 25 crouched,
//! 15 prone) and shrinks with distance and field of view:
//! size = min(90, 100 / distance * 2500 * 45 / fov_x), drawn size * 0.5 wide
//! and size * 0.6667 tall (640x480 units), centred on the point.
//! TODO(fact): BO2 also marks enemy-owned vehicles, turrets and dogs; players only for now.

use bevy::prelude::*;
use frame::ClientStep;
use net::{ClientEntityRecord, ClientEntityPoseState, ClientGuessState, HostViewClient, ShownWorldFrame};

use crate::{ActiveView, MainWindow, SceneCameras, ScreenBox};
use crate::tiers::{OverlayTier, OverlayStackShown};

#[derive(Component)]
struct TargetHighlightNode;

const PICTURE: &str = "compassping_enemysatellite_diamond";
/// cg_maxPlayerHighlightTargetSize, cg_playerHighlightTargetSize.
const MAX_SIZE: f32 = 90.0;
const TARGET_SIZE: f32 = 2500.0;
/// The view trace BO2's overhead names use (and so these diamonds).
const SIGHT_MASK: u32 = 0x0280_1001;
const UPRIGHT: f32 = 46.67;
const DUCK_BUTTON_BIT: f32 = 25.0;
const LYING_FLAT: f32 = 15.0;
/// e_flags: crouched, prone, and the ones that hide a player (dead, not drawn).
const EF_CROUCH: u32 = 0x4;
const EF_PRONE: u32 = 0x8;
const EF_HIDDEN: u32 = 0x20 | 0x20000 | 0x40000;

/// How high on the body the diamond sits.
fn body_height(e_flags: u32) -> f32 {
    if e_flags & EF_PRONE != 0 {
        LYING_FLAT
    } else if e_flags & EF_CROUCH != 0 {
        DUCK_BUTTON_BIT
    } else {
        UPRIGHT
    }
}

/// The diamond's size (640x480 units, before its 0.5 / 0.6667 shape).
fn size(distance: f32, fov_x_deg: f32) -> f32 {
    (100.0 / distance.max(1.0) * TARGET_SIZE * (45.0 / fov_x_deg.max(1.0))).min(MAX_SIZE)
}

/// Horizontal field of view in degrees from the camera's vertical one.
fn fov_x_deg(vertical_fov_radians: f32, aspect: f32) -> f32 {
    (2.0 * ((vertical_fov_radians * 0.5).tan() * aspect).atan()).to_degrees()
}

fn riding_drone(presented: &ShownWorldFrame, local: &HostViewClient) -> bool {
    presented
        .snapshot()
        .and_then(|s| s.meta.script_dvars(local.0).string("bo2mp_vehicle"))
        .is_some_and(|v| v.split(' ').next().is_some_and(|k| k.to_ascii_lowercase().starts_with("qrdrone")))
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    nodes: Query<Entity, With<TargetHighlightNode>>,
    window: MainWindow,
    views: SceneCameras,
    actors: Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    prediction: Option<Res<ClientGuessState>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut pics: ResMut<Assets<Image>>,
    mut picture: Local<Option<Handle<Image>>>,
) {
    for e in &nodes {
        commands.entity(e).try_despawn();
    }
    let (Some(presented), Some(local)) = (presented.as_deref(), local.as_deref()) else { return };
    if !riding_drone(presented, local) {
        return;
    }
    let Some(snapshot) = presented.snapshot() else { return };
    let Some(local_team) = snapshot.meta.for_client(local.0).map(|m| m.client_state_team) else { return };
    let Some(world) = prediction.as_deref().filter(|p| p.0.is_forecast_live() && p.0.world().has_static_collision()).map(|p| p.0.world())
    else {
        return;
    };
    let Some(win) = window.shown() else { return };
    let Some(ActiveView { camera, at: transform, lens: projection }) = views.active() else { return };
    let Projection::Perspective(perspective) = projection else { return };
    if picture.is_none()
        && let Some(image) = icons.as_deref().and_then(|all| crate::hud_picture(all, PICTURE))
    {
        *picture = Some(pics.add((**image).clone()));
    }
    let Some(handle) = picture.as_ref() else { return };
    let fov_x = fov_x_deg(perspective.fov, perspective.aspect_ratio);
    let (kx, ky) = (win.width() / 640.0, win.height() / 480.0);
    let eye = transform.translation();
    for (who, runtime) in &actors {
        let Some(client) = who.client() else { continue };
        if client == local.0 || !runtime.present_in_newer_frame() || runtime.upcoming_condition.lerp.e_flags as u32 & EF_HIDDEN != 0 {
            continue;
        }
        let Some(meta) = snapshot.meta.for_client(client) else { continue };
        let enemy = meta.lifecycle == sim::PlayerSessionStage::Alive
            && meta.client_state_team != entity_t6::team::SPECTATOR
            && (local_team == 0 || meta.client_state_team != local_team);
        if !enemy {
            continue;
        }
        let point = Vec3::from_array(runtime.origin) + Vec3::Z * body_height(runtime.upcoming_condition.lerp.e_flags as u32);
        let hit = world.trace_world(eye.to_array(), point.to_array(), [0.0; 3], [0.0; 3], SIGHT_MASK);
        if hit.fraction < 1.0 || hit.starts_inside {
            continue;
        }
        let Ok(pixel) = camera.world_to_viewport(transform, point) else { continue };
        if !pixel.is_finite() {
            continue;
        }
        let s = size(eye.distance(point), fov_x);
        let (w, h) = (s * 0.5 * kx, s * 0.6667 * ky);
        commands.spawn((
            TargetHighlightNode,
            OverlayTier::Hud,
            OverlayStackShown,
            ImageNode::new(handle.clone())
                .with_color(Color::srgba(1.0, 0.0, 0.0, 1.0))
                .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
            ZIndex(-2300),
            ScreenBox {
                left_px: pixel.x - w * 0.5,
                top_px: pixel.y - h * 0.5,
                width_px: w,
                height_px: h,
            }.node(),
        ));
    }
}

pub(crate) fn register(host: &mut App) {
    host.add_systems(Update, update.in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diamond_caps_at_90_close_and_shrinks_far() {
        assert_eq!(size(500.0, 65.0), MAX_SIZE);
        let far = size(5000.0, 65.0);
        assert!((far - 100.0 / 5000.0 * 2500.0 * 45.0 / 65.0).abs() < 1e-4);
        assert!(size(10000.0, 65.0) < far);
        // A wider view draws it smaller.
        assert!(size(5000.0, 90.0) < far);
    }

    #[test]
    fn diamond_sits_lower_crouched_and_prone() {
        assert_eq!(body_height(0), UPRIGHT);
        assert_eq!(body_height(EF_CROUCH), DUCK_BUTTON_BIT);
        assert_eq!(body_height(EF_PRONE | EF_CROUCH), LYING_FLAT);
    }

    #[test]
    fn horizontal_fov_from_vertical() {
        let x = fov_x_deg(60f32.to_radians(), 1.0);
        assert!((x - 60.0).abs() < 1e-3);
        assert!(fov_x_deg(60f32.to_radians(), 16.0 / 9.0) > 90.0);
    }
}
