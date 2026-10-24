//! The held gun's animation controller: which weapon clip plays, how it blends and when
//! it is over. It reads events from the first-person driver (`fpv`), or is posed clip by
//! clip straight from Black Ops II's own anim player (`apply_bo2_pose`).

use std::sync::Arc;

use asset_anim::{ActiveAnim, AnimClip, ClipScheduler};
use asset_game::{
    ACTION_GOAL_TIME_SECS, ACTIVE_GOAL_WEIGHT, AdsOverlayConvention, IDLE_INTERRUPT_GOAL_TIME_SECS,
    INACTIVE_GOAL_WEIGHT, WEAPON_ANIM_SLOTS, WeaponAnimSlot, WeaponAnimations, playback_rate,
    slot_uses_native_rate,
};

use WeaponAnimSlot as S;

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
        .filter(|&node| node != S::CameraMantle.index())
}

/// The dive animations, full magazine then empty: take-off, in the air, touch-down.
const DIVE_SLOTS: [[WeaponAnimSlot; 3]; 2] = [
    [S::DiveIn, S::DiveLoop, S::DiveOut],
    [S::DiveInEmpty, S::DiveLoopEmpty, S::DiveOutEmpty],
];
const DIVE_AIR: usize = 1;
const DIVE_LAND: usize = 2;

