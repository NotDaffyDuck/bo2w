//! bo2mp: Black Ops II's grenade warning in a multiplayer match: a live
//! grenade near him shows its picture (the weapon's own `indicatorIcon`, or
//! `hud_grenadeicon`) 50 units out from the crosshair toward it, with the
//! `hud_grenadepointer` arrow on its outside, both beating. As BO2 draws it:
//! only a grenade, heavy explosive or fire grenade with a fuse, sitting or
//! rolling slowly (or stuck to someone), within its warning range (its
//! `iIndicatorRadius`, or the larger of 250 and its blast), not far above or
//! below him; never one stuck to him, a teammate's (unless friendly fire is
//! on), in hardcore, or while he looks through a scope.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::ClientSet;
use net::{LocalPresentClient, PresentedProjectile, PresentedSnapshot};

use crate::layers::{UiLayer, UiLayerVisibility};

#[derive(Component)]
struct Bo2mpGrenadeNode;

/// The picture's size and its distance from the crosshair (a 640x480 screen).
const ICON_SIZE: f32 = 25.0;
const ICON_OFFSET: f32 = 50.0;
/// The arrow's size, and its middle from the picture's middle before it
/// turns (its corners (-12, -27) and (13, -15): outside the picture, away
/// from the crosshair).
const POINTER_W: f32 = 25.0;
const POINTER_H: f32 = 12.0;
const POINTER_MID: (f32, f32) = (0.5, -21.0);
/// Its beat: brightest, dimmest (shown no brighter than full) and how many
/// times a second.
const PULSE_MAX: f64 = 1.85;
const PULSE_MIN: f64 = 0.3;
const PULSE_FREQ: f64 = 1.7;
/// The fastest a grenade can move and still show (units a second).
const MAX_SPEED: f32 = 100.0;
/// The warning's own range (a blast that reaches further takes over).
const MIN_RANGE: f32 = 250.0;
/// How far above his eye and below his feet it still shows; a ceiling over
/// him cuts it off this far above the ceiling.
const MAX_HEIGHT: f32 = 104.0;
const CEILING_SLACK: f32 = 70.0;
const CEILING_TRACE: f32 = 1024.0;
/// The most it shows at once.
const MAX_SHOWN: usize = 32;
/// BO2's `weapType`s that can warn: a grenade, a projectile, a mine.
const WEAPTYPE_SHOWN: [i32; 3] = [1, 2, 6];
/// BO2's `projExplosion`s that warn: a grenade, a heavy explosive, fire.
const EXPLOSION_SHOWN: [i32; 3] = [0, 6, 7];
/// BO2's `stickiness` "all" and "all but people": no height cut above him.
const STICKS_ALL: [i32; 2] = [1, 2];
/// The picture a grenade without its own shows.
const DEFAULT_ICON: &str = "hud_grenadeicon";
const POINTER: &str = "hud_grenadepointer";

#[derive(Default)]
struct GrenadeState {
    pictures: HashMap<String, Handle<Image>>,
    missing: HashSet<String>,
    /// His last ADS fraction and whether it was rising (zooming in).
    last_frac: f32,
    to_ads: bool,
    shown: bool,
    logged_at: f64,
}

