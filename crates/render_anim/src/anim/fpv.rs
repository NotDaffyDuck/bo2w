//! The first-person gun's per-frame driver: turns what the local player's state says
//! (authority cues, BO2's own weapon fields) into events and poses on the held gun's
//! [`ViewmodelController`]s, and keeps the cursors that tell a new event from a repeat.

use std::sync::Arc;

use asset_anim::AnimClip;
use asset_game::WeaponAnimSlot;
use weapon_t6::anim::TOGGLE;
use weapon_t6::asset_slots::{VmAnim, vm_anim};
use weapon_t6::viewmodel::{AnimSet, Clip, VmFrame, VmTable, Viewmodel, slot};

use crate::anim::t6_loop_seam::{ViewSkeleton, smooth_loop_seam};
use crate::anim::viewmodel_controller::bo2_slot;
use crate::{AdvanceResult, EventResult, ViewmodelController, ViewmodelEvent, WeaponState};

// ---------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentFpvEvent {
    Fire,
    ReleaseAttack,
    RaiseFirst,
    Raise,
    Reload,
    ReloadEmpty,
    ReloadStart,
    ReloadEnd,
    Rechamber,
    Drop,
    SprintIn,
    SprintLoop,
    SprintOut,
}

/// What the authority said happened this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityFpvCues {
    pub shot_accepted: bool,
    pub spawned: bool,
    pub attack_released: bool,
}

/// The pose the first-person gun ended the frame in.
#[derive(Clone, Debug, PartialEq)]
pub struct FpvPoseSample {
    pub state: WeaponState,
    pub clip_name: Option<String>,
    pub clip_time: f32,
    pub fire_weight: f32,
    pub idle_weight: f32,
}

/// The events an authority tick's cues call for: a shot, then a trigger release.
pub fn map_authority_to_fpv(cues: AuthorityFpvCues) -> Vec<PresentFpvEvent> {
    let mut events = Vec::new();
    if cues.shot_accepted {
        events.push(PresentFpvEvent::Fire);
    }
    if cues.attack_released {
        events.push(PresentFpvEvent::ReleaseAttack);
    }
    events
}

/// The controller event for a present event; releasing the trigger is not an event of
/// its own, the controller just settles.
fn controller_event(event: PresentFpvEvent) -> Option<ViewmodelEvent> {
    let raise = |first| ViewmodelEvent::Raise { first, quick: false };
    Some(match event {
        PresentFpvEvent::Fire => ViewmodelEvent::Fire,
        PresentFpvEvent::ReleaseAttack => return None,
        PresentFpvEvent::RaiseFirst => raise(true),
        PresentFpvEvent::Raise => raise(false),
        PresentFpvEvent::Reload => ViewmodelEvent::Reload { empty: false },
        PresentFpvEvent::ReloadEmpty => ViewmodelEvent::Reload { empty: true },
        PresentFpvEvent::ReloadStart => ViewmodelEvent::ReloadStart,
        PresentFpvEvent::ReloadEnd => ViewmodelEvent::ReloadEnd,
        PresentFpvEvent::Rechamber => ViewmodelEvent::Rechamber,
        PresentFpvEvent::Drop => ViewmodelEvent::Drop,
        PresentFpvEvent::SprintIn => ViewmodelEvent::SprintIn,
        PresentFpvEvent::SprintLoop => ViewmodelEvent::SprintLoop,
        PresentFpvEvent::SprintOut => ViewmodelEvent::SprintOut,
    })
}

fn play_events(controller: &mut ViewmodelController, events: &[PresentFpvEvent]) {
    for &event in events {
        let Some(vm_event) = controller_event(event) else {
            controller.settle_fire_to_idle();
            continue;
        };
        match controller.handle(vm_event) {
            EventResult::Started(_) => {}
            EventResult::IgnoredState(state) => {
                diag::warn!(Fpv, "fpv: {event:?} ignored in state {state:?}");
            }
            EventResult::IgnoredMissingClip(missing) => {
                diag::warn!(Fpv, "fpv: {event:?} ignored: no clip for {missing:?}");
            }
        }
    }
}

