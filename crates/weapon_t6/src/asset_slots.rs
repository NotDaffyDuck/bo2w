//! The viewmodel animation slots the weapon catalog stores a gun's anims in, and how a BO2
//! viewmodel anim (`crate::anim`) picks one. The old viewmodel player (render_anim
//! `viewmodel_controller`) plays these; BO2's own player is `crate::viewmodel`.
//!
//! TODO(fact): the layout is the catalog's, not BO2's: 37 slots plus 28 more for anims BO2
//! has on top. BO2 keeps its anims in `crate::viewmodel::slot`; this goes when the catalog
//! stores those.

use crate::anim as a;
use weap_anim_slot as weap_anim;

/// Slots in the base layout; [`weap_anim_extra`] numbers the ones after it.
pub const WEAPON_ANIM_COUNT: usize = 37;
pub const WEAPON_ANIM_SLOTS: usize = WEAPON_ANIM_COUNT + 28;

pub mod weap_anim_slot {
    pub const ROOT: usize = 0x0;
    pub const IDLE: usize = 0x1;
    pub const EMPTY_IDLE: usize = 0x2;
    pub const FIRE: usize = 0x3;
    pub const HOLD_FIRE: usize = 0x4;
    pub const LASTSHOT: usize = 0x5;
    pub const RECHAMBER: usize = 0x6;
    pub const MELEE: usize = 0x7;
    pub const MELEE_CHARGE: usize = 0x8;
    pub const RELOAD: usize = 0x9;
    pub const RELOAD_EMPTY: usize = 0xA;
    pub const RELOAD_START: usize = 0xB;
    pub const RELOAD_END: usize = 0xC;
    pub const RAISE: usize = 0xD;
    pub const FIRST_RAISE: usize = 0xE;
    pub const BREACH_RAISE: usize = 0xF;
    pub const DROP: usize = 0x10;
    pub const ALT_RAISE: usize = 0x11;
    pub const ALT_DROP: usize = 0x12;
    pub const QUICK_RAISE: usize = 0x13;
    pub const QUICK_DROP: usize = 0x14;
    pub const EMPTY_RAISE: usize = 0x15;
    pub const EMPTY_DROP: usize = 0x16;
    pub const SPRINT_IN: usize = 0x17;
    pub const SPRINT_LOOP: usize = 0x18;
    pub const SPRINT_OUT: usize = 0x19;
    pub const STUNNED_START: usize = 0x1A;
    pub const STUNNED_LOOP: usize = 0x1B;
    pub const STUNNED_END: usize = 0x1C;
    pub const DETONATE: usize = 0x1D;
    pub const NIGHTVISION_WEAR: usize = 0x1E;
    pub const NIGHTVISION_REMOVE: usize = 0x1F;
    pub const ADS_FIRE: usize = 0x20;
    pub const ADS_LASTSHOT: usize = 0x21;
    pub const ADS_RECHAMBER: usize = 0x22;
    pub const ADS_UP: usize = 0x23;
    pub const ADS_DOWN: usize = 0x24;
}

pub mod weap_anim_extra {
    use super::WEAPON_ANIM_COUNT;

    pub const RELOAD_QUICK: usize = WEAPON_ANIM_COUNT;
    pub const RELOAD_QUICK_EMPTY: usize = WEAPON_ANIM_COUNT + 1;
    /// bo2zm: Black Ops II's dive-to-prone animations (`d2p_in`, `_loop`,
    /// `_out`, and their empty-magazine ones).
    pub const DIVE_IN: usize = WEAPON_ANIM_COUNT + 2;
    pub const DIVE_LOOP: usize = WEAPON_ANIM_COUNT + 3;
    pub const DIVE_OUT: usize = WEAPON_ANIM_COUNT + 4;
    pub const DIVE_IN_EMPTY: usize = WEAPON_ANIM_COUNT + 5;
    pub const DIVE_LOOP_EMPTY: usize = WEAPON_ANIM_COUNT + 6;
    pub const DIVE_OUT_EMPTY: usize = WEAPON_ANIM_COUNT + 7;
    /// bo2zm: Black Ops II's empty-magazine sprint animations.
    pub const SPRINT_IN_EMPTY: usize = WEAPON_ANIM_COUNT + 8;
    pub const SPRINT_LOOP_EMPTY: usize = WEAPON_ANIM_COUNT + 9;
    pub const SPRINT_OUT_EMPTY: usize = WEAPON_ANIM_COUNT + 10;
    /// bo2zm: Black Ops II's prone crawl animations: in, the four
    /// directions (looping), out; then their empty-magazine ones.
    pub const CRAWL_IN: usize = WEAPON_ANIM_COUNT + 11;
    pub const CRAWL_FORWARD: usize = WEAPON_ANIM_COUNT + 12;
    pub const CRAWL_BACK: usize = WEAPON_ANIM_COUNT + 13;
    pub const CRAWL_RIGHT: usize = WEAPON_ANIM_COUNT + 14;
    pub const CRAWL_LEFT: usize = WEAPON_ANIM_COUNT + 15;
    pub const CRAWL_OUT: usize = WEAPON_ANIM_COUNT + 16;
    pub const CRAWL_IN_EMPTY: usize = WEAPON_ANIM_COUNT + 17;
    pub const CRAWL_FORWARD_EMPTY: usize = WEAPON_ANIM_COUNT + 18;
    pub const CRAWL_BACK_EMPTY: usize = WEAPON_ANIM_COUNT + 19;
    pub const CRAWL_RIGHT_EMPTY: usize = WEAPON_ANIM_COUNT + 20;
    pub const CRAWL_LEFT_EMPTY: usize = WEAPON_ANIM_COUNT + 21;
    pub const CRAWL_OUT_EMPTY: usize = WEAPON_ANIM_COUNT + 22;
    /// bo2zm: Black Ops II's climb camera animation: it moves only the
    /// camera, played over a low climb that keeps the gun in hand.
    pub const CAMERA_MANTLE: usize = WEAPON_ANIM_COUNT + 23;
    /// bo2zm: Black Ops II's melee and charged melee with an empty
    /// magazine (a pistol's slide stays back).
    pub const MELEE_EMPTY: usize = WEAPON_ANIM_COUNT + 24;
    pub const MELEE_CHARGE_EMPTY: usize = WEAPON_ANIM_COUNT + 25;
    /// bo2zm: Black Ops II's first-shots fire, hip and aimed (the HAMR's
    /// fast start, the AN-94's intro shot).
    pub const FIRE_INTRO: usize = WEAPON_ANIM_COUNT + 26;
    pub const ADS_FIRE_INTRO: usize = WEAPON_ANIM_COUNT + 27;
}

