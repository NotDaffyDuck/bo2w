//! BO2's first-person gun-and-arms animation player (window 7), rebuilt in our Rust.
//!
//! Every frame BO2's client turns the player's `weapAnim` / `weapAnimLeft` request (see
//! [`crate::anim`]) into weights on one blend tree of 88 viewmodel slots ([`slot`]): which
//! slot plays, how long it blends in, how fast it runs (the weapon's time for that anim,
//! sped up by perks), the ADS up/down pose, and the camera anims that ride along with
//! sprint, dive-to-prone and mantle. BO2's own slot numbers are used throughout; nothing is
//! remapped to MW2's anim list.
//!
//! [`Viewmodel::frame`] is one client frame. The renderer reads [`Viewmodel::pose`].
//! Spec and the per-request table: `docs/gun/ANIMS.md`.

use crate::anim;
use crate::env::{T6Dvars, perk, pm_type};
use crate::facts::{inventory, offhand_slot, unhex};
use crate::state;
use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

/// BO2's 88 viewmodel anim slots, by BO2's numbers.
pub mod slot {
    pub const ROOT: usize = 0;
    pub const IDLE: usize = 1;
    pub const EMPTY_IDLE: usize = 2;
    pub const FIRE_INTRO: usize = 3;
    pub const FIRE: usize = 4;
    pub const HOLD_FIRE: usize = 5;
    pub const LASTSHOT: usize = 6;
    pub const FINALSHOT: usize = 7;
    pub const RECHAMBER: usize = 8;
    pub const MELEE: usize = 9;
    pub const MELEE1: usize = 10;
    pub const MELEE2: usize = 11;
    pub const MELEE3: usize = 12;
    pub const MELEE_EMPTY: usize = 13;
    pub const MELEE_CHARGE: usize = 14;
    pub const MELEE_CHARGE_EMPTY: usize = 15;
    pub const RELOAD: usize = 16;
    pub const RELOAD_RIGHT: usize = 17;
    pub const RELOAD_EMPTY: usize = 18;
    pub const RELOAD_START: usize = 19;
    pub const RELOAD_END: usize = 20;
    pub const RELOAD_QUICK: usize = 21;
    pub const RELOAD_QUICK_EMPTY: usize = 22;
    pub const RAISE: usize = 23;
    pub const FIRST_RAISE: usize = 24;
    pub const DROP: usize = 25;
    pub const ALT_RAISE: usize = 26;
    pub const ALT_DROP: usize = 27;
    pub const QUICK_RAISE: usize = 28;
    pub const QUICK_DROP: usize = 29;
    pub const EMPTY_RAISE: usize = 30;
    pub const EMPTY_DROP: usize = 31;
    pub const SPRINT_IN: usize = 32;
    pub const SPRINT_LOOP: usize = 33;
    pub const SPRINT_OUT: usize = 34;
    pub const SPRINT_EMPTY_IN: usize = 35;
    pub const SPRINT_EMPTY_LOOP: usize = 36;
    pub const SPRINT_EMPTY_OUT: usize = 37;
    pub const LOWREADY_IN: usize = 38;
    pub const LOWREADY_LOOP: usize = 39;
    pub const LOWREADY_OUT: usize = 40;
    pub const CONT_FIRE_IN: usize = 41;
    pub const CONT_FIRE_LOOP: usize = 42;
    pub const CONT_FIRE_OUT: usize = 43;
    pub const CRAWL_IN: usize = 44;
    pub const CRAWL_FORWARD: usize = 45;
    pub const CRAWL_BACK: usize = 46;
    pub const CRAWL_RIGHT: usize = 47;
    pub const CRAWL_LEFT: usize = 48;
    pub const CRAWL_OUT: usize = 49;
    pub const CRAWL_EMPTY_IN: usize = 50;
    pub const CRAWL_EMPTY_FORWARD: usize = 51;
    pub const CRAWL_EMPTY_BACK: usize = 52;
    pub const CRAWL_EMPTY_RIGHT: usize = 53;
    pub const CRAWL_EMPTY_LEFT: usize = 54;
    pub const CRAWL_EMPTY_OUT: usize = 55;
    pub const DEPLOY: usize = 56;
    pub const BREAKDOWN: usize = 57;
    pub const DETONATE: usize = 58;
    pub const NIGHTVISION_WEAR: usize = 59;
    pub const NIGHTVISION_REMOVE: usize = 60;
    pub const ADS_FIRE: usize = 61;
    pub const ADS_LASTSHOT: usize = 62;
    pub const ADS_FIRE_INTRO: usize = 63;
    pub const ADS_RECHAMBER: usize = 64;
    pub const DTP_IN: usize = 65;
    pub const DTP_LOOP: usize = 66;
    pub const DTP_OUT: usize = 67;
    pub const DTP_EMPTY_IN: usize = 68;
    pub const DTP_EMPTY_LOOP: usize = 69;
    pub const DTP_EMPTY_OUT: usize = 70;
    pub const SLIDE_IN: usize = 71;
    pub const MANTLE: usize = 72;
    pub const CAMERA_SPRINT_LOOP: usize = 73;
    pub const CAMERA_DTP_IN: usize = 74;
    pub const CAMERA_DTP_LOOP: usize = 75;
    pub const CAMERA_DTP_OUT: usize = 76;
    pub const CAMERA_MANTLE: usize = 77;
    pub const FIRE_LEFT: usize = 78;
    pub const LASTSHOT_LEFT: usize = 79;
    pub const FINALSHOT_LEFT: usize = 80;
    pub const IDLE_LEFT: usize = 81;
    pub const EMPTY_IDLE_LEFT: usize = 82;
    pub const RELOAD_EMPTY_LEFT: usize = 83;
    pub const RELOAD_LEFT: usize = 84;
    pub const ADS_UP: usize = 85;
    pub const ADS_DOWN: usize = 86;
    pub const ADS_UP_OTHER_SCOPE: usize = 87;

    /// Number of slots (NUM_WEAP_ANIMS).
    pub const COUNT: usize = 88;

    /// A hand that is not running any slot.
    pub const NONE: usize = 0x58;

    const NAMES: [&str; COUNT] = [
        "root", "idle", "empty_idle", "fire_intro", "fire", "hold_fire", "lastshot", "finalshot",
        "rechamber", "melee", "melee1", "melee2", "melee3", "melee_empty", "melee_charge",
        "melee_charge_empty", "reload", "reload_right", "reload_empty", "reload_start",
        "reload_end", "reload_quick", "reload_quick_empty", "raise", "first_raise", "drop",
        "alt_raise", "alt_drop", "quick_raise", "quick_drop", "empty_raise", "empty_drop",
        "sprint_in", "sprint_loop", "sprint_out", "sprint_empty_in", "sprint_empty_loop",
        "sprint_empty_out", "lowready_in", "lowready_loop", "lowready_out", "cont_fire_in",
        "cont_fire_loop", "cont_fire_out", "crawl_in", "crawl_forward", "crawl_back",
        "crawl_right", "crawl_left", "crawl_out", "crawl_empty_in", "crawl_empty_forward",
        "crawl_empty_back", "crawl_empty_right", "crawl_empty_left", "crawl_empty_out", "deploy",
        "breakdown", "detonate", "nightvision_wear", "nightvision_remove", "ads_fire",
        "ads_lastshot", "ads_fire_intro", "ads_rechamber", "dtp_in", "dtp_loop", "dtp_out",
        "dtp_empty_in", "dtp_empty_loop", "dtp_empty_out", "slide_in", "mantle",
        "camera_sprint_loop", "camera_dtp_in", "camera_dtp_loop", "camera_dtp_out",
        "camera_mantle", "fire_left", "lastshot_left", "finalshot_left", "idle_left",
        "empty_idle_left", "reload_empty_left", "reload_left", "ads_up", "ads_down",
        "ads_up_other_scope",
    ];

    /// BO2's name for a slot, for traces.
    pub fn name(s: usize) -> &'static str {
        if s == NONE {
            return "none";
        }
        NAMES.get(s).copied().unwrap_or("?")
    }

    /// Slot number from its name (traces and scripts).
    pub fn find(name: &str) -> Option<usize> {
        NAMES.iter().position(|n| n.eq_ignore_ascii_case(name))
    }
}

