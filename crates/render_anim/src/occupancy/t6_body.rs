//! bo2mp: other players, their corpses and the guns and bags on the ground,
//! as Black Ops II draws them, through the same fallback pass as its
//! projectiles and script models (BO2's own materials, lit by the map's
//! light grid).
//!
//! - A player's body is the model BO2's faction scripts gave him
//!   (`setmodel` in `mpbody/class_*`, sent as his `bo2mp_body` client
//!   value), the gun in his hands its BO2 world model (with its attachments)
//!   on the body's `tag_weapon_right`.
//! - His pose: the legs and torso animations the server picked from BO2's
//!   own `mp/playeranim.script` (his state's `legs_anim` / `torso_anim`),
//!   each faded in over its blend time; the torso's animation drives the
//!   upper body, the legs' the rest; his view's pitch bends the spine
//!   through the body's controller tags (`back_low`, `back_mid`, `back_up`).
//!   Nothing is ever drawn in bind pose: a part with no animation plays
//!   the script's standing idle.
//! - A corpse (the engine's clone of a dead player) plays his death
//!   animation to its end and stays in its last pose.
//! - A dropped gun or Scavenger bag: its world model and attachments where
//!   the item lies (a bag only for players with Scavenger, as BO2 shows it).
//! - A player sliding (his legs in one of the script's `slide`
//!   animations): BO2's slide_rumble on him as he starts, stopped as he
//!   stops, and its slide dust between his ankles each time it is due. The
//!   viewer's own too, in first person (BO2 runs it for every player before
//!   it asks whose view it is), with his body's dive sounds and dtp_rumble;
//!   his body is still not drawn.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use entity_t6::flight::path_kind;
use entity_t6::{et, evaluate_trajectory};
use frame::EyeSubject;
use net::{ClientEntityRecord, ClientEntityPoseState, FrameClock, HostViewClient, PlayerDrawGate, ShownWorldFrame};
use playerstate_t6::game::entity_flag_bits;
use playerstate_t6::perks::PERK_SCAVENGER;
use anim_t6::{
    T6AnimFacts, T6Blend, T6PlayerAnims, T6_STANCE_TRANSITION_MS, t6_anim_duration, t6_anim_index,
    t6_anim_part, t6_blend_ms, t6_blend_weight, t6_legs_yaw, t6_loop_start, t6_moving_rate,
    t6_stance_hold_ms, t6_verify_anim,
};
use sim::body_anim_script::T6PlayerAnimsRes;
use xmodel_runtime::DObjAnim as _;

use crate::occupancy::missile::{PosedVerts, T6MissileModel, skin_verts};
use crate::occupancy::ragdoll::{Ragdoll, joint_points};
use diag::{info, warn};

/// BO2's ragdoll trajectory types: limp, limp and thrown, at rest.
const RAGDOLL_TR_TYPES: core::ops::RangeInclusive<u8> =
    entity_t6::path_kind::FIRST_RAGDOLL..=entity_t6::path_kind::LIMP_LAST;
/// Limp and thrown (a `launchragdoll` when its duration is 1).
const TR_RAGDOLL_GRAVITY: u8 = entity_t6::path_kind::LIMP_FALLING;
use render_scene::FirstPersonLens;

/// This frame's BO2 player bodies, corpses and ground items, for the
/// fallback pass.
#[derive(Resource, Default, Clone)]
pub struct T6BodyModels {
    pub items: Vec<T6MissileModel>,
}

/// The client value that names a player's body (see the server's
/// `t6::playeranim`): `<model>` then `;<model>@<tag>` per attachment.
const BODY_DVAR: &str = "bo2mp_body";

/// How fast the drawn legs ease to the server's legs yaw (degrees/s).
const LEGS_YAW_SPEED_DEG: f32 = 360.0;

/// How long a dropped gun takes to tip onto its side as it falls (ms).
const ITEM_TIP_MS: f32 = 250.0;

/// The torso's animation outweighs the legs' on the bones both move (the
/// weights are normalised per bone): the legs' only show where the torso's
/// has no track.
const TORSO_OVER_LEGS: f32 = 1000.0;

/// This frame's posed players, for the effects bolted to their guns (muzzle
/// flashes, shell casings) and for tracers' start: each one's bones (in its
/// own space), where it is, and its gun's flash and brass tags.
#[derive(Resource, Default)]
struct T6BodyPoses {
    rows: Vec<PosedBody>,
    local_corpse_root: Option<([f32; 3], i32)>,
    /// Each posed body's head (`j_head`) in the world: where the names over
    /// players' heads hang.
    heads: Vec<(u16, [f32; 3])>,
}

/// Where the local player's own corpse's `j_mainroot` is in the world this
/// frame (the death-watch camera's origin while he is dead, as the engine's
/// `CG_OffsetThirdPersonView` takes it), or none with no such corpse; with
/// the time that body first showed, which tells his bodies apart.
#[derive(Resource, Default)]
pub struct LocalCorpseRoot(pub Option<([f32; 3], i32)>);

struct PosedBody {
    id: Entity,
    body_no: u16,
    scene_from_local: Mat4,
    bones: Vec<Mat4>,
    flash: Option<u16>,
    brass: Option<u16>,
}

/// A system that took long logs it (a frame's stall named), and how much
/// of that its thread spent working (the rest it waited: on the disk, a
/// lock, or other programs).
pub(crate) struct SlowGuard(std::time::Instant, &'static str, Option<f64>);

impl SlowGuard {
    pub(crate) fn new(name: &'static str) -> Self {
        Self(clock_now(), name, diag::thread_cpu_ms())
    }
}

impl Drop for SlowGuard {
    fn drop(&mut self) {
        let ms = self.0.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut totals) = SYSTEM_TOTALS.lock() {
            match totals.iter_mut().find(|(name, _)| *name == self.1) {
                Some(row) => row.1 += ms,
                None => totals.push((self.1, ms)),
            }
        }
        if ms > 20.0 {
            warn!(World, "bo2mp slow: {} took {ms:.1} ms{}", self.1, diag::busy_note(self.2));
        }
    }
}

