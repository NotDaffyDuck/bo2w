//! The first-person gun's per-frame driver: turns what the local player's state says
//! (authority cues, BO2's own weapon fields) into events and poses on the held gun's
//! [`FirstPersonGunAnimator`]s, and keeps the cursors that tell a new event from a repeat.

use std::sync::Arc;

use anim_bank::SkeletalClip;
use asset_game::GunMoveRole;
use weapon_t6::anim::TOGGLE;
use weapon_t6::asset_slots::{VmAnim, vm_anim};
use weapon_t6::viewmodel::{AnimSet, Clip, Viewmodel, VmFrame, VmTable, slot};

use crate::motion::t6_loop_seam::{ViewSkeleton, smooth_loop_seam};
use crate::motion::gun_rig_ctl::bo2_slot;
use crate::{StepOutcome, EventReaction, FirstPersonGunAnimator, GunAnimEvent, GunPhase};

// ---------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldGunCue {
    Fire,
    TriggerUp,
    LiftFirst,
    Raise,
    Reload,
    DryReload,
    ReloadKickoff,
    ReloadEnd,
    Rechamber,
    Drop,
    SprintIn,
    SprintLoopInfo,
    SprintOut,
}

/// What the authority said happened this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerGunCues {
    pub fire_confirmed: bool,
    pub created: bool,
    pub trigger_let_go: bool,
}

/// The pose the first-person gun ended the frame in.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldPoseReadout {
    pub state: GunPhase,
    pub collision_label: Option<String>,
    pub clip_secs: f32,
    pub shot_blend: f32,
    pub rest_blend: f32,
}

/// The events an authority tick's cues call for: a shot, then a trigger release.
pub fn map_authority_to_fpv(cues: ServerGunCues) -> Vec<HeldGunCue> {
    let in_order = [
        (cues.fire_confirmed, HeldGunCue::Fire),
        (cues.trigger_let_go, HeldGunCue::TriggerUp),
    ];
    in_order
        .into_iter()
        .filter_map(|(called, event)| called.then_some(event))
        .collect::<_>()
}

/// The controller event for a present event; releasing the trigger is not an event of
/// its own, the controller just settles.
fn to_vm_event(ev: HeldGunCue) -> Option<GunAnimEvent> {
    match ev {
        HeldGunCue::TriggerUp => None,
        HeldGunCue::LiftFirst | HeldGunCue::Raise => Some(GunAnimEvent::Raise {
            initial: ev == HeldGunCue::LiftFirst,
            hasty: false,
        }),
        HeldGunCue::Reload | HeldGunCue::DryReload => Some(GunAnimEvent::ReloadInfo {
            empty: ev == HeldGunCue::DryReload,
        }),
        HeldGunCue::Fire => Some(GunAnimEvent::Fire),
        HeldGunCue::ReloadKickoff => Some(GunAnimEvent::ReloadKickoff),
        HeldGunCue::ReloadEnd => Some(GunAnimEvent::ReloadEnd),
        HeldGunCue::Rechamber => Some(GunAnimEvent::Rechamber),
        HeldGunCue::Drop => Some(GunAnimEvent::Drop),
        HeldGunCue::SprintIn => Some(GunAnimEvent::SprintIn),
        HeldGunCue::SprintLoopInfo => Some(GunAnimEvent::SprintLoopData),
        HeldGunCue::SprintOut => Some(GunAnimEvent::SprintOut),
    }
}

/// How a controller refusal for a missing clip is worded in the log.
#[derive(Clone, Copy)]
enum NoClipWording {
    NoClipFor,
    NotStarted,
}

/// Logs why the controller did not start what it was asked to; a started one is silent.
fn warn_unless_started(result: EventReaction, subject: std::fmt::Arguments, wording: NoClipWording) {
    match (result, wording) {
        (EventReaction::Started(_), _) => {}
        (EventReaction::IgnoredState(state), _) => {
            diag::warn!(ViewModelPerspective, "fpv: {subject} ignored in state {state:?}");
        }
        (EventReaction::IgnoredMissingClip(gone), NoClipWording::NoClipFor) => {
            diag::warn!(ViewModelPerspective, "fpv: {subject} ignored: no clip for {gone:?}");
        }
        (EventReaction::IgnoredMissingClip(gone), NoClipWording::NotStarted) => {
            diag::warn!(ViewModelPerspective, "fpv: {subject} ({gone:?}) missing, not started");
        }
    }
}