/// Whether BO2 viewmodel anim `anim` is a shot.
pub fn is_predicted_fire_weap_anim(anim: i32) -> bool {
    weapon_t6::asset_slots::is_fire_anim(anim)
}

/// The pose of whichever clip weighs most right now, with the fire and idle weights.
pub fn sample_pose(controller: &ViewmodelController) -> FpvPoseSample {
    let heaviest = controller.active_anims().max_by(|a, b| a.weight.total_cmp(&b.weight));
    FpvPoseSample {
        state: controller.state(),
        clip_name: heaviest.as_ref().map(|a| a.clip.name.clone()),
        clip_time: heaviest.as_ref().map_or(0.0, |a| a.time),
        fire_weight: controller.animation_weight(WeaponAnimSlot::Fire),
        idle_weight: controller.animation_weight(WeaponAnimSlot::Idle),
    }
}

// ---------------------------------------------------------------------------------------
// Authority samples and the shot cursor
// ---------------------------------------------------------------------------------------

/// The local player's state for one authority tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FpvAuthoritySample {
    pub tick: u32,
    pub weaponstate: i32,
    pub cues: AuthorityFpvCues,
    pub sprinting: bool,
    pub ads_frac: f32,
    /// The viewmodel anim of the right hand, and of the left when dual wielding.
    pub weap_anim: i32,
    pub weap_anim_secondary: i32,
    pub last_weapon_hand: i32,
    pub perks0: u32,
    pub clip_ammo: Option<i32>,
    pub left_clip_ammo: Option<i32>,
}

/// What names one local shot: the same identity twice is the same shot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalShotIdentity {
    pub life_sequence: u32,
    pub hand: u8,
    pub weapon_shot_count: i32,
    /// The anim's toggle bit, which flips to restart a shot animation in place.
    pub weap_anim_restart: bool,
}

pub fn local_shot_identity(
    life_sequence: u32,
    hand: u8,
    weapon_shot_count: i32,
    weap_anim: i32,
) -> LocalShotIdentity {
    LocalShotIdentity {
        life_sequence,
        hand,
        weapon_shot_count,
        weap_anim_restart: weap_anim & TOGGLE != 0,
    }
}

/// Tells new local shots from repeats of the last identity seen.
#[derive(Clone, Debug, Default)]
pub struct LocalShotCursor {
    last: Option<LocalShotIdentity>,
}

impl LocalShotCursor {
    pub fn clear(&mut self) {
        self.last = None;
    }

    pub fn last(&self) -> Option<LocalShotIdentity> {
        self.last
    }

    /// Whether `identity` is a new shot. The first identity ever seen only sets the
    /// baseline. After that a changed shot count is a shot, and so is a flipped restart
    /// bit on the same count; a change of life or hand alone is not.
    pub fn observe(&mut self, identity: LocalShotIdentity) -> bool {
        let Some(prev) = self.last.replace(identity) else {
            return false;
        };
        let new_count = identity.weapon_shot_count != prev.weapon_shot_count;
        let restarted = identity.weap_anim_restart != prev.weap_anim_restart
            && identity.weapon_shot_count == prev.weapon_shot_count;
        new_count || restarted
    }
}

// ---------------------------------------------------------------------------------------
// Viewmodel anim edges
// ---------------------------------------------------------------------------------------

/// What a change of the viewmodel anim asks of the old viewmodel player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeapAnimEdge {
    Unchanged,
    Idle,
    ForceIdle,
    /// Play the clip node with this index.
    Dispatch(usize),
}

/// Compares BO2 viewmodel anim `raw` with the one before it. Anims that play the same
/// slot count as one, so only a new slot or a restart (the toggle bit) is an edge.
fn weap_anim_edge(previous: &mut Option<i32>, raw: i32) -> WeapAnimEdge {
    let vm = vm_anim(raw);
    let kind = match vm {
        VmAnim::Idle => 0,
        VmAnim::ForceIdle => 1,
        VmAnim::Slot(slot) => 2 + slot as i32,
    };
    let key = kind | (raw & TOGGLE);
    if previous.replace(key) == Some(key) {
        return WeapAnimEdge::Unchanged;
    }
    match vm {
        VmAnim::Idle => WeapAnimEdge::Idle,
        VmAnim::ForceIdle => WeapAnimEdge::ForceIdle,
        VmAnim::Slot(slot) => WeapAnimEdge::Dispatch(slot),
    }
}

