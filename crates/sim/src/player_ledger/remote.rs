//! A missile a player is steering by remote.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SteeredMissile {
    pub projectile: crate::ThrownId,
    pub ent_no: i32,
    pub turn: [f32; 3],
    pub armed: bool,
    pub boosted: bool,
    pub attack: bool,
    /// It flies on unsteered and slowly (the Hellstorm once its bomblets are out).
    pub coasting: bool,
    pub unlink_at_ms: Option<i32>,
    /// A Black Ops II Hellstorm: steered by mouse it turns faster, and it boosts once by itself when the scripts
    /// armed it.
    pub t6: bool,
    /// The player's last command held the control button (BO2 boosts on that, not on fire).
    pub holding: bool,
}