fn feed_events(controller: &mut FirstPersonGunAnimator, events: &[HeldGunCue]) {
    for &event in events {
        let Some(vm_event) = to_vm_event(event) else {
            controller.relax_firing_to_idle();
            continue;
        };
        let outcome = controller.hnd(vm_event);
        warn_unless_started(outcome, format_args!("{event:?}"), NoClipWording::NoClipFor);
    }
}

/// Whether BO2 viewmodel anim `anim` is a shot.
pub fn is_predicted_fire_weap_animation(anim: i32) -> bool {
    weapon_t6::asset_slots::is_fire_anim(anim)
}

/// The pose of whichever clip weighs most right now, with the fire and idle weights.
pub fn sample_pose(controller: &FirstPersonGunAnimator) -> HeldPoseReadout {
    let playing = controller.active_anims();
    let heaviest = playing.max_by(|a, b| a.level.total_cmp(&b.level));
    let (collision_label, clip_secs) = match &heaviest {
        Some(a) => (Some(a.reel.name.clone()), a.time),
        None => (None, 0.0),
    };
    let [shot_blend, rest_blend] = [GunMoveRole::Fire, GunMoveRole::Idle]
        .map(|which| controller.animation_wt(which));
    let state = controller.state();
    HeldPoseReadout { clip_secs, rest_blend, state, shot_blend, collision_label }
}

// ---------------------------------------------------------------------------------------
// Authority samples and the shot cursor
// ---------------------------------------------------------------------------------------

/// The local player's state for one authority tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ServerTruthSnapshot {
    pub advance: u32,
    pub gun_state: i32,
    pub cues: ServerGunCues,
    pub is_sprinting: bool,
    pub frac_ads: f32,
    /// The viewmodel anim of the right hand, and of the left when dual wielding.
    pub weap_animation: i32,
    pub weap_animation_secondary: i32,
    pub previous_gun_hand: i32,
    pub specialties: [u32; 2],
    pub magazine_rounds: Option<i32>,
    pub left_clip_ammo: Option<i32>,
}

/// What names one local shot: the same identity twice is the same shot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShotStamp {
    pub lifespan_sequence: u32,
    pub side: u8,
    pub gun_shot_count: i32,
    /// The anim's toggle bit, which flips to restart a shot animation in place.
    pub anim_toggle_bit: bool,
}

pub fn own_shot_signature(
    life_in: u32,
    hand: u8,
    rounds_fired: i32,
    anim_word: i32,
) -> ShotStamp {
    let anim_toggle_bit = anim_word & TOGGLE != 0;
    ShotStamp { anim_toggle_bit, gun_shot_count: rounds_fired, side: hand, lifespan_sequence: life_in }
}

/// Tells new local shots from repeats of the last identity seen.
#[derive(Clone, Debug, Default)]
pub struct ShotStampTracker {
    seen: Option<ShotStamp>,
}

impl ShotStampTracker {
    pub fn clear(&mut self) {
        self.seen.take();
    }

    pub fn last(&self) -> Option<ShotStamp> {
        self.seen
    }

    /// Whether `identity` is a new shot. The first identity ever seen only sets the
    /// baseline. After that a changed shot count is a shot, and so is a flipped restart
    /// bit on the same count; a change of life or hand alone is not.
    pub fn note_shot(&mut self, identity: ShotStamp) -> bool {
        let Some(before) = self.seen.replace(identity) else {
            return false;
        };
        before.gun_shot_count != identity.gun_shot_count
            || before.anim_toggle_bit != identity.anim_toggle_bit
    }
}

// ---------------------------------------------------------------------------------------
// Viewmodel anim edges
// ---------------------------------------------------------------------------------------

/// What a change of the viewmodel anim asks of the old viewmodel player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmAnimChange {
    Unchanged,
    Idle,
    ForceIdle,
    /// Play the clip node with this index.
    DispatchInfo(usize),
}

/// Compares BO2 viewmodel anim `raw` with the one before it. Anims that play the same
/// slot count as one, so only a new slot or a restart (the toggle bit) is an edge.
fn weap_anim_edge(previous: &mut Option<i32>, raw: i32) -> VmAnimChange {
    let vm = vm_anim(raw);
    let base = match vm {
        VmAnim::Idle => 0,
        VmAnim::ForceIdle => 1,
        VmAnim::Slot(slot) => 2 + slot as i32,
    };
    let key = base | (raw & TOGGLE);
    let repeated = previous.replace(key) == Some(key);
    match (repeated, vm) {
        (true, _) => VmAnimChange::Unchanged,
        (false, VmAnim::Idle) => VmAnimChange::Idle,
        (false, VmAnim::ForceIdle) => VmAnimChange::ForceIdle,
        (false, VmAnim::Slot(slot)) => VmAnimChange::DispatchInfo(slot),
    }
}

