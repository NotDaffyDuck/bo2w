//! Names floated over other players' heads: who qualifies, how long each
//! stays up, and the rows handed to `ui` to draw.

use std::collections::HashMap;

use bevy::prelude::*;
use hud_t6::flashbang::{ShockViewType, cg_flashbanged};
use hud_t6::font::R_TEXT_EM;
use hud_t6::overhead_names::*;
use net::{CEntity, CEntityRuntime, FrameClock, LocalPresentClient, PresentedSnapshot};

use crate::gaps::{GapCause, HudGap, HudPresentationGaps};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::surface::Hud2dSurface;

/// Where the render side found a player's head this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverheadPosedHead {
    ExactWorld([f32; 3]),
    NoDObjOrHead,
}

/// The posed heads of the frame, by entity number.
#[derive(Resource, Debug, Default)]
pub struct OverheadPosedPlayerFrame {
    heads: HashMap<u16, OverheadPosedHead>,
}

impl OverheadPosedPlayerFrame {
    pub fn replace(&mut self, rows: impl IntoIterator<Item = (u16, OverheadPosedHead)>) {
        self.heads.clear();
        self.heads.extend(rows);
    }

    pub(crate) fn get(&self, entnum: u16) -> Option<OverheadPosedHead> {
        self.heads.get(&entnum).copied()
    }
}

/// Ordering point: the posed heads are final for the frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverheadPosedPlayerFramePublished;

#[derive(Component)]
pub(crate) struct OverheadNamesRaster;

/// When a name first showed, when it was last shown, and whether the player
/// was a teammate at the time.
#[derive(Clone, Copy)]
struct NameSighting {
    first: i32,
    last: i32,
    friendly: bool,
}

#[derive(Default)]
pub(crate) struct NameMemory {
    last_time: Option<i32>,
    sightings: HashMap<u16, NameSighting>,
}

pub(crate) fn register(app: &mut App) {
    app.init_resource::<OverheadPosedPlayerFrame>().add_systems(
        Update,
        update_overhead_names
            .after(OverheadPosedPlayerFramePublished)
            .after(crate::surface::update_hud_surface)
            .before(crate::plugin::flush_overhead_names_tess)
            .before(crate::gaps::report_hud_gaps)
            .in_set(frame::LifeFrontPublished),
    );
}

/// The colour a name is drawn in. The game picks it relative to the viewer:
/// own side, the other side, or a spectator, at its default values (the ones
/// the zombies scoreboard script sets as well). The kill feed's tan and
/// blue-grey are the faction colours, a different pair.
fn relative_team_color(local_team: i32, target_team: i32) -> [f32; 4] {
    if target_team == entity_t6::team::SPECTATOR {
        [0.25, 0.25, 0.25, 1.0]
    } else if local_team != 0 && local_team == target_team {
        [0.4, 0.7, 0.4, 1.0]
    } else {
        [1.0, 0.315, 0.35, 1.0]
    }
}

/// A name with its colour escapes and control characters taken out, so a
/// player cannot dress up as a friend. Removing an escape can join an earlier
/// caret to a later digit (`^^12`); that pair goes too.
fn plain_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut plain = String::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        at += 1;
        let escape = c == '^' && chars.get(at).is_some_and(char::is_ascii_digit);
        if escape {
            at += 1;
        } else if c.is_control() {
            continue;
        } else if c.is_ascii_digit() && plain.ends_with('^') {
            plain.pop();
        } else {
            plain.push(c);
        }
    }
    plain
}

/// The other living players a HUD pass may mark (these names, the target
/// finder's diamonds): present in the next snapshot, a player entity, not
/// hidden or dead, not the local client, and alive in the meta.
pub(crate) fn live_other_player<'s>(
    snapshot: &'s sim::Snapshot,
    local: sim::ClientId,
    identity: &CEntity,
    runtime: &CEntityRuntime,
) -> Option<(sim::ClientId, &'s sim::ClientSnapshotMeta)> {
    const HIDDEN_FLAG: u32 = 0x20;
    let state = &runtime.next_state;
    let skipped_flags = HIDDEN_FLAG | playerstate_t6::game::eflags::DEAD;
    if !runtime.in_next_snap()
        || state.e_type != entity_t6::et::PLAYER
        || state.lerp.e_flags as u32 & skipped_flags != 0
    {
        return None;
    }
    let client = identity.client().filter(|&client| client != local)?;
    let meta = snapshot.meta.for_client(client)?;
    (meta.lifecycle == sim::ClientLifecycle::Alive).then_some((client, meta))
}