/// `specialty_fastsprintrecovery`: the sprint-out speed-up perk.
pub const PERK_FASTSPRINTRECOVERY: u32 = 57;

/// The dvars the anim player reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VmDvars {
    pub perk_weap_switch_multiplier: f32,
    pub perk_weap_reload_multiplier: f32,
    pub perk_weap_rate_multiplier: f32,
    pub perk_weap_melee_multiplier: f32,
    pub perk_weap_toss_multiplier: f32,
    pub perk_weap_equipment_use_multiplier: f32,
    /// `perk_sprintRecoveryMultiplier`.
    pub perk_sprint_recovery_multiplier: f32,
    /// `player_sprintCameraBob`.
    pub player_sprint_camera_bob: f32,
    /// `mantle_weapon_anim_height`: shorter mantles skip the mantle anim.
    pub mantle_weapon_anim_height: f32,
}

impl Default for VmDvars {
    fn default() -> Self {
        Self::from_t6(&T6Dvars::default())
    }
}

impl VmDvars {
    /// The perk multipliers from the brain's dvars, BO2's defaults for the rest.
    pub fn from_t6(d: &T6Dvars) -> Self {
        Self {
            perk_weap_switch_multiplier: d.perk_weap_switch_multiplier,
            perk_weap_reload_multiplier: d.perk_weap_reload_multiplier,
            perk_weap_rate_multiplier: d.perk_weap_rate_multiplier,
            perk_weap_melee_multiplier: d.perk_weap_melee_multiplier,
            perk_weap_toss_multiplier: d.perk_weap_toss_multiplier,
            perk_weap_equipment_use_multiplier: d.perk_weap_equipment_use_multiplier,
            perk_sprint_recovery_multiplier: 0.3,
            player_sprint_camera_bob: 0.5,
            mantle_weapon_anim_height: 32.0,
        }
    }
}

/// Where a slot's play time (ms) comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeSrc {
    None,
    Def(usize),
    Var(usize),
}

/// BO2's time field for each slot. A slot with no time (or time 0) plays at rate 1.
fn time_src(s: usize) -> TimeSrc {
    use TimeSrc::{Def, None, Var};
    match s {
        5 => Def(D::iHoldFireTime),
        7 | 80 => Def(D::iFlourishTime),
        8 => Def(D::iRechamberTime),
        9..=13 => Def(D::iMeleeTime),
        14 | 15 => Def(D::meleeChargeTime),
        16 | 84 => Var(V::iReloadTime),
        17 => Def(D::iReloadTimeRight),
        18 | 83 => Var(V::iReloadEmptyTime),
        19 => Def(D::iReloadStartTime),
        20 => Def(D::iReloadEndTime),
        21 => Var(V::iReloadQuickTime),
        22 => Var(V::iReloadQuickEmptyTime),
        23 => Def(D::iRaiseTime),
        24 => Def(D::iFirstRaiseTime),
        25 => Def(D::iDropTime),
        26 => Var(V::iAltRaiseTime),
        27 => Def(D::iAltDropTime),
        28 => Def(D::quickRaiseTime),
        29 => Def(D::quickDropTime),
        30 => Def(D::iEmptyRaiseTime),
        31 => Def(D::iEmptyDropTime),
        32 | 35 => Def(D::sprintInTime),
        33 | 36 | 73 => Def(D::sprintLoopTime),
        34 | 37 => Def(D::sprintOutTime),
        38 => Def(D::lowReadyInTime),
        39 => Def(D::lowReadyLoopTime),
        40 => Def(D::lowReadyOutTime),
        41 => Def(D::contFireInTime),
        42 => Def(D::contFireLoopTime),
        43 => Def(D::contFireOutTime),
        44 | 50 => Def(D::crawlInTime),
        45 | 51 => Def(D::crawlForwardTime),
        46 | 52 => Def(D::crawlBackTime),
        47 | 53 => Def(D::crawlRightTime),
        48 | 54 => Def(D::crawlLeftTime),
        49 | 55 => Def(D::crawlOutTime),
        56 => Def(D::deployTime),
        57 => Def(D::breakdownTime),
        59 => Def(D::nightVisionWearTime),
        60 => Def(D::nightVisionRemoveTime),
        65 | 68 | 74 => Def(D::dtpInTime),
        66 | 69 | 75 => Def(D::dtpLoopTime),
        67 | 70 | 76 => Def(D::dtpOutTime),
        71 => Def(D::slideInTime),
        _ => None,
    }
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_i32(b, at) as u32)
}

fn rd_u8(b: &[u8], at: usize) -> u8 {
    b.get(at).copied().unwrap_or(0)
}

/// What the anim player needs to know about one weapon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VmFacts {
    /// Play time (ms) per slot; 0 = play at the clip's own speed.
    pub times: [i32; slot::COUNT],
    pub sprint_cycle_scale: f32,
    pub move_speed_scale: f32,
    pub aim_down_sight: bool,
    pub inventory_type: u8,
    /// `bAltWeaponAdsOnly` as stored.
    pub alt_weapon_ads_only: bool,
    pub dual_wield: bool,
    pub offhand_slot: u8,
    pub intro_fire_length: i32,
    /// An unlimited-ammo weapon (the combat knife, clip 0) is never empty: it idles on IDLE.
    pub unlimited_ammo: bool,
}

impl Default for VmFacts {
    fn default() -> Self {
        Self {
            times: [0; slot::COUNT],
            sprint_cycle_scale: 0.0,
            move_speed_scale: 0.0,
            aim_down_sight: false,
            inventory_type: 0,
            alt_weapon_ads_only: false,
            dual_wield: false,
            offhand_slot: 0,
            intro_fire_length: 0,
            unlimited_ammo: false,
        }
    }
}

impl VmFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let mut times = [0; slot::COUNT];
        for (s, t) in times.iter_mut().enumerate() {
            *t = match time_src(s) {
                TimeSrc::None => 0,
                TimeSrc::Def(at) => rd_i32(def, at),
                TimeSrc::Var(at) => rd_i32(variant, at),
            };
        }
        Self {
            times,
            sprint_cycle_scale: rd_f32(def, D::fSprintCycleScale),
            move_speed_scale: rd_f32(def, D::moveSpeedScale),
            aim_down_sight: rd_u8(def, D::aimDownSight) != 0,
            inventory_type: rd_u8(def, D::inventoryType),
            alt_weapon_ads_only: rd_u8(def, D::bAltWeaponAdsOnly) != 0,
            dual_wield: rd_u8(def, D::bDualWield) != 0,
            offhand_slot: rd_u8(def, D::offhandSlot),
            intro_fire_length: rd_i32(def, D::iIntroFireLength),
            unlimited_ammo: rd_u8(def, D::unlimitedAmmo) != 0,
        }
    }

    /// An alt-mode weapon that only exists while aiming (the PlayADSAnim test).
    pub fn alt_ads_only(&self) -> bool {
        self.inventory_type == inventory::ALTMODE && self.alt_weapon_ads_only
    }
}

/// [`VmFacts`] by weapon index, same numbering and dump format as
/// [`crate::facts::FactsTable`].
#[derive(Clone, Debug, Default)]
pub struct VmTable {
    rows: Vec<Option<VmFacts>>,
}

impl VmTable {
    pub fn set(&mut self, weapon: u32, facts: VmFacts) {
        let i = weapon as usize;
        if self.rows.len() <= i {
            self.rows.resize(i + 1, None);
        }
        self.rows[i] = Some(facts);
    }

    pub fn get(&self, weapon: u32) -> Option<&VmFacts> {
        self.rows.get(weapon as usize).and_then(Option::as_ref)
    }