/// Plays `edge` on one hand's controller. `false` when an idle edge was refused (a
/// non-looping clip is still running), so the caller asks again next frame.
fn apply_edge(controller: &mut FirstPersonGunAnimator, edge: VmAnimChange, empty_mag_in: bool) -> bool {
    let (node, result) = match edge {
        VmAnimChange::Unchanged => return true,
        VmAnimChange::Idle => return controller.apply_resting_weap_anim(empty_mag_in),
        VmAnimChange::ForceIdle => {
            controller.apply_force_idle_weap_anim(empty_mag_in);
            return true;
        }
        VmAnimChange::DispatchInfo(node) => (node, controller.dispatch_sz_xanim_idx(node)),
    };
    warn_unless_started(
        result,
        format_args!("szXAnims[{node}]"),
        NoClipWording::NotStarted,
    );
    true
}

// ---------------------------------------------------------------------------------------
// Present state
// ---------------------------------------------------------------------------------------

/// What the driver remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct ViewmodelShowState {
    pub latest_step_seen: Option<u32>,
    pub prev_ads: Option<f32>,
    pub prev_vm_anim: Option<i32>,
    pub prev_vm_anim_left: Option<i32>,
    pub local_shot: ShotStampTracker,
    pub bo2: Bo2Fpv,
}

impl ViewmodelShowState {
    /// Forgets everything but the per-weapon facts table.
    pub fn clear(&mut self) {
        let table = Arc::clone(&self.bo2.table);
        *self = Self::default();
        self.bo2.table = table;
    }

    pub fn drop_gun_anim_memory(&mut self) {
        self.forget_right_anim();
        self.forget_left_anim();
    }

    fn forget_right_anim(&mut self) {
        self.prev_vm_anim.take();
    }

    fn forget_left_anim(&mut self) {
        self.prev_vm_anim_left.take();
    }

    pub fn vm_anim_change_right(&mut self, raw: i32) -> VmAnimChange {
        weap_anim_edge(&mut self.prev_vm_anim, raw)
    }

    pub fn vm_anim_change_left(&mut self, raw: i32) -> VmAnimChange {
        weap_anim_edge(&mut self.prev_vm_anim_left, raw)
    }
}

/// Black Ops II's first-person anim player (`weapon_t6::viewmodel`), kept across weapon
/// swaps as BO2 keeps its own.
#[derive(Clone, Debug, Default)]
pub struct Bo2Fpv {
    /// BO2's weapon fields for the coming frame, set by whoever reads the local player.
    /// While set, the BO2 player poses the gun (the frame time is filled in here); each
    /// frame is used once.
    pub frame: Option<VmFrame>,
    pub vm: Viewmodel,
    /// BO2's per-weapon viewmodel facts, by this game's weapon numbers.
    pub table: Arc<VmTable>,
    /// The held gun's clips by BO2 slot, and the gun model they were built for.
    set: AnimSet,
    set_for: Option<String>,
    /// Frame time left over below a whole millisecond (BO2 counts whole ms).
    ms_carry: f32,
    /// Whole ms played so far, for the test log.
    log_ms: i64,
    /// The run loops last checked for a wrap jump, right gun then left.
    run_loops: [[Option<Arc<SkeletalClip>>; 2]; 2],
}

/// The held gun's controllers and what the gun is made of.
#[derive(Debug)]
pub struct HeldGunLoadout {
    pub weapon_mesh: String,
    pub gun_index: assets::FpvMeshSlot,
    pub hands_index: assets::FpvMeshSlot,
    pub namespace: asset_core::AssetCategoryScope,
    pub hands: asset_model::FirstPersonArms,
    pub controller: FirstPersonGunAnimator,
    /// The left gun when dual wielding.
    pub left: Option<FirstPersonGunAnimator>,
    /// The arms-and-gun skeleton, so each arm can ease back on its own half of a smoothed
    /// run loop with what it holds kept in its hand (the ballistic knife in the left fist).
    pub view_skeleton: Option<ViewSkeleton>,
}

impl HeldGunLoadout {
    pub fn new(
        model_name: impl Into<String>,
        gun_index: assets::FpvMeshSlot,
        hands_index: assets::FpvMeshSlot,
        namespace: asset_core::AssetCategoryScope,
        hands: asset_model::FirstPersonArms,
        ctrl: FirstPersonGunAnimator,
        left: Option<FirstPersonGunAnimator>,
    ) -> Self {
        let weapon_mesh: String = model_name.into();
        Self {
            view_skeleton: None,
            left,
            controller: ctrl,
            hands,
            namespace,
            hands_index,
            gun_index,
            weapon_mesh,
        }
    }
}

