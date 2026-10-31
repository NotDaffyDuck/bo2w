//! Why the authority turns a request down. Each reason also has a stable lower-case name for logs and the console.

macro_rules! reject_reasons {
    ($($(#[$note:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? })+) => {$(
        $(#[$note])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    )+};
}

reject_reasons! {
    /// A class pick that was refused.
    LoadoutRefusal {
        UnknownOrStaleClass = "unknown_or_stale_class",
        NoSpawnAvailable = "no_spawn_available",
        LockedContent = "locked_content",
        NoSuchGun = "unknown_weapon_id",
    }

    /// A weapon give that was refused.
    HandoutRefusal {
        NotAlive = "not_alive",
        InvalidWeapon = "invalid_weapon",
        NoSuchGun = "unknown_weapon_id",
        UnsupportedWeapon = "unsupported_weapon",
        EmptyCombatProfile = "empty_combat_profile",
    }

    /// An attachment-set change that was refused.
    ConfigurationChangeRejectReason {
        NotAlive = "not_alive",
        StaleSource = "stale_source",
        InvalidTarget = "invalid_target",
        DifferentFamily = "different_family",
        Busy = "busy",
        NoInventorySlot = "no_inventory_slot",
        AmmoTableFull = "ammo_table_full",
        SharedAmmoConflict = "shared_ammo_conflict",
    }
}
/// Older spelling ClassRejectReason still calls; the rename sweep moves it over.
pub use self::LoadoutRefusal as ClassRejectReason;
/// Older spelling GiveRejectReason still calls; the rename sweep moves it over.
pub use self::HandoutRefusal as GiveRejectReason;