    /// `name<TAB>variant hex<TAB>def hex` per line; line N (from 1) is weapon N.
    pub fn from_dump(text: &str) -> Self {
        let mut t = Self::default();
        for (i, line) in text.lines().enumerate() {
            let mut parts = line.split('\t');
            let (Some(_), Some(v), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let (Some(v), Some(d)) = (unhex(v), unhex(d)) else {
                continue;
            };
            t.set(i as u32 + 1, VmFacts::read(&v, &d));
        }
        t
    }

    /// [`Self::from_dump`] by the game's weapon numbers: `rows` is (game number, name). A name
    /// with no dump line uses its base name (before the first '+'), as
    /// [`crate::facts::FactsTable::reindexed`] does.
    pub fn from_dump_reindexed<'a>(
        text: &str,
        rows: impl IntoIterator<Item = (u32, &'a str)>,
    ) -> Self {
        let mut by_name: Vec<(&str, VmFacts)> = Vec::new();
        // `#att` lines carry attachments, not weapons.
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.split('\t');
            let (Some(n), Some(v), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let (Some(v), Some(d)) = (unhex(v), unhex(d)) else {
                continue;
            };
            if !by_name.iter().any(|(m, _)| *m == n) {
                by_name.push((n, VmFacts::read(&v, &d)));
            }
        }
        let find = |name: &str| by_name.iter().find(|(m, _)| *m == name).map(|(_, f)| *f);
        let mut t = Self::default();
        for (i, name) in rows {
            if let Some(f) = find(name).or_else(|| name.split_once('+').and_then(|(b, _)| find(b))) {
                t.set(i, f);
            }
        }
        t
    }
}

/// One xanim clip as the blend tree sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub length_ms: f32,
    pub looping: bool,
}

impl Clip {
    /// A weapon that names no clip at all: one second, looping.
    pub const MISSING: Clip = Clip {
        length_ms: 1000.0,
        looping: true,
    };
}

impl Default for Clip {
    fn default() -> Self {
        Self::MISSING
    }
}

/// A viewmodel's 88 clips. `named[s]` is "the weapon names an anim for slot s" (BO2 checks
/// the name, not the clip); an unnamed slot plays the idle clip.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimSet {
    pub named: [bool; slot::COUNT],
    pub clips: [Clip; slot::COUNT],
}

impl Default for AnimSet {
    fn default() -> Self {
        Self::build(&[None; slot::COUNT], &[None; slot::COUNT])
    }
}

impl AnimSet {
    /// `right` is the weapon's own list; `left` is the left-hand (dual wield) list, which
    /// fills the left-hand slots 78-84 and falls back to its own idle.
    pub fn build(right: &[Option<Clip>; slot::COUNT], left: &[Option<Clip>; slot::COUNT]) -> Self {
        let idle = right[slot::IDLE].unwrap_or(Clip::MISSING);
        let left_idle = left[slot::IDLE].unwrap_or(idle);
        let mut clips = [idle; slot::COUNT];
        let mut named = [false; slot::COUNT];
        for s in 1..slot::COUNT {
            named[s] = right[s].is_some();
            clips[s] = if (slot::FIRE_LEFT..=slot::RELOAD_LEFT).contains(&s) {
                left[s].unwrap_or(left_idle)
            } else {
                right[s].unwrap_or(idle)
            };
        }
        Self { named, clips }
    }
}

/// One running anim on the tree (BO2's XAnimInfo state).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimInfo {
    pub weight: f32,
    pub goal: f32,
    /// Seconds left to reach `goal`.
    pub goal_time: f32,
    pub rate: f32,
    /// Normalized play position, 0..=1.
    pub time: f32,
    pub old_time: f32,
    pub cycle: i32,
    pub old_cycle: i32,
}

/// The viewmodel's blend tree: one info per slot, plus the fading copies a restart leaves
/// behind while it crossfades into the fresh start.
#[derive(Clone, Debug)]
pub struct VmTree {
    infos: [Option<AnimInfo>; slot::COUNT],
    ghosts: Vec<(usize, AnimInfo)>,
}

impl Default for VmTree {
    fn default() -> Self {
        Self {
            infos: [None; slot::COUNT],
            ghosts: Vec::new(),
        }
    }
}

/// Move `info` toward `goal` over `blend` seconds (the weight half of XAnimSetGoalWeight).
fn blend_to(info: &mut AnimInfo, goal: f32, blend: f32) {
    if blend == 0.0 {
        info.weight = goal;
        info.goal_time = 0.0;
        return;
    }
    if info.weight == 0.0 {
        info.weight = goal * 0.001;
    }
    let d = info.weight - goal;
    if d == 0.0 || (info.goal - goal) * d > 0.0 {
        info.goal_time = blend;
    } else {
        info.goal_time = info.goal_time.min(blend);
    }
    if goal != 0.0 {
        let frac = if goal < info.weight {
            d / info.weight
        } else {
            (goal - info.weight) / goal
        };
        info.goal_time = info.goal_time.max(frac * blend);
    }
}

/// One frame of weight and time for one info. False = the info is gone.
fn advance(info: &mut AnimInfo, dt: f32, clip: Clip) -> bool {
    if info.goal_time <= dt {
        info.weight = info.goal;
        info.goal_time = 0.0;
        if info.goal == 0.0 {
            return false;
        }
    } else {
        info.weight += (info.goal - info.weight) * dt / info.goal_time;
        info.goal_time -= dt;
        if info.weight < 1e-6 {
            info.weight = info.goal * 0.001;
            if info.goal == 0.0 {
                return false;
            }
        }
    }
    info.old_time = info.time;
    info.old_cycle = info.cycle;
    if clip.length_ms > 0.0 {
        info.time += info.rate * dt * 1000.0 / clip.length_ms;
    }
    if clip.looping {
        while info.time >= 1.0 {
            info.time -= 1.0;
            info.cycle += 1;
        }
    } else if info.time >= 1.0 {
        info.time = if info.old_time >= 0.9999999 { 1.0 } else { 0.9999999 };
    }
    true
}

impl VmTree {
    /// XAnimSetGoalWeight: aim `s` at `goal` over `blend` seconds at `rate`. `restart`
    /// starts the clip again; a clip that is visibly playing crossfades from a fading copy.
    pub fn set_goal_weight(&mut self, s: usize, goal: f32, blend: f32, rate: f32, restart: bool) {
        let goal = if goal < 0.001 { 0.0 } else { goal };
        if self.infos[s].is_none() {
            if goal == 0.0 {
                return;
            }
            self.infos[s] = Some(AnimInfo::default());
        }
        let Some(info) = self.infos[s].as_mut() else {
            return;
        };
        if restart && (info.time != 0.0 || info.cycle != 0) && info.cycle == info.old_cycle {
            if blend != 0.0 && info.weight != 0.0 {
                let mut ghost = *info;
                blend_to(&mut ghost, 0.0, blend);
                ghost.goal = 0.0;
                self.ghosts.push((s, ghost));
                *info = AnimInfo::default();
            } else {
                info.time = 0.0;
                info.old_time = 0.0;
                info.cycle = 0;
                info.old_cycle = 0;
            }
        }
        blend_to(info, goal, blend);
        info.goal = goal;
        info.rate = rate;
    }

    /// XAnimClearTreeGoalWeights at blend 0: every slot off at once.
    pub fn clear(&mut self) {
        for s in 0..slot::COUNT {
            self.set_goal_weight(s, 0.0, 0.0, 1.0, false);
        }
        self.ghosts.clear();
    }

    /// True when `s` is not running, or has reached its end (or wrapped, for a loop)
    /// this frame.
    pub fn has_finished(&self, s: usize) -> bool {
        match &self.infos[s] {
            None => true,
            Some(i) => !(i.old_time <= i.time && i.time != 1.0 && i.cycle <= i.old_cycle),
        }
    }

    pub fn info(&self, s: usize) -> Option<&AnimInfo> {
        self.infos.get(s).and_then(Option::as_ref)
    }

    pub fn weight(&self, s: usize) -> f32 {
        self.info(s).map_or(0.0, |i| i.weight)
    }

    pub fn set_time(&mut self, s: usize, time: f32) {
        if let Some(i) = self.infos[s].as_mut() {
            i.time = time;
        }
    }

    pub fn set_rate(&mut self, s: usize, rate: f32) {
        if let Some(i) = self.infos[s].as_mut() {
            i.rate = rate;
        }
    }

