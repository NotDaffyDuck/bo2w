//! bo2mp lane C: Black Ops II's damage feedback in a multiplayer match, from
//! BO2's own pictures (common_mp): blood at the screen's edges when he is
//! hit (`overlay_low_health_splat`: BO2's blood colour shown through its
//! reveal map, further in the harder he is hit; mixed here as its
//! sw4_2d_blood technique does, with its BloodBrightness 5 and
//! BloodIntensity 4), the red low-health edge that beats
//! while he is badly hurt (`overlay_low_health`, below BO2's
//! `level.healthoverlaycutoff` 0.55 in `_healthoverlay.gsc`), and an arc
//! around the crosshair pointing back at whoever hit him (`hit_direction`).
//! The MW2 splatter (`hud`'s blood overlay) stays off in a BO2 match.

use bevy::prelude::*;
use frame::ClientStep;
use net::{HostViewClient, ShownWorldFrame};

use crate::{MainWindow, ScreenBox, pinned_node};
use crate::lui_hud::PictureCache;
use crate::tiers::{OverlayTier, OverlayStackShown};

#[derive(Component)]
struct Bo2mpDamageNode;

/// BO2's `level.healthoverlaycutoff`: the low-health edge shows below this
/// share of his health.
const LOW_HEALTH: f32 = 0.55;
/// How long a hit's direction arc stays, and its fade at the end (ms).
const HIT_MS: f64 = 2000.0;
const HIT_FADE_MS: f64 = 500.0;
/// The arc's size and distance from the crosshair (a 640x480 screen).
const ARC_W: f32 = 128.0;
const ARC_H: f32 = 64.0;
const ARC_OFFSET: f32 = 128.0;
/// The hit's blood fades by this much a second.
const SPLAT_FADE: f32 = 0.5;
/// The blood's mix (overlay_low_health_splat's constants) and how many
/// steps of it are made (each a picture, made once).
const BLOOD_BRIGHTNESS: f32 = 5.0;
const BLOOD_INTENSITY: f32 = 4.0;
const BLOOD_STEPS: usize = 16;
const BLOOD_SIZE: (usize, usize) = (512, 256);

#[derive(Default)]
struct DamageState {
    pictures: PictureCache,
    /// The last damage event seen and his health then.
    event: Option<(u32, i32)>,
    /// The hit's blood (0..1).
    splat: f32,
    /// Recent hits: when (ms) and the yaw back toward the attacker (deg).
    hits: Vec<(f64, f32)>,
    shown: bool,
    /// The blood at each step (made when first needed).
    blood: Vec<Option<Handle<Image>>>,
}

const PICTURES: [&str; 2] = ["overlay_low_health", "hit_direction"];

fn to_linear(c: f32) -> f32 {
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// BO2's blood at `amount` (0..1): its colour where the reveal map is
/// brighter than 1 - amount (the edges first), as RGBA8.
fn blood_picture(color: &Image, reveal: &Image, amount: f32) -> Option<Image> {
    let (cw, ch) = (color.texture_descriptor.size.width as usize, color.texture_descriptor.size.height as usize);
    let (rw, rh) = (reveal.texture_descriptor.size.width as usize, reveal.texture_descriptor.size.height as usize);
    let (c, r) = (color.data.as_deref()?, reveal.data.as_deref()?);
    if c.len() < cw * ch * 4 || r.len() < rw * rh * 4 {
        return None;
    }
    let (w, h) = BLOOD_SIZE;
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let ci = (((v * ch as f32) as usize).min(ch - 1) * cw + ((u * cw as f32) as usize).min(cw - 1)) * 4;
            let ri = (((v * rh as f32) as usize).min(rh - 1) * rw + ((u * rw as f32) as usize).min(rw - 1)) * 4;
            let reveal = f32::from(r[ri]) / 255.0;
            // The intensity scales both the reveal and the blood's own alpha.
            let a = ((reveal - (1.0 - amount)) * BLOOD_INTENSITY).clamp(0.0, 1.0)
                * (f32::from(c[ci + 3]) / 255.0 * BLOOD_INTENSITY).min(1.0);
            let o = (y * w + x) * 4;
            // (The brightness scales light, not the stored sRGB value.)
            for k in 0..3 {
                out[o + k] = (to_srgb(to_linear(f32::from(c[ci + k]) / 255.0) * BLOOD_BRIGHTNESS) * 255.0).round() as u8;
            }
            out[o + 3] = (a * 255.0) as u8;
        }
    }
    Some(crate::lui_scene::srgb_image(w, h, out))
}

