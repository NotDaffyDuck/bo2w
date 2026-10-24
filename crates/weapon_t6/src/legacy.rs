//! Bridge for the old renderer and wire while weapon_iw4 still exists: maps a T6 weapon state
//! onto the nearest weapon_iw4 state number, so the viewmodel and third-person code that only
//! knows the old states keeps drawing something sensible. Window 15 deletes this with weapon_iw4.

use crate::state as t6;

/// weapon_iw4's state numbers (its `WeaponState` discriminants).
mod iw4 {
    pub const READY: i32 = 0;
    pub const RAISING: i32 = 1;
    pub const RAISING_ALTSWITCH: i32 = 2;
    pub const DROPPING: i32 = 3;
    pub const DROPPING_QUICK: i32 = 4;
    pub const DROPPING_ALTSWITCH: i32 = 5;
    pub const FIRING: i32 = 6;
    pub const RECHAMBERING: i32 = 7;
    pub const RELOADING: i32 = 8;
    pub const RELOADING_INTERRUPT: i32 = 9;
    pub const RELOAD_START: i32 = 0xA;
    pub const RELOAD_START_INTERRUPT: i32 = 0xB;
    pub const RELOAD_END: i32 = 0xC;
    pub const MELEE_INIT: i32 = 0xD;
    pub const MELEE_FIRE: i32 = 0xE;
    pub const MELEE_END: i32 = 0xF;
    pub const OFFHAND_INIT: i32 = 0x10;
    pub const OFFHAND_PREPARE: i32 = 0x11;
    pub const OFFHAND_HOLD: i32 = 0x12;
    pub const OFFHAND_START: i32 = 0x13;
    pub const OFFHAND: i32 = 0x14;
    pub const OFFHAND_END: i32 = 0x15;
    pub const DETONATING: i32 = 0x16;
    pub const SPRINT_IN: i32 = 0x17;
    pub const SPRINT_LOOP: i32 = 0x18;
    pub const SPRINT_OUT: i32 = 0x19;
    pub const NIGHTVISION_WEAR: i32 = 0x1D;
    pub const NIGHTVISION_REMOVE: i32 = 0x1E;
}

/// The weapon_iw4 state closest to T6 state `s`. States iw4 has no match for read as READY.
pub fn to_iw4_state(s: i32) -> i32 {
    match s {
        t6::READY => iw4::READY,
        t6::RAISING | t6::SWIM_OUT => iw4::RAISING,
        t6::RAISING_ALTSWITCH => iw4::RAISING_ALTSWITCH,
        t6::DROPPING | t6::SWIM_IN => iw4::DROPPING,
        t6::DROPPING_QUICK => iw4::DROPPING_QUICK,
        t6::DROPPING_ALTSWITCH => iw4::DROPPING_ALTSWITCH,
        t6::FIRING
        | t6::CONT_FIRE_IN
        | t6::CONT_FIRE_LOOP
        | t6::CONT_FIRE_OUT
        | t6::FIRING_LEFT
        | t6::FIRING_BOTH => iw4::FIRING,
        t6::RECHAMBER_START | t6::RECHAMBERING => iw4::RECHAMBERING,
        t6::RELOADING_RIGHT
        | t6::RELOADING_LEFT
        | t6::RELOADING
        | t6::RELOAD_QUICK
        | t6::RELOAD_QUICK_EMPTY => iw4::RELOADING,
        t6::RELOADING_INTERUPT => iw4::RELOADING_INTERRUPT,
        t6::RELOAD_START | t6::RELOAD_START_RECHAMBER => iw4::RELOAD_START,
        t6::RELOAD_START_INTERUPT => iw4::RELOAD_START_INTERRUPT,
        t6::RELOAD_END => iw4::RELOAD_END,
        t6::MELEE_INIT => iw4::MELEE_INIT,
        t6::MELEE_FIRE => iw4::MELEE_FIRE,
        t6::MELEE_END => iw4::MELEE_END,
        t6::OFFHAND_INIT => iw4::OFFHAND_INIT,
        t6::OFFHAND_PREPARE => iw4::OFFHAND_PREPARE,
        t6::OFFHAND_HOLD | t6::OFFHAND_HOLDUSE => iw4::OFFHAND_HOLD,
        t6::OFFHAND_START => iw4::OFFHAND_START,
        t6::OFFHAND => iw4::OFFHAND,
        t6::OFFHAND_END => iw4::OFFHAND_END,
        t6::OFFHAND_DETONATE | t6::DETONATING => iw4::DETONATING,
        t6::SPRINT_RAISE | t6::LOWREADY_RAISE => iw4::SPRINT_IN,
        t6::SPRINT_LOOP | t6::LOWREADY_LOOP => iw4::SPRINT_LOOP,
        t6::SPRINT_DROP | t6::LOWREADY_DROP => iw4::SPRINT_OUT,
        t6::NIGHTVISION_WEAR => iw4::NIGHTVISION_WEAR,
        t6::NIGHTVISION_REMOVE => iw4::NIGHTVISION_REMOVE,
        _ => iw4::READY,
    }
}