pub const ACTION_GOAL_TIME_SECS: f32 = 0.0;
pub const IDLE_INTERRUPT_GOAL_TIME_SECS: f32 = 0.5;
pub const ACTIVE_GOAL_WEIGHT: f32 = 1.0;
pub const INACTIVE_GOAL_WEIGHT: f32 = 0.0;

/// Base slots whose clip is stretched over the gun's own time for the action
/// (fire, reload, raise, drop and the rest); the others play at the clip's rate.
const TIMED_SLOTS: [usize; 25] = [
    4, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 31,
];

/// How fast a clip plays so it lasts the action's `weapon_timer_ms`.
pub fn playback_rate(clip_length_ms: i32, weapon_timer_ms: i32) -> f32 {
    if weapon_timer_ms < 0 {
        return 1.0;
    }
    if weapon_timer_ms == 0 {
        return 0.0;
    }
    if clip_length_ms <= 0 {
        return 1.0;
    }
    clip_length_ms as f32 / weapon_timer_ms as f32
}

/// Whether `slot` plays at its clip's own rate rather than over the gun's time.
pub fn slot_uses_native_rate(slot: usize) -> bool {
    // The dive, crawl and empty-magazine melee animations play over the gun's own times.
    if (weap_anim_extra::DIVE_IN..=weap_anim_extra::DIVE_OUT_EMPTY).contains(&slot)
        || (weap_anim_extra::CRAWL_IN..=weap_anim_extra::CRAWL_OUT_EMPTY).contains(&slot)
        || (weap_anim_extra::MELEE_EMPTY..=weap_anim_extra::MELEE_CHARGE_EMPTY).contains(&slot)
    {
        return false;
    }
    !TIMED_SLOTS.contains(&slot)
}

/// What a BO2 viewmodel anim asks the old viewmodel player to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmAnim {
    Idle,
    ForceIdle,
    Slot(usize),
}