// ---------------------------------------------------------------------------------------
// The frame tick
// ---------------------------------------------------------------------------------------

/// The events one authority tick brings; a tick already seen brings none.
pub fn cues_for_server_tick(
    state: &mut ViewmodelShowState,
    beat: ServerTruthSnapshot,
) -> Vec<HeldGunCue> {
    if state.latest_step_seen.replace(beat.advance) == Some(beat.advance) {
        return Vec::new();
    }
    state.prev_ads = Some(beat.frac_ads);
    map_authority_to_fpv(beat.cues)
}

pub fn tick_with_prediction(
    gun: &mut HeldGunLoadout,
    present: &mut ViewmodelShowState,
    sample: Option<ServerTruthSnapshot>,
    did_fire: bool,
    dt_seconds: f32,
) -> (HeldPoseReadout, Vec<String>) {
    let nothing_extra: &[HeldGunCue] = &[];
    let args = (sample, did_fire, nothing_extra);
    step_held_gun_with(gun, present, args.0, args.1, args.2, dt_seconds)
}

/// One frame of the held gun. A pending BO2 frame poses it through BO2's anim player;
/// otherwise the authority sample, a locally predicted shot and `extra_events` drive the
/// controllers. Returns the pose the gun ended in and the notetracks its clips passed.
pub fn step_held_gun_with(
    held: &mut HeldGunLoadout,
    present: &mut ViewmodelShowState,
    sample: Option<ServerTruthSnapshot>,
    shot_predicted: bool,
    also: &[HeldGunCue],
    secs_dt: f32,
) -> (HeldPoseReadout, Vec<String>) {
    if let Some(bo2_frame) = present.bo2.frame.take() {
        present.drop_gun_anim_memory();
        return pose_from_bo2(held, &mut present.bo2, &bo2_frame, secs_dt);
    }

    let mut events = sample.map_or_else(Vec::new, |s| cues_for_server_tick(present, s));
    let locally_asked = shot_predicted
        .then_some(HeldGunCue::Fire)
        .into_iter()
        .chain(also.iter().copied());
    for event in locally_asked {
        if events.contains(&event) {
            continue;
        }
        events.push(event);
    }

    let right = &mut held.controller;
    sample.inspect(|s| right.set_predicted_specialties(s.specialties));
    feed_events(right, &events);
    let right_refused = sample.is_some_and(|s| {
        let edge = present.vm_anim_change_right(s.weap_animation);
        !apply_edge(right, edge, s.magazine_rounds == Some(0))
    });
    if right_refused {
        present.forget_right_anim();
    }
    let StepOutcome { notes_hit: notifies } = right.step_on(secs_dt);
    let ads_frac = sample.map_or_else(|| present.prev_ads.unwrap_or(0.0), |s| s.frac_ads);
    right.apply_ads_overlay_frm(ads_frac);

    let left_hand_turn = sample.filter(|s| s.previous_gun_hand == 1);
    if let (Some(left_side), Some(s)) = (held.left.as_mut(), left_hand_turn) {
        left_side.set_predicted_specialties(s.specialties);
        let edge = present.vm_anim_change_left(s.weap_animation_secondary);
        if !apply_edge(left_side, edge, s.left_clip_ammo == Some(0)) {
            present.forget_left_anim();
        }
        left_side.step_on(secs_dt);
        let hip_fire = f32::default();
        left_side.apply_ads_overlay_frm(hip_fire);
    }
    let pose = sample_pose(&held.controller);
    (pose, notifies)
}

// ---------------------------------------------------------------------------------------
// Posing from BO2's anim player
// ---------------------------------------------------------------------------------------

/// The sprint loops whose wrap `t6_loop_seam` may smooth.
const RUN_LOOPS: [GunMoveRole; 2] =
    [GunMoveRole::SprintLoop, GunMoveRole::SprintLoopEmpty];

fn is_left_hand_slot(s: usize) -> bool {
    (slot::FIRE_LEFT..=slot::RELOAD_LEFT).contains(&s)
}