/// What the local player sees this frame: the world feed, which client he is, and whose eyes the view is in.
#[derive(bevy::ecs::system::SystemParam)]
struct LocalFeed<'w> {
    world: Option<Res<'w, ShownWorldFrame>>,
    me: Option<Res<'w, HostViewClient>>,
    view: Option<Res<'w, frame::EyeSubject>>,
}

/// The packs' HUD pictures and the asset store the blood is copied into.
#[derive(bevy::ecs::system::SystemParam)]
struct BloodPictures<'w> {
    icons: Option<Res<'w, assets::T6HudIcons>>,
    pics: ResMut<'w, Assets<Image>>,
}

fn bo2mp_damage(
    mut commands: Commands,
    feed: LocalFeed,
    store: BloodPictures,
    time: Res<Time>,
    window: MainWindow,
    nodes: Query<Entity, With<Bo2mpDamageNode>>,
    mut st: Local<DamageState>,
) {
    let LocalFeed { world: presented, me: local, view } = feed;
    let BloodPictures { icons, mut pics } = store;
    let clear = |commands: &mut Commands, st: &mut DamageState| {
        if st.shown {
            for e in &nodes {
                commands.entity(e).try_despawn();
            }
            st.shown = false;
        }
    };
    let (Some(p), Some(l)) = (presented.as_deref(), local.as_deref()) else {
        clear(&mut commands, &mut st);
        return;
    };
    // Only a BO2 multiplayer match (its screen feed is there).
    if !p.snapshot().is_some_and(|s| s.meta.script_dvars(l.0).string("bo2mp_gametype").is_some()) {
        clear(&mut commands, &mut st);
        st.event = None;
        return;
    }
    if st.pictures.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for name in PICTURES {
            if let Some(image) = crate::hud_picture(icons, name) {
                st.pictures.keep(name.to_owned(), &pics.add((**image).clone()));
            }
        }
        // Every step of the blood at once (each on the GPU before it is
        // needed).
        let find = |name: &str| icons.0.iter().find(|(n, _)| n == name).map(|(_, i)| i.clone());
        if let (Some(color), Some(reveal)) = (find("iw2_blood_color"), find("iw2_blood_reveal")) {
            st.blood = (0..=BLOOD_STEPS)
                .map(|k| (k > 0).then(|| blood_picture(&color, &reveal, k as f32 / BLOOD_STEPS as f32)).flatten().map(|i| pics.add(i)))
                .collect();
        }
    }
    let now = time.elapsed_secs_f64() * 1000.0;
    // A killcam shows the killer's view: the player in the snapshot is the
    // killer (alive, maybe hurt), but the damage feedback is the local
    // player's own, so none of it draws (as every other HUD part hides
    // while the view is seated: compass, kill feed, score bar, names).
    let seated = view.as_deref().is_some_and(|v| v.replaying_final_death());
    // Alive and playing (not dead, spectating or in a killcam).
    let Some(pstate) = p
        .player(l.0)
        .filter(|pstate| !seated && pstate.health > 0 && pstate.hit_point_ceiling > 0 && pstate.movement_mode < 2)
        .copied()
    else {
        clear(&mut commands, &mut st);
        *st = DamageState { pictures: std::mem::take(&mut st.pictures), blood: std::mem::take(&mut st.blood), ..default() };
        return;
    };
    // A new hit: blood by how much it took, and its direction.
    // (A new life's full health is no hit.)
    if let Some((event, health)) = st.event
        && event != pstate.damage_event
        && pstate.health < health
    {
        let (took, health_now) = (health - pstate.health, pstate.health);
        let lost = took.max(0) as f32 / pstate.hit_point_ceiling as f32;
        st.splat = (st.splat + 0.25 + lost * 2.0).min(1.0);
        // The damage bytes are the hit's travel (Black Ops II's), so
        // the attacker is back the other way; 255/255 = no direction.
        let from = (pstate.damage_yaw != 255 || pstate.damage_pitch != 255)
            .then(|| (pstate.damage_yaw as f32 * 360.0 / 256.0 + 180.0) % 360.0);
        if let Some(from) = from {
            st.hits.push((now, from));
        }
        diag::info!(
            Ui,
            "bo2mp damage: took {} (health now {}), came from yaw {:?}, view yaw {:.0}",
            took,
            health_now,
            from.map(|y| y.round()),
            pstate.look_angles[1]
        );
        if st.hits.len() > 4 {
            st.hits.remove(0);
        }
    }
    st.event = Some((pstate.damage_event, pstate.health));
    st.hits.retain(|(at, _)| now - at < HIT_MS);
    let frac = (pstate.health as f32 / pstate.hit_point_ceiling as f32).clamp(0.0, 1.0);
    let low = ((LOW_HEALTH - frac) / LOW_HEALTH).clamp(0.0, 1.0);
    // The hit's extra blood fades; what stays is how hurt he is (it clears
    // as his health comes back).
    st.splat = (st.splat - time.delta_secs() * SPLAT_FADE).max((1.0 - frac) * 1.25);
    // The low-health edge beats (about once a second).
    let beat = 0.75 + 0.25 * (now as f32 / 1000.0 * std::f32::consts::TAU).sin().abs();
    let low_alpha = low * beat;

    clear(&mut commands, &mut st);
    if st.splat <= 0.0 && low_alpha <= 0.0 && st.hits.is_empty() {
        return;
    }
    let (sw, sh) = window.shown().map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    if std::env::var_os("BO2MP_DAMAGE_LOG").is_some() && (now / 500.0) as i64 != ((now - time.delta_secs_f64() * 1000.0) / 500.0) as i64 {
        let (arcs, steps_ready) = (st.hits.len(), st.blood.iter().flatten().count());
        diag::info!(
            Ui,
            "bo2mp damage: blood {:.2} edge {low_alpha:.2}, {} arcs, pictures {:?}, {} blood steps ready",
            st.splat,
            arcs,
            st.pictures.keys().collect::<Vec<_>>(),
            steps_ready
        );
    }
    let k = sh / 480.0;
    let full = Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..pinned_node(0.0, 0.0)
    };
    // Under the HUD: the beating edge, the blood, then the arcs.
    if low_alpha > 0.0
        && let Some(edge) = st.pictures.kept("overlay_low_health")
    {
        commands.spawn(damage_picture(edge, low_alpha, full.clone(), 0.0, -2100));
    }
    let step = ((st.splat * BLOOD_STEPS as f32).round() as usize).min(BLOOD_STEPS);
    if step > 0
        && let Some(Some(blood)) = st.blood.get(step)
    {
        commands.spawn(damage_picture(blood.clone(), 1.0, full.clone(), 0.0, -2090));
    }
    let Some(arc) = st.pictures.kept("hit_direction") else {
        st.shown = true;
        return;
    };
    for &(at, yaw) in &st.hits {
        let age = now - at;
        let alpha = if age < HIT_MS - HIT_FADE_MS { 1.0 } else { ((HIT_MS - age) / HIT_FADE_MS) as f32 };
        // Clockwise from straight up: where the hit came from, as he faces.
        let turn = -(yaw - pstate.look_angles[1]).to_radians();
        let (w, h, off) = (ARC_W * k, ARC_H * k, ARC_OFFSET * k);
        let (cx, cy) = (sw * 0.5 + turn.sin() * off, sh * 0.5 - turn.cos() * off);
        let node = ScreenBox {
            left_px: cx - w * 0.5,
            top_px: cy - h * 0.5,
            width_px: w,
            height_px: h,
        }.node();
        commands.spawn(damage_picture(arc.clone(), alpha.clamp(0.0, 1.0), node, turn, -2080));
    }
    st.shown = true;
}

/// One of the damage pictures, stretched over `node`, faded to `alpha`, turned by `turn` radians and drawn at depth `z`.
fn damage_picture(picture: Handle<Image>, alpha: f32, node: Node, turn: f32, z: i32) -> impl Bundle {
    (
        Bo2mpDamageNode,
        OverlayTier::Hud,
        OverlayStackShown,
        ImageNode::new(picture)
            .with_color(Color::srgba(1.0, 1.0, 1.0, alpha))
            .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
        UiTransform { rotation: Rot2::radians(turn), ..UiTransform::IDENTITY },
        ZIndex(z),
        node,
    )
}

pub(crate) fn register_bo2mp_damage(host: &mut App) {
    host.add_systems(Update, bo2mp_damage.in_set(ClientStep::Ui));
}
