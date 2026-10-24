use std::sync::Arc;

use asset_game::WeaponAnimSlot;
use weapon_t6::viewmodel::{AnimSet, Clip, VmFrame, VmTable, Viewmodel, slot};

use crate::anim::viewmodel_controller::bo2_slot;
use crate::{EventResult, ViewmodelController, ViewmodelEvent, WeaponState};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityFpvCues {
    pub shot_accepted: bool,
    pub spawned: bool,
    pub attack_released: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FpvPoseSample {
    pub state: WeaponState,

    pub clip_name: Option<String>,
    pub clip_time: f32,
    pub fire_weight: f32,
    pub idle_weight: f32,
}

pub fn map_authority_to_fpv(cues: AuthorityFpvCues) -> Vec<PresentFpvEvent> {
    let mut out = Vec::new();

    if cues.shot_accepted {
        out.push(PresentFpvEvent::Fire);
    }
    if cues.attack_released {
        out.push(PresentFpvEvent::ReleaseAttack);
    }

    out
}

fn viewmodel_event_for(event: PresentFpvEvent) -> Option<ViewmodelEvent> {
    Some(match event {
        PresentFpvEvent::Fire => ViewmodelEvent::Fire,
        PresentFpvEvent::ReleaseAttack => return None,
        PresentFpvEvent::RaiseFirst => ViewmodelEvent::Raise {
            first: true,
            quick: false,
        },
        PresentFpvEvent::Raise => ViewmodelEvent::Raise {
            first: false,
            quick: false,
        },
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

fn apply_present_events(controller: &mut ViewmodelController, events: &[PresentFpvEvent]) {
    for event in events {
        match viewmodel_event_for(*event) {
            None => controller.settle_fire_to_idle(),
            Some(vm) => report_fpv_event_result(*event, controller.handle(vm)),
        }
    }
}

pub fn is_predicted_fire_weap_anim(masked: u32) -> bool {
    use weapon_iw4::weap_anim_event as event_id;
    matches!(
        masked,
        event_id::FIRE
            | event_id::LASTSHOT
            | event_id::ADS_FIRE
            | event_id::ADS_LASTSHOT
            | event_id::FIRE_INTRO
            | event_id::ADS_FIRE_INTRO
    )
}

fn report_dispatch_slot(slot: usize, result: EventResult) {
    match result {
        EventResult::Started(_) => {}
        EventResult::IgnoredState(state) => {
            diag::warn!(Fpv, "fpv: szXAnims[{slot}] ignored in state {state:?}");
        }
        EventResult::IgnoredMissingClip(named) => {
            diag::warn!(
                Fpv,
                "fpv: szXAnims[{slot}] ({named:?}) missing — not started"
            );
        }
    }
}

fn report_fpv_event_result(event: PresentFpvEvent, result: EventResult) {
    match result {
        EventResult::Started(_) => {}
        EventResult::IgnoredState(state) => {
            diag::warn!(Fpv, "fpv: {event:?} ignored in state {state:?}");
        }
        EventResult::IgnoredMissingClip(slot) => {
            diag::warn!(Fpv, "fpv: {event:?} ignored — szXAnims[{slot:?}] missing");
        }
    }
}

pub fn sample_pose(controller: &ViewmodelController) -> FpvPoseSample {
    let active = controller.active_anims().max_by(|a, b| {
        a.weight
            .partial_cmp(&b.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    FpvPoseSample {
        state: controller.state(),
        clip_name: active.map(|a| a.clip.name.clone()),
        clip_time: active.map(|a| a.time).unwrap_or(0.0),
        fire_weight: controller.animation_weight(WeaponAnimSlot::Fire),
        idle_weight: controller.animation_weight(WeaponAnimSlot::Idle),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FpvAuthoritySample {
    pub tick: u32,
    pub weaponstate: i32,
    pub cues: AuthorityFpvCues,

    pub sprinting: bool,

    pub ads_frac: f32,

    pub weap_anim: i32,

    pub weap_anim_secondary: i32,

    pub last_weapon_hand: i32,

    pub perks0: u32,

    pub clip_ammo: Option<i32>,

    pub left_clip_ammo: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalShotIdentity {
    pub life_sequence: u32,
    pub hand: u8,
    pub weapon_shot_count: i32,

    pub weap_anim_restart: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LocalShotCursor {
    last: Option<LocalShotIdentity>,
}

impl LocalShotCursor {
    pub fn clear(&mut self) {
        self.last = None;
    }

    pub fn observe(&mut self, identity: LocalShotIdentity) -> bool {
        match self.last {
            None => {
                self.last = Some(identity);
                false
            }
            Some(prev) if prev == identity => false,
            Some(prev) => {
                let count_edge = identity.weapon_shot_count != prev.weapon_shot_count;
                let restart_edge = identity.weap_anim_restart != prev.weap_anim_restart
                    && identity.weapon_shot_count == prev.weapon_shot_count;
                self.last = Some(identity);
                count_edge || restart_edge
            }
        }
    }

    pub fn last(&self) -> Option<LocalShotIdentity> {
        self.last
    }
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
        weap_anim_restart: (weap_anim as u32 & 0x200) != 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeapAnimEdge {
    Unchanged,

    Idle,

    ForceIdle,

    Dispatch(usize),
}

#[derive(Clone, Debug, Default)]
pub struct FpvPresentState {
    pub last_tick: Option<u32>,

    pub last_ads_frac: Option<f32>,

    pub last_weap_anim: Option<i32>,

    pub last_weap_anim_secondary: Option<i32>,

    pub local_shot: LocalShotCursor,

    /// bo2 gun brain: Black Ops II's anim player and what it reads.
    pub bo2: Bo2Fpv,
}

/// bo2 gun brain: Black Ops II's first-person anim player (`weapon_t6::viewmodel`), kept
/// across weapon swaps like BO2's own.
#[derive(Clone, Debug, Default)]
pub struct Bo2Fpv {
    /// Black Ops II's own weapon fields for the coming frame, set by
    /// whoever reads the local player each frame. While set, BO2's anim player poses the gun
    /// (the frame time is filled in here); each frame uses it once.
    pub frame: Option<VmFrame>,
    pub vm: Viewmodel,
    /// BO2's per-weapon viewmodel facts, by this game's weapon numbers.
    pub table: Arc<VmTable>,
    /// The held gun's clips by BO2 slot, and the gun model they were built for.
    set: AnimSet,
    set_for: Option<String>,
    /// Frame time left over below a whole millisecond (BO2 counts whole ms).
    ms_carry: f32,
    /// Whole ms played so far, for the BO2MP_VMLOG test log.
    log_ms: i64,
}

impl FpvPresentState {
    pub fn clear(&mut self) {
        self.last_tick = None;
        self.last_ads_frac = None;
        self.last_weap_anim = None;
        self.last_weap_anim_secondary = None;
        self.local_shot.clear();
        self.bo2 = Bo2Fpv {
            table: Arc::clone(&self.bo2.table),
            ..Bo2Fpv::default()
        };
    }

    pub fn forget_weap_anim(&mut self) {
        self.last_weap_anim = None;
        self.last_weap_anim_secondary = None;
    }

    pub fn observe_weap_anim_edge(&mut self, raw: i32) -> WeapAnimEdge {
        observe_weap_anim_edge_on(&mut self.last_weap_anim, raw)
    }

    pub fn observe_weap_anim_secondary_edge(&mut self, raw: i32) -> WeapAnimEdge {
        observe_weap_anim_edge_on(&mut self.last_weap_anim_secondary, raw)
    }
}

#[derive(Debug)]
pub struct EquippedFpv {
    pub gun_xmodel: String,
    pub gun_index: assets::FpvMeshIndex,
    pub hands_index: assets::FpvMeshIndex,
    pub namespace: asset_core::AssetNamespace,
    pub hands: asset_model::FpvHands,
    pub controller: ViewmodelController,

    pub left: Option<ViewmodelController>,
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
        }
    }
}

pub fn events_for_authority_tick(
    present: &mut FpvPresentState,
    sample: FpvAuthoritySample,
) -> Vec<PresentFpvEvent> {
    if present.last_tick == Some(sample.tick) {
        return Vec::new();
    }
    let events = map_authority_to_fpv(sample.cues);
    present.last_tick = Some(sample.tick);
    present.last_ads_frac = Some(sample.ads_frac);
    events
}

pub fn tick_equipped_fpv_with_predicted_fire(
    equipped: &mut EquippedFpv,
    present: &mut FpvPresentState,
    sample: Option<FpvAuthoritySample>,
    predicted_local_fire: bool,
    dt_secs: f32,
) -> (FpvPoseSample, Vec<String>) {
    tick_equipped_fpv_with_extra_events(
        equipped,
        present,
        sample,
        predicted_local_fire,
        &[],
        dt_secs,
    )
}

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
        return tick_bo2_fpv(equipped, &mut present.bo2, &frame, dt_secs);
    }
    let mut events = match sample {
        Some(sample) => events_for_authority_tick(present, sample),
        None => Vec::new(),
    };
    if predicted_local_fire && !events.contains(&PresentFpvEvent::Fire) {
        events.push(PresentFpvEvent::Fire);
    }
    for event in extra_events {
        if !events.contains(event) {
            events.push(*event);
        }
    }
    if let Some(sample) = sample {
        equipped.controller.set_predicted_perks(sample.perks0);
    }
    apply_present_events(&mut equipped.controller, &events);
    if let Some(sample) = sample {
        match present.observe_weap_anim_edge(sample.weap_anim) {
            WeapAnimEdge::Unchanged => {}
            WeapAnimEdge::Idle => {
                let empty_mag = sample.clip_ammo == Some(0);
                if !equipped.controller.apply_idle_weap_anim(empty_mag) {
                    present.last_weap_anim = None;
                }
            }
            WeapAnimEdge::ForceIdle => {
                equipped
                    .controller
                    .apply_force_idle_weap_anim(sample.clip_ammo == Some(0));
            }
            WeapAnimEdge::Dispatch(slot) => {
                report_dispatch_slot(slot, equipped.controller.dispatch_sz_xanim_index(slot));
            }
        }
    }
    let crate::AdvanceResult { notifies } = equipped.controller.advance(dt_secs);
    let ads_frac = sample
        .map(|s| s.ads_frac)
        .or(present.last_ads_frac)
        .unwrap_or(0.0);
    equipped.controller.apply_ads_overlay_frame(ads_frac);
    if let (Some(left), Some(sample)) = (equipped.left.as_mut(), sample) {
        if sample.last_weapon_hand == 1 {
            left.set_predicted_perks(sample.perks0);
            match present.observe_weap_anim_secondary_edge(sample.weap_anim_secondary) {
                WeapAnimEdge::Unchanged => {}
                WeapAnimEdge::Idle => {
                    if !left.apply_idle_weap_anim(sample.left_clip_ammo == Some(0)) {
                        present.last_weap_anim_secondary = None;
                    }
                }
                WeapAnimEdge::ForceIdle => {
                    left.apply_force_idle_weap_anim(sample.left_clip_ammo == Some(0));
                }
                WeapAnimEdge::Dispatch(slot) => {
                    report_dispatch_slot(slot, left.dispatch_sz_xanim_index(slot));
                }
            }
            let _ = left.advance(dt_secs);

            left.apply_ads_overlay_frame(0.0);
        }
    }
    (sample_pose(&equipped.controller), notifies)
}

/// bo2 gun brain: one frame of Black Ops II's anim player, posed onto the gun's clips (and the
/// left gun's, dual wield). The player owns every weight and play position, ADS included.
fn tick_bo2_fpv(
    equipped: &mut EquippedFpv,
    bo2: &mut Bo2Fpv,
    frame: &VmFrame,
    dt_secs: f32,
) -> (FpvPoseSample, Vec<String>) {
    if bo2.set_for.as_deref() != Some(equipped.gun_xmodel.as_str()) {
        bo2.set = bo2_anim_set(&equipped.controller, equipped.left.as_ref());
        bo2.set_for = Some(equipped.gun_xmodel.clone());
    }
    bo2.ms_carry += dt_secs.max(0.0) * 1000.0;
    let ms = bo2.ms_carry.floor();
    bo2.ms_carry -= ms;
    let f = VmFrame {
        frametime_ms: ms as i32,
        ..*frame
    };
    bo2.vm.frame(&f, &bo2.table, &bo2.set);
    let pose = bo2.vm.pose();
    bo2.log_ms += ms as i64;
    vm_log(bo2.log_ms, &f, &pose);
    let is_left = |s: usize| (slot::FIRE_LEFT..=slot::RELOAD_LEFT).contains(&s);
    let right_pose = idle_for_unnamed(
        &equipped.controller,
        pose.iter()
            .filter(|p| !is_left(p.0))
            .map(|&(s, w, t)| (slot::name(s), w, t)),
    );
    let right = equipped.controller.apply_bo2_pose(right_pose);
    for n in &right.notifies {
        vm_log_note(&format!("TRACK {n}"));
    }
    if let Some(left) = equipped.left.as_mut() {
        // The left gun's own sounds stay off, as on the old path.
        let left_pose = idle_for_unnamed(
            left,
            pose.iter()
                .filter(|p| is_left(p.0))
                .map(|&(s, w, t)| (slot::name(s).trim_end_matches("_left"), w, t)),
        );
        let _ = left.apply_bo2_pose(left_pose);
    }
    (sample_pose(&equipped.controller), right.notifies)
}

/// BO2 fills every slot a gun leaves unnamed with the gun's idle anim (an empty idle on a gun
/// that has none, a sprint on a gun without sprint anims), so such a slot poses the idle clip.
/// Entries that land on idle together add their weights; the heaviest sets the play position.
fn idle_for_unnamed(
    c: &ViewmodelController,
    pose: impl Iterator<Item = (&'static str, f32, f32)>,
) -> Vec<(&'static str, f32, f32)> {
    let named = |name: &str| bo2_slot(name).is_some_and(|s| c.weapon().clip(s).is_some());
    let mut out: Vec<(&'static str, f32, f32)> = Vec::new();
    let mut idle: Option<(usize, f32)> = None;
    for (name, w, t) in pose {
        let name = if named(name) { name } else { "idle" };
        if name == "idle" {
            if let Some((i, heaviest)) = idle.as_mut() {
                out[*i].1 += w;
                if w > *heaviest {
                    *heaviest = w;
                    out[*i].2 = t;
                }
                continue;
            }
            idle = Some((out.len(), w));
        }
        out.push((name, w, t));
    }
    out
}

/// Test aid: with BO2MP_VMLOG=<file>, one line per frame of what BO2's anim player plays
/// (state, anim request, then each slot with weight and play position).
fn vm_log(ms: i64, f: &VmFrame, pose: &[(usize, f32, f32)]) {
    if vm_log_file().is_none() {
        return;
    }
    let mut line = format!(
        "{ms} w={} ws={} wa={} wt={} spd={:.0}",
        f.weapon, f.weaponstate, f.weap_anim, f.weapon_time, f.move_speed
    );
    if f.mantle_duration_ms != 0 {
        // The last climb's length as the movement timed it (BO2 marks it at the climb's start).
        line.push_str(&format!(" md={}", f.mantle_duration_ms));
    }
    for &(s, w, t) in pose {
        if w > 0.001 {
            line.push_str(&format!(" {}:{w:.2}:{t:.3}", slot::name(s)));
        }
    }
    vm_log_write(&line);
}

/// Test aid: a line of its own in the BO2MP_VMLOG log (the screenshot writer names the shot
/// it captures, so each picture pairs with the frame it shows).
pub fn vm_log_note(note: &str) {
    vm_log_write(&format!("NOTE {note}"));
}

fn vm_log_file() -> Option<&'static diag::LineFile> {
    static LOG: std::sync::OnceLock<Option<diag::LineFile>> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("BO2MP_VMLOG")?;
        diag::LineFile::create(std::path::Path::new(&path))
    })
    .as_ref()
}

fn vm_log_write(line: &str) {
    let Some(file) = vm_log_file() else {
        return;
    };
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    file.line(format!("{wall} {line}"));
}

/// bo2 gun brain: the gun's clips by BO2 slot. A slot is "named" when the gun has a clip for
/// it; the left-hand slots (78-84) come from the left gun's matching clips.
fn bo2_anim_set(right: &ViewmodelController, left: Option<&ViewmodelController>) -> AnimSet {
    let clip_of = |c: &ViewmodelController, name: &str| {
        let clip = c.weapon().clip(bo2_slot(name)?)?;
        Some(Clip {
            length_ms: clip.duration() * 1000.0,
            looping: clip.looping,
        })
    };
    let r: [Option<Clip>; slot::COUNT] =
        std::array::from_fn(|s| if s == 0 { None } else { clip_of(right, slot::name(s)) });
    let l: [Option<Clip>; slot::COUNT] = std::array::from_fn(|s| {
        let left = left?;
        if s == slot::IDLE {
            clip_of(left, "idle")
        } else if (slot::FIRE_LEFT..=slot::RELOAD_LEFT).contains(&s) {
            clip_of(left, slot::name(s).trim_end_matches("_left"))
        } else {
            None
        }
    });
    AnimSet::build(&r, &l)
}

fn observe_weap_anim_edge_on(previous: &mut Option<i32>, raw: i32) -> WeapAnimEdge {
    let last = previous.replace(raw);
    if last == Some(raw) {
        return WeapAnimEdge::Unchanged;
    }
    let masked = raw as u32 & weapon_iw4::WEAP_ANIM_EVENT_MASK;
    match masked {
        0 => WeapAnimEdge::Idle,
        1 => WeapAnimEdge::ForceIdle,
        _ => match weapon_iw4::slot_for_weap_anim_event(masked) {
            Some(slot) => WeapAnimEdge::Dispatch(slot),
            None => WeapAnimEdge::Idle,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_anim::xanim_clip::{AnimClip, Translation};
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

    /// The MP7's reload as BO2 plays it: an 86-frame clip over the 1800 ms reload.
    #[test]
    fn a_bo2_frame_poses_the_gun_and_the_old_player_stands_aside() {
        let gun = WeaponAnimations::empty("mp7_mp")
            .with_clip(WeaponAnimSlot::Idle, clip("idle", 45))
            .with_clip(WeaponAnimSlot::Reload, clip("reload", 86));
        let mut eq = EquippedFpv::new(
            "viewmodel_mp7",
            assets::FpvMeshIndex::from_order(0),
            assets::FpvMeshIndex::from_order(1),
            AssetNamespace::T6,
            asset_model::FpvHands::FromKit {
                namespace: AssetNamespace::T6,
                name: "hands".into(),
            },
            ViewmodelController::new(gun),
            None,
        );
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
        // 480 ms into the 1800 ms reload: the 2867 ms clip has run 480 * 2867 / 1800 ms.
        let t = eq.controller.animation_time(WeaponAnimSlot::Reload);
        assert!((t - 0.48 * 2.8667 / 1.8).abs() < 0.02, "reload at {t}");
        assert!(present.bo2.frame.is_none());

        // No BO2 frame: the old player runs again.
        tick_equipped_fpv_with_extra_events(&mut eq, &mut present, None, false, &[], 0.016);
        assert!(present.last_weap_anim.is_none());
    }

    /// The MP7 has no empty idle: BO2 plays its idle there, so an empty MP7 still holds the
    /// gun in the hands (it used to go unposed and sit inside the head).
    #[test]
    fn an_unnamed_slot_poses_the_idle_clip() {
        let gun = WeaponAnimations::empty("mp7_mp")
            .with_clip(WeaponAnimSlot::Idle, clip("idle", 45))
            .with_clip(WeaponAnimSlot::SprintLoop, clip("sprint", 20));
        let c = ViewmodelController::new(gun);
        let pose = idle_for_unnamed(
            &c,
            [("empty_idle", 0.5, 0.5), ("idle", 0.25, 0.1), ("sprint_loop", 0.25, 0.3)].into_iter(),
        );
        assert_eq!(pose, vec![("idle", 0.75, 0.5), ("sprint_loop", 0.25, 0.3)]);
    }
}