/// Replaces a run loop that jumps at its wrap far more than it moves inside the loop
/// (BO2's knives do) with the smoothed one. Each clip is looked at once.
fn smooth_run_loops(
    controller: &mut FirstPersonGunAnimator,
    seen: &mut [Option<Arc<SkeletalClip>>; 2],
    skeleton: Option<&ViewSkeleton>,
) {
    for (node, seen) in RUN_LOOPS.into_iter().zip(seen) {
        let Some(clip) = controller.weapon().reel(node).cloned() else {
            continue;
        };
        if seen.as_ref().is_some_and(|s| Arc::ptr_eq(s, &clip)) {
            continue;
        }
        *seen = Some(match smooth_loop_seam(&clip, skeleton) {
            Some(smooth) => {
                let smooth = Arc::new(smooth);
                vm_log_note(&format!("SEAM smoothed {}", clip.name));
                controller.replace_clip(node, Arc::clone(&smooth));
                smooth
            }
            None => clip,
        });
    }
}

/// One frame of BO2's anim player, posed onto the gun's clips (and the left gun's when
/// dual wielding). The player owns every weight and play position, ADS included.
fn pose_from_bo2(held: &mut HeldGunLoadout, bo2: &mut Bo2Fpv, frame: &VmFrame, dt_secs: f32) -> (HeldPoseReadout, Vec<String>) {
    let [right_loops, left_loops] = &mut bo2.run_loops;
    // The skeleton eases each arm of a single gun's smoothed loop; dual guns play theirs as BO2 has them.
    let skeleton = held.view_skeleton.as_ref().filter(|_| held.left.is_none());
    smooth_run_loops(&mut held.controller, right_loops, skeleton);
    if let Some(left) = held.left.as_mut() {
        smooth_run_loops(left, left_loops, None);
    }

    if bo2.set_for.as_deref() != Some(held.weapon_mesh.as_str()) {
        bo2.set = anim_set_of(&held.controller, held.left.as_ref());
        bo2.set_for = Some(held.weapon_mesh.clone());
    }

    bo2.ms_carry += dt_secs.max(0.0) * 1000.0;
    let whole_ms = bo2.ms_carry.floor();
    bo2.ms_carry -= whole_ms;
    let frame = VmFrame {
        step_duration_ms: whole_ms as i32,
        ..*frame
    };
    bo2.vm.frame(&frame, &bo2.table, &bo2.set);
    let pose = bo2.vm.pose();
    bo2.log_ms += whole_ms as i64;
    vm_log(bo2.log_ms, &frame, &pose);

    let right_pose = pose
        .iter()
        .filter(|p| !is_left_hand_slot(p.0))
        .map(|&(s, weight, time)| (slot::name(s), weight, time));
    let right_pose = idle_for_unnamed(&held.controller, right_pose);
    let right = held.controller.apply_bo2_pose(right_pose);
    for note in &right.notify_list {
        vm_log_note(&format!("TRACK {note}"));
    }
    if let Some(left) = held.left.as_mut() {
        let left_pose = pose
            .iter()
            .filter(|p| is_left_hand_slot(p.0))
            .map(|&(s, weight, time)| (slot::name(s).trim_end_matches("_left"), weight, time));
        let left_pose = idle_for_unnamed(left, left_pose);
        // The left gun's own notetracks stay silent.
        let _ = left.apply_bo2_pose(left_pose);
    }
    (sample_pose(&held.controller), right.notify_list)
}

/// BO2 fills every slot a gun leaves unnamed with the gun's idle (an empty idle on a gun
/// that has none, a sprint on a gun without sprint anims), so such a slot poses the idle
/// clip. Entries landing on idle together add their weights; the heaviest sets the play
/// position.
fn idle_for_unnamed<'a>(
    controller: &FirstPersonGunAnimator,
    pose: impl Iterator<Item = (&'a str, f32, f32)>,
) -> Vec<(&'a str, f32, f32)> {
    let has_clip =
        |name: &str| bo2_slot(name).is_some_and(|s| controller.weapon().reel(s).is_some());
    let mut out = Vec::<(&str, f32, f32)>::new();
    // Where idle sits in `out`, and the heaviest weight it has seen.
    let mut idle = None::<(usize, f32)>;
    for (name, weight, time) in pose {
        let name = if has_clip(name) { name } else { "idle" };
        if name != "idle" {
            out.push((name, weight, time));
            continue;
        }
        match idle.as_mut() {
            None => {
                idle = Some((out.len(), weight));
                out.push((name, weight, time));
            }
            Some((at, heaviest)) => {
                out[*at].1 += weight;
                if weight > *heaviest {
                    *heaviest = weight;
                    out[*at].2 = time;
                }
            }
        }
    }
    out
}