#[allow(clippy::too_many_arguments)]
fn bo2mp_grenade(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    prediction: Option<Res<net::ClientPredictionState>>,
    cg_clock: Option<Res<net::FrameClock>>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time>,
    window: Query<&Window, With<PrimaryWindow>>,
    nodes: Query<Entity, With<Bo2mpGrenadeNode>>,
    view: Option<Res<frame::ViewSubject>>,
    mut st: Local<GrenadeState>,
) {
    // Last frame's warning goes; this frame's is drawn fresh.
    if st.shown {
        for e in &nodes {
            commands.entity(e).despawn();
        }
        st.shown = false;
    }
    let (Some(p), Some(l), Some(weapons)) = (presented.as_deref(), local.as_deref(), weapons.as_deref()) else {
        return;
    };
    let Some(snapshot) = p.snapshot() else { return };
    let dvars = snapshot.meta.script_dvars(l.0);
    // Only a BO2 multiplayer match, and never in hardcore.
    if dvars.string("bo2mp_gametype").is_none()
        || dvars.int("g_hardcore").or_else(|| dvars.int("scr_hardcore")).unwrap_or(0) != 0
    {
        return;
    }
    // Alive and playing (not dead, spectating or in a killcam).
    if view.as_deref().is_some_and(|v| v.in_killcam()) {
        return;
    }
    let Some(ps) = p.player(l.0).filter(|ps| ps.health > 0 && ps.pm_type < 2) else {
        return;
    };
    // Looking through a scope (a weapon with an overlay, zoomed in past a
    // hundredth): none.
    if ps.f_weapon_pos_frac > st.last_frac + 1e-4 {
        st.to_ads = true;
    } else if ps.f_weapon_pos_frac + 1e-4 < st.last_frac {
        st.to_ads = false;
    }
    st.last_frac = ps.f_weapon_pos_frac;
    let reg = &weapons.0;
    let held = playerstate_t6::game::get_viewmodel_weapon_index(ps);
    if let Some(f) = reg.facts_of(held)
        && (f.overlay_reticle != 0 || reg.overlay_material_of(held).is_some_and(|m| !m.is_empty()))
        && ps.f_weapon_pos_frac != 0.0
    {
        let frac = if st.to_ads { f.ads_zoom_in_frac } else { f.ads_zoom_out_frac }.max(1e-3);
        if (ps.f_weapon_pos_frac - (1.0 - frac)) / frac > 0.01 {
            return;
        }
    }

    let my_team = snapshot.meta.for_client(l.0).map_or(0, |m| m.client_state_team);
    let friendly_fire = dvars.int("bg_friendlyfire").or_else(|| dvars.int("scr_team_fftype")).unwrap_or(0) != 0;
    let cg_time = cg_clock
        .as_deref()
        .filter(|c| c.started())
        .map(|c| c.time())
        .unwrap_or_else(|| p.tick().map(sim::level_time_ms).unwrap_or(0));
    let at_time = p.trajectory_time_ms(cg_time);
    let me = ps.origin;
    // How high the ceiling over him is (traced once, when a grenade above
    // him needs it).
    let mut ceiling: Option<f32> = None;
    let mut roof = || {
        *ceiling.get_or_insert_with(|| {
            let Some(world) = prediction
                .as_deref()
                .filter(|p| p.0.is_armed() && p.0.world().has_world_clip())
                .map(|p| p.0.world())
            else {
                return f32::MAX;
            };
            let hit = world.trace_world(
                me,
                [me[0], me[1], me[2] + CEILING_TRACE],
                sim::bullet_collision::PLAYER_MINS,
                sim::bullet_collision::PLAYER_MAXS,
                sim::bullet_collision::MASK_PLAYER_SOLID,
            );
            if hit.fraction < 1.0 { hit.fraction * CEILING_TRACE } else { f32::MAX }
        })
    };

    let mut shown: Vec<([f32; 3], String)> = Vec::new();
    for row in p.presented_projectiles() {
        if shown.len() >= MAX_SHOWN {
            break;
        }
        let w = row.weapon();
        if reg.namespace_of(w) != Some(asset_core::AssetNamespace::T6) {
            continue;
        }
        let Some(f) = reg.facts_of(w) else { continue };
        if !WEAPTYPE_SHOWN.contains(&f.t6_weap_type)
            || !EXPLOSION_SHOWN.contains(&f.projectile_explosion_type)
            || !f.timed_detonation
        {
            continue;
        }
        // Never one stuck to him (BO2 links it to him); one stuck to
        // someone else shows however it moves.
        let stuck = match row {
            PresentedProjectile::Authoritative(s) => {
                if !s.live || s.rider.is_some_and(|r| r.client == l.0) {
                    continue;
                }
                s.rider.is_some()
            }
            PresentedProjectile::Predicted { .. } => false,
        };
        let owner = row.owner();
        if owner != l.0 && my_team != 0 && !friendly_fire {
            let theirs = snapshot.meta.for_client(owner).map_or(0, |m| m.client_state_team);
            if theirs == my_team {
                continue;
            }
        }
        // Still flying fast: not yet (a sitting one, TR_STATIONARY, has no
        // speed).
        let pos = row.pos();
        let speed2 = if pos.tr_type == 0 { 0.0 } else { pos.tr_delta.iter().map(|v| v * v).sum::<f32>() };
        if !stuck && speed2 > MAX_SPEED * MAX_SPEED {
            continue;
        }
        let at = row.origin_at(at_time);
        let d = [at[0] - me[0], at[1] - me[1], at[2] - me[2]];
        let range =
            if f.indicator_radius != 0 { f.indicator_radius as f32 } else { MIN_RANGE.max(f.explosion_radius as f32) };
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > range * range {
            continue;
        }
        if d[2] >= 0.0 {
            if !STICKS_ALL.contains(&f.stickiness)
                && d[2] > (ps.view_height_current + MAX_HEIGHT).min(roof() + CEILING_SLACK)
            {
                continue;
            }
        } else if d[2] < -MAX_HEIGHT {
            continue;
        }
        let icon = reg
            .reticle_of(w)
            .and_then(|r| r.indicator_icon.clone())
            .unwrap_or_else(|| DEFAULT_ICON.to_owned());
        shown.push((d, icon));
    }
    if shown.is_empty() {
        return;
    }

    // Pictures, loaded the first time each is needed.
    if let Some(icons) = icons.as_deref() {
        for name in std::iter::once(POINTER).chain(shown.iter().map(|(_, n)| n.as_str())) {
            if st.pictures.contains_key(name) || st.missing.contains(name) {
                continue;
            }
            match icons.0.iter().find(|(n, _)| n == name) {
                Some((_, image)) => {
                    let handle = images.add((**image).clone());
                    st.pictures.insert(name.to_owned(), handle);
                }
                None => {
                    st.missing.insert(name.to_owned());
                }
            }
        }
    }
    let now = time.elapsed_secs_f64() * 1000.0;
    if std::env::var_os("BO2MP_GRENADE_LOG").is_some() && now - st.logged_at >= 500.0 {
        st.logged_at = now;
        diag::info!(
            Ui,
            "bo2mp grenade: showing {} {:?}, facing {:.0}, pictures {:?} missing {:?}",
            shown.len(),
            shown.iter().map(|(d, n)| (d.map(|v| v.round()), n.as_str())).collect::<Vec<_>>(),
            ps.viewangles[1],
            st.pictures.keys().collect::<Vec<_>>(),
            st.missing
        );
    }

    let (sw, sh) = window.single().map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    let k = sh / 480.0;
    // White, beating (its alpha capped at full).
    let swing = (PULSE_MAX - PULSE_MIN) * 0.5;
    let alpha = ((f64::from(cg_time) * 0.006_283_185_4 * PULSE_FREQ).sin() * swing + swing + PULSE_MIN).clamp(0.0, 1.0)
        as f32;
    let pictures = st.pictures.clone();
    let spawn = |commands: &mut Commands, name: &str, node: Node, turn: f32, z: i32| {
        let Some(handle) = pictures.get(name) else { return };
        commands.spawn((
            Bo2mpGrenadeNode,
            UiLayer::Hud,
            UiLayerVisibility,
            ImageNode::new(handle.clone())
                .with_color(Color::srgba(1.0, 1.0, 1.0, alpha))
                .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
            UiTransform { rotation: Rot2::radians(turn), ..UiTransform::IDENTITY },
            ZIndex(z),
            node,
        ));
    };
    for (d, icon) in &shown {
        // Clockwise from straight up: where it is, as he faces.
        let yaw = if d[0] == 0.0 && d[1] == 0.0 { 0.0 } else { d[1].atan2(d[0]).to_degrees() };
        let turn = -(yaw - ps.viewangles[1]).rem_euclid(360.0).to_radians();
        let off = ICON_OFFSET * k;
        let (px, py) = (sw * 0.5 + turn.sin() * off, sh * 0.5 - turn.cos() * off);
        // The arrow turns about the picture's middle.
        let (mx, my) = (POINTER_MID.0 * k, POINTER_MID.1 * k);
        let (ax, ay) = (px + mx * turn.cos() - my * turn.sin(), py + mx * turn.sin() + my * turn.cos());
        let (w, h) = (POINTER_W * k, POINTER_H * k);
        let arrow = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(ax - w * 0.5),
            top: Val::Px(ay - h * 0.5),
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        };
        spawn(&mut commands, POINTER, arrow, turn, -2070);
        let s = ICON_SIZE * k;
        let picture = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(px - s * 0.5),
            top: Val::Px(py - s * 0.5),
            width: Val::Px(s),
            height: Val::Px(s),
            ..default()
        };
        spawn(&mut commands, icon, picture, 0.0, -2060);
    }
    st.shown = true;
}

pub(crate) fn register_bo2mp_grenade(app: &mut App) {
    app.add_systems(Update, bo2mp_grenade.in_set(ClientSet::Ui));
}