    /// One frame (`dt` seconds): weights, then play positions.
    pub fn update(&mut self, dt: f32, set: &AnimSet) {
        for s in 1..slot::COUNT {
            if let Some(i) = self.infos[s].as_mut()
                && !advance(i, dt, set.clips[s])
            {
                self.infos[s] = None;
            }
        }
        self.ghosts
            .retain_mut(|(s, g)| advance(g, dt, set.clips[*s]));
    }

    /// Every slot with weight: (slot, weight, play position), fading copies included.
    pub fn pose(&self) -> Vec<(usize, f32, f32)> {
        let mut out: Vec<_> = (1..slot::COUNT)
            .filter_map(|s| self.infos[s].map(|i| (s, i.weight, i.time)))
            .filter(|p| p.1 > 0.0)
            .collect();
        out.extend(self.ghosts.iter().map(|(s, g)| (*s, g.weight, g.time)));
        out
    }
}

/// One hand's running slot and the last request it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hand {
    pub slot: usize,
    pub prev_anim: i32,
}

impl Default for Hand {
    fn default() -> Self {
        Self {
            slot: slot::NONE,
            prev_anim: -1,
        }
    }
}

/// What one client frame hands the anim player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VmFrame {
    pub weapon: u32,
    /// The viewmodel model in use. A new model rebuilds the tree.
    pub model: u32,
    pub weap_anim: i32,
    pub weap_anim_left: i32,
    pub weaponstate: i32,
    pub weaponstate_left: i32,
    pub weapon_time: i32,
    pub weapon_time_left: i32,
    pub ammo_in_clip: i32,
    /// The dual-wield partner's clip (left hand).
    pub ammo_in_clip_left: i32,
    pub intro_shots_fired: i32,
    pub weapon_pos_frac: f32,
    /// Player perks plus the weapon's own.
    pub perks: u64,
    pub off_hand_weapon: u32,
    pub zombies: bool,
    pub mantle_weapon_equipped: bool,
    pub mantle_duration_ms: i32,
    /// BO2's Mantle_DoAnim: the climb is higher than `mantle_weapon_anim_height`, so a kept gun plays its climb anim.
    pub mantle_do_anim: bool,
    /// The player's current move speed and their max (`ps.speed`), for the crawl rate.
    pub move_speed: f32,
    pub ps_speed: f32,
    pub pm_type: i32,
    pub frametime_ms: i32,
    /// Replay every request from scratch (BO2's forced pass).
    pub forced: bool,
    /// A cg flag the swap-time ADS pose reads for alt-ADS-only weapons.
    pub alt_ads_pose_flag: bool,
    pub dvars: VmDvars,
}

impl Default for VmFrame {
    fn default() -> Self {
        Self {
            weapon: 0,
            model: 0,
            weap_anim: 0,
            weap_anim_left: anim::LEFT_NONE,
            weaponstate: 0,
            weaponstate_left: 0,
            weapon_time: 0,
            weapon_time_left: 0,
            ammo_in_clip: 0,
            ammo_in_clip_left: 0,
            intro_shots_fired: 0,
            weapon_pos_frac: 0.0,
            perks: 0,
            off_hand_weapon: 0,
            zombies: false,
            mantle_weapon_equipped: false,
            mantle_duration_ms: 0,
            mantle_do_anim: false,
            move_speed: 0.0,
            ps_speed: 0.0,
            pm_type: pm_type::NORMAL,
            frametime_ms: 16,
            forced: false,
            alt_ads_pose_flag: false,
            dvars: VmDvars::default(),
        }
    }
}

impl VmFrame {
    fn has_perk(&self, p: u32) -> bool {
        p < 64 && self.perks & (1u64 << p) != 0
    }
}

/// The first-person gun and arms: the tree and what each hand is playing.
#[derive(Clone, Debug, Default)]
pub struct Viewmodel {
    pub tree: VmTree,
    pub hands: [Hand; 2],
    pub last_weapon: u32,
    built_model: Option<u32>,
    rng: u32,
}

/// Per-call context so the helpers don't carry six arguments each.
struct Ctx<'a> {
    f: &'a VmFrame,
    w: &'a VmFacts,
    table: &'a VmTable,
    set: &'a AnimSet,
}

impl Ctx<'_> {
    fn named(&self, s: usize) -> bool {
        self.set.named[s]
    }

    /// The right hand's clip is empty; unlimited ammo never is (BO2's rule).
    fn clip_empty(&self) -> bool {
        !self.w.unlimited_ammo && self.f.ammo_in_clip == 0
    }

    /// GetWeaponAnimRate: the rate that makes slot `s` last the weapon's time for it.
    fn anim_rate(&self, s: usize) -> f32 {
        // BO2 takes the clip length in whole ms here.
        let len = self.set.clips[s].length_ms.trunc();
        if (s == slot::MANTLE || s == slot::CAMERA_MANTLE) && self.f.mantle_weapon_equipped {
            let dur = self.f.mantle_duration_ms as f32;
            return if dur != 0.0 { len / dur } else { 1.0 };
        }
        // In the offhand states the anims playing are the offhand's own (its pin
        // pull, throw, etc.), so their times come from the offhand.
        let offhand = (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&self.f.weaponstate);
        let w = if offhand {
            self.table.get(self.f.off_hand_weapon).unwrap_or(self.w)
        } else {
            self.w
        };
        let mut t = w.times[s] as f32;
        if t <= 0.0 {
            return 1.0;
        }
        if s == slot::SPRINT_LOOP {
            t = t * self.w.sprint_cycle_scale * 0.5 * self.w.move_speed_scale
                / self.f.dvars.player_sprint_camera_bob;
            if t == 0.0 {
                return 1.0;
            }
        }
        len / t
    }

    /// GetWeaponAnimTimeFrac: where a forced restart of `s` picks up.
    fn time_frac(&self, s: usize) -> f32 {
        let weapon_time = match s {
            8 | 16..=31 | 64 => self.f.weapon_time,
            83 | 84 => self.f.weapon_time_left,
            _ => return 0.0,
        };
        let t = self.w.times[s] as f32;
        if t <= 0.0 {
            return 0.0;
        }
        ((t - weapon_time as f32) / t).clamp(0.0, 1.0)
    }

    /// The perk speed-up StartWeaponAnim divides the rate by (1 = none).
    fn perk_divisor(&self) -> f32 {
        let f = self.f;
        let d = &f.dvars;
        let st = f.weaponstate;
        let or_1000 = |m: f32| if m == 0.0 { 1000.0 } else { m };
        let reloading = |s: i32| (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&s);
        let offhand = (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st);
        let off_slot = self.table.get(f.off_hand_weapon).map(|o| o.offhand_slot);
        if (reloading(st) || reloading(f.weaponstate_left)) && f.has_perk(perk::FASTRELOAD) {
            or_1000(d.perk_weap_reload_multiplier)
        } else if (st == state::RECHAMBER_START || st == state::RECHAMBERING)
            && f.has_perk(perk::RATEOFFIRE)
        {
            or_1000(d.perk_weap_rate_multiplier)
        } else if f.has_perk(perk::FASTSWITCH)
            && (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&st)
            && self.w.offhand_slot != offhand_slot::SPECIFIC_USE
        {
            d.perk_weap_switch_multiplier
        } else if f.has_perk(perk::FASTMELEE) && st == state::MELEE_END {
            d.perk_weap_melee_multiplier
        } else if st == state::SPRINT_DROP && f.has_perk(PERK_FASTSPRINTRECOVERY) {
            or_1000(d.perk_sprint_recovery_multiplier)
        } else if offhand
            && f.has_perk(perk::FASTTOSS)
            && off_slot.is_some_and(|o| o != offhand_slot::EQUIPMENT)
        {
            d.perk_weap_toss_multiplier
        } else if offhand
            && f.has_perk(perk::FASTEQUIPMENTUSE)
            && off_slot == Some(offhand_slot::EQUIPMENT)
        {
            d.perk_weap_equipment_use_multiplier
        } else {
            1.0
        }
    }
}

impl Viewmodel {
    pub fn new() -> Self {
        Self {
            rng: 0x2545_f491,
            ..Self::default()
        }
    }

    /// Every slot with weight: (slot, weight, play position).
    pub fn pose(&self) -> Vec<(usize, f32, f32)> {
        self.tree.pose()
    }