/// The gun's clips by BO2 slot: a slot is "named" when the gun has a clip for it. The
/// left-hand slots come from the left gun's matching clips.
fn anim_set_of(right: &FirstPersonGunAnimator, left: Option<&FirstPersonGunAnimator>) -> AnimSet {
    let clip_for = |controller: &FirstPersonGunAnimator, name: &str| {
        let clip = controller.weapon().reel(bo2_slot(name)?)?;
        Some(Clip {
            length_ms: clip.duration() * 1000.0,
            looping: clip.looping,
        })
    };
    let right_clips: [Option<Clip>; slot::COUNT] = std::array::from_fn(|s| {
        if s == 0 {
            None
        } else {
            clip_for(right, slot::name(s))
        }
    });
    let left_clips: [Option<Clip>; slot::COUNT] = std::array::from_fn(|s| {
        let left = left?;
        if s == slot::IDLE {
            clip_for(left, "idle")
        } else if is_left_hand_slot(s) {
            clip_for(left, slot::name(s).trim_end_matches("_left"))
        } else {
            None
        }
    });
    AnimSet::build(&right_clips, &left_clips)
}

// ---------------------------------------------------------------------------------------
// Test log (BO2MP_VMLOG=<file>)
// ---------------------------------------------------------------------------------------

fn vm_log_file() -> Option<&'static diag::LineFile> {
    static LOG: std::sync::OnceLock<Option<diag::LineFile>> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("BO2MP_VMLOG")?;
        diag::LineFile::create(std::path::Path::new(&path))
    })
    .as_ref()
}

/// Whether `BO2MP_VMLOG` is on, so a log line is worth building.
pub fn vm_log_enabled() -> bool {
    vm_log_file().is_some()
}

fn vm_log_line(line: &str) {
    let Some(file) = vm_log_file() else {
        return;
    };
    let wall_ms = std::time::UNIX_EPOCH
        .elapsed()
        .map_or(0, |since| since.as_millis());
    file.line(format!("{wall_ms} {line}"));
}

/// A line of its own in the log (the screenshot writer names the shot it captures, so
/// each picture pairs with the frame it shows).
pub fn vm_log_note(note: &str) {
    vm_log_line(&format!("NOTE {note}"));
}

/// One line per frame: the weapon fields, then each slot with weight and play position.
fn vm_log(ms: i64, frame: &VmFrame, pose: &[(usize, f32, f32)]) {
    if !vm_log_enabled() {
        return;
    }
    use std::fmt::Write as _;
    let mut line = String::with_capacity(160);
    let _ = write!(
        line,
        "{ms} w={} ws={} wa={} wt={} spd={:.0}",
        frame.weapon, frame.gun_phase, frame.gun_clip, frame.gun_timer_ms, frame.stride_velocity
    );
    if frame.mantle_duration_ms != 0 {
        let _ = write!(line, " md={}", frame.mantle_duration_ms);
    }
    for &(s, weight, time) in pose.iter().filter(|p| p.1 > 0.001) {
        let _ = write!(line, " {}:{weight:.2}:{time:.3}", slot::name(s));
    }
    vm_log_line(&line);
}

mod tests {
    #![cfg(test)]
    use super::*;
    use anim_bank::motion_clip::RootShift;
    use asset_core::AssetCategoryScope;
    use asset_game::GunClipSet;
    use weapon_t6::viewmodel::VmFacts;
    use weapon_t6::{anim, state};

