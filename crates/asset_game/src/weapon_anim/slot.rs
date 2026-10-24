use weapon_t6::asset_slots::{WEAPON_ANIM_SLOTS, weap_anim_extra as extra, weap_anim_slot as base};

/// Declares the slot enum and the list of every slot in one place, so the
/// two cannot drift apart.
macro_rules! anim_slots {
    ($($name:ident = $node:expr,)+) => {
        /// The animations a gun can play. A slot's value is the node its clip
        /// sits at in the clip tree.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum WeaponAnimSlot {
            $($name = $node as u8,)+
        }

        const EVERY_SLOT: &[WeaponAnimSlot] = &[$(WeaponAnimSlot::$name,)+];
    };
}

anim_slots! {
    Idle = base::IDLE,
    EmptyIdle = base::EMPTY_IDLE,
    Fire = base::FIRE,
    HoldFire = base::HOLD_FIRE,
    LastShot = base::LASTSHOT,
    Rechamber = base::RECHAMBER,
    Melee = base::MELEE,
    MeleeCharge = base::MELEE_CHARGE,
    Reload = base::RELOAD,
    ReloadEmpty = base::RELOAD_EMPTY,
    ReloadStart = base::RELOAD_START,
    ReloadEnd = base::RELOAD_END,
    ReloadQuick = extra::RELOAD_QUICK,
    ReloadQuickEmpty = extra::RELOAD_QUICK_EMPTY,
    Raise = base::RAISE,
    FirstRaise = base::FIRST_RAISE,
    Drop = base::DROP,
    AltRaise = base::ALT_RAISE,
    AltDrop = base::ALT_DROP,
    QuickRaise = base::QUICK_RAISE,
    QuickDrop = base::QUICK_DROP,
    EmptyRaise = base::EMPTY_RAISE,
    EmptyDrop = base::EMPTY_DROP,
    SprintIn = base::SPRINT_IN,
    SprintLoop = base::SPRINT_LOOP,
    SprintOut = base::SPRINT_OUT,
    AdsFire = base::ADS_FIRE,
    AdsLastShot = base::ADS_LASTSHOT,
    AdsRechamber = base::ADS_RECHAMBER,
    AdsUp = base::ADS_UP,
    AdsDown = base::ADS_DOWN,
    DiveIn = extra::DIVE_IN,
    DiveLoop = extra::DIVE_LOOP,
    DiveOut = extra::DIVE_OUT,
    DiveInEmpty = extra::DIVE_IN_EMPTY,
    DiveLoopEmpty = extra::DIVE_LOOP_EMPTY,
    DiveOutEmpty = extra::DIVE_OUT_EMPTY,
    SprintInEmpty = extra::SPRINT_IN_EMPTY,
    SprintLoopEmpty = extra::SPRINT_LOOP_EMPTY,
    SprintOutEmpty = extra::SPRINT_OUT_EMPTY,
    CrawlIn = extra::CRAWL_IN,
    CrawlForward = extra::CRAWL_FORWARD,
    CrawlBack = extra::CRAWL_BACK,
    CrawlRight = extra::CRAWL_RIGHT,
    CrawlLeft = extra::CRAWL_LEFT,
    CrawlOut = extra::CRAWL_OUT,
    CrawlInEmpty = extra::CRAWL_IN_EMPTY,
    CrawlForwardEmpty = extra::CRAWL_FORWARD_EMPTY,
    CrawlBackEmpty = extra::CRAWL_BACK_EMPTY,
    CrawlRightEmpty = extra::CRAWL_RIGHT_EMPTY,
    CrawlLeftEmpty = extra::CRAWL_LEFT_EMPTY,
    CrawlOutEmpty = extra::CRAWL_OUT_EMPTY,
    CameraMantle = extra::CAMERA_MANTLE,
    MeleeEmpty = extra::MELEE_EMPTY,
    MeleeChargeEmpty = extra::MELEE_CHARGE_EMPTY,
    FireIntro = extra::FIRE_INTRO,
    AdsFireIntro = extra::ADS_FIRE_INTRO,
}

/// Slot by node, filled once at compile time.
const BY_NODE: [Option<WeaponAnimSlot>; WEAPON_ANIM_SLOTS] = {
    let mut table = [None; WEAPON_ANIM_SLOTS];
    let mut at = 0;
    while at < EVERY_SLOT.len() {
        table[EVERY_SLOT[at] as usize] = Some(EVERY_SLOT[at]);
        at += 1;
    }
    table
};

impl WeaponAnimSlot {
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
        for &slot in EVERY_SLOT {
            assert_eq!(WeaponAnimSlot::from_index(slot.index()), Some(slot));
        }
    }

    #[test]
    fn no_two_slots_share_a_node() {
        let filled = BY_NODE.iter().flatten().count();
        assert_eq!(filled, EVERY_SLOT.len());
    }

    #[test]
    fn nodes_without_an_animation_have_no_slot() {
        assert_eq!(WeaponAnimSlot::from_index(base::ROOT), None);
        assert_eq!(WeaponAnimSlot::from_index(base::STUNNED_LOOP), None);
        assert_eq!(WeaponAnimSlot::from_index(WEAPON_ANIM_SLOTS), None);
    }

    #[test]
    fn extra_slots_sit_after_the_authored_ones() {
        assert_eq!(WeaponAnimSlot::ReloadQuick.index(), extra::RELOAD_QUICK);
        assert!(WeaponAnimSlot::AdsDown.index() < WeaponAnimSlot::ReloadQuick.index());
    }
}
