//! The bob cycle's footstep beat: the BO2 callers' home.

/// True when the bob cycle crossed a footstep beat between `old` and `new`.
/// TODO(fact): BO2's bob cycle wrap. Stand-in: MW2's
/// `crate::bridge::bob_cycle_wrapped` (the cycle's high bit, a quarter turn
/// on, changed).
#[inline]
#[must_use]
pub fn bob_cycle_wrapped(old: u8, new: u8) -> bool {
    crate::bridge::bob_cycle_wrapped(old, new)
}