/// The slot BO2 viewmodel anim `anim` plays (its toggle bit ignored). Anims with no slot of
/// their own (slide, climb, jam, the crawl-in) read as idle.
pub fn vm_anim(anim: i32) -> VmAnim {
    use weap_anim_extra as x;
    let s = match a::number(anim) {
        a::FORCE_IDLE => return VmAnim::ForceIdle,
        a::ATTACK | a::ATTACK_LEFT | a::CONT_FIRE_IN | a::CONT_FIRE_LOOP | a::CONT_FIRE_OUT => {
            weap_anim::FIRE
        }
        a::ATTACK_LASTSHOT | a::ATTACK_LASTSHOT_LEFT => weap_anim::LASTSHOT,
        a::RECHAMBER => weap_anim::RECHAMBER,
        a::ADS_ATTACK => weap_anim::ADS_FIRE,
        a::ADS_ATTACK_LASTSHOT => weap_anim::ADS_LASTSHOT,
        a::ADS_RECHAMBER => weap_anim::ADS_RECHAMBER,
        a::MELEE_ATTACK => weap_anim::MELEE,
        a::MELEE_CHARGE => weap_anim::MELEE_CHARGE,
        a::DROP | a::BREAKING_DOWN => weap_anim::DROP,
        a::RAISE | a::DEPLOYING => weap_anim::RAISE,
        a::FIRST_RAISE => weap_anim::FIRST_RAISE,
        a::RELOAD => weap_anim::RELOAD,
        a::RELOAD_EMPTY => weap_anim::RELOAD_EMPTY,
        a::RELOAD_START => weap_anim::RELOAD_START,
        a::RELOAD_END => weap_anim::RELOAD_END,
        a::ALTSWITCHFROM => weap_anim::ALT_DROP,
        a::ALTSWITCHTO => weap_anim::ALT_RAISE,
        a::QUICK_DROP => weap_anim::QUICK_DROP,
        a::QUICK_RAISE => weap_anim::QUICK_RAISE,
        a::EMPTY_DROP => weap_anim::EMPTY_DROP,
        a::EMPTY_RAISE => weap_anim::EMPTY_RAISE,
        a::SPRINT_IN | a::LOWREADY_IN => weap_anim::SPRINT_IN,
        a::SPRINT_LOOP | a::LOWREADY_LOOP => weap_anim::SPRINT_LOOP,
        a::SPRINT_OUT | a::LOWREADY_OUT => weap_anim::SPRINT_OUT,
        a::HOLD_FIRE => weap_anim::HOLD_FIRE,
        a::DETONATE => weap_anim::DETONATE,
        a::NIGHTVISION_WEAR => weap_anim::NIGHTVISION_WEAR,
        a::NIGHTVISION_REMOVE => weap_anim::NIGHTVISION_REMOVE,
        a::RELOAD_QUICK => x::RELOAD_QUICK,
        a::RELOAD_QUICK_EMPTY => x::RELOAD_QUICK_EMPTY,
        a::DTP_IN | a::DTP_LOOP => x::DIVE_IN,
        a::DTP_OUT => x::DIVE_OUT,
        a::DTP_EMPTY_IN | a::DTP_EMPTY_LOOP => x::DIVE_IN_EMPTY,
        a::DTP_EMPTY_OUT => x::DIVE_OUT_EMPTY,
        a::SPRINT_EMPTY_IN => x::SPRINT_IN_EMPTY,
        a::SPRINT_EMPTY_LOOP => x::SPRINT_LOOP_EMPTY,
        a::SPRINT_EMPTY_OUT => x::SPRINT_OUT_EMPTY,
        a::CRAWL_FORWARD => x::CRAWL_FORWARD,
        a::CRAWL_BACK => x::CRAWL_BACK,
        a::CRAWL_RIGHT => x::CRAWL_RIGHT,
        a::CRAWL_LEFT => x::CRAWL_LEFT,
        a::CRAWL_OUT => x::CRAWL_OUT,
        a::CRAWL_EMPTY_FORWARD => x::CRAWL_FORWARD_EMPTY,
        a::CRAWL_EMPTY_BACK => x::CRAWL_BACK_EMPTY,
        a::CRAWL_EMPTY_RIGHT => x::CRAWL_RIGHT_EMPTY,
        a::CRAWL_EMPTY_LEFT => x::CRAWL_LEFT_EMPTY,
        a::CRAWL_EMPTY_OUT => x::CRAWL_OUT_EMPTY,
        a::MELEE_ATTACK_EMPTY => x::MELEE_EMPTY,
        a::MELEE_CHARGE_EMPTY => x::MELEE_CHARGE_EMPTY,
        _ => return VmAnim::Idle,
    };
    VmAnim::Slot(s)
}

/// Whether `anim` is a shot the local client can show before the server confirms it.
pub fn is_fire_anim(anim: i32) -> bool {
    matches!(
        vm_anim(anim),
        VmAnim::Slot(
            weap_anim::FIRE | weap_anim::LASTSHOT | weap_anim::ADS_FIRE | weap_anim::ADS_LASTSHOT
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anims_pick_their_slots() {
        assert_eq!(vm_anim(a::IDLE), VmAnim::Idle);
        assert_eq!(vm_anim(a::SLIDE_IN), VmAnim::Idle);
        assert_eq!(vm_anim(a::FORCE_IDLE | a::TOGGLE), VmAnim::ForceIdle);
        assert_eq!(vm_anim(a::RAISE | a::TOGGLE), VmAnim::Slot(weap_anim::RAISE));
        assert_eq!(vm_anim(a::CONT_FIRE_LOOP), VmAnim::Slot(weap_anim::FIRE));
        assert_eq!(vm_anim(a::CRAWL_EMPTY_OUT), VmAnim::Slot(weap_anim_extra::CRAWL_OUT_EMPTY));
        assert!(is_fire_anim(a::ADS_ATTACK_LASTSHOT));
        assert!(!is_fire_anim(a::RECHAMBER));
    }

    #[test]
    fn timed_slots_follow_the_gun() {
        assert!(slot_uses_native_rate(weap_anim::IDLE));
        assert!(!slot_uses_native_rate(weap_anim::RELOAD));
        assert!(!slot_uses_native_rate(weap_anim_extra::CRAWL_IN));
        assert!(slot_uses_native_rate(weap_anim_extra::RELOAD_QUICK));
    }
}
