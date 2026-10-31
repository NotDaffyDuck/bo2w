use weapon_t6::asset_slots::{WEAPON_ANIM_SLOTS, weap_anim_extra, weap_anim_slot};

/// Declares the slot enum and the list of every slot in one place, so the
/// two cannot drift apart.
macro_rules! anim_slots {
    ($($name:ident = $node:expr,)+) => {
        /// The animations a gun can play. A slot's value is the node its clip
        /// sits at in the clip tree.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum GunMoveRole {
            $($name = $node as u8,)+
        }

        const EVERY_SLOT: &[GunMoveRole] = &[$(GunMoveRole::$name,)+];
    };
}

anim_slots! {
    Idle = weap_anim_slot::IDLE,
    EmptyIdle = weap_anim_slot::DRY_REST_CLIP,
    Fire = weap_anim_slot::TRIGGER_SLOT,
    HoldFire = weap_anim_slot::HOLD_FIRE,
    LastShot = weap_anim_slot::FINAL_SHOT_CLIP,
    Rechamber = weap_anim_slot::RECHAMBER,
    Melee = weap_anim_slot::CLOSE_COMBAT,
    MeleeCharge = weap_anim_slot::MELEE_CHARGE,
    Reload = weap_anim_slot::RELOAD,
    DryReload = weap_anim_slot::RELOAD_EMPTY,
    ReloadKickoff = weap_anim_slot::RELOAD_START,
    ReloadEnd = weap_anim_slot::RELOAD_END,
    ReloadQuick = weap_anim_extra::RELOAD_QUICK,
    ReloadQuickEmpty = weap_anim_extra::RELOAD_QUICK_EMPTY,
    Raise = weap_anim_slot::RAISE,
    FirstRaise = weap_anim_slot::FIRST_RAISE,
    Drop = weap_anim_slot::DROP,
    AltRaise = weap_anim_slot::ATTACHMENT_LIFT_CLIP,
    AltDrop = weap_anim_slot::ATTACHMENT_LOWER_CLIP,
    QuickRaise = weap_anim_slot::QUICK_RAISE,
    QuickDrop = weap_anim_slot::QUICK_DROP,
    EmptyRaise = weap_anim_slot::EMPTY_RAISE,
    EmptyDrop = weap_anim_slot::EMPTY_DROP,
    SprintIn = weap_anim_slot::SPRINT_IN,
    SprintLoop = weap_anim_slot::SPRINT_LOOP,
    SprintOut = weap_anim_slot::SPRINT_OUT,
    AdsFire = weap_anim_slot::SCOPE_SHOT_CLIP,
    AdsLastShot = weap_anim_slot::SCOPE_FINAL_SHOT_CLIP,
    AdsRechamber = weap_anim_slot::ADS_RECHAMBER,
    AdsUp = weap_anim_slot::SCOPE_RAISE_CLIP,
    AdsDown = weap_anim_slot::SCOPE_LOWER_CLIP,
    DiveIn = weap_anim_extra::DIVE_IN,
    DiveLoop = weap_anim_extra::DIVE_LOOP,
    DiveOut = weap_anim_extra::DIVE_OUT,
    DiveInEmpty = weap_anim_extra::DIVE_IN_EMPTY,
    DiveLoopEmpty = weap_anim_extra::DIVE_LOOP_EMPTY,
    DiveOutEmpty = weap_anim_extra::DIVE_OUT_EMPTY,
    SprintInEmpty = weap_anim_extra::SPRINT_IN_EMPTY,
    SprintLoopEmpty = weap_anim_extra::SPRINT_LOOP_EMPTY,
    SprintOutEmpty = weap_anim_extra::SPRINT_OUT_EMPTY,
    CrawlIn = weap_anim_extra::CRAWL_IN,
    CrawlForward = weap_anim_extra::CRAWL_FORWARD,
    CrawlBack = weap_anim_extra::CRAWL_BACK,
    CrawlRight = weap_anim_extra::CRAWL_RIGHT,
    CrawlLeft = weap_anim_extra::CRAWL_LEFT,
    CrawlOut = weap_anim_extra::CRAWL_OUT,
    CrawlInEmpty = weap_anim_extra::CRAWL_IN_EMPTY,
    CrawlForwardEmpty = weap_anim_extra::CRAWL_FORWARD_EMPTY,
    CrawlBackEmpty = weap_anim_extra::CRAWL_BACK_EMPTY,
    CrawlRightEmpty = weap_anim_extra::CRAWL_RIGHT_EMPTY,
    CrawlLeftEmpty = weap_anim_extra::CRAWL_LEFT_EMPTY,
    CrawlOutEmpty = weap_anim_extra::CRAWL_OUT_EMPTY,
    CameraMantle = weap_anim_extra::CAMERA_MANTLE,
    MeleeEmpty = weap_anim_extra::MELEE_EMPTY,
    MeleeChargeEmpty = weap_anim_extra::MELEE_CHARGE_EMPTY,
    FireIntro = weap_anim_extra::FIRE_INTRO,
    AdsFireIntro = weap_anim_extra::ADS_FIRE_INTRO,
}

/// Slot by node, filled once at compile time.
const BY_NODE: [Option<GunMoveRole>; WEAPON_ANIM_SLOTS] = {
    let mut table = [None; WEAPON_ANIM_SLOTS];
    let mut at = 0;
    while at < EVERY_SLOT.len() {
        table[EVERY_SLOT[at] as usize] = Some(EVERY_SLOT[at]);
        at += 1;
    }
    table
};

impl GunMoveRole {
    /// The node of the clip tree this slot plays on.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The slot a clip-tree node belongs to; nodes no gun animation uses
    /// (the root, the stun and night-vision nodes) have none.
    pub fn from_index(node: usize) -> Option<Self> {
        BY_NODE.get(node).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_is_found_at_its_own_node() {
        for &role in EVERY_SLOT {
            assert_eq!(GunMoveRole::from_index(role.index()), Some(role));
        }
    }

    #[test]
    fn no_two_slots_share_a_node() {
        let filled = BY_NODE.iter().flatten().count();
        assert_eq!(filled, EVERY_SLOT.len());
    }

    #[test]
    fn nodes_without_an_animation_have_no_slot() {
        assert_eq!(GunMoveRole::from_index(weap_anim_slot::ROOT), None);
        assert_eq!(GunMoveRole::from_index(weap_anim_slot::DAZED_REPEAT_CLIP), None);
        assert_eq!(GunMoveRole::from_index(WEAPON_ANIM_SLOTS), None);
    }

    #[test]
    fn extra_slots_sit_after_the_authored_ones() {
        assert_eq!(GunMoveRole::ReloadQuick.index(), weap_anim_extra::RELOAD_QUICK);
        assert!(GunMoveRole::AdsDown.index() < GunMoveRole::ReloadQuick.index());
    }
}