    fn reel(caption: &str, frames: u16) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: caption.to_owned(),
            fps: 30.0,
            frame_total: frames,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        })
    }

    fn equipped(gun: GunClipSet) -> HeldGunLoadout {
        HeldGunLoadout::new(
            "viewmodel_mp7",
            assets::FpvMeshSlot::from_sequence(0),
            assets::FpvMeshSlot::from_sequence(1),
            AssetCategoryScope::T6,
            asset_model::FirstPersonArms::FromKit {
                namespace: AssetCategoryScope::T6,
                name: "hands".into(),
            },
            FirstPersonGunAnimator::new(gun),
            None,
        )
    }

    fn sample(beat: u32, cues: ServerGunCues) -> ServerTruthSnapshot {
        ServerTruthSnapshot {
            advance: beat,
            gun_state: 0,
            cues,
            is_sprinting: false,
            frac_ads: 0.25,
            weap_animation: anim::REST,
            weap_animation_secondary: anim::REST,
            previous_gun_hand: 0, specialties: [0; 2],
            magazine_rounds: None,
            left_clip_ammo: None,
        }
    }

    /// The MP7's reload as BO2 plays it: an 86-frame clip over the 1800 ms reload.
    #[test]
    fn a_bo2_frame_poses_the_gun_and_the_old_player_stands_aside() {
        let gun = GunClipSet::blank("mp7_mp")
            .with_clip(GunMoveRole::Idle, reel("idle", 45))
            .with_clip(GunMoveRole::Reload, reel("reload", 86));
        let mut eq = equipped(gun);
        let mut facts = VmFacts::default();
        facts.times[slot::RELOAD] = 1800;
        let mut table = VmTable::default();
        table.set(1, facts);
        let mut present = ViewmodelShowState::default();
        present.bo2.table = Arc::new(table);
        let frame = VmFrame {
            weapon: 1,
            model: 1,
            gun_clip: anim::start(anim::REST, anim::RELOAD),
            gun_phase: state::RELOADING,
            gun_timer_ms: 1800,
            ammo_in_clip: 10,
            ..VmFrame::default()
        };
        for _ in 0..30 {
            present.bo2.frame = Some(frame);
            step_held_gun_with(&mut eq, &mut present, None, false, &[], 0.016);
        }
        assert_eq!(eq.controller.animation_wt(GunMoveRole::Reload), 1.0);
        // 480 ms into the 1800 ms reload; the clip is 2867 ms long.
        let t = eq.controller.animation_when(GunMoveRole::Reload);
        assert!((t - 0.48 * 2.8667 / 1.8).abs() < 0.02, "reload at {t}");
        assert!(present.bo2.frame.is_none(), "a frame is used once");

        // With no BO2 frame the controller-driven path runs again.
        step_held_gun_with(&mut eq, &mut present, None, false, &[], 0.016);
        assert!(present.prev_vm_anim.is_none());
    }

    /// The MP7 has no empty idle: BO2 plays its idle there, so an empty MP7 still holds the
    /// gun in the hands.
    #[test]
    fn an_unnamed_slot_poses_the_idle_clip() {
        let gun = GunClipSet::blank("mp7_mp")
            .with_clip(GunMoveRole::Idle, reel("idle", 45))
            .with_clip(GunMoveRole::SprintLoop, reel("sprint", 20));
        let controller = FirstPersonGunAnimator::new(gun);
        let pose = idle_for_unnamed(
            &controller,
            [
                ("empty_idle", 0.5, 0.5),
                ("idle", 0.25, 0.1),
                ("sprint_loop", 0.25, 0.3),
            ]
            .into_iter(),
        );
        assert_eq!(pose, vec![("idle", 0.75, 0.5), ("sprint_loop", 0.25, 0.3)]);
    }

    #[test]
    fn the_shot_cursor_takes_the_first_identity_as_a_baseline() {
        let mut cursor = ShotStampTracker::default();
        let shot = |count, anim| own_shot_signature(1, 0, count, anim);
        assert!(!cursor.note_shot(shot(5, anim::ATTACK)), "baseline");
        assert!(!cursor.note_shot(shot(5, anim::ATTACK)), "same shot");
        assert!(cursor.note_shot(shot(6, anim::ATTACK)), "new count");
        assert!(
            cursor.note_shot(shot(6, anim::ATTACK | TOGGLE)),
            "restart on the same count"
        );
        assert!(
            !cursor.note_shot(own_shot_signature(2, 1, 6, anim::ATTACK | TOGGLE)),
            "life or hand alone"
        );
        assert_eq!(cursor.last().map(|i| i.lifespan_sequence), Some(2));
        cursor.clear();
        assert!(cursor.last().is_none());
        assert!(
            !cursor.note_shot(shot(9, anim::ATTACK)),
            "baseline again after a clear"
        );
    }

    #[test]
    fn present_events_map_to_controller_events() {
        let raise = |first| {
            Some(GunAnimEvent::Raise {
                initial: first,
                hasty: false,
            })
        };
        let table = [
            (HeldGunCue::Fire, Some(GunAnimEvent::Fire)),
            (HeldGunCue::TriggerUp, None),
            (HeldGunCue::LiftFirst, raise(true)),
            (HeldGunCue::Raise, raise(false)),
            (HeldGunCue::Reload, Some(GunAnimEvent::ReloadInfo { empty: false })),
            (HeldGunCue::DryReload, Some(GunAnimEvent::ReloadInfo { empty: true })),
            (HeldGunCue::ReloadKickoff, Some(GunAnimEvent::ReloadKickoff)),
            (HeldGunCue::ReloadEnd, Some(GunAnimEvent::ReloadEnd)),
            (HeldGunCue::Rechamber, Some(GunAnimEvent::Rechamber)),
            (HeldGunCue::Drop, Some(GunAnimEvent::Drop)),
            (HeldGunCue::SprintIn, Some(GunAnimEvent::SprintIn)),
            (HeldGunCue::SprintLoopInfo, Some(GunAnimEvent::SprintLoopData)),
            (HeldGunCue::SprintOut, Some(GunAnimEvent::SprintOut)),
        ];
        for (event, expected) in table {
            assert_eq!(to_vm_event(event), expected, "{event:?}");
        }
    }

    #[test]
    fn forgetting_one_hand_keeps_the_other() {
        let mut present = ViewmodelShowState::default();
        present.prev_vm_anim = Some(1);
        present.prev_vm_anim_left = Some(2);
        present.forget_right_anim();
        assert_eq!(
            (present.prev_vm_anim, present.prev_vm_anim_left),
            (None, Some(2))
        );
        present.prev_vm_anim = Some(1);
        present.forget_left_anim();
        assert_eq!(
            (present.prev_vm_anim, present.prev_vm_anim_left),
            (Some(1), None)
        );
    }

    #[test]
    fn only_the_cues_that_fired_bring_events() {
        let released = ServerGunCues {
            trigger_let_go: true,
            ..ServerGunCues::default()
        };
        assert_eq!(
            map_authority_to_fpv(released),
            [HeldGunCue::TriggerUp]
        );
        let accepted = ServerGunCues {
            fire_confirmed: true,
            ..ServerGunCues::default()
        };
        assert_eq!(map_authority_to_fpv(accepted), [HeldGunCue::Fire]);
    }

    #[test]
    fn a_weap_anim_edge_is_a_new_slot_or_a_restart() {
        let mut last = None;
        assert_eq!(weap_anim_edge(&mut last, anim::REST), VmAnimChange::Idle);
        assert_eq!(
            weap_anim_edge(&mut last, anim::REST),
            VmAnimChange::Unchanged
        );
        assert_eq!(
            weap_anim_edge(&mut last, anim::FORCE_IDLE),
            VmAnimChange::ForceIdle
        );
        let first = weap_anim_edge(&mut last, anim::RELOAD);
        assert!(matches!(first, VmAnimChange::DispatchInfo(_)));
        assert_eq!(
            weap_anim_edge(&mut last, anim::RELOAD),
            VmAnimChange::Unchanged
        );
        assert_eq!(
            weap_anim_edge(&mut last, anim::RELOAD | TOGGLE),
            first,
            "the toggle bit restarts it"
        );
    }

    #[test]
    fn an_authority_tick_is_used_once_and_remembers_the_aim() {
        let mut present = ViewmodelShowState::default();
        let shot = ServerGunCues {
            created: false, trigger_let_go: true,
            fire_confirmed: true,
        };
        assert_eq!(
            cues_for_server_tick(&mut present, sample(7, shot)),
            [HeldGunCue::Fire, HeldGunCue::TriggerUp]
        );
        assert!(cues_for_server_tick(&mut present, sample(7, shot)).is_empty());
        assert_eq!(present.prev_ads, Some(0.25));
        assert!(map_authority_to_fpv(ServerGunCues::default()).is_empty());
    }

    #[test]
    fn a_predicted_shot_and_an_authority_shot_are_one_fire() {
        let gun = GunClipSet::blank("mp7_mp")
            .with_clip(GunMoveRole::Idle, reel("idle", 45))
            .with_clip(GunMoveRole::Fire, reel("fire", 6));
        let mut eq = equipped(gun);
        let mut present = ViewmodelShowState::default();
        let cues = ServerGunCues {
            fire_confirmed: true,
            ..ServerGunCues::default()
        };
        let (pose, _) = tick_with_prediction(
            &mut eq,
            &mut present,
            Some(sample(1, cues)),
            true,
            0.01,
        );
        assert_eq!(pose.state, GunPhase::Firing);
        assert!(pose.shot_blend > 0.0);
    }

    #[test]
    fn clearing_the_present_state_keeps_only_the_facts_table() {
        let mut present = ViewmodelShowState::default();
        let mut table = VmTable::default();
        table.set(3, VmFacts::default());
        present.bo2.table = Arc::new(table);
        let kept = Arc::clone(&present.bo2.table);
        present.latest_step_seen = Some(4);
        present.prev_vm_anim = Some(2);
        present.local_shot.note_shot(own_shot_signature(1, 0, 3, 0));
        present.bo2.frame = Some(VmFrame::default());
        present.clear();
        assert!(present.latest_step_seen.is_none() && present.prev_vm_anim.is_none());
        assert!(present.bo2.frame.is_none());
        assert!(present.local_shot.last().is_none());
        assert!(Arc::ptr_eq(&present.bo2.table, &kept));
    }
}
