//! The held gun's animation controller: which weapon clip plays, how it blends and when
//! it is over. It reads events from the first-person driver (`fpv`), or is posed clip by
//! clip straight from Black Ops II's own anim player (`apply_bo2_pose`).

use std::sync::Arc;

use anim_bank::{PlayingMotion, SkeletalClip, ClipPlanner};
use asset_game::{
    ACTION_BLEND_SECONDS, LIVE_BLEND_TARGET, ScopeOverlayStyle, REST_BREAK_BLEND_SECONDS,
    DORMANT_BLEND_TARGET, WEAPON_ANIM_SLOTS, GunMoveRole, GunClipSet, play_speed,
    clip_keeps_authored_rate,
};

/// Slots below this index are the base weapon table; the ones from here on are extras.
const BASE_SLOT_COUNT: usize = 37;
/// The base slots a started action takes over (index 0 is the always-there idle).
const FIRST_ACTION_SLOT: usize = 1;
const LAST_ACTION_SLOT: usize = 0x22;

/// Every slot a new action clears before it starts. The climb camera is posed from the
/// climb itself, so it is not one of them.
fn action_slots() -> impl Iterator<Item = usize> {
    (FIRST_ACTION_SLOT..=LAST_ACTION_SLOT)
        .chain(BASE_SLOT_COUNT..WEAPON_ANIM_SLOTS)
        .filter(|&node| node != GunMoveRole::CameraMantle.index())
}

/// The dive animations, full magazine then empty: take-off, in the air, touch-down.
const DIVE_SLOTS: [[GunMoveRole; 3]; 2] = [
    [GunMoveRole::DiveIn, GunMoveRole::DiveLoop, GunMoveRole::DiveOut],
    [GunMoveRole::DiveInEmpty, GunMoveRole::DiveLoopEmpty, GunMoveRole::DiveOutEmpty],
];
const DIVE_AIR: usize = 1;
const DIVE_LAND: usize = 2;

/// The crawl animations, full magazine then empty: in, forward, back, right, left, out.
const CRAWL_SLOTS: [[GunMoveRole; 6]; 2] = [
    [
        GunMoveRole::CrawlIn,
        GunMoveRole::CrawlForward,
        GunMoveRole::CrawlBack,
        GunMoveRole::CrawlRight,
        GunMoveRole::CrawlLeft,
        GunMoveRole::CrawlOut,
    ],
    [
        GunMoveRole::CrawlInEmpty,
        GunMoveRole::CrawlForwardEmpty,
        GunMoveRole::CrawlBackEmpty,
        GunMoveRole::CrawlRightEmpty,
        GunMoveRole::CrawlLeftEmpty,
        GunMoveRole::CrawlOutEmpty,
    ],
];
const CRAWL_IN: usize = 0;
const CRAWL_OUT: usize = 5;
/// Where each crawl part's time sits in the gun's crawl times (index 5 there is the
/// out-fire time, which is not a part of its own).
const CRAWL_TIME_AT: [usize; 6] = [0, 1, 2, 3, 4, 6];
const CRAWL_OUT_FIRE_TIME: usize = 5;

/// Aiming out of a dive brings the gun up this fast.
const DIVE_INTERRUPT_SECS: f32 = 0.1;
/// A new crawl direction blends in over this long.
const CRAWL_TURN_SECS: f32 = 0.15;
/// A shot or aim out of a crawl blends over the gun's crawl out-fire time, or this when
/// the gun has none.
const CRAWL_OUT_FIRE_DEFAULT_SECS: f32 = 0.05;

/// `(empty magazine, part)` of a slot within a family laid out like `families`.
fn family_part<const N: usize>(
    families: &[[GunMoveRole; N]; 2],
    slot: GunMoveRole,
) -> Option<(bool, usize)> {
    families.iter().enumerate().find_map(|(empty, parts)| {
        parts
            .iter()
            .position(|&s| s == slot)
            .map(|part| (empty == 1, part))
    })
}

fn crawl_part(bay: GunMoveRole) -> Option<(bool, usize)> {
    family_part(&CRAWL_SLOTS, bay)
}

fn dive_part(bay: GunMoveRole) -> Option<(bool, usize)> {
    family_part(&DIVE_SLOTS, bay)
}

/// The slot a gun falls back on when it has no clip for `slot`: the full-magazine sprint,
/// melee or crawl for an empty one, and its plain fire for a first-shots fire.
fn fallback_slot(bay: GunMoveRole) -> Option<GunMoveRole> {
    Some(match bay {
        GunMoveRole::SprintInEmpty => GunMoveRole::SprintIn,
        GunMoveRole::SprintLoopEmpty => GunMoveRole::SprintLoop,
        GunMoveRole::SprintOutEmpty => GunMoveRole::SprintOut,
        GunMoveRole::MeleeEmpty => GunMoveRole::Melee,
        GunMoveRole::MeleeChargeEmpty => GunMoveRole::MeleeCharge,
        GunMoveRole::FireIntro => GunMoveRole::Fire,
        GunMoveRole::AdsFireIntro => GunMoveRole::AdsFire,
        _ => {
            let (empty, part) = crawl_part(bay)?;
            return empty.then_some(CRAWL_SLOTS[0][part]);
        }
    })
}

/// BO2's slot names, and the clip slot each one plays on. BO2 slots this game has no clip
/// slot for (lowready, cont_fire, melee1-3, reload_right, finalshot, deploy, breakdown,
/// detonate, nightvision, slide_in, mantle, the sprint and dive cameras, the left hand's
/// slots, ads_up_other_scope) are not here.
const BO2_SLOT_NAMES: [(&str, GunMoveRole); 59] = [
    ("idle", GunMoveRole::Idle),
    ("empty_idle", GunMoveRole::EmptyIdle),
    ("fire_intro", GunMoveRole::FireIntro),
    ("fire", GunMoveRole::Fire),
    ("hold_fire", GunMoveRole::HoldFire),
    ("lastshot", GunMoveRole::LastShot),
    ("rechamber", GunMoveRole::Rechamber),
    ("melee", GunMoveRole::Melee),
    ("melee_empty", GunMoveRole::MeleeEmpty),
    ("melee_charge", GunMoveRole::MeleeCharge),
    ("melee_charge_empty", GunMoveRole::MeleeChargeEmpty),
    ("reload", GunMoveRole::Reload),
    ("reload_empty", GunMoveRole::DryReload),
    ("reload_start", GunMoveRole::ReloadKickoff),
    ("reload_end", GunMoveRole::ReloadEnd),
    ("reload_quick", GunMoveRole::ReloadQuick),
    ("reload_quick_empty", GunMoveRole::ReloadQuickEmpty),
    ("raise", GunMoveRole::Raise),
    ("first_raise", GunMoveRole::FirstRaise),
    ("drop", GunMoveRole::Drop),
    ("alt_raise", GunMoveRole::AltRaise),
    ("alt_drop", GunMoveRole::AltDrop),
    ("quick_raise", GunMoveRole::QuickRaise),
    ("quick_drop", GunMoveRole::QuickDrop),
    ("empty_raise", GunMoveRole::EmptyRaise),
    ("empty_drop", GunMoveRole::EmptyDrop),
    ("sprint_in", GunMoveRole::SprintIn),
    ("sprint_loop", GunMoveRole::SprintLoop),
    ("sprint_out", GunMoveRole::SprintOut),
    ("sprint_empty_in", GunMoveRole::SprintInEmpty),
    ("sprint_empty_loop", GunMoveRole::SprintLoopEmpty),
    ("sprint_empty_out", GunMoveRole::SprintOutEmpty),
    ("crawl_in", GunMoveRole::CrawlIn),
    ("crawl_forward", GunMoveRole::CrawlForward),
    ("crawl_back", GunMoveRole::CrawlBack),
    ("crawl_right", GunMoveRole::CrawlRight),
    ("crawl_left", GunMoveRole::CrawlLeft),
    ("crawl_out", GunMoveRole::CrawlOut),
    ("crawl_empty_in", GunMoveRole::CrawlInEmpty),
    ("crawl_empty_forward", GunMoveRole::CrawlForwardEmpty),
    ("crawl_empty_back", GunMoveRole::CrawlBackEmpty),
    ("crawl_empty_right", GunMoveRole::CrawlRightEmpty),
    ("crawl_empty_left", GunMoveRole::CrawlLeftEmpty),
    ("crawl_empty_out", GunMoveRole::CrawlOutEmpty),
    ("ads_fire", GunMoveRole::AdsFire),
    ("ads_lastshot", GunMoveRole::AdsLastShot),
    ("ads_fire_intro", GunMoveRole::AdsFireIntro),
    ("ads_rechamber", GunMoveRole::AdsRechamber),
    ("dtp_in", GunMoveRole::DiveIn),
    ("dtp_loop", GunMoveRole::DiveLoop),
    ("dtp_out", GunMoveRole::DiveOut),
    ("dtp_empty_in", GunMoveRole::DiveInEmpty),
    ("dtp_empty_loop", GunMoveRole::DiveLoopEmpty),
    ("dtp_empty_out", GunMoveRole::DiveOutEmpty),
    ("camera_mantle", GunMoveRole::CameraMantle),
    ("ads_up", GunMoveRole::AdsUp),
    ("ads_down", GunMoveRole::AdsDown),
    ("ads_up_other", GunMoveRole::AdsUp),
    ("ads_down_other", GunMoveRole::AdsDown),
];

