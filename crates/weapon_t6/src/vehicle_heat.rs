//! BO2's heat on a vehicle's own guns (VEH_UpdateDriverWeapons for its
//! turret, VEH_UpdateGunnerWeapon for a gunner's gun). A gun that overheats
//! heats while its trigger is held, locks at 100 when full, and fires again
//! only once it has cooled to its end value. In multiplayer the VTOL
//! Warship's rockets are the one such gun: their heat is their reload.

use crate::pm::fire::FireFacts;

/// One vehicle gun's heat (BO2's `heatVal` and `overheating`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunHeat {
    /// 0 to 100.
    pub heat: f32,
    pub overheating: bool,
}

/// What one frame of heat changed. A gunner's gun tells its scripts
/// (`gunner_turret_overheat`, `gunner_turret_stop_overheat`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeatChange {
    #[default]
    None,
    /// It reached 100 and locked.
    Overheated,
    /// It cooled down to its end value.
    Cooled,
}

impl GunHeat {
    /// One server frame of `msec` with its trigger `held` by the player at
    /// the gun. A gun that does not overheat is left alone.
    pub fn step(&mut self, facts: &FireFacts, held: bool, msec: i32) -> HeatChange {
        if !facts.overheat {
            return HeatChange::None;
        }
        let secs = msec as f32 * 0.001;
        let mut change = HeatChange::None;
        // It cools unless its trigger is held on a gun that does not cool
        // while firing, and always once it has overheated.
        if !held || facts.cool_while_firing || self.overheating {
            let before = self.heat;
            self.heat = (self.heat - secs * facts.cooldown_rate).max(0.0);
            if self.heat <= facts.overheat_end_val {
                self.overheating = false;
                if before > facts.overheat_end_val {
                    change = HeatChange::Cooled;
                }
            }
        }
        // A held trigger heats it, whether or not a shot leaves this frame.
        if held && !self.overheating {
            self.heat += secs * facts.overheat_rate;
            if self.heat >= 100.0 {
                self.heat = 100.0;
                self.overheating = true;
                change = HeatChange::Overheated;
            }
        }
        change
    }

    /// BO2 fires a vehicle gun only while it is not overheated.
    pub fn can_fire(&self) -> bool {
        !self.overheating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The VTOL Warship's rockets, as its weapon file has them.
    fn rockets() -> FireFacts {
        FireFacts {
            overheat: true,
            overheat_rate: 150.0,
            cooldown_rate: 17.0,
            overheat_end_val: 0.0,
            ..FireFacts::default()
        }
    }

    #[test]
    fn rockets_lock_after_two_thirds_of_a_second_and_reload_in_six() {
        let f = rockets();
        let mut h = GunHeat::default();
        // 50 ms frames: 7.5 a frame, full on the 14th.
        for _ in 0..13 {
            assert_eq!(h.step(&f, true, 50), HeatChange::None);
            assert!(h.can_fire());
        }
        assert_eq!(h.step(&f, true, 50), HeatChange::Overheated);
        assert_eq!(h.heat, 100.0);
        assert!(!h.can_fire());
        // Holding the trigger does not stop it cooling once locked: 0.85 a
        // frame, 118 frames (5.9 s) to empty.
        let mut frames = 0;
        loop {
            frames += 1;
            let change = h.step(&f, true, 50);
            if change == HeatChange::Cooled {
                break;
            }
            assert!(!h.can_fire(), "frame {frames}");
            assert!(frames < 200);
        }
        assert_eq!(frames, 118);
        // Still held, it fires and heats again that same frame.
        assert!(h.can_fire());
        assert!((h.heat - 7.5).abs() < 1e-3);
    }

    #[test]
    fn letting_go_cools_it_and_a_gun_without_heat_never_locks() {
        let f = rockets();
        let mut h = GunHeat::default();
        for _ in 0..4 {
            h.step(&f, true, 50);
        }
        assert!((h.heat - 30.0).abs() < 1e-3);
        // Let go: it cools, and tells when it is empty.
        assert_eq!(h.step(&f, false, 1000), HeatChange::None);
        assert!((h.heat - 13.0).abs() < 1e-3);
        assert_eq!(h.step(&f, false, 1000), HeatChange::Cooled);
        assert_eq!(h.step(&f, false, 1000), HeatChange::None);
        // A gun that cools while firing never gains while it cools faster.
        let mut cool = rockets();
        cool.cool_while_firing = true;
        cool.cooldown_rate = 200.0;
        let mut h = GunHeat::default();
        for _ in 0..100 {
            h.step(&cool, true, 50);
        }
        assert!(h.can_fire());
        // No overheat: nothing happens.
        let plain = FireFacts::default();
        let mut h = GunHeat::default();
        for _ in 0..100 {
            assert_eq!(h.step(&plain, true, 50), HeatChange::None);
        }
        assert_eq!(h, GunHeat::default());
    }
}