/// Plays `edge` on one hand's controller. `false` when an idle edge was refused (a
/// non-looping clip is still running), so the caller asks again next frame.
fn play_edge(controller: &mut ViewmodelController, edge: WeapAnimEdge, empty_mag: bool) -> bool {
    match edge {
        WeapAnimEdge::Unchanged => {}
        WeapAnimEdge::Idle => return controller.apply_idle_weap_anim(empty_mag),
        WeapAnimEdge::ForceIdle => controller.apply_force_idle_weap_anim(empty_mag),
        WeapAnimEdge::Dispatch(node) => match controller.dispatch_sz_xanim_index(node) {
            EventResult::Started(_) => {}
            EventResult::IgnoredState(state) => {
                diag::warn!(Fpv, "fpv: szXAnims[{node}] ignored in state {state:?}");
            }
            EventResult::IgnoredMissingClip(missing) => {
                diag::warn!(Fpv, "fpv: szXAnims[{node}] ({missing:?}) missing, not started");
            }
        },
    }
    true
}

// ---------------------------------------------------------------------------------------
// Present state
// ---------------------------------------------------------------------------------------

/// What the driver remembers between frames.
#[derive(Clone, Debug, Default)]
pub struct FpvPresentState {
    pub last_tick: Option<u32>,
    pub last_ads_frac: Option<f32>,
    pub last_weap_anim: Option<i32>,
    pub last_weap_anim_secondary: Option<i32>,
    pub local_shot: LocalShotCursor,
    pub bo2: Bo2Fpv,
}

impl FpvPresentState {
    /// Forgets everything but the per-weapon facts table.
    pub fn clear(&mut self) {
        *self = Self {
            bo2: Bo2Fpv { table: Arc::clone(&self.bo2.table), ..Bo2Fpv::default() },
            ..Self::default()
        };
    }

    pub fn forget_weap_anim(&mut self) {
        self.last_weap_anim = None;
        self.last_weap_anim_secondary = None;
    }

    pub fn observe_weap_anim_edge(&mut self, raw: i32) -> WeapAnimEdge {
        weap_anim_edge(&mut self.last_weap_anim, raw)
    }