/// The clip slot for a Black Ops II viewmodel slot, by BO2's slot name.
pub fn bo2_slot(name: &str) -> Option<GunMoveRole> {
    BO2_SLOT_NAMES
        .iter()
        .find(|(seen, _)| *seen == name)
        .map(|&(_, seat)| seat)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GunAnimEvent {
    Fire,
    Raise { initial: bool, hasty: bool },
    ReloadInfo { empty: bool },
    ReloadKickoff,
    ReloadEnd,
    Rechamber,
    Drop,
    SprintIn,
    SprintLoopData,
    SprintOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GunPhase {
    Ready,
    Raising { opening: bool, rushed: bool },
    Firing,
    ReloadingInfo { empty: bool },
    ReloadOpening,
    ReloadFinishing,
    ChamberingRound,
    Dropping,
    SprintIn,
    SprintHold,
    SprintOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventReaction {
    Started(GunPhase),
    IgnoredState(GunPhase),
    IgnoredMissingClip(GunMoveRole),
}

/// What [`FirstPersonGunAnimator::apply_bo2_pose`] did.
#[derive(Debug, Clone, Default)]
pub struct Bo2Pose<'a> {
    /// BO2 slots with no clip here (they draw nothing).
    pub unplaced: Vec<&'a str>,
    /// Notetracks the posed clips passed.
    pub notify_list: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StepOutcome {
    pub notes_hit: Vec<String>,
}

/// A gun's time in ms as a timer, when it gives one at all (0 and below mean none).
fn timer_if_set(ms: i32) -> Option<i32> {
    if ms > 0 { Some(ms) } else { None }
}

/// Whether the state is any part of a reload.
fn is_reload_state(state: GunPhase) -> bool {
    matches!(
        state,
        GunPhase::ReloadingInfo { .. } | GunPhase::ReloadOpening | GunPhase::ReloadFinishing
    )
}

#[derive(Debug)]
pub struct FirstPersonGunAnimator {
    /// How far up the aim overlay was last frame, for choosing a fire clip.
    ads_seen: f32,
    schedule: ClipPlanner,
    anims: GunClipSet,
    /// Seconds the action's timer has left; `None` when it has no timer or loops.
    timer_left: Option<f32>,
    gun_phase: GunPhase,
    /// The slot of the action being played; `None` while idling.
    playing: Option<GunMoveRole>,
    predicted_perks: [u32; 2],
    /// The trigger came up while a fire animation still played: settle to idle when it ends.
    settle_when_done: bool,
    /// The crawl direction to play when the crawl-in ends.
    crawl_next: Option<GunMoveRole>,
}

impl FirstPersonGunAnimator {
    pub fn new(anims: GunClipSet) -> Self {
        let mut sched = ClipPlanner::new(WEAPON_ANIM_SLOTS);
        let installed = anims.install_clips(&mut sched);
        installed.expect("clip index is inside the scheduler");
        let predicted_perks = [0; 2];
        let mut ctl = Self {
            schedule: sched,
            anims,
            predicted_perks,
            gun_phase: GunPhase::Ready,
            ads_seen: 0.0,
            timer_left: None,
            playing: None,
            settle_when_done: false,
            crawl_next: None,
        };
        ctl.set_slot_blend(GunMoveRole::Idle, LIVE_BLEND_TARGET, ACTION_BLEND_SECONDS);

        // Bones only the aim clips animate need their lowered pose at the hip.
        match ctl.anims.reel(GunMoveRole::AdsDown).map(|c| c.duration()) {
            Some(down) => {
                ctl.set_slot_blend(GunMoveRole::AdsDown, LIVE_BLEND_TARGET, 0.0);
                ctl.scrub_overlay(GunMoveRole::AdsDown, down, None);
            }
            None if ctl.anims.ads_overlay == ScopeOverlayStyle::StartAimMotion => {
                ctl.set_slot_blend(GunMoveRole::AdsUp, LIVE_BLEND_TARGET, 0.0);
            }
            None => {}
        }
        ctl
    }

    pub const fn state(&self) -> GunPhase {
        self.gun_phase
    }

    pub fn set_predicted_specialties(&mut self, perks: [u32; 2]) {
        self.predicted_perks = perks;
    }

    pub fn gun_name(&self) -> &str {
        self.anims.name.as_str()
    }

    pub const fn weapon(&self) -> &GunClipSet {
        let Self { anims, .. } = self;
        anims
    }

    /// Swaps one slot's clip in place (the smoothed run loop), keeping its weight and
    /// play position.
    pub fn replace_clip(&mut self, bay: GunMoveRole, clip_in: Arc<SkeletalClip>) {
        let weapon = std::mem::replace(&mut self.anims, GunClipSet::blank(""));
        self.anims = weapon.with_clip(bay, Arc::clone(&clip_in));
        let time = self.animation_when(bay);
        if self.schedule.load_clip(bay.index(), clip_in).is_ok() {
            self.put_time(bay.index(), time);
        }
    }

    pub fn active_anims(&self) -> impl Iterator<Item = PlayingMotion<'_>> {
        ClipPlanner::active(&self.schedule)
    }

    pub fn animation_wt(&self, which: GunMoveRole) -> f32 {
        self.schedule.weight(which.index()).unwrap_or_default()
    }

    pub fn animation_when(&self, which: GunMoveRole) -> f32 {
        self.schedule.playhead(which.index()).unwrap_or_default()
    }

    // -----------------------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------------------

    /// The result for an event that changes nothing: the state the gun is in stays.
    fn ignored(&self) -> EventReaction {
        EventReaction::IgnoredState(self.gun_phase)
    }

    fn permits(&self, event_in: GunAnimEvent) -> bool {
        let now = self.gun_phase;
        match event_in {
            GunAnimEvent::Fire => now == GunPhase::Ready || now == GunPhase::Firing,
            GunAnimEvent::SprintLoopData => now != GunPhase::SprintHold,
            GunAnimEvent::SprintIn => matches!(now, GunPhase::Ready | GunPhase::Firing | GunPhase::SprintOut),
            _ => true,
        }
    }

    pub fn hnd(&mut self, ev: GunAnimEvent) -> EventReaction {
        if !self.permits(ev) {
            return self.ignored();
        }
        let slot = match ev {
            GunAnimEvent::Fire if self.ads_seen > 0.0 && self.has_clip(GunMoveRole::AdsFire) => {
                GunMoveRole::AdsFire
            }
            GunAnimEvent::Fire => GunMoveRole::Fire,
            GunAnimEvent::Raise { initial: true, .. } => GunMoveRole::FirstRaise,
            GunAnimEvent::Raise { hasty: true, .. } => GunMoveRole::QuickRaise,
            GunAnimEvent::Raise { .. } => GunMoveRole::Raise,
            GunAnimEvent::ReloadInfo { empty: true } => GunMoveRole::DryReload,
            GunAnimEvent::ReloadInfo { empty: false } => GunMoveRole::Reload,
            GunAnimEvent::ReloadKickoff => GunMoveRole::ReloadKickoff,
            GunAnimEvent::ReloadEnd => GunMoveRole::ReloadEnd,
            GunAnimEvent::Rechamber => GunMoveRole::Rechamber,
            GunAnimEvent::Drop => GunMoveRole::Drop,
            GunAnimEvent::SprintIn => GunMoveRole::SprintIn,
            GunAnimEvent::SprintLoopData => GunMoveRole::SprintLoop,
            GunAnimEvent::SprintOut => GunMoveRole::SprintOut,
        };
        let plan = match (ev, self.plan_for(slot)) {
            (GunAnimEvent::Raise { initial: first, hasty: quick }, (_, ms)) => {
                (GunPhase::Raising { opening: first, rushed: quick }, ms)
            }
            (_, planned) => planned,
        };
        self.begin_action(slot, plan)
    }

    /// Idles, and says so as an event result.
    fn idle_started(&mut self, empty_in: bool, quick: bool) -> EventReaction {
        self.settle_idle(empty_in, quick);
        EventReaction::Started(GunPhase::Ready)
    }

    /// Plays the clip node `node_index` as the viewmodel anim names it.
    pub fn dispatch_sz_xanim_idx(&mut self, node_index: usize) -> EventReaction {
        let slot = match GunMoveRole::from_index(node_index) {
            // The aim clips and the climb camera are posed by their own drivers.
            None | Some(GunMoveRole::AdsUp | GunMoveRole::AdsDown | GunMoveRole::CameraMantle) => return self.ignored(),
            Some(GunMoveRole::Idle | GunMoveRole::EmptyIdle) => return self.idle_started(false, false),
            Some(slot) => slot,
        };
        // A gun without the dive animations comes straight up.
        if let Some((empty, _)) = dive_part(slot).filter(|_| !self.has_clip(slot)) {
            return self.idle_started(empty, true);
        }
        let slot = self.clip_or_fallback(slot);
        match crawl_part(slot) {
            Some((empty, part)) => self.dispatch_crawl(slot, empty, part),
            None => self.begin_action(slot, self.plan_for(slot)),
        }
    }

    fn has_clip(&self, bay: GunMoveRole) -> bool {
        self.anims.reel(bay).is_some()
    }

    /// `slot`, or the one standing in for it when the gun has no clip for it.
    fn clip_or_fallback(&self, bay: GunMoveRole) -> GunMoveRole {
        match fallback_slot(bay) {
            Some(fallback) if !self.has_clip(bay) => fallback,
            _ => bay,
        }
    }

    /// The state an action in `slot` leaves the gun in, and the time (ms) the gun's
    /// facts give it, if any.
    fn plan_for(&self, bay: GunMoveRole) -> (GunPhase, Option<i32>) {
        let w = &self.anims;
        let raising = |first, quick| GunPhase::Raising { opening: first, rushed: quick };
        let (state, ms) = match bay {
            GunMoveRole::Fire | GunMoveRole::AdsFire | GunMoveRole::LastShot | GunMoveRole::AdsLastShot => (GunPhase::Firing, w.fire_time_ms),
            // bo2zm: the wind-up plays over the wind-up time, not the throw's
            // (the Monkey Bomb's pull-back ran at double speed).
            GunMoveRole::HoldFire => (GunPhase::Firing, w.trigger_hold_millis),
            GunMoveRole::FireIntro | GunMoveRole::AdsFireIntro => (GunPhase::Firing, w.intro_fire_time_ms),
            GunMoveRole::FirstRaise => (raising(true, false), 0),
            GunMoveRole::QuickRaise => (raising(false, true), w.quick_raise_time_ms),
            GunMoveRole::AltRaise => (raising(false, false), w.alternate_raise_time_ms),
            GunMoveRole::EmptyRaise => (raising(false, false), 0),
            GunMoveRole::Raise => (raising(false, false), w.raise_time_ms),
            GunMoveRole::Reload => (GunPhase::ReloadingInfo { empty: false }, w.reload_duration_millis),
            GunMoveRole::DryReload => (GunPhase::ReloadingInfo { empty: true }, w.refill_dry_millis),
            GunMoveRole::ReloadQuick => (GunPhase::ReloadingInfo { empty: false }, w.reload_quick_time_ms),
            GunMoveRole::ReloadQuickEmpty => (GunPhase::ReloadingInfo { empty: true }, w.reload_quick_empty_time_ms),
            GunMoveRole::ReloadKickoff => (GunPhase::ReloadOpening, w.refill_begin_millis),
            GunMoveRole::ReloadEnd => (GunPhase::ReloadFinishing, w.refill_finish_millis),
            GunMoveRole::Rechamber | GunMoveRole::AdsRechamber => (GunPhase::ChamberingRound, 0),
            GunMoveRole::Drop => (GunPhase::Dropping, w.drop_time_ms),
            GunMoveRole::QuickDrop => (GunPhase::Dropping, w.quick_drop_time_ms),
            GunMoveRole::AltDrop => (GunPhase::Dropping, w.alternate_drop_time_ms),
            GunMoveRole::EmptyDrop => (GunPhase::Dropping, 0),
            GunMoveRole::SprintIn | GunMoveRole::SprintInEmpty => (GunPhase::SprintIn, w.run_lift_millis),
            GunMoveRole::SprintLoop | GunMoveRole::SprintLoopEmpty => (GunPhase::SprintHold, w.run_cycle_millis),
            GunMoveRole::SprintOut | GunMoveRole::SprintOutEmpty => (GunPhase::SprintOut, w.run_lower_millis),
            GunMoveRole::Melee | GunMoveRole::MeleeEmpty => (GunPhase::Ready, w.melee_time_ms),
            GunMoveRole::MeleeCharge | GunMoveRole::MeleeChargeEmpty => (GunPhase::Ready, w.melee_charge_time_ms),
            GunMoveRole::Idle | GunMoveRole::EmptyIdle | GunMoveRole::AdsUp | GunMoveRole::AdsDown | GunMoveRole::CameraMantle => (GunPhase::Ready, 0),
            _ => {
                let ms = match (crawl_part(bay), dive_part(bay)) {
                    (Some((_, part)), _) => w.crawl_times_ms[CRAWL_TIME_AT[part]],
                    (_, Some((_, part))) => w.dive_times_ms[part],
                    _ => 0,
                };
                (GunPhase::Ready, ms)
            }
        };
        (state, timer_if_set(ms))
    }

    /// A reload timer shortened by the fast-reload perk the local player holds.
    fn perk_scaled(&self, state: GunPhase, timer_ms_in: Option<i32>) -> Option<i32> {
        use weapon_t6::combat_facts;
        let factor = combat_facts::QUICK_HANDS_FACTOR_STOCK;
        let ms = timer_ms_in?;
        let faster = is_reload_state(state)
            && factor > 0.0
            && combat_facts::perk_fastreload_eligible(self.predicted_perks, self.anims.carries_over_talents);
        Some(if faster {
            (ms as f32 * factor).round() as i32
        } else {
            ms
        })
    }

    // -----------------------------------------------------------------------------------
    // Crawl
    // -----------------------------------------------------------------------------------

    /// A crawl slot asked for. From anything else a direction plays the crawl-in first and
    /// then the direction the player crawls by then; a new direction blends across; the
    /// crawl-out settles to idle. A gun without the clip keeps what it plays.
    fn dispatch_crawl(&mut self, bay: GunMoveRole, empty_here: bool, part: usize) -> EventReaction {
        if !self.has_clip(bay) {
            return EventReaction::Started(self.gun_phase);
        }
        let playing = self.playing.and_then(crawl_part);
        if part == CRAWL_OUT {
            if playing.is_none() {
                return self.ignored();
            }
            return self.start_crawl(bay, self.crawl_out_fire_secs());
        }
        match playing {
            Some((_, CRAWL_IN)) => {
                self.crawl_next = Some(bay);
                EventReaction::Started(self.gun_phase)
            }
            Some(_) => self.start_crawl(bay, CRAWL_TURN_SECS),
            None => {
                let crawl_in = self.clip_or_fallback(CRAWL_SLOTS[usize::from(empty_here)][CRAWL_IN]);
                if self.has_clip(crawl_in) {
                    let started = self.start_crawl(crawl_in, ACTION_BLEND_SECONDS);
                    self.crawl_next = Some(bay);
                    started
                } else {
                    self.start_crawl(bay, CRAWL_TURN_SECS)
                }
            }
        }
    }

    fn start_crawl(&mut self, bay: GunMoveRole, goal_secs: f32) -> EventReaction {
        self.begin_over(bay, self.plan_for(bay), goal_secs)
    }

    /// Whether a crawl-in or a crawl direction plays (the crawl-out does not count).
    fn crawling(&self) -> bool {
        self.playing
            .and_then(crawl_part)
            .is_some_and(|(_k, part)| part != CRAWL_OUT)
    }

    fn crawl_out_fire_secs(&self) -> f32 {
        timer_if_set(self.anims.crawl_times_ms[CRAWL_OUT_FIRE_TIME])
            .map_or(CRAWL_OUT_FIRE_DEFAULT_SECS, |ms| ms as f32 / 1000.0)
    }

    // -----------------------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------------------

    pub fn step_on(&mut self, elapsed: f32) -> StepOutcome {
        let step = elapsed.max(0.0);
        let heard = self.schedule.run_for(step);
        self.timer_left = self.timer_left.map(|left| (left - step).max(0.0));
        self.chain_dive();
        self.chain_crawl();
        self.restart_ended_run_loop();

        let fire_over = self.gun_phase == GunPhase::Firing
            && self.playing.is_some_and(|slot| self.has_played_out(slot));
        if fire_over {
            self.enter_action(None, None, GunPhase::Ready);
        }
        // A fire animation that outlasted the trigger settles to idle once played out.
        if self.settle_when_done {
            if self.gun_phase != GunPhase::Ready || self.playing.is_some() {
                self.settle_when_done = false;
            } else if !self.action_clips_unfinished() {
                self.settle_when_done = false;
                self.settle_idle(false, false);
            }
        }
        StepOutcome { notes_hit: heard }
    }

    /// The dive: take-off runs into the in-air clip, which holds its last frame until
    /// touch-down; touch-down settles to idle.
    fn chain_dive(&mut self) {
        let Some(slot) = self.playing else { return };
        let Some((empty, part)) = dive_part(slot) else {
            return;
        };
        if part == DIVE_AIR || !self.has_played_out(slot) {
            return;
        }
        let air = DIVE_SLOTS[usize::from(empty)][DIVE_AIR];
        if part == DIVE_LAND {
            self.settle_idle(empty, false);
        } else if self.has_clip(air) {
            self.begin_action(air, self.plan_for(air));
        }
    }

    /// The crawl: the crawl-in runs into the direction the player crawls; the crawl-out
    /// settles to idle.
    fn chain_crawl(&mut self) {
        let Some(slot) = self.playing else { return };
        let Some((empty, part)) = crawl_part(slot) else {
            return;
        };
        if !matches!(part, CRAWL_IN | CRAWL_OUT) || !self.has_played_out(slot) {
            return;
        }
        match self.crawl_next.take() {
            Some(next) if part == CRAWL_IN => {
                self.start_crawl(next, ACTION_BLEND_SECONDS);
            }
            _ => self.settle_idle(empty, false),
        }
    }

    /// A run loop that is not a looping clip (the ballistic knife's is 1000 ms inside a
    /// 600 ms sprint loop time) starts over each time it ends, as BO2 replays it, instead
    /// of holding its last frame.
    fn restart_ended_run_loop(&mut self) {
        let slot = match self.playing {
            Some(slot @ (GunMoveRole::SprintLoop | GunMoveRole::SprintLoopEmpty))
                if self.gun_phase == GunPhase::SprintHold =>
            {
                slot
            }
            _ => return,
        };
        let one_shot = self.anims.reel(slot).is_some_and(|clip| !clip.looping);
        if one_shot && self.has_played_out(slot) {
            self.begin_over(slot, self.plan_for(slot), 0.0);
        }
    }

    /// Whether the action's own timer is out, or (with none) its clip has played to the end.
    fn has_played_out(&self, slot_ix: GunMoveRole) -> bool {
        if let Some(left) = self.timer_left {
            return left <= 0.0;
        }
        let Some(clip) = self.anims.reel(slot_ix) else {
            return true;
        };
        !clip.looping && self.animation_when(slot_ix) >= clip.duration()
    }

    /// Whether any clip in the action slots has weight and has not played out (a looping
    /// clip never has).
    fn action_clips_unfinished(&self) -> bool {
        action_slots().any(|node| {
            let held = self.schedule.weight(node).unwrap_or_default() > 0.0;
            let at = self.schedule.playhead(node).unwrap_or_default();
            held && self
                .anims
                .clip_for_node(node)
                .is_some_and(|c| c.looping || at < c.duration())
        })
    }

    // -----------------------------------------------------------------------------------
    // Idle and settling
    // -----------------------------------------------------------------------------------

    /// The viewmodel anim went to idle. `false` when an action clip is still playing and
    /// the idle has to wait; a crawl is ended quickly instead.
    pub fn apply_resting_weap_anim(&mut self, mag_empty: bool) -> bool {
        let crawling = self.crawling();
        let must_wait = !crawling && self.action_clips_unfinished();
        if !must_wait {
            self.settle_idle(mag_empty, crawling);
        }
        !must_wait
    }

    /// The viewmodel anim forced idle: it cuts into whatever plays (a dive, a crawl and a
    /// running action blend out faster).
    pub fn apply_force_idle_weap_anim(&mut self, empty_mag: bool) {
        let blend = match self.playing {
            Some(slot) if dive_part(slot).is_some() => DIVE_INTERRUPT_SECS,
            Some(slot) if crawl_part(slot).is_some() => self.crawl_out_fire_secs(),
            _ if self.action_clips_unfinished() => REST_BREAK_BLEND_SECONDS,
            _ => ACTION_BLEND_SECONDS,
        };
        self.settle_idle_over(empty_mag, blend);
    }

    /// The trigger came up. A fire clip that has weight settles to idle; one the gun's
    /// facts say plays out (a recoil longer than the fire time) is left to finish first.
    pub fn relax_firing_to_idle(&mut self) {
        if self.gun_phase != GunPhase::Ready || self.playing.is_some() {
            return;
        }
        if self.anims.fire_plays_out && self.action_clips_unfinished() {
            self.settle_when_done = true;
            return;
        }
        if [GunMoveRole::Fire, GunMoveRole::FireIntro]
            .into_iter()
            .any(|fire| self.animation_wt(fire) > 0.01)
        {
            self.settle_idle(false, false);
        }
    }

    /// Back to idle; `quick` blends the way an interrupt does.
    fn settle_idle(&mut self, empty_v: bool, quick: bool) {
        let blend = [ACTION_BLEND_SECONDS, REST_BREAK_BLEND_SECONDS][usize::from(quick)];
        self.settle_idle_over(empty_v, blend);
    }

    fn settle_idle_over(&mut self, empty_val: bool, blend: f32) {
        let idle = match empty_val && self.has_clip(GunMoveRole::EmptyIdle) {
            true => GunMoveRole::EmptyIdle,
            false => GunMoveRole::Idle,
        };
        self.crawl_next = None;
        self.enter_action(None, None, GunPhase::Ready);
        self.clear_action_slots(blend);
        self.set_slot_blend(idle, LIVE_BLEND_TARGET, blend);
    }

    /// Records which action plays, how long its timer has left and the state it leaves.
    fn enter_action(
        &mut self,
        bay: Option<GunMoveRole>,
        remaining: Option<f32>,
        next_in: GunPhase,
    ) {
        self.playing = bay;
        self.timer_left = remaining;
        self.gun_phase = next_in;
    }

    // -----------------------------------------------------------------------------------
    // Starting an action
    // -----------------------------------------------------------------------------------

    fn begin_action(&mut self, bay: GunMoveRole, plan: (GunPhase, Option<i32>)) -> EventReaction {
        // A shot (or anything else) out of a crawl blends from it over the crawl out-fire time.
        let blend = match self.crawling() {
            true => self.crawl_out_fire_secs(),
            false => ACTION_BLEND_SECONDS,
        };
        self.begin_over(bay, plan, blend)
    }

    fn begin_over(&mut self, bay: GunMoveRole, plan: (GunPhase, Option<i32>), blend: f32) -> EventReaction {
        self.crawl_next = None;
        match self.anims.reel(bay).map(Arc::clone) {
            Some(clip) => EventReaction::Started(self.run_clip(bay, &clip, plan, blend)),
            None => {
                self.clear_action_slots(blend);
                self.enter_action(None, None, plan.0);
                EventReaction::IgnoredMissingClip(bay)
            }
        }
    }

    /// Starts `clip` on `slot`, stretched or squeezed to the time the gun's facts give it.
    fn run_clip(
        &mut self,
        slot_in: GunMoveRole,
        reel: &Arc<SkeletalClip>,
        (state, planned_ms): (GunPhase, Option<i32>),
        blend: f32,
    ) -> GunPhase {
        let timer_ms = self.perk_scaled(state, planned_ms).and_then(timer_if_set);
        self.clear_action_slots(blend);
        self.schedule
            .load_clip(slot_in.index(), Arc::clone(reel))
            .expect("slot index fits the blend tree");
        let rate = timer_ms
            .filter(|_| !clip_keeps_authored_rate(slot_in.index()))
            .map_or(1.0, |ms| {
                play_speed((reel.duration() * 1000.0).round() as i32, ms)
            });
        self.put_rate(slot_in.index(), rate);
        self.put_node_blend(slot_in.index(), LIVE_BLEND_TARGET, blend);
        let remaining = timer_ms
            .filter(|_| !reel.looping)
            .map(|ms| ms as f32 / 1000.0);
        self.enter_action(Some(slot_in), remaining, state);
        state
    }

    fn clear_action_slots(&mut self, goal_secs: f32) {
        for node in action_slots() {
            self.put_node_blend(node, DORMANT_BLEND_TARGET, goal_secs);
            self.put_rate(node, 1.0);
        }
    }

    // -----------------------------------------------------------------------------------
    // The aim overlay
    // -----------------------------------------------------------------------------------

    /// Scrubs the aim-up and aim-down clips to how far the aim is (`gun_raise_share`).
    /// A melee clears them.
    pub fn apply_ads_overlay_frm(&mut self, gun_raise_share: f32) {
        let scrub = weapon_t6::ads_overlay::ads_overlay_scrub(gun_raise_share);
        let up_weight = scrub.scope_raise_blend;
        self.ads_seen = up_weight;
        let meleeing = matches!(
            self.playing,
            Some(GunMoveRole::Melee | GunMoveRole::MeleeCharge | GunMoveRole::MeleeEmpty | GunMoveRole::MeleeChargeEmpty)
        );
        if meleeing {
            self.set_slot_blend(GunMoveRole::AdsUp, DORMANT_BLEND_TARGET, 0.0);
            self.set_slot_blend(GunMoveRole::AdsDown, DORMANT_BLEND_TARGET, 0.0);
            return;
        }
        let aiming = gun_raise_share > 0.0;
        let scripted_anim_playback = self.anims.ads_overlay == ScopeOverlayStyle::StartAimMotion;
        let has_down = self.has_clip(GunMoveRole::AdsDown);
        let active = |on: bool| [DORMANT_BLEND_TARGET, LIVE_BLEND_TARGET][usize::from(on)];

        if let Some(length) = self.anims.reel(GunMoveRole::AdsUp).map(|c| c.duration()) {
            let weight = if scripted_anim_playback {
                active(aiming || !has_down)
            } else {
                up_weight
            };
            self.scrub_overlay(GunMoveRole::AdsUp, length * scrub.scope_raise_progress, Some(weight));
        }
        if let Some(length) = self.anims.reel(GunMoveRole::AdsDown).map(|c| c.duration()) {
            let weight = scripted_anim_playback.then(|| active(!aiming));
            self.scrub_overlay(GunMoveRole::AdsDown, length * scrub.scope_lower_progress, weight);
        }
    }

    /// Holds an overlay clip at `time`, and moves its weight to `weight` when given.
    fn scrub_overlay(&mut self, bay: GunMoveRole, time: f32, weight: Option<f32>) {
        self.put_rate(bay.index(), 0.0);
        self.put_time(bay.index(), time);
        if let Some(weight) = weight {
            self.put_node_blend(bay.index(), weight, ACTION_BLEND_SECONDS);
        }
    }

    // -----------------------------------------------------------------------------------
    // Posing from BO2's anim player
    // -----------------------------------------------------------------------------------

    /// Poses the gun straight from Black Ops II's anim player (`weapon_t6::viewmodel`).
    /// `pose` is every slot with weight as (BO2 slot name, weight, play position 0..=1);
    /// the player has done the blending, so weights go in as given and no clip runs on its
    /// own. A slot listed twice (a restart's fading copy) keeps its first entry, the fresh
    /// play. Notetracks are those each posed clip passed since the last pose: from the
    /// top when it was not playing or went back to an earlier point without looping.
    pub fn apply_bo2_pose<'a>(
        &mut self,
        pose: impl IntoIterator<Item = (&'a str, f32, f32)>,
    ) -> Bo2Pose<'a> {
        let mut was_at = [None; WEAPON_ANIM_SLOTS];
        for (node, was) in was_at.iter_mut().enumerate() {
            if self.schedule.weight(node).expect("node is within tree") > 0.0 {
                *was = Some(self.schedule.playhead(node).expect("node is within tree"));
            }
            self.put_node_blend(node, DORMANT_BLEND_TARGET, 0.0);
        }
        let mut done = [false; WEAPON_ANIM_SLOTS];
        let mut out = Bo2Pose::default();
        for (name, weight, frac) in pose {
            let Some(slot) = bo2_slot(name) else {
                out.unplaced.push(name);
                continue;
            };
            let node = slot.index();
            if std::mem::replace(&mut done[node], true) {
                continue;
            }
            let clip = match self.anims.reel(slot) {
                Some(found) => Arc::clone(found),
                None => {
                    out.unplaced.push(name);
                    continue;
                }
            };
            let time = clip.duration() * frac.clamp(0.0, 1.0);
            if weight > 0.0 {
                let from = match was_at[node] {
                    Some(old) if time >= old || clip.looping => old,
                    _ => 0.0,
                };
                out.notify_list.extend(clip.notes_between(from, time));
            }
            self.put_rate(node, 0.0);
            self.put_time(node, time);
            self.put_node_blend(node, weight, 0.0);
        }
        out
    }

    // -----------------------------------------------------------------------------------
    // The clip tree
    // -----------------------------------------------------------------------------------

    /// Moves a slot's weight, but only when the gun has a clip there.
    fn set_slot_blend(&mut self, bay: GunMoveRole, blend_to: f32, goal_secs: f32) {
        if self.has_clip(bay) {
            self.put_node_blend(bay.index(), blend_to, goal_secs);
        }
    }

    fn put_node_blend(&mut self, node_ix: usize, target_wt: f32, goal_secs: f32) {
        self.schedule
            .assign_target_blend(node_ix, target_wt, goal_secs)
            .expect("node is within tree");
    }

    fn put_rate(&mut self, node_in: usize, rate: f32) {
        self.schedule.set_speed(node_in, rate).expect("node is within tree");
    }

    fn put_time(&mut self, node_v: usize, time: f32) {
        self.schedule.seek(node_v, time).expect("node is within tree");
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use anim_bank::motion_clip::{ClipEventMark, RootShift};

    fn reel(caption: &str, frames: u16, looping: bool) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: caption.to_owned(),
            fps: 30.0,
            frame_total: frames,
            looping,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        })
    }

    fn gun(name: &str, clips: &[(GunMoveRole, &str, u16, bool)]) -> GunClipSet {
        clips.iter().fold(
            GunClipSet::blank(name).with_clip(GunMoveRole::Idle, reel("idle", 30, true)),
            |gun, &(slot, collision_label, frames, looping)| {
                gun.with_clip(slot, reel(collision_label, frames, looping))
            },
        )
    }

    fn on(c: &FirstPersonGunAnimator, bay: GunMoveRole) -> bool {
        c.animation_wt(bay) == LIVE_BLEND_TARGET
    }

    fn off(c: &FirstPersonGunAnimator, bay: GunMoveRole) -> bool {
        c.animation_wt(bay) == 0.0
    }

    /// Plays clip node `slot` and runs `secs` of frames.
    fn go(c: &mut FirstPersonGunAnimator, bay: GunMoveRole, secs: f32) -> EventReaction {
        let result = c.dispatch_sz_xanim_idx(bay.index());
        c.step_on(secs);
        result
    }

    // ---- slot names ------------------------------------------------------------------

    #[test]
    fn bo2_slot_names_map_to_clip_slots_and_unknown_ones_to_none() {
        assert_eq!(bo2_slot("dtp_loop"), Some(GunMoveRole::DiveLoop));
        assert_eq!(bo2_slot("sprint_empty_loop"), Some(GunMoveRole::SprintLoopEmpty));
        assert_eq!(bo2_slot("camera_mantle"), Some(GunMoveRole::CameraMantle));
        assert_eq!(bo2_slot("lowready_in"), None);
        assert_eq!(bo2_slot(""), None);
        let distinct: std::collections::HashSet<&str> =
            BO2_SLOT_NAMES.iter().map(|entry| entry.0).collect();
        assert_eq!(
            distinct.len(),
            BO2_SLOT_NAMES.len(),
            "no name is listed twice"
        );
    }

    // ---- empty magazine and first shots ----------------------------------------------

    #[test]
    fn an_empty_melee_plays_its_own_clip_or_the_full_one() {
        let base = gun("fiveseven_zm", &[(GunMoveRole::Melee, "tactical_melee", 20, false)]);
        let mut c = FirstPersonGunAnimator::new(base.clone());
        go(&mut c, GunMoveRole::MeleeEmpty, 0.05);
        assert!(on(&c, GunMoveRole::Melee), "no empty clip: the full one plays");

        let own = base.with_clip(GunMoveRole::MeleeEmpty, reel("tactical_melee_empty", 20, false));
        let mut c = FirstPersonGunAnimator::new(own);
        go(&mut c, GunMoveRole::MeleeEmpty, 0.05);
        assert!(on(&c, GunMoveRole::MeleeEmpty) && !on(&c, GunMoveRole::Melee));
    }

    #[test]
    fn an_empty_gun_without_empty_clips_plays_the_full_ones() {
        let mut c = FirstPersonGunAnimator::new(gun(
            "ak74u_zm",
            &[
                (GunMoveRole::SprintLoop, "sprint_loop", 20, true),
                (GunMoveRole::CrawlIn, "crawl_in", 10, false),
                (GunMoveRole::CrawlBack, "crawl_back", 30, true),
            ],
        ));
        assert_eq!(
            go(&mut c, GunMoveRole::SprintLoopEmpty, 0.1),
            EventReaction::Started(GunPhase::SprintHold)
        );
        assert!(on(&c, GunMoveRole::SprintLoop));
        go(&mut c, GunMoveRole::Idle, 0.0);
        go(&mut c, GunMoveRole::CrawlBackEmpty, 0.1);
        assert!(on(&c, GunMoveRole::CrawlIn));
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::CrawlBack));
    }

    #[test]
    fn the_first_shots_play_their_own_clip_or_the_guns_fire() {
        let mut c = FirstPersonGunAnimator::new(gun(
            "hamr_mp",
            &[
                (GunMoveRole::Fire, "secondary_fire", 6, false),
                (GunMoveRole::FireIntro, "fire", 6, false),
            ],
        ));
        assert_eq!(
            go(&mut c, GunMoveRole::FireIntro, 0.01),
            EventReaction::Started(GunPhase::Firing)
        );
        assert!(on(&c, GunMoveRole::FireIntro));

        let mut c = FirstPersonGunAnimator::new(gun("an94_mp", &[(GunMoveRole::Fire, "fire", 6, false)]));
        assert_eq!(
            go(&mut c, GunMoveRole::FireIntro, 0.01),
            EventReaction::Started(GunPhase::Firing)
        );
        assert!(on(&c, GunMoveRole::Fire));
    }

    // ---- dive ------------------------------------------------------------------------

    /// The M14's dive as BO2 has it: 10, 15 and 10 frames over 200, 600 and 250 ms.
    fn m14(dive_clips: bool) -> FirstPersonGunAnimator {
        let dive: &[_] = if dive_clips {
            &[
                (GunMoveRole::DiveIn, "d2p_in", 10, false),
                (GunMoveRole::DiveLoop, "d2p_loop", 15, false),
                (GunMoveRole::DiveOut, "d2p_out", 10, false),
            ]
        } else {
            &[]
        };
        let mut weapon = gun("m14_zm", dive);
        weapon.dive_times_ms = [200, 600, 250];
        FirstPersonGunAnimator::new(weapon)
    }

    #[test]
    fn a_dive_takes_off_holds_in_the_air_then_lands_to_idle() {
        let mut c = m14(true);
        go(&mut c, GunMoveRole::DiveIn, 0.15);
        assert!(on(&c, GunMoveRole::DiveIn) && !on(&c, GunMoveRole::Idle));
        c.step_on(0.1);
        assert!(
            on(&c, GunMoveRole::DiveLoop) && off(&c, GunMoveRole::DiveIn),
            "take-off ran into the air"
        );
        c.step_on(2.0);
        assert!(on(&c, GunMoveRole::DiveLoop), "held until touch-down");
        go(&mut c, GunMoveRole::DiveOut, 0.2);
        assert!(on(&c, GunMoveRole::DiveOut));
        c.step_on(0.1);
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::DiveOut));
        assert_eq!(c.state(), GunPhase::Ready);
    }

    #[test]
    fn aiming_mid_dive_brings_the_gun_up_quickly() {
        let mut c = m14(true);
        go(&mut c, GunMoveRole::DiveIn, 0.1);
        c.apply_force_idle_weap_anim(false);
        c.step_on(0.1);
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::DiveIn));
    }

    #[test]
    fn a_gun_without_dive_clips_stays_up() {
        let mut c = m14(false);
        assert_eq!(
            c.dispatch_sz_xanim_idx(GunMoveRole::DiveIn.index()),
            EventReaction::Started(GunPhase::Ready)
        );
        c.step_on(0.6);
        assert!(on(&c, GunMoveRole::Idle));
    }

    #[test]
    fn a_shot_mid_dive_replaces_the_dive() {
        let mut c = m14(true);
        go(&mut c, GunMoveRole::DiveIn, 0.1);
        assert_eq!(
            c.hnd(GunAnimEvent::Fire),
            EventReaction::IgnoredMissingClip(GunMoveRole::Fire)
        );
        c.step_on(0.5);
        assert!(!on(&c, GunMoveRole::DiveLoop), "no in-air clip after a shot");
    }

    // ---- crawl -----------------------------------------------------------------------

    /// The 870's crawl as BO2 has it: in 10 frames over 300 ms, direction loops over
    /// 1000-1200 ms, out 10 frames over 100 ms, out-fire 48 ms.
    fn r870(crawl_clips: bool) -> FirstPersonGunAnimator {
        let crawl: &[_] = if crawl_clips {
            &[
                (GunMoveRole::CrawlIn, "crawl_in", 10, false),
                (GunMoveRole::CrawlForward, "crawl_forward", 30, true),
                (GunMoveRole::CrawlLeft, "crawl_left", 30, true),
                (GunMoveRole::CrawlOut, "crawl_out", 10, false),
            ]
        } else {
            &[]
        };
        let mut weapon = gun("870mcs_zm", crawl).with_clip(GunMoveRole::Fire, reel("fire", 10, false));
        weapon.crawl_times_ms = [300, 1100, 1000, 1200, 1100, 48, 100];
        FirstPersonGunAnimator::new(weapon)
    }

    #[test]
    fn a_crawl_plays_in_then_the_direction_then_out_to_idle() {
        let mut c = r870(true);
        go(&mut c, GunMoveRole::CrawlForward, 0.2);
        assert!(on(&c, GunMoveRole::CrawlIn) && !on(&c, GunMoveRole::Idle));
        c.step_on(0.15);
        assert!(on(&c, GunMoveRole::CrawlForward) && off(&c, GunMoveRole::CrawlIn));
        c.step_on(3.0);
        assert!(on(&c, GunMoveRole::CrawlForward), "the crawl loops");
        go(&mut c, GunMoveRole::CrawlOut, 0.05);
        assert!(on(&c, GunMoveRole::CrawlOut) && off(&c, GunMoveRole::CrawlForward));
        c.step_on(0.06);
        assert!(
            on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::CrawlOut),
            "out settled to idle"
        );
        assert_eq!(
            c.dispatch_sz_xanim_idx(GunMoveRole::CrawlOut.index()),
            EventReaction::IgnoredState(GunPhase::Ready),
            "a crawl-out with no crawl does nothing"
        );
    }

    #[test]
    fn a_new_crawl_direction_waits_for_the_crawl_in_then_blends_across() {
        let mut c = r870(true);
        go(&mut c, GunMoveRole::CrawlForward, 0.1);
        go(&mut c, GunMoveRole::CrawlLeft, 0.0);
        assert!(on(&c, GunMoveRole::CrawlIn), "the crawl-in plays on");
        c.step_on(0.25);
        assert!(
            on(&c, GunMoveRole::CrawlLeft) && off(&c, GunMoveRole::CrawlForward),
            "then the newest direction"
        );
        go(&mut c, GunMoveRole::CrawlForward, 0.05);
        let blend = c.animation_wt(GunMoveRole::CrawlForward);
        assert!(blend > 0.0 && blend < 1.0, "blending in, {blend}");
        c.step_on(0.2);
        assert!(on(&c, GunMoveRole::CrawlForward) && off(&c, GunMoveRole::CrawlLeft));
    }

    #[test]
    fn leaving_a_crawl_blends_over_the_out_fire_time() {
        let crawling = || {
            let mut c = r870(true);
            go(&mut c, GunMoveRole::CrawlForward, 0.5);
            c
        };
        let mut c = crawling();
        c.hnd(GunAnimEvent::Fire);
        c.step_on(0.02);
        let blend = c.animation_wt(GunMoveRole::Fire);
        assert!(blend > 0.0 && blend < 1.0, "a shot blends in, {blend}");
        c.step_on(0.04);
        assert!(on(&c, GunMoveRole::Fire) && off(&c, GunMoveRole::CrawlForward));

        let mut c = crawling();
        c.apply_force_idle_weap_anim(false);
        c.step_on(0.06);
        assert!(
            on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::CrawlForward),
            "aiming out of it"
        );

        let mut c = crawling();
        assert!(c.apply_resting_weap_anim(false), "the idle edge ends a crawl");
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::Idle));
    }

    #[test]
    fn a_gun_without_crawl_clips_stays_as_it_is() {
        let mut c = r870(false);
        assert_eq!(
            go(&mut c, GunMoveRole::CrawlForward, 1.0),
            EventReaction::Started(GunPhase::Ready)
        );
        assert!(on(&c, GunMoveRole::Idle));
    }

    // ---- run loop, fire, events ------------------------------------------------------

    #[test]
    fn a_run_loop_that_does_not_loop_starts_over_instead_of_freezing() {
        // The ballistic knife's: 1000 ms, not a looping clip, in a 600 ms sprint loop time.
        let mut weapon = gun(
            "knife_ballistic_mp",
            &[(GunMoveRole::SprintLoop, "sprint_loop", 30, false)],
        );
        weapon.run_cycle_millis = 600;
        let mut c = FirstPersonGunAnimator::new(weapon);
        go(&mut c, GunMoveRole::SprintLoop, 0.5);
        let mid = c.animation_when(GunMoveRole::SprintLoop);
        c.step_on(0.2);
        assert!(on(&c, GunMoveRole::SprintLoop));
        assert!(
            c.animation_when(GunMoveRole::SprintLoop) < mid,
            "the run loop starts over"
        );
    }

    #[test]
    fn events_are_refused_in_the_states_that_cannot_take_them() {
        let mut weapon = gun(
            "mp7_mp",
            &[
                (GunMoveRole::Fire, "fire", 6, false),
                (GunMoveRole::Reload, "reload", 60, false),
                (GunMoveRole::SprintIn, "sprint_in", 10, false),
                (GunMoveRole::SprintLoop, "sprint_loop", 20, true),
            ],
        );
        weapon.reload_duration_millis = 2000;
        let mut c = FirstPersonGunAnimator::new(weapon);
        assert_eq!(
            c.hnd(GunAnimEvent::ReloadInfo { empty: false }),
            EventReaction::Started(GunPhase::ReloadingInfo { empty: false })
        );
        assert_eq!(
            c.hnd(GunAnimEvent::Fire),
            EventReaction::IgnoredState(GunPhase::ReloadingInfo { empty: false })
        );
        assert_eq!(
            c.hnd(GunAnimEvent::SprintIn),
            EventReaction::IgnoredState(GunPhase::ReloadingInfo { empty: false })
        );
        // Sprinting out of anything else is always allowed, a second loop is not.
        assert_eq!(
            c.hnd(GunAnimEvent::SprintLoopData),
            EventReaction::Started(GunPhase::SprintHold)
        );
        assert_eq!(
            c.hnd(GunAnimEvent::SprintLoopData),
            EventReaction::IgnoredState(GunPhase::SprintHold)
        );
        assert_eq!(
            c.hnd(GunAnimEvent::SprintOut),
            EventReaction::IgnoredMissingClip(GunMoveRole::SprintOut)
        );
    }

    #[test]
    fn a_timed_action_plays_over_the_time_the_gun_gives_it() {
        let mut weapon = gun("mp7_mp", &[(GunMoveRole::Reload, "reload", 60, false)]);
        weapon.reload_duration_millis = 1000;
        let mut c = FirstPersonGunAnimator::new(weapon);
        c.hnd(GunAnimEvent::ReloadInfo { empty: false });
        c.step_on(0.5);
        // A 2 s clip over 1 s of reload time runs twice as fast.
        let rate_is_native = clip_keeps_authored_rate(GunMoveRole::Reload.index());
        let expected = if rate_is_native { 0.5 } else { 1.0 };
        assert!((c.animation_when(GunMoveRole::Reload) - expected).abs() < 0.05);
        assert_eq!(c.state(), GunPhase::ReloadingInfo { empty: false });
        c.step_on(0.6);
        assert!(
            matches!(c.state(), GunPhase::ReloadingInfo { empty: false }),
            "reloads end by the next edge"
        );
    }

    #[test]
    fn a_shot_settles_when_the_clip_is_out_and_the_trigger_is_up() {
        let mut weapon = gun("mp7_mp", &[(GunMoveRole::Fire, "fire", 30, false)]);
        weapon.fire_time_ms = 100;
        weapon.fire_plays_out = true;
        let mut c = FirstPersonGunAnimator::new(weapon);
        c.hnd(GunAnimEvent::Fire);
        c.step_on(0.2);
        assert_eq!(c.state(), GunPhase::Ready, "the fire time is out");
        assert!(c.animation_when(GunMoveRole::Fire) < 1.0, "the recoil clip is not");
        c.relax_firing_to_idle();
        assert!(
            on(&c, GunMoveRole::Fire),
            "the trigger came up, but the recoil plays out"
        );
        c.step_on(1.0);
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::Fire), "then it settles");
    }

    #[test]
    fn a_shot_that_does_not_play_out_settles_at_once() {
        let mut weapon = gun("mp7_mp", &[(GunMoveRole::Fire, "fire", 30, false)]);
        weapon.fire_time_ms = 100;
        let mut c = FirstPersonGunAnimator::new(weapon);
        c.hnd(GunAnimEvent::Fire);
        c.step_on(0.2);
        c.relax_firing_to_idle();
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::Fire));
        // With nothing fired there is nothing to settle.
        let mut c = FirstPersonGunAnimator::new(gun("mp7_mp", &[]));
        c.relax_firing_to_idle();
        assert_eq!(c.state(), GunPhase::Ready);
    }

    #[test]
    fn the_idle_edge_waits_for_a_playing_clip_and_forced_idle_cuts_in() {
        let mut weapon = gun("mp7_mp", &[(GunMoveRole::Reload, "reload", 60, false)]);
        weapon.reload_duration_millis = 2000;
        let mut c = FirstPersonGunAnimator::new(weapon);
        c.hnd(GunAnimEvent::ReloadInfo { empty: false });
        c.step_on(0.5);
        assert!(!c.apply_resting_weap_anim(false), "the reload still plays");
        c.apply_force_idle_weap_anim(false);
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::Reload));
        assert_eq!(c.state(), GunPhase::Ready);
    }

    #[test]
    fn an_empty_idle_is_used_only_when_the_gun_has_one() {
        let mut c = FirstPersonGunAnimator::new(gun("a", &[(GunMoveRole::EmptyIdle, "empty_idle", 30, true)]));
        c.apply_force_idle_weap_anim(true);
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::EmptyIdle));
        c.apply_force_idle_weap_anim(false);
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::Idle) && off(&c, GunMoveRole::EmptyIdle));
        let mut c = FirstPersonGunAnimator::new(gun("b", &[]));
        c.apply_force_idle_weap_anim(true);
        c.step_on(0.5);
        assert!(on(&c, GunMoveRole::Idle));
    }

    // ---- aim overlay -----------------------------------------------------------------

    #[test]
    fn the_aim_overlay_follows_the_aim_by_the_guns_convention() {
        let mut weapon = gun("mp7_mp", &[(GunMoveRole::AdsUp, "ads_up", 7, false)]);
        weapon.ads_overlay = ScopeOverlayStyle::WeightFractional;
        let mut c = FirstPersonGunAnimator::new(weapon.clone());
        c.apply_ads_overlay_frm(0.0);
        c.step_on(0.2);
        assert_eq!(c.animation_wt(GunMoveRole::AdsUp), 0.0);
        c.apply_ads_overlay_frm(1.0);
        c.step_on(0.2);
        assert!(
            c.animation_wt(GunMoveRole::AdsUp) > 0.9,
            "the weight follows the aim"
        );

        weapon.ads_overlay = ScopeOverlayStyle::StartAimMotion;
        let mut c = FirstPersonGunAnimator::new(weapon);
        assert!(
            on(&c, GunMoveRole::AdsUp),
            "a gun with only an aim-up clip plays it at the hip"
        );
        c.apply_ads_overlay_frm(0.0);
        assert!(on(&c, GunMoveRole::AdsUp));
    }

    #[test]
    fn a_melee_clears_the_aim_overlay() {
        let mut c = FirstPersonGunAnimator::new(gun(
            "mp7_mp",
            &[
                (GunMoveRole::Melee, "melee", 20, false),
                (GunMoveRole::AdsUp, "ads_up", 7, false),
                (GunMoveRole::AdsDown, "ads_down", 7, false),
            ],
        ));
        assert!(on(&c, GunMoveRole::AdsDown), "the lowered pose is held at the hip");
        go(&mut c, GunMoveRole::Melee, 0.05);
        c.apply_ads_overlay_frm(0.5);
        assert!(off(&c, GunMoveRole::AdsUp) && off(&c, GunMoveRole::AdsDown));
    }

    #[test]
    fn a_shot_while_aiming_uses_the_aim_fire_clip_when_there_is_one() {
        let mut c = FirstPersonGunAnimator::new(gun(
            "mp7_mp",
            &[
                (GunMoveRole::Fire, "fire", 6, false),
                (GunMoveRole::AdsFire, "ads_fire", 6, false),
            ],
        ));
        c.apply_ads_overlay_frm(1.0);
        c.hnd(GunAnimEvent::Fire);
        assert!(on(&c, GunMoveRole::AdsFire) && off(&c, GunMoveRole::Fire));
    }

    // ---- BO2 pose --------------------------------------------------------------------

    #[test]
    fn a_bo2_pose_sets_each_slot_as_given_and_clears_the_rest() {
        let mut c = FirstPersonGunAnimator::new(gun(
            "mp7_mp",
            &[
                (GunMoveRole::Reload, "reload", 54, false),
                (GunMoveRole::AdsUp, "ads_up", 6, false),
            ],
        ));
        let pose = c.apply_bo2_pose([
            ("reload", 0.75, 0.5),
            ("idle", 0.25, 1.0),
            ("ads_up", 1.0, 0.0),
            ("reload", 0.1, 0.9),
            ("idle_left", 1.0, 0.0),
        ]);
        assert_eq!(pose.unplaced, ["idle_left"]);
        assert_eq!(c.animation_wt(GunMoveRole::Reload), 0.75);
        assert!(
            (c.animation_when(GunMoveRole::Reload) - 0.9).abs() < 1e-5,
            "the clip is 1.8 s long"
        );
        assert_eq!(c.animation_wt(GunMoveRole::Idle), 0.25);
        assert_eq!(c.animation_wt(GunMoveRole::AdsUp), 1.0);
        assert_eq!(c.animation_wt(GunMoveRole::AdsDown), 0.0);

        // BO2's player owns the clock: advancing moves nothing that was posed.
        c.step_on(0.1);
        assert!((c.animation_when(GunMoveRole::Reload) - 0.9).abs() < 1e-5);

        assert!(c.apply_bo2_pose([("idle", 1.0, 0.2)]).unplaced.is_empty());
        assert!(off(&c, GunMoveRole::Reload) && off(&c, GunMoveRole::AdsUp));
    }

    #[test]
    fn a_bo2_slot_with_no_clip_draws_nothing() {
        let mut c = FirstPersonGunAnimator::new(
            GunClipSet::blank("fiveseven_mp").with_clip(GunMoveRole::Idle, reel("idle", 30, false)),
        );
        assert_eq!(
            c.apply_bo2_pose([("sprint_in", 1.0, 0.0)]).unplaced,
            ["sprint_in"]
        );
        assert_eq!(c.active_anims().count(), 0);
    }

    #[test]
    fn a_bo2_pose_reports_the_notetracks_it_passes() {
        let mut reload = (*reel("reload", 60, false)).clone();
        reload.notifies = vec![
            ClipEventMark {
                name: "mag_out".into(),
                time: 0.25,
            },
            ClipEventMark {
                name: "mag_in".into(),
                time: 0.75,
            },
        ];
        let weapon = GunClipSet::blank("mp7_mp").with_clip(GunMoveRole::Reload, Arc::new(reload));
        let mut c = FirstPersonGunAnimator::new(weapon);
        let mut notes = |frac, weight| c.apply_bo2_pose([("reload", weight, frac)]).notify_list;
        assert!(notes(0.0, 1.0).is_empty());
        assert_eq!(notes(0.5, 1.0), ["mag_out"]);
        assert!(notes(0.6, 1.0).is_empty());
        assert_eq!(notes(1.0, 1.0), ["mag_in"]);
        assert_eq!(
            notes(0.3, 1.0),
            ["mag_out"],
            "a restart plays from the top again"
        );
        assert!(notes(0.9, 0.0).is_empty(), "a fading slot says nothing");
    }

    #[test]
    fn replacing_a_clip_keeps_its_weight_and_play_position() {
        let mut c = FirstPersonGunAnimator::new(gun("mp7_mp", &[(GunMoveRole::SprintLoop, "sprint", 20, true)]));
        c.apply_bo2_pose([("sprint_loop", 0.6, 0.5)]);
        let before = c.animation_when(GunMoveRole::SprintLoop);
        c.replace_clip(GunMoveRole::SprintLoop, reel("sprint_smooth", 20, true));
        assert_eq!(c.animation_when(GunMoveRole::SprintLoop), before);
        assert_eq!(c.animation_wt(GunMoveRole::SprintLoop), 0.6);
        assert_eq!(
            c.weapon().reel(GunMoveRole::SprintLoop).map(|c| c.name.as_str()),
            Some("sprint_smooth")
        );
    }
}
