//! A lock-on weapon's targeting state.

/// Bit set in `flags` while the weapon is still acquiring a target.
const ACQUIRING: u8 = 1;
/// Bit set in `flags` once a target is held.
const HELD: u8 = 2;
/// Bit set in `flags` when the target is nearer than the weapon allows.
const TOO_NEAR: u8 = 16;
/// Bits (16 and 32) that forbid firing even with a held target; 16 is `TOO_NEAR`.
const FIRE_BLOCKERS: u8 = 48;

/// What a lock-on weapon is doing about its target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponLock {
    pub weapon: u32,
    pub life: u32,
    pub flags: u8,
    pub target: [f32; 3],
    /// The entity the missile will steer to, once there is one.
    pub aim: Option<crate::MissileTarget>,
    pub acquire_started_at: i32,
}

impl WeaponLock {
    /// Acquiring but not yet holding a target.
    pub fn locking(self) -> bool {
        self.flags & (ACQUIRING | HELD) == ACQUIRING
    }

    /// Holding a target.
    pub fn locked(self) -> bool {
        self.flags & HELD != 0
    }

    /// The target is too near for the weapon.
    pub fn too_close(self) -> bool {
        self.flags & TOO_NEAR != 0
    }

    /// Whether the weapon `weapon` in life `life` may fire at the locked target now.
    pub fn can_fire(self, weapon: u32, life: u32) -> bool {
        let same_owner = self.weapon == weapon && self.life == life;
        same_owner && self.locked() && self.flags & FIRE_BLOCKERS == 0 && self.aim.is_some()
    }
}