/// Each guarded system's time summed since the last frame-parts line (the
/// systems that fill a stretch, logged with it).
static SYSTEM_TOTALS: std::sync::Mutex<Vec<(&'static str, f64)>> = std::sync::Mutex::new(Vec::new());

/// When each part of the last frame began: First, PreUpdate, Update,
/// PostUpdate, Last (finding which part a stall is in).
static FRAME_MARKS: std::sync::Mutex<[Option<std::time::Instant>; 5]> = std::sync::Mutex::new([None; 5]);

fn mark_frame<const PART: usize>() {
    if let Ok(mut marks) = FRAME_MARKS.lock() {
        marks[PART] = Some(clock_now());
    }
}

/// The frame length (ms) above which a gap is logged: 250 unless `BO2MP_FRAME_GAP_MS` lowers it
/// (a measured run wants the 40 to 100 ms hitches a player feels, not only the stalls).
fn frame_gap_floor_ms() -> f64 {
    static FLOOR: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| {
        std::env::var("BO2MP_FRAME_GAP_MS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(250.0)
    })
}

/// A frame that came long after the last one, logged with the time each
/// part of it took (finding stalls). "after Last" is the render world's
/// extraction and the wait for the frame to show.
fn log_frame_gaps(mut last: Local<Option<std::time::Instant>>, mut last_cpu: Local<Option<f64>>) {
    let now = clock_now();
    let cpu = diag::process_cpu_ms();
    let marks = FRAME_MARKS.lock().map(|m| *m).unwrap_or([None; 5]);
    if let Some(prev) = *last {
        let ms = (now - prev).as_secs_f64() * 1000.0;
        if ms > frame_gap_floor_ms() {
            let span = |a: Option<std::time::Instant>, b: Option<std::time::Instant>| match (a, b) {
                (Some(a), Some(b)) if b >= a => format!("{:.0}", (b - a).as_secs_f64() * 1000.0),
                _ => "?".to_owned(),
            };
            diag::warn!(
                World,
                "bo2mp frame gap: {ms:.0} ms (First {} ms, PreUpdate+fixed {} ms, Update {} ms, PostUpdate {} ms, after Last {} ms; Update stretches: {}){}",
                span(marks[0], marks[1]),
                span(marks[1], marks[2]),
                span(marks[2], marks[3]),
                span(marks[3], marks[4]),
                span(marks[4], Some(now)),
                slow_update_spans(),
                match (*last_cpu, cpu) {
                    (Some(a), Some(b)) => format!(", all threads worked {:.0} ms of it", (b - a).max(0.0)),
                    _ => String::new(),
                },
            );
        }
        add_part_times(&marks, now);
    }
    *last = Some(now);
    *last_cpu = cpu;
    mark_frame::<0>();
}

/// Each part's time summed over the last 10 s: the average frame's parts
/// logged (where a low frame rate's time goes, not only its stalls).
struct PartTotals {
    since: std::time::Instant,
    frame_total: u32,
    pieces: [f64; 5],
    refresh: [f64; 15],
}

static PART_TOTALS: std::sync::Mutex<Option<PartTotals>> = std::sync::Mutex::new(None);

fn add_part_times(marks: &[Option<std::time::Instant>; 5], now: std::time::Instant) {
    let ms = |a: Option<std::time::Instant>, b: Option<std::time::Instant>| match (a, b) {
        (Some(a), Some(b)) if b >= a => (b - a).as_secs_f64() * 1000.0,
        _ => 0.0,
    };
    let update = UPDATE_MARKS.lock().map(|m| *m).unwrap_or([None; 15]);
    let Ok(mut totals) = PART_TOTALS.lock() else { return };
    let t = totals.get_or_insert_with(|| PartTotals { since: now, frame_total: 0, pieces: [0.0; 5], refresh: [0.0; 15] });
    t.frame_total += 1;
    for k in 0..4 {
        t.pieces[k] += ms(marks[k], marks[k + 1]);
    }
    t.pieces[4] += ms(marks[4], Some(now));
    for k in 1..update.len() {
        t.refresh[k] += ms(update[k - 1], update[k]);
    }
    let secs = (now - t.since).as_secs_f64();
    if secs < 10.0 {
        return;
    }
    let n = f64::from(t.frame_total.max(1));
    let stretches: Vec<String> = (1..15)
        .filter(|k| t.refresh[*k] / n >= 0.5)
        .map(|k| format!("{} {:.1}", UPDATE_SPANS[k], t.refresh[k] / n))
        .collect::<_>();
    diag::info!(
        World,
        "bo2mp frame parts: {:.1} fps over {secs:.0} s, ms per frame: First {:.1}, PreUpdate+fixed {:.1}, Update {:.1}, PostUpdate {:.1}, after Last {:.1}; Update: {}",
        n / secs,
        t.pieces[0] / n,
        t.pieces[1] / n,
        t.pieces[2] / n,
        t.pieces[3] / n,
        t.pieces[4] / n,
        stretches.join(", ")
    );
    let mut systems = SYSTEM_TOTALS.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
    systems.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<String> = systems
        .iter()
        .take(10)
        .filter(|(_k, ms)| ms / n >= 0.05)
        .map(|(label, ms)| format!("{label} {:.2}", ms / n))
        .collect::<_>();
    if !top.is_empty() {
        info!(World, "bo2mp systems (ms a frame, 10 s): {}", top.join(", "));
    }
    *t = PartTotals { since: now, frame_total: 0, pieces: [0.0; 5], refresh: [0.0; 15] };
}

/// When each stretch of Update ended (its client sets in order), for the
/// late-frame log.
const UPDATE_SPANS: [&str; 15] = [
    "start", "Load", "Receive", "Reconcile", "Input", "Predict", "Send", "FrontendPrepare", "Anim", "Fx",
    "FrontendAssemble", "Gpu", "Ui", "Effects", "Diag",
];
static UPDATE_MARKS: std::sync::Mutex<[Option<std::time::Instant>; 15]> = std::sync::Mutex::new([None; 15]);

fn mark_update<const K: usize>() {
    if let Ok(mut marks) = UPDATE_MARKS.lock() {
        marks[K] = Some(clock_now());
    }
}

/// The stretches of the last Update that took 10 ms or more.
fn slow_update_spans() -> String {
    let marks = UPDATE_MARKS.lock().map(|m| *m).unwrap_or([None; 15]);
    let mut result = Vec::new();
    for k in 1..marks.len() {
        if let (Some(a), Some(b)) = (marks[k - 1], marks[k])
            && b >= a
        {
            let ms = (b - a).as_secs_f64() * 1000.0;
            if ms >= 10.0 {
                result.push(format!("{} {ms:.0}", UPDATE_SPANS[k]));
            }
        }
    }
    result.join(", ")
}

fn register_update_marks(application: &mut App) {
    use frame::{ClientStep, DrawPassStage};
    application.add_systems(
        Update,
        (
            mark_update::<0>.before(ClientStep::Load),
            mark_update::<1>.after(ClientStep::Load).before(ClientStep::Receive),
            mark_update::<2>.after(ClientStep::Receive).before(ClientStep::Reconcile),
            mark_update::<3>.after(ClientStep::Reconcile).before(ClientStep::Input),
            mark_update::<4>.after(ClientStep::Input).before(ClientStep::Predict),
            mark_update::<5>.after(ClientStep::Predict).before(ClientStep::Send),
            mark_update::<6>.after(ClientStep::Send).before(ClientStep::Present),
            mark_update::<7>.after(DrawPassStage::FrontPrep).before(DrawPassStage::Anim),
            mark_update::<8>.after(DrawPassStage::Anim).before(DrawPassStage::Fx),
            mark_update::<9>.after(DrawPassStage::Fx).before(DrawPassStage::FrontBuild),
            mark_update::<10>.after(DrawPassStage::FrontBuild).before(DrawPassStage::Gpu),
            mark_update::<11>.after(ClientStep::Present).before(ClientStep::Ui),
            mark_update::<12>.after(ClientStep::Ui).before(ClientStep::Effects),
            mark_update::<13>.after(ClientStep::Effects).before(ClientStep::Diag),
            mark_update::<14>.after(ClientStep::Diag),
        ),
    );
}

pub fn register_t6_body_systems(app_in: &mut App) {
    register_update_marks(app_in);
    crate::occupancy::t6_sounds::register_t6_sound_systems(app_in);
    app_in.add_systems(First, log_frame_gaps)
        .add_systems(bevy::app::PreUpdate, mark_frame::<1>)
        .add_systems(bevy::app::Update, mark_frame::<2>)
        .add_systems(bevy::app::PostUpdate, mark_frame::<3>)
        .add_systems(bevy::app::Last, mark_frame::<4>);
    app_in.init_resource::<T6ItemShapes>()
        .add_systems(bevy::app::Update, prepare_t6_bodies.before(collect_t6_bodies));
    app_in.init_resource::<T6BodyModels>()
        .init_resource::<T6BodyPoses>()
        .init_resource::<LocalCorpseRoot>()
        .add_message::<T6SlideDust>()
        .add_systems(
            Update,
            collect_t6_bodies
                .in_set(frame::DrawPassStage::Anim)
                .after(net::ShownStatePosted)
                .before(frame::JobQueueStage::CellDynModel),
        )
        .add_systems(
            Update,
            publish_t6_body_poses
                .after(collect_t6_bodies)
                .after(crate::motion::puppet_pose::start_skeleton_pose_tick)
                .after(frame::JobQueueStage::SkinModel)
                .before(frame::JobQueueStage::FxRemaining)
                .in_set(T6BodyPosesPublished).in_set(frame::ClientStep::Present),
        );
}

/// The BO2 bodies' bones and heads are published for this frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct T6BodyPosesPublished;

/// Each posed player's bones go where the effect system looks for bolted
/// effects (his number), and his entity carries his gun's flash and brass
/// tags, as the engine's own remote bodies do: his muzzle flash and shell
/// casings play from his gun, his tracers start at its muzzle.
fn publish_t6_body_poses(
    poses: Res<T6BodyPoses>,
    mut corpse_root: ResMut<LocalCorpseRoot>,
    mut dobj_frame: ResMut<crate::motion::puppet_pose::HostSkeletonFrame>,
    mut posed_players: ResMut<crate::motion::puppet_pose::AnimatedBodyFrame>,
    mut cmds: Commands,
) {
    corpse_root.0 = poses.local_corpse_root;
    for &(entnum, head) in &poses.heads {
        posed_players.post_body(crate::motion::puppet_pose::AnimatedBody {
            ent_ix: entnum,
            head: crate::motion::puppet_pose::AnimatedHeadMode::Exact(Vec3::from_array(head)),
        });
    }
    for row in &poses.rows {
        let _ = dobj_frame.publish(u32::from(row.body_no), true, 0, row.scene_from_local, &row.bones);
        cmds
            .entity(row.id)
            .try_insert(crate::occupancy::peer_body::OtherBodyEffectTags {
                knife: None,
                laser: None,
                brass: bolt_on_gun(row, row.brass),
                flash: bolt_on_gun(row, row.flash),
            });
    }
}

/// Where an effect bolted to one of this body's bones plays from, or none when
/// the body has no such bone.
fn bolt_on_gun(row: &PosedBody, tag: Option<u16>) -> Option<fx::EffectBoltTarget> {
    let tag = tag?;
    let bone_to_body = *row.bones.get(usize::from(tag))?;
    let at = xmodel_runtime::bone_orientation(&[row.scene_from_local * bone_to_body], 0).ok()?;
    Some(fx::EffectBoltTarget {
        orientation: fx::AttachPointFrame {
            axis: at.basis_dirs,
            origin: at.at,
        },
        client_actor_teleport: false,
        bone: tag,
        dobj: u32::from(row.body_no),
    })
}

#[derive(Clone, Copy, Default, Debug)]
struct Playing {
    value: i32,
    /// How far into its clip (seconds of clip time).
    phase: f32,
}

/// What a part's animation change happens with.
#[derive(Clone, Copy, Debug)]
struct Change {
    now: i32,
    /// The body's first frame: no blend.
    first: bool,
    /// What is left of a stance change's hold on the legs' blends (ms).
    stance_hold_ms: i32,
    /// His client number (where his moving loops start).
    client_no: u32,
}

/// Animations still fading out on a part (BO2's tree has no cap; past this
/// the faintest is dropped).
const FADING_MAX: usize = 6;

#[derive(Clone, Copy, Default, Debug)]
struct PartTrack {
    cur: Playing,
    /// The current animation's weight when it was switched to (BO2 blends a
    /// goal weight from the weight the animation has, not from 0).
    cur_from: f32,
    /// The others still fading out, each with its weight at the switch.
    fading: [Option<(Playing, f32)>; FADING_MAX],
    switched: i32,
    crossfade_millis: i32,
}

impl PartTrack {
    /// A new animation on this part, as BG_SetNewAnimation takes it: it
    /// blends in by BO2's rules ([`t6_blend_ms`]), every other animation on
    /// the part blending out over the same time (XAnimClearTreeGoalWeights),
    /// each from the weight it has now. A moving loop with no weight left
    /// starts at the time of the moving loop it takes over from, or where
    /// the clock and his client number put it when that was none
    /// ([`t6_loop_start`]); one still fading keeps its time. `clip` gives a
    /// value's clip (length in seconds, looping).
    fn see(
        &mut self,
        value: i32,
        change: Change,
        script: &T6PlayerAnims,
        reel: &dyn Fn(i32) -> Option<(f32, bool)>,
    ) {
        let value = t6_anim_part(value);
        if !change.first && value == self.cur.value {
            return;
        }
        let new = t6_anim_index(value);
        let old = t6_anim_index(self.cur.value).filter(|_| !change.first);
        let command = new.and_then(|a| script.command_of(a));
        let moving = |a: Option<u16>| a.is_some_and(|a| script.speed_of(a) > 0.0);
        let blend = t6_blend_ms(&T6Blend {
            new_index: new.map_or(0, |a| a + 1),
            initial_lerp: command.and_then(|c| c.crossfade_millis).unwrap_or(-1),
            final_lerp: command.and_then(|c| c.blend_out_ms).unwrap_or(-1),
            old_exists: old.is_some(),
            old_moving: moving(old),
            new_moving: moving(new),
            sync_root: false,
            stance_hold_ms: change.stance_hold_ms,
            first: change.first,
        });
        // Every animation on the part with the weight it has now.
        let mut all = [None; FADING_MAX + 1];
        if !change.first && self.cur.value != 0 {
            all[0] = Some((self.cur, self.cur_weight(change.now)));
            let s = self.progress(change.now);
            for (slot, f) in all[1..].iter_mut().zip(self.fading) {
                *slot = f.map(|(p, w0)| (p, w0 * (1.0 - s)));
            }
        }
        let same = |p: &Playing| new.is_some() && t6_anim_index(p.value) == new;
        let kept = all.iter().flatten().find(|(p, w)| same(p) && *w > 0.0).copied();
        let mut phase = 0.0;
        if moving(new)
            && let Some((len, true)) = reel(value)
        {
            if let Some((p, _)) = kept {
                phase = p.phase;
            } else {
                let old_clip = old.and_then(|_| reel(self.cur.value));
                phase = match old_clip {
                    Some((old_len, true)) if moving(old) => {
                        (self.cur.phase / old_len.max(1e-3)).rem_euclid(1.0) * len
                    }
                    _ => {
                        let old_ms = old_clip.map(|(l, _)| (l * 1000.0) as i32);
                        t6_loop_start(change.now, old_ms, change.client_no) * len
                    }
                };
            }
        }
        let mut fading: Vec<(Playing, f32)> =
            all.iter().flatten().filter(|(p, w)| !same(p) && *w > 0.01).copied().collect();
        fading.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.fading = [None; FADING_MAX];
        for (slot, f) in self.fading.iter_mut().zip(fading) {
            *slot = Some(f);
        }
        self.cur = Playing { value, phase };
        self.cur_from = kept.map_or(0.0, |(_, w)| w);
        self.switched = change.now;
        self.crossfade_millis = blend.max(1);
    }

    /// How far the last switch's blend has gone (0 to 1).
    fn progress(&self, now: i32) -> f32 {
        t6_blend_weight(now, self.switched, self.crossfade_millis)
    }

    fn cur_weight(&self, now: i32) -> f32 {
        if self.fading.iter().all(Option::is_none) {
            return 1.0;
        }
        self.cur_from + (1.0 - self.cur_from) * self.progress(now)
    }

    /// BG_PlayerAnimation_VerifyAnim: an old animation is cleared once its
    /// weight reaches 0 (the new one is all in).
    fn verify(&mut self, now: i32) {
        if t6_verify_anim(1.0 - self.progress(now)) {
            self.fading = [None; FADING_MAX];
        }
    }

    /// Every animation on by `dt` seconds at `rate`.
    fn step_on(&mut self, dt: f32, rate: f32) {
        self.cur.phase += dt * rate;
        for (p, _) in self.fading.iter_mut().flatten() {
            p.phase += dt * rate;
        }
    }

    /// (playing, weight) pairs: the current one first, fading in over the
    /// others.
    fn weighted(&self, now: i32) -> Vec<(Playing, f32)> {
        let s = self.progress(now);
        let mut out = vec![(self.cur, self.cur_weight(now))];
        out.extend(self.fading.iter().flatten().map(|&(p, w0)| (p, w0 * (1.0 - s))).filter(|(_, w)| *w > 0.0));
        out
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct BodyTrack {
    legs: PartTrack,
    torso: PartTrack,
    /// The legs' yaw offset as drawn (degrees), easing to the server's.
    legs_yaw: f32,
    seen: i32,
    corpse: bool,
    /// His stance (0 stand, 1 crouch, 2 prone) and when its last change's
    /// hold on the legs' blends ends.
    stance: u8,
    stance_until: i32,
    /// The legs' animation and its phase last frame (for its notetracks).
    heard: Option<Playing>,
}

#[derive(Default)]
struct BodyLocals {
    tracks: HashMap<u16, BodyTrack>,
    /// Each corpse's body: (entity, his client, death animation) -> body.
    corpse_bodies: HashMap<(u16, u32, i32), String>,
    last_now: i32,
    /// When each ground item was first seen (its tip onto its side).
    item_seen: HashMap<i32, i32>,
    logged: Vec<String>,
    /// Each corpse gone limp: its ragdoll (none when its skeleton lacks a
    /// joint the ragdoll names) and the start time of the last throw it
    /// took (each new `launchragdoll` throws it again).
    limp: HashMap<u16, (Option<Ragdoll>, Option<i32>)>,
    /// The limp bodies stepped this frame, for the Gravity Gun.
    beam_seen: HashMap<u32, super::ragdoll::BodySeen>,
    /// Each corpse's ragdoll joints last frame and when, while its death
    /// animation still plays (the speed it goes limp at).
    joints_before: HashMap<u16, (Vec<Vec3>, i32)>,
    /// Each body's composition, kept while what he wears and holds stays.
    built: HashMap<u16, Built>,
    spent: BodySpent,
    /// Which body each corpse entity holds, and how long it has been out of
    /// the snapshot (so its animation and ragdoll can be kept through a gap).
    corpse_meta: HashMap<u16, CorpseMeta>,
    /// The live world's bodies, set aside while a killcam replays the past.
    live: Option<Box<LiveBodies>>,
    /// BO2's nextSlideFX for each player sliding: when his next slide dust
    /// is due (kept only while he slides).
    slide_fx: HashMap<u16, i32>,
    /// The script's slide animations (found once per script, by its address).
    slide_anims: (usize, Vec<u16>),
}

/// BO2 lets a ragdoll live 9000 ms (ragdoll_max_life); a corpse that leaves
/// the snapshot is kept that long, so one that comes back is the same body
/// carrying on and not a new one standing up again.
const CORPSE_GAP_KEEP_MS: i32 = 9000;

/// A step back in the clock smaller than this is a hiccup, not a killcam's
/// replay starting.
const CLOCK_BACK_REPLAY_MS: i32 = 250;

/// Whether bodies are logged (BO2MP_BODYLOG, as the server side does).
fn body_log() -> bool {
    static BODYLOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    env_flag(&BODYLOG, "BO2MP_BODYLOG")
}

/// Whether every drawn frame of another player's body is logged
/// (BO2MP_SHAKELOG): grep the log for "shakelog:".
fn shake_log() -> bool {
    static SHAKELOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    env_flag(&SHAKELOG, "BO2MP_SHAKELOG")
}

/// Whether the environment variable is set, looked up once and kept in `cell`.
fn env_flag(cell: &'static std::sync::OnceLock<bool>, var: &str) -> bool {
    *cell.get_or_init(|| std::env::var_os(var).is_some())
}

/// Which body a corpse entity holds. BO2 hands each new clone its entity with
/// the "new body" flag flipped and a fresh use count; this is how that shows
/// here: the flag and the client it belongs to, as last seen.
#[derive(Clone, Copy, Debug)]
struct CorpseMeta {
    owner_client: u32,
    new_body_bit: u32,
    /// How long it has been out of the snapshot (ms of drawn frames).
    away: i32,
    /// When this body first showed (client clock): the newest of a player's
    /// corpses is his latest death.
    since: i32,
}

/// What the live world keeps for its bodies, while a killcam's replay uses
/// the same maps for its own.
#[derive(Default)]
struct LiveBodies {
    tracks: HashMap<u16, BodyTrack>,
    corpse_bodies: HashMap<(u16, u32, i32), String>,
    limp: HashMap<u16, (Option<Ragdoll>, Option<i32>)>,
    joints_before: HashMap<u16, (Vec<Vec3>, i32)>,
    corpse_meta: HashMap<u16, CorpseMeta>,
}

/// A corpse entity that now holds a different body: everything kept for the
/// old one goes, so the new one starts its death animation and ragdoll over.
fn forget_corpse(l: &mut BodyLocals, number: u16) {
    l.tracks.remove(&number);
    l.limp.remove(&number);
    l.joints_before.remove(&number);
    l.built.remove(&number);
    l.corpse_bodies.retain(|(n, _, _), _| *n != number);
}

/// Corpses out of the snapshot this frame grow older; one gone longer than
/// BO2 keeps a ragdoll is forgotten (the retains that follow drop its state).
fn age_corpse_gaps(l: &mut BodyLocals, seen: &[u16], frame_ms: i32) {
    let mut gone: Vec<(u16, i32)> = vec![];
    for (n, m) in &mut l.corpse_meta {
        if seen.contains(n) {
            continue;
        }
        m.away += frame_ms;
        if m.away > CORPSE_GAP_KEEP_MS {
            gone.push((*n, m.away));
        }
    }
    for (n, away) in gone {
        l.corpse_meta.remove(&n);
        if body_log() {
            info!(World, "bo2mp corpse {n}: out of the snapshot {away} ms, its animation and ragdoll are dropped");
        }
    }
}

/// What a body's composition is built from: what he wears, what is in his
/// hand (a gun, or the grenade or equipment he is using), whether he is a
/// corpse, and the catalogs (a new map has its own).
#[derive(Clone, PartialEq, Eq)]
struct BuiltKey {
    body: String,
    weap: u32,
    offhand: bool,
    corpse: bool,
    catalogs: (usize, u64, u64),
}

/// A body's models on their tags and its bones, built when what he wears
/// or holds changes rather than every frame, and each animation's tracks
/// on those bones, found once.
struct Built {
    key: BuiltKey,
    pieces: Vec<Part>,
    obj: xmodel_runtime::PosedSkeleton,
    upper: dobj_t6::PartSet,
    all: dobj_t6::PartSet,
    right_gun: Option<usize>,
    flash: Option<u16>,
    brass: Option<u16>,
    head: Option<usize>,
    mainroot: Option<usize>,
    /// His ankles (j_ankle_le, j_ankle_ri): the slide's dust comes from
    /// between them.
    ankles: Option<(usize, usize)>,
    tracks: HashMap<usize, (Arc<xmodel_runtime::SkeletalClip>, Arc<[Option<usize>]>)>,
    /// The bones it was last skinned in and those vertices: a body whose
    /// bones have not moved since (a corpse lying still, a settled ragdoll)
    /// hands on the same vertices, so nothing is skinned or uploaded again.
    skinned_bones: Vec<Mat4>,
    skinned: Vec<Arc<PosedVerts>>,
    /// A corpse at the end of its death animation: where it lay (its
    /// origin and yaw) and where that rested it on the ground.
    rested: Option<([f32; 3], f32, Mat4)>,
}

impl Built {
    /// An animation's tracks on these bones (found once per clip; the clip
    /// is kept with them, so its address names it).
    fn tracks_for(&mut self, reel: &Arc<xmodel_runtime::SkeletalClip>) -> Arc<[Option<usize>]> {
        let at = Arc::as_ptr(reel) as usize;
        if let Some((_at, tracks)) = self.tracks.get(&at) {
            return tracks.clone();
        }
        let tracks: Arc<[Option<usize>]> = self.obj.tracks_for(reel).into();
        self.tracks.insert(at, (reel.clone(), tracks.clone()));
        tracks
    }
}

/// Where the bodies' time went (ms) since the last log, logged every 10 s
/// so a slow frame's cause has a name: compositions built, animations
/// posed, corpses laid on the ground, vertices skinned, guns on the ground.
#[derive(Default)]
struct BodySpent {
    since: Option<std::time::Instant>,
    n_frames: u32,
    bodies: u32,
    corpses: u32,
    builds: u32,
    /// Bodies that kept last frame's vertices (they had not moved).
    still: u32,
    /// Bodies away from the view (posed, not drawn or skinned).
    unseen: u32,
    build: f64,
    pose: f64,
    ground: f64,
    skin: f64,
    all: f64,
    items: f64,
}

/// The clock every timing in this file reads.
fn clock_now() -> std::time::Instant {
    std::time::Instant::now()
}

fn ms_since(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn log_spent(spent: &mut BodySpent) {
    let now = clock_now();
    let since = *spent.since.get_or_insert(now);
    if now.duration_since(since).as_secs_f64() < 10.0 {
        return;
    }
    if spent.n_frames > 0 && spent.bodies > 0 {
        let f = f64::from(spent.n_frames);
        diag::info!(
            World,
            "bo2mp bodies (10 s): {:.1} a frame ({:.1} corpses, {:.1} still, {:.1} off the view), {} rebuilt; ms a frame: {:.2} in all (build {:.2}, pose {:.2}, ground {:.2}, skin {:.2}), guns on the ground {:.2}",
            f64::from(spent.bodies) / f,
            f64::from(spent.corpses) / f,
            f64::from(spent.still) / f,
            f64::from(spent.unseen) / f,
            spent.builds,
            spent.all / f,
            spent.build / f,
            spent.pose / f,
            spent.ground / f,
            spent.skin / f,
            spent.items / f
        );
    }
    *spent = BodySpent {
        since: Some(now),
        ..Default::default()
    };
}

/// A weapon's shape on the ground: its world model and attachments placed
/// on it (bind pose), and how far across it is.
fn item_shape(
    registry: &asset_game::WeaponLedger,
    catalog: &asset_model::LevelWeaponRoster,
    weapon_ix: u32,
) -> Option<Arc<ItemShape>> {
    let parts = gun_parts(registry, catalog, weapon_ix, None, 0);
    if parts.is_empty() {
        return None;
    }
    let dobj = build_dobj(&parts)?;
    let ident = Mat4::IDENTITY;
        let world = dobj.pose(&[], &dobj.full_part_set(), ident);
    let posed = skin_all(&dobj, &parts, &world);
    let ys = posed.iter().flat_map(|v| v.positions.iter().map(|p| p[1]));
    let (low_y, high_y) = ys.fold((0.0f32, 0.0f32), |(lo, hi), y| (lo.min(y), hi.max(y)));
    Some(Arc::new(ItemShape {
        models: parts
            .into_iter()
            .zip(posed)
            .map(|(p, v)| (p.bone_rig, p.mat_list, v))
            .collect::<_>(),
        low_y,
        high_y,
    }))
}

/// Every weapon's ground shape, built once (at map load, see
/// [`prepare_t6_bodies`]).
#[derive(Resource, Default)]
struct T6ItemShapes(HashMap<u32, Option<Arc<ItemShape>>>);

/// At map load, before the match: every animation BO2's player script names
/// decoded (in parallel), and every weapon's ground shape built, so the
/// first body or dropped gun of a kind costs nothing extra mid-match.
fn prepare_t6_bodies(
    script: Option<Res<T6PlayerAnimsRes>>,
    clips: Option<Res<assets::StagedXAnims>>,
    arms: Option<Res<assets::StagedWeapons>>,
    world_arms: Option<Res<assets::ReadiedWorldWeapons>>,
    mut shapes: ResMut<T6ItemShapes>,
    mut done: Local<Option<(usize, u64)>>,
) {
    let (Some(script), Some(clips), Some(arms), Some(world_arms)) = (script, clips, arms, world_arms) else {
        return;
    };
    let Some(script) = script.0.as_ref() else {
        return;
    };
    // Once per loaded map (its catalogs), when the weapons are bound to
    // its world models.
    if arms.0.world_catalog_identity() != world_arms.0.identity() {
        return;
    }
    let owner = (std::sync::Arc::as_ptr(&arms.0) as usize, world_arms.0.identity());
    if done.replace(owner) == Some(owner) {
        return;
    }
    let began = clock_now();
    let names: Vec<&str> = script.anims.iter().map(String::as_str).collect();
    let per_task = names.len().div_ceil(8).max(1);
    let decoded: usize = bevy::tasks::ComputeTaskPool::get()
        .scope(|sc| {
            for part in names.chunks(per_task) {
                let catalog = &clips.0;
                sc.spawn(async move {
                    part.iter()
                        .filter(|name| catalog.clip(asset_core::AssetCategoryScope::T6, name).is_some())
                        .count()
                });
            }
        })
        .into_iter()
        .sum();
    let clips_ms = ms_since(began);
    shapes.0.clear();
    if !world_arms.0.is_empty() {
        let top = arms.0.len() as u32;
        shapes
            .0
            .extend((1..=top).map(|w| (w, item_shape(&arms.0, &world_arms.0, w))));
    }
    let built = shapes.0.values().filter(|s| s.is_some()).count();
    diag::info!(
        World,
        "bo2mp bodies prepared at load: {decoded} of {} player animations decoded in {clips_ms:.0} ms, {built} weapon ground shapes; {:.0} ms in all",
        names.len(),
        ms_since(began)
    );
}

struct ItemShape {
    models: Vec<(Arc<asset_model::MeshRig>, Vec<Option<u32>>, Arc<PosedVerts>)>,
    /// The gun's extent across (its own y): how far it lies above its
    /// origin when it rests on either side.
    low_y: f32,
    high_y: f32,
}

fn note_once(locals: &mut BodyLocals, line: String) {
    if !locals.logged.contains(&line) {
        diag::info!(World, "{line}");
        locals.logged.push(line);
    }
}

/// A model with surfaces that have no material (they draw nothing), once.
fn note_unbound(locals: &mut BodyLocals, part: &Part) {
    let missing = part.mat_list.iter().filter(|m| m.is_none()).count();
    if missing > 0 {
        note_once(
            locals,
            format!(
                "bo2mp body: {} has {missing} of {} surfaces with no material",
                part.bone_rig.name,
                part.mat_list.len()
            ),
        );
    }
}

/// One model of a composition: its skeleton and per-surface materials.
struct Part {
    bone_rig: Arc<asset_model::MeshRig>,
    mat_list: Vec<Option<u32>>,
    attach: Option<xmodel_runtime::MountedModel>,
    /// Its root bound to the parent tag at this (rotation, offset) instead
    /// of on it: a stowed gun at its weapon's stowed placement.
    root_bind: Option<(Quat, Vec3)>,
    /// bo2mp: its bones a gun's hide tags hide (the tags and all under
    /// them: the iron sights under a sight), not drawn.
    hidden: Vec<bool>,
}

/// bo2mp: which of a gun model's bones its weapon's hide tags hide (the
/// tag and everything under it), as in first person.
fn hidden_bones(bone_rig: &asset_model::MeshRig, tags: &[String]) -> Vec<bool> {
    if tags.is_empty() {
        return Vec::new();
    }
    (0..bone_rig.joint_labels.len())
        .map(|b| asset_game::joint_under_concealed_parent(&bone_rig.joint_labels, |i| bone_rig.hanging_from(i), b, tags))
        .collect::<_>()
}

/// A bone's skin matrix, its vertices collapsed onto the bone (nothing
/// drawn) when hidden.
fn shown(part: &Part, b: usize, m: Mat4) -> Mat4 {
    if part.hidden.get(b).copied().unwrap_or(false) {
        Mat4::from_cols(bevy::math::Vec4::ZERO, bevy::math::Vec4::ZERO, bevy::math::Vec4::ZERO, m.w_axis)
    } else {
        m
    }
}

/// Per surface, its material in the catalog (none = not drawn).
macro_rules! materials_of {
    ($edges:expr) => {
        $edges
            .iter()
            .map(|edge| edge.extent_position().map(|i| i as u32))
            .collect::<Vec<Option<u32>>>()
    };
}

/// A weapon's world model and its attachments (on the gun's tags), the gun
/// itself attached to `parent` at `tag` (or the root of the composition).
fn gun_parts(
    registry: &asset_game::WeaponLedger,
    catalog: &asset_model::LevelWeaponRoster,
    weapon_idx: u32,
    owner: Option<(usize, &str)>,
    base_index: usize,
) -> Vec<Part> {
    let Some(entry) = registry.world_model_entry(weapon_idx, catalog) else {
        return Vec::new();
    };
    if entry.bone_rig.pose.is_none() {
        return Vec::new();
    }
    let mut out = vec![Part {
        bone_rig: entry.bone_rig.clone(),
        mat_list: materials_of!(entry.surface_links),
        attach: owner.map(|(p, tag)| xmodel_runtime::MountedModel {
            host_model: p,
            mount_tag: tag.to_owned(),
        }),
        root_bind: None,
        hidden: hidden_bones(&entry.bone_rig, &asset_game::effective_hide_tags(registry, weapon_idx)),
    }];
    for attachment in crate::motion::other_body::world_attachments(registry, catalog, weapon_idx) {
        out.push(Part {
            bone_rig: attachment.entry.bone_rig.clone(),
            mat_list: materials_of!(attachment.entry.surface_links),
            attach: Some(xmodel_runtime::MountedModel {
                host_model: base_index,
                mount_tag: attachment.tag.to_string(),
            }),
            root_bind: None,
            hidden: Vec::new(),
        });
    }
    out
}

/// His body's composition (see [`Built`]): the body model, the models on
/// its tags (a stowed gun at its weapon's stowed placement), and what is in
/// his hands.
fn build_body(
    locals: &mut BodyLocals,
    key: BuiltKey,
    fpv: &assets::ReadiedFpvMeshes,
    registry: Option<&asset_game::WeaponLedger>,
    catalog: Option<&asset_model::LevelWeaponRoster>,
    script: &T6PlayerAnims,
) -> Option<Built> {
    let t_start = clock_now();
    let mut names = key.body.split(';');
    let model = names.next().unwrap_or("");
    let Some(entry) = fpv.0.by_label(asset_core::AssetCategoryScope::T6, model) else {
        note_once(locals, format!("bo2mp body: {model} is not in the model catalog"));
        return None;
    };
    let mut parts = vec![Part {
        bone_rig: entry.bone_rig.clone(),
        mat_list: materials_of!(entry.surface_links),
        attach: None,
        root_bind: None,
        hidden: Vec::new(),
    }];
    let mut left_hand_model = false;
    for extra in names {
        let Some((m, tag)) = extra.split_once('@') else { continue };
        left_hand_model |= tag == "tag_weapon_left";
        // A weapon on a tag (the stowed gun on his back): its world
        // model and attachments.
        if let Some(index) = m.strip_prefix('#') {
            if let (Ok(w), Some(registry), Some(catalog)) = (index.parse::<u32>(), registry, catalog) {
                let base = parts.len();
                parts.extend(gun_parts(registry, catalog, w, Some((0, tag)), base));
                // BO2 hangs it at the weapon's own stowed offset and
                // angle (a launcher slung across his back), not
                // square on the tag.
                if let (Some(gun_in), Some(f)) = (parts.get_mut(base), registry.summary_for(w)) {
                    let q = asset_model::angles_quat(f.stowed_rotation);
                    gun_in.root_bind = Some((Quat::from_array(q).normalize(), Vec3::from_array(f.stowed_offset)));
                }
            }
            continue;
        }
        // A model from the character catalog, or a weapon's world model
        // (BO2's stow_on_hip attaches one at tag_stowed_hip_rear).
        let found = fpv
            .0
            .by_label(asset_core::AssetCategoryScope::T6, m)
            .map(|e| (e.bone_rig.clone(), materials_of!(e.surface_links)))
            .or_else(|| {
                let e = catalog?.by_label(asset_core::AssetCategoryScope::T6, m)?;
                Some((e.bone_rig.clone(), materials_of!(e.surface_links)))
            });
        let Some((bone_rig, materials)) = found else {
            note_once(locals, format!("bo2mp body: {m} is not in the model catalog"));
            continue;
        };
        let tag = if tag.is_empty() {
            parts[0]
                .bone_rig
                .pose
                .as_ref()
                .zip(bone_rig.pose.as_ref())
                .map(|(p, c)| xmodel_runtime::empty_tag_attach(p, c))
                .unwrap_or_default()
        } else {
            tag.to_owned()
        };
        parts.push(Part {
            bone_rig,
            mat_list: materials,
            attach: Some(xmodel_runtime::MountedModel { host_model: 0, mount_tag: tag }),
            root_bind: None,
            hidden: Vec::new(),
        });
    }
    // The gun in his right hand (a corpse dropped his), and a dual-wield
    // pair's left-hand gun on tag_weapon_left. While a torso animation
    // handles a grenade or a piece of equipment (the script's grenadeAnim:
    // a throw, a claymore planted), that item is in his hand instead of the
    // gun.
    let t_models = ms_since(t_start);
    let weapon = key.weap;
    let mut right_gun = None;
    if !key.corpse
        && weapon != 0
        && let (Some(registry), Some(catalog)) = (registry, catalog)
    {
        let base = parts.len();
        // BO2 holds the riot shield on the left arm (its riotshield
        // animations brace it there); every other weapon in the right hand.
        let shield = registry
            .summary_for(weapon)
            .is_some_and(|f| script.anim_type_name(f.player_motion_kind) == "riotshield");
        let tag = if shield {
            "tag_weapon_left"
        } else {
            parts[0]
                .bone_rig
                .pose
                .as_ref()
                .and_then(|p| xmodel_runtime::tp_weapon_attach_tag(&p.joint_labels))
                .unwrap_or("tag_weapon_right")
        };
        // The riot shield in his hand is the carried model the script
        // attaches there (refreshshieldattachment); the weapon's own world
        // model is the one that lies on the ground.
        let gun_in = if shield && left_hand_model {
            Vec::new()
        } else {
            gun_parts(registry, catalog, weapon, Some((0, tag)), base)
        };
        if !gun_in.is_empty() {
            right_gun = Some(base);
        }
        parts.extend(gun_in);
        let left = registry.dual_wield_weapon_of(weapon);
        if !key.offhand && left != 0 && left != weapon {
            let base = parts.len();
            parts.extend(gun_parts(registry, catalog, left, Some((0, "tag_weapon_left")), base));
        }
    }
    let t_guns = ms_since(t_start);
    let Some(dobj) = build_dobj(&parts) else {
        note_once(locals, format!("bo2mp body: {model} has no skeleton to pose"));
        return None;
    };
    for part in &parts {
        note_unbound(locals, part);
    }
    let gun_tag = |name: &str| -> Option<u16> {
        let gun_in = right_gun?;
        let slot = dobj.bones.iter().position(|b| b.model == gun_in && b.name == name)?;
        u16::try_from(slot).ok()
    };
    let (flash, brass) = (gun_tag("tag_flash"), gun_tag("tag_brass"));
    let t_all = ms_since(t_start);
    if t_all > 20.0 {
        warn!(World, "bo2mp slow: build_body {model} weapon {weapon}: body+tag models {t_models:.1} ms, guns {:.1} ms, skeleton+sets {:.1} ms", t_guns - t_models, t_all - t_guns);
    }
    Some(Built {
        upper: upper_body(&dobj),
        all: dobj.full_part_set(),
        head: dobj.find("j_head"),
        mainroot: dobj.find("j_mainroot"),
        ankles: dobj.find("j_ankle_le").zip(dobj.find("j_ankle_ri")),
        flash,
        brass,
        right_gun,
        pieces: parts,
        obj: dobj,
        tracks: HashMap::new(),
        skinned_bones: Vec::new(),
        skinned: Vec::new(),
        rested: None,
        key,
    })
}

fn build_dobj(pieces: &[Part]) -> Option<xmodel_runtime::PosedSkeleton> {
    let srcs: Vec<(&xmodel_runtime::PoseDonorModel, Option<xmodel_runtime::MountedModel>)> = pieces
        .iter()
        .map(|p| Some((p.bone_rig.pose.as_ref()?, p.attach.clone())))
        .collect::<Option<Vec<_>>>()?;
    let binds: Vec<Option<(Quat, Vec3)>> = pieces.iter().map(|p| p.root_bind).collect();
    xmodel_runtime::PosedSkeleton::build_with_root_binds(&srcs, &binds).ok()
}

/// Bones where they were when last skinned, to a thousandth of an inch (a
/// settled ragdoll's creep adds up against the skinned pose, so it is
/// skinned again once it has moved that far).
fn same_pose(skinned: &[Mat4], now: &[Mat4]) -> bool {
    !skinned.is_empty() && skinned.len() == now.len() && skinned.iter().zip(now).all(|(a, b)| a.abs_diff_eq(*b, 1e-3))
}

/// A body this far outside the view (degrees) is still drawn: the camera
/// may turn that far before the frame is drawn, and its place here is the
/// one it had when the frame began.
const VIEW_MARGIN_DEG: f32 = 20.0;

/// How far past a body its shadow may reach (units): a body just outside
/// the view still draws, for the shadow it casts into it.
const SHADOW_REACH: f32 = 128.0;

/// The cone the camera sees, VIEW_MARGIN_DEG wider all round: a body
/// wholly outside it is posed (his gun's flash, his sounds and his ragdoll
/// go on) but not drawn, so not skinned.
struct ViewCone {
    eye: Vec3,
    forward: Vec3,
    cos: f32,
    sin: f32,
}

impl ViewCone {
    fn of(cameras: &Query<(&Projection, &GlobalTransform, &Camera), With<FirstPersonLens>>) -> Option<Self> {
        if std::env::var_os("BO2MP_NO_FRUSTUM").is_some() {
            return None;
        }
        let (proj, xf, camera) = cameras.single().ok()?;
        let Projection::Perspective(p) = proj else {
            return None;
        };
        let aspect = if p.aspect_ratio > 1e-3 {
            p.aspect_ratio
        } else {
            camera
                .logical_viewport_size()
                .filter(|v| v.y > 0.0)
                .map_or(16.0 / 9.0, |v| v.x / v.y)
        };
        // Its corners' angle off its middle, and the margin.
        let half = ((p.fov * 0.5).tan() * (1.0 + aspect * aspect).sqrt()).atan() + VIEW_MARGIN_DEG.to_radians();
        let (_, turn_matrix, eye) = xf.to_scale_rotation_translation();
        Some(Self {
            eye,
            forward: turn_matrix * Vec3::NEG_Z,
            cos: half.cos(),
            sin: half.sin(),
        })
    }

    /// Whether any of a sphere is inside it.
    fn sees(&self, centre: Vec3, r: f32) -> bool {
        let d = centre - self.eye;
        let along = d.dot(self.forward);
        let across = (d.length_squared() - along * along).max(0.0).sqrt();
        across * self.cos - along * self.sin <= r
    }
}

/// A body skinned after the loop: its bones, and where its models start
/// in this frame's items.
struct SkinJob {
    body_no: u16,
    first_item: usize,
    bones: Vec<Mat4>,
}

fn skin_one(w: Option<(&xmodel_runtime::PosedSkeleton, &[Part], &[Mat4])>) -> Option<Vec<Arc<PosedVerts>>> {
    w.map(|(dobj, parts, bones)| bones_all(dobj, parts, bones))
}

/// Skins the bodies, several at once across the cores (BO2 skins on its
/// worker threads too).
fn skin_bodies(work: &[Option<(&xmodel_runtime::PosedSkeleton, &[Part], &[Mat4])>]) -> Vec<Option<Vec<Arc<PosedVerts>>>> {
    match bevy::tasks::ComputeTaskPool::try_get() {
        Some(pool) if work.len() > 1 => pool.scope(|s| {
            for w in work {
                let w = *w;
                s.spawn(async move { skin_one(w) });
            }
        }),
        _ => work.iter().map(|w| skin_one(*w)).collect(),
    }
}

fn skin_all(dobj: &xmodel_runtime::PosedSkeleton, pieces: &[Part], world: &[Mat4]) -> Vec<Arc<PosedVerts>> {
    let skin = dobj.skin_matrices(world);
    pieces
        .iter()
        .enumerate()
        .map(|(mi, part)| {
            let base = dobj.models.get(mi).map_or(0, |m| m.first_joint);
            Arc::new(skin_verts(&part.bone_rig, |b| {
                shown(part, b, skin.get(base + b).copied().unwrap_or(Mat4::IDENTITY))
            }))
        })
        .collect::<_>()
}

/// bo2mp: each part's skin joint_transforms, for the graphics card to skin it
/// with (BO2MP_CPU_SKIN=1: skinned here, as before). Skinning ~150k
/// vertices a frame here and copying them cost a drawn window ~5 ms of
/// game thread and ~8 ms of render thread.
fn bones_all(dobj: &xmodel_runtime::PosedSkeleton, pieces: &[Part], world: &[Mat4]) -> Vec<Arc<PosedVerts>> {
    static CPU_SKIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if env_flag(&CPU_SKIN, "BO2MP_CPU_SKIN") {
        return skin_all(dobj, pieces, world);
    }
    let skin = dobj.skin_matrices(world);
    pieces
        .iter()
        .enumerate()
        .map(|(mi, part)| {
            let (base, count) = dobj
                .models
                .get(mi)
                .map_or((0, part.bone_rig.bones.len()), |m| (m.first_joint, m.joint_total));
            Arc::new(PosedVerts::from_bones(count, |b| {
                shown(part, b, skin.get(base + b).copied().unwrap_or(Mat4::IDENTITY))
            }))
        })
        .collect::<_>()
}

/// The bones the torso's animation drives: `j_spinelower` and everything
/// below it in the hierarchy (spine, arms, head, the gun on the hand).
/// Parents come before their children in a DObj's bone list.
fn upper_body(dobj: &xmodel_runtime::PosedSkeleton) -> dobj_t6::PartSet {
    let Some(root) = dobj.find("j_spinelower") else {
        return dobj.full_part_set();
    };
    let mut under_root = vec![false; dobj.bones.len()];
    for (i, bone) in dobj.bones.iter().enumerate() {
        under_root[i] = i == root || matches!(bone.up, Some(p) if p < i && under_root[p]);
    }
    let mut bits = dobj_t6::PartSet::default();
    (0..under_root.len())
        .filter(|&i| under_root[i])
        .for_each(|i| bits.set(i));
    bits
}

/// The surface under a point (BO2's surface flags), by the clip.
fn surface_under(solids: Option<&asset_world::CollisionClipData>, at: [f32; 3]) -> u32 {
    let Some(solids) = solids else { return 0 };
    let hit = solids.sweep_box([at[0], at[1], at[2] + 8.0], [at[0], at[1], at[2] - 64.0], [0.0; 3], [0.0; 3], super::ragdoll::MASK_PHYS_WORLD);
    if hit.began_inside || hit.fraction >= 1.0 { 0 } else { hit.face_bits }
}

/// BO2 plays the slide's dust again 5 ms after it last did: every frame.
const SLIDE_FX_MS: i32 = 5;

/// BO2's rumble for a player sliding.
const SLIDE_RUMBLE: &str = "slide_rumble";

/// BO2's effect for a player sliding (its code's fxPlayerSliding).
pub const T6_SLIDE_DUST_FX: &str = "system_elements/fx_snow_sm_em";

/// The slide's dust this frame: between his ankles, turned as his right
/// ankle is. The effect system plays it.
#[derive(Message)]
#[derive(Clone, Copy, Debug)]
pub struct T6SlideDust {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlideRumble {
    Start,
    Stop,
}

/// BO2's slide for one player this frame (CG_Player): not sliding, his
/// slide_rumble stops (it does nothing when none plays, so only as he
/// stops); sliding, it starts as he begins, and his dust plays once its
/// time has passed. Returns the rumble change and whether the dust plays.
fn slide_step(next_fx: &mut HashMap<u16, i32>, sliding: bool, number_in: u16, now: i32) -> (Option<SlideRumble>, bool) {
    if !sliding {
        return (next_fx.remove(&number_in).map(|_| SlideRumble::Stop), false);
    }
    let next = next_fx.entry(number_in).or_insert(0);
    let rumble = (*next == 0).then_some(SlideRumble::Start);
    let dust = *next < now;
    if dust {
        *next = now + SLIDE_FX_MS;
    }
    (rumble, dust)
}

/// The sounds his body's animations call for this frame: entering a dive's
/// takeoff or landing animation (another player's), and every bodyfall
/// notetrack his legs' animation passed since the last frame. A dive's
/// landing also starts BO2's dtp_rumble on that player (felt only on the
/// viewer's own).
#[allow(clippy::too_many_arguments, reason = "kept as written")]
fn body_sounds(
    out: &mut MessageWriter<audio::AliasCommand>,
    rumbles: &mut MessageWriter<audio::CodeRumbleT6>,
    script: &T6PlayerAnims,
    animation_refs: &assets::StagedXAnims,
    solids: Option<&asset_world::CollisionClipData>,
    track: &mut BodyTrack,
    origin_in: [f32; 3],
    number: u32,
    corpse: bool,
    other: bool,
) {
    use crate::occupancy::t6_sounds::{dive_surface_name, play_t6, surface_name};
    let cur = track.legs.cur;
    let last = track.heard.replace(cur);
    let Some(name) = script.anim_name(cur.value) else { return };
    let same = last.is_some_and(|l| t6_anim_part(l.value) == t6_anim_part(cur.value));
    if !same && !corpse && last.is_some() && name.contains("dive_prone") {
        // Another player's in the world; his own (`plr`) without a place,
        // as his own landing sounds play.
        let (who, at) = if other { ("npc", Some(origin_in)) } else { ("plr", None) };
        if name.ends_with("_land") {
            let surface = dive_surface_name(surface_under(solids, origin_in));
            play_t6(out, format!("fly_dtp_land_{who}_{surface}"), Some(format!("fly_dtp_land_{who}_default")), at, Some(number));
            if let Ok(number) = u16::try_from(number) {
                rumbles.write(audio::CodeRumbleT6 { name: "dtp_rumble", number });
            }
        } else {
            play_t6(out, format!("fly_dtp_launch_{who}"), None, at, Some(number));
        }
    }
    let Some(anim) = animation_refs.0.clip(asset_core::AssetCategoryScope::T6, name) else { return };
    if anim.notifies.is_empty() {
        return;
    }
    let duration = anim.duration().max(1e-3);
    // Where the animation was last frame and is now (fractions); a new
    // animation from its start, one first seen mid-way from where it is.
    let from = match last {
        Some(l) if same => l.phase / duration,
        Some(_) => 0.0,
        None => cur.phase / duration,
    };
    let to = cur.phase / duration;
    if to <= from && !(!same && last.is_some()) {
        return;
    }
    for note in &anim.notifies {
        let crossed = if !same && last.is_some() {
            note.time >= 0.0 && note.time <= to
        } else {
            note.time > from && note.time <= to
        };
        if !crossed {
            continue;
        }
        let size = match note.name.as_str() {
            "bodyfall large" => "large",
            "bodyfall small" => "small",
            _ => continue,
        };
        // BO2 names a body fall for these surfaces (in multiplayer each is
        // its gear fall, the alias's secondary); others fall as the gear.
        const BODYFALL_SURFACES: [&str; 15] = [
            "asphalt", "bark", "carpet", "ceramic", "cloth", "concrete", "dirt", "grass", "gravel", "mud", "plastic",
            "rock", "rubber", "sand", "wood",
        ];
        let surface = surface_name(surface_under(solids, origin_in));
        let alias = if BODYFALL_SURFACES.contains(&surface) {
            format!("fly_bodyfall_{size}_{surface}")
        } else {
            "fly_gear_fall_npc".to_owned()
        };
        play_t6(out, alias, None, Some(origin_in), Some(number));
    }
}

/// A corpse's fit to the ground at one placement: the body tilted onto
/// the slope under it, the height it is moved by, and how far each point
/// (hips, head, feet) is left above its own ground.
struct GroundFit {
    turned: Mat4,
    shift: f32,
    pivot: Vec3,
    /// (its world point, the ground under it, how far it hangs over that)
    rows: Vec<(Vec3, f32, f32)>,
    /// The height the body would need to move (before the 24-unit cap).
    need: f32,
}

impl GroundFit {
    fn worst_hang(&self) -> f32 {
        self.rows.iter().map(|r| r.2).fold(0.0, f32::max)
    }

    fn placed(&self) -> Mat4 {
        Mat4::from_translation(Vec3::new(0.0, 0.0, self.shift)) * self.turned
    }
}

fn fit_to_ground(
    dobj: &xmodel_runtime::PosedSkeleton,
    world_mats: &[Mat4],
    placed: Mat4,
    org: [f32; 3],
    clip: &asset_world::CollisionClipData,
    lying: f32,
) -> Option<GroundFit> {
    const SOLID: u32 = super::ragdoll::MASK_PHYS_WORLD;
    let bone = |name: &str| dobj.find(name).and_then(|i| world_mats.get(i)).map(|m| m.w_axis.truncate());
    let (pelvis, head) = (bone("pelvis")?, bone("j_head")?);
    let mut keys = vec![pelvis, head];
    keys.extend(["j_ankle_le", "j_ankle_ri"].iter().filter_map(|n| bone(n)));
    // (model-space point, its world point, the ground under it)
    let mut rows = Vec::<(Vec3, Vec3, f32)>::new();
    for local in keys {
        let at = placed.transform_point3(local);
        let top = [at.x, at.y, org[2] + 48.0];
        let bottom = [at.x, at.y, org[2] - 64.0];
        let hit = clip.sweep_box(top, bottom, [0.0; 3], [0.0; 3], SOLID);
        if hit.began_inside || hit.fraction >= 1.0 {
            continue;
        }
        rows.push((local, at, hit.finish_point[2]));
    }
    rows.first()?;
    let pivot = placed.transform_point3(pelvis);
    // The body's length on the ground: hips to head, flat.
    let along = {
        let d = placed.transform_point3(head) - pivot;
        Vec3::new(d.x, d.y, 0.0).normalize_or_zero()
    };
    let mut turned = placed;
    if along != Vec3::ZERO && rows.len() >= 2 {
        // Least squares: ground height against distance along the body.
        let n = rows.len() as f32;
        let ss: Vec<(f32, f32)> = rows.iter().map(|(_, at, g)| ((*at - pivot).dot(along), *g)).collect();
        let (ms, mg) = ss.iter().fold((0.0, 0.0), |(a, b), (s, g)| (a + s / n, b + g / n));
        let (num, den) = ss
            .iter()
            .fold((0.0, 0.0), |(a, b), (s, g)| (a + (s - ms) * (g - mg), b + (s - ms) * (s - ms)));
        if den > 16.0 {
            let slope = (num / den).clamp(-0.7, 0.7);
            let pitch = slope.atan() * lying;
            let axis = Vec3::Z.cross(along).normalize_or_zero();
            if axis != Vec3::ZERO {
                // Raise the head end when the ground rises toward it.
                let tilt = Mat4::from_translation(pivot)
                    * Mat4::from_axis_angle(axis, -pitch)
                    * Mat4::from_translation(-pivot);
                turned = tilt * placed;
            }
        }
    }
    // Each point's height over its ground against its height in the pose.
    let needs: Vec<f32> = rows
        .iter()
        .map(|(local, _, g)| (g + local.z) - turned.transform_point3(*local).z)
        .collect::<_>();
    let need = needs.iter().copied().fold(f32::MIN, f32::max);
    let shift = need.clamp(-24.0, 24.0);
    Some(GroundFit {
        turned,
        shift,
        pivot,
        rows: rows.iter().zip(&needs).map(|((_, at, g), n)| (*at, *g, shift - n)).collect(),
        need,
    })
}

/// Ground contact for a corpse: the ground below its head, hips and feet
/// (straight down, solid only), a slope fitted along its length (feet to
/// head), the body tilted onto it about the hips by `lying` (0 standing at
/// the start of the death animation .. 1 lying at its end), then raised or
/// lowered so the point that would sink most sits at its own height in the
/// pose (at most 24 units). A body the death animation carried out over a
/// ledge (part of it, or all of it, left hanging more than 24 units over
/// its ground) slides back toward the side that has ground first. No
/// physics.
fn rest_on_ground(
    dobj: &xmodel_runtime::PosedSkeleton,
    world: &[Mat4],
    placed: Mat4,
    at_origin: [f32; 3],
    clip: &asset_world::CollisionClipData,
    lying: f32,
) -> Mat4 {
    let Some(mut fit) = fit_to_ground(dobj, world, placed, at_origin, clip, lying) else {
        return placed;
    };
    let mut slid = Vec3::ZERO;
    if fit.worst_hang() > 24.0 {
        // The way back to ground: toward the points that rest on it, from
        // the ones left hanging; with none resting (all of it floats),
        // toward the spot the engine's corpse lies on (on ground, by its
        // own trace).
        let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
        let mean = |pick: &dyn Fn(f32) -> bool| {
            let pts: Vec<Vec3> = fit.rows.iter().filter(|r| pick(r.2)).map(|r| flat(r.0)).collect();
            (!pts.is_empty()).then(|| pts.iter().copied().sum::<Vec3>() / pts.len() as f32)
        };
        let held = mean(&|hang| hang < 4.0);
        let hanging = mean(&|hang| hang > 24.0);
        let way = match (held, hanging) {
            (Some(held), Some(hanging)) => held - hanging,
            _ => flat(Vec3::from(at_origin)) - flat(fit.pivot),
        };
        if way.length() > 1.0 {
            // Half, all, then half again of the way: the first that leaves
            // nothing hanging, else the one that hangs least.
            for f in [0.5f32, 1.0, 1.5] {
                let step = way * f;
                let Some(next) = fit_to_ground(dobj, world, Mat4::from_translation(step) * placed, at_origin, clip, lying)
                else {
                    continue;
                };
                let better = next.worst_hang() < fit.worst_hang();
                if better {
                    fit = next;
                    slid = step;
                }
                if fit.worst_hang() <= 24.0 {
                    break;
                }
            }
        }
    }
    if std::env::var_os("BO2MP_BODYDRAWLOG").is_some() {
        static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
        let now = std::time::UNIX_EPOCH
            .elapsed()
            .map_or(0, |since| since.as_secs());
        if let Ok(mut last) = LAST.lock()
            && *last != now
        {
            *last = now;
            diag::info!(
                World,
                "bo2mp corpse rest: ground points {:?} (x y ground hang) lying {lying:.2} need {:.1} shift {:.1} slid {:.0}",
                fit.rows
                    .iter()
                    .map(|(at, g, hang)| [at.x.round(), at.y.round(), g.round(), hang.round()])
                    .collect::<Vec<_>>(),
                fit.need,
                fit.shift,
                slid.length()
            );
        }
    }
    fit.placed()
}

/// Turn the legs (the `pelvis` and everything under it but the torso, which
/// hangs from `torso_stabilizer`) about the pelvis by `angle` (radians,
/// about up), leaving the upper body where the pose put it.
fn twist_legs(dobj: &xmodel_runtime::PosedSkeleton, world: &mut [Mat4], angle_in: f32) {
    if angle_in.abs() < 1e-4 {
        return;
    }
    let (Some(pelvis), Some(stabilizer)) = (dobj.find("pelvis"), dobj.find("torso_stabilizer")) else {
        return;
    };
    let Some(pivot) = world.get(pelvis).map(|m| m.w_axis.truncate()) else {
        return;
    };
    let turn = Mat4::from_translation(pivot) * Mat4::from_rotation_z(angle_in) * Mat4::from_translation(-pivot);
    let mut legs = vec![false; dobj.bones.len().min(world.len())];
    for i in 0..legs.len() {
        legs[i] = i == pelvis
            || (i != stabilizer && dobj.bones[i].up.is_some_and(|p| p < i && legs[p]));
        if legs[i] {
            world[i] = turn * world[i];
        }
    }
}

/// The script's standing idle for these facts: what a part plays when the
/// server has named nothing yet.
fn idle_value(script: &T6PlayerAnims) -> i32 {
    script
        .pick_move("idle", &T6AnimFacts::default())
        .and_then(|c| c.first())
        .map_or(0, |c| anim_t6::t6_pack_anim(c.anim, false))
}

#[allow(clippy::too_many_arguments, reason = "kept as written")]
fn collect_t6_bodies(
    fpv: Option<Res<assets::ReadiedFpvMeshes>>,
    xanim_set: Option<Res<assets::StagedXAnims>>,
    arms: Option<Res<assets::StagedWeapons>>,
    world_arms: Option<Res<assets::ReadiedWorldWeapons>>,
    script: Option<Res<T6PlayerAnimsRes>>,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    view_subject: Option<Res<EyeSubject>>,
    cg_clock: Option<Res<FrameClock>>,
    (clip, ragdoll_def, cameras): (
        Option<Res<crate::occupancy::PropPhysicsShape>>,
        Option<Res<sim::ragdoll_defs::T6RagdollRes>>,
        Query<(&Projection, &GlobalTransform, &Camera), With<FirstPersonLens>>,
    ),
    mut shapes: ResMut<T6ItemShapes>,
    ents: Query<(Entity, &ClientEntityRecord, &ClientEntityPoseState)>,
    mut out: ResMut<T6BodyModels>,
    mut poses: ResMut<T6BodyPoses>,
    mut locals: Local<BodyLocals>,
    (mut sounds, mut rumbles, mut rumble_stops, mut dust): (
        MessageWriter<audio::AliasCommand>,
        MessageWriter<audio::CodeRumbleT6>,
        MessageWriter<audio::CodeRumbleStopT6>,
        MessageWriter<T6SlideDust>,
    ),
) {
    out.items.clear();
    poses.rows.clear();
    poses.heads.clear();
    poses.local_corpse_root = None;
    let mut corpse_root_time: Option<i32> = None;
    log_spent(&mut locals.spent);
    let bodies_started = clock_now();
    if std::env::var_os("BO2MP_NO_T6BODY").is_some() {
        return;
    }
    let (Some(fpv), Some(xanim_set), Some(script), Some(presented)) = (fpv, xanim_set, script, presented) else {
        return;
    };
    let Some(script) = script.0.clone() else {
        return;
    };
    let snapshot = match presented.snapshot() {
        Some(snapshot) => snapshot,
        None => return,
    };
    let script_at = Arc::as_ptr(&script) as usize;
    if locals.slide_anims.0 != script_at {
        let anims = script
            .commands()
            .filter(|(event, block, _)| !*event && *block == "slide")
            .map(|(_, _, c)| c.anim)
            .collect::<_>();
        locals.slide_anims = (script_at, anims);
    }
    let now = match cg_clock.as_deref() {
        Some(clock) if clock.started() => clock.time(),
        _ => sim::level_time_ms(snapshot.tick),
    };
    // The clock went back (a killcam's replay starts): every animation
    // starts again from what the replay shows. The live world's bodies are
    // set aside, not thrown away: they carry on when the replay is over.
    let rewound = now + CLOCK_BACK_REPLAY_MS < locals.last_now;
    if rewound {
        // A slide in the replay is the replay's own.
        locals.slide_fx.clear();
        let aside = LiveBodies {
            tracks: std::mem::take(&mut locals.tracks),
            corpse_bodies: std::mem::take(&mut locals.corpse_bodies),
            limp: std::mem::take(&mut locals.limp),
            joints_before: std::mem::take(&mut locals.joints_before),
            corpse_meta: std::mem::take(&mut locals.corpse_meta),
        };
        if locals.live.is_none() {
            if body_log() {
                diag::info!(
                    World,
                    "bo2mp corpse: the clock went back {} ms, {} bodies' animations and {} ragdolls set aside for the replay",
                    locals.last_now - now,
                    aside.tracks.len(),
                    aside.limp.len()
                );
            }
            locals.live = Some(Box::new(aside));
        }
    }
    let ragdoll_def = ragdoll_def.and_then(|r| r.0.clone());
    let frame_ms = (now - locals.last_now).clamp(0, 250);
    let dt = frame_ms as f32 / 1000.0;
    locals.last_now = now;
    let local_id = local.as_ref().map(|l| l.0);
    let replaying_final_death = view_subject.as_deref().map_or(false, |s| s.replaying_final_death());
    // The replay is over: the live world's bodies are back, where they were.
    if !rewound
        && !replaying_final_death
        && let Some(live) = locals.live.take()
    {
        let live = *live;
        locals.tracks = live.tracks;
        locals.corpse_bodies = live.corpse_bodies;
        locals.limp = live.limp;
        locals.joints_before = live.joints_before;
        locals.corpse_meta = live.corpse_meta;
        // Time spent watching the replay is not time the bodies were gone.
        for m in locals.corpse_meta.values_mut() {
            m.away = 0;
        }
        if body_log() {
            diag::info!(
                World,
                "bo2mp corpse: the replay is over, {} bodies' animations and {} ragdolls carry on",
                locals.tracks.len(),
                locals.limp.len()
            );
        }
    }
    let seat_focus = match view_subject.as_deref() {
        Some(EyeSubject::ChairSlot { focus, .. }) => *focus,
        _ => None,
    };
    let eye_ent = match seat_focus {
        Some(focus) => i32::try_from(focus).unwrap_or(0),
        None => local_id.map_or(0, |l| i32::try_from(l.0).unwrap_or(0)),
    };
    let gate = PlayerDrawGate {
        viewer_entity: eye_ent,
        secondary_state_bits: local_id
            .and_then(|l| presented.player(l))
            .map_or(0, |ps| ps.secondary_state_bits),
        showing_outside_camera: local_id.is_some_and(|l| {
            crate::occupancy::other_view::shown_in_third_person(&presented, l, replaying_final_death)
        }),
    };
    let registry = arms.as_ref().map(|w| &*w.0);
    let catalog = world_arms.as_ref().map(|w| &w.0).filter(|c| !c.is_empty());
    let catalogs = (
        registry.map_or(0, |r| std::ptr::from_ref(r) as usize),
        catalog.map_or(0, |c| c.identity()),
        fpv.0.identity(),
    );
    let fallback = idle_value(&script);
    let view = ViewCone::of(&cameras);
    let mut skin_jobs: Vec<SkinJob> = Vec::new();
    let before = BodySpent {
        bodies: locals.spent.bodies,
        corpses: locals.spent.corpses,
        builds: locals.spent.builds,
        build: locals.spent.build,
        pose: locals.spent.pose,
        ground: locals.spent.ground,
        skin: locals.spent.skin,
        ..Default::default()
    };
    let mut seen: Vec<u16> = Vec::new();
    for (ent, identity, rt) in &ents {
        let mut corpse_since: Option<i32> = None;
        let kind = net::peer_avatar_submission_class(identity.number(), rt, gate);
        // His own body in first person is not drawn, but BO2 still runs its
        // animations' sounds and his slide.
        let own_view = kind == net::OtherPlayerDrawRoute::SelfFpv;
        if !kind.submits() && !own_view {
            continue;
        }
        let corpse = rt.pose_entity_kind == et::PLAYER_CORPSE as u8
            || rt.upcoming_condition.e_type == et::PLAYER_CORPSE;
        let client_in = if corpse {
            u32::try_from(rt.upcoming_condition.client_num).ok().map(sim::ClientId)
        } else {
            identity.client()
        };
        let Some(client_in) = client_in else { continue };
        if corpse {
            // Which body this entity holds: a new one in the same entity (BO2
            // flips the new-body flag and counts a new use) starts over, the
            // same one seen again after a gap carries on where it was.
            let at = identity.number();
            let bit = rt.upcoming_condition.lerp.e_flags as u32 & entity_flag_bits::TELEPORT;
            let mut since = now;
            match locals.corpse_meta.get(&at).copied() {
                Some(m) if m.owner_client != client_in.0 || m.new_body_bit != bit => {
                    forget_corpse(&mut locals, at);
                    if body_log() {
                        diag::info!(
                            World,
                            "bo2mp corpse {at}: a new body (client {} flag {bit}, was client {} flag {}) starts its death animation and ragdoll over",
                            client_in.0,
                            m.owner_client,
                            m.new_body_bit
                        );
                    }
                }
                Some(m) => {
                    since = m.since;
                    if m.away > 0 && body_log() {
                        diag::info!(
                            World,
                            "bo2mp corpse {at}: back after {} ms out of the snapshot, its death animation and {} carry on",
                            m.away,
                            if locals.limp.contains_key(&at) { "running ragdoll" } else { "ragdoll (not started)" }
                        );
                    }
                }
                None => {}
            }
            locals.corpse_meta.insert(
                at,
                CorpseMeta { owner_client: client_in.0, new_body_bit: bit, away: 0, since },
            );
            corpse_since = Some(since);
        }
        let current = snapshot.meta.for_client(client_in).into_iter()
            .flat_map(|m| m.client_dvars.iter())
            .find(|(k, _)| k == BODY_DVAR)
            .map(|(_, v)| v.as_str()).unwrap_or("");
        // A corpse keeps the body it fell in (he may respawn as another
        // class): the one his client had when it first showed.
        let body: String = if corpse {
            let lookup = (identity.number(), client_in.0, rt.upcoming_condition.un2.player_anim().legs_clip);
            locals
                .corpse_bodies
                .entry(lookup)
                .or_insert_with(|| current.to_owned())
                .clone()
        } else {
            current.to_owned()
        };
        if body.split(';').next().unwrap_or("").is_empty() {
            note_once(&mut locals, format!("bo2mp body: client {} has no body model yet", client_in.0));
            continue;
        }
        let gun_in_hand = rt.upcoming_condition.weapon.0;
        let offhand = anim_t6::t6_offhand(rt.upcoming_condition.un2.player_anim().torso_clip);
        let weapon = offhand.unwrap_or(gun_in_hand);
        // His composition: kept from the last frame while it is the same.
        let number = identity.number();
        let key = BuiltKey {
            body,
            weap: weapon,
            offhand: offhand.is_some(),
            corpse,
            catalogs,
        };
        let started = clock_now();
        let built = match locals.built.remove(&number) {
            Some(b) if b.key == key => Some(b),
            _ => {
                locals.spent.builds += 1;
                build_body(&mut locals, key, &fpv, registry, catalog, &script)
            }
        };
        locals.spent.build += ms_since(started);
        let Some(mut built) = built else { continue };
        locals.spent.bodies += 1;
        locals.spent.corpses += u32::from(corpse);
        // His animations, as the server picked them.
        seen.push(number);
        let first = !locals.tracks.contains_key(&number);
        let track = locals.tracks.entry(number).or_default();
        if !first && track.corpse != corpse {
            *track = BodyTrack::default();
        }
        let first = first || track.seen == 0;
        let legs = match rt.upcoming_condition.un2.player_anim().legs_clip {
            v if t6_anim_index(v).is_some() => v,
            _ => fallback,
        };
        let torso = match rt.upcoming_condition.un2.player_anim().torso_clip {
            v if t6_anim_index(v).is_some() => v,
            _ => legs,
        };
        // A stance change (to or from crouch or prone) holds the legs'
        // blends for BO2's 400 ms.
        let flags = rt.upcoming_condition.lerp.e_flags as u32;
        let stance = if flags & entity_flag_bits::LYING_FLAT != 0 {
            2
        } else {
            u8::from(flags & entity_flag_bits::CROUCH_BIT != 0)
        };
        if !first && stance != track.stance {
            track.stance_until = now + T6_STANCE_TRANSITION_MS;
        }
        track.stance = stance;
        let clip_of = |v: i32| {
            let clip = xanim_set.0.clip(asset_core::AssetCategoryScope::T6, script.anim_name(v)?)?;
            Some((clip.duration(), clip.looping))
        };
        let stance_hold_ms = t6_stance_hold_ms(track.stance_until, now);
        let change = Change { now, first, stance_hold_ms, client_no: client_in.0 };
        track.legs.see(legs, change, &script, &clip_of);
        track.torso.see(torso, Change { stance_hold_ms: 0, ..change }, &script, &clip_of);
        // A moving legs animation plays at his speed over its own (feet
        // that keep to the ground); the torso with it when it is the same.
        let velocity = if corpse {
            [0.0; 3]
        } else {
            presented.player(client_in).map_or([0.0; 3], |ps| ps.velocity)
        };
        let legs_rate = t6_anim_index(track.legs.cur.value)
            .map_or(1.0, |a| t6_legs_rate(&script, a, velocity));
        // A torso animation timed to the weapon (a reload, a raise or drop)
        // plays over the weapon's time.
        let timed_rate = t6_anim_duration(torso).and_then(|secs| {
            let name = script.anim_name(torso)?;
            let clip = xanim_set.0.clip(asset_core::AssetCategoryScope::T6, name)?;
            Some((clip.duration() / secs.max(0.05)).clamp(0.2, 5.0))
        });
        let torso_rate = if let Some(rate) = timed_rate {
            rate
        } else if t6_anim_index(track.torso.cur.value) == t6_anim_index(track.legs.cur.value) {
            legs_rate
        } else {
            1.0
        };
        track.legs.verify(now);
        track.torso.verify(now);
        track.legs.step_on(dt, legs_rate);
        track.torso.step_on(dt, torso_rate);
        // BO2's own sounds his body's animations call for: a dive's takeoff
        // and landing, and a body hitting the ground (the death animation's
        // bodyfall notetracks), by the surface under him. His own in first
        // person too: BO2 plays them for every player.
        body_sounds(
            &mut sounds,
            &mut rumbles,
            &script,
            &xanim_set,
            clip.as_deref().and_then(|c| c.0.as_deref()),
            track,
            rt.origin,
            u32::from(identity.number()),
            corpse,
            Some(client_in) != local_id,
        );
        let legs_target = if corpse { 0.0 } else { t6_legs_yaw(rt.upcoming_condition.un2.player_anim().legs_clip) };
        track.legs_yaw = if first {
            legs_target
        } else {
            let step = LEGS_YAW_SPEED_DEG * dt;
            let d = legs_target - track.legs_yaw;
            if d.abs() <= step { legs_target } else { track.legs_yaw + step * d.signum() }
        };
        track.seen = now;
        track.corpse = corpse;
        let track = *track;
        // BO2's slide, for every live player (CG_Player): slide_rumble as
        // he starts, stopped as he stops, and its dust when it is due.
        let sliding = !corpse
            && t6_anim_index(rt.upcoming_condition.un2.player_anim().legs_clip).is_some_and(|a| locals.slide_anims.1.contains(&a));
        let (slide_rumble, dust_due) = slide_step(&mut locals.slide_fx, sliding, number, now);
        match slide_rumble {
            Some(SlideRumble::Start) => {
                rumbles.write(audio::CodeRumbleT6 { name: SLIDE_RUMBLE, number });
            }
            Some(SlideRumble::Stop) => {
                rumble_stops.write(audio::CodeRumbleStopT6 { name: SLIDE_RUMBLE, number });
            }
            None => {}
        }
        // His own body in first person is posed only for its dust.
        if own_view && !dust_due {
            locals.built.insert(number, built);
            continue;
        }
        let posing = clock_now();
        // A body its ragdoll has laid to rest lies as it settled (the
        // ragdoll keeps that pose, below): its death animation is not posed
        // again only to be thrown away.
        let settled = {
            let next = &rt.upcoming_condition;
            let launched = (next.lerp.pos.tr_type == TR_RAGDOLL_GRAVITY && next.lerp.pos.span_ms == 1)
                .then_some(next.lerp.pos.began_at);
            corpse
                && ragdoll_def.is_some()
                && (RAGDOLL_TR_TYPES).contains(&next.lerp.pos.tr_type)
                && locals.limp.get(&number).is_some_and(|(r, thrown)| {
                    r.as_ref().is_some_and(|r| r.asleep()) && (launched.is_none() || *thrown == launched)
                })
        };
        let mut clips = Vec::<(Arc<xmodel_runtime::SkeletalClip>, Arc<[Option<usize>]>, f32, f32, bool)>::new();
        for (part, torso_part) in [(&track.legs, false), (&track.torso, true)].into_iter().filter(|_| !settled) {
            for (playing, w) in part.weighted(now) {
                let Some(name) = script.anim_name(playing.value) else { continue };
                let Some(clip) = xanim_set.0.clip(asset_core::AssetCategoryScope::T6, name) else {
                    note_once(&mut locals, format!("bo2mp body: animation {name} is not loaded"));
                    continue;
                };
                let duration = clip.duration().max(1e-3);
                let mut t = playing.phase.max(0.0);
                t = if clip.looping { t.rem_euclid(duration) } else { t.min(duration) };
                let tracks = built.tracks_for(&clip);
                let weight = if torso_part { w * TORSO_OVER_LEGS } else { w };
                clips.push((clip, tracks, t, weight, torso_part));
            }
        }
        if clips.is_empty() && !settled {
            // Never a bind pose: the script's idle.
            if let Some(clip) = script
                .anim_name(fallback)
                .and_then(|n| xanim_set.0.clip(asset_core::AssetCategoryScope::T6, n))
            {
                let tracks = built.tracks_for(&clip);
                clips.push((clip, tracks, 0.0, 1.0, false));
            } else {
                locals.built.insert(number, built);
                continue;
            }
        }
        let (dobj, parts) = (&built.obj, &built.pieces);
        let instances: Vec<xmodel_runtime::ClipPlayback<'_>> = clips
            .iter()
            .map(|(clip, tracks, time, weight, torso_part)| xmodel_runtime::ClipPlayback {
                clip,
                tracks: &tracks[..],
                time: *time,
                weight: *weight,
                parts: torso_part.then_some(&built.upper),
            })
            .collect::<_>();
        let pitch = rt.angles[0];
        let prone = rt.upcoming_condition.lerp.e_flags as u32 & entity_flag_bits::LYING_FLAT != 0;
        let crouch = rt.upcoming_condition.lerp.e_flags as u32 & entity_flag_bits::CROUCH_BIT != 0;
        let legs_yaw = track.legs_yaw.to_radians();
        let mut world = if settled {
            vec![Mat4::IDENTITY; dobj.bones.len()]
        } else {
            let ident = Mat4::IDENTITY;
            dobj.pose_using_aim(&instances, &built.all, ident, |dobj, _, locals| {
                if !corpse {
                    xmodel_runtime::pose_body_aim(
                        dobj,
                        locals,
                        xmodel_runtime::BodyAimInput {
                            body_lean_ratio: 0.0,
                            crouched: crouch,
                            lying: prone,
                            look_pitch_degrees: pitch,
                        },
                    );
                }
            })
        };
        // On a ladder he faces it, all of him (the climb's hands are on its
        // rungs); otherwise his legs turn in place behind his view and twist
        // to a diagonal move while the torso keeps to the view.
        let climbing = !settled
            && script
                .anim_name(track.legs.cur.value)
                .is_some_and(|n| n.starts_with("pb_climb") || n.starts_with("pb_riot_climb"));
        if !corpse && !climbing {
            twist_legs(dobj, &mut world, legs_yaw);
        }
        locals.spent.pose += ms_since(posing);
        let grounding = clock_now();
        let origin = rt.origin;
        let body_yaw = if climbing { rt.angles[1] + track.legs_yaw } else { rt.angles[1] };
        if shake_log() && !corpse && Some(client_in) != local_id {
            // His ankles where they are drawn (body space) and what his legs blend.
            let foot = |n: &str| dobj.find(n).and_then(|i| world.get(i)).map_or([0.0; 2], |m| [m.w_axis.x, m.w_axis.y]);
            let (fl, fr) = (foot("j_ankle_le"), foot("j_ankle_ri"));
            let mix: Vec<String> = track
                .legs
                .weighted(now)
                .iter()
                .map(|(p, w)| {
                    let name = script.anim_name(p.value).unwrap_or("?");
                    let len = xanim_set.0.clip(asset_core::AssetCategoryScope::T6, name).map_or(1.0, |c| c.duration().max(1e-3));
                    format!("{name}@{:.3}*{w:.2}", (p.phase / len).rem_euclid(1.0))
                })
                .collect();
            info!(
                World,
                "shakelog: t {now} c {} at {:.1} {:.1} {:.1} yaw {:.2} legs {legs:#x} torso {torso:#x} legsyaw {:.1} drawn {:?} snap {} feet {:.1} {:.1} {:.1} {:.1} mix {}",
                client_in.0, origin[0], origin[1], origin[2], rt.angles[1], track.legs_yaw,
                rt.shown_participant.map(|(_, at)| at), snapshot.tick.0, fl[0], fl[1], fr[0], fr[1], mix.join(" "),
            );
        }
        let body_rot = Quat::from_rotation_z(body_yaw.to_radians());
        let mut world_to_local = Mat4::from_scale_rotation_translation(Vec3::splat(1.0), body_rot, origin.into());
        // A corpse rests on the ground under it: tilted along its length to
        // the ground under its head, hips and feet, and set so none of them
        // sinks below the height the death animation gives it.
        // A body gone limp is placed by its ragdoll alone (below).
        let ragdolled = corpse
            && (RAGDOLL_TR_TYPES).contains(&rt.upcoming_condition.lerp.pos.tr_type)
            && locals.limp.get(&number).is_some_and(|(r, _)| r.is_some());
        if corpse
            && !ragdolled
            && let Some(clip) = clip.as_deref().and_then(|c| c.0.as_deref())
        {
            let lying = track
                .legs
                .weighted(now)
                .first()
                .and_then(|(p, _)| {
                    let clip = xanim_set.0.clip(asset_core::AssetCategoryScope::T6, script.anim_name(p.value)?)?;
                    Some((p.phase / clip.duration().max(1e-3)).clamp(0.0, 1.0))
                })
                .unwrap_or(1.0);
            // At the end of its death animation, where it lay last frame,
            // it rests where it rested then (nothing traced again).
            world_to_local = match built.rested {
                Some((at, yaw, rested)) if lying >= 1.0 && at == origin && yaw == body_yaw => rested,
                _ => {
                    let _slow = SlowGuard::new("a corpse resting on the ground");
                    let rested = rest_on_ground(dobj, &world, world_to_local, origin, clip, lying);
                    built.rested = (lying >= 1.0).then_some((origin, body_yaw, rested));
                    rested
                }
            };
        }
        // A body gone limp (the server's ragdoll trajectory, from the
        // script's startragdoll or launchragdoll): BO2's ragdoll takes it
        // from its death animation where it is, at the speed it moved.
        if corpse && let Some(def) = ragdoll_def.as_deref() {
            let next = &rt.upcoming_condition;
            let limp = (RAGDOLL_TR_TYPES).contains(&next.lerp.pos.tr_type);
            let launch = (next.lerp.pos.tr_type == TR_RAGDOLL_GRAVITY && next.lerp.pos.span_ms == 1)
                .then(|| (Vec3::from_array(next.lerp.pos.rate), next.lerp.pos.began_at));
            let clip = clip.as_deref().and_then(|c| c.0.as_deref());
            let l = &mut *locals;
            if !limp {
                l.limp.remove(&number);
                if let Some(points) = joint_points(def, dobj, &world, world_to_local) {
                    l.joints_before.insert(number, (points, now));
                }
            } else {
                let mut started = None;
                let (ragdoll, thrown) = l.limp.entry(number).or_insert_with(|| {
                    let last = l
                        .joints_before
                        .get(&number)
                        .map(|(p, t)| (p.as_slice(), (now - *t) as f32 / 1000.0));
                    let _slow = SlowGuard::new("a ragdoll's start");
                    let r = Ragdoll::start(def, dobj, &world, world_to_local, last, launch.map_or(Vec3::ZERO, |(v, _)| v), clip);
                    started = Some(r.as_ref().map_or_else(
                        || format!("bo2mp ragdoll: {} lacks a joint the ragdoll names", parts[0].bone_rig.name),
                        |r| r.note().to_owned(),
                    ));
                    (r, launch.map(|(_, at)| at))
                });
                if let Some(r) = ragdoll {
                    // Each throw once: as in BO2's Ragdoll_ExplosionEvent, a
                    // later one (a grenade, a destructible's blast) kicks a
                    // body that is already limp or at rest again.
                    if let Some((v, at)) = launch
                        && *thrown != Some(at)
                    {
                        r.fling(v);
                        *thrown = Some(at);
                    }
                    // The Gravity Gun's hold or throw (the menu's guest arms).
                    let (held, throw) = super::ragdoll::beam_on(u32::from(number));
                    r.beam(held, throw);
                    {
                        let _slow = SlowGuard::new("a ragdoll's step");
                        r.step(dt, clip);
                    }
                    l.beam_seen.insert(u32::from(number), r.seen());
                    world_to_local = r.pose(&mut world);
                }
                l.joints_before.remove(&number);
                if let Some(line) = started {
                    note_once(l, line);
                }
            }
        }
        locals.spent.ground += ms_since(grounding);
        if dust_due
            && let Some((le, ri)) = built.ankles
            && let (Some(le), Some(ri)) = (world.get(le), world.get(ri))
        {
            let (le, ri) = (world_to_local * *le, world_to_local * *ri);
            dust.write(T6SlideDust {
                origin: ((le.w_axis + ri.w_axis) * 0.5).truncate().to_array(),
                axis: [ri.x_axis, ri.y_axis, ri.z_axis].map(|a| a.truncate().normalize_or_zero().to_array()),
            });
        }
        if own_view {
            locals.built.insert(number, built);
            continue;
        }
        // Drawn (and so skinned) only near the view; a body whose bones
        // have not moved keeps its vertices; the rest are skinned after the
        // loop, together.
        let (light_at, reach) = parts[0].bone_rig.bounds.map_or(
            ([origin[0], origin[1], origin[2] + 36.0], 48.0),
            |(lo, hi)| {
                let mid = Vec3::new((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5);
                let half = Vec3::new(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]).length() * 0.5;
                (world_to_local.transform_point3(mid).to_array(), half)
            },
        );
        let shown = view
            .as_ref()
            .is_none_or(|v| v.sees(Vec3::from_array(light_at), reach + SHADOW_REACH));
        let kept = shown && built.skinned.len() == parts.len() && same_pose(&built.skinned_bones, &world);
        if !shown {
            locals.spent.unseen += 1;
        } else if kept {
            locals.spent.still += 1;
        }
        // The local player's own corpse: where its j_mainroot is, for the
        // death-watch camera. With several, the body that showed last: his
        // latest death (a corpse's trajectory time is no age: it goes to 0
        // once it lands, so every settled body of his ties).
        if corpse
            && local_id == Some(client_in)
            && let Some(since) = corpse_since
            && corpse_root_time.is_none_or(|t| since >= t)
            && let Some(root) = built.mainroot.and_then(|i| world.get(i))
        {
            corpse_root_time = Some(since);
            poses.local_corpse_root = Some((world_to_local.transform_point3(root.w_axis.truncate()).to_array(), since));
            if std::env::var_os("BO2MP_BODYDRAWLOG").is_some() {
                info!(World, "bo2mp corpse root: {:?} entity origin {:?}", poses.local_corpse_root, origin);
            }
        }
        if shown {
            if !kept {
                skin_jobs.push(SkinJob {
                    body_no: number,
                    first_item: out.items.len(),
                    bones: world.clone(),
                });
            }
            for (i, part) in parts.iter().enumerate() {
                out.items.push(T6MissileModel {
                    bone_rig: part.bone_rig.clone(),
                    materials: part.mat_list.clone(),
                    model_to_world: world_to_local,
                    origin,
                    light_at,
                    // Its kept vertices, or this frame's (skinned below).
                    posed: if kept { built.skinned.get(i).cloned() } else { None },
                    hide: None,
                    eye_luminance: None,
                });
            }
        }
        // His gun's tags, for its muzzle flash, shells and tracers.
        if let Some(head) = built.head.and_then(|i| world.get(i)) {
            let head = world_to_local.transform_point3(head.w_axis.truncate());
            poses.heads.push((identity.number(), head.to_array()));
        }
        if built.right_gun.is_some() {
            poses.rows.push(PosedBody {
                id: ent,
                body_no: identity.number(),
                scene_from_local: world_to_local,
                flash: built.flash,
                brass: built.brass,
                bones: world.clone(),
            });
        }
        if std::env::var_os("BO2MP_BODYDRAWLOG").is_some() && (now / 1000) != ((now - 16) / 1000) {
            diag::info!(
                World,
                "bo2mp body draw: ent {number} client {} {} corpse {corpse} weapon {} parts {} legs {:?} torso {:?} at {:?} yaw {:.0} pitch {:.0}",
                client_in.0,
                parts[0].bone_rig.name,
                built.key.weap,
                parts.len(),
                script.anim_name(track.legs.cur.value),
                script.anim_name(track.torso.cur.value),
                origin,
                rt.angles[1],
                pitch
            );
        }
        locals.built.insert(number, built);
    }
    // This frame's skinning, the bodies at once.
    let skinning = clock_now();
    let bodies_end = out.items.len();
    let skinned = {
        let work: Vec<_> = skin_jobs
            .iter()
            .map(|j| {
                locals
                    .built
                    .get(&j.body_no)
                    .map(|b| (&b.obj, b.pieces.as_slice(), j.bones.as_slice()))
            })
            .collect::<_>();
        skin_bodies(&work)
    };
    let mut unskinned = false;
    for (job, posed) in skin_jobs.into_iter().zip(skinned) {
        let Some(posed) = posed else {
            unskinned = true;
            continue;
        };
        for (k, vertex_points) in posed.iter().enumerate() {
            if let Some(item) = out.items.get_mut(job.first_item + k) {
                item.posed = Some(vertex_points.clone());
            }
        }
        if let Some(b) = locals.built.get_mut(&job.body_no) {
            b.skinned_bones = job.bones;
            b.skinned = posed;
        }
    }
    // Never a bind pose: a body left unskinned is not drawn.
    if unskinned {
        let mut at = 0;
        out.items.retain(|item| {
            at += 1;
            at > bodies_end || item.posed.is_some()
        });
    }
    locals.spent.skin += ms_since(skinning);
    // A corpse out of the snapshot for a moment keeps its animation and
    // ragdoll (until BO2's ragdoll would be gone); everything else goes with
    // the frame it was last seen in.
    age_corpse_gaps(&mut locals, &seen, frame_ms);
    let l = &mut *locals;
    l.tracks.retain(|n, _| seen.contains(n) || l.corpse_meta.contains_key(n));
    l.built.retain(|n, _| seen.contains(n));
    let took = ms_since(bodies_started);
    locals.spent.all += took;
    locals.spent.n_frames += 1;
    // A slow frame names its part.
    if took > 20.0 {
        let s = &locals.spent;
        diag::warn!(
            World,
            "bo2mp slow: t6 bodies took {took:.1} ms (build {:.1} with {} rebuilt, pose {:.1}, ground {:.1}, skin {:.1}; {} bodies, {} corpses)",
            s.build - before.build,
            s.builds - before.builds,
            s.pose - before.pose,
            s.ground - before.ground,
            s.skin - before.skin,
            s.bodies - before.bodies,
            s.corpses - before.corpses
        );
    }
    let l = &mut *locals;
    l.corpse_bodies.retain(|(n, _, _), _| seen.contains(n) || l.corpse_meta.contains_key(n));
    l.limp.retain(|n, _| seen.contains(n) || l.corpse_meta.contains_key(n));
    {
        let mut bodies: Vec<(u32, &mut Ragdoll)> = l
            .limp
            .iter_mut()
            .filter_map(|(n, (r, _))| r.as_mut().map(|r| (u32::from(*n), r)))
            .collect::<_>();
        super::ragdoll::knock(&mut bodies);
    }
    super::ragdoll::publish_seen(std::mem::take(&mut l.beam_seen));
    l.joints_before.retain(|n, _| seen.contains(n) || l.corpse_meta.contains_key(n));

    let _slow_items = SlowGuard::new("t6 ground items");
    let items_started = clock_now();
    // Guns and Scavenger bags on the ground.
    let (Some(registry), Some(catalog)) = (registry, catalog) else {
        return;
    };
    let scavenger = local_id
        .and_then(|l| presented.player(l))
        .is_some_and(|ps| ps.has_perk_bit(PERK_SCAVENGER));
    let mut seen_items: Vec<i32> = Vec::new();
    for es in snapshot.meta.entities.iter().filter(|es| es.e_type == et::ITEM) {
        let Some(weapon) = Some(es.weapon.0).filter(|&w| w != 0) else {
            continue;
        };
        if es.lerp.e_flags & 0x20 != 0 {
            continue;
        }
        let bag = snapshot
            .meta
            .item_ammo
            .iter()
            .any(|row| row.entnum == es.slot_index && row.scavenger != 0);
        if bag && !scavenger {
            continue;
        }
        let shape = shapes
            .0
            .entry(weapon)
            .or_insert_with(|| item_shape(registry, catalog, weapon))
            .clone();
        let Some(shape) = shape else { continue };
        let origin = evaluate_trajectory(&es.lerp.pos, now);
        let [_, yaw, _] = evaluate_trajectory(&es.lerp.apos, now);
        // A dropped gun lies on its side with the yaw it fell with (which
        // side by its number), tipping over as it falls; lifted so that
        // side rests on the ground.
        let first_seen = *locals.item_seen.entry(es.slot_index).or_insert(now);
        seen_items.push(es.slot_index);
        let tip = if es.lerp.pos.tr_type == path_kind::FALLING {
            ((now - first_seen) as f32 / ITEM_TIP_MS).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let side = if es.slot_index % 2 == 0 { 1.0 } else { -1.0 };
        let lift = if side > 0.0 { -shape.low_y } else { shape.high_y } * tip;
        let turn = Quat::from_rotation_z(yaw.to_radians()) * Quat::from_rotation_x((side * 90.0 * tip).to_radians());
        let placed_mat = Mat4::from_scale_rotation_translation(Vec3::splat(1.0), turn, Vec3::new(origin[0], origin[1], origin[2] + lift));
        let light_at = [origin[0], origin[1], origin[2] + 4.0];
        for (bone_rig, materials, vertex_points) in &shape.models {
            out.items.push(T6MissileModel {
                bone_rig: bone_rig.clone(),
                materials: materials.clone(),
                model_to_world: placed_mat,
                origin,
                light_at,
                posed: Some(vertex_points.clone()),
                hide: None,
                eye_luminance: None,
            });
        }
    }
    locals.item_seen.retain(|n, _| seen_items.contains(n));
    locals.spent.items += ms_since(items_started);
}

/// The rate a legs animation plays at, as BO2's BG_RunLerpFrameRate does
/// it: an `animrate` plays at that rate; a moving animation at his speed
/// over its own (a ladder climb by his climbing speed), within
/// [`t6_moving_rate`]'s limits. The fast mantle's clips play at their
/// `animrate` too (2.5), as BO2's do.
fn t6_legs_rate(script: &T6PlayerAnims, anim: u16, velocity_in: [f32; 3]) -> f32 {
    if let Some(rate) = script.command_of(anim).and_then(|c| c.anim_rate) {
        return rate;
    }
    let natural = script.speed_of(anim);
    // An animation that does not move: BO2 plays one that turns by
    // anim_t6's t6_turn_rate, but our parsed script has no rotSpeed, so it
    // plays at 1 (facts/QUESTIONS.md).
    if natural <= 0.0 {
        return 1.0;
    }
    let ladder = script.is_ladder(anim);
    let speed = if ladder {
        velocity_in[2].abs()
    } else {
        velocity_in[0].hypot(velocity_in[1])
    };
    // On the ground a body that is not moving keeps its rate (the server
    // has him idle a tick later).
    if !ladder && speed <= 1.0 {
        return 1.0;
    }
    t6_moving_rate(speed / natural, natural, ladder)
}

mod slide_tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn slide_rumble_starts_once_and_stops_once() {
        let mut fx = HashMap::new();
        assert_eq!(slide_step(&mut fx, false, 3, 100), (None, false));
        assert_eq!(slide_step(&mut fx, true, 3, 100), (Some(SlideRumble::Start), true));
        assert_eq!(fx[&3], 105);
        // Due again only once 5 ms have passed.
        assert_eq!(slide_step(&mut fx, true, 3, 104), (None, false));
        assert_eq!(slide_step(&mut fx, true, 3, 105), (None, false));
        assert_eq!(slide_step(&mut fx, true, 3, 106), (None, true));
        assert_eq!(slide_step(&mut fx, false, 3, 120), (Some(SlideRumble::Stop), false));
        assert_eq!(slide_step(&mut fx, false, 3, 130), (None, false));
        // A new slide starts it again.
        assert_eq!(slide_step(&mut fx, true, 3, 140), (Some(SlideRumble::Start), true));
    }
}

mod part_track_tests {
    #![cfg(test)]
    use super::*;

    const SCRIPT: &str = "DEFINES\nANIMATIONS\nSTATE COMBAT\n{\n\trun\n\t{\n\t\tdefault\n\t\t{\n\t\t\tboth pb_run\n\t\t}\n\t}\n\twalk\n\t{\n\t\tdefault\n\t\t{\n\t\t\tboth pb_walk\n\t\t}\n\t}\n}\nEVENTS\n";

    fn script() -> T6PlayerAnims {
        let mut s = T6PlayerAnims::parse(SCRIPT, "none\ndefault").unwrap();
        s.set_speeds(&[("pb_run".into(), 190.0), ("pb_walk".into(), 75.0)]);
        s
    }

    /// The packed value of the named animation, with the toggle bit when asked.
    fn value(s: &T6PlayerAnims, name: &str, toggle: bool) -> i32 {
        let n = s.anims.iter().position(|a| a == name).unwrap() as i32 + 1;
        if toggle { n | anim_t6::T6_ANIM_TOGGLE } else { n }
    }

    fn change(now: i32, first: bool) -> Change {
        Change { now, first, stance_hold_ms: 0, client_no: 3 }
    }

    /// Both loops one second long.
    fn reel(_: i32) -> Option<(f32, bool)> {
        Some((1.0, true))
    }

    fn weight_of(track: &PartTrack, now: i32, v: i32) -> f32 {
        track.weighted(now).iter().filter(|(p, _)| t6_anim_index(p.value) == t6_anim_index(v)).map(|(_, w)| w).sum()
    }

    #[test]
    fn a_walk_taking_over_a_run_starts_at_the_runs_time() {
        let s = script();
        let (run, walk) = (value(&s, "pb_run", false), value(&s, "pb_walk", true));
        let mut track = PartTrack::default();
        track.see(run, change(0, true), &s, &reel);
        track.step_on(0.4, 1.0);
        let at = (track.cur.phase / 1.0).rem_euclid(1.0);
        track.see(walk, change(400, false), &s, &reel);
        assert!((track.cur.phase - at).abs() < 1e-4, "walk starts at {} not the run's {at}", track.cur.phase);
    }

    #[test]
    fn a_run_back_inside_the_blend_keeps_its_weight_and_time() {
        let s = script();
        let (run, walk, run_again) = (value(&s, "pb_run", false), value(&s, "pb_walk", true), value(&s, "pb_run", false));
        let mut track = PartTrack::default();
        track.see(run, change(0, true), &s, &reel);
        track.step_on(1.3, 1.0);
        track.see(walk, change(1300, false), &s, &reel);
        let blend = track.crossfade_millis;
        // A quarter of the way into the walk's blend the run is back.
        let back = 1300 + blend / 4;
        let run_w = weight_of(&track, back, run);
        track.step_on(blend as f32 / 4000.0, 1.0);
        let run_phase = track.weighted(back).iter().find(|(p, _)| t6_anim_index(p.value) == t6_anim_index(run)).unwrap().0.phase;
        track.see(run_again, change(back, false), &s, &reel);
        // No jump: the run has the weight it had, and its time.
        assert!((weight_of(&track, back, run) - run_w).abs() < 1e-4, "run weight {} -> {}", run_w, weight_of(&track, back, run));
        assert!((weight_of(&track, back, walk) - (1.0 - run_w)).abs() < 1e-4);
        assert!((track.cur.phase - run_phase).abs() < 1e-4);
        // And the weights still add up to one on the way in.
        let mid = back + track.crossfade_millis / 2;
        assert!((weight_of(&track, mid, run) + weight_of(&track, mid, walk) - 1.0).abs() < 1e-4);
        track.verify(back + track.crossfade_millis);
        assert_eq!(track.weighted(back + track.crossfade_millis).len(), 1);
    }
}
