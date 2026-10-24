//! The killcam script's timings and weapon lists. BO2's _killcam.gsc is compiled bytecode
//! whose numbers were not decoded; its names (calckillcamtime, calcpostdelay, killcamlength,
//! archivetime, deathtimeoffset, dofinalkillcam, addkillcamtimer) match MW2's, so MW2's values
//! stand in, each marked until BO2's are read.

/// Seconds after death before the killcam (postdelay).
pub const POST_DELAY_S: f32 = 2.0; // BO2 unknown
/// The shortest killcam (maxtime floor).
pub const MIN_MAX_TIME_S: f32 = 2.0; // BO2 unknown
/// Archive trim epsilon.
pub const ARCHIVE_TRIM_EPSILON: f32 = 0.0001; // BO2 unknown
/// Two presses of skip inside this many seconds skip the killcam.
pub const DOUBLE_TAP_SKIP_S: f32 = 0.5; // BO2 unknown
/// The final killcam's maxtime (effectively unbounded).
pub const FINAL_MAX_TIME: f32 = 10000.0; // BO2 unknown
/// The final killcam's settle and poll steps (ms).
pub const FINAL_SETTLE_MS: i32 = 100; // BO2 unknown
pub const FINAL_POLL_MS: i32 = 50; // BO2 unknown

/// iskillcamgrenadeweapon: grenades whose killcam follows the grenade.
pub const GRENADE_WEAPONS: [&str; 4] = [
    "tabun_gas_mp",
    "sticky_grenade_mp",
    "frag_grenade_short_mp",
    "frag_grenade_mp",
];

/// iskillcamentityweapon: weapons whose killcam follows their entity.
pub const ENTITY_WEAPONS: [&str; 1] = ["planemortar_mp"];

#[must_use]
pub fn is_grenade_weapon(name: &str) -> bool {
    GRENADE_WEAPONS.contains(&name)
}

#[must_use]
pub fn is_entity_weapon(name: &str) -> bool {
    ENTITY_WEAPONS.contains(&name)
}
