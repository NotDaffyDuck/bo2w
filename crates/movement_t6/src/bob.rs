//! The bob cycle's footstep beat: the BO2 callers' home.

/// True when the bob cycle crossed a footstep beat between `old` and `new`.
/// TODO(fact): BO2's bob cycle wrap. Stand-in: MW2's
/// `crate::bridge::sway_phase_looped` (the cycle's high bit, a quarter turn
/// on, changed).
#[inline]
#[must_use]
pub fn sway_phase_looped(prev: u8, cur: u8) -> bool {
    crate::bridge::sway_phase_looped(prev, cur)
}