/// The crawl animations, full magazine then empty: in, forward, back, right, left, out.
const CRAWL_SLOTS: [[WeaponAnimSlot; 6]; 2] = [
    [S::CrawlIn, S::CrawlForward, S::CrawlBack, S::CrawlRight, S::CrawlLeft, S::CrawlOut],
    [
        S::CrawlInEmpty,
        S::CrawlForwardEmpty,
        S::CrawlBackEmpty,
        S::CrawlRightEmpty,
        S::CrawlLeftEmpty,
        S::CrawlOutEmpty,
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
    families: &[[WeaponAnimSlot; N]; 2],
    slot: WeaponAnimSlot,
) -> Option<(bool, usize)> {
    families.iter().enumerate().find_map(|(empty, parts)| {
        parts.iter().position(|&s| s == slot).map(|part| (empty == 1, part))
    })
}

fn crawl_part(slot: WeaponAnimSlot) -> Option<(bool, usize)> {
    family_part(&CRAWL_SLOTS, slot)
}

fn dive_part(slot: WeaponAnimSlot) -> Option<(bool, usize)> {
    family_part(&DIVE_SLOTS, slot)
}

/// The slot a gun falls back on when it has no clip for `slot`: the full-magazine sprint,
/// melee or crawl for an empty one, and its plain fire for a first-shots fire.
fn fallback_slot(slot: WeaponAnimSlot) -> Option<WeaponAnimSlot> {
    Some(match slot {
        S::SprintInEmpty => S::SprintIn,
        S::SprintLoopEmpty => S::SprintLoop,
        S::SprintOutEmpty => S::SprintOut,
        S::MeleeEmpty => S::Melee,
        S::MeleeChargeEmpty => S::MeleeCharge,
        S::FireIntro => S::Fire,
        S::AdsFireIntro => S::AdsFire,
        _ => {
            let (empty, part) = crawl_part(slot)?;
            return empty.then_some(CRAWL_SLOTS[0][part]);
        }
    })
}

/// BO2's slot names, and the clip slot each one plays on. BO2 slots this game has no clip
/// slot for (lowready, cont_fire, melee1-3, reload_right, finalshot, deploy, breakdown,
/// detonate, nightvision, slide_in, mantle, the sprint and dive cameras, the left hand's
/// slots, ads_up_other_scope) are not here.
const BO2_SLOT_NAMES: [(&str, WeaponAnimSlot); 59] = [
    ("idle", S::Idle),
    ("empty_idle", S::EmptyIdle),
    ("fire_intro", S::FireIntro),
    ("fire", S::Fire),
    ("hold_fire", S::HoldFire),
    ("lastshot", S::LastShot),
    ("rechamber", S::Rechamber),
    ("melee", S::Melee),
    ("melee_empty", S::MeleeEmpty),
    ("melee_charge", S::MeleeCharge),
    ("melee_charge_empty", S::MeleeChargeEmpty),
    ("reload", S::Reload),
    ("reload_empty", S::ReloadEmpty),
    ("reload_start", S::ReloadStart),
    ("reload_end", S::ReloadEnd),
    ("reload_quick", S::ReloadQuick),
    ("reload_quick_empty", S::ReloadQuickEmpty),
    ("raise", S::Raise),
    ("first_raise", S::FirstRaise),
    ("drop", S::Drop),
    ("alt_raise", S::AltRaise),
    ("alt_drop", S::AltDrop),
    ("quick_raise", S::QuickRaise),
    ("quick_drop", S::QuickDrop),
    ("empty_raise", S::EmptyRaise),
    ("empty_drop", S::EmptyDrop),
    ("sprint_in", S::SprintIn),
    ("sprint_loop", S::SprintLoop),
    ("sprint_out", S::SprintOut),
    ("sprint_empty_in", S::SprintInEmpty),
    ("sprint_empty_loop", S::SprintLoopEmpty),
    ("sprint_empty_out", S::SprintOutEmpty),
    ("crawl_in", S::CrawlIn),
    ("crawl_forward", S::CrawlForward),
    ("crawl_back", S::CrawlBack),
    ("crawl_right", S::CrawlRight),
    ("crawl_left", S::CrawlLeft),
    ("crawl_out", S::CrawlOut),
    ("crawl_empty_in", S::CrawlInEmpty),
    ("crawl_empty_forward", S::CrawlForwardEmpty),
    ("crawl_empty_back", S::CrawlBackEmpty),
    ("crawl_empty_right", S::CrawlRightEmpty),
    ("crawl_empty_left", S::CrawlLeftEmpty),
    ("crawl_empty_out", S::CrawlOutEmpty),
    ("ads_fire", S::AdsFire),
    ("ads_lastshot", S::AdsLastShot),
    ("ads_fire_intro", S::AdsFireIntro),
    ("ads_rechamber", S::AdsRechamber),
    ("dtp_in", S::DiveIn),
    ("dtp_loop", S::DiveLoop),
    ("dtp_out", S::DiveOut),
    ("dtp_empty_in", S::DiveInEmpty),
    ("dtp_empty_loop", S::DiveLoopEmpty),
    ("dtp_empty_out", S::DiveOutEmpty),
    ("camera_mantle", S::CameraMantle),
    ("ads_up", S::AdsUp),
    ("ads_down", S::AdsDown),
    ("ads_up_other", S::AdsUp),
    ("ads_down_other", S::AdsDown),
];

/// The clip slot for a Black Ops II viewmodel slot, by BO2's slot name.
pub fn bo2_slot(name: &str) -> Option<WeaponAnimSlot> {
    BO2_SLOT_NAMES.iter().find(|(known, _)| *known == name).map(|&(_, slot)| slot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewmodelEvent {
    Fire,
    Raise { first: bool, quick: bool },
    Reload { empty: bool },
    ReloadStart,
    ReloadEnd,
    Rechamber,
    Drop,
    SprintIn,
    SprintLoop,
    SprintOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponState {
    Ready,
    Raising { first: bool, quick: bool },
    Firing,
    Reloading { empty: bool },
    ReloadStarting,
    ReloadEnding,
    Rechambering,
    Dropping,
    SprintIn,
    SprintLoop,
    SprintOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Started(WeaponState),
    IgnoredState(WeaponState),
    IgnoredMissingClip(WeaponAnimSlot),
}

/// What [`ViewmodelController::apply_bo2_pose`] did.
#[derive(Debug, Clone, Default)]
pub struct Bo2Pose<'a> {
    /// BO2 slots with no clip here (they draw nothing).
    pub unplaced: Vec<&'a str>,
    /// Notetracks the posed clips passed.
    pub notifies: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AdvanceResult {
    pub notifies: Vec<String>,
}

fn positive_ms(ms: i32) -> Option<i32> {
    (ms > 0).then_some(ms)
}

#[derive(Debug)]
pub struct ViewmodelController {
    weapon: WeaponAnimations,
    tree: ClipScheduler,
    state: WeaponState,
    /// The slot of the action being played; `None` while idling.
    action: Option<WeaponAnimSlot>,
    /// Seconds the action's timer has left; `None` when it has no timer or loops.
    action_remaining: Option<f32>,
    /// How far up the aim overlay was last frame, for choosing a fire clip.
    last_ads_frac: f32,
    predicted_perks0: u32,
    /// The trigger came up while a fire animation still played: settle to idle when it ends.
    settle_when_done: bool,
    /// The crawl direction to play when the crawl-in ends.
    crawl_next: Option<WeaponAnimSlot>,
}

impl ViewmodelController {
    pub fn new(weapon: WeaponAnimations) -> Self {
        let mut tree = ClipScheduler::new(WEAPON_ANIM_SLOTS);
        weapon.install_clips(&mut tree).expect("weapon animation index is within tree");
        let mut this = Self {
            weapon,
            tree,
            state: WeaponState::Ready,
            action: None,
            action_remaining: None,
            last_ads_frac: 0.0,
            predicted_perks0: 0,
            settle_when_done: false,
            crawl_next: None,
        };
        this.set_weight(S::Idle, ACTIVE_GOAL_WEIGHT, ACTION_GOAL_TIME_SECS);

        // Bones only the aim clips animate need their lowered pose at the hip.
        if let Some(down) = this.weapon.clip(S::AdsDown).map(|c| c.duration()) {
            this.set_weight(S::AdsDown, ACTIVE_GOAL_WEIGHT, 0.0);
            this.put_time(S::AdsDown.index(), down);
            this.put_rate(S::AdsDown.index(), 0.0);
        } else if this.weapon.ads_overlay == AdsOverlayConvention::PlayAdsAnim {
            this.set_weight(S::AdsUp, ACTIVE_GOAL_WEIGHT, 0.0);
        }
        this
    }

    pub fn state(&self) -> WeaponState {
        self.state
    }

    pub fn set_predicted_perks(&mut self, perks0: u32) {
        self.predicted_perks0 = perks0;
    }

    pub fn weapon_name(&self) -> &str {
        &self.weapon.name
    }

    pub fn weapon(&self) -> &WeaponAnimations {
        &self.weapon
    }

    /// Swaps one slot's clip in place (the smoothed run loop), keeping its weight and
    /// play position.
    pub fn replace_clip(&mut self, slot: WeaponAnimSlot, clip: Arc<AnimClip>) {
        let weapon = std::mem::replace(&mut self.weapon, WeaponAnimations::empty(""));
        self.weapon = weapon.with_clip(slot, Arc::clone(&clip));
        let time = self.tree.time(slot.index()).unwrap_or(0.0);
        if self.tree.set_clip(slot.index(), clip).is_ok() {
            self.put_time(slot.index(), time);
        }
    }

    pub fn active_anims(&self) -> impl Iterator<Item = ActiveAnim<'_>> {
        self.tree.active()
    }

    pub fn animation_weight(&self, slot: WeaponAnimSlot) -> f32 {
        self.tree.weight(slot.index()).unwrap_or(0.0)
    }

    pub fn animation_time(&self, slot: WeaponAnimSlot) -> f32 {
        self.tree.time(slot.index()).unwrap_or(0.0)
    }

    // -----------------------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------------------

    fn event_allowed(&self, event: ViewmodelEvent) -> bool {
        use WeaponState as W;
        match event {
            ViewmodelEvent::Fire => matches!(self.state, W::Ready | W::Firing),
            ViewmodelEvent::SprintLoop => self.state != W::SprintLoop,
            ViewmodelEvent::SprintIn => matches!(self.state, W::Ready | W::Firing | W::SprintOut),
            _ => true,
        }
    }

    pub fn handle(&mut self, event: ViewmodelEvent) -> EventResult {
        if !self.event_allowed(event) {
            return EventResult::IgnoredState(self.state);
        }
        let slot = match event {
            ViewmodelEvent::Fire if self.last_ads_frac > 0.0 && self.has_clip(S::AdsFire) => {
                S::AdsFire
            }
            ViewmodelEvent::Fire => S::Fire,
            ViewmodelEvent::Raise { first: true, .. } => S::FirstRaise,
            ViewmodelEvent::Raise { quick: true, .. } => S::QuickRaise,
            ViewmodelEvent::Raise { .. } => S::Raise,
            ViewmodelEvent::Reload { empty: true } => S::ReloadEmpty,
            ViewmodelEvent::Reload { empty: false } => S::Reload,
            ViewmodelEvent::ReloadStart => S::ReloadStart,
            ViewmodelEvent::ReloadEnd => S::ReloadEnd,
            ViewmodelEvent::Rechamber => S::Rechamber,
            ViewmodelEvent::Drop => S::Drop,
            ViewmodelEvent::SprintIn => S::SprintIn,
            ViewmodelEvent::SprintLoop => S::SprintLoop,
            ViewmodelEvent::SprintOut => S::SprintOut,
        };
        let (mut state, timer) = self.plan_for(slot);
        if let ViewmodelEvent::Raise { first, quick } = event {
            state = WeaponState::Raising { first, quick };
        }
        self.start_action(slot, state, timer)
    }

    /// Plays the clip node `node_index` as the viewmodel anim names it.
    pub fn dispatch_sz_xanim_index(&mut self, node_index: usize) -> EventResult {
        let Some(slot) = WeaponAnimSlot::from_index(node_index) else {
            return EventResult::IgnoredState(self.state);
        };
        // The aim clips and the climb camera are posed by their own drivers.
        if matches!(slot, S::AdsUp | S::AdsDown | S::CameraMantle) {
            return EventResult::IgnoredState(self.state);
        }
        if matches!(slot, S::Idle | S::EmptyIdle) {
            self.start_idle_family(false, false);
            return EventResult::Started(WeaponState::Ready);
        }
        // A gun without the dive animations comes straight up.
        if let Some((empty, _)) = dive_part(slot)
            && !self.has_clip(slot)
        {
            self.start_idle_family(empty, true);
            return EventResult::Started(WeaponState::Ready);
        }
        let slot = self.clip_or_fallback(slot);
        if let Some((empty, part)) = crawl_part(slot) {
            return self.dispatch_crawl(slot, empty, part);
        }
        let (state, timer) = self.plan_for(slot);
        self.start_action(slot, state, timer)
    }

    fn has_clip(&self, slot: WeaponAnimSlot) -> bool {
        self.weapon.clip(slot).is_some()
    }

    /// `slot`, or the one standing in for it when the gun has no clip for it.
    fn clip_or_fallback(&self, slot: WeaponAnimSlot) -> WeaponAnimSlot {
        match fallback_slot(slot) {
            Some(fallback) if !self.has_clip(slot) => fallback,
            _ => slot,
        }
    }

    /// The state an action in `slot` leaves the gun in, and the time (ms) the gun's
    /// facts give it, if any.
    fn plan_for(&self, slot: WeaponAnimSlot) -> (WeaponState, Option<i32>) {
        use WeaponState as W;
        let w = &self.weapon;
        let raising = |first, quick| W::Raising { first, quick };
        let (state, ms) = match slot {
            S::Fire | S::AdsFire | S::LastShot | S::AdsLastShot | S::HoldFire => {
                (W::Firing, w.fire_time_ms)
            }
            S::FireIntro | S::AdsFireIntro => (W::Firing, w.intro_fire_time_ms),
            S::FirstRaise => (raising(true, false), 0),
            S::QuickRaise => (raising(false, true), w.quick_raise_time_ms),
            S::AltRaise => (raising(false, false), w.alternate_raise_time_ms),
            S::EmptyRaise => (raising(false, false), 0),
            S::Raise => (raising(false, false), w.raise_time_ms),
            S::Reload => (W::Reloading { empty: false }, w.reload_time_ms),
            S::ReloadEmpty => (W::Reloading { empty: true }, w.reload_empty_time_ms),
            S::ReloadQuick => (W::Reloading { empty: false }, w.reload_quick_time_ms),
            S::ReloadQuickEmpty => (W::Reloading { empty: true }, w.reload_quick_empty_time_ms),
            S::ReloadStart => (W::ReloadStarting, w.reload_start_time_ms),
            S::ReloadEnd => (W::ReloadEnding, w.reload_end_time_ms),
            S::Rechamber | S::AdsRechamber => (W::Rechambering, 0),
            S::Drop => (W::Dropping, w.drop_time_ms),
            S::QuickDrop => (W::Dropping, w.quick_drop_time_ms),
            S::AltDrop => (W::Dropping, w.alternate_drop_time_ms),
            S::EmptyDrop => (W::Dropping, 0),
            S::SprintIn | S::SprintInEmpty => (W::SprintIn, w.sprint_raise_time_ms),
            S::SprintLoop | S::SprintLoopEmpty => (W::SprintLoop, w.sprint_loop_time_ms),
            S::SprintOut | S::SprintOutEmpty => (W::SprintOut, w.sprint_drop_time_ms),
            S::Melee | S::MeleeEmpty => (W::Ready, w.melee_time_ms),
            S::MeleeCharge | S::MeleeChargeEmpty => (W::Ready, w.melee_charge_time_ms),
            S::Idle | S::EmptyIdle | S::AdsUp | S::AdsDown | S::CameraMantle => (W::Ready, 0),
            _ => {
                let ms = match (crawl_part(slot), dive_part(slot)) {
                    (Some((_, part)), _) => w.crawl_times_ms[CRAWL_TIME_AT[part]],
                    (_, Some((_, part))) => w.dive_times_ms[part],
                    _ => 0,
                };
                (W::Ready, ms)
            }
        };
        (state, positive_ms(ms))
    }

    /// A reload timer shortened by the fast-reload perk the local player holds.
    fn perk_scaled(&self, state: WeaponState, timer_ms: Option<i32>) -> Option<i32> {
        let reloading = matches!(
            state,
            WeaponState::Reloading { .. } | WeaponState::ReloadStarting | WeaponState::ReloadEnding
        );
        let multiplier = weapon_t6::combat_facts::PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT;
        let perk = weapon_t6::combat_facts::perk_fastreload_eligible(
            self.predicted_perks0,
            self.weapon.inherits_perks,
        );
        if !(reloading && perk && multiplier > 0.0) {
            return timer_ms;
        }
        timer_ms.map(|ms| (ms as f32 * multiplier).round() as i32)
    }

    // -----------------------------------------------------------------------------------
    // Crawl
    // -----------------------------------------------------------------------------------

    /// A crawl slot asked for. From anything else a direction plays the crawl-in first and
    /// then the direction the player crawls by then; a new direction blends across; the
    /// crawl-out settles to idle. A gun without the clip keeps what it plays.
    fn dispatch_crawl(&mut self, slot: WeaponAnimSlot, empty: bool, part: usize) -> EventResult {
        if !self.has_clip(slot) {
            return EventResult::Started(self.state);
        }
        let playing = self.action.and_then(crawl_part);
        if part == CRAWL_OUT {
            if playing.is_none() {
                return EventResult::IgnoredState(self.state);
            }
            return self.start_crawl(slot, self.crawl_out_fire_secs());
        }
        match playing {
            Some((_, CRAWL_IN)) => {
                self.crawl_next = Some(slot);
                EventResult::Started(self.state)
            }
            Some(_) => self.start_crawl(slot, CRAWL_TURN_SECS),
            None => {
                let crawl_in = self.clip_or_fallback(CRAWL_SLOTS[usize::from(empty)][CRAWL_IN]);
                if !self.has_clip(crawl_in) {
                    return self.start_crawl(slot, CRAWL_TURN_SECS);
                }
                let started = self.start_crawl(crawl_in, ACTION_GOAL_TIME_SECS);
                self.crawl_next = Some(slot);
                started
            }
        }
    }

    fn start_crawl(&mut self, slot: WeaponAnimSlot, goal_secs: f32) -> EventResult {
        let (state, timer) = self.plan_for(slot);
        self.start_action_over(slot, state, timer, goal_secs)
    }

    /// Whether a crawl-in or a crawl direction plays (the crawl-out does not count).
    fn crawling(&self) -> bool {
        self.action.and_then(crawl_part).is_some_and(|(_, part)| part != CRAWL_OUT)
    }

    fn crawl_out_fire_secs(&self) -> f32 {
        positive_ms(self.weapon.crawl_times_ms[CRAWL_OUT_FIRE_TIME])
            .map_or(CRAWL_OUT_FIRE_DEFAULT_SECS, |ms| ms as f32 / 1000.0)
    }

    // -----------------------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------------------

    pub fn advance(&mut self, dt_secs: f32) -> AdvanceResult {
        let dt_secs = dt_secs.max(0.0);
        let notifies = self.tree.advance(dt_secs);
        if let Some(remaining) = self.action_remaining.as_mut() {
            *remaining = (*remaining - dt_secs).max(0.0);
        }
        self.chain_dive();
        self.chain_crawl();
        self.restart_ended_run_loop();

        if self.state == WeaponState::Firing
            && let Some(slot) = self.action
            && self.action_finished(slot)
        {
            self.action = None;
            self.action_remaining = None;
            self.state = WeaponState::Ready;
        }
        // A fire animation that outlasted the trigger settles to idle once played out.
        if self.settle_when_done {
            if self.state != WeaponState::Ready || self.action.is_some() {
                self.settle_when_done = false;
            } else if !self.action_clips_unfinished() {
                self.settle_when_done = false;
                self.start_idle_family(false, false);
            }
        }
        AdvanceResult { notifies }
    }

    /// The dive: take-off runs into the in-air clip, which holds its last frame until
    /// touch-down; touch-down settles to idle.
    fn chain_dive(&mut self) {
        let Some(slot) = self.action else { return };
        let Some((empty, part)) = dive_part(slot) else { return };
        if part == DIVE_AIR || !self.action_finished(slot) {
            return;
        }
        if part == DIVE_LAND {
            self.start_idle_family(empty, false);
            return;
        }
        let air = DIVE_SLOTS[usize::from(empty)][DIVE_AIR];
        if self.has_clip(air) {
            let (state, timer) = self.plan_for(air);
            self.start_action(air, state, timer);
        }
    }

    /// The crawl: the crawl-in runs into the direction the player crawls; the crawl-out
    /// settles to idle.
    fn chain_crawl(&mut self) {
        let Some(slot) = self.action else { return };
        let Some((empty, part)) = crawl_part(slot) else { return };
        if !matches!(part, CRAWL_IN | CRAWL_OUT) || !self.action_finished(slot) {
            return;
        }
        match self.crawl_next.take() {
            Some(next) if part == CRAWL_IN => {
                self.start_crawl(next, ACTION_GOAL_TIME_SECS);
            }
            _ => self.start_idle_family(empty, false),
        }
    }

    /// A run loop that is not a looping clip (the ballistic knife's is 1000 ms inside a
    /// 600 ms sprint loop time) starts over each time it ends, as BO2 replays it, instead
    /// of holding its last frame.
    fn restart_ended_run_loop(&mut self) {
        if self.state != WeaponState::SprintLoop {
            return;
        }
        let Some(slot) = self.action else { return };
        let one_shot = self.weapon.clip(slot).is_some_and(|clip| !clip.looping);
        if matches!(slot, S::SprintLoop | S::SprintLoopEmpty) && one_shot && self.action_finished(slot)
        {
            let (state, timer) = self.plan_for(slot);
            self.start_action_over(slot, state, timer, 0.0);
        }
    }

    /// Whether the action's own timer is out, or (with none) its clip has played to the end.
    fn action_finished(&self, slot: WeaponAnimSlot) -> bool {
        if let Some(remaining) = self.action_remaining {
            return remaining <= 0.0;
        }
        match self.weapon.clip(slot) {
            Some(clip) => !clip.looping && self.animation_time(slot) >= clip.duration(),
            None => true,
        }
    }

    /// Whether any clip in the action slots has weight and has not played out (a looping
    /// clip never has).
    fn action_clips_unfinished(&self) -> bool {
        action_slots().any(|node| {
            let weight = self.tree.weight(node).unwrap_or(0.0);
            let Some(clip) = self.weapon.clip_at(node).filter(|_| weight > 0.0) else {
                return false;
            };
            clip.looping || self.tree.time(node).unwrap_or(0.0) < clip.duration()
        })
    }

    // -----------------------------------------------------------------------------------
    // Idle and settling
    // -----------------------------------------------------------------------------------

    /// The viewmodel anim went to idle. `false` when an action clip is still playing and
    /// the idle has to wait; a crawl is ended quickly instead.
    pub fn apply_idle_weap_anim(&mut self, empty_mag: bool) -> bool {
        if self.crawling() {
            self.start_idle_family(empty_mag, true);
            return true;
        }
        if self.action_clips_unfinished() {
            return false;
        }
        self.start_idle_family(empty_mag, false);
        true
    }

    /// The viewmodel anim forced idle: it cuts into whatever plays (a dive, a crawl and a
    /// running action blend out faster).
    pub fn apply_force_idle_weap_anim(&mut self, empty_mag: bool) {
        if self.action.is_some_and(|slot| dive_part(slot).is_some()) {
            self.start_idle_family_over(empty_mag, DIVE_INTERRUPT_SECS);
        } else if self.action.is_some_and(|slot| crawl_part(slot).is_some()) {
            self.start_idle_family_over(empty_mag, self.crawl_out_fire_secs());
        } else {
            let interrupt = self.action_clips_unfinished();
            self.start_idle_family(empty_mag, interrupt);
        }
    }

    /// The trigger came up. A fire clip that has weight settles to idle; one the gun's
    /// facts say plays out (a recoil longer than the fire time) is left to finish first.
    pub fn settle_fire_to_idle(&mut self) {
        if self.state != WeaponState::Ready || self.action.is_some() {
            return;
        }
        if self.weapon.fire_plays_out && self.action_clips_unfinished() {
            self.settle_when_done = true;
            return;
        }
        if self.animation_weight(S::Fire) > 0.01 || self.animation_weight(S::FireIntro) > 0.01 {
            self.start_idle_family(false, false);
        }
    }

    fn start_idle_family(&mut self, empty: bool, interrupt: bool) {
        let goal_secs =
            if interrupt { IDLE_INTERRUPT_GOAL_TIME_SECS } else { ACTION_GOAL_TIME_SECS };
        self.start_idle_family_over(empty, goal_secs);
    }

    fn start_idle_family_over(&mut self, empty: bool, goal_secs: f32) {
        let slot = if empty && self.has_clip(S::EmptyIdle) { S::EmptyIdle } else { S::Idle };
        self.action = None;
        self.action_remaining = None;
        self.crawl_next = None;
        self.state = WeaponState::Ready;
        self.clear_action_slots(goal_secs);
        self.set_weight(slot, ACTIVE_GOAL_WEIGHT, goal_secs);
    }

    // -----------------------------------------------------------------------------------
    // Starting an action
    // -----------------------------------------------------------------------------------

    fn start_action(
        &mut self,
        slot: WeaponAnimSlot,
        state: WeaponState,
        timer_ms: Option<i32>,
    ) -> EventResult {
        // A shot (or anything else) out of a crawl blends from it over the crawl out-fire time.
        let goal_secs =
            if self.crawling() { self.crawl_out_fire_secs() } else { ACTION_GOAL_TIME_SECS };
        self.start_action_over(slot, state, timer_ms, goal_secs)
    }

    fn start_action_over(
        &mut self,
        slot: WeaponAnimSlot,
        state: WeaponState,
        timer_ms: Option<i32>,
        goal_secs: f32,
    ) -> EventResult {
        self.crawl_next = None;
        let Some(clip) = self.weapon.clip(slot).cloned() else {
            self.clear_action_slots(goal_secs);
            self.action = None;
            self.action_remaining = None;
            self.state = state;
            return EventResult::IgnoredMissingClip(slot);
        };
        let timer_ms = self.perk_scaled(state, timer_ms).filter(|&ms| ms > 0);
        self.clear_action_slots(goal_secs);
        self.tree.set_clip(slot.index(), Arc::clone(&clip)).expect("action index is within tree");
        // The clip is stretched or squeezed to the time the gun's facts give it.
        let rate = match timer_ms {
            Some(ms) if !slot_uses_native_rate(slot.index()) => {
                playback_rate((clip.duration() * 1000.0).round() as i32, ms)
            }
            _ => 1.0,
        };
        self.put_rate(slot.index(), rate);
        self.put_weight(slot.index(), ACTIVE_GOAL_WEIGHT, goal_secs);
        self.action = Some(slot);
        self.action_remaining = timer_ms.filter(|_| !clip.looping).map(|ms| ms as f32 / 1000.0);
        self.state = state;
        EventResult::Started(state)
    }

    fn clear_action_slots(&mut self, goal_secs: f32) {
        for node in action_slots() {
            self.put_weight(node, INACTIVE_GOAL_WEIGHT, goal_secs);
            self.put_rate(node, 1.0);
        }
    }

    // -----------------------------------------------------------------------------------
    // The aim overlay
    // -----------------------------------------------------------------------------------

    /// Scrubs the aim-up and aim-down clips to how far the aim is (`weapon_pos_frac`).
    /// A melee clears them.
    pub fn apply_ads_overlay_frame(&mut self, weapon_pos_frac: f32) {
        let scrub = weapon_t6::ads_overlay::ads_overlay_scrub(weapon_pos_frac);
        self.last_ads_frac = scrub.ads_up_weight;
        let meleeing = matches!(
            self.action,
            Some(S::Melee | S::MeleeCharge | S::MeleeEmpty | S::MeleeChargeEmpty)
        );
        if meleeing {
            self.set_weight(S::AdsUp, INACTIVE_GOAL_WEIGHT, 0.0);
            self.set_weight(S::AdsDown, INACTIVE_GOAL_WEIGHT, 0.0);
            return;
        }
        let aiming = weapon_pos_frac > 0.0;
        let play_anim = self.weapon.ads_overlay == AdsOverlayConvention::PlayAdsAnim;
        let has_down = self.has_clip(S::AdsDown);
        let active = |on: bool| if on { ACTIVE_GOAL_WEIGHT } else { INACTIVE_GOAL_WEIGHT };

        if let Some(length) = self.weapon.clip(S::AdsUp).map(|c| c.duration()) {
            let weight = if play_anim { active(aiming || !has_down) } else { scrub.ads_up_weight };
            self.scrub_overlay(S::AdsUp, length * scrub.ads_up_time_norm, Some(weight));
        }
        if let Some(length) = self.weapon.clip(S::AdsDown).map(|c| c.duration()) {
            let weight = play_anim.then(|| active(!aiming));
            self.scrub_overlay(S::AdsDown, length * scrub.ads_down_time_norm, weight);
        }
    }

    /// Holds an overlay clip at `time`, and moves its weight to `weight` when given.
    fn scrub_overlay(&mut self, slot: WeaponAnimSlot, time: f32, weight: Option<f32>) {
        self.put_rate(slot.index(), 0.0);
        self.put_time(slot.index(), time);
        if let Some(weight) = weight {
            self.put_weight(slot.index(), weight, ACTION_GOAL_TIME_SECS);
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
            if self.tree.weight(node).expect("node is within tree") > 0.0 {
                *was = Some(self.tree.time(node).expect("node is within tree"));
            }
            self.put_weight(node, INACTIVE_GOAL_WEIGHT, 0.0);
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
            let Some(clip) = self.weapon.clip(slot).cloned() else {
                out.unplaced.push(name);
                continue;
            };
            let time = clip.duration() * frac.clamp(0.0, 1.0);
            if weight > 0.0 {
                let from = match was_at[node] {
                    Some(old) if time >= old || clip.looping => old,
                    _ => 0.0,
                };
                out.notifies.extend(clip.crossed_notifies(from, time));
            }
            self.put_rate(node, 0.0);
            self.put_time(node, time);
            self.put_weight(node, weight, 0.0);
        }
        out
    }

    // -----------------------------------------------------------------------------------
    // The clip tree
    // -----------------------------------------------------------------------------------

    /// Moves a slot's weight, but only when the gun has a clip there.
    fn set_weight(&mut self, slot: WeaponAnimSlot, weight: f32, goal_secs: f32) {
        if self.has_clip(slot) {
            self.put_weight(slot.index(), weight, goal_secs);
        }
    }

    fn put_weight(&mut self, node: usize, weight: f32, goal_secs: f32) {
        self.tree.set_goal_weight(node, weight, goal_secs).expect("node is within tree");
    }

    fn put_rate(&mut self, node: usize, rate: f32) {
        self.tree.set_rate(node, rate).expect("node is within tree");
    }

    fn put_time(&mut self, node: usize, time: f32) {
        self.tree.set_time(node, time).expect("node is within tree");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_anim::xanim_clip::{ClipNotify, Translation};

    fn clip(name: &str, frames: u16, looping: bool) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: name.to_owned(),
            framerate: 30.0,
            numframes: frames,
            looping,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        })
    }

    fn gun(name: &str, clips: &[(WeaponAnimSlot, &str, u16, bool)]) -> WeaponAnimations {
        clips.iter().fold(
            WeaponAnimations::empty(name).with_clip(S::Idle, clip("idle", 30, true)),
            |gun, &(slot, clip_name, frames, looping)| {
                gun.with_clip(slot, clip(clip_name, frames, looping))
            },
        )
    }

    fn on(c: &ViewmodelController, slot: WeaponAnimSlot) -> bool {
        c.animation_weight(slot) == ACTIVE_GOAL_WEIGHT
    }

    fn off(c: &ViewmodelController, slot: WeaponAnimSlot) -> bool {
        c.animation_weight(slot) == 0.0
    }

    /// Plays clip node `slot` and runs `secs` of frames.
    fn go(c: &mut ViewmodelController, slot: WeaponAnimSlot, secs: f32) -> EventResult {
        let result = c.dispatch_sz_xanim_index(slot.index());
        c.advance(secs);
        result
    }

    // ---- slot names ------------------------------------------------------------------

    #[test]
    fn bo2_slot_names_map_to_clip_slots_and_unknown_ones_to_none() {
        assert_eq!(bo2_slot("dtp_loop"), Some(S::DiveLoop));
        assert_eq!(bo2_slot("sprint_empty_loop"), Some(S::SprintLoopEmpty));
        assert_eq!(bo2_slot("camera_mantle"), Some(S::CameraMantle));
        assert_eq!(bo2_slot("lowready_in"), None);
        assert_eq!(bo2_slot(""), None);
        let mut names: Vec<_> = BO2_SLOT_NAMES.iter().map(|e| e.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BO2_SLOT_NAMES.len(), "no name is listed twice");
    }

    // ---- empty magazine and first shots ----------------------------------------------

    #[test]
    fn an_empty_melee_plays_its_own_clip_or_the_full_one() {
        let base = gun("fiveseven_zm", &[(S::Melee, "tactical_melee", 20, false)]);
        let mut c = ViewmodelController::new(base.clone());
        go(&mut c, S::MeleeEmpty, 0.05);
        assert!(on(&c, S::Melee), "no empty clip: the full one plays");

        let own = base.with_clip(S::MeleeEmpty, clip("tactical_melee_empty", 20, false));
        let mut c = ViewmodelController::new(own);
        go(&mut c, S::MeleeEmpty, 0.05);
        assert!(on(&c, S::MeleeEmpty) && !on(&c, S::Melee));
    }

    #[test]
    fn an_empty_gun_without_empty_clips_plays_the_full_ones() {
        let mut c = ViewmodelController::new(gun(
            "ak74u_zm",
            &[
                (S::SprintLoop, "sprint_loop", 20, true),
                (S::CrawlIn, "crawl_in", 10, false),
                (S::CrawlBack, "crawl_back", 30, true),
            ],
        ));
        assert_eq!(
            go(&mut c, S::SprintLoopEmpty, 0.1),
            EventResult::Started(WeaponState::SprintLoop)
        );
        assert!(on(&c, S::SprintLoop));
        go(&mut c, S::Idle, 0.0);
        go(&mut c, S::CrawlBackEmpty, 0.1);
        assert!(on(&c, S::CrawlIn));
        c.advance(0.5);
        assert!(on(&c, S::CrawlBack));
    }

    #[test]
    fn the_first_shots_play_their_own_clip_or_the_guns_fire() {
        let mut c = ViewmodelController::new(gun(
            "hamr_mp",
            &[(S::Fire, "secondary_fire", 6, false), (S::FireIntro, "fire", 6, false)],
        ));
        assert_eq!(go(&mut c, S::FireIntro, 0.01), EventResult::Started(WeaponState::Firing));
        assert!(on(&c, S::FireIntro));

        let mut c = ViewmodelController::new(gun("an94_mp", &[(S::Fire, "fire", 6, false)]));
        assert_eq!(go(&mut c, S::FireIntro, 0.01), EventResult::Started(WeaponState::Firing));
        assert!(on(&c, S::Fire));
    }

    // ---- dive ------------------------------------------------------------------------

    /// The M14's dive as BO2 has it: 10, 15 and 10 frames over 200, 600 and 250 ms.
    fn m14(dive_clips: bool) -> ViewmodelController {
        let dive: &[_] = if dive_clips {
            &[
                (S::DiveIn, "d2p_in", 10, false),
                (S::DiveLoop, "d2p_loop", 15, false),
                (S::DiveOut, "d2p_out", 10, false),
            ]
        } else {
            &[]
        };
        let mut weapon = gun("m14_zm", dive);
        weapon.dive_times_ms = [200, 600, 250];
        ViewmodelController::new(weapon)
    }

    #[test]
    fn a_dive_takes_off_holds_in_the_air_then_lands_to_idle() {
        let mut c = m14(true);
        go(&mut c, S::DiveIn, 0.15);
        assert!(on(&c, S::DiveIn) && !on(&c, S::Idle));
        c.advance(0.1);
        assert!(on(&c, S::DiveLoop) && off(&c, S::DiveIn), "take-off ran into the air");
        c.advance(2.0);
        assert!(on(&c, S::DiveLoop), "held until touch-down");
        go(&mut c, S::DiveOut, 0.2);
        assert!(on(&c, S::DiveOut));
        c.advance(0.1);
        assert!(on(&c, S::Idle) && off(&c, S::DiveOut));
        assert_eq!(c.state(), WeaponState::Ready);
    }

    #[test]
    fn aiming_mid_dive_brings_the_gun_up_quickly() {
        let mut c = m14(true);
        go(&mut c, S::DiveIn, 0.1);
        c.apply_force_idle_weap_anim(false);
        c.advance(0.1);
        assert!(on(&c, S::Idle) && off(&c, S::DiveIn));
    }

    #[test]
    fn a_gun_without_dive_clips_stays_up() {
        let mut c = m14(false);
        assert_eq!(
            c.dispatch_sz_xanim_index(S::DiveIn.index()),
            EventResult::Started(WeaponState::Ready)
        );
        c.advance(0.6);
        assert!(on(&c, S::Idle));
    }

    #[test]
    fn a_shot_mid_dive_replaces_the_dive() {
        let mut c = m14(true);
        go(&mut c, S::DiveIn, 0.1);
        assert_eq!(c.handle(ViewmodelEvent::Fire), EventResult::IgnoredMissingClip(S::Fire));
        c.advance(0.5);
        assert!(!on(&c, S::DiveLoop), "no in-air clip after a shot");
    }

    // ---- crawl -----------------------------------------------------------------------

    /// The 870's crawl as BO2 has it: in 10 frames over 300 ms, direction loops over
    /// 1000-1200 ms, out 10 frames over 100 ms, out-fire 48 ms.
    fn r870(crawl_clips: bool) -> ViewmodelController {
        let crawl: &[_] = if crawl_clips {
            &[
                (S::CrawlIn, "crawl_in", 10, false),
                (S::CrawlForward, "crawl_forward", 30, true),
                (S::CrawlLeft, "crawl_left", 30, true),
                (S::CrawlOut, "crawl_out", 10, false),
            ]
        } else {
            &[]
        };
        let mut weapon = gun("870mcs_zm", crawl).with_clip(S::Fire, clip("fire", 10, false));
        weapon.crawl_times_ms = [300, 1100, 1000, 1200, 1100, 48, 100];
        ViewmodelController::new(weapon)
    }

    #[test]
    fn a_crawl_plays_in_then_the_direction_then_out_to_idle() {
        let mut c = r870(true);
        go(&mut c, S::CrawlForward, 0.2);
        assert!(on(&c, S::CrawlIn) && !on(&c, S::Idle));
        c.advance(0.15);
        assert!(on(&c, S::CrawlForward) && off(&c, S::CrawlIn));
        c.advance(3.0);
        assert!(on(&c, S::CrawlForward), "the crawl loops");
        go(&mut c, S::CrawlOut, 0.05);
        assert!(on(&c, S::CrawlOut) && off(&c, S::CrawlForward));
        c.advance(0.06);
        assert!(on(&c, S::Idle) && off(&c, S::CrawlOut), "out settled to idle");
        assert_eq!(
            c.dispatch_sz_xanim_index(S::CrawlOut.index()),
            EventResult::IgnoredState(WeaponState::Ready),
            "a crawl-out with no crawl does nothing"
        );
    }

    #[test]
    fn a_new_crawl_direction_waits_for_the_crawl_in_then_blends_across() {
        let mut c = r870(true);
        go(&mut c, S::CrawlForward, 0.1);
        go(&mut c, S::CrawlLeft, 0.0);
        assert!(on(&c, S::CrawlIn), "the crawl-in plays on");
        c.advance(0.25);
        assert!(on(&c, S::CrawlLeft) && off(&c, S::CrawlForward), "then the newest direction");
        go(&mut c, S::CrawlForward, 0.05);
        let blend = c.animation_weight(S::CrawlForward);
        assert!(blend > 0.0 && blend < 1.0, "blending in, {blend}");
        c.advance(0.2);
        assert!(on(&c, S::CrawlForward) && off(&c, S::CrawlLeft));
    }

    #[test]
    fn leaving_a_crawl_blends_over_the_out_fire_time() {
        let crawling = || {
            let mut c = r870(true);
            go(&mut c, S::CrawlForward, 0.5);
            c
        };
        let mut c = crawling();
        c.handle(ViewmodelEvent::Fire);
        c.advance(0.02);
        let blend = c.animation_weight(S::Fire);
        assert!(blend > 0.0 && blend < 1.0, "a shot blends in, {blend}");
        c.advance(0.04);
        assert!(on(&c, S::Fire) && off(&c, S::CrawlForward));

        let mut c = crawling();
        c.apply_force_idle_weap_anim(false);
        c.advance(0.06);
        assert!(on(&c, S::Idle) && off(&c, S::CrawlForward), "aiming out of it");

        let mut c = crawling();
        assert!(c.apply_idle_weap_anim(false), "the idle edge ends a crawl");
        c.advance(0.5);
        assert!(on(&c, S::Idle));
    }

    #[test]
    fn a_gun_without_crawl_clips_stays_as_it_is() {
        let mut c = r870(false);
        assert_eq!(
            go(&mut c, S::CrawlForward, 1.0),
            EventResult::Started(WeaponState::Ready)
        );
        assert!(on(&c, S::Idle));
    }

    // ---- run loop, fire, events ------------------------------------------------------

    #[test]
    fn a_run_loop_that_does_not_loop_starts_over_instead_of_freezing() {
        // The ballistic knife's: 1000 ms, not a looping clip, in a 600 ms sprint loop time.
        let mut weapon = gun("knife_ballistic_mp", &[(S::SprintLoop, "sprint_loop", 30, false)]);
        weapon.sprint_loop_time_ms = 600;
        let mut c = ViewmodelController::new(weapon);
        go(&mut c, S::SprintLoop, 0.5);
        let mid = c.animation_time(S::SprintLoop);
        c.advance(0.2);
        assert!(on(&c, S::SprintLoop));
        assert!(c.animation_time(S::SprintLoop) < mid, "the run loop starts over");
    }

    #[test]
    fn events_are_refused_in_the_states_that_cannot_take_them() {
        let mut weapon = gun(
            "mp7_mp",
            &[
                (S::Fire, "fire", 6, false),
                (S::Reload, "reload", 60, false),
                (S::SprintIn, "sprint_in", 10, false),
                (S::SprintLoop, "sprint_loop", 20, true),
            ],
        );
        weapon.reload_time_ms = 2000;
        let mut c = ViewmodelController::new(weapon);
        assert_eq!(c.handle(ViewmodelEvent::Reload { empty: false }), EventResult::Started(WeaponState::Reloading { empty: false }));
        assert_eq!(c.handle(ViewmodelEvent::Fire), EventResult::IgnoredState(WeaponState::Reloading { empty: false }));
        assert_eq!(c.handle(ViewmodelEvent::SprintIn), EventResult::IgnoredState(WeaponState::Reloading { empty: false }));
        // Sprinting out of anything else is always allowed, a second loop is not.
        assert_eq!(c.handle(ViewmodelEvent::SprintLoop), EventResult::Started(WeaponState::SprintLoop));
        assert_eq!(c.handle(ViewmodelEvent::SprintLoop), EventResult::IgnoredState(WeaponState::SprintLoop));
        assert_eq!(c.handle(ViewmodelEvent::SprintOut), EventResult::IgnoredMissingClip(S::SprintOut));
    }

    #[test]
    fn a_timed_action_plays_over_the_time_the_gun_gives_it() {
        let mut weapon = gun("mp7_mp", &[(S::Reload, "reload", 60, false)]);
        weapon.reload_time_ms = 1000;
        let mut c = ViewmodelController::new(weapon);
        c.handle(ViewmodelEvent::Reload { empty: false });
        c.advance(0.5);
        // A 2 s clip over 1 s of reload time runs twice as fast.
        let rate_is_native = slot_uses_native_rate(S::Reload.index());
        let expected = if rate_is_native { 0.5 } else { 1.0 };
        assert!((c.animation_time(S::Reload) - expected).abs() < 0.05);
        assert_eq!(c.state(), WeaponState::Reloading { empty: false });
        c.advance(0.6);
        assert_eq!(c.state(), WeaponState::Reloading { empty: false }, "reloads end by the next edge");
    }

    #[test]
    fn a_shot_settles_when_the_clip_is_out_and_the_trigger_is_up() {
        let mut weapon = gun("mp7_mp", &[(S::Fire, "fire", 30, false)]);
        weapon.fire_time_ms = 100;
        weapon.fire_plays_out = true;
        let mut c = ViewmodelController::new(weapon);
        c.handle(ViewmodelEvent::Fire);
        c.advance(0.2);
        assert_eq!(c.state(), WeaponState::Ready, "the fire time is out");
        assert!(c.animation_time(S::Fire) < 1.0, "the recoil clip is not");
        c.settle_fire_to_idle();
        assert!(on(&c, S::Fire), "the trigger came up, but the recoil plays out");
        c.advance(1.0);
        assert!(on(&c, S::Idle) && off(&c, S::Fire), "then it settles");
    }

    #[test]
    fn a_shot_that_does_not_play_out_settles_at_once() {
        let mut weapon = gun("mp7_mp", &[(S::Fire, "fire", 30, false)]);
        weapon.fire_time_ms = 100;
        let mut c = ViewmodelController::new(weapon);
        c.handle(ViewmodelEvent::Fire);
        c.advance(0.2);
        c.settle_fire_to_idle();
        assert!(on(&c, S::Idle) && off(&c, S::Fire));
        // With nothing fired there is nothing to settle.
        let mut c = ViewmodelController::new(gun("mp7_mp", &[]));
        c.settle_fire_to_idle();
        assert_eq!(c.state(), WeaponState::Ready);
    }

    #[test]
    fn the_idle_edge_waits_for_a_playing_clip_and_forced_idle_cuts_in() {
        let mut weapon = gun("mp7_mp", &[(S::Reload, "reload", 60, false)]);
        weapon.reload_time_ms = 2000;
        let mut c = ViewmodelController::new(weapon);
        c.handle(ViewmodelEvent::Reload { empty: false });
        c.advance(0.5);
        assert!(!c.apply_idle_weap_anim(false), "the reload still plays");
        c.apply_force_idle_weap_anim(false);
        c.advance(0.5);
        assert!(on(&c, S::Idle) && off(&c, S::Reload));
        assert_eq!(c.state(), WeaponState::Ready);
    }

    #[test]
    fn an_empty_idle_is_used_only_when_the_gun_has_one() {
        let mut c = ViewmodelController::new(gun("a", &[(S::EmptyIdle, "empty_idle", 30, true)]));
        c.apply_force_idle_weap_anim(true);
        c.advance(0.5);
        assert!(on(&c, S::EmptyIdle));
        c.apply_force_idle_weap_anim(false);
        c.advance(0.5);
        assert!(on(&c, S::Idle) && off(&c, S::EmptyIdle));
        let mut c = ViewmodelController::new(gun("b", &[]));
        c.apply_force_idle_weap_anim(true);
        c.advance(0.5);
        assert!(on(&c, S::Idle));
    }

    // ---- aim overlay -----------------------------------------------------------------

    #[test]
    fn the_aim_overlay_follows_the_aim_by_the_guns_convention() {
        let mut weapon = gun("mp7_mp", &[(S::AdsUp, "ads_up", 7, false)]);
        weapon.ads_overlay = AdsOverlayConvention::WeightIsFrac;
        let mut c = ViewmodelController::new(weapon.clone());
        c.apply_ads_overlay_frame(0.0);
        c.advance(0.2);
        assert_eq!(c.animation_weight(S::AdsUp), 0.0);
        c.apply_ads_overlay_frame(1.0);
        c.advance(0.2);
        assert!(c.animation_weight(S::AdsUp) > 0.9, "the weight follows the aim");

        weapon.ads_overlay = AdsOverlayConvention::PlayAdsAnim;
        let mut c = ViewmodelController::new(weapon);
        assert!(on(&c, S::AdsUp), "a gun with only an aim-up clip plays it at the hip");
        c.apply_ads_overlay_frame(0.0);
        assert!(on(&c, S::AdsUp));
    }

    #[test]
    fn a_melee_clears_the_aim_overlay() {
        let mut c = ViewmodelController::new(gun(
            "mp7_mp",
            &[(S::Melee, "melee", 20, false), (S::AdsUp, "ads_up", 7, false), (S::AdsDown, "ads_down", 7, false)],
        ));
        assert!(on(&c, S::AdsDown), "the lowered pose is held at the hip");
        go(&mut c, S::Melee, 0.05);
        c.apply_ads_overlay_frame(0.5);
        assert!(off(&c, S::AdsUp) && off(&c, S::AdsDown));
    }

    #[test]
    fn a_shot_while_aiming_uses_the_aim_fire_clip_when_there_is_one() {
        let mut c = ViewmodelController::new(gun(
            "mp7_mp",
            &[(S::Fire, "fire", 6, false), (S::AdsFire, "ads_fire", 6, false)],
        ));
        c.apply_ads_overlay_frame(1.0);
        c.handle(ViewmodelEvent::Fire);
        assert!(on(&c, S::AdsFire) && off(&c, S::Fire));
    }

    // ---- BO2 pose --------------------------------------------------------------------

    #[test]
    fn a_bo2_pose_sets_each_slot_as_given_and_clears_the_rest() {
        let mut c = ViewmodelController::new(gun(
            "mp7_mp",
            &[(S::Reload, "reload", 54, false), (S::AdsUp, "ads_up", 6, false)],
        ));
        let pose = c.apply_bo2_pose([
            ("reload", 0.75, 0.5),
            ("idle", 0.25, 1.0),
            ("ads_up", 1.0, 0.0),
            ("reload", 0.1, 0.9),
            ("idle_left", 1.0, 0.0),
        ]);
        assert_eq!(pose.unplaced, ["idle_left"]);
        assert_eq!(c.animation_weight(S::Reload), 0.75);
        assert!((c.animation_time(S::Reload) - 0.9).abs() < 1e-5, "the clip is 1.8 s long");
        assert_eq!(c.animation_weight(S::Idle), 0.25);
        assert_eq!(c.animation_weight(S::AdsUp), 1.0);
        assert_eq!(c.animation_weight(S::AdsDown), 0.0);

        // BO2's player owns the clock: advancing moves nothing that was posed.
        c.advance(0.1);
        assert!((c.animation_time(S::Reload) - 0.9).abs() < 1e-5);

        assert!(c.apply_bo2_pose([("idle", 1.0, 0.2)]).unplaced.is_empty());
        assert!(off(&c, S::Reload) && off(&c, S::AdsUp));
    }

    #[test]
    fn a_bo2_slot_with_no_clip_draws_nothing() {
        let mut c = ViewmodelController::new(WeaponAnimations::empty("fiveseven_mp").with_clip(S::Idle, clip("idle", 30, false)));
        assert_eq!(c.apply_bo2_pose([("sprint_in", 1.0, 0.0)]).unplaced, ["sprint_in"]);
        assert_eq!(c.active_anims().count(), 0);
    }

    #[test]
    fn a_bo2_pose_reports_the_notetracks_it_passes() {
        let mut reload = (*clip("reload", 60, false)).clone();
        reload.notifies = vec![
            ClipNotify { name: "mag_out".into(), time: 0.25 },
            ClipNotify { name: "mag_in".into(), time: 0.75 },
        ];
        let weapon = WeaponAnimations::empty("mp7_mp").with_clip(S::Reload, Arc::new(reload));
        let mut c = ViewmodelController::new(weapon);
        let mut notes = |frac, weight| c.apply_bo2_pose([("reload", weight, frac)]).notifies;
        assert!(notes(0.0, 1.0).is_empty());
        assert_eq!(notes(0.5, 1.0), ["mag_out"]);
        assert!(notes(0.6, 1.0).is_empty());
        assert_eq!(notes(1.0, 1.0), ["mag_in"]);
        assert_eq!(notes(0.3, 1.0), ["mag_out"], "a restart plays from the top again");
        assert!(notes(0.9, 0.0).is_empty(), "a fading slot says nothing");
    }

    #[test]
    fn replacing_a_clip_keeps_its_weight_and_play_position() {
        let mut c = ViewmodelController::new(gun("mp7_mp", &[(S::SprintLoop, "sprint", 20, true)]));
        c.apply_bo2_pose([("sprint_loop", 0.6, 0.5)]);
        let before = c.animation_time(S::SprintLoop);
        c.replace_clip(S::SprintLoop, clip("sprint_smooth", 20, true));
        assert_eq!(c.animation_time(S::SprintLoop), before);
        assert_eq!(c.animation_weight(S::SprintLoop), 0.6);
        assert_eq!(c.weapon().clip(S::SprintLoop).map(|c| c.name.as_str()), Some("sprint_smooth"));
    }
}
