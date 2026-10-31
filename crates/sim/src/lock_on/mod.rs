//! A lock-on weapon's targeting state.

use crate::MissileTarget;

/// The lock's `flags` word.
mod bit {
    pub const ACQUIRING: u8 = 1 << 0;
    pub const HELD: u8 = 1 << 1;
    /// The target is nearer than the weapon allows.
    pub const TOO_NEAR: u8 = 1 << 4;
    /// Blocks firing even with a held target.
    pub const NO_FIRE: u8 = TOO_NEAR | 1 << 5;
}

/// What a lock-on weapon is doing about its target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponLock {
    pub gun: u32,
    pub life_no: u32,
    pub bits: u8,
    pub target: [f32; 3],
    /// What the missile will steer at, once there is something.
    pub aim: Option<MissileTarget>,
    pub acquire_started_at: i32,
}

impl WeaponLock {
    fn has(self, bits: u8) -> bool {
        self.bits & bits != 0
    }

    /// Still acquiring, no target held yet.
    pub fn locking(self) -> bool {
        self.has(bit::ACQUIRING) && !self.has(bit::HELD)
    }

    /// A target is held.
    pub fn locked(self) -> bool {
        self.has(bit::HELD)
    }

    /// The target is too near for the weapon.
    pub fn too_close(self) -> bool {
        self.has(bit::TOO_NEAR)
    }

    /// Whether `gun` in `spawn_life` may fire on the held target right now.
    pub fn can_fire(self, gun: u32, spawn_life: u32) -> bool {
        if (self.gun, self.life_no) != (gun, spawn_life) {
            return false;
        }
        self.locked() && !self.has(bit::NO_FIRE) && self.aim.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(flags: u8) -> WeaponLock {
        WeaponLock { gun: 7, life_no: 2, bits: flags, ..WeaponLock::default() }
    }

    #[test]
    fn acquiring_turns_into_locked() {
        assert!(lock(1).locking() && !lock(1).locked());
        assert!(lock(3).locked() && !lock(3).locking());
        assert!(!lock(0).locking() && !lock(0).locked());
    }

    #[test]
    fn firing_needs_the_same_weapon_life_and_an_aim() {
        let mut held = lock(2);
        assert!(!held.can_fire(7, 2), "nothing to steer at yet");
        held.aim = Some(MissileTarget::Point([0.0; 3]));
        assert!(held.can_fire(7, 2));
        assert!(!held.can_fire(8, 2));
        assert!(!held.can_fire(7, 3));
    }

    #[test]
    fn too_near_and_the_other_blocker_stop_the_shot() {
        for blocker in [16, 32] {
            let mut held = lock(2 | blocker);
            held.aim = Some(MissileTarget::Point([0.0; 3]));
            assert!(!held.can_fire(7, 2));
        }
        assert!(lock(16).too_close() && !lock(32).too_close());
    }
}
