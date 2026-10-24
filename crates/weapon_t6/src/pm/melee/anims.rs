//! Which first-person animation a melee anim number plays, and the time it is stretched to.

use super::MeleeFacts;
use crate::anim;

/// The first-person animation slots melee plays (`weapAnimFiles_t`).
pub mod file {
    pub const MELEE: usize = 9;
    pub const MELEE1: usize = 10;
    pub const MELEE2: usize = 11;
    pub const MELEE3: usize = 12;
    pub const MELEE_EMPTY: usize = 13;
    pub const MELEE_CHARGE: usize = 14;
    pub const MELEE_CHARGE_EMPTY: usize = 15;
    pub const QUICK_RAISE: usize = 28;
}

/// Which slots of a weapon hold an animation (`CoreFacts::anim_slots`), one bit per `file::*` index.
pub fn has(slots: u128, file: usize) -> bool {
    file < 128 && slots & (1 << file) != 0
}

/// The slot a melee anim plays. A normal swing picks evenly among MELEE and MELEE1-3,
/// counting up to the first missing one; `rand(n)` returns 0..n. The empty and lunge anims
/// fall back to the full ones when the weapon has no empty version.
pub fn pick_anim_file(slots: u128, weap_anim: i32, rand: &mut impl FnMut(usize) -> usize) -> usize {
    let mut swing = || {
        let n = 1 + [file::MELEE1, file::MELEE2, file::MELEE3]
            .iter()
            .take_while(|&&f| has(slots, f))
            .count();
        file::MELEE + rand(n)
    };
    match anim::number(weap_anim) {
        anim::MELEE_ATTACK_EMPTY if has(slots, file::MELEE_EMPTY) => file::MELEE_EMPTY,
        anim::MELEE_ATTACK | anim::MELEE_ATTACK_EMPTY => swing(),
        anim::MELEE_CHARGE_EMPTY if has(slots, file::MELEE_CHARGE_EMPTY) => file::MELEE_CHARGE_EMPTY,
        anim::MELEE_CHARGE | anim::MELEE_CHARGE_EMPTY => file::MELEE_CHARGE,
        anim::QUICK_RAISE => file::QUICK_RAISE,
        _ => file::MELEE,
    }
}

/// The time a melee animation is stretched to fit (ms).
pub fn anim_fit_ms(m: &MeleeFacts, slot: usize, quick_raise_time: i32) -> i32 {
    match slot {
        file::MELEE..=file::MELEE_EMPTY => m.time,
        file::MELEE_CHARGE | file::MELEE_CHARGE_EMPTY => m.charge_time,
        file::QUICK_RAISE => quick_raise_time,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(files: &[usize]) -> u128 {
        files.iter().fold(0, |b, &f| b | 1 << f)
    }

    #[test]
    fn empty_gun_and_lunge_pick_their_anims() {
        let knife = bits(&[file::MELEE, file::MELEE_CHARGE]);
        let empty = anim::start(0, anim::MELEE_ATTACK_EMPTY);
        // No MELEE_EMPTY in the weapon: the full swing plays.
        assert_eq!(pick_anim_file(knife, empty, &mut |_| 0), file::MELEE);
        let with_empty = knife | bits(&[file::MELEE_EMPTY]);
        assert_eq!(pick_anim_file(with_empty, empty, &mut |_| 0), file::MELEE_EMPTY);
        let charge_empty = anim::start(0, anim::MELEE_CHARGE_EMPTY);
        assert_eq!(pick_anim_file(knife, charge_empty, &mut |_| 0), file::MELEE_CHARGE);
        let m = MeleeFacts { time: 800, charge_time: 1160, ..MeleeFacts::default() };
        assert_eq!(anim_fit_ms(&m, file::MELEE_EMPTY, 400), 800);
        assert_eq!(anim_fit_ms(&m, file::MELEE_CHARGE, 400), 1160);
        assert_eq!(anim_fit_ms(&m, file::QUICK_RAISE, 400), 400);
    }

    #[test]
    fn swing_variants_count_up_to_the_first_missing() {
        // MELEE2 missing: MELEE3 never plays.
        let slots = bits(&[file::MELEE, file::MELEE1, file::MELEE3]);
        let mut seen = std::collections::BTreeSet::new();
        for r in 0..8 {
            seen.insert(pick_anim_file(slots, anim::MELEE_ATTACK, &mut |n| r % n));
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![file::MELEE, file::MELEE1]);
    }
}
