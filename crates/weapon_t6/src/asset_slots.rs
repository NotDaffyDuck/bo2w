//! The viewmodel animation slots the weapon catalog stores a gun's anims in, and how a BO2
//! viewmodel anim (`crate::anim`) picks one. The old viewmodel player (render_anim
//! `viewmodel_controller`) plays these; BO2's own player is `crate::viewmodel`.
//!
//! TODO(fact): the layout is the catalog's, not BO2's: 37 slots plus 28 more for anims BO2
//! has on top. BO2 keeps its anims in `crate::viewmodel::slot`; this goes when the catalog
//! stores those.

use crate::anim;

/// Declares `pub const` slot indices numbered in the order listed, the first
/// taking `start`. The order is the catalog layout; never reorder or insert
/// in the middle.
macro_rules! numbered_slots {
    (start = $start:expr; $($(#[$doc:meta])* $name:ident),+ $(,)?) => {
        numbered_slots!(@next $start; $($(#[$doc])* $name),+);
    };
    (@next $at:expr; $(#[$doc:meta])* $name:ident $(, $(#[$rest_doc:meta])* $rest:ident)*) => {
        $(#[$doc])*
        pub const $name: usize = $at;
        numbered_slots!(@next $at + 1; $($(#[$rest_doc])* $rest),*);
    };
    (@next $at:expr;) => {};
}

/// How many slots the base layout has; [`weap_anim_extra`] continues after it.
pub const GUN_MOTION_TOTAL: usize = 37;
/// Base slots plus the extra slots for animations added on top.
pub const WEAPON_ANIM_SLOTS: usize = GUN_MOTION_TOTAL + 28;

/// The base slot layout.
pub mod weap_anim_slot {
    numbered_slots! {
        start = 0;
        ROOT, IDLE, DRY_REST_CLIP, TRIGGER_SLOT, HOLD_FIRE, FINAL_SHOT_CLIP, RECHAMBER,
        CLOSE_COMBAT, MELEE_CHARGE,
        RELOAD, RELOAD_EMPTY, RELOAD_START, RELOAD_END,
        RAISE, FIRST_RAISE, BREACHER_LIFT_CLIP, DROP,
        ATTACHMENT_LIFT_CLIP, ATTACHMENT_LOWER_CLIP,
        QUICK_RAISE, QUICK_DROP, EMPTY_RAISE, EMPTY_DROP,
        SPRINT_IN, SPRINT_LOOP, SPRINT_OUT,
        DAZED_BEGIN_CLIP, DAZED_REPEAT_CLIP, DAZED_FINISH_CLIP,
        DETONATE, NIGHTVISION_WEAR, NIGHTVISION_REMOVE,
        SCOPE_SHOT_CLIP, SCOPE_FINAL_SHOT_CLIP, ADS_RECHAMBER, SCOPE_RAISE_CLIP, SCOPE_LOWER_CLIP,
    }
}

/// Slots past the base layout.
pub mod weap_anim_extra {
    numbered_slots! {
        start = super::GUN_MOTION_TOTAL;
        RELOAD_QUICK, RELOAD_QUICK_EMPTY,
        /// Dive-to-prone: in, loop, out, then the same three with an empty magazine.
        DIVE_IN, DIVE_LOOP, DIVE_OUT, DIVE_IN_EMPTY, DIVE_LOOP_EMPTY, DIVE_OUT_EMPTY,
        /// Sprint in, loop and out with an empty magazine.
        SPRINT_IN_EMPTY, SPRINT_LOOP_EMPTY, SPRINT_OUT_EMPTY,
        /// Prone crawl: in, the four looping directions, out; then the empty-magazine set.
        CRAWL_IN, CRAWL_FORWARD, CRAWL_BACK, CRAWL_RIGHT, CRAWL_LEFT, CRAWL_OUT,
        CRAWL_IN_EMPTY, CRAWL_FORWARD_EMPTY, CRAWL_BACK_EMPTY, CRAWL_RIGHT_EMPTY,
        CRAWL_LEFT_EMPTY, CRAWL_OUT_EMPTY,
        /// Camera-only climb motion, layered over a low climb with the gun kept in hand.
        CAMERA_MANTLE,
        /// Melee and charged melee with an empty magazine (slide locked back).
        MELEE_EMPTY, MELEE_CHARGE_EMPTY,
        /// First shots of a trigger pull, hip and aimed.
        FIRE_INTRO, ADS_FIRE_INTRO,
    }
}

// Each list must fill its range exactly.
const _: () = assert!(weap_anim_slot::SCOPE_LOWER_CLIP + 1 == GUN_MOTION_TOTAL);
const _: () = assert!(weap_anim_extra::ADS_FIRE_INTRO + 1 == WEAPON_ANIM_SLOTS);

pub const ACTION_BLEND_SECONDS: f32 = 0.0;
pub const REST_BREAK_BLEND_SECONDS: f32 = 0.5;
pub const LIVE_BLEND_TARGET: f32 = 1.0;
pub const DORMANT_BLEND_TARGET: f32 = 0.0;

/// Base slots whose clip is stretched over the gun's own time for the action
/// (fire, reload, raise, drop and the rest); the others play at the clip's rate.
const TIMED_SLOTS: [usize; 25] = [
    4, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 31,
];

/// How fast a clip plays so it lasts the action's `gun_action_millis`.
pub fn play_speed(anim_duration_millis: i32, gun_action_millis: i32) -> f32 {
    if gun_action_millis < 0 {
        return 1.0;
    }
    if gun_action_millis == 0 {
        return 0.0;
    }
    if anim_duration_millis <= 0 {
        return 1.0;
    }
    anim_duration_millis as f32 / gun_action_millis as f32
}

/// Whether `slot` plays at its clip's own rate rather than over the gun's time.
pub fn clip_keeps_authored_rate(slot: usize) -> bool {
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
    let s = match anim::number(anim) {
        anim::FORCE_IDLE => return VmAnim::ForceIdle,
        anim::ATTACK | anim::ATTACK_LEFT | anim::CONT_FIRE_IN | anim::CONT_FIRE_LOOP | anim::CONT_FIRE_OUT => {
            weap_anim_slot::TRIGGER_SLOT
        }
        anim::ATTACK_LASTSHOT | anim::ATTACK_LASTSHOT_LEFT => weap_anim_slot::FINAL_SHOT_CLIP,
        anim::BOLT_CYCLE => weap_anim_slot::RECHAMBER,
        anim::ADS_ATTACK => weap_anim_slot::SCOPE_SHOT_CLIP,
        anim::ADS_ATTACK_LASTSHOT => weap_anim_slot::SCOPE_FINAL_SHOT_CLIP,
        anim::ADS_BOLT_CYCLE => weap_anim_slot::ADS_RECHAMBER,
        anim::MELEE_ATTACK => weap_anim_slot::CLOSE_COMBAT,
        anim::MELEE_CHARGE => weap_anim_slot::MELEE_CHARGE,
        anim::PUT_AWAY | anim::BREAKING_DOWN => weap_anim_slot::DROP,
        anim::RAISE | anim::DEPLOYING => weap_anim_slot::RAISE,
        anim::FIRST_TAKE_OUT => weap_anim_slot::FIRST_RAISE,
        anim::RELOAD => weap_anim_slot::RELOAD,
        anim::RELOAD_DRY => weap_anim_slot::RELOAD_EMPTY,
        anim::RELOAD_BEGIN => weap_anim_slot::RELOAD_START,
        anim::RELOAD_FINISH => weap_anim_slot::RELOAD_END,
        anim::ALTSWITCHFROM => weap_anim_slot::ATTACHMENT_LOWER_CLIP,
        anim::ALTSWITCHTO => weap_anim_slot::ATTACHMENT_LIFT_CLIP,
        anim::FAST_PUT_AWAY => weap_anim_slot::QUICK_DROP,
        anim::FAST_TAKE_OUT => weap_anim_slot::QUICK_RAISE,
        anim::EMPTY_PUT_AWAY => weap_anim_slot::EMPTY_DROP,
        anim::EMPTY_TAKE_OUT => weap_anim_slot::EMPTY_RAISE,
        anim::SPRINT_ENTER | anim::LOWREADY_IN => weap_anim_slot::SPRINT_IN,
        anim::SPRINT_HOLD | anim::LOWREADY_LOOP => weap_anim_slot::SPRINT_LOOP,
        anim::SPRINT_EXIT | anim::LOWREADY_OUT => weap_anim_slot::SPRINT_OUT,
        anim::FIRE_HELD => weap_anim_slot::HOLD_FIRE,
        anim::TRIGGER_DETONATOR => weap_anim_slot::DETONATE,
        anim::NVG_ON => weap_anim_slot::NIGHTVISION_WEAR,
        anim::NVG_OFF => weap_anim_slot::NIGHTVISION_REMOVE,
        anim::RELOAD_QUICK => weap_anim_extra::RELOAD_QUICK,
        anim::RELOAD_QUICK_EMPTY => weap_anim_extra::RELOAD_QUICK_EMPTY,
        anim::DTP_IN | anim::DTP_LOOP => weap_anim_extra::DIVE_IN,
        anim::DTP_OUT => weap_anim_extra::DIVE_OUT,
        anim::DTP_EMPTY_IN | anim::DTP_EMPTY_LOOP => weap_anim_extra::DIVE_IN_EMPTY,
        anim::DTP_EMPTY_OUT => weap_anim_extra::DIVE_OUT_EMPTY,
        anim::SPRINT_EMPTY_IN => weap_anim_extra::SPRINT_IN_EMPTY,
        anim::SPRINT_EMPTY_LOOP => weap_anim_extra::SPRINT_LOOP_EMPTY,
        anim::SPRINT_EMPTY_OUT => weap_anim_extra::SPRINT_OUT_EMPTY,
        anim::CRAWL_FORWARD => weap_anim_extra::CRAWL_FORWARD,
        anim::CRAWL_BACK => weap_anim_extra::CRAWL_BACK,
        anim::CRAWL_RIGHT => weap_anim_extra::CRAWL_RIGHT,
        anim::CRAWL_LEFT => weap_anim_extra::CRAWL_LEFT,
        anim::CRAWL_OUT => weap_anim_extra::CRAWL_OUT,
        anim::CRAWL_EMPTY_FORWARD => weap_anim_extra::CRAWL_FORWARD_EMPTY,
        anim::CRAWL_EMPTY_BACK => weap_anim_extra::CRAWL_BACK_EMPTY,
        anim::CRAWL_EMPTY_RIGHT => weap_anim_extra::CRAWL_RIGHT_EMPTY,
        anim::CRAWL_EMPTY_LEFT => weap_anim_extra::CRAWL_LEFT_EMPTY,
        anim::CRAWL_EMPTY_OUT => weap_anim_extra::CRAWL_OUT_EMPTY,
        anim::MELEE_ATTACK_EMPTY => weap_anim_extra::MELEE_EMPTY,
        anim::MELEE_CHARGE_EMPTY => weap_anim_extra::MELEE_CHARGE_EMPTY,
        _ => return VmAnim::Idle,
    };
    VmAnim::Slot(s)
}

/// Whether `anim` is a shot the local client can show before the server confirms it.
pub fn is_fire_anim(anim: i32) -> bool {
    matches!(
        vm_anim(anim),
        VmAnim::Slot(
            weap_anim_slot::TRIGGER_SLOT | weap_anim_slot::FINAL_SHOT_CLIP | weap_anim_slot::SCOPE_SHOT_CLIP | weap_anim_slot::SCOPE_FINAL_SHOT_CLIP
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anims_pick_their_slots() {
        assert_eq!(vm_anim(anim::REST), VmAnim::Idle);
        assert_eq!(vm_anim(anim::SLIDE_IN), VmAnim::Idle);
        assert_eq!(vm_anim(anim::FORCE_IDLE | anim::TOGGLE), VmAnim::ForceIdle);
        assert_eq!(vm_anim(anim::RAISE | anim::TOGGLE), VmAnim::Slot(weap_anim_slot::RAISE));
        assert_eq!(vm_anim(anim::CONT_FIRE_LOOP), VmAnim::Slot(weap_anim_slot::TRIGGER_SLOT));
        assert_eq!(vm_anim(anim::CRAWL_EMPTY_OUT), VmAnim::Slot(weap_anim_extra::CRAWL_OUT_EMPTY));
        assert!(is_fire_anim(anim::ADS_ATTACK_LASTSHOT));
        assert!(!is_fire_anim(anim::BOLT_CYCLE));
    }

    #[test]
    fn layout_numbers_are_fixed() {
        assert_eq!([weap_anim_slot::ROOT, weap_anim_slot::RELOAD, weap_anim_slot::DROP], [0, 9, 16]);
        assert_eq!((weap_anim_slot::ADS_RECHAMBER, weap_anim_slot::SCOPE_LOWER_CLIP), (34, 36));
        assert_eq!((weap_anim_extra::RELOAD_QUICK, weap_anim_extra::DIVE_IN), (37, 39));
        assert_eq!((weap_anim_extra::CAMERA_MANTLE, weap_anim_extra::ADS_FIRE_INTRO), (60, 64));
    }

    #[test]
    fn timed_slots_follow_the_gun() {
        assert!(clip_keeps_authored_rate(weap_anim_slot::IDLE));
        assert!(!clip_keeps_authored_rate(weap_anim_slot::RELOAD));
        assert!(!clip_keeps_authored_rate(weap_anim_extra::CRAWL_IN));
        assert!(clip_keeps_authored_rate(weap_anim_extra::RELOAD_QUICK));
    }
}