    /// irand(0, n): 0..n.
    fn irand(&mut self, n: u32) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if n == 0 { 0 } else { self.rng % n }
    }

    /// StartWeaponAnim: play `s` from the top over `blend` seconds and fade every other
    /// gun slot (and the camera slots) out over the same time. The other hand's slot keeps
    /// playing on a dual-wield gun.
    fn start(&mut self, c: &Ctx, s: usize, blend: f32) {
        let rate = c.anim_rate(s) / c.perk_divisor();
        self.tree.set_goal_weight(s, 1.0, blend, rate, true);
        if c.f.forced {
            self.tree.set_time(s, c.time_frac(s));
        }
        for o in 1..=slot::RELOAD_LEFT {
            if o == s {
                continue;
            }
            if !c.w.dual_wield || (o != self.hands[0].slot && o != self.hands[1].slot) {
                self.tree.set_goal_weight(o, 0.0, blend, 1.0, false);
            }
        }
    }

    fn start_right(&mut self, c: &Ctx, s: usize, blend: f32) {
        self.hands[0].slot = s;
        self.start(c, s, blend);
    }

    fn start_camera(&mut self, c: &Ctx, s: usize) {
        let rate = c.anim_rate(s);
        self.tree.set_goal_weight(s, 1.0, 0.0, rate, true);
        for o in slot::CAMERA_SPRINT_LOOP..=slot::CAMERA_MANTLE {
            if o != s {
                self.tree.set_goal_weight(o, 0.0, 0.0, 1.0, false);
            }
        }
    }

    fn stop_camera(&mut self) {
        for o in slot::CAMERA_SPRINT_LOOP..=slot::CAMERA_MANTLE {
            self.tree.set_goal_weight(o, 0.0, 0.0, 1.0, false);
        }
    }

    fn play_idle(&mut self, c: &Ctx, blend: f32) {
        let s = if c.clip_empty() {
            slot::EMPTY_IDLE
        } else {
            slot::IDLE
        };
        self.start_right(c, s, blend);
    }

    fn play_left_idle(&mut self, c: &Ctx, blend: f32) {
        let s = if c.f.ammo_in_clip_left == 0 {
            slot::EMPTY_IDLE_LEFT
        } else {
            slot::IDLE_LEFT
        };
        self.hands[1].slot = s;
        self.start(c, s, blend);
    }

    /// The ADS up/down pose, every frame: held at the weapon's aim position.
    fn play_ads(&mut self, c: &Ctx) {
        let (alt_only, frac) = if c.w.aim_down_sight {
            (c.w.alt_ads_only(), c.f.weapon_pos_frac)
        } else if c.named(slot::ADS_DOWN) {
            (false, 0.0)
        } else {
            return;
        };
        let t = &mut self.tree;
        if c.w.aim_down_sight {
            t.set_goal_weight(slot::ADS_UP, if alt_only { 0.0 } else { 1.0 }, 0.1, 0.0, !alt_only);
            t.set_goal_weight(slot::ADS_UP_OTHER_SCOPE, if alt_only { 1.0 } else { 0.0 }, 0.1, 0.0, alt_only);
            t.set_goal_weight(slot::ADS_DOWN, 0.0, 0.1, 0.0, false);
        } else {
            t.set_goal_weight(slot::ADS_UP_OTHER_SCOPE, 0.0, 0.1, 0.0, false);
            t.set_goal_weight(slot::ADS_UP, 0.0, 0.1, 0.0, false);
            t.set_goal_weight(slot::ADS_DOWN, 1.0, 0.1, 0.0, false);
        }
        t.set_time(slot::ADS_UP_OTHER_SCOPE, frac);
        t.set_time(slot::ADS_UP, frac);
        t.set_time(slot::ADS_DOWN, 1.0 - frac);
    }

    /// Hold one ADS slot at its end (the swap-time pose).
    fn hold_ads_pose(&mut self, c: &Ctx, s: usize) {
        if s == slot::IDLE || c.named(s) {
            self.tree.set_goal_weight(s, 1.0, 0.0, 0.0, true);
            self.tree.set_time(s, 1.0);
        }
    }

    /// ChangeViewmodelDobj: a new viewmodel starts clean on its idle and its ADS pose.
    fn rebuild(&mut self, c: &Ctx) {
        self.hands[0].prev_anim = -1;
        self.hands[1].prev_anim = -1;
        self.tree.clear();
        self.tree.set_goal_weight(slot::ROOT, 1.0, 0.0, 1.0, false);
        let idle = if c.clip_empty() {
            slot::EMPTY_IDLE
        } else {
            slot::IDLE
        };
        self.tree.set_goal_weight(idle, 1.0, 0.0, 1.0, true);
        if c.f.alt_ads_pose_flag && c.w.alt_weapon_ads_only {
            let s = if c.f.weaponstate != state::RAISING_ALTSWITCH
                && c.f.weaponstate != state::DROPPING_ALTSWITCH
            {
                slot::ADS_DOWN
            } else if c.w.inventory_type == inventory::ALTMODE {
                slot::ADS_UP
            } else {
                slot::ADS_UP_OTHER_SCOPE
            };
            self.hold_ads_pose(c, s);
        } else if c.named(slot::ADS_DOWN) {
            self.hold_ads_pose(c, slot::ADS_DOWN);
        }
    }

    /// CG_ResetWeaponAnimTrees: everything off, idle on (dead players).
    fn reset_trees(&mut self, c: &Ctx) {
        self.tree.clear();
        self.tree.set_goal_weight(slot::ROOT, 1.0, 0.0, 1.0, false);
        if c.f.weapon != 0 {
            self.start(c, slot::IDLE, 0.0);
        }
    }

    /// One client frame (CG_UpdateViewWeaponAnim).
    pub fn frame(&mut self, f: &VmFrame, table: &VmTable, set: &AnimSet) {
        let none = VmFacts::default();
        let w = table.get(f.weapon).unwrap_or(&none);
        let c = Ctx { f, w, table, set };
        if f.pm_type > pm_type::LASTSTAND_TRANSITION {
            self.reset_trees(&c);
            return;
        }
        if self.built_model != Some(f.model) {
            self.built_model = Some(f.model);
            self.rebuild(&c);
        }
        self.run(&c);
        self.tree.update(f.frametime_ms as f32 * 0.001, set);
        if (state::CRAWL_FORWARD..=state::CRAWL_LEFT).contains(&f.weaponstate) {
            let k = (f.weaponstate - state::CRAWL_FORWARD) as usize;
            let s = slot::CRAWL_FORWARD + k;
            let t = w.times[s] as f32;
            if t != 0.0 && f.ps_speed != 0.0 {
                let rate = (f.move_speed / (f.ps_speed * 0.15)) / t * 1000.0;
                self.tree.set_rate(s, rate.abs());
            }
        }
    }

    /// WeaponRunXModelAnims.
    fn run(&mut self, c: &Ctx) {
        let f = c.f;
        self.play_ads(c);
        if c.w.dual_wield
            && (f.forced || f.weapon != self.last_weapon || f.weap_anim_left != self.hands[1].prev_anim)
        {
            self.run_left(c);
        }
        if f.forced || f.weapon != self.last_weapon || f.weap_anim != self.hands[0].prev_anim {
            if self.run_right(c) {
                self.hands[0].prev_anim = f.weap_anim;
            }
            self.last_weapon = f.weapon;
        }
        if c.w.dual_wield {
            self.ensure_left(c);
        }
    }

    fn run_left(&mut self, c: &Ctx) {
        let f = c.f;
        let req = anim::number(f.weap_anim_left);
        match req {
            anim::IDLE | anim::FORCE_IDLE => {
                let mut blend = 0.0;
                for s in slot::FIRE_LEFT..=slot::RELOAD_LEFT {
                    if !self.tree.has_finished(s) {
                        self.hands[1].prev_anim = -1;
                        if req == anim::IDLE {
                            return;
                        }
                        if !f.forced {
                            blend = 0.5;
                        }
                    }
                }
                self.play_left_idle(c, blend);
            }
            anim::ATTACK => self.start_left(c, slot::FIRE_LEFT),
            anim::ATTACK_LASTSHOT => self.start_left(c, slot::LASTSHOT_LEFT),
            anim::RELOAD => self.start_left(c, slot::RELOAD_LEFT),
            anim::RELOAD_EMPTY => self.start_left(c, slot::RELOAD_EMPTY_LEFT),
            _ => self.hands[1].slot = slot::NONE,
        }
        self.hands[1].prev_anim = f.weap_anim_left;
    }

    fn start_left(&mut self, c: &Ctx, s: usize) {
        self.hands[1].slot = s;
        self.start(c, s, 0.0);
    }

    /// The right hand. False = an idle request is waiting for the running anim to end
    /// (the request is not marked as taken, so it is checked again next frame).
    fn run_right(&mut self, c: &Ctx) -> bool {
        let f = c.f;
        let req = anim::number(f.weap_anim);
        let named = |s| c.named(s);
        match req {
            anim::IDLE | anim::FORCE_IDLE => {
                let mut blend = 0.0;
                for s in 1..=slot::RELOAD_LEFT {
                    if !self.tree.has_finished(s) && s != self.hands[1].slot {
                        self.hands[0].prev_anim = -1;
                        if req == anim::IDLE {
                            return false;
                        }
                        blend = if f.forced { 0.0 } else { 0.5 };
                        if f.has_perk(perk::FASTSWITCH) {
                            blend *= f.dvars.perk_weap_switch_multiplier;
                        }
                    }
                }
                self.play_idle(c, blend);
            }
            anim::ATTACK => {
                let intro = c.w.intro_fire_length != 0 && f.intro_shots_fired < c.w.intro_fire_length;
                self.start_right(c, if intro { slot::FIRE_INTRO } else { slot::FIRE }, 0.0);
            }
            anim::ATTACK_LASTSHOT => self.start_right(c, slot::LASTSHOT, 0.0),
            anim::ATTACK_LASTSHOT_LEFT => {
                self.hands[1].slot = slot::LASTSHOT_LEFT;
                self.start(c, slot::LASTSHOT_LEFT, 0.0);
            }
            anim::RECHAMBER => self.start_right(c, slot::RECHAMBER, 0.0),
            anim::ADS_ATTACK => {
                let intro = c.w.intro_fire_length != 0 && f.intro_shots_fired < c.w.intro_fire_length;
                self.start_right(c, if intro { slot::ADS_FIRE_INTRO } else { slot::ADS_FIRE }, 0.0);
            }
            anim::ADS_ATTACK_LASTSHOT => self.start_right(c, slot::ADS_LASTSHOT, 0.0),
            anim::ADS_RECHAMBER => self.start_right(c, slot::ADS_RECHAMBER, 0.0),
            anim::MELEE_ATTACK => self.melee(c),
            anim::MELEE_ATTACK_EMPTY => {
                if named(slot::MELEE_EMPTY) {
                    self.start_right(c, slot::MELEE_EMPTY, 0.0);
                } else {
                    self.melee(c);
                }
            }
            anim::MELEE_CHARGE => self.start_right(c, slot::MELEE_CHARGE, 0.0),
            anim::MELEE_CHARGE_EMPTY => {
                let s = if named(slot::MELEE_CHARGE_EMPTY) {
                    slot::MELEE_CHARGE_EMPTY
                } else {
                    slot::MELEE_CHARGE
                };
                self.start_right(c, s, 0.0);
            }
            anim::DROP => self.start_right(c, slot::DROP, 0.0),
            anim::RAISE => self.start_right(c, slot::RAISE, 0.0),
            anim::FIRST_RAISE => self.start_right(c, slot::FIRST_RAISE, 0.0),
            anim::RELOAD => self.start_right(c, slot::RELOAD, 0.0),
            anim::RELOAD_EMPTY => self.start_right(c, slot::RELOAD_EMPTY, 0.0),
            anim::RELOAD_START..=anim::RELOAD_QUICK_EMPTY => {
                self.start_right(c, req as usize, 0.0);
            }
            anim::ALTSWITCHFROM => self.start_right(c, slot::ALT_DROP, 0.0),
            anim::ALTSWITCHTO => self.start_right(c, slot::ALT_RAISE, 0.0),
            anim::QUICK_DROP => self.start_right(c, slot::QUICK_DROP, 0.0),
            anim::QUICK_RAISE => self.start_right(c, slot::QUICK_RAISE, 0.0),
            anim::EMPTY_DROP => self.start_right(c, slot::EMPTY_DROP, 0.0),
            anim::EMPTY_RAISE => self.start_right(c, slot::EMPTY_RAISE, 0.0),
            anim::SPRINT_IN => {
                if named(slot::SPRINT_IN) {
                    self.start_right(c, slot::SPRINT_IN, 0.0);
                }
            }
            anim::SPRINT_LOOP => {
                if named(slot::SPRINT_IN) {
                    self.start_right(c, slot::SPRINT_LOOP, 0.0);
                } else {
                    self.play_idle(c, 0.0);
                }
                self.camera_if_named(c, slot::CAMERA_SPRINT_LOOP);
            }
            anim::SPRINT_OUT => {
                if named(slot::SPRINT_OUT) {
                    self.start_right(c, slot::SPRINT_OUT, 0.0);
                }
                if named(slot::CAMERA_SPRINT_LOOP) {
                    self.stop_camera();
                }
            }
            anim::SPRINT_EMPTY_IN => {
                if named(slot::SPRINT_EMPTY_IN) {
                    self.start_right(c, slot::SPRINT_EMPTY_IN, 0.0);
                } else if named(slot::SPRINT_IN) {
                    self.start_right(c, slot::SPRINT_IN, 0.0);
                }
            }
            anim::SPRINT_EMPTY_LOOP => {
                if named(slot::SPRINT_EMPTY_LOOP) {
                    self.start_right(c, slot::SPRINT_EMPTY_LOOP, 0.0);
                } else if named(slot::SPRINT_IN) {
                    self.start_right(c, slot::SPRINT_LOOP, 0.0);
                } else {
                    self.play_idle(c, 0.0);
                }
                self.camera_if_named(c, slot::CAMERA_SPRINT_LOOP);
            }
            anim::SPRINT_EMPTY_OUT => {
                if named(slot::SPRINT_EMPTY_OUT) {
                    self.start_right(c, slot::SPRINT_EMPTY_OUT, 0.0);
                } else if named(slot::SPRINT_OUT) {
                    self.start_right(c, slot::SPRINT_OUT, 0.0);
                }
                if named(slot::CAMERA_SPRINT_LOOP) {
                    self.stop_camera();
                }
            }
            anim::LOWREADY_IN | anim::LOWREADY_LOOP | anim::LOWREADY_OUT => {
                let s = slot::LOWREADY_IN + (req - anim::LOWREADY_IN) as usize;
                if f.zombies && named(s) {
                    self.start_right(c, s, 0.0);
                } else {
                    self.play_idle(c, 0.0);
                }
            }
            anim::CONT_FIRE_IN | anim::CONT_FIRE_LOOP | anim::CONT_FIRE_OUT => {
                let s = slot::CONT_FIRE_IN + (req - anim::CONT_FIRE_IN) as usize;
                self.start_right(c, s, 0.0);
            }
            anim::HOLD_FIRE => self.start_right(c, slot::HOLD_FIRE, 0.0),
            anim::DETONATE => self.start_right(c, slot::DETONATE, 0.0),
            anim::NIGHTVISION_WEAR => self.start_right(c, slot::NIGHTVISION_WEAR, 0.0),
            anim::NIGHTVISION_REMOVE => self.start_right(c, slot::NIGHTVISION_REMOVE, 0.0),
            anim::DEPLOYING => self.start_right(c, slot::DEPLOY, 0.0),
            anim::BREAKING_DOWN => self.start_right(c, slot::BREAKDOWN, 0.0),
            anim::DTP_IN | anim::DTP_LOOP | anim::DTP_OUT => {
                let k = (req - anim::DTP_IN) as usize;
                self.named_or_idle(c, &[slot::DTP_IN + k]);
                self.camera_if_named(c, slot::CAMERA_DTP_IN + k);
            }
            anim::DTP_EMPTY_IN | anim::DTP_EMPTY_LOOP | anim::DTP_EMPTY_OUT => {
                let k = (req - anim::DTP_EMPTY_IN) as usize;
                self.named_or_idle(c, &[slot::DTP_EMPTY_IN + k, slot::DTP_IN + k]);
                self.camera_if_named(c, slot::CAMERA_DTP_IN + k);
            }
            anim::SLIDE_IN => self.named_or_idle(c, &[slot::SLIDE_IN]),
            anim::MANTLE => {
                let do_anim = f.mantle_do_anim;
                if named(slot::MANTLE) && do_anim {
                    self.start_right(c, slot::MANTLE, 0.0);
                } else {
                    self.play_idle(c, 0.0);
                }
                if named(slot::CAMERA_MANTLE) && do_anim {
                    self.start_camera(c, slot::CAMERA_MANTLE);
                }
            }
            anim::CRAWL_IN => {
                if named(slot::CRAWL_IN) {
                    self.start_right(c, slot::CRAWL_IN, 0.0);
                }
            }
            anim::CRAWL_FORWARD..=anim::CRAWL_LEFT => {
                let s = slot::CRAWL_FORWARD + (req - anim::CRAWL_FORWARD) as usize;
                if named(s) {
                    self.start_right(c, s, 0.5);
                }
            }
            anim::CRAWL_OUT => {
                if named(slot::CRAWL_OUT) {
                    self.start_right(c, slot::CRAWL_OUT, 0.0);
                }
            }
            anim::CRAWL_EMPTY_IN => {
                if named(slot::CRAWL_EMPTY_IN) {
                    self.start_right(c, slot::CRAWL_EMPTY_IN, 0.0);
                } else if named(slot::CRAWL_IN) {
                    self.start_right(c, slot::CRAWL_IN, 0.0);
                }
            }
            anim::CRAWL_EMPTY_FORWARD..=anim::CRAWL_EMPTY_LEFT => {
                let k = (req - anim::CRAWL_EMPTY_FORWARD) as usize;
                if named(slot::CRAWL_EMPTY_FORWARD + k) {
                    self.start_right(c, slot::CRAWL_EMPTY_FORWARD + k, 0.5);
                } else if named(slot::CRAWL_FORWARD + k) {
                    self.start_right(c, slot::CRAWL_FORWARD + k, 0.5);
                }
            }
            anim::CRAWL_EMPTY_OUT => {
                if named(slot::CRAWL_EMPTY_OUT) {
                    self.start_right(c, slot::CRAWL_EMPTY_OUT, 0.0);
                } else if named(slot::CRAWL_OUT) {
                    self.start_right(c, slot::CRAWL_OUT, 0.0);
                }
            }
            // ATTACK_LEFT, JAM and anything unknown.
            _ => self.play_idle(c, 0.0),
        }
        true
    }

    fn named_or_idle(&mut self, c: &Ctx, slots: &[usize]) {
        match slots.iter().copied().find(|&s| c.named(s)) {
            Some(s) => self.start_right(c, s, 0.0),
            None => self.play_idle(c, 0.0),
        }
    }

    fn camera_if_named(&mut self, c: &Ctx, s: usize) {
        if c.named(s) {
            self.start_camera(c, s);
        }
    }

    /// One of the weapon's named melee swings, picked at random.
    fn melee(&mut self, c: &Ctx) {
        let mut n = 1;
        if c.named(slot::MELEE1) {
            n = 2;
            if c.named(slot::MELEE2) {
                n = 3;
                if c.named(slot::MELEE3) {
                    n = 4;
                }
            }
        }
        let s = slot::MELEE + self.irand(n) as usize;
        self.start_right(c, s, 0.0);
    }

    /// EnsureValidLeftAnim: a dual-wield right hand on idle/fire/reload with nothing on
    /// the left puts the left gun back on its idle.
    fn ensure_left(&mut self, c: &Ctx) {
        let right_ok = matches!(self.hands[0].slot, 1 | 2 | 4 | 6 | 7 | 16 | 18);
        let left_empty = (slot::FIRE_LEFT..=slot::RELOAD_LEFT).all(|s| self.tree.weight(s) == 0.0);
        if right_ok && left_empty {
            self.hands[1].slot = slot::IDLE_LEFT;
            self.start(c, slot::IDLE_LEFT, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1;

    fn table(mut f: impl FnMut(&mut VmFacts)) -> VmTable {
        let mut v = VmFacts::default();
        v.times[slot::RAISE] = 600;
        v.times[slot::DROP] = 400;
        v.times[slot::RELOAD] = 2000;
        v.times[slot::SPRINT_IN] = 300;
        v.times[slot::SPRINT_LOOP] = 1000;
        v.sprint_cycle_scale = 1.0;
        v.move_speed_scale = 1.0;
        f(&mut v);
        let mut t = VmTable::default();
        t.set(W, v);
        t
    }

    fn set_with(named: &[usize]) -> AnimSet {
        let mut right = [None; slot::COUNT];
        right[slot::IDLE] = Some(Clip { length_ms: 2000.0, looping: true });
        for &s in named {
            right[s] = Some(Clip { length_ms: 1000.0, looping: s == slot::SPRINT_LOOP });
        }
        AnimSet::build(&right, &[None; slot::COUNT])
    }

    fn frame(anim: i32) -> VmFrame {
        VmFrame {
            weapon: W,
            model: W,
            weap_anim: anim,
            ammo_in_clip: 30,
            frametime_ms: 50,
            ..VmFrame::default()
        }
    }

    #[test]
    fn slot_names_line_up() {
        assert_eq!(slot::name(slot::ADS_UP_OTHER_SCOPE), "ads_up_other_scope");
        assert_eq!(slot::name(slot::CAMERA_MANTLE), "camera_mantle");
        assert_eq!(slot::find("reload_left"), Some(slot::RELOAD_LEFT));
    }

    #[test]
    fn unlimited_ammo_at_clip_zero_idles_full() {
        let set = set_with(&[slot::EMPTY_IDLE]);
        let mut f = frame(anim::start(0, anim::IDLE));
        f.ammo_in_clip = 0;
        for (unlimited, want, other) in
            [(false, slot::EMPTY_IDLE, slot::IDLE), (true, slot::IDLE, slot::EMPTY_IDLE)]
        {
            let t = table(|v| v.unlimited_ammo = unlimited);
            let mut vm = Viewmodel::new();
            for _ in 0..40 {
                vm.frame(&f, &t, &set);
            }
            assert_eq!(vm.tree.weight(want), 1.0, "unlimited {unlimited}");
            assert_eq!(vm.tree.weight(other), 0.0, "unlimited {unlimited}");
        }
    }

    #[test]
    fn raise_lasts_the_raise_time_then_idle_snaps_in() {
        let t = table(|_| {});
        let set = set_with(&[slot::RAISE]);
        let mut vm = Viewmodel::new();
        let a = anim::start(0, anim::RAISE);
        vm.frame(&frame(a), &t, &set);
        assert_eq!(vm.hands[0].slot, slot::RAISE);
        let r = vm.tree.info(slot::RAISE).unwrap();
        assert!((r.rate - 1000.0 / 600.0).abs() < 1e-4);
        // The gun asks for idle right away; idle waits for the raise to end.
        let idle = anim::start(a, anim::IDLE);
        let mut frames = 1;
        while vm.hands[0].slot != slot::IDLE {
            vm.frame(&frame(idle), &t, &set);
            frames += 1;
            assert!(frames < 40);
        }
        // 600 ms at 50 ms frames: the raise ends on frame 12, idle starts on frame 13.
        assert_eq!(frames, 14);
        assert_eq!(vm.tree.weight(slot::IDLE), 1.0);
        assert_eq!(vm.tree.weight(slot::RAISE), 0.0);
    }

    #[test]
    fn force_idle_blends_half_a_second() {
        let t = table(|_| {});
        let set = set_with(&[slot::RELOAD]);
        let mut vm = Viewmodel::new();
        let a = anim::start(0, anim::RELOAD);
        vm.frame(&frame(a), &t, &set);
        let b = anim::start(a, anim::FORCE_IDLE);
        vm.frame(&frame(b), &t, &set);
        let i = *vm.tree.info(slot::IDLE).unwrap();
        assert_eq!(i.goal, 1.0);
        assert!(i.weight < 0.2);
        let mut perk = frame(anim::start(b, anim::RELOAD));
        vm.frame(&perk, &t, &set);
        perk.weap_anim = anim::start(perk.weap_anim, anim::FORCE_IDLE);
        perk.perks = 1 << perk::FASTSWITCH;
        vm.frame(&perk, &t, &set);
        // Fast hands halves the blend: weight after one 50 ms frame is about twice as far.
        assert!(vm.tree.weight(slot::IDLE) > i.weight * 1.5);
    }

    #[test]
    fn fast_reload_divides_the_rate() {
        let t = table(|_| {});
        let set = set_with(&[slot::RELOAD]);
        let mut vm = Viewmodel::new();
        let mut f = frame(anim::start(0, anim::RELOAD));
        f.weaponstate = state::RELOADING;
        f.perks = 1 << perk::FASTRELOAD;
        vm.frame(&f, &t, &set);
        let r = vm.tree.info(slot::RELOAD).unwrap().rate;
        assert!((r - (1000.0 / 2000.0) / 0.5).abs() < 1e-4);
    }

    #[test]
    fn sprint_in_needs_its_anim_and_loop_runs_the_camera() {
        let t = table(|_| {});
        let bare = set_with(&[]);
        let mut vm = Viewmodel::new();
        // No sprint_in anim: the request does nothing; the idle the new viewmodel started keeps playing.
        vm.frame(&frame(anim::start(0, anim::SPRINT_IN)), &t, &bare);
        assert_eq!(vm.hands[0].slot, slot::NONE);
        assert_eq!(vm.tree.weight(slot::IDLE), 1.0);
        let set = set_with(&[slot::SPRINT_IN, slot::SPRINT_LOOP, slot::CAMERA_SPRINT_LOOP]);
        let mut vm = Viewmodel::new();
        let a = anim::start(0, anim::SPRINT_IN);
        vm.frame(&frame(a), &t, &set);
        assert_eq!(vm.hands[0].slot, slot::SPRINT_IN);
        vm.frame(&frame(anim::start(a, anim::SPRINT_LOOP)), &t, &set);
        assert_eq!(vm.hands[0].slot, slot::SPRINT_LOOP);
        assert_eq!(vm.tree.weight(slot::CAMERA_SPRINT_LOOP), 1.0);
    }

    #[test]
    fn restart_while_playing_crossfades_from_a_copy() {
        let mut tree = VmTree::default();
        let set = set_with(&[slot::FIRE]);
        tree.set_goal_weight(slot::FIRE, 1.0, 0.0, 1.0, true);
        tree.update(0.05, &set);
        tree.set_goal_weight(slot::FIRE, 1.0, 0.2, 1.0, true);
        assert_eq!(tree.ghosts.len(), 1);
        assert_eq!(tree.info(slot::FIRE).unwrap().time, 0.0);
        for _ in 0..5 {
            tree.update(0.05, &set);
        }
        assert!(tree.ghosts.is_empty());
        assert_eq!(tree.weight(slot::FIRE), 1.0);
    }

    #[test]
    fn non_looping_clip_holds_its_last_frame_then_finishes() {
        let mut tree = VmTree::default();
        let set = set_with(&[slot::FIRE]);
        tree.set_goal_weight(slot::FIRE, 1.0, 0.0, 1.0, true);
        tree.update(1.5, &set);
        assert_eq!(tree.info(slot::FIRE).unwrap().time, 0.9999999);
        assert!(!tree.has_finished(slot::FIRE));
        tree.update(0.05, &set);
        assert_eq!(tree.info(slot::FIRE).unwrap().time, 1.0);
        assert!(tree.has_finished(slot::FIRE));
    }

    #[test]
    fn ads_pose_follows_the_aim_fraction() {
        let t = table(|v| v.aim_down_sight = true);
        let set = set_with(&[slot::ADS_UP, slot::ADS_DOWN]);
        let mut vm = Viewmodel::new();
        let mut f = frame(0);
        f.weapon_pos_frac = 0.25;
        vm.frame(&f, &t, &set);
        let up = vm.tree.info(slot::ADS_UP).unwrap();
        assert_eq!(up.rate, 0.0);
        assert_eq!(up.goal, 1.0);
    }

    #[test]
    fn dual_wield_left_hand_fires_on_its_own() {
        let t = table(|v| v.dual_wield = true);
        let set = set_with(&[slot::FIRE]);
        let mut vm = Viewmodel::new();
        let mut f = frame(0);
        f.ammo_in_clip_left = 12;
        f.weap_anim_left = anim::start(0, anim::IDLE);
        vm.frame(&f, &t, &set);
        assert_eq!(vm.hands[1].slot, slot::IDLE_LEFT);
        f.weap_anim_left = anim::start(f.weap_anim_left, anim::ATTACK);
        vm.frame(&f, &t, &set);
        assert_eq!(vm.hands[1].slot, slot::FIRE_LEFT);
        assert_eq!(vm.tree.weight(slot::FIRE_LEFT), 1.0);
        // The right idle waits one frame for the faded left idle to clear, then starts
        // without touching the left hand's fire.
        vm.frame(&f, &t, &set);
        assert_eq!(vm.hands[0].slot, slot::IDLE);
        assert_eq!(vm.tree.weight(slot::IDLE), 1.0);
        assert_eq!(vm.tree.weight(slot::FIRE_LEFT), 1.0);
    }

    #[test]
    fn dead_player_resets_to_idle() {
        let t = table(|_| {});
        let set = set_with(&[slot::RELOAD]);
        let mut vm = Viewmodel::new();
        vm.frame(&frame(anim::start(0, anim::RELOAD)), &t, &set);
        let mut f = frame(anim::start(0, anim::RELOAD));
        f.pm_type = pm_type::DEAD;
        vm.frame(&f, &t, &set);
        assert_eq!(vm.tree.weight(slot::IDLE), 1.0);
        assert_eq!(vm.tree.weight(slot::RELOAD), 0.0);
    }

    #[test]
    fn a_reindexed_table_finds_each_gun_by_name_then_base_name() {
        let mut def = vec![0u8; D::aimDownSight + 1];
        def[D::aimDownSight] = 1;
        let hex: String = def.iter().map(|b| format!("{b:02x}")).collect();
        let dump = format!("fiveseven_mp\t00\t00\nmp7_mp\t00\t{hex}\n");
        let t = VmTable::from_dump_reindexed(
            &dump,
            [(4, "mp7_mp"), (9, "mp7_mp+reflex"), (2, "fiveseven_mp"), (3, "nothing_mp")],
        );
        assert!(t.get(4).is_some_and(|f| f.aim_down_sight));
        assert!(t.get(9).is_some_and(|f| f.aim_down_sight));
        assert!(t.get(2).is_some_and(|f| !f.aim_down_sight));
        assert!(t.get(3).is_none());
        assert!(t.get(1).is_none());
    }

    #[test]
    fn climb_anim_plays_only_when_bo2_says_so() {
        // Mantle_DoAnim: a kept gun on a climb above 32 units plays its climb anim and camera,
        // stretched to the climb's length; a lower one just idles.
        let t = table(|_| {});
        let set = set_with(&[slot::MANTLE, slot::CAMERA_MANTLE]);
        for (do_anim, want) in [(true, slot::MANTLE), (false, slot::IDLE)] {
            let mut vm = Viewmodel::new();
            let mut f = frame(anim::start(0, anim::MANTLE));
            f.mantle_weapon_equipped = true;
            f.mantle_duration_ms = 800;
            f.mantle_do_anim = do_anim;
            vm.frame(&f, &t, &set);
            assert_eq!(vm.hands[0].slot, want, "do_anim {do_anim}");
            let cam = vm.tree.info(slot::CAMERA_MANTLE).map_or(0.0, |i| i.goal);
            assert_eq!(cam, if do_anim { 1.0 } else { 0.0 }, "do_anim {do_anim}");
            if do_anim {
                let r = vm.tree.info(slot::MANTLE).unwrap();
                assert!((r.rate - 1000.0 / 800.0).abs() < 1e-4);
            }
        }
    }
}