    pub fn observe_weap_anim_secondary_edge(&mut self, raw: i32) -> WeapAnimEdge {
        weap_anim_edge(&mut self.last_weap_anim_secondary, raw)
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
    run_loops: [[Option<Arc<AnimClip>>; 2]; 2],
}

/// The held gun's controllers and what the gun is made of.
#[derive(Debug)]
pub struct EquippedFpv {
    pub gun_xmodel: String,
    pub gun_index: assets::FpvMeshIndex,
    pub hands_index: assets::FpvMeshIndex,
    pub namespace: asset_core::AssetNamespace,
    pub hands: asset_model::FpvHands,
    pub controller: ViewmodelController,
    /// The left gun when dual wielding.
    pub left: Option<ViewmodelController>,
    /// The arms-and-gun skeleton, so each arm can ease back on its own half of a smoothed
    /// run loop with what it holds kept in its hand (the ballistic knife in the left fist).
    pub view_skeleton: Option<ViewSkeleton>,
}

impl EquippedFpv {
    pub fn new(
        gun_xmodel: impl Into<String>,
        gun_index: assets::FpvMeshIndex,
        hands_index: assets::FpvMeshIndex,
        namespace: asset_core::AssetNamespace,
        hands: asset_model::FpvHands,
        controller: ViewmodelController,
        left: Option<ViewmodelController>,
    ) -> Self {
        Self {
            gun_xmodel: gun_xmodel.into(),
            gun_index,
            hands_index,
            namespace,
            hands,
            controller,
            left,
            view_skeleton: None,
        }
    }
}

// ---------------------------------------------------------------------------------------
// The frame tick
// ---------------------------------------------------------------------------------------

/// The events one authority tick brings; a tick already seen brings none.
pub fn events_for_authority_tick(
    present: &mut FpvPresentState,
    sample: FpvAuthoritySample,
) -> Vec<PresentFpvEvent> {
    if present.last_tick == Some(sample.tick) {
        return Vec::new();
    }
    present.last_tick = Some(sample.tick);
    present.last_ads_frac = Some(sample.ads_frac);
    map_authority_to_fpv(sample.cues)
}

pub fn tick_equipped_fpv_with_predicted_fire(
    equipped: &mut EquippedFpv,
    present: &mut FpvPresentState,
    sample: Option<FpvAuthoritySample>,
    predicted_local_fire: bool,
    dt_secs: f32,
) -> (FpvPoseSample, Vec<String>) {
    tick_equipped_fpv_with_extra_events(equipped, present, sample, predicted_local_fire, &[], dt_secs)
}

/// One frame of the held gun. A pending BO2 frame poses it through BO2's anim player;
/// otherwise the authority sample, a locally predicted shot and `extra_events` drive the
/// controllers. Returns the pose the gun ended in and the notetracks its clips passed.
pub fn tick_equipped_fpv_with_extra_events(
    equipped: &mut EquippedFpv,
    present: &mut FpvPresentState,
    sample: Option<FpvAuthoritySample>,
    predicted_local_fire: bool,
    extra_events: &[PresentFpvEvent],
    dt_secs: f32,
) -> (FpvPoseSample, Vec<String>) {
    if let Some(frame) = present.bo2.frame.take() {
        present.forget_weap_anim();
        return tick_bo2(equipped, &mut present.bo2, &frame, dt_secs);
    }

    let mut events = sample.map_or_else(Vec::new, |s| events_for_authority_tick(present, s));
    let wanted = predicted_local_fire
        .then_some(PresentFpvEvent::Fire)
        .into_iter()
        .chain(extra_events.iter().copied());
    for event in wanted {
        if !events.contains(&event) {
            events.push(event);
        }
    }

    let right = &mut equipped.controller;
    if let Some(sample) = sample {
        right.set_predicted_perks(sample.perks0);
    }
    play_events(right, &events);
    if let Some(sample) = sample {
        let edge = present.observe_weap_anim_edge(sample.weap_anim);
        if !play_edge(right, edge, sample.clip_ammo == Some(0)) {
            present.last_weap_anim = None;
        }
    }
    let AdvanceResult { notifies } = right.advance(dt_secs);
    let ads_frac = sample.map(|s| s.ads_frac).or(present.last_ads_frac).unwrap_or(0.0);
    right.apply_ads_overlay_frame(ads_frac);

    if let (Some(left), Some(sample)) = (equipped.left.as_mut(), sample)
        && sample.last_weapon_hand == 1
    {
        left.set_predicted_perks(sample.perks0);
        let edge = present.observe_weap_anim_secondary_edge(sample.weap_anim_secondary);
        if !play_edge(left, edge, sample.left_clip_ammo == Some(0)) {
            present.last_weap_anim_secondary = None;
        }
        left.advance(dt_secs);
        left.apply_ads_overlay_frame(0.0);
    }
    (sample_pose(&equipped.controller), notifies)
}

// ---------------------------------------------------------------------------------------
// Posing from BO2's anim player
// ---------------------------------------------------------------------------------------

/// The sprint loops whose wrap `t6_loop_seam` may smooth.
const RUN_LOOPS: [WeaponAnimSlot; 2] = [WeaponAnimSlot::SprintLoop, WeaponAnimSlot::SprintLoopEmpty];

fn is_left_slot(s: usize) -> bool {
    (slot::FIRE_LEFT..=slot::RELOAD_LEFT).contains(&s)
}

/// Replaces a run loop that jumps at its wrap far more than it moves inside the loop
/// (BO2's knives do) with the smoothed one. Each clip is looked at once.
fn smooth_run_loops(
    controller: &mut ViewmodelController,
    seen: &mut [Option<Arc<AnimClip>>; 2],
    skeleton: Option<&ViewSkeleton>,
) {
    for (node, seen) in RUN_LOOPS.into_iter().zip(seen) {
        let Some(clip) = controller.weapon().clip(node).cloned() else {
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
fn tick_bo2(
    equipped: &mut EquippedFpv,
    bo2: &mut Bo2Fpv,
    frame: &VmFrame,
    dt_secs: f32,
) -> (FpvPoseSample, Vec<String>) {
    let [right_loops, left_loops] = &mut bo2.run_loops;
    // The skeleton eases each arm of a single gun's smoothed loop; dual guns play theirs as BO2 has them.
    let skeleton = equipped.view_skeleton.as_ref().filter(|_| equipped.left.is_none());
    smooth_run_loops(&mut equipped.controller, right_loops, skeleton);
    if let Some(left) = equipped.left.as_mut() {
        smooth_run_loops(left, left_loops, None);
    }

    if bo2.set_for.as_deref() != Some(equipped.gun_xmodel.as_str()) {
        bo2.set = anim_set_of(&equipped.controller, equipped.left.as_ref());
        bo2.set_for = Some(equipped.gun_xmodel.clone());
    }

    bo2.ms_carry += dt_secs.max(0.0) * 1000.0;
    let whole_ms = bo2.ms_carry.floor();
    bo2.ms_carry -= whole_ms;
    let frame = VmFrame { frametime_ms: whole_ms as i32, ..*frame };
    bo2.vm.frame(&frame, &bo2.table, &bo2.set);
    let pose = bo2.vm.pose();
    bo2.log_ms += whole_ms as i64;
    vm_log(bo2.log_ms, &frame, &pose);

    let right_pose = pose
        .iter()
        .filter(|p| !is_left_slot(p.0))
        .map(|&(s, weight, time)| (slot::name(s), weight, time));
    let right_pose = idle_for_unnamed(&equipped.controller, right_pose);
    let right = equipped.controller.apply_bo2_pose(right_pose);
    for note in &right.notifies {
        vm_log_note(&format!("TRACK {note}"));
    }
    if let Some(left) = equipped.left.as_mut() {
        let left_pose = pose
            .iter()
            .filter(|p| is_left_slot(p.0))
            .map(|&(s, weight, time)| (slot::name(s).trim_end_matches("_left"), weight, time));
        let left_pose = idle_for_unnamed(left, left_pose);
        // The left gun's own notetracks stay silent.
        let _ = left.apply_bo2_pose(left_pose);
    }
    (sample_pose(&equipped.controller), right.notifies)
}

/// BO2 fills every slot a gun leaves unnamed with the gun's idle (an empty idle on a gun
/// that has none, a sprint on a gun without sprint anims), so such a slot poses the idle
/// clip. Entries landing on idle together add their weights; the heaviest sets the play
/// position.
fn idle_for_unnamed<'a>(
    controller: &ViewmodelController,
    pose: impl Iterator<Item = (&'a str, f32, f32)>,
) -> Vec<(&'a str, f32, f32)> {
    let has_clip = |name: &str| bo2_slot(name).is_some_and(|s| controller.weapon().clip(s).is_some());
    let mut out: Vec<(&str, f32, f32)> = Vec::new();
    // Where idle sits in `out`, and the heaviest weight it has seen.
    let mut idle: Option<(usize, f32)> = None;
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
fn anim_set_of(right: &ViewmodelController, left: Option<&ViewmodelController>) -> AnimSet {
    let clip_for = |controller: &ViewmodelController, name: &str| {
        let clip = controller.weapon().clip(bo2_slot(name)?)?;
        Some(Clip { length_ms: clip.duration() * 1000.0, looping: clip.looping })
    };
    let right_clips: [Option<Clip>; slot::COUNT] =
        std::array::from_fn(|s| if s == 0 { None } else { clip_for(right, slot::name(s)) });
    let left_clips: [Option<Clip>; slot::COUNT] = std::array::from_fn(|s| {
        let left = left?;
        if s == slot::IDLE {
            clip_for(left, "idle")
        } else if is_left_slot(s) {
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
    let wall_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
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
    let mut line = format!(
        "{ms} w={} ws={} wa={} wt={} spd={:.0}",
        frame.weapon, frame.weaponstate, frame.weap_anim, frame.weapon_time, frame.move_speed
    );
    if frame.mantle_duration_ms != 0 {
        line.push_str(&format!(" md={}", frame.mantle_duration_ms));
    }
    for &(s, weight, time) in pose.iter().filter(|p| p.1 > 0.001) {
        line.push_str(&format!(" {}:{weight:.2}:{time:.3}", slot::name(s)));
    }
    vm_log_line(&line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_anim::xanim_clip::Translation;
    use asset_core::AssetNamespace;
    use asset_game::WeaponAnimations;
    use weapon_t6::viewmodel::VmFacts;
    use weapon_t6::{anim, state};

    fn clip(name: &str, frames: u16) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: name.to_owned(),
            framerate: 30.0,
            numframes: frames,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        })
    }

    fn equipped(gun: WeaponAnimations) -> EquippedFpv {
        EquippedFpv::new(
            "viewmodel_mp7",
            assets::FpvMeshIndex::from_order(0),
            assets::FpvMeshIndex::from_order(1),
            AssetNamespace::T6,
            asset_model::FpvHands::FromKit { namespace: AssetNamespace::T6, name: "hands".into() },
            ViewmodelController::new(gun),
            None,
        )
    }

    fn sample(tick: u32, cues: AuthorityFpvCues) -> FpvAuthoritySample {
        FpvAuthoritySample {
            tick,
            weaponstate: 0,
            cues,
            sprinting: false,
            ads_frac: 0.25,
            weap_anim: anim::IDLE,
            weap_anim_secondary: anim::IDLE,
            last_weapon_hand: 0,
            perks0: 0,
            clip_ammo: None,
            left_clip_ammo: None,
        }
    }

    /// The MP7's reload as BO2 plays it: an 86-frame clip over the 1800 ms reload.
    #[test]
    fn a_bo2_frame_poses_the_gun_and_the_old_player_stands_aside() {
        let gun = WeaponAnimations::empty("mp7_mp")
            .with_clip(WeaponAnimSlot::Idle, clip("idle", 45))
            .with_clip(WeaponAnimSlot::Reload, clip("reload", 86));
        let mut eq = equipped(gun);
        let mut facts = VmFacts::default();
        facts.times[slot::RELOAD] = 1800;
        let mut table = VmTable::default();
        table.set(1, facts);
        let mut present = FpvPresentState::default();
        present.bo2.table = Arc::new(table);
        let frame = VmFrame {
            weapon: 1,
            model: 1,
            weap_anim: anim::start(anim::IDLE, anim::RELOAD),
            weaponstate: state::RELOADING,
            weapon_time: 1800,
            ammo_in_clip: 10,
            ..VmFrame::default()
        };
        for _ in 0..30 {
            present.bo2.frame = Some(frame);
            tick_equipped_fpv_with_extra_events(&mut eq, &mut present, None, false, &[], 0.016);
        }
        assert_eq!(eq.controller.animation_weight(WeaponAnimSlot::Reload), 1.0);
        // 480 ms into the 1800 ms reload; the clip is 2867 ms long.
        let t = eq.controller.animation_time(WeaponAnimSlot::Reload);
        assert!((t - 0.48 * 2.8667 / 1.8).abs() < 0.02, "reload at {t}");
        assert!(present.bo2.frame.is_none(), "a frame is used once");

        // With no BO2 frame the controller-driven path runs again.
        tick_equipped_fpv_with_extra_events(&mut eq, &mut present, None, false, &[], 0.016);
        assert!(present.last_weap_anim.is_none());
    }

    /// The MP7 has no empty idle: BO2 plays its idle there, so an empty MP7 still holds the
    /// gun in the hands.
    #[test]
    fn an_unnamed_slot_poses_the_idle_clip() {
        let gun = WeaponAnimations::empty("mp7_mp")
            .with_clip(WeaponAnimSlot::Idle, clip("idle", 45))
            .with_clip(WeaponAnimSlot::SprintLoop, clip("sprint", 20));
        let controller = ViewmodelController::new(gun);
        let pose = idle_for_unnamed(
            &controller,
            [("empty_idle", 0.5, 0.5), ("idle", 0.25, 0.1), ("sprint_loop", 0.25, 0.3)].into_iter(),
        );
        assert_eq!(pose, vec![("idle", 0.75, 0.5), ("sprint_loop", 0.25, 0.3)]);
    }

    #[test]
    fn the_shot_cursor_takes_the_first_identity_as_a_baseline() {
        let mut cursor = LocalShotCursor::default();
        let shot = |count, anim| local_shot_identity(1, 0, count, anim);
        assert!(!cursor.observe(shot(5, anim::ATTACK)), "baseline");
        assert!(!cursor.observe(shot(5, anim::ATTACK)), "same shot");
        assert!(cursor.observe(shot(6, anim::ATTACK)), "new count");
        assert!(cursor.observe(shot(6, anim::ATTACK | TOGGLE)), "restart on the same count");
        assert!(!cursor.observe(local_shot_identity(2, 1, 6, anim::ATTACK | TOGGLE)), "life or hand alone");
        assert_eq!(cursor.last().map(|i| i.life_sequence), Some(2));
        cursor.clear();
        assert!(cursor.last().is_none());
        assert!(!cursor.observe(shot(9, anim::ATTACK)), "baseline again after a clear");
    }

    #[test]
    fn a_weap_anim_edge_is_a_new_slot_or_a_restart() {
        let mut last = None;
        assert_eq!(weap_anim_edge(&mut last, anim::IDLE), WeapAnimEdge::Idle);
        assert_eq!(weap_anim_edge(&mut last, anim::IDLE), WeapAnimEdge::Unchanged);
        assert_eq!(weap_anim_edge(&mut last, anim::FORCE_IDLE), WeapAnimEdge::ForceIdle);
        let first = weap_anim_edge(&mut last, anim::RELOAD);
        assert!(matches!(first, WeapAnimEdge::Dispatch(_)));
        assert_eq!(weap_anim_edge(&mut last, anim::RELOAD), WeapAnimEdge::Unchanged);
        assert_eq!(weap_anim_edge(&mut last, anim::RELOAD | TOGGLE), first, "the toggle bit restarts it");
    }

    #[test]
    fn an_authority_tick_is_used_once_and_remembers_the_aim() {
        let mut present = FpvPresentState::default();
        let shot = AuthorityFpvCues { shot_accepted: true, attack_released: true, spawned: false };
        assert_eq!(
            events_for_authority_tick(&mut present, sample(7, shot)),
            [PresentFpvEvent::Fire, PresentFpvEvent::ReleaseAttack]
        );
        assert!(events_for_authority_tick(&mut present, sample(7, shot)).is_empty());
        assert_eq!(present.last_ads_frac, Some(0.25));
        assert!(map_authority_to_fpv(AuthorityFpvCues::default()).is_empty());
    }

    #[test]
    fn a_predicted_shot_and_an_authority_shot_are_one_fire() {
        let gun = WeaponAnimations::empty("mp7_mp")
            .with_clip(WeaponAnimSlot::Idle, clip("idle", 45))
            .with_clip(WeaponAnimSlot::Fire, clip("fire", 6));
        let mut eq = equipped(gun);
        let mut present = FpvPresentState::default();
        let cues = AuthorityFpvCues { shot_accepted: true, ..AuthorityFpvCues::default() };
        let (pose, _) = tick_equipped_fpv_with_predicted_fire(
            &mut eq,
            &mut present,
            Some(sample(1, cues)),
            true,
            0.01,
        );
        assert_eq!(pose.state, WeaponState::Firing);
        assert!(pose.fire_weight > 0.0);
    }

    #[test]
    fn clearing_the_present_state_keeps_only_the_facts_table() {
        let mut present = FpvPresentState::default();
        let mut table = VmTable::default();
        table.set(3, VmFacts::default());
        present.bo2.table = Arc::new(table);
        let kept = Arc::clone(&present.bo2.table);
        present.last_tick = Some(4);
        present.last_weap_anim = Some(2);
        present.bo2.frame = Some(VmFrame::default());
        present.clear();
        assert!(present.last_tick.is_none() && present.last_weap_anim.is_none());
        assert!(present.bo2.frame.is_none());
        assert!(Arc::ptr_eq(&present.bo2.table, &kept));
    }
}