/// weapon_iw4's viewmodel anim event numbers (`weap_anim_event`).
mod iw4_anim {
    pub const RESTART: i32 = 0x200;
}

/// The weapon_iw4 viewmodel anim closest to T6 anim `a` (toggle bit kept as iw4's restart bit).
/// T6 anims iw4 has no match for read as idle.
pub fn to_iw4_anim(a: i32) -> i32 {
    use crate::anim as t;
    let n = match t::number(a) {
        t::IDLE | t::SLIDE_IN | t::MANTLE | t::JAM | t::CRAWL_IN | t::CRAWL_EMPTY_IN => 0,
        t::FORCE_IDLE => 1,
        t::ATTACK | t::ATTACK_LEFT | t::CONT_FIRE_IN | t::CONT_FIRE_LOOP | t::CONT_FIRE_OUT => 2,
        t::ATTACK_LASTSHOT | t::ATTACK_LASTSHOT_LEFT => 3,
        t::RECHAMBER => 4,
        t::ADS_ATTACK => 5,
        t::ADS_ATTACK_LASTSHOT => 6,
        t::ADS_RECHAMBER => 7,
        t::MELEE_ATTACK => 8,
        t::MELEE_CHARGE => 9,
        t::DROP | t::BREAKING_DOWN => 0xA,
        t::RAISE | t::DEPLOYING => 0xB,
        t::FIRST_RAISE => 0xC,
        t::RELOAD => 0xD,
        t::RELOAD_EMPTY => 0xE,
        t::RELOAD_START => 0xF,
        t::RELOAD_END => 0x10,
        t::ALTSWITCHFROM => 0x11,
        t::ALTSWITCHTO => 0x12,
        t::QUICK_DROP => 0x13,
        t::QUICK_RAISE => 0x14,
        t::EMPTY_DROP => 0x15,
        t::EMPTY_RAISE => 0x16,
        t::SPRINT_IN | t::LOWREADY_IN => 0x17,
        t::SPRINT_LOOP | t::LOWREADY_LOOP => 0x18,
        t::SPRINT_OUT | t::LOWREADY_OUT => 0x19,
        t::HOLD_FIRE => 0x1D,
        t::DETONATE => 0x1E,
        t::NIGHTVISION_WEAR => 0x1F,
        t::NIGHTVISION_REMOVE => 0x20,
        t::RELOAD_QUICK => 0x21,
        t::RELOAD_QUICK_EMPTY => 0x22,
        t::DTP_IN | t::DTP_LOOP => 0x23,
        t::DTP_OUT => 0x24,
        t::DTP_EMPTY_IN | t::DTP_EMPTY_LOOP => 0x25,
        t::DTP_EMPTY_OUT => 0x26,
        t::SPRINT_EMPTY_IN => 0x27,
        t::SPRINT_EMPTY_LOOP => 0x28,
        t::SPRINT_EMPTY_OUT => 0x29,
        t::CRAWL_FORWARD => 0x2A,
        t::CRAWL_BACK => 0x2B,
        t::CRAWL_RIGHT => 0x2C,
        t::CRAWL_LEFT => 0x2D,
        t::CRAWL_OUT => 0x2E,
        t::CRAWL_EMPTY_FORWARD => 0x2F,
        t::CRAWL_EMPTY_BACK => 0x30,
        t::CRAWL_EMPTY_RIGHT => 0x31,
        t::CRAWL_EMPTY_LEFT => 0x32,
        t::CRAWL_EMPTY_OUT => 0x33,
        t::MELEE_ATTACK_EMPTY => 0x34,
        t::MELEE_CHARGE_EMPTY => 0x35,
        _ => 0,
    };
    if a & t::TOGGLE != 0 { n | iw4_anim::RESTART } else { n }
}

#[cfg(test)]
mod tests {
    use super::{to_iw4_anim, to_iw4_state};
    use crate::state as t6;

    #[test]
    fn switch_states_keep_their_numbers() {
        for s in 0..=6 {
            assert_eq!(to_iw4_state(s), s);
        }
        assert_eq!(to_iw4_state(t6::SWIM_OUT), 1);
        assert_eq!(to_iw4_state(t6::CRAWL_OUT), 0);
        assert_eq!(to_iw4_state(t6::NIGHTVISION_REMOVE), 0x1E);
    }

    #[test]
    fn anims_map_with_the_restart_bit() {
        use crate::anim as a;
        assert_eq!(to_iw4_anim(a::RAISE), 0xB);
        assert_eq!(to_iw4_anim(a::DROP | a::TOGGLE), 0xA | 0x200);
        assert_eq!(to_iw4_anim(a::FIRST_RAISE), 0xC);
        assert_eq!(to_iw4_anim(a::CRAWL_EMPTY_OUT), 0x33);
    }
}
