//! Ammunition on dropped guns, and the record of a gun being taken.

/// What a dropped gun still holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DroppedItemAmmo {
    pub entnum: i32,
    pub clip_r: i32,
    pub clip_l: i32,
    pub stock: i32,
    pub stock2: i32,
    pub scavenger: i32,
    pub owner_clear_ms: i32,
}

/// One pickup, as the clients replay it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemPickupRecord {
    pub picker: i32,
    pub weapon: u32,
    pub from_entnum: i32,
    pub clip_r: i32,
    pub clip_l: i32,
    pub stock: i32,
    pub swapped_entnum: i32,
    pub picker_pm_type: i32,
    /// Ammo only: the gun went into one the picker already holds (a touch, a blade picked back up) and the item may
    /// still lie there.
    pub ammo_only: bool,
}