/// A world trace along `from` to `to` with the names' mask: how far it got
/// (as a fraction) and whether it began inside something solid.
pub(crate) fn name_trace(world: &sim::SimWorld, from: Vec3, to: Vec3) -> (f32, bool) {
    let hit = world.trace_world(
        from.to_array(),
        to.to_array(),
        [0.0; 3],
        [0.0; 3],
        OVERHEAD_TRACE_MASK,
    );
    (hit.fraction, hit.startsolid)
}

/// Nothing solid between the eye and the head.
pub(crate) fn head_visible(world: &sim::SimWorld, eye: Vec3, head: Vec3) -> bool {
    let (fraction, started_solid) = name_trace(world, eye, head);
    fraction >= 1.0 && !started_solid
}

/// How far along the view ray it enters the player's bounds (box around the
/// origin and head, so crouching and prone are covered), if it does.
pub(crate) fn aimed_distance(eye: Vec3, forward: Vec3, origin: Vec3, head: Vec3) -> Option<f32> {
    let low = origin.min(head) - Vec3::new(15.0, 15.0, 0.0);
    let high = origin.max(head) + Vec3::new(15.0, 15.0, 8.0);
    let mut enter = 0.0f32;
    let mut leave = CROSSHAIR_SCAN_DISTANCE;
    for axis in 0..3 {
        if forward[axis].abs() < 1e-6 {
            // Parallel to this slab: inside it the whole way or never.
            if eye[axis] < low[axis] || eye[axis] > high[axis] {
                return None;
            }
            continue;
        }
        let first = (low[axis] - eye[axis]) / forward[axis];
        let second = (high[axis] - eye[axis]) / forward[axis];
        enter = enter.max(first.min(second));
        leave = leave.min(first.max(second));
    }
    (enter <= leave).then_some(enter)
}

/// A player who could get a name this frame.
struct Candidate {
    ent: u16,
    team: i32,
    name: String,
    anchor: Vec3,
    pixel: Vec2,
    visible: bool,
    aimed: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
fn update_overhead_names(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    posed: Res<OverheadPosedPlayerFrame>,
    players: Query<(&CEntity, &CEntityRuntime)>,
    clock: Res<FrameClock>,
    surface: Res<Hud2dSurface>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prediction: Option<Res<net::ClientPredictionState>>,
    view: Res<frame::ViewSubject>,
    mut pass: ResMut<HudTessPass>,
    mut memory: Local<NameMemory>,
    mut gaps: ResMut<HudPresentationGaps>,
    mut rows: Option<ResMut<ui::OverheadNameRows>>,
) {
    pass.overhead_names = TessJob::Hide;
    if let Some(rows) = rows.as_deref_mut() {
        rows.0.clear();
    }
    gaps.clear(HudGap::OverheadNames);
    let now = clock.time();
    if memory.last_time.is_some_and(|last| now < last) {
        memory.sightings.clear();
    }
    memory.last_time = Some(now);

    let Some(snapshot) = presented.snapshot() else {
        memory.sightings.clear();
        return;
    };
    if !surface.is_ready() || view.in_killcam() {
        memory.sightings.clear();
        return;
    }
    let Some(player) = presented.alive_player(local.0) else {
        memory.sightings.clear();
        return;
    };
    let Some(local_meta) = snapshot.meta.for_client(local.0) else {
        return;
    };
    let blinded = cg_flashbanged(
        now,
        player.shellshock_time,
        player.shellshock_duration,
        presented
            .shellshock(local.0)
            .map_or(ShockViewType::Blurred, |shock| shock.screen_blend.kind),
    );
    if blinded {
        memory.sightings.clear();
        return;
    }
    let Some((camera, transform)) = cameras.iter().find(|(camera, _)| camera.is_active) else {
        return;
    };
    let Some(world) = prediction
        .as_ref()
        .filter(|state| state.0.is_armed() && state.0.world().has_world_clip())
        .map(|state| state.0.world())
    else {
        gaps.raise(GapCause::OverheadVisibilityUnavailable);
        return;
    };

    let eye = transform.translation();
    let forward = *transform.forward();
    let mut candidates = Vec::new();
    for (identity, runtime) in &players {
        let Some((_, meta)) = live_other_player(snapshot, local.0, identity, runtime) else {
            continue;
        };
        let name = entity_t6::client_state::c_str(&meta.name);
        if name.is_empty() {
            continue;
        }
        let ent = identity.number();
        let origin = Vec3::from_array(runtime.origin);
        let head = match posed.get(ent) {
            Some(OverheadPosedHead::ExactWorld(head)) => Vec3::from_array(head),
            Some(OverheadPosedHead::NoDObjOrHead) => {
                origin + Vec3::Z * (OVERHEAD_ORIGIN_FALLBACK_Z - OVERHEAD_HEAD_LIFT)
            }
            None => {
                gaps.raise(GapCause::OverheadHeadUnavailable { entnum: ent });
                continue;
            }
        };
        if eye.distance_squared(head) > OVERHEAD_MAX_DISTANCE_DEFAULT.powi(2) {
            continue;
        }
        let anchor = head + Vec3::Z * OVERHEAD_HEAD_LIFT;
        let Ok(pixel) = camera.world_to_viewport(transform, anchor) else {
            continue;
        };
        let in_window = pixel.is_finite()
            && (0.0..=surface.width()).contains(&pixel.x)
            && (0.0..=surface.height()).contains(&pixel.y);
        if !in_window {
            continue;
        }
        let visible = head_visible(world, eye, head);
        candidates.push(Candidate {
            ent,
            team: meta.client_state_team,
            name: plain_name(name),
            anchor,
            pixel,
            visible,
            aimed: aimed_distance(eye, forward, origin, head).filter(|_| visible),
        });
    }

    // Of the players under the crosshair, the nearest one earns a name.
    let aimed_at = candidates
        .iter()
        .filter_map(|candidate| candidate.aimed.map(|distance| (candidate.ent, distance)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(ent, _)| ent);
    memory
        .sightings
        .retain(|ent, _| candidates.iter().any(|candidate| candidate.ent == *ent));

    for candidate in candidates {
        let ent = candidate.ent;
        let friendly = local_meta.client_state_team != 0
            && local_meta.client_state_team == candidate.team;
        if candidate.visible && (friendly || aimed_at == Some(ent)) {
            let sighting = memory.sightings.entry(ent).or_insert(NameSighting {
                first: now,
                last: now,
                friendly,
            });
            if sighting.friendly != friendly {
                *sighting = NameSighting {
                    first: now,
                    last: now,
                    friendly,
                };
            }
            sighting.last = now;
        }
        let Some(sighting) = memory.sightings.get(&ent).copied() else {
            continue;
        };
        if sighting.friendly != friendly {
            memory.sightings.remove(&ent);
            continue;
        }
        let fade_ms = if friendly {
            FRIENDLY_NAME_FADE_OUT_DEFAULT_MS
        } else {
            ENEMY_NAME_FADE_MS
        };
        if now.wrapping_sub(sighting.last) >= fade_ms {
            memory.sightings.remove(&ent);
            continue;
        }
        let fade_in_ms = if friendly { 0 } else { ENEMY_NAME_FADE_MS };
        let alpha = overhead_fade_alpha(now, sighting.first, sighting.last, fade_in_ms, fade_ms);
        if alpha <= 0.0 {
            continue;
        }
        let distance_scale = overhead_distance_scale(
            eye.to_array(),
            candidate.anchor.to_array(),
            OVERHEAD_NEAR_DISTANCE_DEFAULT,
            OVERHEAD_FAR_DISTANCE_DEFAULT,
            OVERHEAD_FAR_SCALE_DEFAULT,
        );
        let Some(rows) = rows.as_deref_mut() else {
            continue;
        };
        let mut color = relative_team_color(local_meta.client_state_team, candidate.team);
        color[3] = alpha;
        // The size is in text ems in window pixels (720p config: 0.65), and
        // the name's baseline sits on the head point.
        let px = OVERHEAD_NAME_SIZE_DEFAULT * R_TEXT_EM * distance_scale;
        rows.0.push(ui::OverheadNameRow {
            ent,
            text: candidate.name,
            x: candidate.pixel.x,
            y: candidate.pixel.y.round() - px * 0.5,
            px,
            color,
        });
    }
}
